//! Serial hub: wsmd is the only client of the VM's serial socket. It keeps a ring buffer of recent
//! output (so a late `wsm shell` still sees the boot log) and relays to token-authenticated CLI sessions.
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpStream;
use wsm_core::ipc::Stream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RING: usize = 64 * 1024;

pub struct Hub {
    ring: Mutex<VecDeque<u8>>,
    clients: Mutex<Vec<Stream>>,
    qemu: Mutex<Option<TcpStream>>,
}

impl Hub {
    /// Connect to the chardev's socket file in the background (QEMU may not be listening yet).
    pub fn start(sock: std::path::PathBuf, log: std::path::PathBuf) -> Arc<Hub> {
        let hub = Arc::new(Hub { ring: Default::default(), clients: Default::default(), qemu: Default::default() });
        let h = hub.clone();
        std::thread::spawn(move || {
            // serial-timed.log: the same lines prefixed with seconds since QEMU was spawned, for boot analysis.
            let timed_path = log.with_file_name("serial-timed.log");
            let mut log = std::fs::OpenOptions::new().create(true).append(true).open(log).ok();
            let mut timed = std::fs::OpenOptions::new().create(true).append(true).open(timed_path).ok();
            let mut line: Vec<u8> = Vec::new();
            let t = Instant::now();
            let mut s = loop {
                match wsm_core::afunix::connect(&sock) {
                    Ok(s) => break s,
                    Err(_) if t.elapsed() < Duration::from_secs(60) => std::thread::sleep(Duration::from_millis(50)),
                    Err(_) => return,
                }
            };
            *h.qemu.lock().unwrap() = s.try_clone().ok();
            let mut buf = [0u8; 4096];
            while let Ok(n) = s.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if let Some(f) = log.as_mut() { let _ = f.write_all(&buf[..n]); }
                if let Some(f) = timed.as_mut() {
                    for &b in &buf[..n] {
                        if b == b'\n' {
                            let _ = writeln!(f, "[{:8.3}] {}", t.elapsed().as_secs_f64(), String::from_utf8_lossy(&line).trim_end());
                            line.clear();
                        } else if line.len() < 4096 {
                            line.push(b);
                        }
                    }
                }
                {
                    let mut r = h.ring.lock().unwrap();
                    r.extend(&buf[..n]);
                    let over = r.len().saturating_sub(RING);
                    r.drain(..over);
                }
                h.clients.lock().unwrap().retain_mut(|c| c.write_all(&buf[..n]).is_ok());
            }
            *h.qemu.lock().unwrap() = None;
        });
        hub
    }

    /// Serve one CLI session until it disconnects.
    pub fn attach(&self, mut client: Stream) {
        let backlog: Vec<u8> = {
            let r = self.ring.lock().unwrap();
            let skip = r.len().saturating_sub(8 * 1024);
            r.iter().skip(skip).copied().collect()
        };
        if client.write_all(&backlog).is_err() {
            return;
        }
        let Ok(out) = client.try_clone() else { return };
        self.clients.lock().unwrap().push(out);
        let mut buf = [0u8; 1024];
        while let Ok(n) = client.read(&mut buf) {
            if n == 0 {
                break;
            }
            let q = self.qemu.lock().unwrap().as_ref().and_then(|s| s.try_clone().ok());
            match q {
                Some(mut q) => {
                    if q.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                None => break,
            }
        }
    }
}
