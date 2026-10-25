//! Reproducible base builder (P3): Tahoe installer ISO -> generic, compacted base qcow2.
//!
//! Stages (each can be restarted with `from_stage`; the work dir keeps what earlier stages made):
//!  0  extract BaseSystem.dmg from the ISO (7-Zip), convert it to a writable qcow2; blank target disk
//!  1  single-user BaseSystem (root shell on serial): install the builder daemon from a seed disk
//!  2  BaseSystem + ISO + target: the daemon runs startosinstall; the installer's own phases follow
//!     on the target until the installed system's first boot
//!  3  BaseSystem + target: the daemon puts the first-boot service, trim list and setup flags on the
//!     installed Data volume and powers off
//!  4  compact into `bases/base-tahoe-26.3-<sha12>.qcow2` + sidecar sha256 + manifest
//!
//! Nothing machine-specific goes on the disk: no user, keys, host keys or SMBIOS identity (those come
//! from each instance's config disk at first boot, `guest/wsm-firstboot.sh`). The OpenCore image used
//! here is a throwaway with a fixed builder identity.
use anyhow::{Context, Result, bail};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wsm_core::provider::{self, Runtime, fwd};
use wsm_core::{afunix, opencore, seed};

/// Guest-side files, embedded so `wsm.exe` alone can build a base.
const STAGE1: &[u8] = include_bytes!("../../../guest/build/stage1.sh");
const BUILD_SH: &[u8] = include_bytes!("../../../guest/build/build.sh");
const BUILD_PLIST: &[u8] = include_bytes!("../../../guest/build/com.wsm.build.plist");
const PAYLOAD: &[(&str, &[u8])] = &[
    ("wsm-firstboot.sh", include_bytes!("../../../guest/wsm-firstboot.sh")),
    ("wsm-init.sh", include_bytes!("../../../guest/wsm-init.sh")),
    ("wsm-serial.sh", include_bytes!("../../../guest/wsm-serial.sh")),
    ("wsm-gui.sh", include_bytes!("../../../guest/wsm-gui.sh")),
    ("com.wsm.firstboot.plist", include_bytes!("../../../guest/com.wsm.firstboot.plist")),
    ("com.wsm.init.plist", include_bytes!("../../../guest/com.wsm.init.plist")),
    ("com.wsm.serial.plist", include_bytes!("../../../guest/com.wsm.serial.plist")),
    ("launchctl-disable.txt", include_bytes!("../../../guest/launchctl-disable.txt")),
    ("launchctl-user-disable.txt", include_bytes!("../../../guest/launchctl-user-disable.txt")),
    ("clt.sh", include_bytes!("../../../guest/build/clt.sh")),
    ("clt-pins.json", include_bytes!("../../../guest/build/clt-pins.json")),
    ("com.wsm.clt.plist", include_bytes!("../../../guest/build/com.wsm.clt.plist")),
];

const ISO_BASESYSTEM: &str = "Install macOS Tahoe/BaseSystem/BaseSystem.dmg";
const TARGET_BYTES: u64 = 64 << 30;
const INSTALL_PEAK_BYTES: u64 = 40 << 30;
const BASE_BYTES_ESTIMATE: u64 = 18 << 30;
const RAM_MIB: u32 = 4096;
const VCPUS: u32 = 4;

pub struct Opts {
    pub iso: PathBuf,
    pub clt: bool,
    pub from_stage: u8,
    pub resume_install: bool,
}

pub type Progress<'a> = &'a mut dyn FnMut(&str);

struct Work {
    dir: PathBuf,
}

impl Work {
    fn p(&self, name: &str) -> PathBuf { self.dir.join(name) }
    fn basesystem(&self) -> PathBuf { self.p("basesystem.qcow2") }
    fn target(&self) -> PathBuf { self.p("target.qcow2") }
}

