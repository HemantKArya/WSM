//! WebDAV bridge, both directions, with nothing installed on either side:
//! - `wsm mount`: the guest home as a Windows drive. Windows' own WebDAV redirector (WebClient,
//!   trigger-started for a normal user) maps `\\127.0.0.1@<port>\<token>`; files go over SFTP.
//! - `wsm share`: a Windows folder for the guest. macOS' own `mount_webdav` reaches this host's
//!   loopback at 10.0.2.2 (QEMU user networking); files are served from the local folder.
//!
//! Only loopback is bound and only paths under the random per-share token are served.
//! Scope: what the two clients use (OPTIONS, PROPFIND, GET/HEAD with Range, PUT, DELETE, MKCOL,
//! MOVE, COPY, PROPPATCH, LOCK/UNLOCK as no-op locks).
use anyhow::{Context, Result};
use ssh2::{OpenFlags, OpenType, RenameFlags, Session, Sftp};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

/// Gives the SFTP source a live SSH target, resuming the instance when needed.
pub trait Backend: Send + Sync + 'static {
    fn target(&self, name: &str) -> Result<(String, u16, PathBuf)>;
    fn touch(&self, name: &str);
}

pub struct Meta {
    pub dir: bool,
    pub size: u64,
    pub mtime: u64,
}

pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// The file operations the DAV methods need. Paths are absolute in the source's own namespace.
pub trait Fs {
    fn root(&self) -> &Path;
    fn stat(&self, p: &Path) -> Result<Meta>;
    fn list(&self, p: &Path) -> Result<Vec<(PathBuf, Meta)>>;
    fn open(&self, p: &Path) -> Result<Box<dyn ReadSeek + '_>>;
    fn create(&self, p: &Path) -> Result<Box<dyn Write + '_>>;
    fn mkdir(&self, p: &Path) -> Result<()>;
    fn remove_all(&self, p: &Path) -> Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    fn copy_all(&self, from: &Path, to: &Path) -> Result<()> {
        if self.stat(from)?.dir {
            self.mkdir(to)?;
            for (child, _) in self.list(from)? {
                self.copy_all(&child, &to.join(child.file_name().unwrap_or_default()))?;
            }
        } else {
            std::io::copy(&mut self.open(from)?, &mut self.create(to)?)?;
        }
        Ok(())
    }
}

/// Where a connection gets its filesystem; `None` from `touch` means nothing to record.
pub trait Source: Send + Sync + 'static {
    fn connect(&self) -> Result<Box<dyn Fs + Send>>;
    fn touch(&self) {}
}

// ------------------------------------------------------------------------------------- SFTP

pub struct SftpSource {
    pub backend: Arc<dyn Backend>,
    pub name: String,
}

struct SftpFs {
    _session: Session,
    sftp: Sftp,
    home: PathBuf,
}

impl Source for SftpSource {
    fn connect(&self) -> Result<Box<dyn Fs + Send>> {
        let (user, port, key) = self.backend.target(&self.name)?;
        let tcp = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(8))?;
        tcp.set_nodelay(true)?;
        let mut s = Session::new()?;
        s.set_tcp_stream(tcp);
        s.set_timeout(60_000);
        s.handshake().context("SSH handshake")?;
        s.userauth_pubkey_file(&user, None, &key, None).context("SSH public-key login")?;
        let sftp = s.sftp().context("SFTP subsystem")?;
        let home = sftp.realpath(Path::new("."))?;
        Ok(Box::new(SftpFs { _session: s, sftp, home }))
    }
    fn touch(&self) { self.backend.touch(&self.name); }
}

fn sftp_meta(s: &ssh2::FileStat) -> Meta {
    Meta { dir: s.is_dir(), size: s.size.unwrap_or(0), mtime: s.mtime.unwrap_or(0) }
}

