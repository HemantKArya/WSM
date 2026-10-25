//! `wsm`: Windows Subsystem for macOS command line.
mod console;
mod daemon;
mod doctor;
mod jobs;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use wsm_core::paths::{self, InstanceDirs};
use wsm_core::manifest::Manifest;
use wsm_core::ssh::{self, Target};

#[derive(Parser)]
#[command(name = "wsm", version, about = "Windows Subsystem for macOS")]
struct Cli {
    /// Instance name
    #[arg(long, short = 'n', global = true, default_value = "default")]
    name: String,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Attach a shell (serial console first, SSH once available)
    Shell {
        /// Stay on the serial console even when SSH is available
        #[arg(long)]
        serial: bool,
        /// Wait for SSH and use it only
        #[arg(long)]
        ssh: bool,
    },
    /// Run a command in the guest; returns its exit code
    Exec {
        /// Run as root (lite bases: key-only root login over SSH, in place of sudo)
        #[arg(long)]
        root: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        argv: Vec<String>,
    },
    /// Run a command in the guest; --detach keeps it running after the session ends (see `wsm jobs`)
    Run {
        #[arg(long)]
        detach: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        argv: Vec<String>,
    },
    /// List detached jobs (`wsm run --detach`)
    Jobs,
    /// Stop a detached job
    Kill { id: String },
    /// Start (resume if saved state exists, else cold boot)
    Start {
        /// Ignore saved state
        #[arg(long)]
        cold: bool,
        /// Return as soon as QEMU is up instead of waiting for SSH
        #[arg(long)]
        no_wait: bool,
    },
    /// Suspend to file (default), or --shutdown / --kill
    Stop {
        #[arg(long)]
        shutdown: bool,
        #[arg(long)]
        kill: bool,
    },
    /// Suspend to file
    Suspend,
    /// Resume a suspended instance (same as start)
    Resume,
    /// Start or stop the GUI session (GPU instances)
    Gui {
        #[arg(default_value = "start", value_parser = ["start", "stop", "logout", "status", "prewarm"])]
        action: String,
    },
    Status,
    /// List instances and their state
    List,
    /// Show instance logs
    Logs {
        #[arg(long, default_value = "qemu.err.log")]
        file: String,
        #[arg(long, default_value_t = 60)]
        tail: usize,
    },
    /// Create an instance from a base image
    Create {
        #[arg(long)]
        gpu: bool,
        #[arg(long)]
        base: Option<PathBuf>,
        #[arg(long)]
        ram: Option<u32>,
        #[arg(long)]
        vcpus: Option<u32>,
    },
    Delete,
    /// Check the host and runtime
    Doctor {
        /// Apply the per-user GPU preference if it is missing
        #[arg(long)]
        fix: bool,
    },
    /// Copy files; prefix guest paths with ':' (wsm cp :/tmp/a.txt .)
    Cp {
        src: String,
        dst: String,
        #[arg(short, long)]
        recursive: bool,
    },
    /// Map the guest home to a drive letter (built-in Windows WebDAV client; no install needed)
    Mount {
        #[arg(default_value = "W:")]
        drive: String,
        /// Use SSHFS-Win/WinFsp instead, if installed
        #[arg(long)]
        sshfs: bool,
    },
    Unmount {
        #[arg(default_value = "W:")]
        drive: String,
    },
    /// Make a Windows folder available in the guest at ~/win/<label> (WebDAV over 10.0.2.2)
    Share {
        folder: PathBuf,
        /// Folder name in the guest (default: the Windows folder's name)
        #[arg(long = "as")]
        label: Option<String>,
    },
    /// Unmount ~/win/<label> in the guest
    Unshare { label: String },
    /// Base image management
    #[command(subcommand)]
    Base(BaseCmd),
    /// Remove %LOCALAPPDATA%\wsm and the per-user GPU preference
    Uninstall {
        #[arg(long)]
        yes: bool,
        /// Remove only the portable runtime; leave Windows GPU preferences untouched
        #[arg(long)]
        portable: bool,
    },
    /// Stop the background service
    Quit,
}

