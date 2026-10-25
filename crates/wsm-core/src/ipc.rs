//! Per-user daemon IPC. Windows uses a local-only named pipe with owner + SYSTEM permissions.
use anyhow::Result;
use interprocess::TryClone;
#[cfg(not(windows))]
use interprocess::local_socket::{self, prelude::*, GenericNamespaced, ListenerOptions, ConnectOptions};
use std::{io::{self, Read, Write}, path::Path, time::{Duration, Instant}, sync::{Arc, atomic::{AtomicU64, Ordering}}};

#[cfg(windows)]
type RawStream = interprocess::os::windows::named_pipe::DuplexPipeStream<interprocess::os::windows::named_pipe::pipe_mode::Bytes>;
#[cfg(windows)]
type RawListener = interprocess::os::windows::named_pipe::PipeListener<interprocess::os::windows::named_pipe::pipe_mode::Bytes, interprocess::os::windows::named_pipe::pipe_mode::Bytes>;
#[cfg(not(windows))]
type RawStream = local_socket::Stream;
#[cfg(not(windows))]
type RawListener = local_socket::Listener;

pub struct Stream { inner: RawStream, limits: Arc<Limits> }
#[derive(Default)]
struct Limits { read_ms: AtomicU64, write_ms: AtomicU64 }
pub struct Listener(RawListener);

pub fn name(home: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    home.to_string_lossy().replace('\\', "/").to_lowercase().hash(&mut hash);
    std::env::var("USERDOMAIN").unwrap_or_default().hash(&mut hash);
    std::env::var("USERNAME").unwrap_or_default().hash(&mut hash);
    format!("wsm-{:016x}", hash.finish())
}

impl Listener {
    pub fn bind(name: &str) -> Result<Self> {
        #[cfg(windows)]
        {
            use interprocess::os::windows::{named_pipe::PipeListenerOptions, security_descriptor::SecurityDescriptor};
            let sddl = widestring::U16CString::from_str("D:P(A;;GA;;;OW)(A;;GA;;;SY)")?;
            let mut options = PipeListenerOptions::new();
            options.path = std::borrow::Cow::Owned(pipe_path(name)?);
            options.security_descriptor = Some(SecurityDescriptor::deserialize(&sddl)?);
            // The generic local-socket wrapper uses 512-byte buffers. Exec frames can be 128 KiB;
            // a nonblocking write larger than the available pipe buffer cannot make progress.
            options.input_buffer_size_hint = 1 << 20;
            options.output_buffer_size_hint = 1 << 20;
            Ok(Self(options.create_duplex()?))
        }
        #[cfg(not(windows))]
        { Ok(Self(ListenerOptions::new().name(name.to_ns_name::<GenericNamespaced>()?).create_sync()?)) }
    }

    pub fn accept(&self) -> io::Result<Stream> { Stream::wrap(self.0.accept()?) }
}

impl Stream {
    fn wrap(inner: RawStream) -> io::Result<Self> {
        // Windows named pipes do not implement socket-style IO timeouts. Nonblocking pipe IO
        // lets this wrapper enforce deadlines without parking an unkillable reader thread.
        inner.set_nonblocking(true)?;
        Ok(Self { inner, limits: Default::default() })
    }
    pub fn connect(name: &str) -> Result<Self> {
        #[cfg(windows)]
        { Ok(Self::wrap(RawStream::connect_by_path(pipe_path(name)?)?)?) }
        #[cfg(not(windows))]
        {
        let name = name.to_ns_name::<GenericNamespaced>()?;
        let options = ConnectOptions::new().name(name).wait_mode(interprocess::ConnectWaitMode::Timeout(Duration::from_secs(3)));
        Ok(Self::wrap(options.connect_sync()?)?)
        }
    }
    pub fn try_clone(&self) -> io::Result<Self> { Ok(Self { inner: self.inner.try_clone()?, limits: self.limits.clone() }) }
    pub fn set_read_timeout(&self, limit: Option<Duration>) -> io::Result<()> {
        self.limits.read_ms.store(timeout_ms(limit), Ordering::Relaxed); Ok(())
    }
    pub fn set_write_timeout(&self, limit: Option<Duration>) -> io::Result<()> {
        self.limits.write_ms.store(timeout_ms(limit), Ordering::Relaxed); Ok(())
    }
}
fn timeout_ms(limit: Option<Duration>) -> u64 { limit.map(|t| u64::try_from(t.as_millis().max(1)).unwrap_or(u64::MAX)).unwrap_or(0) }
fn retry<T>(limit: u64, mut io: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let start = Instant::now();
    loop {
        match io() {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if limit != 0 && start.elapsed() >= Duration::from_millis(limit) {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "named-pipe IO deadline exceeded"));
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            result => return result,
        }
    }
}
impl Read for Stream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() { return Ok(0); }
        retry(self.limits.read_ms.load(Ordering::Relaxed), || {
            #[cfg(windows)]
            match pipe_readable(&self.inner) {
                Ok(false) => return Err(io::ErrorKind::WouldBlock.into()),
                Err(error) if error.kind() == io::ErrorKind::BrokenPipe => return Ok(0),
                Err(error) => return Err(error),
                Ok(true) => {}
            }
            self.inner.read(bytes)
        })
    }
}
impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        retry(self.limits.write_ms.load(Ordering::Relaxed), || {
            match self.inner.write(&bytes[..bytes.len().min(65536)]) {
                Ok(0) if !bytes.is_empty() => Err(io::ErrorKind::WouldBlock.into()),
                result => result,
            }
        })
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

#[cfg(windows)]
fn pipe_path(name: &str) -> Result<widestring::U16CString> { Ok(widestring::U16CString::from_str(format!(r"\\.\pipe\{name}"))?) }

#[cfg(windows)]
fn pipe_readable(pipe: &RawStream) -> io::Result<bool> {
    use std::{ffi::c_void, os::windows::io::{AsHandle, AsRawHandle}};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn PeekNamedPipe(pipe: *mut c_void, buffer: *mut c_void, size: u32, read: *mut u32, available: *mut u32, left: *mut u32) -> i32;
    }
    let mut available = 0;
    let ok = unsafe { PeekNamedPipe(pipe.as_handle().as_raw_handle(), std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut available, std::ptr::null_mut()) };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(available > 0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplex_named_pipe_and_exclusive_listener() {
        let name = format!("wsm-test-{}", crate::provider::random_hex(8));
        let listener = Listener::bind(&name).unwrap();
        assert!(Listener::bind(&name).is_err(), "a second daemon must not bind the same pipe");
        let server = std::thread::spawn(move || {
            let mut s = listener.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut b = [0; 4]; s.read_exact(&mut b).unwrap(); assert_eq!(&b, b"ping");
            s.write_all(b"pong").unwrap();
        });
        let mut client = Stream::connect(&name).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        client.write_all(b"ping").unwrap();
        let mut b = [0; 4]; client.read_exact(&mut b).unwrap(); assert_eq!(&b, b"pong");
        server.join().unwrap();
    }
}
