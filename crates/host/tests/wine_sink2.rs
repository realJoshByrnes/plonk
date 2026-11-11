//! Dialogs, common controls (listview, status bar, listbox, combo, scrollbar), clipboard, modal loop in a window procedure.
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn dialogs_and_controls() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("comctl32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.lenient = true;
    p.files.insert("c:\sink2.exe".into(), include_bytes!("../../../tests/guests/sink2.exe").to_vec());
    p.boot("c:\sink2.exe").unwrap();
    let mut st = p.run_until_idle(5_000_000);
    for _ in 0..200 {
        if matches!(st, Ok(RunState::Exited(_)) | Err(_)) { break; }
        p.tick(50);
        st = p.run_until_idle(5_000_000);
    }
    assert_eq!(st, Ok(RunState::Exited(0)));
    assert_eq!(String::from_utf8_lossy(&p.stdout), "clip=1\r\ndialog=77\r\ndone2\r\n");
}