#[derive(Subcommand)]
enum BaseCmd {
    /// Build the light core base from the installer's BaseSystem (no macOS install, minutes)
    BuildLite {
        #[arg(long)]
        iso: PathBuf,
        /// Optional tools pack (tar.gz of bin/, libexec/, share/) unpacked into every instance's /usr/local
        #[arg(long)]
        extras: Option<PathBuf>,
    },
    /// Rebuild only the data-disk template of a lite base with a new tools pack (the base image is untouched)
    LiteData {
        base: PathBuf,
        #[arg(long)]
        extras: PathBuf,
    },
    /// Build a generic base from the Tahoe installer
    Build {
        #[arg(long)]
        iso: PathBuf,
        #[arg(long)]
        clt: bool,
        /// Restart at this stage (0 extract, 1 seed, 2 install, 3 inject, 4 compact); earlier
        /// stages' results in the work dir are reused
        #[arg(long, default_value_t = 0)]
        from_stage: u8,
        /// Continue interrupted installer phases on the existing target without erasing it
        #[arg(long, requires = "from_stage")]
        resume_install: bool,
    },
    Import {
        src: String,
        #[arg(long)]
        sha256: Option<String>,
    },
    Export {
        dest: PathBuf,
        #[arg(long)]
        base: Option<PathBuf>,
    },
    List,
}

fn main() {
    wsm_core::hostinfo::private_standard_handles();
    let code = match run() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("wsm: {e:#}");
            1
        }
    };
    std::process::exit(code);
}

fn progress(m: &str) { eprintln!("wsm: {m}"); }

fn rpc(cmd: &str, name: &str, args: Value) -> Result<Value> {
    let home = paths::home()?;
    let info = daemon::ensure(&home)?;
    let r = wsm_core::proto::call(&info, cmd, name, args, progress)?;
    if r.ok == Some(true) { Ok(r.data) } else { bail!("{}", r.msg) }
}

fn target_for(name: &str) -> Result<(Manifest, PathBuf)> {
    let home = paths::home()?;
    let d = InstanceDirs::new(&home, name);
    let m = Manifest::load(&d.manifest()).with_context(|| format!("instance '{name}' not found; run `wsm create`"))?;
    Ok((m, d.keys().join("id_ed25519")))
}

