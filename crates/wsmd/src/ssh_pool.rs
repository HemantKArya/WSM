//! Checked-out SSH connections: one channel per connection, up to four idle connections per VM.
//! Nonblocking channel IO drains stdout and stderr together, so neither pipe can deadlock the other.
use anyhow::{Context, Result};
use ssh2::Session;
use std::{collections::{HashMap, VecDeque}, io::{Read, Write}, net::TcpStream,
          sync::{Mutex, mpsc, atomic::{AtomicBool, Ordering}}, time::Duration};
use wsm_core::{proto::Response, ssh::{Target, shell_join}};

#[derive(serde::Deserialize, serde::Serialize)]
pub struct Input {
    #[serde(default)]
    pub data: Vec<u8>,
    #[serde(default)]
    pub eof: bool,
}

#[derive(Default)]
pub struct Pool(Mutex<HashMap<String, Vec<Session>>>);

impl Pool {
    /// Drops the instance's pooled connections, including its root ones (`name#root`).
    pub fn invalidate(&self, name: &str) {
        let root = format!("{name}#root");
        self.0.lock().unwrap().retain(|k, _| k != name && *k != root);
    }

    fn connect(t: &Target) -> Result<Session> {
        let tcp = TcpStream::connect_timeout(&([127, 0, 0, 1], t.port).into(), Duration::from_secs(8))?;
        tcp.set_nodelay(true)?;
        let mut session = Session::new()?;
        session.set_tcp_stream(tcp);
        session.set_timeout(8000);
        session.handshake().context("SSH handshake")?;
        session.userauth_pubkey_file(t.user, None, t.key, None).context("SSH public-key login")?;
        anyhow::ensure!(session.authenticated(), "SSH authentication failed");
        Ok(session)
    }

    pub fn exec(&self, name: &str, t: &Target, argv: &[String], input: mpsc::Receiver<Input>, cancelled: &AtomicBool,
                mut emit: impl FnMut(Response) -> Result<()>) -> Result<i32> {
        let cached = self.0.lock().unwrap().get_mut(name).and_then(Vec::pop);
        let mut session = match cached { Some(s) => s, None => Self::connect(t)? };
        // Reconnect only before a command is sent. Retrying after exec could run a mutation twice.
        let mut channel = match session.channel_session() {
            Ok(c) => c,
            Err(_) => { session = Self::connect(t)?; session.channel_session()? }
        };
        channel.exec(&shell_join(argv)).context("starting remote command")?;
        session.set_blocking(false);
        let mut pending: VecDeque<Vec<u8>> = VecDeque::new();
        let (mut offset, mut input_eof, mut eof_sent) = (0, false, false);
        let mut buf = [0u8; 32768];
        let result = (|| -> Result<i32> {
            loop {
                anyhow::ensure!(!cancelled.load(Ordering::Relaxed), "exec client disconnected");
                let mut progress = false;
                while !input_eof && pending.len() < 4 {
                    match input.try_recv() {
                        Ok(i) => { input_eof = i.eof; if !i.data.is_empty() { pending.push_back(i.data); } }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => { input_eof = true; break; }
                    }
                }
                if let Some(bytes) = pending.front() {
                    match channel.write(&bytes[offset..]) {
                        Ok(n) => {
                            progress |= n > 0; offset += n;
                            if offset == bytes.len() { pending.pop_front(); offset = 0; }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                if input_eof && pending.is_empty() && !eof_sent {
                    match channel.send_eof() {
                        Ok(()) => eof_sent = true,
                        Err(e) if e.code() == ssh2::ErrorCode::Session(-37) => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                for stream_id in [0, 1] {
                    let mut stream = channel.stream(stream_id);
                    for _ in 0..8 {
                        match stream.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                let bytes = Some(buf[..n].to_vec());
                                emit(if stream_id == 0 { Response { stdout: bytes, ..Default::default() } }
                                     else { Response { stderr: bytes, ..Default::default() } })?;
                                progress = true;
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                            Err(e) => return Err(e.into()),
                        }
                    }
                }
                if channel.eof() { break; }
                if !progress { std::thread::sleep(Duration::from_millis(2)); }
            }
            session.set_blocking(true);
            channel.wait_close()?;
            channel.exit_status().map_err(Into::into)
        })();
        drop(channel);
        if result.is_ok() {
            let mut map = self.0.lock().unwrap();
            let idle = map.entry(name.to_string()).or_default();
            if idle.len() < 4 { idle.push(session); }
        }
        result
    }
}

/// Test helper used by the opt-in real-VM test below.
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    #[ignore = "requires a running real VM; WSM_TEST_PORT and WSM_TEST_KEY"]
    fn real_vm_streams_and_reuses_connection() {
        let port = std::env::var("WSM_TEST_PORT").unwrap().parse().unwrap();
        let key = std::env::var("WSM_TEST_KEY").unwrap();
        let t = Target { user: "dev", port, key: Path::new(&key) };
        let pool = Pool::default();
        for _ in 0..2 {
            let (tx, rx) = mpsc::channel();
            tx.send(Input { data: b"hello\n".to_vec(), eof: true }).unwrap();
            let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
            let code = pool.exec("test", &t, &["cat; printf err >&2; exit 7".into()], rx, &AtomicBool::new(false), |r| {
                stdout.extend(r.stdout.unwrap_or_default()); stderr.extend(r.stderr.unwrap_or_default()); Ok(())
            }).unwrap();
            assert_eq!(stdout, b"hello\n"); assert_eq!(stderr, b"err"); assert_eq!(code, 7);
            assert_eq!(pool.0.lock().unwrap()["test"].len(), 1);
        }
    }
}
