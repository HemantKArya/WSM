//! CLI <-> wsmd protocol: one JSON request line, then zero or more `progress` lines and one final line.
//! Transport is an owner-only named pipe; discovery and a token live in `<home>\wsmd.json`.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use crate::ipc::Stream;
use std::path::Path;
use std::time::Duration;

pub const PROTOCOL_VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DaemonInfo {
    pub pipe: String,
    pub token: String,
    pub pid: u32,
    pub protocol: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Request {
    pub token: String,
    pub cmd: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub args: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Response {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub progress: Option<String>,
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

pub fn info_path(home: &Path) -> std::path::PathBuf { home.join("wsmd.json") }

pub fn read_info(home: &Path) -> Result<DaemonInfo> {
    let s = std::fs::read_to_string(info_path(home))?;
    Ok(serde_json::from_str(&s)?)
}

/// Send one request; `on_progress` sees each progress line. Returns the final response.
pub fn call(
    info: &DaemonInfo,
    cmd: &str,
    name: &str,
    args: serde_json::Value,
    mut on_progress: impl FnMut(&str),
) -> Result<Response> {
    let s = Stream::connect(&info.pipe)
        .context("connecting to wsmd")?;
    s.set_read_timeout(Some(Duration::from_secs(if cmd == "ping" { 3 } else { 180 })))?;
    let mut w = s.try_clone()?;
    let req = Request { token: info.token.clone(), cmd: cmd.into(), name: name.into(), args };
    w.write_all(format!("{}\n", serde_json::to_string(&req)?).as_bytes())?;
    for line in BufReader::new(s).lines() {
        let r: Response = serde_json::from_str(&line?)?;
        if let Some(p) = &r.progress {
            on_progress(p);
            continue;
        }
        return Ok(r);
    }
    bail!("wsmd closed the connection without a result")
}

/// Open a raw byte stream (`serial`): the daemon answers one JSON line, then relays bytes.
pub fn open_stream(info: &DaemonInfo, cmd: &str, name: &str) -> Result<Stream> {
    open_request(info, cmd, name, serde_json::json!({}))
}

pub fn open_request(info: &DaemonInfo, cmd: &str, name: &str, args: serde_json::Value) -> Result<Stream> {
    use std::io::Read;
    let mut s = Stream::connect(&info.pipe)
        .context("connecting to wsmd")?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let req = Request { token: info.token.clone(), cmd: cmd.into(), name: name.into(), args };
    s.write_all(format!("{}\n", serde_json::to_string(&req)?).as_bytes())?;
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    while s.read(&mut b)? == 1 && b[0] != b'\n' {
        line.push(b[0]);
    }
    let r: Response = serde_json::from_slice(&line).context("daemon reply")?;
    if r.ok != Some(true) {
        bail!("{}", r.msg);
    }
    s.set_read_timeout(None)?;
    Ok(s)
}