fn ensure_running(name: &str, wait_ssh: bool) -> Result<Value> {
    rpc("start", name, json!({"wait": if wait_ssh { "ssh" } else { "none" }}))
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    let name = cli.name.as_str();
    match cli.cmd.unwrap_or(Cmd::Shell { serial: false, ssh: false }) {
        Cmd::Shell { serial, ssh: ssh_only } => shell(name, serial, ssh_only),
        Cmd::Exec { root, argv } => {
            pooled_exec(name, argv, root)
        }
        Cmd::Run { detach, mut argv } => {
            if !detach {
                return pooled_exec(name, argv, false);
            }
            if let [one] = argv.as_slice() {
                argv = vec!["sh".into(), "-c".into(), one.clone()];
            }
            pooled_exec(name, vec![jobs::start_script(&argv)], false)
        }
        Cmd::Jobs => pooled_exec(name, vec![jobs::list_script()], false),
        Cmd::Kill { id } => pooled_exec(name, vec![jobs::kill_script(&id).map_err(anyhow::Error::msg)?], false),
        Cmd::Start { cold, no_wait } => {
            let t = Instant::now();
            let r = rpc("start", name, json!({"wait": if no_wait { "none" } else { "ssh" }, "cold": cold}))?;
            println!("{} in {:.1}s ({})", name, t.elapsed().as_secs_f64(), r["mode"].as_str().unwrap_or("?"));
            Ok(0)
        }
        Cmd::Resume => { rpc("start", name, json!({"wait": "ssh"}))?; Ok(0) }
        Cmd::Stop { shutdown, kill } => {
            if kill {
                rpc("kill", name, json!({}))?;
            } else if shutdown {
                let r = rpc("shutdown", name, json!({}))?;
                println!("shut down ({} s)", r["s"]);
            } else {
                print_suspend(&rpc("suspend", name, json!({}))?);
            }
            Ok(0)
        }
        Cmd::Suspend => {
            print_suspend(&rpc("suspend", name, json!({}))?);
            Ok(0)
        }
        Cmd::Gui { action } => {
            ensure_running(name, true)?;
            let r = rpc("gui", name, json!({"action": action}))?;
            print!("{}", r["out"].as_str().unwrap_or(""));
            Ok(0)
        }
        Cmd::Status => {
            let r = rpc("status", name, json!({}))?;
            println!("{}", serde_json::to_string_pretty(&r)?);
            Ok(0)
        }
        Cmd::List => {
            let names = rpc("list", "", json!({}))?;
            for n in names.as_array().cloned().unwrap_or_default() {
                let n = n.as_str().unwrap_or_default();
                let st = rpc("status", n, json!({"probe": false}))?;
                println!("{n:<16} {:<10} {}{}", st["state"].as_str().unwrap_or("?"),
                    if st["gpu"] == true { "gpu " } else { "" }, format_args!("{} MiB, {} vCPU", st["ram_mib"], st["vcpus"]));
            }
            Ok(0)
        }
        Cmd::Logs { file, tail } => {
            let d = InstanceDirs::new(&paths::home()?, name);
            let s = std::fs::read_to_string(d.logs().join(&file)).with_context(|| format!("no log {file}"))?;
            let lines: Vec<_> = s.lines().collect();
            for l in &lines[lines.len().saturating_sub(tail)..] { println!("{l}"); }
            Ok(0)
        }
        Cmd::Create { gpu, base, ram, vcpus } => {
            let r = rpc("create", name, json!({"gpu": gpu, "base": base.map(|b| b.display().to_string()), "ram": ram, "vcpus": vcpus}))?;
            println!("created '{name}' (ssh port {})", r["ssh_port"]);
            Ok(0)
        }
        Cmd::Delete => { rpc("delete", name, json!({}))?; Ok(0) }
        Cmd::Doctor { fix } => doctor::run(fix),
        Cmd::Cp { src, dst, recursive } => {
            ensure_running(name, true)?;
            let (m, key) = target_for(name)?;
            ssh::copy(&Target { user: &m.user, port: m.ssh_port, key: &key }, &src, &dst, recursive)
        }
        Cmd::Mount { drive, sshfs } => if sshfs { mount_sshfs(name, &drive) } else { mount_dav(name, &drive) },
        Cmd::Unmount { drive } => {
            let st = std::process::Command::new("net").args(["use", &drive, "/delete", "/y"]).status()?;
            Ok(st.code().unwrap_or(1))
        }
        Cmd::Share { folder, label } => share(name, &folder, label),
        Cmd::Unshare { label } => {
            let label = guest_label(&label);
            pooled_exec(name, vec![format!("umount \"$HOME/win/{label}\" && rmdir \"$HOME/win/{label}\"")], false)
        }
        Cmd::Base(b) => base(b),
        Cmd::Uninstall { yes, portable } => uninstall(yes, portable),
        Cmd::Quit => {
            let home = paths::home()?;
            if let Ok(info) = wsm_core::proto::read_info(&home) { let _ = wsm_core::proto::call(&info, "quit", "", json!({}), |_| {}); }
            Ok(0)
        }
    }
}

fn pooled_exec(name: &str, argv: Vec<String>, root: bool) -> Result<i32> {
    use std::io::{BufRead, BufReader, Read, Write};
    let home = paths::home()?;
    let info = daemon::ensure(&home)?;
    let stream = wsm_core::proto::open_request(&info, "exec", name, json!({"argv": argv, "root": root}))?;
    let mut input = stream.try_clone()?;
    std::thread::spawn(move || {
        let mut buf = [0u8; 32768];
        let mut stdin = std::io::stdin();
        loop {
            let Ok(n) = stdin.read(&mut buf) else { return; };
            let frame = json!({"data": &buf[..n], "eof": n == 0});
            if writeln!(input, "{frame}").is_err() || n == 0 { return; }
        }
    });
    for line in BufReader::new(stream).lines() {
        let r: wsm_core::proto::Response = serde_json::from_str(&line?)?;
        if let Some(p) = r.progress { progress(&p); }
        if let Some(bytes) = r.stdout { let mut out = std::io::stdout(); out.write_all(&bytes)?; out.flush()?; }
        if let Some(bytes) = r.stderr { let mut err = std::io::stderr(); err.write_all(&bytes)?; err.flush()?; }
        if r.ok == Some(false) { bail!("{}", r.msg); }
        if let Some(code) = r.exit_code { return Ok(code); }
    }
    bail!("wsmd disconnected before reporting the command exit code")
}

