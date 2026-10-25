//! AF_UNIX stream sockets on Windows (Winsock, Windows 10 1803+) for the VM's control channels.
//!
//! QMP, the serial console and the migration stream used to be loopback TCP ports, which any local
//! process can reach: the serial console is a passwordless login shell, QMP controls the VM, and the
//! `-incoming` port accepts whatever VM state connects first. A socket file under the instance
//! directory (`%LOCALAPPDATA%\wsm\instances\<name>`, owner + SYSTEM + Administrators only) is
//! reachable only by the owner. A Winsock AF_UNIX socket is an ordinary `SOCKET`, so it's wrapped in
//! `TcpStream` to reuse std's read/write/timeout code.
use anyhow::{Context, Result, bail};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

#[cfg(windows)]
mod sys {
    pub type Socket = usize;
    pub const INVALID_SOCKET: Socket = !0;
    pub const AF_UNIX: i32 = 1;
    pub const SOCK_STREAM: i32 = 1;

    #[repr(C)]
    pub struct SockaddrUn {
        pub family: u16,
        pub path: [u8; 108],
    }

    #[link(name = "ws2_32")]
    unsafe extern "system" {
        pub fn WSAStartup(version: u16, data: *mut u8) -> i32;
        pub fn WSAGetLastError() -> i32;
        pub fn socket(af: i32, ty: i32, protocol: i32) -> Socket;
        pub fn connect(s: Socket, name: *const SockaddrUn, len: i32) -> i32;
        pub fn bind(s: Socket, name: *const SockaddrUn, len: i32) -> i32;
        pub fn listen(s: Socket, backlog: i32) -> i32;
        pub fn accept(s: Socket, addr: *mut SockaddrUn, len: *mut i32) -> Socket;
        pub fn closesocket(s: Socket) -> i32;
    }
}

#[cfg(windows)]
fn addr(path: &Path) -> Result<sys::SockaddrUn> {
    let s = path.to_str().context("socket path is not valid Unicode")?;
    // Winsock takes a narrow, NUL-terminated path; keep it ASCII so no code page is involved.
    anyhow::ensure!(s.is_ascii(), "socket path must be ASCII: {s}");
    anyhow::ensure!(s.len() < 108, "socket path longer than 107 bytes: {s}");
    let mut a = sys::SockaddrUn { family: sys::AF_UNIX as u16, path: [0; 108] };
    a.path[..s.len()].copy_from_slice(s.as_bytes());
    Ok(a)
}

#[cfg(windows)]
fn new_socket() -> Result<sys::Socket> {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let mut data = [0u8; 512]; // WSADATA is ~408 bytes on x64
        unsafe { sys::WSAStartup(0x0202, data.as_mut_ptr()) };
    });
    let s = unsafe { sys::socket(sys::AF_UNIX, sys::SOCK_STREAM, 0) };
    if s == sys::INVALID_SOCKET {
        bail!("AF_UNIX socket() failed: WSA error {}", unsafe { sys::WSAGetLastError() });
    }
    Ok(s)
}

#[cfg(windows)]
fn wrap(s: sys::Socket) -> TcpStream {
    use std::os::windows::io::FromRawSocket;
    unsafe { TcpStream::from_raw_socket(s as _) }
}

/// Connect to a listening socket file once.
#[cfg(windows)]
pub fn connect(path: &Path) -> Result<TcpStream> {
    let a = addr(path)?;
    let s = new_socket()?;
    if unsafe { sys::connect(s, &a, std::mem::size_of::<sys::SockaddrUn>() as i32) } != 0 {
        let e = unsafe { sys::WSAGetLastError() };
        unsafe { sys::closesocket(s) };
        bail!("connecting to {}: WSA error {e}", path.display());
    }
    Ok(wrap(s))
}

/// Connect, retrying until `timeout` (the server may not have created the socket yet).
pub fn connect_retry(path: &Path, timeout: Duration) -> Result<TcpStream> {
    let t = Instant::now();
    loop {
        match connect(path) {
            Ok(s) => return Ok(s),
            Err(e) if t.elapsed() >= timeout => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// A listening socket file, removed again on drop.
#[cfg(windows)]
pub struct Listener {
    sock: sys::Socket,
    path: std::path::PathBuf,
}

#[cfg(windows)]
impl Listener {
    pub fn bind(path: &Path) -> Result<Self> {
        let a = addr(path)?;
        let _ = std::fs::remove_file(path); // a stale file from a crash blocks bind
        let s = new_socket()?;
        let len = std::mem::size_of::<sys::SockaddrUn>() as i32;
        if unsafe { sys::bind(s, &a, len) } != 0 || unsafe { sys::listen(s, 1) } != 0 {
            let e = unsafe { sys::WSAGetLastError() };
            unsafe { sys::closesocket(s) };
            bail!("listening on {}: WSA error {e}", path.display());
        }
        Ok(Listener { sock: s, path: path.to_path_buf() })
    }

    pub fn accept(&self) -> Result<TcpStream> {
        let mut a = sys::SockaddrUn { family: 0, path: [0; 108] };
        let mut len = std::mem::size_of::<sys::SockaddrUn>() as i32;
        let c = unsafe { sys::accept(self.sock, &mut a, &mut len) };
        if c == sys::INVALID_SOCKET {
            bail!("accept on {}: WSA error {}", self.path.display(), unsafe { sys::WSAGetLastError() });
        }
        Ok(wrap(c))
    }
}

#[cfg(windows)]
impl Drop for Listener {
    fn drop(&mut self) {
        unsafe { sys::closesocket(self.sock) };
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(not(windows))]
pub fn connect(path: &Path) -> Result<TcpStream> {
    bail!("AF_UNIX transport is only implemented for Windows hosts ({})", path.display())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn roundtrip_over_socket_file() {
        let dir = std::env::temp_dir().join(format!("wsm-afunix-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.sock");
        let l = Listener::bind(&p).unwrap();
        let p2 = p.clone();
        let t = std::thread::spawn(move || {
            let mut c = connect_retry(&p2, Duration::from_secs(5)).unwrap();
            c.write_all(b"ping").unwrap();
        });
        let mut s = l.accept().unwrap();
        let mut b = [0u8; 4];
        s.read_exact(&mut b).unwrap();
        assert_eq!(&b, b"ping");
        t.join().unwrap();
        drop(l);
        assert!(!p.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
