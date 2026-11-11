//! Run a Linux i386 ELF under the Linux personality: cargo run --release -p plonk-host --example linuxrun -- <elf> [args...]
//! Files come from tests/linux/root (/lib) and wine/out32 (/wine). TRACE=1 prints the syscall trace tail.
use plonk_host::linux::LinuxProc;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let mut p = LinuxProc::new();
    for (dir, pre) in [("tests/linux/root", "/lib/"), ("wine/out32", "/wine/")] {
        if let Ok(rd) = std::fs::read_dir(format!("{root}{dir}")) {
            for e in rd.flatten() { p.files.insert(format!("{pre}{}", e.file_name().to_string_lossy()), std::fs::read(e.path()).unwrap()); }
        }
    }
    let elf = std::fs::read(&a[1]).expect("read elf");
    let argv: Vec<&str> = a[1..].iter().map(|s| s.as_str()).collect();
    p.load(&elf, &argv, &["HOME=/home/user", "WINEPREFIX=/home/user/.wine", "LD_LIBRARY_PATH=/lib:/wine"]).unwrap();
    let r = p.run(2_000_000_000);
    println!("result {r:?}\nstdout: {}", String::from_utf8_lossy(&p.stdout));
    println!("unknown: {:?}", p.unknown);
    let n: usize = std::env::var("TRACE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    for l in p.trace.iter().skip(p.trace.len().saturating_sub(n)) { println!("{l}"); }
}
