//! Menu bar + glass popup: click File, choose "Say hi" then "Quit"; the app exits with the last command id.
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn menu_bar_and_popup() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("user32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\\menu.exe".into(), include_bytes!("../../../tests/guests/gui_menu.exe").to_vec());
    p.boot("c:\\menu.exe").unwrap();
    assert_eq!(p.run_until_idle(3_000_000), Ok(RunState::Idle));
    let click = |p: &mut WineProc, x, y| {
        p.mouse(0, x, y);
        p.mouse(2, x, y);
        assert_eq!(p.run_until_idle(3_000_000), Ok(RunState::Idle));
    };
    // window at (40,40): menu bar band y = 72..96, "File" starts at x = 48
    click(&mut p, 60, 80); // open File
    click(&mut p, 80, 112); // "Say hi" row (y 100..126)
    click(&mut p, 60, 80); // open again
    p.mouse(0, 80, 148); // "Quit" row (y 135..161)
    p.mouse(2, 80, 148);
    assert_eq!(p.run_until_idle(3_000_000), Ok(RunState::Exited(12)), "last command id");
}
