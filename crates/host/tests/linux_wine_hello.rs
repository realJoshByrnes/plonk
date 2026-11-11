//! A Windows console program runs through real, unmodified Wine: the i386 Linux `wine` loader, its own wineserver (spawned by
//! wine itself), ntdll.so and the PE DLLs, all inside the interpreter under the Linux personality.
use plonk_host::linux::LinuxProc;

#[test]
fn hello_exe_through_real_wine() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    if !std::path::Path::new(&format!("{root}wine/out32/wine")).exists() || !std::path::Path::new(&format!("{root}wine/out/kernel32.dll")).exists() {
        eprintln!("skipped: needs wine/out and wine/out32");
        return;
    }
    let mut p = LinuxProc::new();
    let mut add = |dir: &str, pre: &str, want: &dyn Fn(&str) -> bool| {
        for e in std::fs::read_dir(format!("{root}{dir}")).unwrap().flatten().filter(|e| e.path().is_file()) {
            let name = e.file_name().to_string_lossy().into_owned();
            if want(&name) { p.files.insert(format!("{pre}{name}"), std::fs::read(e.path()).unwrap()); }
        }
    };
    add("tests/linux/root", "/lib/", &|_| true);
    add("wine/out32", "/usr/bin/", &|n| !n.ends_with(".so"));
    add("wine/out32", "/usr/lib/wine/i386-unix/", &|n| n.ends_with(".so"));
    add("wine/out", "/usr/lib/wine/i386-windows/", &|n| ["ntdll.dll", "kernel32.dll", "kernelbase.dll", "ucrtbase.dll", "msvcrt.dll", "advapi32.dll", "sechost.dll", "apisetschema.dll"].contains(&n));
    add("wine/out32/nls", "/usr/share/wine/nls/", &|_| true);
    add("tests/guests", "/home/user/", &|n| n == "hello.exe");
    p.mkdir("/home/user/.wine");
    let elf = p.files["/usr/bin/wine"].clone();
    p.load(&elf, &["/usr/bin/wine", "/home/user/hello.exe"], &["HOME=/home/user", "WINEPREFIX=/home/user/.wine", "PATH=/usr/bin:/bin", "WINELOADERNOEXEC=1", "WINEBOOTSTRAPMODE=1"]).unwrap();
    let r = p.run(4_000_000_000);
    let out = String::from_utf8_lossy(&p.stdout).into_owned();
    assert_eq!(r, Ok(Some(0)), "{out}");
    assert!(out.contains("Hello from Plonk"), "{out}");
}
