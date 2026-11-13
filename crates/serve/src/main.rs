//! Plonk!'s local web server: static files plus the compressed Wine archives the page asks for. Everything it serves comes from
//! this repository's disk; it receives nothing from the page except the file requests themselves.
//!
//!   cargo run --release -p plonk-serve [port]      then open http://localhost:8080/
//!
//! Routes (besides plain files under the repository root):
//!   /                      redirects to /web/real.html
//!   /real/root.tar, /real/prefix.tar, /real/snapshot.bin   gzip-compressed once, cached on disk next to the original
//!   /real/index            the listing of wine/root.tar (type, size, name, link target) without file bodies
//!   /real/boot.tar         the files listed in wine/snapshot.bin.files, as one tar archive
//!   /real/f/<path>         one file of wine/root.tar
//!   /manifest.json         the test programs in tests/guests
use flate2::{write::GzEncoder, Compression};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

struct RootIndex { key: String, by_name: HashMap<String, (u64, u64)>, text: String, boot: Option<(String, Vec<u8>)> }

struct State { root: PathBuf, index: Mutex<Option<RootIndex>> }

enum Body { Bytes(Vec<u8>), File(File, u64) }

struct Response { status: u16, headers: Vec<(String, String)>, body: Body }

impl Response {
    fn text(status: u16, msg: &str) -> Response { Response { status, headers: vec![("content-type".into(), "text/plain".into())], body: Body::Bytes(msg.as_bytes().to_vec()) } }
    fn not_modified() -> Response { Response { status: 304, headers: vec![], body: Body::Bytes(vec![]) } }
}

fn mime(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html", "js" | "mjs" => "text/javascript", "css" => "text/css", "wasm" => "application/wasm", "json" => "application/json",
        "png" => "image/png", "svg" => "image/svg+xml", "ico" => "image/x-icon", "txt" | "md" => "text/plain", _ => "application/octet-stream",
    }
}

fn gzip(data: &[u8], level: u32) -> Vec<u8> {
    let mut e = GzEncoder::new(Vec::with_capacity(data.len() / 3), Compression::new(level));
    e.write_all(data).expect("gzip");
    e.finish().expect("gzip")
}

fn mtime_ms(p: &Path) -> u128 { fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis()) }

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) { out.push(v); i += 3; continue; }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn gz_response(body: Vec<u8>, orig_len: usize, etag: String) -> Response {
    Response { status: 200, headers: vec![("content-type".into(), "application/octet-stream".into()), ("content-encoding".into(), "gzip".into()), ("etag".into(), etag), ("cache-control".into(), "no-cache".into()), ("x-size".into(), orig_len.to_string())], body: Body::Bytes(body) }
}

/// Parse wine/root.tar once per version of the file: where each regular file's bytes are, and the index text for the kernel.
fn load_index(st: &State) -> Option<()> {
    let src = st.root.join("wine/root.tar");
    let meta = fs::metadata(&src).ok()?;
    let key = format!("{}-{}", meta.len(), mtime_ms(&src));
    let mut g = st.index.lock().unwrap();
    if g.as_ref().map_or(false, |i| i.key == key) { return Some(()); }
    let mut f = BufReader::new(File::open(&src).ok()?);
    let (mut by_name, mut text) = (HashMap::new(), String::new());
    let (mut off, mut long): (u64, Option<String>) = (0, None);
    let mut h = [0u8; 512];
    let field = |b: &[u8]| { let z = b.iter().position(|&c| c == 0).unwrap_or(b.len()); String::from_utf8_lossy(&b[..z]).into_owned() };
    loop {
        f.seek(SeekFrom::Start(off)).ok()?;
        if f.read_exact(&mut h).is_err() || h[0] == 0 { break; }
        let size = u64::from_str_radix(field(&h[124..136]).trim(), 8).unwrap_or(0);
        let ty = if h[156] == 0 { '0' } else { h[156] as char };
        let body = off + 512;
        off = body + (size + 511) / 512 * 512;
        if ty == 'L' {
            let mut b = vec![0u8; size as usize];
            f.read_exact(&mut b).ok()?;
            long = Some(field(&b));
            continue;
        }
        let pre = field(&h[345..500]);
        let name = long.take().unwrap_or_else(|| if pre.is_empty() { field(&h[0..100]) } else { format!("{pre}/{}", field(&h[0..100])) });
        text.push_str(&format!("{ty}\t{size}\t{name}\t{}\n", field(&h[157..257])));
        if ty == '0' { by_name.insert(format!("/{}", name.trim_start_matches("./")), (body, size)); }
    }
    *g = Some(RootIndex { key, by_name, text, boot: None });
    Some(())
}

