//! Shell behaviour: two windows from one process, drag by title bar, raise on click.
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn drag_and_raise() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("user32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\two.exe".into(), include_bytes!("../../../tests/guests/gui_two.exe").to_vec());
    p.boot("c:\two.exe").unwrap();
    assert_eq!(p.run_until_idle(1_000_000), Ok(RunState::Idle));
    let px = |p: &WineProc, x: usize, y: usize| { let f = p.compose_rgba(1024, 768); let o = (y * 1024 + x) * 4; (f[o], f[o + 1], f[o + 2]) };
    // B (created last) is on top; A's client area is visible where B doesn't cover it
    assert_eq!(px(&p, 60, 150), (255, 255, 255), "A client background");
    // click A's title bar -> A raised and becomes active (dark caption)
    p.mouse(0, 60, 50);
    p.mouse(2, 60, 50);
    assert_eq!(p.run_until_idle(1_000_000), Ok(RunState::Idle));
    assert_eq!(px(&p, 150, 50), (10, 36, 106), "A active caption, overlapping B");
    // drag A by (+240,+200): grab at (60,50), release at (300,250)
    p.mouse(0, 60, 50);
    p.mouse(1, 300, 250);
    p.mouse(2, 300, 250);
    assert_eq!(p.run_until_idle(1_000_000), Ok(RunState::Idle));
    assert_eq!(px(&p, 350, 300), (40, 120, 220), "A's blue rect after the move");
    assert_eq!(px(&p, 60, 150), (58, 110, 165), "old position is desktop again");
}
