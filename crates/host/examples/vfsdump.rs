//! Run a guest under the lazy-file harness, then list the files it created in the VFS with sizes and CRC32 (debugging installers).
//! cargo run --release -p plonk-host --example vfsdump -- <exe> [steps]
use plonk_host::wine::{RunState, WineProc};

fn crc32(d: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in d {
        c ^= b as u32;
        for _ in 0..8 { c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 }; }
    }
    !c
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../wine/out");
    let mut p = WineProc::new();
    p.load_dir(dir);
    p.lenient = true;
    let base: std::collections::HashSet<String> = p.files.keys().cloned().collect();
    p.files.insert("c:\\app.exe".into(), std::fs::read(&a[1]).expect("read exe"));
    // EXTRA="vfs\path=host\path;..." adds more files to the VFS before boot
    if let Ok(ex) = std::env::var("EXTRA") {
        for kv in ex.split(';').filter(|s| !s.is_empty()) {
            let (k, v) = kv.split_once('=').unwrap();
            p.files.insert(k.to_string(), std::fs::read(v).expect("read extra"));
        }
    }
    p.boot("c:\\app.exe").unwrap();
    let mut st = p.run_until_idle(50_000_000);
    for _ in 0..40 {
        if let Ok(RunState::Busy) = st { st = p.run_until_idle(50_000_000); } else { break; }
    }
    // EVENTS="t,ms;d,x,y;u,x,y;k,vk,ch;..." scripted input (t = advance the guest clock), each followed by a run to idle
    if let Ok(ev) = std::env::var("EVENTS") {
        for e in ev.split(';').filter(|s| !s.is_empty()) {
            let f: Vec<&str> = e.split(',').collect();
            match f[0] {
                "t" => p.tick(f[1].parse().unwrap()),
                "d" => p.mouse(0, f[1].parse().unwrap(), f[2].parse().unwrap()),
                "u" => p.mouse(2, f[1].parse().unwrap(), f[2].parse().unwrap()),
                "k" => { let (vk, ch): (u32, u32) = (f[1].parse().unwrap(), f[2].parse().unwrap()); p.key(0, vk, ch); p.key(1, vk, 0); }
                _ => {}
            }
            st = p.run_until_idle(50_000_000);
            for _ in 0..200 { if let Ok(RunState::Busy) = st { st = p.run_until_idle(50_000_000); } else { break; } }
        }
    }
    if std::env::var("ALLWIN").is_ok() { println!("{}", p.debug_all_windows()); }
    if std::env::var("WINDOWS").is_ok() { println!("windows: {}", p.debug_windows()); }
    println!("state {:?}", st);
    println!("stdout: {}", String::from_utf8_lossy(&p.stdout));
    if std::env::var("STUBBED").is_ok() { println!("stubbed: {}", p.stubbed.join(" ")); }
    if std::env::var("OPENS").is_ok() { for l in p.trace.iter().filter(|l| l.contains("(open ")) { println!("{l}"); } }
    if std::env::var("MODS").is_ok() { for m in p.module_names() { println!("module {m}"); } }
    let mut names: Vec<&String> = p.files.keys().filter(|k| !base.contains(*k)).collect();
    names.sort();
    for n in names {
        let d = &p.files[n];
        if let Ok(dir) = std::env::var("DUMP") { let f = n.rsplit('\\').next().unwrap_or("f"); let _ = std::fs::write(format!("{dir}/{f}"), d); }
        println!("{:60} {:>9} bytes crc32 {:08x}  first {:02x?}", n, d.len(), crc32(d), &d[..d.len().min(8)]);
    }
}