impl Fs for SftpFs {
    fn root(&self) -> &Path { &self.home }
    fn stat(&self, p: &Path) -> Result<Meta> { Ok(sftp_meta(&self.sftp.stat(p)?)) }
    fn list(&self, p: &Path) -> Result<Vec<(PathBuf, Meta)>> {
        Ok(self.sftp.readdir(p)?.into_iter().map(|(p, s)| (p, sftp_meta(&s))).collect())
    }
    fn open(&self, p: &Path) -> Result<Box<dyn ReadSeek + '_>> { Ok(Box::new(self.sftp.open(p)?)) }
    fn create(&self, p: &Path) -> Result<Box<dyn Write + '_>> {
        Ok(Box::new(self.sftp.open_mode(p, OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE, 0o644, OpenType::File)?))
    }
    fn mkdir(&self, p: &Path) -> Result<()> { Ok(self.sftp.mkdir(p, 0o755)?) }
    fn remove_all(&self, p: &Path) -> Result<()> {
        if self.sftp.lstat(p)?.is_dir() {
            for (child, _) in self.sftp.readdir(p)? { self.remove_all(&child)?; }
            self.sftp.rmdir(p)?;
        } else {
            self.sftp.unlink(p)?;
        }
        Ok(())
    }
    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        Ok(self.sftp.rename(from, to, Some(RenameFlags::OVERWRITE | RenameFlags::ATOMIC | RenameFlags::NATIVE))?)
    }
}

// ------------------------------------------------------------------------------------ local

pub struct LocalSource(pub PathBuf);

struct LocalFs(PathBuf);

impl Source for LocalSource {
    fn connect(&self) -> Result<Box<dyn Fs + Send>> {
        anyhow::ensure!(self.0.is_dir(), "shared folder {} is gone", self.0.display());
        Ok(Box::new(LocalFs(self.0.clone())))
    }
}

fn local_meta(m: &std::fs::Metadata) -> Meta {
    let mtime = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    Meta { dir: m.is_dir(), size: m.len(), mtime }
}

impl Fs for LocalFs {
    fn root(&self) -> &Path { &self.0 }
    fn stat(&self, p: &Path) -> Result<Meta> { Ok(local_meta(&std::fs::metadata(p)?)) }
    fn list(&self, p: &Path) -> Result<Vec<(PathBuf, Meta)>> {
        let mut v = Vec::new();
        for e in std::fs::read_dir(p)?.flatten() {
            if let Ok(m) = e.metadata() { v.push((e.path(), local_meta(&m))); }
        }
        Ok(v)
    }
    fn open(&self, p: &Path) -> Result<Box<dyn ReadSeek + '_>> { Ok(Box::new(std::fs::File::open(p)?)) }
    fn create(&self, p: &Path) -> Result<Box<dyn Write + '_>> { Ok(Box::new(std::fs::File::create(p)?)) }
    fn mkdir(&self, p: &Path) -> Result<()> { Ok(std::fs::create_dir(p)?) }
    fn remove_all(&self, p: &Path) -> Result<()> {
        // Never follow a junction or symlink out of the shared folder.
        if std::fs::symlink_metadata(p)?.is_dir() { std::fs::remove_dir_all(p)? } else { std::fs::remove_file(p)? }
        Ok(())
    }
    fn rename(&self, from: &Path, to: &Path) -> Result<()> { Ok(std::fs::rename(from, to)?) }
}

// ------------------------------------------------------------------------------------ server

pub struct Mount {
    pub port: u16,
    pub token: String,
}