pub fn run(home: &Path, rt: &Runtime, opts: &Opts, progress: Progress) -> Result<PathBuf> {
    anyhow::ensure!(opts.from_stage <= 4, "--from-stage must be in 0..=4");
    anyhow::ensure!(opts.iso.exists(), "ISO {} not found", opts.iso.display());
    for p in [rt.qemu(), rt.qemu_img(), rt.seven_zip(), rt.ovmf_code(), rt.ovmf_vars()] {
        anyhow::ensure!(p.exists(), "runtime file missing: {}", p.display());
    }
    let w = Work { dir: home.join("build").join("tahoe-26.3") };
    std::fs::create_dir_all(&w.dir)?;
    if opts.from_stage <= 2 {
        // The install grows the target to ~38 GB (installer staging + OS; measured 38.44 GB before a
        // repack), then stage 4 writes a ~17 GB compressed copy next to it. A full disk mid-install
        // (seen: 1.3 MB free) leaves a half-written target, so check before starting.
        let existing = if opts.resume_install { std::fs::metadata(w.target()).map(|m| m.len()).unwrap_or(0) } else { 0 };
        let need_gib = (INSTALL_PEAK_BYTES + BASE_BYTES_ESTIMATE).saturating_sub(existing).div_ceil(1 << 30);
        let free_gib = wsm_core::hostinfo::free_disk_gb(&w.dir).context("cannot determine free disk space")?;
        anyhow::ensure!(free_gib >= need_gib,
            "not enough disk space for a base build: {free_gib} GiB free in {}, need about {need_gib} GiB", w.dir.display());
    }
    wsm_core::ssh::restrict_to_owner(&w.dir);
    let t_all = Instant::now();
    let mut times = serde_json::Map::new();
    let mut timed = |n: u8, name: &str, f: &mut dyn FnMut() -> Result<()>| -> Result<()> {
        if opts.from_stage > n {
            return Ok(());
        }
        let t = Instant::now();
        f().with_context(|| format!("stage {n} ({name})"))?;
        times.insert(format!("{n}_{name}_s"), (t.elapsed().as_secs_f64().round() as u64).into());
        Ok(())
    };
    timed(0, "extract", &mut || stage0_extract(rt, &w, &opts.iso, progress))?;
    timed(1, "seed", &mut || stage1_seed(rt, &w, progress))?;
    timed(2, "install", &mut || if opts.resume_install {
        stage2_resume(rt, &w, progress)
    } else { stage2_install(rt, &w, &opts.iso, progress) })?;
    timed(3, "inject", &mut || {
        // The daemon lives inside BaseSystem. Refresh it even when restarting directly at stage 3,
        // otherwise fixes in the host binary would silently run the previous guest script.
        stage1_seed(rt, &w, progress)?;
        stage3_inject(rt, &w, opts.clt, progress)?;
        if opts.clt {
            progress("installing pinned Command Line Tools on the full OS (no user or instance identity)");
            let mut vm = Vm::launch(rt, &w, "tools", false, &[
                Disk { path: w.target(), format: "qcow2", snapshot: false },
            ])?;
            vm.con.wait_any(0, &["WSM-CLT: done", "WSM-CLT: FAIL"], Duration::from_secs(60 * 60))
                .and_then(|marker| { anyhow::ensure!(marker == "WSM-CLT: done", "Command Line Tools installation failed; see tools/serial.log"); Ok(()) })?;
            anyhow::ensure!(vm.wait_exit(Duration::from_secs(120))?, "tools stage did not shut down");
            // Stock sshd can generate host keys during this boot. Sanitize again from Recovery.
            stage3_inject(rt, &w, false, progress)?;
        }
        Ok(())
    })?;
    let mut out = None;
    timed(4, "compact", &mut || {
        out = Some(stage4_compact(home, rt, &w, &opts.iso, opts.clt, progress)?);
        Ok(())
    })?;
    let out = out.context("stage 4 did not run")?;
    let mut m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(manifest_path(&out))?)?;
    m["stage_seconds"] = times.into();
    m["build_seconds"] = (t_all.elapsed().as_secs()).into();
    std::fs::write(manifest_path(&out), serde_json::to_string_pretty(&m)?)?;
    Ok(out)
}

fn manifest_path(base: &Path) -> PathBuf { base.with_extension("json") }

fn hidden(c: &mut Command) -> &mut Command { wsm_core::ssh::hidden(c) }

fn qemu_img(rt: &Runtime, args: &[&str]) -> Result<()> {
    let st = hidden(Command::new(rt.qemu_img()).args(args)).status()?;
    anyhow::ensure!(st.success(), "qemu-img {} failed", args.join(" "));
    Ok(())
}

// ------------------------------------------------------------------------------------------ stage 0
fn stage0_extract(rt: &Runtime, w: &Work, iso: &Path, progress: Progress) -> Result<()> {
    progress("extracting BaseSystem.dmg from the ISO");
    let dmg = w.p("BaseSystem.dmg");
    let mut c = Command::new(rt.seven_zip());
    c.args(["e", "-y"]).arg(format!("-o{}", w.dir.display())).arg(iso).arg(ISO_BASESYSTEM).stdout(Stdio::null());
    let st = hidden(&mut c).status()?;
    anyhow::ensure!(st.success() && dmg.exists(), "7-Zip could not extract {ISO_BASESYSTEM}");
    progress("converting BaseSystem to qcow2");
    qemu_img(rt, &["convert", "-q", "-f", "dmg", "-O", "qcow2", &dmg.display().to_string(), &w.basesystem().display().to_string()])?;
    std::fs::remove_file(&dmg)?;
    progress("creating the blank target disk");
    let _ = std::fs::remove_file(w.target());
    qemu_img(rt, &["create", "-q", "-f", "qcow2", &w.target().display().to_string(), &TARGET_BYTES.to_string()])?;
    Ok(())
}

// --------------------------------------------------------------------------------- VM + console
struct Disk {
    path: PathBuf,
    format: &'static str,
    /// Writes go to a temporary overlay (QEMU's IDE can't be read-only).
    snapshot: bool,
}

struct Vm {
    child: Child,
    con: Console,
    qmp_sock: PathBuf,
}

impl Vm {
    fn launch(rt: &Runtime, w: &Work, tag: &str, single_user: bool, disks: &[Disk]) -> Result<Vm> {
        Self::launch_impl(rt, w, tag, single_user, disks, None)
    }

