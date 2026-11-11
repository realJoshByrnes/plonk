//! cargo run --release -p plonk-host --example drive -- <exe> [seconds]: run a GUI exe driving the shell clock, print stdout + gaps.
//! LAZY=1 mimics the browser: only the core DLLs are preloaded, everything else is supplied on demand.
use plonk_host::wine::{RunState, WineProc};
const APP: &str = "c:\\app.exe";
const CORE: [&str; 13] = ["apisetschema", "ntdll", "kernelbase", "kernel32", "win32u", "user32", "gdi32", "msvcrt", "ucrtbase", "advapi32", "sechost", "rpcrt4", "imm32"];

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let secs: u32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let lazy = std::env::var("LAZY").is_ok();
    let mut p = WineProc::new();
    p.load_dir("wine/out");
    if lazy {
        let drop: Vec<String> = p.files.keys().filter(|k| k.starts_with("c:\\windows\\") && !CORE.iter().any(|c| k.ends_with(&format!("\\{c}.dll")))).cloned().collect();
        for k in drop { p.files.remove(&k); }
        let nls: Vec<String> = p.files.keys().filter(|k| k.starts_with("nls\\") && !k.ends_with("locale.nls") && !k.ends_with("l_intl.nls")).cloned().collect();
        for k in nls { p.files.remove(&k); }
        for e in std::fs::read_dir("wine/out").unwrap().chain(std::fs::read_dir("wine/out/nls").unwrap()).flatten() {
            p.remote.insert(e.file_name().to_string_lossy().to_ascii_lowercase());
        }
    }
    p.lenient = true;
    if let Ok(v) = std::env::var("SLICE") { p.slice = v.parse().unwrap(); }
    p.files.insert(APP.into(), std::fs::read(&a[1]).unwrap());
    if let Err(e) = p.boot(APP) { println!("boot: {e}"); return; }
    if std::env::var("WATCH").is_ok() { // debug: report where the guest is spinning
        let ptr = &p as *const WineProc as usize;
        std::thread::spawn(move || {
            for _ in 0..3 {
                std::thread::sleep(std::time::Duration::from_secs(4));
                let pp = unsafe { &*(ptr as *const WineProc) };
                eprintln!("watch: eip={:#x} {:?} regs={:x?}", pp.cpu.eip, pp.module_at(pp.cpu.eip), pp.cpu.r);
            }
            std::process::exit(3);
        });
    }
    let mut fetched = Vec::new();
    let mut supply = |p: &mut WineProc, st: Result<RunState, String>| -> Result<RunState, String> {
        let mut st = st;
        while let Ok(RunState::Need(k)) = &st {
            let base = k.rsplit('\\').next().unwrap().to_string();
            fetched.push(base.clone()); if fetched.len() < 40 { eprintln!("need {k}"); }
            let path = if std::path::Path::new(&format!("wine/out/{base}")).exists() { format!("wine/out/{base}") } else { format!("wine/out/nls/{base}") };
            p.files.insert(k.clone(), std::fs::read(path).unwrap());
            st = p.run_until_idle(std::env::var("BUDGET").ok().and_then(|v| v.parse().ok()).unwrap_or(5_000_000));
        }
        st
    };
    let first = p.run_until_idle(std::env::var("BUDGET").ok().and_then(|v| v.parse().ok()).unwrap_or(5_000_000));
    let mut st = supply(&mut p, first);
    for _ in 0..secs * 20 {
        match &st { Ok(RunState::Exited(_)) | Err(_) => break, _ => {} }
        p.tick(50);
        let r = p.run_until_idle(std::env::var("BUDGET").ok().and_then(|v| v.parse().ok()).unwrap_or(5_000_000));
        st = supply(&mut p, r);
    }
    println!("state: {:?} eip {:#x} {:?}", st, p.cpu.eip, p.module_at(p.cpu.eip));
    if st.is_err() { for l in p.trace.iter().rev().take(std::env::var("N").ok().and_then(|v| v.parse().ok()).unwrap_or(14)).collect::<Vec<_>>().into_iter().rev() { println!("  {l}"); } println!("  regs {:x?} eip {:#x}", p.cpu.r, p.cpu.eip); }
    println!("fetched: {}", fetched.join(" "));
    println!("windows: {}", p.debug_windows());
    println!("invalid: {}", p.debug_invalid());
    println!("stdout: {:?}", String::from_utf8_lossy(&p.stdout));
    println!("stubbed: {}", p.stubbed.join(" "));
}
