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

    // RESTORE=file: start from a snapshot, then start the program inside that session
    let restored = if let Ok(f) = std::env::var("RESTORE") { if let Err(e) = p.restore(&std::fs::read(f).expect("snapshot")) { println!("restore failed: {e}"); std::process::exit(1); } true } else { false };
    if let Ok(f) = std::env::var("PREFIX2") { p.add_tar(&std::fs::read(f).expect("prefix2 tar")); } // added after RESTORE (does not change the snapshot fingerprint)
    let argv: Vec<&str> = a[1..].iter().map(|s| s.as_str()).collect();
    let extra = std::env::var("ENV").unwrap_or_default();
    // STD=1: the standard Wine environment of the web page (overrides that skip slow start-up work) instead of a bare one
    let mut env = if std::env::var("STD").is_ok() { plonk_host::linux::WINE_ENV.to_vec() } else { vec!["HOME=/home/user", "WINEPREFIX=/home/user/.wine", "PATH=/usr/local/bin:/usr/bin:/bin", "WINELOADERNOEXEC=1", "USER=user"] };
    let extra: Vec<&str> = extra.split(';').filter(|s| !s.is_empty()).collect();
    env.retain(|e| !extra.iter().any(|x| x.split('=').next() == e.split('=').next())); // ENV overrides the standard variables
    env.extend(extra);
    // PRE="prog arg;prog arg": programs run to completion (until everything idles) in the restored session before the main program
    for pre in std::env::var("PRE").unwrap_or_default().split(';').filter(|s| !s.is_empty()) {
        let a: Vec<&str> = pre.split(' ').collect();
        p.wait_all = true;
        p.spawn_path(&a, &env).expect("PRE program");
        let _ = p.run(2_000_000_000);
    }
    // the program is a guest path, or failing that a host file
    if p.spawn_path(&argv, &env).is_err() { p.load(&std::fs::read(&a[1]).expect("program not found in the guest tree or on the host"), &argv, &env).unwrap(); }
    p.wait_all = restored || std::env::var("SAVE").is_ok() || std::env::var("WAIT").is_ok();
    p.auto_time = std::env::var("AUTO").is_ok(); // jump the guest clock whenever everything waits on a timeout
    // run; whenever everything blocks, let a second of guest time pass (up to IDLE rounds)
    let idle: u32 = std::env::var("IDLE").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    let steps: u64 = std::env::var("STEPS").ok().and_then(|v| v.parse().ok()).unwrap_or(2_000_000_000);
    if std::env::var("PROF").is_ok() { p.prof = Some(Default::default()); } // PROF=1: sample eips / count syscalls; event "z" resets
    let mut r = p.run(steps);
    // RT=seconds: run against the real clock, like the browser page does (guest time = wall time)
    if let Some(secs) = std::env::var("RT").ok().and_then(|v| v.parse::<f64>().ok()) {
        p.clock_step_ns = 0;
        if p.prof.is_some() { p.prof = Some(Default::default()); } // profile only the real-time phase
        let (t0, mut last, mut busy, mut shown) = (std::time::Instant::now(), std::time::Instant::now(), std::time::Duration::ZERO, false);
        while t0.elapsed().as_secs_f64() < secs && !matches!(r, Ok(Some(_)) | Err(_)) {
            let now = std::time::Instant::now();
            p.advance_ns((now - last).as_nanos() as u64);
            last = now;
            r = p.run(20_000_000);
            busy += now.elapsed();
            if !shown && p.windows.values().any(|w| w.visible && !w.pixels.is_empty() && w.sw > 50) { shown = true; println!("first window after {:.1}s wall, {:.1}s busy, {} M instructions", t0.elapsed().as_secs_f64(), busy.as_secs_f64(), p.retired() / 1_000_000); }
            if p.blocked { std::thread::sleep(std::time::Duration::from_millis(2)); }
        }
        println!("real-time run: {:.1}s wall, {:.1}s busy, {} M instructions", t0.elapsed().as_secs_f64(), busy.as_secs_f64(), p.retired() / 1_000_000);
    }
    let mut rounds = 0;
    while matches!(r, Ok(None)) && p.blocked && rounds < idle { p.advance(1000); rounds += 1; r = p.run(2_000_000_000); }
    // EVENTS="t,ms;d,x,y;u,x,y;m,x,y;k,vk": scripted input after startup (t = let guest time pass), each followed by a run
    for e in std::env::var("EVENTS").unwrap_or_default().split(';').filter(|s| !s.is_empty()) {
        let f: Vec<i32> = e.split(',').skip(1).map(|v| v.parse().unwrap()).collect();
        match &e[..1] {
            "t" => p.advance(f[0] as u64),
            "z" => { if p.prof.is_some() { p.prof = Some(Default::default()); } println!("profile reset at {} M instructions", p.retired() / 1_000_000); }
            "d" => p.mouse(0, f[0], f[1]),
            "u" => p.mouse(2, f[0], f[1]),
            "m" => p.mouse(1, f[0], f[1]),
            "k" => { p.push_event(5, 0, 0, 0, f[0] as u32, 0); p.push_event(6, 0, 0, 0, f[0] as u32, 0); }
            _ => {}
        }
        r = p.run(steps);
        for _ in 0..3 { if matches!(r, Ok(None)) && p.blocked { p.advance(200); r = p.run(steps); } }
    }
    if let Some((eips, sys)) = &p.prof {
        let mut v: Vec<_> = eips.iter().collect(); v.sort_by_key(|x| std::cmp::Reverse(*x.1));
        println!("PROF top eips (pid, eip: instructions in 997-insn chunks starting there); total {}", eips.values().sum::<u64>());
        for (k, n) in v.iter().take(25) { println!("  pid {} {:#010x} {}", k.0, k.1, n); }
        let mut v: Vec<_> = sys.iter().collect(); v.sort_by_key(|x| std::cmp::Reverse(*x.1));
        println!("PROF top syscalls (pid, nr: count)");
        for (k, n) in v.iter().take(80) { println!("  pid {} nr {} {}", k.0, k.1, n); }
    }
    if let Ok(d) = std::env::var("PEEK") { // PEEK=pid:hexaddr:len: hex dump of guest memory
        let f: Vec<&str> = d.split(':').collect();
        println!("peek {d}: {}", p.peek(f[0].parse().unwrap(), u32::from_str_radix(f[1], 16).unwrap(), f[2].parse().unwrap()));
    }
    println!("result {r:?} blocked={} after {rounds} idle seconds", p.blocked);
    println!("stdout: {}", String::from_utf8_lossy(&p.stdout));
    println!("unknown: {:?}", p.unknown);
    // SNAP=file: save a snapshot of the whole session (take it when the program has exited and the rest is idle)
    if let Ok(f) = std::env::var("SNAP") {
        print!("{}", p.ps());
        let t = p.snapshot(); println!("snapshot: {} MB", t.len() >> 20);
        std::fs::write(format!("{f}.files"), p.pristine_files().join(&char::from(10).to_string()) + &char::from(10).to_string()).unwrap(); // what the page loads up front
        std::fs::write(f, t).unwrap();
    }
    // EXPORT=file.tar: export ~/.wine immediately (no clean shutdown; files only)
    if let Ok(f) = std::env::var("EXPORT") { std::fs::write(f, p.export_tar("/home/user/.wine")).unwrap(); }
    if std::env::var("SAVE").is_ok() { // shut Wine down cleanly so wineserver writes the registry files
        p.auto_time = true;
        let pid = p.spawn_path(&["/usr/local/bin/wine", "wineboot", "--shutdown"], &env);
        let r2 = p.run(30_000_000_000);
        println!("shutdown: {pid:?} {r2:?} blocked={}", p.blocked);
        print!("{}", p.ps());
    }
    if let Ok(f) = std::env::var("SAVE") { let t = p.export_tar("/home/user/.wine"); println!("saved prefix: {} bytes", t.len()); std::fs::write(f, t).unwrap(); }
    for h in &p.zorder { let w = &p.windows[h]; println!("window {:#x} {:?} visible={} surface {}x{} {:?}", w.hwnd, w.rect, w.visible, w.sw, w.sh, w.title); }
    if let Ok(f) = std::env::var("SHOT") { // SHOT=out.ppm: screenshot of the composed windows
        let nl = char::from(10);
        let mut out = format!("P6{nl}{} {}{nl}255{nl}", p.screen.0, p.screen.1).into_bytes();
        out.extend(p.compose_rgb());
        std::fs::write(f, out).unwrap();
    }
    if let Ok(f) = std::env::var("AUDIO") { println!("audio: {} bytes at {} Hz x{}", p.audio.out.len(), p.audio.rate, p.audio.ch); std::fs::write(f, &p.audio.out).unwrap(); }
    if std::env::var("PS").is_ok() { print!("{}", p.ps()); }
    let n: usize = std::env::var("TRACE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let pat = std::env::var("GREP").unwrap_or_default();
    let lines: Vec<&String> = p.trace.iter().filter(|l| l.contains(pat.as_str())).collect();
    for l in lines.iter().skip(lines.len().saturating_sub(n)) { println!("{l}"); }
}