fn shell(name: &str, serial_only: bool, ssh_only: bool) -> Result<i32> {
    let t0 = Instant::now();
    // Like WSL's first launch: the default instance is created on first use when a base exists.
    let home = paths::home()?;
    if name == "default" && !InstanceDirs::new(&home, name).manifest().exists() {
        let base = newest(&home).context("no instance and no base image yet: run `wsm base import FILE` or `wsm base build --iso FILE`")?;
        progress(&format!("creating instance 'default' from {}", base.display()));
        rpc("create", name, json!({"base": base.display().to_string()}))?;
    }
    let st = rpc("status", name, json!({"probe": false}))?;
    if st["state"] != "running" {
        let word = if st["state"] == "suspended" { "resuming" } else { "booting" };
        progress(word);
        // Resume must finish before anything can be typed; cold boot returns when QEMU is up.
        let wait = if st["state"] == "suspended" || ssh_only { "ssh" } else { "none" };
        let r = rpc("start", name, json!({"wait": wait}))?;
        progress(&format!("{} in {:.1}s", r["mode"].as_str().unwrap_or("?"), t0.elapsed().as_secs_f64()));
    }
    let (m, key) = target_for(name)?;
    let _ = rpc("lease", name, json!({"delta": 1}));
    let tgt = Target { user: &m.user, port: m.ssh_port, key: &key };
    let rc = (|| -> Result<i32> {
        if !serial_only && (ssh_only || ssh::ready(&tgt)) {
            while !ssh::ready(&tgt) {
                if t0.elapsed() > Duration::from_secs(180) { bail!("SSH did not come up"); }
                std::thread::sleep(Duration::from_millis(300));
            }
            return ssh::exec_inherit(&tgt, &[], true);
        }
        progress("serial console (Ctrl-] to detach); switching to SSH when it is up");
        let info = daemon::ensure(&paths::home()?)?;
        let stream = wsm_core::proto::open_stream(&info, "serial", name)?;
        let switch = (!serial_only).then(|| (m.user.clone(), m.ssh_port, key.clone()));
        let switched = console::attach(stream, switch)?;
        if switched {
            progress("SSH is up, switching");
            return ssh::exec_inherit(&tgt, &[], true);
        }
        Ok(0)
    })();
    let _ = rpc("lease", name, json!({"delta": -1}));
    rc
}

/// wsmd serves the guest home over WebDAV on loopback under a random token; Windows' own WebDAV
/// redirector (WebClient service, started on demand for a normal user) maps it. Files above
/// WebClient's download limit (50 MB by default) need `wsm cp`.
fn mount_dav(name: &str, drive: &str) -> Result<i32> {
    ensure_running(name, true)?;
    let r = rpc("mount", name, json!({}))?;
    let unc = r["unc"].as_str().context("wsmd returned no share path")?;
    let st = std::process::Command::new("net").args(["use", drive, unc, "/persistent:no"]).status()?;
    if st.success() {
        println!("{drive} -> {name}:~ (WebDAV via wsmd; `wsm unmount {drive}` to remove)");
    }
    Ok(st.code().unwrap_or(1))
}

/// Letters, digits, '.', '_' and '-' only: the label becomes a guest path inside a shell command.
fn guest_label(s: &str) -> String {
    let l: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
    let l = l.trim_matches('.').to_string();
    if l.is_empty() { "share".into() } else { l }
}

