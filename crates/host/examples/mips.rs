//! Native interpreter speed: cargo run --release -p plonk-host --example mips [guest.exe]
use plonk_host::wine::{RunState, WineProc};

fn main() {
    let exe = std::env::args().nth(1).unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/guests/bench.exe").to_string());
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../wine/out");
    let mut p = WineProc::new();
    p.load_dir(dir);
    p.lenient = true;
    p.files.insert("c:\\bench.exe".into(), std::fs::read(&exe).expect("read guest"));
    p.boot("c:\\bench.exe").unwrap();
    let (t0, i0) = (std::time::Instant::now(), p.total_retired());
    let mut st = p.run_until_idle(50_000_000);
    while st == Ok(RunState::Busy) { st = p.run_until_idle(50_000_000); }
    let dt = t0.elapsed().as_secs_f64();
    let n = p.total_retired() - i0;
    println!("{:?}  {} M instructions in {:.2} s = {:.0} MIPS", st, n / 1_000_000, dt, n as f64 / 1e6 / dt);
}
