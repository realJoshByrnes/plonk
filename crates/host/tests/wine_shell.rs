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
    p.files.insert("c:\\two.exe".into(), include_bytes!("../../../tests/guests/gui_two.exe").to_vec());
    p.boot("c:\\two.exe").unwrap();
    assert_eq!(p.run_until_idle(1_000_000), Ok(RunState::Idle));
    let px = |p: &WineProc, x: usize, y: usize| { let f = p.compose_rgba(1024, 768); let o = (y * 1024 + x) * 4; (f[o], f[o + 1], f[o + 2]) };
    // B (created last) is on top: in the overlap B's client face shows, not A's white
    assert_ne!(px(&p, 200, 150), (255, 255, 255), "B covers A in the overlap");
    // click A's title bar -> A raised: its opaque white client area now wins the overlap
    p.mouse(0, 60, 50);
    p.mouse(2, 60, 50);
    assert_eq!(p.run_until_idle(1_000_000), Ok(RunState::Idle));
    assert_eq!(px(&p, 200, 150), (255, 255, 255), "A raised above B");
    // drag A by (+240,+200): grab at (60,50), release at (300,250)
    p.mouse(0, 60, 50);
    p.mouse(1, 300, 250);
    p.mouse(2, 300, 250);
    assert_eq!(p.run_until_idle(1_000_000), Ok(RunState::Idle));
    assert_eq!(px(&p, 350, 300), (40, 120, 220), "A's blue rect after the move");
    let wp = plonk_display::wallpaper(60, 150, 1024, 768);
    assert_eq!(px(&p, 60, 150), ((wp >> 16) as u8, (wp >> 8) as u8, wp as u8), "old position shows the wallpaper again");
}
