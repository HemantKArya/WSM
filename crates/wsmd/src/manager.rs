//! VM lifecycle: create / start (resume or cold boot) / suspend / stop / status, plus idle policy.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use wsm_core::hostinfo;
use wsm_core::manifest::Manifest;
use wsm_core::paths::{self, InstanceDirs};
use wsm_core::provider::{self, Runtime};
use wsm_core::qmp::Qmp;
use wsm_core::seed;
use wsm_core::ssh::{self, Target};
use wsm_core::suspend;

use crate::serial::Hub;

pub type Progress<'a> = &'a mut dyn FnMut(&str);

pub struct Manager {
    pub home: PathBuf,
    pub rt: Runtime,
    /// Per-instance operation lock, so two CLI calls can't race a start against a suspend.
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Open leases (attached shells, running execs) per instance and last activity time.
    activity: Mutex<HashMap<String, (u32, Instant)>>,
    hubs: Mutex<HashMap<String, Arc<Hub>>>,
    ssh_ready: Mutex<HashMap<String, u32>>,
    pool: crate::ssh_pool::Pool,
    gpu_start: Mutex<()>,
    /// `wsm mount` WebDAV bridges: instance -> (port, token).
    mounts: Mutex<HashMap<String, (u16, String)>>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StateMeta {
    disk_len: u64,
    disk_mtime_ns: u128,
    ram_mib: u32,
    vcpus: u32,
    gpu: bool,
    saved_unix: u64,
    bytes: u64,
}

fn mtime_ns(p: &std::path::Path) -> u128 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

impl Manager {
    pub fn new(home: PathBuf) -> Self {
        let rt = Runtime::new(paths::runtime_dir(&home));
        Manager { home, rt, locks: Default::default(), activity: Default::default(), hubs: Default::default(), ssh_ready: Default::default(), pool: Default::default(), gpu_start: Default::default(), mounts: Default::default() }
    }

    fn lock(&self, name: &str) -> Arc<Mutex<()>> {
        self.locks.lock().unwrap().entry(name.to_string()).or_default().clone()
    }

    fn dirs(&self, name: &str) -> InstanceDirs { InstanceDirs::new(&self.home, name) }

    fn load(&self, name: &str) -> Result<(Manifest, InstanceDirs)> {
        let d = self.dirs(name);
        let m = Manifest::load(&d.manifest()).with_context(|| format!("instance '{name}' does not exist (wsm create)"))?;
        Ok((m, d))
    }

    pub fn list(&self) -> Vec<String> {
        std::fs::read_dir(self.home.join("instances"))
            .map(|rd| rd.flatten().filter(|e| e.path().join("manifest.toml").exists()).map(|e| e.file_name().to_string_lossy().into()).collect())
            .unwrap_or_default()
    }