/// Serve `source` on 127.0.0.1 (ephemeral port) under `/<token>/`.
pub fn serve(source: Arc<dyn Source>, token: String) -> Result<Mount> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let tok = token.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let (s, t) = (source.clone(), tok.clone());
            std::thread::spawn(move || {
                let mut c = Conn { source: s, token: t, fs: None };
                if let Err(e) = c.run(conn) {
                    eprintln!("wsmd: dav: {e:#}");
                }
            });
        }
    });
    Ok(Mount { port, token })
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl Request {
    fn header(&self, k: &str) -> Option<&str> {
        self.headers.iter().find(|(h, _)| h.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
}

struct Resp {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl Resp {
    fn new(status: u16) -> Resp { Resp { status, headers: Vec::new(), body: Vec::new() } }
    fn xml(status: u16, body: String) -> Resp {
        Resp { status, headers: vec![("Content-Type", "application/xml; charset=utf-8".into())], body: body.into_bytes() }
    }
}

struct Conn {
    source: Arc<dyn Source>,
    token: String,
    fs: Option<Box<dyn Fs + Send>>,
}

fn reason(s: u16) -> &'static str {
    match s {
        200 => "OK", 201 => "Created", 204 => "No Content", 206 => "Partial Content", 207 => "Multi-Status",
        400 => "Bad Request", 403 => "Forbidden", 404 => "Not Found", 405 => "Method Not Allowed",
        409 => "Conflict", 412 => "Precondition Failed", 415 => "Unsupported Media Type", 416 => "Range Not Satisfiable", _ => "Internal Server Error",
    }
}

impl Conn {
    fn run(&mut self, stream: TcpStream) -> Result<()> {
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_secs(300)))?;
        let mut w = stream.try_clone()?;
        let mut r = BufReader::with_capacity(64 << 10, stream);
        loop {
            let Some(req) = read_head(&mut r)? else { return Ok(()) };
            let close = req.header("Connection").is_some_and(|v| v.eq_ignore_ascii_case("close"));
            let mut body = Body::new(&req)?;
            self.source.touch();
            let resp = match self.handle(&req, &mut r, &mut body, &mut w) {
                Ok(Some(resp)) => resp,
                Ok(None) => { if close { return Ok(()); } continue; } // response already streamed
                Err(e) => {
                    let st = status_of(&e);
                    if st == 500 { eprintln!("wsmd: dav {} {}: {e:#}", req.method, req.path); }
                    Resp::new(st)
                }
            };
            body.drain(&mut r)?;
            write_resp(&mut w, &resp, req.method == "HEAD")?;
            if close { return Ok(()); }
        }
    }

    /// The filesystem, (re)connected when the old one stopped answering (e.g. after a suspend).
    fn fs(&mut self) -> Result<&dyn Fs> {
        let alive = self.fs.as_ref().is_some_and(|f| f.stat(f.root()).is_ok());
        if !alive {
            self.fs = None;
            self.fs = Some(self.source.connect()?);
        }
        Ok(self.fs.as_deref().unwrap())
    }

    /// Source path for a request path, or None when it is outside this share's token.
    fn local_path(&mut self, url_path: &str) -> Result<Option<PathBuf>> {
        let p = percent_decode(url_path.split('?').next().unwrap_or(""));
        let prefix = format!("/{}", self.token);
        let Some(rest) = p.strip_prefix(&prefix) else { return Ok(None) };
        if !(rest.is_empty() || rest.starts_with('/')) { return Ok(None); }
        let mut out = self.fs()?.root().to_path_buf();
        for part in rest.split('/').filter(|s| !s.is_empty()) {
            // '\\' and ':' would let a Windows path escape the folder (other drive, ADS, UNC).
            anyhow::ensure!(part != ".." && part != "." && !part.contains(['\\', ':', '\0']), HttpError(400));
            out.push(part);
        }
        Ok(Some(out))
    }

    fn href(&self, p: &Path) -> String {
        let root = self.fs.as_ref().map(|f| f.root().to_path_buf()).unwrap_or_default();
        let rel = p.strip_prefix(&root).unwrap_or(Path::new(""));
        let mut h = format!("/{}", self.token);
        for c in rel.iter() { h.push('/'); h.push_str(&percent_encode(&c.to_string_lossy())); }
        h
    }

    fn handle(&mut self, req: &Request, r: &mut BufReader<TcpStream>, body: &mut Body, w: &mut TcpStream) -> Result<Option<Resp>> {
        if req.method == "OPTIONS" {
            let mut resp = Resp::new(200);
            resp.headers.push(("DAV", "1,2".into()));
            resp.headers.push(("MS-Author-Via", "DAV".into()));
            resp.headers.push(("Allow", "OPTIONS, GET, HEAD, PUT, DELETE, PROPFIND, PROPPATCH, MKCOL, MOVE, COPY, LOCK, UNLOCK".into()));
            return Ok(Some(resp));
        }
        let Some(path) = self.local_path(&req.path)? else { return Ok(Some(Resp::new(404))) };
        match req.method.as_str() {
            "PROPFIND" => {
                let depth = req.header("Depth").unwrap_or("1").to_string();
                let fs = self.fs()?;
                let st = fs.stat(&path)?;
                let dir = st.dir;
                let mut entries = vec![(path.clone(), st)];
                if dir && depth != "0" { entries.extend(fs.list(&path)?); }
                let mut x = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\">");
                for (p, s) in &entries { x.push_str(&prop_xml(&self.href(p), p, s)); }
                x.push_str("</D:multistatus>");
                Ok(Some(Resp::xml(207, x)))
            }
            "GET" | "HEAD" => {
                let fs = self.fs()?;
                let st = fs.stat(&path)?;
                if st.dir { return Ok(Some(Resp::new(405))); }
                let (start, end) = match req.header("Range").and_then(|v| parse_range(v, st.size)) {
                    Some(Ok(rg)) => rg,
                    Some(Err(())) => return Ok(Some(Resp::new(416))),
                    None => (0, st.size),
                };
                let partial = (start, end) != (0, st.size);
                let mut head = format!("HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nLast-Modified: {}\r\nAccept-Ranges: bytes\r\n",
                    if partial { 206 } else { 200 }, if partial { "Partial Content" } else { "OK" }, end - start, http_date(st.mtime));
                if partial { head.push_str(&format!("Content-Range: bytes {start}-{}/{}\r\n", end - 1, st.size)); }
                head.push_str("\r\n");
                body.drain(r)?;
                w.write_all(head.as_bytes())?;
                if req.method == "GET" {
                    let mut f = fs.open(&path)?;
                    if start > 0 { f.seek(SeekFrom::Start(start))?; }
                    std::io::copy(&mut f.take(end - start), w)?;
                }
                w.flush()?;
                Ok(None)
            }
            "PUT" => {
                let fs = self.fs()?;
                let existed = fs.stat(&path).is_ok();
                let mut f = fs.create(&path)?;
                body.copy_to(r, &mut f)?;
                f.flush()?;
                Ok(Some(Resp::new(if existed { 204 } else { 201 })))
            }
            "MKCOL" => {
                if body.remaining() > 0 { return Ok(Some(Resp::new(415))); }
                self.fs()?.mkdir(&path)?;
                Ok(Some(Resp::new(201)))
            }
            "DELETE" => {
                self.fs()?.remove_all(&path)?;
                Ok(Some(Resp::new(204)))
            }
            "MOVE" | "COPY" => {
                let dest = req.header("Destination").context(HttpError(400))?;
                let dest_path = dest.find("://").map(|i| &dest[i + 3..]).map(|s| s.find('/').map(|j| &s[j..]).unwrap_or("/")).unwrap_or(dest);
                let Some(to) = self.local_path(dest_path)? else { return Ok(Some(Resp::new(403))) };
                let overwrite = req.header("Overwrite").is_none_or(|v| !v.eq_ignore_ascii_case("F"));
                let fs = self.fs()?;
                let existed = fs.stat(&to).is_ok();
                if existed {
                    if !overwrite { return Ok(Some(Resp::new(412))); }
                    fs.remove_all(&to)?;
                }
                if req.method == "MOVE" { fs.rename(&path, &to)?; } else { fs.copy_all(&path, &to)?; }
                Ok(Some(Resp::new(if existed { 204 } else { 201 })))
            }
            "PROPPATCH" => {
                let text = body.read_text(r)?;
                self.fs()?.stat(&path)?;
                // Accept Windows' timestamp/attribute updates without applying them (the source keeps its own).
                let mut props = String::new();
                for name in ["Win32CreationTime", "Win32LastAccessTime", "Win32LastModifiedTime", "Win32FileAttributes"] {
                    if text.contains(name) { props.push_str(&format!("<Z:{name}/>")); }
                }
                let x = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\" xmlns:Z=\"urn:schemas-microsoft-com:\"><D:response><D:href>{}</D:href><D:propstat><D:prop>{props}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>", xml_escape(&req.path));
                Ok(Some(Resp::xml(207, x)))
            }
            "LOCK" => {
                body.read_text(r)?;
                let token = format!("opaquelocktoken:{}", wsm_core::provider::random_hex(16));
                let x = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:prop xmlns:D=\"DAV:\"><D:lockdiscovery><D:activelock><D:locktype><D:write/></D:locktype><D:lockscope><D:exclusive/></D:lockscope><D:depth>0</D:depth><D:timeout>Second-3600</D:timeout><D:locktoken><D:href>{token}</D:href></D:locktoken></D:activelock></D:lockdiscovery></D:prop>");
                let mut resp = Resp::xml(200, x);
                resp.headers.push(("Lock-Token", format!("<{token}>")));
                Ok(Some(resp))
            }
            "UNLOCK" => Ok(Some(Resp::new(204))),
            _ => Ok(Some(Resp::new(405))),
        }
    }
}

