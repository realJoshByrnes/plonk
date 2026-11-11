//! Wine's real wineserver (i386 Linux build in wine/out32) starts under the Linux personality and waits for clients.
use plonk_host::linux::LinuxProc;

#[test]
fn wineserver_reaches_main_loop() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let Ok(elf) = std::fs::read(format!("{root}wine/out32/wineserver")) else { eprintln!("skipped: no wine/out32"); return };
    let mut p = LinuxProc::new();
    for (dir, pre) in [("tests/linux/root", "/lib/"), ("wine/out32/nls", "/usr/share/wine/nls/")] {
        for e in std::fs::read_dir(format!("{root}{dir}")).unwrap().flatten() { p.files.insert(format!("{pre}{}", e.file_name().to_string_lossy()), std::fs::read(e.path()).unwrap()); }
    }
    p.mkdir("/home/user/.wine");
    p.load(&elf, &["/wine/wineserver", "-f", "-d"], &["HOME=/home/user", "WINEPREFIX=/home/user/.wine"]).unwrap();
    let r = p.run(2_000_000_000);
    let out = String::from_utf8_lossy(&p.stdout).into_owned();
    assert_eq!(r, Ok(None), "{out}");
    assert!(p.blocked, "should be waiting in poll; unknown {:?}", p.unknown);
    assert!(out.contains("wineserver: starting"), "{out}");
    let made = p.created();
    assert!(made.iter().any(|f| f.starts_with("/tmp/.wine-1000/server-") && f.ends_with("/socket")), "{made:?}");
}