fn read_root_file(st: &State, off: u64, size: u64) -> io::Result<Vec<u8>> {
    let mut f = File::open(st.root.join("wine/root.tar"))?;
    f.seek(SeekFrom::Start(off))?;
    let mut b = vec![0u8; size as usize];
    f.read_exact(&mut b)?;
    Ok(b)
}

fn tar_header(name: &str, ty: u8, size: u64) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..name.len().min(100)].copy_from_slice(&name.as_bytes()[..name.len().min(100)]);
    h[100..108].copy_from_slice(b"0000644\0");
    h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    h[148..156].copy_from_slice(b"        ");
    h[156] = ty;
    h[257..265].copy_from_slice(b"ustar  \0");
    let sum: u32 = h.iter().map(|&b| b as u32).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

fn pad(out: &mut Vec<u8>) { while out.len() % 512 != 0 { out.push(0); } }

fn handle(st: &State, method: &str, target: &str, inm: Option<&str>) -> Response {
    let path = percent_decode(target.split('?').next().unwrap_or("/"));
    let _ = method;
    if path == "/" || path == "/real" { return Response { status: 302, headers: vec![("location".into(), "/web/real.html".into())], body: Body::Bytes(vec![]) }; }
    if path == "/manifest.json" {
        let mut apps: Vec<String> = fs::read_dir(st.root.join("tests/guests")).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".exe")).collect()).unwrap_or_default();
        apps.sort();
        let list = apps.iter().map(|a| format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\""))).collect::<Vec<_>>().join(",");
        return Response { status: 200, headers: vec![("content-type".into(), "application/json".into())], body: Body::Bytes(format!("{{\"core\":[],\"apps\":[{list}],\"remote\":[]}}").into_bytes()) };
    }
    if matches!(path.as_str(), "/real/root.tar" | "/real/prefix.tar" | "/real/snapshot.bin") {
        let name = &path[6..];
        let src = st.root.join("wine").join(name);
        let Ok(meta) = fs::metadata(&src) else { return Response::text(404, &format!("missing {name} (see wine/build32.sh)")) };
        let etag = format!("\"{}-{}\"", meta.len(), mtime_ms(&src));
        if inm == Some(etag.as_str()) { return Response::not_modified(); }
        let gz_path = st.root.join("wine").join(format!(".{name}-{}-{}.gz", meta.len(), mtime_ms(&src)));
        if !gz_path.exists() {
            let t0 = std::time::Instant::now();
            if let Ok(d) = fs::read_dir(st.root.join("wine")) { for e in d.flatten() { if e.file_name().to_string_lossy().starts_with(&format!(".{name}-")) { let _ = fs::remove_file(e.path()); } } }
            let Ok(data) = fs::read(&src) else { return Response::text(500, "read failed") };
            let _ = fs::write(&gz_path, gzip(&data, 6));
            println!("{path}: {} MB -> {} MB gzip in {} ms", data.len() >> 20, fs::metadata(&gz_path).map_or(0, |m| m.len()) >> 20, t0.elapsed().as_millis());
        }
        let Ok(f) = File::open(&gz_path) else { return Response::text(500, "cache failed") };
        let len = f.metadata().map_or(0, |m| m.len());
        return Response { status: 200, headers: vec![("content-type".into(), "application/octet-stream".into()), ("content-encoding".into(), "gzip".into()), ("etag".into(), etag), ("cache-control".into(), "no-cache".into()), ("x-size".into(), meta.len().to_string())], body: Body::File(f, len) };
    }
    if path == "/real/index" || path == "/real/boot.tar" || path.starts_with("/real/f/") {
        if load_index(st).is_none() { return Response::text(404, "missing root.tar (see wine/build32.sh)"); }
        let mut g = st.index.lock().unwrap();
        let ix = g.as_mut().unwrap();
        if path == "/real/index" {
            let etag = format!("\"ix-{}\"", ix.key);
            if inm == Some(etag.as_str()) { return Response::not_modified(); }
            return gz_response(gzip(ix.text.as_bytes(), 5), ix.text.len(), etag);
        }
        if path == "/real/boot.tar" {
            let list = st.root.join("wine/snapshot.bin.files");
            let Ok(names) = fs::read_to_string(&list) else { return Response::text(404, "no file list for the snapshot") };
            let lk = format!("{}-{}", ix.key, mtime_ms(&list));
            let etag = format!("\"boot-{lk}\"");
            if inm == Some(etag.as_str()) { return Response::not_modified(); }
            if ix.boot.as_ref().map_or(true, |b| b.0 != lk) {
                let mut tar = Vec::new();
                for n in names.lines().filter(|l| !l.is_empty()) {
                    let Some(&(off, size)) = ix.by_name.get(n) else { continue };
                    let Ok(data) = read_root_file(st, off, size) else { continue };
                    let mut nb = n[1..].as_bytes().to_vec(); // a GNU long-name record carries the path, so the name field never overflows
                    nb.push(0);
                    tar.extend_from_slice(&tar_header("././@LongLink", b'L', nb.len() as u64));
                    tar.extend_from_slice(&nb); pad(&mut tar);
                    tar.extend_from_slice(&tar_header("f", b'0', size));
                    tar.extend_from_slice(&data); pad(&mut tar);
                }
                tar.extend_from_slice(&[0u8; 1024]);
                let gz = gzip(&tar, 5);
                ix.boot = Some((lk, [tar.len().to_le_bytes().to_vec(), gz].concat()));
            }
            let b = &ix.boot.as_ref().unwrap().1;
            let orig = u64::from_le_bytes(b[..8].try_into().unwrap()) as usize;
            return gz_response(b[8..].to_vec(), orig, etag);
        }
        let Some(&(off, size)) = ix.by_name.get(&path[7..]) else { return Response::text(404, "no such file") };
        let etag = format!("\"f-{}-{off}\"", ix.key);
        if inm == Some(etag.as_str()) { return Response::not_modified(); }
        drop(g);
        let Ok(data) = read_root_file(st, off, size) else { return Response::text(500, "read failed") };
        return gz_response(gzip(&data, 5), data.len(), etag);
    }
    // plain files under the repository root
    let rel = path.trim_start_matches('/');
    if rel.split('/').any(|c| c == ".." || c.contains('\\') || c.contains(':')) { return Response::text(404, "not found"); }
    let p = st.root.join(rel);
    let Ok(meta) = fs::metadata(&p) else { return Response::text(404, "not found") };
    if !meta.is_file() { return Response::text(404, "not found"); }
    let etag = format!("\"{}-{}\"", meta.len(), mtime_ms(&p));
    if inm == Some(etag.as_str()) { return Response::not_modified(); }
    let Ok(f) = File::open(&p) else { return Response::text(404, "not found") };
    Response { status: 200, headers: vec![("content-type".into(), mime(&p).into()), ("etag".into(), etag), ("cache-control".into(), "no-cache".into())], body: Body::File(f, meta.len()) }
}