#[derive(Debug)]
struct HttpError(u16);
impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "HTTP {}", self.0) }
}

fn status_of(e: &anyhow::Error) -> u16 {
    if let Some(h) = e.downcast_ref::<HttpError>() { return h.0; }
    if let Some(s) = e.downcast_ref::<ssh2::Error>() {
        // libssh2 SFTP status codes: 2 no such file, 3 permission denied, 11 file exists, 18 dir not empty
        return match s.code() {
            ssh2::ErrorCode::SFTP(2) | ssh2::ErrorCode::SFTP(10) => 404,
            ssh2::ErrorCode::SFTP(3) => 403,
            ssh2::ErrorCode::SFTP(11) => 405,
            ssh2::ErrorCode::SFTP(18) => 409,
            _ => 500,
        };
    }
    if let Some(io) = e.downcast_ref::<std::io::Error>() {
        return match io.kind() {
            std::io::ErrorKind::NotFound => 404,
            std::io::ErrorKind::PermissionDenied => 403,
            std::io::ErrorKind::AlreadyExists => 405,
            std::io::ErrorKind::DirectoryNotEmpty => 409,
            _ => 500,
        };
    }
    500
}

fn prop_xml(href: &str, p: &Path, s: &Meta) -> String {
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let kind = if s.dir {
        "<D:resourcetype><D:collection/></D:resourcetype>".to_string()
    } else {
        format!("<D:resourcetype/><D:getcontentlength>{}</D:getcontentlength><D:getcontenttype>application/octet-stream</D:getcontenttype>", s.size)
    };
    let href = if s.dir && !href.ends_with('/') { format!("{href}/") } else { href.to_string() };
    // Dot files are hidden in Explorer, as in Finder.
    let attrs = if name.starts_with('.') { 0x2 } else { 0 } | if s.dir { 0x10 } else { 0x20 };
    format!("<D:response><D:href>{}</D:href><D:propstat><D:prop><D:displayname>{}</D:displayname>{kind}\
             <D:getlastmodified>{}</D:getlastmodified><D:creationdate>{}</D:creationdate>\
             <Z:Win32FileAttributes xmlns:Z=\"urn:schemas-microsoft-com:\">{attrs:08X}</Z:Win32FileAttributes>\
             </D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
        xml_escape(&href), xml_escape(&name), http_date(s.mtime), iso_date(s.mtime))
}