    fn launch_impl(rt: &Runtime, w: &Work, tag: &str, single_user: bool, disks: &[Disk], saved_vars: Option<&Path>) -> Result<Vm> {
        let run = w.p(tag);
        if run.exists() {
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
            std::fs::rename(&run, w.p(&format!("{tag}-{stamp}")))?;
        }
        std::fs::create_dir_all(&run)?;
        let oc = run.join("opencore.img");
        let id = opencore::Identity { serial: "C02WSMBUILDER", mlb: "C02WSMBUILDER0000", uuid: "00000000-0000-4000-8000-000000000001", rom: [0x52, 0x54, 0, 0, 0, 1] };
        let tmpl = rt.oc_template(wsm_core::hostinfo::cpu_vendor().oc_variant());
        opencore::make_image(&tmpl, &oc, VCPUS, &id, |cfg| {
            opencore::no_picker(cfg)?;
            if single_user { opencore::add_boot_args(cfg, "-s")?; }
            Ok(())
        })?;
        // Fresh NVRAM per run, so the boot default is the first disk unless the installer blesses one.
        let vars = run.join("OVMF_VARS.fd");
        std::fs::copy(saved_vars.unwrap_or(&rt.ovmf_vars()), &vars)?;
        let (ser, qmp) = (run.join("serial.sock"), run.join("qmp.sock"));
        let mut a: Vec<String> = Vec::new();
        let mut p = |s: &str| a.push(s.to_string());
        p("-L"); p(&fwd(&rt.bios_dir()));
        p("-name"); p(&format!("wsm-build-{tag}"));
        p("-machine"); p("q35"); p("-accel"); p("whpx,hyperv=off");
        p("-cpu"); p(provider::CPU_MODEL);
        p("-smp"); p(&format!("{VCPUS},sockets=1,cores={VCPUS},threads=1")); p("-m"); p(&format!("{RAM_MIB}M"));
        for g in ["ICH9-LPC.disable_s3=1", "ICH9-LPC.disable_s4=1"] { p("-global"); p(g); }
        p("-device"); p(provider::APPLESMC);
        p("-drive"); p(&format!("if=pflash,format=raw,readonly=on,file={}", fwd(&rt.ovmf_code())));
        p("-drive"); p(&format!("if=pflash,format=raw,file={}", fwd(&vars)));
        p("-device"); p("ich9-ahci,id=sata,addr=0x2");
        p("-drive"); p(&format!("id=oc,if=none,format=raw,snapshot=on,file={}", fwd(&oc)));
        p("-device"); p("ide-hd,bus=sata.0,drive=oc,bootindex=1");
        for (i, d) in disks.iter().enumerate() {
            p("-drive");
            p(&format!("id=d{i},if=none,format={},{}file={}", d.format,
                if d.snapshot { "snapshot=on," } else { "discard=unmap,detect-zeroes=unmap," }, fwd(&d.path)));
            p("-device"); p(&format!("ide-hd,bus=sata.{},drive=d{i},rotation_rate=1", i + 1));
        }
        p("-netdev"); p("user,id=net0,ipv6=off");
        p("-device"); p("virtio-net-pci,netdev=net0,addr=0x3");
        p("-device"); p("VGA");
        p("-device"); p("qemu-xhci,id=xhci,addr=0x5"); p("-device"); p("usb-kbd,bus=xhci.0"); p("-device"); p("usb-tablet,bus=xhci.0");
        p("-display"); p("none"); p("-monitor"); p("none");
        p("-chardev"); p(&format!("socket,id=ser0,path={},server=on,wait=off", fwd(&ser)));
        p("-serial"); p("chardev:ser0");
        p("-qmp"); p(&format!("unix:{},server=on,wait=off", fwd(&qmp)));
        p("-msg"); p("timestamp=on");
        // No -no-shutdown: a guest halt/poweroff ends QEMU, which is how stages signal they are done.
        let err = std::fs::File::create(run.join("qemu.err.log"))?;
        let mut c = Command::new(rt.qemu());
        c.args(&a).env("TMP", &run).env("TEMP", &run).stdin(Stdio::null()).stdout(Stdio::null()).stderr(err);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000 | 0x0000_0200);
        }
        let child = c.spawn().context("starting QEMU")?;
        let con = Console::connect(&ser, &run.join("serial.log"))?;
        Ok(Vm { child, con, qmp_sock: qmp })
    }

    /// Wait for QEMU to exit on its own (guest halt), killing it after `limit`.
    fn wait_exit(&mut self, limit: Duration) -> Result<bool> {
        let t = Instant::now();
        while t.elapsed() < limit {
            if self.child.try_wait()?.is_some() { return Ok(true); }
            std::thread::sleep(Duration::from_millis(500));
        }
        self.kill();
        Ok(false)
    }

    fn kill(&mut self) {
        if let Ok(mut q) = wsm_core::qmp::Qmp::connect(&self.qmp_sock, Duration::from_secs(3)) {
            let _ = q.execute("quit", serde_json::json!({}));
        }
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(10) {
            if matches!(self.child.try_wait(), Ok(Some(_))) { return; }
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) { self.kill(); }
        // The per-run OpenCore copy is ~0.5 GB of mostly empty FAT; serial/QEMU logs stay.
        if let Some(run) = self.qmp_sock.parent() { let _ = std::fs::remove_file(run.join("opencore.img")); }
    }
}

/// The guest serial console: everything is appended to `serial.log`; waits search the text that
/// arrived after a mark, so an old match can't satisfy a new wait.
struct Console {
    buf: Arc<Mutex<Vec<u8>>>,
    w: TcpStream,
}

