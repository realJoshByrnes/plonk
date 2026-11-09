//! gui.exe through Wine's PE user32/gdi32 with our native win32u: window, paint, WM_CLOSE, quit.
use plonk_host::wine::WineProc;
use std::path::PathBuf;

#[test]
fn gui_through_wine() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("user32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\\gui.exe".into(), include_bytes!("../../../tests/guests/gui.exe").to_vec());
    p.wm.keep_snapshot = true;
    let r = p.boot("c:\\gui.exe").and_then(|_| p.run(200_000));
    let tail: Vec<_> = p.trace.iter().rev().take(5).collect();
    assert_eq!(r, Ok(7), "last syscalls: {tail:?}");
    let (w, h, pix) = p.wm.snapshot.clone().expect("window was painted");
    assert_eq!((w, h), (320, 240));
    // client area starts at (4, 32); FillRect(20,20,180,100) in RGB(200,30,30)
    let at = |x: u32, y: u32| pix[(y * w + x) as usize];
    assert_eq!(at(4 + 50, 32 + 50), 0xFFC8_1E1E, "red rect");
    assert_eq!(at(4 + 5, 32 + 5), 0xFFFF_FFFF, "class background (COLOR_WINDOW)");
}
