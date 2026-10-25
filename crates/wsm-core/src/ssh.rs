use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Never flash a console window: the daemon has none, so every child would open its own.
pub fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// OpenSSH client shipped with Windows 11 (System32\OpenSSH), else PATH.
pub fn ssh_exe(name: &str) -> PathBuf {
    let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
    let p = sys.join("System32").join("OpenSSH").join(format!("{name}.exe"));
    if p.exists() { p } else { PathBuf::from(name) }
}

pub struct Target<'a> {
    pub user: &'a str,
    pub port: u16,
    pub key: &'a Path,
}

fn common(t: &Target, port_flag: &str) -> Vec<String> {
    vec![
        "-i".into(), t.key.display().to_string(), port_flag.into(), t.port.to_string(),
        "-o".into(), "StrictHostKeyChecking=no".into(), "-o".into(), "UserKnownHostsFile=NUL".into(),
        "-o".into(), "LogLevel=ERROR".into(), "-o".into(), "BatchMode=yes".into(),
        "-o".into(), "ConnectTimeout=10".into(),
    ]
}

pub fn exec(t: &Target, argv: &[String]) -> Result<Output> {
    let mut c = Command::new(ssh_exe("ssh"));
    c.args(common(t, "-p")).arg(format!("{}@127.0.0.1", t.user)).arg(shell_join(argv)).stdin(Stdio::null());
    hidden(&mut c).output().context("running ssh")
}

/// Run with inherited stdio; returns the remote exit code.
pub fn exec_inherit(t: &Target, argv: &[String], tty: bool) -> Result<i32> {
    let mut cmd = Command::new(ssh_exe("ssh"));
    cmd.args(common(t, "-p"));
    if tty {
        cmd.arg("-t");
    }
    cmd.arg(format!("{}@127.0.0.1", t.user));
    if !argv.is_empty() {
        cmd.arg(shell_join(argv));
    }
    Ok(cmd.status().context("running ssh")?.code().unwrap_or(255))
}

/// `scp`; a path starting with `:` is on the guest.
pub fn copy(t: &Target, src: &str, dst: &str, recursive: bool) -> Result<i32> {
    let mut cmd = Command::new(ssh_exe("scp"));
    cmd.args(common(t, "-P"));
    if recursive {
        cmd.arg("-r");
    }
    let fix = |s: &str| {
        s.strip_prefix(':').map(|r| format!("{}@127.0.0.1:{r}", t.user)).unwrap_or_else(|| s.to_string())
    };
    cmd.arg(fix(src)).arg(fix(dst));
    Ok(cmd.status().context("running scp")?.code().unwrap_or(255))
}

/// A single argument is a shell snippet (like `ssh host 'a | b'`); several are quoted word by word.
pub fn shell_join(argv: &[String]) -> String {
    if let [one] = argv {
        return one.clone();
    }
    argv.iter()
        .map(|a| {
            if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+".contains(c)) {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One readiness probe, bounded in time. QEMU's user-mode network accepts the forwarded TCP connection
/// before the guest's sshd listens, so ssh then waits for a banner that may never come;
/// `ConnectTimeout` has already been satisfied by then. Without this bound a single probe during boot
/// or resume could block `wsm start` indefinitely (seen: 3 and 5 min).
/// On a generic base SSH accepts the user's key before com.wsm.firstboot has finished (sudoers, sshd
/// config, user trim), so readiness also requires its completion marker. Images without that service
/// (round-2 bases) only need a login.
pub const READY_PROBE: &str = "if [ -f /Library/LaunchDaemons/com.wsm.firstboot.plist ]; then \
    sudo -n test -f /var/db/wsm/firstboot.done; else true; fi";

pub fn ready(t: &Target) -> bool {
    // Cheap gate first: until the guest's sshd is reachable, QEMU holds the accepted connection open
    // with no banner, and a full ssh probe would sit there until its 8 s kill (measured: cold boot
    // reported SSH ~25 s). A short banner read retries every ~1.5 s instead.
    if !banner(t.port) { return false; }
    let mut c = Command::new(ssh_exe("ssh"));
    c.args(common(t, "-p")).arg(format!("{}@127.0.0.1", t.user)).arg(READY_PROBE)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let Ok(mut child) = hidden(&mut c).spawn() else { return false };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return st.success(),
            Ok(None) if start.elapsed() < std::time::Duration::from_secs(8) => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// True when the forwarded port answers with an SSH identification string within 1.5 s.
pub fn banner(port: u16) -> bool {
    use std::io::Read;
    let Ok(mut s) = std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), std::time::Duration::from_secs(1)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(std::time::Duration::from_millis(1500)));
    let mut b = [0u8; 4];
    s.read_exact(&mut b).is_ok() && &b == b"SSH-"
}

/// Replace `path`'s ACL with full control for the current user and SYSTEM only (no inheritance from
/// above). For a directory the grant is inherited by everything inside it.
pub fn restrict_to_owner(path: &Path) {
    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(d), Ok(u)) => format!("{d}\\{u}"),
        (_, u) => u.unwrap_or_default(),
    };
    let inherit = if path.is_dir() { "(OI)(CI)" } else { "" };
    let mut c = Command::new("icacls");
    c.arg(path)
        .args(["/inheritance:r", "/grant:r", &format!("{user}:{inherit}F"), "/grant:r", &format!("*S-1-5-18:{inherit}F")])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _ = hidden(&mut c).status();
}

/// OpenSSH on Windows refuses a private key readable by anyone else.
pub fn secure_key(key: &Path) {
    restrict_to_owner(key);
}

/// Generate an ed25519 key pair with ssh-keygen if absent.
/// Host key for a lite instance (BaseSystem has no ssh-keygen); delivered on the config disk.
pub fn ensure_host_key(dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let key = dir.join("ssh_host_ed25519_key");
    if !key.exists() {
        let mut c = Command::new(ssh_exe("ssh-keygen"));
        c.args(["-q", "-t", "ed25519", "-N", "", "-C", "wsm-host", "-f"]).arg(&key);
        anyhow::ensure!(hidden(&mut c).status()?.success(), "ssh-keygen (host key) failed");
    }
    secure_key(&key);
    Ok(key)
}

pub fn ensure_key(dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let key = dir.join("id_ed25519");
    if !key.exists() {
        let mut c = Command::new(ssh_exe("ssh-keygen"));
        c.args(["-q", "-t", "ed25519", "-N", "", "-C", "wsm", "-f"]).arg(&key);
        let st = hidden(&mut c).status()?;
        anyhow::ensure!(st.success(), "ssh-keygen failed");
    }
    secure_key(&key);
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(shell_join(&["echo".into(), "a b".into(), "it's".into()]), "echo 'a b' 'it'\\''s'");
        assert_eq!(shell_join(&["ls".into(), "-l".into()]), "ls -l");
        assert_eq!(shell_join(&["echo a | wc".into()]), "echo a | wc");
    }
}
