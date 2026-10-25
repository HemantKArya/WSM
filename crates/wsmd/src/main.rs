//! wsmd: per-user background service. Owns VM lifecycle; the CLI uses an owner-only named pipe.
mod dav;
mod manager;
mod serial;
mod ssh_pool;

use anyhow::Result;
use manager::Manager;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use wsm_core::ipc::{self, Listener, Stream};
use std::sync::Arc;
use wsm_core::paths;
use wsm_core::proto::{DaemonInfo, PROTOCOL_VERSION, Request, Response, info_path};
use wsm_core::provider::random_hex;

fn send(w: &mut Stream, r: &Response) {
    let _ = w.write_all(format!("{}\n", serde_json::to_string(r).unwrap()).as_bytes());
}

fn handle(mgr: &Arc<Manager>, token: &str, s: Stream) -> Result<()> {
    let mut w = s.try_clone()?;
    let mut line = String::new();
    BufReader::new(s.try_clone()?).read_line(&mut line)?;
    let req: Request = serde_json::from_str(&line)?;
    if req.token != token {
        send(&mut w, &Response { ok: Some(false), msg: "bad token".into(), ..Default::default() });
        return Ok(());
    }
    if req.cmd == "exec" {
        send(&mut w, &Response { ok: Some(true), ..Default::default() });
        if let Err(e) = mgr.exec(&req.name, &req.args, s) {
            send(&mut w, &Response { ok: Some(false), msg: format!("{e:#}"), ..Default::default() });
        }
        return Ok(());
    }
    if req.cmd == "serial" {
        match mgr.is_running(&req.name).then_some(()) {
            Some(()) => {
                send(&mut w, &Response { ok: Some(true), ..Default::default() });
                return mgr.serial_attach(&req.name, s);
            }
            None => {
                send(&mut w, &Response { ok: Some(false), msg: "instance is not running".into(), ..Default::default() });
                return Ok(());
            }
        }
    }
    let mut prog = |m: &str| send(&mut w.try_clone().unwrap(), &Response { progress: Some(m.into()), ..Default::default() });
    let n = req.name.as_str();
    let a = &req.args;
    let result: Result<Value> = match req.cmd.as_str() {
        "ping" => Ok(json!({"protocol": PROTOCOL_VERSION})),
        "list" => Ok(json!(mgr.list())),
        "create" => mgr.create(n, a, &mut prog),
        "delete" => mgr.delete(n),
        "status" => mgr.status(n, a["probe"].as_bool().unwrap_or(true)),
        "start" => mgr.start(n, a["wait"].as_str().unwrap_or("none"), a["cold"].as_bool().unwrap_or(false), &mut prog),
        "suspend" => mgr.suspend(n, &mut prog),
        "shutdown" => mgr.shutdown(n, &mut prog),
        "kill" => mgr.kill(n),
        "mount" => mgr.mount(n),
        "share" => mgr.share(a["folder"].as_str().unwrap_or_default()),
        "gui" => mgr.gui(n, a["action"].as_str().unwrap_or("start")),
        "lease" => { mgr.lease(n, a["delta"].as_i64().unwrap_or(0) as i32); Ok(json!({})) }
        "quit" => { std::process::exit(0) }
        other => Err(anyhow::anyhow!("unknown command {other}")),
    };
    let r = match result {
        Ok(data) => Response { ok: Some(true), data, ..Default::default() },
        Err(e) => Response { ok: Some(false), msg: format!("{e:#}"), ..Default::default() },
    };
    send(&mut w, &r);
    Ok(())
}

fn main() -> Result<()> {
    wsm_core::hostinfo::private_standard_handles();
    let home = paths::home()?;
    std::fs::create_dir_all(&home)?;
    wsm_core::ssh::restrict_to_owner(&home);
    let pipe = ipc::name(&home);
    let listener = Listener::bind(&pipe)?;
    let token = random_hex(16);
    let info = DaemonInfo { pipe, token: token.clone(), pid: std::process::id(), protocol: PROTOCOL_VERSION };
    std::fs::write(info_path(&home), serde_json::to_string(&info)?)?;
    let mgr = Arc::new(Manager::new(home));
    {
        let m = mgr.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(20));
            m.idle_sweep();
        });
    }
    loop {
        let conn = listener.accept()?;
        let (m, t) = (mgr.clone(), token.clone());
        std::thread::spawn(move || {
            if let Err(e) = handle(&m, &t, conn) {
                eprintln!("wsmd: request failed: {e:#}");
            }
        });
    }
}