    // ---------------------------------------------------------------- create
    pub fn create(&self, name: &str, args: &Value, progress: Progress) -> Result<Value> {
        let l = self.lock(name);
        let _g = l.lock().unwrap();
        anyhow::ensure!(
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "instance name must be [A-Za-z0-9_-]"
        );
        let d = self.dirs(name);
        anyhow::ensure!(!d.manifest().exists(), "instance '{name}' already exists");
        let gpu = args["gpu"].as_bool().unwrap_or(false);
        let base = match args["base"].as_str() {
            Some(b) => PathBuf::from(b),
            None => newest_base(&self.home, gpu).context("no base image: run `wsm base import <file>` or `wsm base build --iso <iso>`")?,
        };
        anyhow::ensure!(base.exists(), "base image {} not found", base.display());
        let base = std::path::absolute(base)?;
        let mut m = Manifest::new(name, &base.display().to_string(), gpu, suspend::free_port()?);
        // A lite base (BaseSystem-derived) ships a data-disk template next to it.
        let lite_data = base.file_name().and_then(|f| f.to_str()).filter(|f| f.starts_with("base-lite-")).map(|f| base.with_file_name(f.replace(".qcow2", "-data.qcow2")));
        if let Some(t) = &lite_data {
            anyhow::ensure!(!gpu, "a lite base has no GPU/GUI support; create the GPU instance from the full base");
            anyhow::ensure!(t.exists(), "data-disk template {} missing next to the lite base", t.display());
            m.kind = "lite".into();
            m.ram_mib = 4096;
        }
        if let Some(r) = args["ram"].as_u64() { m.ram_mib = r as u32; }
        if let Some(c) = args["vcpus"].as_u64() { m.vcpus = c as u32; }
        m.validate()?;
        if !self.rt.missing().is_empty() {
            bail!("runtime incomplete, missing: {:?}", self.rt.missing());
        }
        std::fs::create_dir_all(d.logs())?;
        ssh::restrict_to_owner(&d.root);
        progress("creating overlay disk");
        let mut c = Command::new(self.rt.qemu_img());
        c.args(["create", "-q", "-f", "qcow2", "-F", "qcow2", "-b"]).arg(&base).arg(d.disk());
        let st = ssh::hidden(&mut c).status()?;
        anyhow::ensure!(st.success(), "qemu-img create failed");
        if let Some(t) = &lite_data {
            progress("creating data disk");
            let mut c = Command::new(self.rt.qemu_img());
            c.args(["create", "-q", "-f", "qcow2", "-F", "qcow2", "-b"]).arg(t).arg(d.data_disk());
            anyhow::ensure!(ssh::hidden(&mut c).status()?.success(), "qemu-img create (data disk) failed");
        }
        std::fs::copy(self.rt.ovmf_vars(), d.vars())?;
        let tmpl = self.rt.oc_template(hostinfo::cpu_vendor().oc_variant());
        anyhow::ensure!(tmpl.exists(), "OpenCore template {} missing", tmpl.display());
        let oc = d.root.join("opencore.img");
        m.oc_image = oc.display().to_string().replace('\\', "/");
        progress("generating this instance's OpenCore image");
        self.regen_opencore(&mut m)?;
        progress("generating SSH key and config disk");
        let key = ssh::ensure_key(&d.keys())?;
        let pubkey = std::fs::read_to_string(key.with_extension("pub"))?;
        let host_key = if lite_data.is_some() { Some(std::fs::read_to_string(ssh::ensure_host_key(&d.keys())?)?) } else { None };
        seed::write_config_disk(&d.config_disk(), &m.user, &format!("wsm-{}", name.replace('_', "-")), &pubkey, host_key.as_deref())?;
        m.save(&d.manifest())?;
        Ok(json!({"name": name, "ssh_port": m.ssh_port, "gpu": gpu}))
    }

    /// Per-instance OpenCore: template for this host's CPU vendor + identity + vCPU topology patch.
    fn regen_opencore(&self, m: &mut Manifest) -> Result<()> {
        let tmpl = self.rt.oc_template(hostinfo::cpu_vendor().oc_variant());
        let id = wsm_core::opencore::Identity {
            serial: &m.smbios_serial, mlb: &m.smbios_mlb, uuid: &m.smbios_uuid, rom: m.rom(),
        };
        wsm_core::opencore::make_instance_image(&tmpl, std::path::Path::new(&m.oc_image), m.vcpus, &id)?;
        if m.kind == "lite" {
            wsm_core::opencore::edit_config(std::path::Path::new(&m.oc_image), |cfg| wsm_core::opencore::set_csr(cfg, 0x67))?;
        }
        m.oc_vcpus = m.vcpus;
        Ok(())
    }

    pub fn delete(&self, name: &str) -> Result<Value> {
        let l = self.lock(name);
        let _g = l.lock().unwrap();
        let (_, d) = self.load(name)?;
        if provider::running_pid(&d).is_some() {
            bail!("instance is running; `wsm stop --kill` first");
        }
        std::fs::remove_dir_all(&d.root)?;
        Ok(json!({}))
    }

    // ---------------------------------------------------------------- status
    /// Cheap check (no SSH probe) for paths that must answer quickly.
    pub fn is_running(&self, name: &str) -> bool {
        self.load(name).map(|(_, d)| provider::running_pid(&d).is_some()).unwrap_or(false)
    }

