//! A real Wine BUTTON control inside a window: three clicks reach the app as WM_COMMAND.
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn button_clicks_reach_app() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("user32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\\btn.exe".into(), include_bytes!("../../../tests/guests/gui_button.exe").to_vec());
    p.boot("c:\\btn.exe").unwrap();
    assert_eq!(p.run_until_idle(2_000_000), Ok(RunState::Idle));
    for _ in 0..3 {
        p.mouse(0, 100, 100);
        assert_eq!(p.run_until_idle(2_000_000), Ok(RunState::Idle));
        p.mouse(2, 100, 100);
        assert_eq!(p.run_until_idle(2_000_000), Ok(RunState::Idle));
    }
    p.mouse(0, 326, 52); // close box
    p.mouse(2, 326, 52);
    assert_eq!(p.run_until_idle(2_000_000), Ok(RunState::Exited(3)), "exit code = number of clicks");
}
