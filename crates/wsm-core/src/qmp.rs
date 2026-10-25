use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

pub struct Qmp {
    r: BufReader<TcpStream>,
    w: TcpStream,
}

impl Qmp {
    /// Connect to the instance's QMP socket file (one attempt; `timeout` bounds each reply).
    pub fn connect(sock: &Path, timeout: Duration) -> Result<Self> {
        let s = crate::afunix::connect(sock)?;
        s.set_read_timeout(Some(timeout))?;
        s.set_write_timeout(Some(timeout))?;
        let mut q = Qmp { r: BufReader::new(s.try_clone()?), w: s };
        let mut greeting = String::new();
        q.r.read_line(&mut greeting).context("QMP greeting")?;
        q.execute("qmp_capabilities", json!({}))?;
        Ok(q)
    }

    pub fn execute(&mut self, cmd: &str, args: Value) -> Result<Value> {
        let req = json!({"execute": cmd, "arguments": args});
        self.w.write_all(format!("{req}\n").as_bytes())?;
        loop {
            let mut line = String::new();
            if self.r.read_line(&mut line).context("QMP read")? == 0 {
                bail!("QMP closed during {cmd}");
            }
            let v: Value = serde_json::from_str(&line)?;
            if let Some(r) = v.get("return") {
                return Ok(r.clone());
            }
            if let Some(e) = v.get("error") {
                bail!("QMP {cmd}: {e}");
            }
        }
    }

    pub fn status(&mut self) -> Result<String> {
        Ok(self.execute("query-status", json!({}))?["status"].as_str().unwrap_or("?").to_string())
    }
}
