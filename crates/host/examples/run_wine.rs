//! cargo run -q -p plonk-host --release --example run_wine -- <exe> [wine/out dir]
use plonk_host::wine::WineProc;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = a.get(2).cloned().unwrap_or("wine/out".into());
    let mut p = WineProc::new();
    for d in ["ntdll", "kernel32", "kernelbase", "win32u", "user32", "gdi32"] {
        if let Ok(b) = std::fs::read(format!("{dir}/{d}.dll")) {
            p.files.insert(format!("c:\\windows\\system32\\{d}.dll"), b);
        }
    }
    if let Ok(rd) = std::fs::read_dir(format!("{dir}/nls")) {
        for e in rd.flatten() {
            p.files.insert(format!("nls\\{}", e.file_name().to_string_lossy()), std::fs::read(e.path()).unwrap());
        }
    }
    p.files.insert("c:\\hello.exe".into(), std::fs::read(&a[1]).unwrap());
    let r = p.boot("c:\\hello.exe").and_then(|_| p.run(100_000));
    let n = p.trace.len();
    for l in p.trace.iter().skip(n.saturating_sub(12)) {
        println!("  {l}");
    }
    println!("result: {:?} stdout={:?} eip={:#x}", r, String::from_utf8_lossy(&p.stdout), p.cpu.eip);
}
