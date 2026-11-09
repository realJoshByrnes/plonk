//! cargo run -q -p plonk-host --release --example run_wine -- <exe> [wine/out dir]
use plonk_host::wine::WineProc;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = a.get(2).cloned().unwrap_or("wine/out".into());
    let mut p = WineProc::new();
    p.load_dir(&dir);
    p.files.insert("c:\\hello.exe".into(), std::fs::read(&a[1]).unwrap());
    p.lenient = std::env::var("LENIENT").is_ok();
    if let Ok(v) = std::env::var("TRACE_RANGE") { let (a, b) = v.split_once('-').unwrap(); p.step_trace = Some((u32::from_str_radix(a, 16).unwrap(), u32::from_str_radix(b, 16).unwrap())); }
    let r = p.boot("c:\\hello.exe").and_then(|_| p.run(100_000));
    let n = p.trace.len();
    for l in p.trace.iter().skip(n.saturating_sub(std::env::var("N").ok().and_then(|v| v.parse().ok()).unwrap_or(12))) {
        println!("  {l}");
    }
    if let Ok(b) = p.mem.read_bytes(p.cpu.eip.wrapping_sub(8), 24) { println!("  code@eip-8: {:02x?}", b); }
    println!("  module: {:x?}", p.module_at(p.cpu.eip));
    println!("  regs: {:x?}", p.cpu.r);
    if !p.stubbed.is_empty() { println!("  stubbed: {}", p.stubbed.join(" ")); }
    println!("result: {:?} stdout={:?} eip={:#x}", r, String::from_utf8_lossy(&p.stdout), p.cpu.eip);
}
