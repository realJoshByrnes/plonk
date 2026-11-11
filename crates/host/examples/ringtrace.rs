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
    // EVENTS="t,ms;d,x,y;u,x,y;k,vk,ch": scripted input, then a bounded amount of work (MAXRUNS slices) so an endless loop can be inspected
    let maxruns: u32 = std::env::var("MAXRUNS").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
    if let Ok(ev) = std::env::var("EVENTS") {
        for e in ev.split(';').filter(|s| !s.is_empty()) {
            let f: Vec<&str> = e.split(',').collect();
            match f[0] {
                "t" => p.tick(f[1].parse().unwrap()),
                "d" => p.mouse(0, f[1].parse().unwrap(), f[2].parse().unwrap()),
                "u" => p.mouse(2, f[1].parse().unwrap(), f[2].parse().unwrap()),
                "k" => { let (vk, ch): (u32, u32) = (f[1].parse().unwrap(), f[2].parse().unwrap()); p.key(0, vk, ch); p.key(1, vk, 0); }
                _ => {}
            }
            p.slice = 300_000;
            st = p.run_until_idle(20_000);
            let mut n = 0;
            while matches!(st, Ok(RunState::Busy)) && n < maxruns { st = p.run_until_idle(20_000); n += 1; }
            if n >= maxruns { println!("still busy after event {e}: stopping to inspect"); break; }
        }
    }
    println!("state {st:?}");
    for l in p.trace.iter().filter(|l| l.contains("(exception") || l.contains("(ring before") || l.contains("(stack return") || l.contains("(regs at") || l.contains("(stack words")) { println!("{l}"); }
    if let Some(i) = p.trace.iter().position(|l| l.contains("(exception")) {
        for l in p.trace.iter().skip(i.saturating_sub(8)).take(8) { println!("  before: {l}"); }
    }
    let n = p.cpu.ring_pos;
    for i in n.saturating_sub(40)..n {
        let Some(&eip) = p.cpu.ring.get(i & 63) else { continue };
        let m = p.module_at(eip).map(|(b, nm)| format!("{nm}+{:#x}", eip - b)).unwrap_or_default();
        let bytes = p.mem.read_bytes(eip, 8).map(|b| b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")).unwrap_or_default();
        println!("  {eip:#010x} {bytes:24} {m}");
    }
}
