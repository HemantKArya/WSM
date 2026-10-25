//! Suspend/resume. QEMU on Windows cannot `migrate file:`, so a socket bridge writes/reads the file. The
//! bridge is an owner-only AF_UNIX socket file: a TCP port would let any local process read the VM's
//! memory on save or inject VM state on resume.
use anyhow::{Context, Result, bail};
use serde_json::json;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};

use crate::afunix;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::qmp::Qmp;

/// Where a save spent its time (seconds), for `wsm stop` output and the P6 numbers.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct SaveStats {
    pub bytes: u64,
    /// `migrate` until QMP reports completed.
    pub migrate_s: f64,
    /// Reading the rest of the stream after completed.
    pub drain_s: f64,
    /// `fsync` of the state file.
    pub sync_s: f64,
}

/// Pause the VM and stream its state to `file`.
pub fn save(qmp_sock: &Path, mig_sock: &Path, file: &Path) -> Result<SaveStats> {
    let listener = afunix::Listener::bind(mig_sock)?;
    let tmp = file.with_extension("part");
    let tmp2 = tmp.clone();
    // QEMU on Windows never closes the migration socket after "completed" (P1, logs/r3/p1-save-trace2.txt),
    // so EOF can't mark the end. Once QMP says completed, the whole stream is already in the socket:
    // drain it and stop at the first idle read.
    let done = Arc::new(AtomicBool::new(false));
    let done2 = done.clone();
    let sink: JoinHandle<Result<(u64, f64)>> = std::thread::spawn(move || {
        let mut c = listener.accept()?;
        drop(listener); // one connection only; removes the socket file
        // Short timeout: after "completed" the rest is already in loopback buffers.
        c.set_read_timeout(Some(Duration::from_millis(100)))?;
        let mut f = File::create(&tmp2)?;
        let mut buf = vec![0u8; 4 << 20];
        let mut n = 0u64;
        loop {
            let k = match c.read(&mut buf) {
                Ok(k) => k,
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    if done2.load(Ordering::SeqCst) { break } else { continue }
                }
                Err(e) => return Err(e.into()),
            };
            if k == 0 {
                break;
            }
            f.write_all(&buf[..k])?;
            n += k as u64;
        }
        let t = Instant::now();
        f.sync_all()?;
        Ok((n, t.elapsed().as_secs_f64()))
    });
    // The completion pass runs with QEMU's main loop busy, so QMP replies can take as long as the copy.
    // If QEMU never connects, a connection of our own releases the sink's accept().
    let release = |done: &AtomicBool| {
        done.store(true, Ordering::SeqCst);
        let _ = afunix::connect(mig_sock);
    };
    let mut q = match Qmp::connect(qmp_sock, Duration::from_secs(180)) {
        Ok(q) => q,
        Err(e) => { release(&done); return Err(e) }
    };
    q.execute("stop", json!({}))?;
    // This QEMU's WHPX backend has no dirty-page tracking, so every sync reports all RAM dirty and
    // precopy never converges. The VM is stopped, so one full pass is the whole state: allow the
    // "downtime" to cover it and QEMU completes after the first pass.
    q.execute("migrate-set-parameters", json!({"downtime-limit": 600_000}))?;
    if let Err(e) = q.execute("migrate", json!({"uri": format!("unix:{}", mig_sock.display().to_string().replace('\\', "/"))})) {
        release(&done);
        return Err(e);
    }
    let t0 = Instant::now();
    loop {
        let st = q.execute("query-migrate", json!({}))?;
        match st["status"].as_str().unwrap_or("") {
            "completed" => break,
            "failed" | "cancelled" => {
                release(&done);
                bail!("migration {}: {}", st["status"], st["error-desc"])
            }
            _ => {}
        }
        if t0.elapsed() > Duration::from_secs(120) {
            let _ = q.execute("migrate_cancel", json!({}));
            release(&done);
            bail!("migration timeout");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let migrate_s = t0.elapsed().as_secs_f64();
    done.store(true, Ordering::SeqCst);
    let t1 = Instant::now();
    let (bytes, sync_s) = sink.join().map_err(|_| anyhow::anyhow!("sink panicked"))??;
    let drain_s = t1.elapsed().as_secs_f64() - sync_s;
    std::fs::rename(&tmp, file)?;
    Ok(SaveStats { bytes, migrate_s, drain_s, sync_s })
}

/// Feed `file` into a QEMU started with `-incoming unix:<mig_sock>`. Returns the open connection:
/// QEMU on Windows doesn't close it after loading (P1), so the caller watches QMP for the end of the
/// load and drops the stream afterwards.
pub fn feed(mig_sock: &Path, file: &Path) -> Result<TcpStream> {
    let mut c = afunix::connect_retry(mig_sock, Duration::from_secs(15)).context("connecting to -incoming")?;
    let mut f = File::open(file)?;
    let mut buf = vec![0u8; 4 << 20];
    loop {
        let k = f.read(&mut buf)?;
        if k == 0 {
            break;
        }
        c.write_all(&buf[..k])?;
    }
    c.flush()?;
    Ok(c)
}

/// A free loopback port.
pub fn free_port() -> Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}
