//! Raw serial console attach (QEMU chardev named pipe <-> this console).
use anyhow::Result;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use wsm_core::ssh::{self, Target};

#[cfg(windows)]
mod sys {
    unsafe extern "system" {
        pub fn GetStdHandle(n: u32) -> isize;
        pub fn GetConsoleMode(h: isize, m: *mut u32) -> i32;
        pub fn SetConsoleMode(h: isize, m: u32) -> i32;
    }
}

struct RawMode {
    #[cfg(windows)]
    saved: Vec<(isize, u32)>,
}

impl RawMode {
    fn enter() -> RawMode {
        #[cfg(windows)]
        unsafe {
            let mut saved = Vec::new();
            let hin = sys::GetStdHandle(-10i32 as u32);
            let hout = sys::GetStdHandle(-11i32 as u32);
            let mut m = 0u32;
            if sys::GetConsoleMode(hin, &mut m) != 0 {
                saved.push((hin, m));
                // no line input / echo / ctrl-c processing; VT input so arrows arrive as escape sequences
                sys::SetConsoleMode(hin, (m & !(0x1 | 0x2 | 0x4)) | 0x200);
            }
            if sys::GetConsoleMode(hout, &mut m) != 0 {
                saved.push((hout, m));
                sys::SetConsoleMode(hout, m | 0x4);
            }
            RawMode { saved }
        }
        #[cfg(not(windows))]
        RawMode {}
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            for (h, m) in &self.saved {
                sys::SetConsoleMode(*h, *m);
            }
        }
    }
}

/// Attach to the serial pipe. Returns true if it detached because SSH came up (`switch` given).
pub fn attach(f: wsm_core::ipc::Stream, switch: Option<(String, u16, PathBuf)>) -> Result<bool> {
    let _ = Instant::now();
    let done = Arc::new(AtomicBool::new(false));
    let switched = Arc::new(AtomicBool::new(false));
    if let Some((user, port, key)) = switch {
        let (d, s) = (done.clone(), switched.clone());
        std::thread::spawn(move || {
            let t = Target { user: &user, port, key: &key };
            while !d.load(Ordering::Relaxed) {
                if ssh::ready(&t) {
                    s.store(true, Ordering::Relaxed);
                    d.store(true, Ordering::Relaxed);
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    }
    let _raw = RawMode::enter();
    let mut rd = f.try_clone()?;
    let d2 = done.clone();
    std::thread::spawn(move || {
        let mut out = std::io::stdout();
        let mut buf = [0u8; 4096];
        while !d2.load(Ordering::Relaxed) {
            match rd.read(&mut buf) {
                Ok(0) | Err(_) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Ok(n) => {
                    let _ = out.write_all(&buf[..n]);
                    let _ = out.flush();
                }
            }
        }
    });
    let mut wr = f;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 256];
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    while !done.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(b) => {
                if b.contains(&0x1d) {
                    break; // Ctrl-]
                }
                // The serial tty expects CR for Enter; the console already sends CR.
                let _ = wr.write_all(&b);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
    }
    done.store(true, Ordering::Relaxed);
    println!();
    Ok(switched.load(Ordering::Relaxed))
}