// ---------------------------------------------------------------------------------------- HTTP

fn read_head(r: &mut BufReader<TcpStream>) -> Result<Option<Request>> {
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 { return Ok(None); }
        if !line.trim().is_empty() { break; }
    }
    let mut it = line.split_whitespace();
    let (method, path) = (it.next().unwrap_or("").to_string(), it.next().unwrap_or("/").to_string());
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        anyhow::ensure!(r.read_line(&mut h)? > 0, "connection closed in headers");
        let h = h.trim_end();
        if h.is_empty() { break; }
        if let Some((k, v)) = h.split_once(':') { headers.push((k.trim().to_string(), v.trim().to_string())); }
        anyhow::ensure!(headers.len() < 100, "too many headers");
    }
    Ok(Some(Request { method, path, headers }))
}

/// Request body reader: Content-Length or chunked; whatever the handler doesn't read is drained.
enum Body {
    Length(u64),
    Chunked { left: u64, done: bool },
}

impl Body {
    fn new(req: &Request) -> Result<Body> {
        if req.header("Transfer-Encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
            return Ok(Body::Chunked { left: 0, done: false });
        }
        Ok(Body::Length(req.header("Content-Length").map(|v| v.parse()).transpose()?.unwrap_or(0)))
    }

    fn remaining(&self) -> u64 {
        match self { Body::Length(n) => *n, Body::Chunked { done, .. } => u64::from(!*done) }
    }

    fn read(&mut self, r: &mut BufReader<TcpStream>, buf: &mut [u8]) -> Result<usize> {
        match self {
            Body::Length(n) => {
                if *n == 0 { return Ok(0); }
                let cap = (*n).min(buf.len() as u64) as usize;
                let k = r.read(&mut buf[..cap])?;
                anyhow::ensure!(k > 0, "connection closed in body");
                *n -= k as u64;
                Ok(k)
            }
            Body::Chunked { left, done } => {
                if *done { return Ok(0); }
                if *left == 0 {
                    let mut l = String::new();
                    r.read_line(&mut l)?;
                    *left = u64::from_str_radix(l.trim().split(';').next().unwrap_or("0"), 16)?;
                    if *left == 0 {
                        loop { let mut t = String::new(); if r.read_line(&mut t)? == 0 || t.trim().is_empty() { break; } }
                        *done = true;
                        return Ok(0);
                    }
                }
                let cap = (*left).min(buf.len() as u64) as usize;
                let k = r.read(&mut buf[..cap])?;
                anyhow::ensure!(k > 0, "connection closed in body");
                *left -= k as u64;
                if *left == 0 { let mut crlf = String::new(); r.read_line(&mut crlf)?; }
                Ok(k)
            }
        }
    }