    pub fn status(&self, name: &str, probe: bool) -> Result<Value> {
        let (m, d) = self.load(name)?;
        let pid = provider::running_pid(&d);
        let key = d.keys().join("id_ed25519");
        let ssh_ready = pid.is_some() && if probe {
            ssh::ready(&Target { user: &m.user, port: m.ssh_port, key: &key })
        } else {
            self.ssh_ready.lock().unwrap().get(name).copied() == pid
        };
        let state = if pid.is_some() { "running" } else if d.state().exists() { "suspended" } else { "stopped" };
        Ok(json!({
            "name": name, "state": state, "pid": pid, "ssh_ready": ssh_ready, "ssh_port": m.ssh_port,
            "gpu": m.gpu, "ram_mib": m.ram_mib, "vcpus": m.vcpus,
            "state_bytes": std::fs::metadata(d.state()).map(|x| x.len()).ok(),
            "disk_bytes": std::fs::metadata(d.disk()).map(|x| x.len()).ok(),
        }))
    }

    // ----------------------------------------------------------------- start
    fn other_gpu_running(&self, name: &str) -> Option<String> {
        for n in self.list() {
            if n == name { continue; }
            if let Ok((m, d)) = self.load(&n)
                && m.gpu && provider::running_pid(&d).is_some() { return Some(n); }
        }
        None
    }

    fn state_is_fresh(&self, m: &Manifest, d: &InstanceDirs) -> bool {
        let Ok(s) = std::fs::read_to_string(d.state_meta()) else { return false };
        let Ok(meta) = serde_json::from_str::<StateMeta>(&s) else { return false };
        d.state().exists()
            && meta.disk_len == std::fs::metadata(d.disk()).map(|x| x.len()).unwrap_or(0)
            && meta.disk_mtime_ns == mtime_ns(&d.disk())
            && meta.ram_mib == m.ram_mib && meta.vcpus == m.vcpus && meta.gpu == m.gpu
    }

    fn drop_state(&self, d: &InstanceDirs) {
        let _ = std::fs::remove_file(d.state());
        let _ = std::fs::remove_file(d.state_meta());
    }