impl Console {
    fn connect(sock: &Path, log: &Path) -> Result<Console> {
        let s = afunix::connect_retry(sock, Duration::from_secs(30)).context("serial console")?;
        let w = s.try_clone()?;
        let buf = Arc::new(Mutex::new(Vec::new()));
        let (b, mut r) = (buf.clone(), s);
        let mut f = std::fs::File::create(log)?;
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = r.read(&mut chunk) {
                if n == 0 { break; }
                let _ = f.write_all(&chunk[..n]);
                b.lock().unwrap().extend_from_slice(&chunk[..n]);
            }
        });
        Ok(Console { buf, w })
    }

    fn mark(&self) -> usize { self.buf.lock().unwrap().len() }

    /// First of `pats` seen after `from`, or an error after `limit`.
    fn wait_any(&self, from: usize, pats: &[&str], limit: Duration) -> Result<String> {
        let t = Instant::now();
        loop {
            {
                let b = self.buf.lock().unwrap();
                let text = String::from_utf8_lossy(&b[from.min(b.len())..]);
                if let Some(p) = pats.iter().find(|p| text.contains(**p)) { return Ok(p.to_string()); }
                if let Some(i) = text.find("WSM-BUILD: FAIL") {
                    let line = text[i..].lines().next().unwrap_or("").to_string();
                    bail!("guest reported: {line}");
                }
            }
            if t.elapsed() > limit { bail!("timed out after {limit:?} waiting for {pats:?}"); }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Wait for a boot (a `Kernel boot args:` line after `from`) whose args do not contain `bad`,
    /// after at least one boot that does: i.e. the transition out of that boot mode.
    fn wait_boot_without(&self, from: usize, bad: &str, limit: Duration) -> Result<()> {
        let t = Instant::now();
        let mut seen_bad;
        loop {
            {
                let b = self.buf.lock().unwrap();
                let text = String::from_utf8_lossy(&b[from.min(b.len())..]).to_string();
                seen_bad = false;
                for l in text.lines().filter(|l| l.contains("Kernel boot args:")) {
                    if l.contains(bad) { seen_bad = true; } else if seen_bad && is_installed_boot(l) { return Ok(()); }
                }
                if let Some(i) = text.find("userspace panic") {
                    bail!("guest panicked during the install: {}", text[i..].lines().next().unwrap_or(""));
                }
            }
            if t.elapsed() > limit {
                bail!("no boot without `{bad}` within {limit:?} (saw an {bad} boot: {seen_bad})");
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    fn send(&mut self, line: &str) -> Result<()> {
        // The serial tty wants CR for Enter.
        self.w.write_all(line.as_bytes())?;
        self.w.write_all(b"\r")?;
        Ok(())
    }

    /// Run a shell command and wait for its marker; `$((…))` makes the marker appear only in the
    /// output, never in the echoed input.
    fn sh(&mut self, cmd: &str, done: &str, limit: Duration) -> Result<()> {
        let m = self.mark();
        self.send(cmd)?;
        self.wait_any(m, &[done], limit).map(|_| ())
    }
}

// ------------------------------------------------------------------------------------------ stage 1
fn stage1_seed(rt: &Runtime, w: &Work, progress: Progress) -> Result<()> {
    progress("stage 1: single-user BaseSystem, installing the builder daemon");
    let seed_img = w.p("seed.img");
    std::fs::write(&seed_img, seed::make_tar(&[
        ("wsm-build/stage1.sh", STAGE1, 0o755),
        ("wsm-build/build.sh", BUILD_SH, 0o755),
        ("wsm-build/com.wsm.build.plist", BUILD_PLIST, 0o644),
    ]))?;
    let mut vm = Vm::launch(rt, w, "seed", true, &[
        Disk { path: w.basesystem(), format: "qcow2", snapshot: false },
        Disk { path: seed_img, format: "raw", snapshot: true },
    ])?;
    vm.con.wait_any(0, &["-sh-3.2#", "root#"], Duration::from_secs(180))?;
    // Input typed before the AHCI probe finishes is echoed but Enter is lost (round 1 X2): retry a
    // no-op until it really runs.
    let t = Instant::now();
    loop {
        let m = vm.con.mark();
        vm.con.send("echo WSM-READY-$((6*7))")?;
        if vm.con.wait_any(m, &["WSM-READY-42"], Duration::from_secs(3)).is_ok() { break; }
        if t.elapsed() > Duration::from_secs(120) { bail!("single-user shell never ran a command"); }
    }
    vm.con.sh("mount -uw / && echo WSM-RW-$((1+1))", "WSM-RW-2", Duration::from_secs(60))?;
    vm.con.sh(
        "for d in /dev/rdisk[0-9]*; do case $d in *s[0-9]*) continue;; esac; \
         if dd if=$d bs=512 count=1 2>/dev/null | head -c 9 | grep -q '^wsm-build'; then tar -xf $d -C / && echo WSM-SEED-$((2+2)); fi; done",
        "WSM-SEED-4",
        Duration::from_secs(60),
    )?;
    vm.con.sh("sh /wsm-build/stage1.sh", "WSM-BUILD: stage1 done", Duration::from_secs(60))?;
    vm.con.send("halt")?;
    anyhow::ensure!(vm.wait_exit(Duration::from_secs(120))?, "BaseSystem did not halt");
    Ok(())
}

fn job_disk(w: &Work, name: &str, env: &str, payload: bool) -> Result<PathBuf> {
    let mut files: Vec<(String, &[u8], u32)> = vec![("wsm-job.env".into(), env.as_bytes(), 0o644)];
    if payload {
        for (n, b) in PAYLOAD { files.push((format!("payload/{n}"), b, 0o644)); }
    }
    let refs: Vec<(&str, &[u8], u32)> = files.iter().map(|(n, b, m)| (n.as_str(), *b, *m)).collect();
    let p = w.p(name);
    std::fs::write(&p, seed::make_tar(&refs))?;
    Ok(p)
}

// ------------------------------------------------------------------------------------------ stage 2
fn stage2_install(rt: &Runtime, w: &Work, iso: &Path, progress: Progress) -> Result<()> {
    progress("stage 2: startosinstall onto the target (the longest stage)");
    let job = job_disk(w, "job-install.img", &format!("MODE=install\nTARGET_BYTES={TARGET_BYTES}\n"), false)?;
    let mut vm = Vm::launch(rt, w, "install", false, &[
        Disk { path: w.basesystem(), format: "qcow2", snapshot: false },
        Disk { path: iso.to_path_buf(), format: "raw", snapshot: true },
        Disk { path: w.target(), format: "qcow2", snapshot: false },
        Disk { path: job, format: "raw", snapshot: true },
    ])?;
    vm.con.wait_any(0, &["WSM-BUILD: startosinstall begin"], Duration::from_secs(600))?;
    progress("  startosinstall running (prepare phase ~13 min)");
    let m = vm.con.mark();
    vm.con.wait_any(m, &["WSM-BUILD: startosinstall exit"], Duration::from_secs(45 * 60))?;
    let line = {
        let b = vm.con.buf.lock().unwrap();
        let t = String::from_utf8_lossy(&b[m..]).to_string();
        t.lines().find(|l| l.contains("WSM-BUILD: startosinstall exit")).unwrap_or("").to_string()
    };
    // startosinstall reboots the machine itself when it succeeds, so any exit line means it stopped.
    if !line.trim_end().ends_with("exit 0") {
        let tail = {
            let b = vm.con.buf.lock().unwrap();
            let t = String::from_utf8_lossy(&b[m..]).to_string();
            t.lines().rev().filter(|l| l.contains("osinstallersetupd") || l.contains("AHCI")).take(4).collect::<Vec<_>>().join("
  ")
        };
        bail!("{}
  last installer lines:
  {tail}
  full log: {}", line.trim(), w.p("install").join("serial.log").display());
    }
    progress("  installer phases on the target (autoinstall boot, then the installed system's first boot)");
    // After startosinstall's prepare phase the machine reboots into the installer's own environment
    // (boot-args carry `lca-boot-mode=autoinstall-msu`; it has a loginwindow too, which is why
    // "loginwindow" is NOT the end marker). The install is complete when a boot appears whose
    // kernel boot-args no longer say autoinstall.
    vm.con.wait_boot_without(m, "autoinstall", Duration::from_secs(90 * 60))?;
    progress("  installed system booted; letting its first boot settle");
    std::thread::sleep(Duration::from_secs(120));
    let mut q = wsm_core::qmp::Qmp::connect(&vm.qmp_sock, Duration::from_secs(5))?;
    q.execute("system_powerdown", serde_json::json!({}))?;
    if !vm.wait_exit(Duration::from_secs(120))? {
        progress("  ACPI shutdown was not handled; Recovery will replay APFS and verify the sealed snapshot before injection");
    }
    Ok(())
}

/// The final ramrod restore boot drops `autoinstall` but still boots an installer ramdisk.
fn is_installed_boot(line: &str) -> bool {
    line.contains("Kernel boot args:") && !["autoinstall", "-restore", "-rootdmg-ramdisk", "-bsdmgroot", "auth-root-dmg=", "lca-boot-mode="]
        .iter().any(|bad| line.contains(bad))
}

fn stage2_resume(rt: &Runtime, w: &Work, progress: Progress) -> Result<()> {
    progress("resuming installation from its saved NVRAM and existing target; target will not be erased");
    let vars = w.p("install").join("OVMF_VARS.fd");
    anyhow::ensure!(vars.exists(), "no install-stage NVRAM to resume");
    // No job disk or BaseSystem: an installer fallback must never run eraseDisk a second time.
    let mut vm = Vm::launch_impl(rt, w, "install-resume", false, &[
        Disk { path: w.target(), format: "qcow2", snapshot: false },
    ], Some(&vars))?;
    let start = Instant::now();
    loop {
        {
            let b = vm.con.buf.lock().unwrap();
            let text = String::from_utf8_lossy(&b);
            if text.lines().any(is_installed_boot) { break; }
            anyhow::ensure!(!text.contains("panic(cpu"), "guest panicked; see install-resume/serial.log");
        }
        anyhow::ensure!(start.elapsed() < Duration::from_secs(90 * 60), "resumed installer never reached the installed OS");
        std::thread::sleep(Duration::from_millis(500));
    }
    progress("installed OS booted after resume; waiting for initialization");
    std::thread::sleep(Duration::from_secs(120));
    let mut q = wsm_core::qmp::Qmp::connect(&vm.qmp_sock, Duration::from_secs(5))?;
    q.execute("system_powerdown", serde_json::json!({}))?;
    if !vm.wait_exit(Duration::from_secs(120))? {
        progress("  ACPI shutdown was not handled; Recovery will replay APFS and verify the sealed snapshot before injection");
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------ stage 3
fn stage3_inject(rt: &Runtime, w: &Work, clt: bool, progress: Progress) -> Result<()> {
    progress("stage 3: first-boot service, trim list and setup flags onto the installed Data volume");
    let job = job_disk(w, "job-inject.img", &format!("MODE=inject\nCLT={}\n", u8::from(clt)), true)?;
    let mut vm = Vm::launch(rt, w, "inject", false, &[
        Disk { path: w.basesystem(), format: "qcow2", snapshot: true },
        Disk { path: w.target(), format: "qcow2", snapshot: false },
        Disk { path: job, format: "raw", snapshot: true },
    ])?;
    vm.con.wait_any(0, &["WSM-BUILD: inject done"], Duration::from_secs(600))?;
    let grants = String::from_utf8_lossy(&vm.con.buf.lock().unwrap()).contains("WSM-BUILD: remote-login TCC grants: 4");
    anyhow::ensure!(grants, "remote-login privacy grants were not written; see inject/serial.log");
    anyhow::ensure!(vm.wait_exit(Duration::from_secs(120))?, "inject stage did not halt");
    Ok(())
}

// ------------------------------------------------------------------------------------------ stage 4
fn stage4_compact(home: &Path, rt: &Runtime, w: &Work, iso: &Path, clt: bool, progress: Progress) -> Result<PathBuf> {
    progress("stage 4: compacting");
    let tmp = w.p("base.qcow2");
    if tmp.exists() { std::fs::remove_file(&tmp).context("removing incomplete compacted output")?; }
    let mut measure = Command::new(rt.qemu_img());
    measure.args(["measure", "--output=json", "-O", "qcow2"]).arg(w.target());
    let measured = hidden(&mut measure).output()?;
    anyhow::ensure!(measured.status.success(), "cannot measure target allocation: {}", String::from_utf8_lossy(&measured.stderr));
    let measured: serde_json::Value = serde_json::from_slice(&measured.stdout)?;
    // The existing file can contain freed qcow2 clusters. Measure live allocation rather than its
    // high-water file length; compression cannot require more than the uncompressed output bound.
    let required_gib = measured["required"].as_u64().context("qemu-img measure omitted required bytes")?.div_ceil(1 << 30) + 2;
    let available_gib = wsm_core::hostinfo::free_disk_gb(&w.dir).context("cannot determine free disk space before compaction")?;
    anyhow::ensure!(available_gib >= required_gib,
        "not enough disk space for compaction: {available_gib} GiB free, conservatively require {required_gib} GiB; source target is preserved");
    qemu_img(rt, &["convert", "-q", "-c", "-O", "qcow2", &w.target().display().to_string(), &tmp.display().to_string()])?;
    progress("  sha256");
    let sum = crate::sha256_file(&tmp)?;
    let bases = wsm_core::paths::bases_dir(home);
    std::fs::create_dir_all(&bases)?;
    let out = bases.join(format!("base-tahoe-26.3-{}.qcow2", &sum[..12]));
    std::fs::rename(&tmp, &out)?;
    std::fs::write(
        out.with_extension("qcow2.sha256"),
        format!("{sum}  {}\n", out.file_name().unwrap().to_string_lossy()),
    )?;
    let manifest = serde_json::json!({
        "name": out.file_name().unwrap().to_string_lossy(),
        "sha256": sum,
        "bytes": std::fs::metadata(&out)?.len(),
        "os": "macOS Tahoe 26.3 (25D125)",
        "iso": iso.file_name().map(|f| f.to_string_lossy().to_string()),
        "iso_sha256": crate::sha256_file(iso)?,
        "generic": true,
        "clt": clt,
        "disabled_services": String::from_utf8_lossy(PAYLOAD.iter().find(|(n, _)| *n == "launchctl-disable.txt").unwrap().1)
            .lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).count(),
        "wsm_version": env!("CARGO_PKG_VERSION"),
    });
    std::fs::write(manifest_path(&out), serde_json::to_string_pretty(&manifest)?)?;
    let mut permissions = std::fs::metadata(&out)?.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&out, permissions)?;
    Ok(out)
}

// ------------------------------------------------------------------------------------------ lite base
// BaseSystem.dmg from the ISO, provisioned once in single-user mode, no macOS installer run. The data
// disk template (APFS WSMData + WSMLocal) is formatted by booting the finished base once.
const LITE_FILES: &[(&str, &[u8], u32)] = &[
    ("init.sh", include_bytes!("../../../guest/lite/init.sh"), 0o755),
    ("setup.sh", include_bytes!("../../../guest/lite/setup.sh"), 0o755),
    ("serial.sh", include_bytes!("../../../guest/lite/serial.sh"), 0o755),
    ("sshd.sh", include_bytes!("../../../guest/lite/sshd.sh"), 0o755),
    ("com.wsm.sshd.plist", include_bytes!("../../../guest/lite/com.wsm.sshd.plist"), 0o644),
    ("com.wsm.lite.plist", include_bytes!("../../../guest/lite/com.wsm.lite.plist"), 0o644),
    ("com.wsm.serial.plist", include_bytes!("../../../guest/lite/com.wsm.serial.plist"), 0o644),
    ("disable-daemons.txt", include_bytes!("../../../guest/lite/disable-daemons.txt"), 0o644),
    ("disable-agents.txt", include_bytes!("../../../guest/lite/disable-agents.txt"), 0o644),
];
const LITE_DATA_BYTES: u64 = 64 << 30;

/// Boots `base` (read-only, snapshot) once with a blank disk and the optional extras pack on the config disk;
/// init.sh formats WSMData + WSMLocal, unpacks the pack into WSMLocal and halts. Returns the template (uncompacted).
fn format_data_template(rt: &Runtime, w: &Work, base: &Path, extras: Option<&Path>) -> Result<PathBuf> {
    let data = w.p("data.qcow2");
    qemu_img(rt, &["create", "-q", "-f", "qcow2", &data.display().to_string(), &LITE_DATA_BYTES.to_string()])?;
    let cfg_img = w.p("config.img");
    let extras_tgz = match extras { Some(p) => Some(std::fs::read(p).with_context(|| format!("reading {}", p.display()))?), None => None };
    let mut cfg_files: Vec<(&str, &[u8], u32)> = vec![
        ("wsm-config.json", br#"{"user":"builder","hostname":"wsm-lite-build"}"#, 0o644),
        ("authorized_keys", b"", 0o600),
        ("format-data", b"1", 0o644),
    ];
    if let Some(x) = &extras_tgz { cfg_files.push(("extras.tgz", x, 0o644)); }
    std::fs::write(&cfg_img, seed::make_tar(&cfg_files))?;
    let mut vm = Vm::launch(rt, w, "format", false, &[
        Disk { path: base.to_path_buf(), format: "qcow2", snapshot: true },
        Disk { path: cfg_img, format: "raw", snapshot: true },
        Disk { path: data.clone(), format: "qcow2", snapshot: false },
    ])?;
    vm.con.wait_any(0, &["WSM-LITE: FORMATTED"], Duration::from_secs(300))?;
    if extras.is_some() {
        let ok = String::from_utf8_lossy(&vm.con.buf.lock().unwrap()).contains("WSM-LITE: extras installed");
        anyhow::ensure!(ok, "extras pack was not installed; see format/serial.log");
    }
    anyhow::ensure!(vm.wait_exit(Duration::from_secs(120))?, "format boot did not halt");
    Ok(data)
}

/// Rebuilds only the data-disk template of an existing lite base with a new extras pack (the base image is
/// untouched). The old template is kept as `<name>.old` until the caller removes it.
pub fn run_lite_data(home: &Path, rt: &Runtime, base: &Path, extras: &Path, progress: Progress) -> Result<PathBuf> {
    for p in [rt.qemu(), rt.qemu_img(), rt.ovmf_code(), rt.ovmf_vars(), base.to_path_buf(), extras.to_path_buf()] {
        anyhow::ensure!(p.exists(), "file missing: {}", p.display());
    }
    let w = Work { dir: home.join("build").join("lite-data") };
    if w.dir.exists() { std::fs::remove_dir_all(&w.dir).context("clearing the previous scratch")?; }
    std::fs::create_dir_all(&w.dir)?;
    progress("formatting the data-disk template with the new extras pack");
    let data = format_data_template(rt, &w, base, Some(extras))?;
    let tmp = w.p("data-c.qcow2");
    qemu_img(rt, &["convert", "-q", "-c", "-O", "qcow2", &data.display().to_string(), &tmp.display().to_string()])?;
    let dsum = crate::sha256_file(&tmp)?;
    let out = crate::data_template(base);
    let mut old = out.clone().into_os_string();
    old.push(".old");
    let old = PathBuf::from(old);
    for f in [&out, &out.with_extension("qcow2.sha256")] {
        if f.exists() {
            let dest = if f == &out { old.clone() } else { PathBuf::from(format!("{}.old", f.display())) };
            crate::set_readonly(f, false)?;
            std::fs::rename(f, dest)?;
        }
    }
    std::fs::copy(&tmp, &out)?;
    std::fs::write(out.with_extension("qcow2.sha256"), format!("{dsum}  {}
", out.file_name().unwrap().to_string_lossy()))?;
    let mp = manifest_path(base);
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&mp)?)?;
    m["data_sha256"] = dsum.into();
    m["data_bytes"] = std::fs::metadata(&out)?.len().into();
    m["extras_sha256"] = crate::sha256_file(extras)?.into();
    std::fs::write(&mp, serde_json::to_string_pretty(&m)?)?;
    crate::set_readonly(&out, true)?;
    Ok(out)
}

pub fn run_lite(home: &Path, rt: &Runtime, iso: &Path, extras: Option<&Path>, progress: Progress) -> Result<(PathBuf, PathBuf)> {
    anyhow::ensure!(iso.exists(), "ISO {} not found", iso.display());
    for p in [rt.qemu(), rt.qemu_img(), rt.seven_zip(), rt.ovmf_code(), rt.ovmf_vars()] {
        anyhow::ensure!(p.exists(), "runtime file missing: {}", p.display());
    }
    let w = Work { dir: home.join("build").join("lite") };
    if w.dir.exists() { std::fs::remove_dir_all(&w.dir).context("clearing the previous lite build scratch")?; }
    std::fs::create_dir_all(&w.dir)?;
    // L1: BaseSystem.dmg -> qcow2
    progress("lite 1/4: extracting BaseSystem.dmg from the ISO");
    let dmg = w.p("BaseSystem.dmg");
    let mut c = Command::new(rt.seven_zip());
    c.args(["e", "-y"]).arg(format!("-o{}", w.dir.display())).arg(iso).arg(ISO_BASESYSTEM).stdout(Stdio::null());
    let st = hidden(&mut c).status()?;
    anyhow::ensure!(st.success() && dmg.exists(), "7-Zip could not extract {ISO_BASESYSTEM}");
    qemu_img(rt, &["convert", "-q", "-f", "dmg", "-O", "qcow2", &dmg.display().to_string(), &w.basesystem().display().to_string()])?;
    std::fs::remove_file(&dmg)?;
    // L2a: provision in single-user mode from a raw tar seed disk
    progress("lite 2/4: provisioning BaseSystem (single-user, seed disk)");
    let files: Vec<(String, &[u8], u32)> = LITE_FILES.iter().map(|(n, b, m)| (format!("wsm-lite/{n}"), *b, *m)).collect();
    let refs: Vec<(&str, &[u8], u32)> = files.iter().map(|(n, b, m)| (n.as_str(), *b, *m)).collect();
    let seed_img = w.p("seed.img");
    std::fs::write(&seed_img, seed::make_tar(&refs))?;
    {
        let mut vm = Vm::launch(rt, &w, "seed", true, &[
            Disk { path: w.basesystem(), format: "qcow2", snapshot: false },
            Disk { path: seed_img, format: "raw", snapshot: true },
        ])?;
        vm.con.wait_any(0, &["-sh-3.2#", "root#"], Duration::from_secs(180))?;
        let t = Instant::now();
        loop {
            let m = vm.con.mark();
            vm.con.send("echo WSM-READY-$((6*7))")?;
            if vm.con.wait_any(m, &["WSM-READY-42"], Duration::from_secs(3)).is_ok() { break; }
            if t.elapsed() > Duration::from_secs(120) { bail!("single-user shell never ran a command"); }
        }
        vm.con.sh("mount -uw / && echo WSM-RW-$((1+1))", "WSM-RW-2", Duration::from_secs(60))?;
        vm.con.sh(
            "for d in /dev/rdisk[0-9]*; do case $d in *s[0-9]*) continue;; esac;              if dd if=$d bs=512 count=1 2>/dev/null | head -c 9 | grep -q '^wsm-lite'; then tar -xf $d -C / && echo WSM-SEED-$((2+2)); fi; done",
            "WSM-SEED-4", Duration::from_secs(60))?;
        vm.con.sh("sh /wsm-lite/setup.sh", "WSM-LITE-SETUP-DONE", Duration::from_secs(120))?;
        vm.con.send("halt")?;
        anyhow::ensure!(vm.wait_exit(Duration::from_secs(120))?, "BaseSystem did not halt");
    }
    // L2b: boot the finished base once with a blank disk to make the data-disk template
    progress("lite 3/4: formatting the data-disk template (WSMData, WSMLocal)");
    let data = format_data_template(rt, &w, &w.basesystem(), extras)?;
    // L2c: compact into bases/
    progress("lite 4/4: compacting");
    let tmp_base = w.p("base-c.qcow2");
    let tmp_data = w.p("data-c.qcow2");
    qemu_img(rt, &["convert", "-q", "-c", "-O", "qcow2", &w.basesystem().display().to_string(), &tmp_base.display().to_string()])?;
    qemu_img(rt, &["convert", "-q", "-c", "-O", "qcow2", &data.display().to_string(), &tmp_data.display().to_string()])?;
    let sum = crate::sha256_file(&tmp_base)?;
    let dsum = crate::sha256_file(&tmp_data)?;
    let bases = wsm_core::paths::bases_dir(home);
    std::fs::create_dir_all(&bases)?;
    let out = bases.join(format!("base-lite-tahoe-26.3-{}.qcow2", &sum[..12]));
    let out_data = bases.join(format!("base-lite-tahoe-26.3-{}-data.qcow2", &sum[..12]));
    std::fs::rename(&tmp_base, &out)?;
    std::fs::rename(&tmp_data, &out_data)?;
    for (f, h) in [(&out, &sum), (&out_data, &dsum)] {
        std::fs::write(f.with_extension("qcow2.sha256"), format!("{h}  {}
", f.file_name().unwrap().to_string_lossy()))?;
    }
    let manifest = serde_json::json!({
        "name": out.file_name().unwrap().to_string_lossy(), "kind": "lite", "sha256": sum,
        "bytes": std::fs::metadata(&out)?.len(), "data_template": out_data.file_name().unwrap().to_string_lossy(),
        "data_sha256": dsum, "data_bytes": std::fs::metadata(&out_data)?.len(),
        "os": "macOS Tahoe 26.3 BaseSystem (no installer run)", "iso": iso.file_name().map(|f| f.to_string_lossy().to_string()),
        "iso_sha256": crate::sha256_file(iso)?, "generic": true,
        "extras_sha256": match extras { Some(p) => Some(crate::sha256_file(p)?), None => None }, "wsm_version": env!("CARGO_PKG_VERSION"),
    });
    std::fs::write(manifest_path(&out), serde_json::to_string_pretty(&manifest)?)?;
    for f in [&out, &out_data] {
        let mut permissions = std::fs::metadata(f)?.permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(f, permissions)?;
    }
    Ok((out, out_data))
}

#[cfg(test)]
mod tests {
    use super::is_installed_boot;

    #[test]
    fn excludes_all_observed_installer_boot_modes() {
        for args in ["lca-boot-mode=autoinstall-msu", "-restore -rootdmg-ramdisk auth-root-dmg=file:///com.apple.installer/x86_64SURamDisk.dmg", "-bsdmgroot"] {
            assert!(!is_installed_boot(&format!("Kernel boot args: '-v keepsyms=1 {args}'")), "{args}");
        }
        assert!(is_installed_boot("Kernel boot args: '-v keepsyms=1 serial=3 debug=0x108 '"));
        assert!(!is_installed_boot("loginwindow started inside Recovery"));
    }
}
