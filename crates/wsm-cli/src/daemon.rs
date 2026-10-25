use anyhow::{Result, bail};
use std::path::Path;
use std::time::{Duration, Instant};
use wsm_core::proto::{self, DaemonInfo};

fn alive(info: &DaemonInfo) -> bool {
    if info.protocol != proto::PROTOCOL_VERSION { return false; }
    proto::call(info, "ping", "", serde_json::json!({}), |_| {}).map(|r| r.ok == Some(true)).unwrap_or(false)
}

/// Return a live wsmd, starting one detached if needed.
pub fn ensure(home: &Path) -> Result<DaemonInfo> {
    if let Ok(i) = proto::read_info(home)
        && alive(&i) {
            return Ok(i);
        }
    let exe = std::env::current_exe()?.with_file_name("wsmd.exe");
    if !exe.exists() {
        bail!("{} not found next to wsm.exe", exe.display());
    }
    let _ = std::fs::remove_file(proto::info_path(home));
    let mut cmd = std::process::Command::new(exe);
    std::fs::create_dir_all(home)?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(home.join("wsmd.log"))?;
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(log);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000 | 0x0000_0200 | 0x0000_0008); // no window, new group, detached
    }
    cmd.spawn()?;
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(10) {
        if let Ok(i) = proto::read_info(home)
            && alive(&i) {
                return Ok(i);
            }
        std::thread::sleep(Duration::from_millis(50));
    }
    bail!("wsmd did not start")
}