    fn copy_to(&mut self, r: &mut BufReader<TcpStream>, out: &mut impl Write) -> Result<()> {
        let mut buf = vec![0u8; 256 << 10];
        loop {
            let n = self.read(r, &mut buf)?;
            if n == 0 { return Ok(()); }
            out.write_all(&buf[..n])?;
        }
    }

    fn read_text(&mut self, r: &mut BufReader<TcpStream>) -> Result<String> {
        let mut v = Vec::new();
        self.copy_to(r, &mut v)?;
        Ok(String::from_utf8_lossy(&v).into_owned())
    }

    fn drain(&mut self, r: &mut BufReader<TcpStream>) -> Result<()> {
        self.copy_to(r, &mut std::io::sink())
    }
}

fn write_resp(w: &mut TcpStream, resp: &Resp, head_only: bool) -> Result<()> {
    let mut h = format!("HTTP/1.1 {} {}\r\nContent-Length: {}\r\n", resp.status, reason(resp.status), resp.body.len());
    for (k, v) in &resp.headers { h.push_str(&format!("{k}: {v}\r\n")); }
    h.push_str("\r\n");
    w.write_all(h.as_bytes())?;
    if !head_only { w.write_all(&resp.body)?; }
    w.flush()?;
    Ok(())
}

/// `bytes=a-b`, `bytes=a-`, `bytes=-n` -> half-open [start, end); Err for unsatisfiable.
fn parse_range(v: &str, size: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = v.strip_prefix("bytes=")?;
    if spec.contains(',') { return None; }
    let (a, b) = spec.split_once('-')?;
    let r = match (a.trim(), b.trim()) {
        ("", n) => { let n: u64 = n.parse().ok()?; (size.saturating_sub(n), size) }
        (a, "") => (a.parse().ok()?, size),
        (a, b) => (a.parse().ok()?, b.parse::<u64>().ok()?.saturating_add(1).min(size)),
    };
    Some(if r.0 >= r.1 || r.0 >= size { Err(()) } else { Ok(r) })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(s: &str) -> String {
    let mut o = String::new();
    for &c in s.as_bytes() {
        if c.is_ascii_alphanumeric() || b"-._~".contains(&c) { o.push(c as char); } else { o.push_str(&format!("%{c:02X}")); }
    }
    o
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// (year, month, day, hh, mm, ss, weekday 0=Sunday) from Unix seconds (UTC).
fn civil(t: u64) -> (i64, u32, u32, u64, u64, u64, usize) {
    let days = (t / 86400) as i64;
    let secs = t % 86400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, secs / 3600, secs % 3600 / 60, secs % 60, ((days + 4).rem_euclid(7)) as usize)
}

fn http_date(t: u64) -> String {
    const WD: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MO: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (y, m, d, hh, mm, ss, wd) = civil(t);
    format!("{}, {d:02} {} {y} {hh:02}:{mm:02}:{ss:02} GMT", WD[wd], MO[m as usize - 1])
}

fn iso_date(t: u64) -> String {
    let (y, m, d, hh, mm, ss, _) = civil(t);
    format!("{y}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(http_date(1_793_000_000), "Mon, 26 Oct 2026 07:33:20 GMT");
        assert_eq!(iso_date(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn ranges_and_escaping() {
        assert_eq!(parse_range("bytes=0-9", 100), Some(Ok((0, 10))));
        assert_eq!(parse_range("bytes=90-", 100), Some(Ok((90, 100))));
        assert_eq!(parse_range("bytes=-10", 100), Some(Ok((90, 100))));
        assert_eq!(parse_range("bytes=100-", 100), Some(Err(())));
        assert_eq!(percent_decode("/a%20b/%E2%9C%93"), "/a b/\u{2713}");
        assert_eq!(percent_encode("a b&\u{2713}"), "a%20b%26%E2%9C%93");
        assert_eq!(percent_decode("%2"), "%2");
    }
}
