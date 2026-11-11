//! The Rust controls showcase (button, check, radio, edit, combo, list box, list view, tree view, tab, progress,
//! trackbar, up-down, status bar, date picker, IP address, hot key): it must come up, react to input and exit.
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn controls_showcase_runs_and_reacts() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("comctl32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.lenient = true;
    p.files.insert("c:\\controls.exe".into(), include_bytes!("../../../tests/guests/rust_controls.exe").to_vec());
    p.boot("c:\\controls.exe").unwrap();
    assert_eq!(p.run_until_idle(10_000_000), Ok(RunState::Idle));
    assert!(p.debug_windows().contains("PlonkControlsDemo:true"), "main window visible: {}", p.debug_windows());
    // click the "Three-state" check box, type into the first edit, then close the window
    p.mouse(0, 190, 129);
    p.mouse(2, 190, 129);
    assert_eq!(p.run_until_idle(10_000_000), Ok(RunState::Idle));
    p.mouse(0, 100, 265);
    p.mouse(2, 100, 265);
    p.key(0, 88, 120);
    p.key(1, 88, 0);
    assert_eq!(p.run_until_idle(10_000_000), Ok(RunState::Idle));
    p.mouse(0, 788, 40); // close box
    p.mouse(2, 788, 40);
    assert_eq!(p.run_until_idle(10_000_000), Ok(RunState::Exited(0)));
}
