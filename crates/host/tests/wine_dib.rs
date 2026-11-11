//! DIB orientation: StretchDIBits / SetDIBitsToDevice with bottom-up and top-down DIBs must put the image's top row on top.
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn dibs_are_not_flipped() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("gdi32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.lenient = true;
    p.files.insert("c:\\app.exe".into(), include_bytes!("../../../tests/guests/dibflip.exe").to_vec());
    p.boot("c:\\app.exe").unwrap();
    let mut st = p.run_until_idle(50_000_000);
    while st == Ok(RunState::Busy) { st = p.run_until_idle(50_000_000); }
    assert_eq!(st, Ok(RunState::Exited(0)));
    let out = String::from_utf8_lossy(&p.stdout).to_string();
    for l in out.lines().filter(|l| l.contains("top=")) {
        assert!(l.contains("top=red") && l.contains("bottom=green"), "flipped: {l}\n{out}");
    }
    assert!(out.contains("row0=other") && out.contains("row1=green"), "partial source rect: {out}");
}
