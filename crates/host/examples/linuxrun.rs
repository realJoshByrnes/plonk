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
    // ROOT=1: use the full Wine install from wine/root.tar (/usr/local/...) instead of the hand-picked files above
    if std::env::var("ROOT").is_ok() {
        p.files.retain(|k, _| !k.starts_with("/usr/"));
        p.add_tar(&std::fs::read(format!("{root}wine/root.tar")).expect("wine/root.tar"));
    }
    p.mkdir("/home/user/.wine");
    // DIR="hostdir=/guest/dir": add a host directory (not recursive) to the guest file tree
    if let Ok(d) = std::env::var("DIR") {
        let (h, g) = d.split_once('=').expect("DIR=host=guest");
        for e in std::fs::read_dir(h).expect("DIR host dir").flatten().filter(|e| e.path().is_file()) { p.files.insert(format!("{g}/{}", e.file_name().to_string_lossy()), std::fs::read(e.path()).unwrap()); }
    }
    // PREFIX=file.tar: start from a saved Wine prefix; SAVE=file.tar: save the prefix afterwards (waits for all processes to end)
    if let Ok(f) = std::env::var("PREFIX") { p.add_tar(&std::fs::read(f).expect("prefix tar")); }
    p.wait_all = std::env::var("SAVE").is_ok();
    let elf = p.files.get(&a[1]).cloned().unwrap_or_else(|| std::fs::read(&a[1]).expect("read elf"));
    let argv: Vec<&str> = a[1..].iter().map(|s| s.as_str()).collect();
    let extra = std::env::var("ENV").unwrap_or_default();
    let mut env = vec!["HOME=/home/user", "WINEPREFIX=/home/user/.wine", "PATH=/usr/local/bin:/usr/bin:/bin", "WINELOADERNOEXEC=1", "USER=user"];
    env.extend(extra.split(';').filter(|s| !s.is_empty()));
    p.load(&elf, &argv, &env).unwrap();
    p.auto_time = std::env::var("AUTO").is_ok(); // jump the guest clock whenever everything waits on a timeout
    // run; whenever everything blocks, let a second of guest time pass (up to IDLE rounds)
    let idle: u32 = std::env::var("IDLE").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    let steps: u64 = std::env::var("STEPS").ok().and_then(|v| v.parse().ok()).unwrap_or(2_000_000_000);
    let mut r = p.run(steps);
    let mut rounds = 0;
    while matches!(r, Ok(None)) && p.blocked && rounds < idle { p.advance(1000); rounds += 1; r = p.run(2_000_000_000); }
    // EVENTS="t,ms;d,x,y;u,x,y;m,x,y;k,vk": scripted input after startup (t = let guest time pass), each followed by a run
    for e in std::env::var("EVENTS").unwrap_or_default().split(';').filter(|s| !s.is_empty()) {
        let f: Vec<i32> = e.split(',').skip(1).map(|v| v.parse().unwrap()).collect();
        match &e[..1] {
            "t" => p.advance(f[0] as u64),
            "d" => p.mouse(0, f[0], f[1]),
            "u" => p.mouse(2, f[0], f[1]),
            "m" => p.mouse(1, f[0], f[1]),
            "k" => { p.push_event(5, 0, 0, 0, f[0] as u32, 0); p.push_event(6, 0, 0, 0, f[0] as u32, 0); }
            _ => {}
        }
        r = p.run(steps);
        for _ in 0..3 { if matches!(r, Ok(None)) && p.blocked { p.advance(200); r = p.run(steps); } }
    }
    println!("result {r:?} blocked={} after {rounds} idle seconds", p.blocked);
    println!("stdout: {}", String::from_utf8_lossy(&p.stdout));
    println!("unknown: {:?}", p.unknown);
    if let Ok(f) = std::env::var("SAVE") { let t = p.export_tar("/home/user/.wine"); println!("saved prefix: {} bytes", t.len()); std::fs::write(f, t).unwrap(); }
    for h in &p.zorder { let w = &p.windows[h]; println!("window {:#x} {:?} visible={} surface {}x{} {:?}", w.hwnd, w.rect, w.visible, w.sw, w.sh, w.title); }
    if let Ok(f) = std::env::var("SHOT") { // SHOT=out.ppm: screenshot of the composed windows
        let nl = char::from(10);
        let mut out = format!("P6{nl}{} {}{nl}255{nl}", p.screen.0, p.screen.1).into_bytes();
        out.extend(p.compose_rgb());
        std::fs::write(f, out).unwrap();
    }
    if std::env::var("PS").is_ok() { print!("{}", p.ps()); }
    let n: usize = std::env::var("TRACE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let pat = std::env::var("GREP").unwrap_or_default();
    let lines: Vec<&String> = p.trace.iter().filter(|l| l.contains(pat.as_str())).collect();
    for l in lines.iter().skip(lines.len().saturating_sub(n)) { println!("{l}"); }
}
