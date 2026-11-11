//! Print the last executed instruction addresses (plain interpreter) when a guest dies: cargo run --release -p plonk-host --example ringtrace -- <exe>
use plonk_host::wine::{RunState, WineProc};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut p = WineProc::new();
    p.load_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../wine/out"));
    p.lenient = true;
    p.files.insert("c:\\app.exe".into(), std::fs::read(&a[1]).expect("read exe"));
    p.cpu.debug_ring = true;
    p.boot("c:\\app.exe").unwrap();
    let mut st = p.run_until_idle(50_000_000);
    for _ in 0..400 {
        if matches!(st, Ok(RunState::Busy)) { st = p.run_until_idle(50_000_000); } else { break; }
    }
    println!("state {st:?}");
    for l in p.trace.iter().filter(|l| l.contains("(exception") || l.contains("(ring before") || l.contains("(stack return") || l.contains("(regs at") || l.contains("(stack words")) { println!("{l}"); }
    let n = p.cpu.ring_pos;
    for i in n.saturating_sub(40)..n {
        let Some(&eip) = p.cpu.ring.get(i & 63) else { continue };
        let m = p.module_at(eip).map(|(b, nm)| format!("{nm}+{:#x}", eip - b)).unwrap_or_default();
        let bytes = p.mem.read_bytes(eip, 8).map(|b| b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")).unwrap_or_default();
        println!("  {eip:#010x} {bytes:24} {m}");
    }
}