/// wsmd serves the folder on the host's loopback; QEMU's user network shows that as 10.0.2.2 in the
/// guest, where macOS' own mount_webdav mounts it (the base grants remote-login sessions network
/// volume access). mount_webdav's front end can linger after mounting, so it runs detached and the
/// mount table is polled.
fn share(name: &str, folder: &std::path::Path, label: Option<String>) -> Result<i32> {
    let folder = std::fs::canonicalize(folder).with_context(|| format!("{}", folder.display()))?;
    let label = guest_label(&label.unwrap_or_else(|| folder.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()));
    ensure_running(name, true)?;
    let r = rpc("share", name, json!({"folder": folder.display().to_string()}))?;
    let url = r["url"].as_str().context("wsmd returned no share URL")?;
    let script = format!(r#"d="$HOME/win/{label}"; mkdir -p "$d"
if ! mount | grep -qF " on $d ("; then
    nohup /sbin/mount_webdav -v {label} {url} "$d" >/dev/null 2>&1 </dev/null &
    i=0; while [ $i -lt 75 ] && ! mount | grep -qF " on $d ("; do sleep 0.2; i=$((i+1)); done
fi
mount | grep -qF " on $d (" || {{ echo "mount_webdav did not mount $d" >&2; exit 1; }}
ls "$d" >/dev/null || exit 1
echo "{} -> $d""#, folder.display().to_string().replace(['"', '$', '`', '\\'], "/"));
    pooled_exec(name, vec![script], false)
}

fn mount_sshfs(name: &str, drive: &str) -> Result<i32> {
    if !wsm_core::hostinfo::winfsp_installed() {
        bail!("WinFsp/SSHFS-Win not found. Install WinFsp and SSHFS-Win (https://winfsp.dev, https://github.com/winfsp/sshfs-win), then retry. `wsm cp` works without them.");
    }
    ensure_running(name, true)?;
    let (m, key) = target_for(name)?;
    // SSHFS-Win's `\\sshfs.k\` form always authenticates with %USERPROFILE%\.ssh\id_rsa, not the
    // instance key: authorise that key's public half in the guest first.
    let home = std::env::var_os("USERPROFILE").context("USERPROFILE is not set")?;
    let pubkey = std::path::Path::new(&home).join(".ssh").join("id_rsa.pub");
    let pk = std::fs::read_to_string(&pubkey).with_context(|| {
        format!("SSHFS-Win's key mode needs {} (create it with `ssh-keygen -t rsa`)", pubkey.display())
    })?;
    let pk = pk.trim();
    anyhow::ensure!(pk.starts_with("ssh-") && !pk.contains('\''), "unexpected public key format in {}", pubkey.display());
    let add = format!("mkdir -p ~/.ssh && touch ~/.ssh/authorized_keys && (grep -qxF '{pk}' ~/.ssh/authorized_keys || echo '{pk}' >> ~/.ssh/authorized_keys)");
    let o = ssh::exec(&Target { user: &m.user, port: m.ssh_port, key: &key }, &[add])?;
    anyhow::ensure!(o.status.success(), "could not authorise {} in the guest", pubkey.display());
    let unc = format!(r"\\sshfs.k\{}@127.0.0.1!{}", m.user, m.ssh_port);
    let st = std::process::Command::new("net").args(["use", drive, &unc]).status()?;
    Ok(st.code().unwrap_or(1))
}

fn base(b: BaseCmd) -> Result<i32> {
    let home = paths::home()?;
    let rt = wsm_core::provider::Runtime::new(paths::runtime_dir(&home));
    match b {
        BaseCmd::Import { src, sha256 } => {
            let p = wsm_builder::import(&src, &paths::bases_dir(&home), sha256.as_deref(), &rt.qemu_img())?;
            println!("imported {}", p.display());
        }
        BaseCmd::Export { dest, base } => {
            let b = match base { Some(b) => b, None => newest(&home)? };
            let sum = wsm_builder::export(&b, &dest)?;
            println!("exported {} (sha256 {sum})", dest.display());
        }
        BaseCmd::List => {
            for e in std::fs::read_dir(paths::bases_dir(&home)).into_iter().flatten().flatten() {
                if e.path().extension().is_some_and(|x| x == "qcow2") {
                    println!("{}  {} MB", e.path().display(), e.metadata()?.len() / 1_000_000);
                }
            }
        }
        BaseCmd::BuildLite { iso, extras } => {
            let (b, d) = wsm_builder::build::run_lite(&home, &rt, &iso, extras.as_deref(), &mut |m| eprintln!("wsm: {m}"))?;
            println!("built {}
built {}", b.display(), d.display());
        }
        BaseCmd::LiteData { base, extras } => {
            let d = wsm_builder::build::run_lite_data(&home, &rt, &base, &extras, &mut |m| eprintln!("wsm: {m}"))?;
            println!("rebuilt {} (previous template kept as .old)", d.display());
        }
        BaseCmd::Build { iso, clt, from_stage, resume_install } => {
            anyhow::ensure!(!resume_install || from_stage == 2, "--resume-install requires --from-stage 2");
            let opts = wsm_builder::build::Opts { iso, clt, from_stage, resume_install };
            let p = wsm_builder::build::run(&home, &rt, &opts, &mut |m| eprintln!("wsm: {m}"))?;
            println!("built {}", p.display());
        }
    }
    Ok(0)
}

fn newest(home: &std::path::Path) -> Result<PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(paths::bases_dir(home))?.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "qcow2") && !e.file_name().to_string_lossy().ends_with("-data.qcow2")).collect();
    v.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    v.last().map(|e| e.path()).context("no base image installed")
}

fn uninstall(yes: bool, portable: bool) -> Result<i32> {
    let home = paths::home()?;
    if !yes {
        bail!("this deletes {} including all instances and bases; re-run with --yes", home.display());
    }
    let marker = home.join("package-manifest.json");
    let marker: Value = serde_json::from_str(&std::fs::read_to_string(&marker)
        .context("automatic uninstall requires a packaged runtime with package-manifest.json")?)?;
    anyhow::ensure!(marker["files"]["wsm.exe"].is_object(), "runtime package marker is invalid");
    let rt = wsm_core::provider::Runtime::new(paths::runtime_dir(&home));
    if let Ok(info) = wsm_core::proto::read_info(&home) {
        if let Ok(list) = wsm_core::proto::call(&info, "list", "", json!({}), |_| {}) {
            for n in list.data.as_array().cloned().unwrap_or_default() {
                let n = n.as_str().unwrap_or("");
                let _ = wsm_core::proto::call(&info, "kill", n, json!({}), |_| {});
            }
        }
        let _ = wsm_core::proto::call(&info, "quit", "", json!({}), |_| {});
        std::thread::sleep(Duration::from_millis(500));
    }
    if !portable {
        wsm_core::hostinfo::remove_gpu_preference(&rt.qemu());
        remove_from_user_path(&home);
    }
    if std::env::current_exe()?.canonicalize()?.starts_with(home.canonicalize()?) {
        // Windows cannot unlink the running executable. A hidden helper waits for this CLI to exit.
        let temp = std::env::temp_dir();
        let helper = temp.join(format!("wsm-uninstall-{}.ps1", std::process::id()));
        std::fs::write(&helper, include_str!("../../../src/build/uninstall.ps1"))?;
        let mut command = std::process::Command::new("powershell.exe");
        command.args(["-NoProfile", "-WindowStyle", "Hidden", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&helper).arg("-Runtime").arg(&home).arg("-LauncherPid").arg(std::process::id().to_string())
            .current_dir(&temp).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        ssh::hidden(&mut command).spawn().context("starting deferred runtime cleanup")?;
        println!("removing {} after this command exits", home.display());
    } else {
        std::fs::remove_dir_all(&home).context("removing runtime dir")?;
        println!("removed {}", home.display());
    }
    Ok(0)
}

/// Undo install.ps1's per-user PATH entry (the only PATH change it makes).
fn remove_from_user_path(dir: &std::path::Path) {
    let script = r"$d=[IO.Path]::GetFullPath($env:WSM_UNINSTALL_DIR).TrimEnd('\');
        $p=[Environment]::GetEnvironmentVariable('Path','User');
        if ($p) {
            $n=($p -split ';' | Where-Object { $_ -and ([IO.Path]::GetFullPath($_).TrimEnd('\') -ne $d) }) -join ';';
            if ($n -ne $p) { [Environment]::SetEnvironmentVariable('Path',$n,'User') }
        }";
    let mut c = std::process::Command::new("powershell.exe");
    c.args(["-NoProfile", "-Command", script]).env("WSM_UNINSTALL_DIR", dir);
    let _ = ssh::hidden(&mut c).status();
}

/// GPU instances can't be suspended; wsmd shuts them down instead and says so in `mode`.
fn print_suspend(r: &serde_json::Value) {
    let s = r["save_s"].as_f64().unwrap_or(0.0);
    if r["mode"] == "shutdown" {
        println!("shut down: {s:.1}s (GPU instances can't be suspended; next start cold-boots)");
    } else {
        let st = &r["stats"];
        let f = |k: &str| st[k].as_f64().unwrap_or(0.0);
        println!(
            "suspended: {s:.1}s, {} MB (migrate {:.1}s, drain {:.1}s, fsync {:.1}s; QEMU exit {:.1}s)",
            r["bytes"].as_u64().unwrap_or(0) / 1_000_000, f("migrate_s"), f("drain_s"), f("sync_s"),
            r["quit_s"].as_f64().unwrap_or(0.0)
        );
    }
}