    /// `wait`: "none" returns once QEMU is up (serial available), "ssh" waits for SSH.
    pub fn start(&self, name: &str, wait: &str, force_cold: bool, progress: Progress) -> Result<Value> {
        let l = self.lock(name);
        let _g = l.lock().unwrap();
        let (mut m, d) = self.load(name)?;
        self.touch(name);
        if let Some(pid) = provider::running_pid(&d) {
            if wait == "ssh" && self.ssh_ready.lock().unwrap().get(name) != Some(&pid) {
                let key = d.keys().join("id_ed25519");
                let target = Target { user: &m.user, port: m.ssh_port, key: &key };
                let deadline = Instant::now();
                while !ssh::ready(&target) {
                    anyhow::ensure!(deadline.elapsed() < Duration::from_secs(120), "SSH did not become ready");
                    anyhow::ensure!(provider::running_pid(&d).is_some(), "QEMU exited while waiting for SSH");
                    std::thread::sleep(Duration::from_millis(250));
                }
                self.ssh_ready.lock().unwrap().insert(name.to_string(), pid);
            }
            return Ok(json!({"already_running": true, "pid": pid, "mode": "running"}));
        }
        let _gpu_guard = if m.gpu { Some(self.gpu_start.lock().unwrap()) } else { None };
        if m.gpu
            && let Some(o) = self.other_gpu_running(name) {
                bail!("GPU instance '{o}' is already running; only one GPU instance can run at a time");
            }
        if !self.rt.missing().is_empty() {
            bail!("runtime incomplete, missing: {:?}", self.rt.missing());
        }
        if std::net::TcpListener::bind(("127.0.0.1", m.ssh_port)).is_err() {
            let np = suspend::free_port()?;
            progress(&format!("ssh port {} is taken, using {np}", m.ssh_port));
            m.ssh_port = np;
            m.save(&d.manifest())?;
        }
        // The instance dir holds the serial console (a login shell), QMP and the migration socket:
        // only the owner may reach them, wherever WSM_HOME points.
        ssh::restrict_to_owner(&d.root);
        ssh::secure_key(&d.keys().join("id_ed25519"));
        if m.oc_vcpus != m.vcpus && !self.state_is_fresh(&m, &d) {
            progress("vCPU count changed: regenerating the OpenCore image");
            self.regen_opencore(&mut m)?;
            m.save(&d.manifest())?;
        }
        let t0 = Instant::now();
        self.ssh_ready.lock().unwrap().remove(name);
        self.pool.invalidate(name);
        let fresh = !force_cold && self.state_is_fresh(&m, &d);
        if !fresh && d.state().exists() {
            progress("saved state is stale, discarding it (cold boot)");
            self.drop_state(&d);
        }
        
        let mode = if fresh {
            progress("resuming from saved state");
            let pid = provider::spawn(&m, &d, &self.rt, true)?;
            self.hub_for(&m, &d);
            let feeder = {
                let (st, sock) = (d.state(), d.migrate_sock());
                std::thread::spawn(move || suspend::feed(&sock, &st))
            };
            let res = (|| -> Result<()> {
                // Keep the stream open until QEMU leaves "inmigrate": it never closes the socket itself.
                let _stream = feeder.join().map_err(|_| anyhow::anyhow!("feeder panicked"))??;
                let mut q = self.qmp_wait(&d, 30)?;
                let t = Instant::now();
                loop {
                    let s = q.status()?;
                    if s == "paused" || s == "running" { break; }
                    if s == "internal-error" || t.elapsed() > Duration::from_secs(30) { bail!("restore ended in state {s}"); }
                    std::thread::sleep(Duration::from_millis(50));
                }
                q.execute("cont", json!({}))?;
                Ok(())
            })();
            if let Err(e) = res {
                provider::kill(pid);
                // The state file stays: a failed restore must not destroy the only copy.
                let _ = std::fs::rename(d.state(), d.root.join("state.bin.failed"));
                self.drop_state(&d);
                bail!("resume failed: {e:#}; see {}; falling back requires `wsm start --cold`", d.logs().join("qemu.err.log").display());
            }
            self.drop_state(&d); // the guest now diverges from the saved image
            sync_clock_when_ready(&m, &d);
            "resume"
        } else {
            progress("cold boot");
            provider::spawn(&m, &d, &self.rt, false)?;
            self.hub_for(&m, &d);
            self.qmp_wait(&d, 30)?;
            "cold"
        };
        let qemu_up = t0.elapsed().as_secs_f64();
        let mut ssh_s = None;
        if wait == "ssh" {
            progress("waiting for SSH");
            let key = d.keys().join("id_ed25519");
            let t = Target { user: &m.user, port: m.ssh_port, key: &key };
            let limit = Duration::from_secs(if mode == "resume" { 60 } else { 120 });
            while t0.elapsed() < limit {
                if ssh::ready(&t) { ssh_s = Some(t0.elapsed().as_secs_f64()); break; }
                if provider::running_pid(&d).is_none() { bail!("QEMU exited during boot; see {}", d.logs().join("qemu.err.log").display()); }
                std::thread::sleep(Duration::from_millis(250));
            }
            if ssh_s.is_none() { bail!("SSH did not come up within {limit:?}"); }
            if let Some(pid) = provider::running_pid(&d) {
                self.ssh_ready.lock().unwrap().insert(name.to_string(), pid);
            }
        }
        Ok(json!({"mode": mode, "qemu_up_s": qemu_up, "ssh_s": ssh_s, "ssh_port": m.ssh_port}))
    }

    fn hub_for(&self, m: &Manifest, d: &InstanceDirs) -> Arc<Hub> {
        let mut h = self.hubs.lock().unwrap();
        h.insert(m.name.clone(), Hub::start(d.serial_sock(), d.logs().join("serial.log")));
        h[&m.name].clone()
    }

    /// Relay a CLI connection to the instance's serial console (blocks until the client leaves).
    pub fn serial_attach(&self, name: &str, client: wsm_core::ipc::Stream) -> Result<()> {
        let (m, d) = self.load(name)?;
        anyhow::ensure!(provider::running_pid(&d).is_some(), "instance is not running");
        let hub = { self.hubs.lock().unwrap().get(name).cloned() }.unwrap_or_else(|| self.hub_for(&m, &d));
        hub.attach(client);
        Ok(())
    }

