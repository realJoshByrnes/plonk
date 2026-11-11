//! Run a Linux i386 ELF under the Linux personality: cargo run --release -p plonk-host --example linuxrun -- <guest path> [args...]
//! The guest file tree is laid out like a Wine install: /lib (tests/linux/root), /usr/bin + /usr/lib/wine/i386-unix (wine/out32),
//! /usr/lib/wine/i386-windows (wine/out), /usr/share/wine/nls, /home/user (tests/guests). TRACE=n prints the last n trace lines,
//! IDLE=n lets n idle seconds pass, ENV="A=1;B=2" adds environment variables.
use plonk_host::linux::LinuxProc;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let mut p = LinuxProc::new();
    for (dir, pre) in [("tests/linux/root", "/lib/"), ("wine/out32", "/usr/bin/"), ("wine/out32", "/usr/lib/wine/i386-unix/"), ("wine/out", "/usr/lib/wine/i386-windows/"),
        ("wine/out32/nls", "/usr/share/wine/nls/"), ("tests/guests", "/home/user/"), ("tests/linux", "/bin/")] {
        if let Ok(rd) = std::fs::read_dir(format!("{root}{dir}")) {
            for e in rd.flatten().filter(|e| e.path().is_file()) {
                let name = e.file_name().to_string_lossy().into_owned();
                let unix_lib = name.ends_with(".so");
                if (pre == "/usr/bin/" && unix_lib) || (pre == "/usr/lib/wine/i386-unix/" && !unix_lib) || name.ends_with(".c") { continue; }
                p.files.insert(format!("{pre}{name}"), std::fs::read(e.path()).unwrap());
            }
        }
    }
    p.mkdir("/home/user/.wine");
    let elf = p.files.get(&a[1]).cloned().unwrap_or_else(|| std::fs::read(&a[1]).expect("read elf"));
    let argv: Vec<&str> = a[1..].iter().map(|s| s.as_str()).collect();
    let extra = std::env::var("ENV").unwrap_or_default();
    let mut env = vec!["HOME=/home/user", "WINEPREFIX=/home/user/.wine", "PATH=/usr/bin:/bin", "WINELOADERNOEXEC=1", "USER=user"];
    env.extend(extra.split(';').filter(|s| !s.is_empty()));
    p.load(&elf, &argv, &env).unwrap();
    // run; whenever everything blocks, let a second of guest time pass (up to IDLE rounds)
    let idle: u32 = std::env::var("IDLE").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    let mut r = p.run(2_000_000_000);
    let mut rounds = 0;
    while matches!(r, Ok(None)) && p.blocked && rounds < idle { p.advance(1000); rounds += 1; r = p.run(2_000_000_000); }
    println!("result {r:?} blocked={} after {rounds} idle seconds", p.blocked);
    println!("stdout: {}", String::from_utf8_lossy(&p.stdout));
    println!("unknown: {:?}", p.unknown);
    let n: usize = std::env::var("TRACE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let pat = std::env::var("GREP").unwrap_or_default();
    let lines: Vec<&String> = p.trace.iter().filter(|l| l.contains(pat.as_str())).collect();
    for l in lines.iter().skip(lines.len().saturating_sub(n)) { println!("{l}"); }
}
