//! Keyboard input into a real Wine EDIT control: click to focus, type, read the text back.
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn typing_into_edit() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("user32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\\edit.exe".into(), include_bytes!("../../../tests/guests/gui_edit.exe").to_vec());
    p.boot("c:\\edit.exe").unwrap();
    assert_eq!(p.run_until_idle(3_000_000), Ok(RunState::Idle));
    p.mouse(0, 100, 100); // edit control at client (20,20): screen (64, 92)..(264, 114)
    p.mouse(2, 100, 100);
    assert_eq!(p.run_until_idle(3_000_000), Ok(RunState::Idle));
    for (vk, ch) in [(b'H' as u32, b'h' as u32), (b'I' as u32, b'i' as u32), (b'!' as u32, b'!' as u32)] {
        p.key(0, vk, ch);
        p.key(1, vk, 0);
        assert_eq!(p.run_until_idle(3_000_000), Ok(RunState::Idle));
    }
    p.key(0, 8, 8); // backspace
    p.key(1, 8, 0);
    assert_eq!(p.run_until_idle(3_000_000), Ok(RunState::Idle));
    p.mouse(0, 326, 52); // close box
    p.mouse(2, 326, 52);
    assert_eq!(p.run_until_idle(3_000_000), Ok(RunState::Exited(2)), "exit code = typed text length");
}