    fn qmp_wait(&self, d: &InstanceDirs, secs: u64) -> Result<Qmp> {
        let t = Instant::now();
        loop {
            match Qmp::connect(&d.qmp_sock(), Duration::from_secs(5)) {
                Ok(q) => return Ok(q),
                Err(e) if t.elapsed() > Duration::from_secs(secs) => return Err(e).context("QMP never came up"),
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    // --------------------------------------------------------------- suspend
    pub fn suspend(&self, name: &str, progress: Progress) -> Result<Value> {
        let l = self.lock(name);
        let _g = l.lock().unwrap();
        let (m, d) = self.load(name)?;
        self.pool.invalidate(name);
        let Some(pid) = provider::running_pid(&d) else { bail!("instance is not running") };
        if m.gpu {
            // The apple-gfx device registers a migration blocker and its host-side Vulkan state is not in
            // the VM state stream (P1, logs/r3-p1-*): a GPU instance can only be shut down and cold-booted.
            drop(_g);
            progress("GPU instances cannot be suspended; shutting down cleanly");
            let r = self.shutdown(name, progress)?;
            return Ok(json!({"save_s": r["s"], "bytes": 0, "mode": "shutdown"}));
        }
        progress("saving VM state");
        let t = Instant::now();
        let stats = match suspend::save(&d.qmp_sock(), &d.migrate_sock(), &d.state()) {
            Ok(s) => s,
            Err(e) => {
                // Leave the VM running and usable rather than half-suspended.
                if let Ok(mut q) = Qmp::connect(&d.qmp_sock(), Duration::from_secs(5)) { let _ = q.execute("cont", json!({})); }
                let _ = std::fs::remove_file(d.state().with_extension("part"));
                bail!("suspend failed, VM left running: {e:#}");
            }
        };
        let save_s = t.elapsed().as_secs_f64();
        let bytes = stats.bytes;
        let tq = Instant::now();
        self.quit_qemu(&d, pid);
        let quit_s = tq.elapsed().as_secs_f64();
        let meta = StateMeta {
            disk_len: std::fs::metadata(d.disk())?.len(), disk_mtime_ns: mtime_ns(&d.disk()),
            ram_mib: m.ram_mib, vcpus: m.vcpus, gpu: m.gpu,
            saved_unix: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs(), bytes,
        };
        std::fs::write(d.state_meta(), serde_json::to_string(&meta)?)?;
        Ok(json!({"save_s": save_s, "bytes": bytes, "stats": stats, "quit_s": quit_s}))
    }

    fn quit_qemu(&self, d: &InstanceDirs, pid: u32) {
        if let Ok(mut q) = Qmp::connect(&d.qmp_sock(), Duration::from_secs(3)) { let _ = q.execute("quit", json!({})); }
        let t = Instant::now();
        while provider::pid_alive(pid) && t.elapsed() < Duration::from_secs(8) {
            std::thread::sleep(Duration::from_millis(100));
        }
        // `quit` can leave a GPU-rail QEMU alive (round 2); this is our own recorded PID, never a name match.
        if provider::pid_alive(pid) { provider::kill(pid); }
    }

    // ------------------------------------------------------------------ stop
    pub fn shutdown(&self, name: &str, progress: Progress) -> Result<Value> {
        let l = self.lock(name);
        let _g = l.lock().unwrap();
        let (m, d) = self.load(name)?;
        self.pool.invalidate(name);
        let Some(pid) = provider::running_pid(&d) else { self.drop_state(&d); return Ok(json!({"was_running": false})) };
        progress("requesting guest shutdown");
        let key = d.keys().join("id_ed25519");
        let t = Target { user: &m.user, port: m.ssh_port, key: &key };
        let _ = ssh::exec(&t, &["sudo".into(), "shutdown".into(), "-h".into(), "now".into()]);
        if let Ok(mut q) = Qmp::connect(&d.qmp_sock(), Duration::from_secs(3)) { let _ = q.execute("system_powerdown", json!({})); }
        let start = Instant::now();
        let mut clean = false;
        while start.elapsed() < Duration::from_secs(60) {
            // -no-shutdown keeps QEMU alive after the guest powers off.
            if let Ok(mut q) = Qmp::connect(&d.qmp_sock(), Duration::from_secs(2)) {
                if q.status().map(|s| s == "shutdown").unwrap_or(false) { clean = true; break; }
            } else if !provider::pid_alive(pid) { clean = true; break; }
            std::thread::sleep(Duration::from_millis(500));
        }
        self.quit_qemu(&d, pid);
        self.drop_state(&d);
        let _ = std::fs::remove_file(d.pid());
        Ok(json!({"was_running": true, "clean": clean, "s": start.elapsed().as_secs_f64()}))
    }

    pub fn kill(&self, name: &str) -> Result<Value> {
        let l = self.lock(name);
        let _g = l.lock().unwrap();
        self.pool.invalidate(name);
        let (_, d) = self.load(name)?;
        if let Some(pid) = provider::running_pid(&d) { provider::kill(pid); }
        let _ = std::fs::remove_file(d.pid());
        // A kill leaves a disk the saved state no longer matches.
        self.drop_state(&d);
        Ok(json!({}))
    }

    // ------------------------------------------------------------- activity
    pub fn exec(&self, name: &str, args: &Value, client: wsm_core::ipc::Stream) -> Result<()> {
        use std::io::{BufRead, BufReader, Write};
        use std::sync::{mpsc, atomic::{AtomicBool, Ordering}};
        use wsm_core::proto::Response;
        let argv: Vec<String> = serde_json::from_value(args["argv"].clone())?;
        anyhow::ensure!(!argv.is_empty(), "exec needs a command");
        let mut out = client.try_clone()?;
        out.set_write_timeout(Some(Duration::from_secs(30)))?;
        let mut emit = |r: Response| -> Result<()> {
            writeln!(out, "{}", serde_json::to_string(&r)?)?;
            Ok(())
        };
        self.start(name, "ssh", false, &mut |p| { let _ = emit(Response { progress: Some(p.into()), ..Default::default() }); })?;
        let (m, d) = self.load(name)?;
        let key = d.keys().join("id_ed25519");
        let root = args["root"].as_bool().unwrap_or(false);
        let target = Target { user: if root { "root" } else { &m.user }, port: m.ssh_port, key: &key };
        let pool_key = if root { format!("{name}#root") } else { name.to_string() };
        let (tx, rx) = mpsc::sync_channel(8);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_reader = cancelled.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(client).lines() {
                let Ok(line) = line else { break; };
                let Ok(i) = serde_json::from_str::<crate::ssh_pool::Input>(&line) else { break; };
                if tx.send(i).is_err() { return; }
            }
            cancel_reader.store(true, Ordering::Relaxed);
        });
        self.lease(name, 1);
        let result = self.pool.exec(&pool_key, &target, &argv, rx, &cancelled, &mut emit);
        self.lease(name, -1);
        let code = result?;
        emit(Response { ok: Some(true), exit_code: Some(code), ..Default::default() })
    }

    pub fn touch(&self, name: &str) {
        let mut a = self.activity.lock().unwrap();
        a.entry(name.to_string()).or_insert((0, Instant::now())).1 = Instant::now();
    }

    pub fn lease(&self, name: &str, delta: i32) {
        let mut a = self.activity.lock().unwrap();
        let e = a.entry(name.to_string()).or_insert((0, Instant::now()));
        e.0 = (e.0 as i32 + delta).max(0) as u32;
        e.1 = Instant::now();
    }

    /// Suspend instances idle for `idle_minutes` with no leases. Called by the daemon's timer.
    pub fn idle_sweep(&self) {
        for n in self.list() {
            let Ok((m, d)) = self.load(&n) else { continue };
            if m.idle_minutes == 0 || m.gpu || provider::running_pid(&d).is_none() { continue; }
            let idle = {
                let a = self.activity.lock().unwrap();
                match a.get(&n) {
                    Some((0, t)) => t.elapsed() >= Duration::from_secs(m.idle_minutes as u64 * 60),
                    Some(_) => false,
                    None => { drop(a); self.touch(&n); false }
                }
            };
            if idle {
                let _ = self.suspend(&n, &mut |_| {});
            }
        }
    }

    pub fn gui(&self, name: &str, action: &str) -> Result<Value> {
        let (m, d) = self.load(name)?;
        anyhow::ensure!(m.gpu, "instance '{name}' has no GPU profile (create with --gpu)");
        let key = d.keys().join("id_ed25519");
        let t = Target { user: &m.user, port: m.ssh_port, key: &key };
        let o = ssh::exec(&t, &["sudo".into(), "/usr/local/libexec/wsm-gui".into(), action.into()])?;
        let (out, err) = (String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        anyhow::ensure!(o.status.success(), "wsm-gui {action}: {}{}", err.trim(), out.trim());
        // The desktop session stays logged in and warm while hidden (see guest/wsm-gui.sh).
        if matches!(action, "start" | "stop" | "logout") && let Some(pid) = provider::running_pid(&d) {
            hostinfo::set_vm_window_visible(pid, action == "start");
        }
        Ok(json!({"out": format!("{out}{err}")}))
    }
}

impl crate::dav::Backend for Manager {
    /// Resumes or starts a stopped instance when its drive is used.
    fn target(&self, name: &str) -> Result<(String, u16, PathBuf)> {
        self.start(name, "ssh", false, &mut |_| {})?;
        let (m, d) = self.load(name)?;
        Ok((m.user, m.ssh_port, d.keys().join("id_ed25519")))
    }
    /// Every file request counts as activity for the idle policy.
    fn touch(&self, name: &str) { Manager::touch(self, name); }
}

impl Manager {
    /// Start (once per daemon) the WebDAV bridge serving this instance's guest home to Windows.
    pub fn mount(self: &Arc<Self>, name: &str) -> Result<Value> {
        self.load(name)?;
        let mut mounts = self.mounts.lock().unwrap();
        let (port, token) = match mounts.get(name) {
            Some(m) => m.clone(),
            None => {
                let src = Arc::new(crate::dav::SftpSource { backend: self.clone(), name: name.to_string() });
                let m = crate::dav::serve(src, provider::random_hex(16))?;
                mounts.insert(name.to_string(), (m.port, m.token.clone()));
                (m.port, m.token)
            }
        };
        Ok(json!({"port": port, "token": token, "unc": format!(r"\\127.0.0.1@{port}\{token}")}))
    }

    /// Serve a Windows folder (once per daemon and folder) for guests to mount from 10.0.2.2.
    pub fn share(&self, folder: &str) -> Result<Value> {
        let dir = std::fs::canonicalize(folder).with_context(|| format!("shared folder {folder}"))?;
        anyhow::ensure!(dir.is_dir(), "{} is not a folder", dir.display());
        let key = dir.display().to_string();
        let mut mounts = self.mounts.lock().unwrap();
        let (port, token) = match mounts.get(&key) {
            Some(m) => m.clone(),
            None => {
                let m = crate::dav::serve(Arc::new(crate::dav::LocalSource(dir.clone())), provider::random_hex(16))?;
                mounts.insert(key, (m.port, m.token.clone()));
                (m.port, m.token)
            }
        };
        Ok(json!({"port": port, "token": token, "folder": dir.display().to_string(),
                  "url": format!("http://10.0.2.2:{port}/{token}")}))
    }
}

/// Guest time stands still while suspended (partition time and TSC are restored as saved), so after a
/// resume the guest clock is behind by the suspend duration. Set it from the host once SSH answers,
/// the way a Mac re-reads its RTC on wake. Runs in the background so `wsm start --no-wait` stays fast.
fn sync_clock_when_ready(m: &Manifest, d: &InstanceDirs) {
    let (user, port, key) = (m.user.clone(), m.ssh_port, d.keys().join("id_ed25519"));
    std::thread::spawn(move || {
        let t = Target { user: &user, port, key: &key };
        let start = Instant::now();
        while !ssh::ready(&t) {
            if start.elapsed() > Duration::from_secs(90) { return; }
            std::thread::sleep(Duration::from_millis(250));
        }
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|x| x.as_secs()).unwrap_or(0);
        let cmd = format!("sudo -n /bin/date -u \"$(/bin/date -u -r {now} +%m%d%H%M%Y.%S)\" >/dev/null");
        let _ = ssh::exec(&t, &[cmd]);
    });
}

/// The newest `base-*.qcow2` under `<home>\bases`.
/// Newest base for a new instance: GPU instances need the full base; core instances prefer a lite base.
pub fn newest_base(home: &std::path::Path, gpu: bool) -> Option<PathBuf> {
    let all: Vec<_> = std::fs::read_dir(paths::bases_dir(home)).ok()?.flatten()
        .filter(|e| { let n = e.file_name().to_string_lossy().to_string(); n.ends_with(".qcow2") && !n.ends_with("-data.qcow2") }).collect();
    let lite = |e: &std::fs::DirEntry| e.file_name().to_string_lossy().starts_with("base-lite-");
    let mut v: Vec<_> = if gpu { all.into_iter().filter(|e| !lite(e)).collect() }
        else if all.iter().any(lite) { all.into_iter().filter(lite).collect() } else { all };
    v.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    v.last().map(|e| e.path())
}