fn serve(mut stream: TcpStream, st: Arc<State>) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut rd = BufReader::new(stream.try_clone()?);
    loop {
        let mut line = String::new();
        if rd.read_line(&mut line)? == 0 { return Ok(()); }
        let mut it = line.split_whitespace();
        let (method, target) = (it.next().unwrap_or("").to_string(), it.next().unwrap_or("/").to_string());
        let (mut inm, mut close) = (None, false);
        loop {
            let mut h = String::new();
            if rd.read_line(&mut h)? == 0 || h.trim().is_empty() { break; }
            if let Some((k, v)) = h.split_once(':') {
                match k.trim().to_ascii_lowercase().as_str() { "if-none-match" => inm = Some(v.trim().to_string()), "connection" => close = v.trim().eq_ignore_ascii_case("close"), _ => {} }
            }
        }
        let r = if method == "GET" || method == "HEAD" { handle(&st, &method, &target, inm.as_deref()) } else { Response::text(405, "method not allowed") };
        let len = match &r.body { Body::Bytes(b) => b.len() as u64, Body::File(_, n) => *n };
        let reason = match r.status { 200 => "OK", 302 => "Found", 304 => "Not Modified", 404 => "Not Found", 405 => "Method Not Allowed", _ => "Error" };
        let mut head = format!("HTTP/1.1 {} {reason}\r\ncontent-length: {}\r\n", r.status, if r.status == 304 || r.status == 302 { 0 } else { len });
        for (k, v) in &r.headers { head.push_str(&format!("{k}: {v}\r\n")); }
        head.push_str(if close { "connection: close\r\n\r\n" } else { "connection: keep-alive\r\n\r\n" });
        stream.write_all(head.as_bytes())?;
        if method != "HEAD" && r.status != 304 && r.status != 302 {
            match r.body { Body::Bytes(b) => stream.write_all(&b)?, Body::File(mut f, _) => { io::copy(&mut f, &mut stream)?; } }
        }
        stream.flush()?;
        if close { return Ok(()); }
    }
}

fn main() {
    let port: u16 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(8080);
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let root = root.canonicalize().unwrap_or(root);
    let listener = match TcpListener::bind(("127.0.0.1", port)) { Ok(l) => l, Err(e) => { eprintln!("cannot listen on port {port}: {e}"); std::process::exit(1); } };
    println!("Plonk! is at http://localhost:{port}/  (serving {})", root.display());
    let st = Arc::new(State { root, index: Mutex::new(None) });
    for conn in listener.incoming().flatten() {
        let st = st.clone();
        std::thread::spawn(move || { let _ = serve(conn, st); });
    }
}
