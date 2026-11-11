//! MessageBox layout: Wine sizes the buttons from GetWindowTextW (WM_GETTEXT), which the default window procedure must answer.
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn messagebox_buttons_are_sized_from_their_text() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("user32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.lenient = true;
    p.files.insert("c:\\app.exe".into(), include_bytes!("../../../tests/guests/msgbox.exe").to_vec());
    p.boot("c:\\app.exe").unwrap();
    assert_eq!(p.run_until_idle(20_000_000), Ok(RunState::Idle));
    let all = p.debug_all_windows();
    let buttons: Vec<&str> = all.lines().filter(|l| l.contains(":Button:")).collect();
    assert_eq!(buttons.len(), 2, "Yes and No buttons: {all}");
    for b in buttons {
        // rect is the last field: [l, t, r, b] in dialog coordinates
        let nums: Vec<i32> = b.rsplit_once('[').unwrap().1.trim_end_matches(']').split(", ").map(|v| v.parse().unwrap()).collect();
        assert!(nums[2] - nums[0] >= 30 && nums[3] - nums[1] >= 15, "button too small: {b}");
    }
}
