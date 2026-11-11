//! cargo run --release -p plonk-host --example drive -- <exe> [seconds]: run a GUI exe driving the shell clock, print stdout + gaps.
use plonk_host::wine::{RunState, WineProc};
const APP: &str = r"c:pp.exe";
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let secs: u32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let mut p = WineProc::new();
    p.load_dir("wine/out");
    p.lenient = true;
    p.files.insert(APP.into(), std::fs::read(&a[1]).unwrap());
    if let Err(e) = p.boot(APP) { println!("boot: {e}"); return; }
    let mut st = p.run_until_idle(5_000_000);
    for _ in 0..secs * 20 {
        match &st { Ok(RunState::Exited(_)) | Err(_) => break, _ => {} }
        p.tick(50);
        st = p.run_until_idle(5_000_000);
    }
    println!("state: {:?}", st);
    if st.is_err() { for l in p.trace.iter().rev().take(std::env::var("N").ok().and_then(|v| v.parse().ok()).unwrap_or(14)).collect::<Vec<_>>().into_iter().rev() { println!("  {l}"); } println!("  regs {:x?} eip {:#x}", p.cpu.r, p.cpu.eip); }
    println!("stdout: {:?}", String::from_utf8_lossy(&p.stdout));
    println!("stubbed: {}", p.stubbed.join(" "));
}
