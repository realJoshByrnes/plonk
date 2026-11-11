//! clipboard / keyboard layout / display / window station / raw input / hotkey syscalls (misc_batch.rs).
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn misc_batch_calls() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("gdi32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.lenient = true;
    p.files.insert("c:\\app.exe".into(), include_bytes!("../../../tests/guests/misc_batch.exe").to_vec());
    p.boot("c:\\app.exe").unwrap();
    let mut st = p.run_until_idle(50_000_000);
    while st == Ok(RunState::Busy) { st = p.run_until_idle(50_000_000); }
    assert_eq!(st, Ok(RunState::Exited(0)));
    let out = String::from_utf8_lossy(&p.stdout).to_string();
    for want in [
        "clip count0=0 viewer=0 owner=0",
        "listener add=1 remove=1 remove2=0",
        "kl=00000409 n=1",
        "monitors=1",
        "adapter0=1 adapter1=0",
        "name=\\\\.\\DISPLAY1 primary=1",
        "bpp=32 hz=60",
        "change=0",
        "winsta=1 desktop=1",
        "rawlist ret=0 n=0",
        "rawreg=1",
        "rawregistered n=1",
        "hotkey reg=1 dup=0 unreg=1 unreg2=0",
        "done",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
}
