//! GDI bitmap engine through Wine's gdi32: memory DC + compatible bitmap, BitBlt, StretchBlt,
//! DIB section written directly by the app, AlphaBlend with per-pixel alpha.
use plonk_host::wine::WineProc;
use std::path::PathBuf;

#[test]
fn bitmaps_blit_and_alpha() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("msimg32.dll").exists() {
        eprintln!("skipped: wine/out not built (needs msimg32)");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\\bmp.exe".into(), include_bytes!("../../../tests/guests/gui_bitmap.exe").to_vec());
    p.wm.keep_snapshot = true;
    let r = p.boot("c:\\bmp.exe").and_then(|_| p.run(500_000));
    let tail: Vec<_> = p.trace.iter().rev().take(5).collect();
    assert_eq!(r, Ok(0), "last syscalls: {tail:?}");
    let (w, _h, pix) = p.wm.snapshot.clone().expect("painted");
    // window client origin inside the surface is (4, 32)
    let at = |cx: u32, cy: u32| { let c = pix[((32 + cy) * w + 4 + cx) as usize]; ((c >> 16 & 0xff) as i32, (c >> 8 & 0xff) as i32, (c & 0xff) as i32) };
    assert_eq!(at(10 + 80, 10 + 50), (20, 60, 200), "BitBlt of the offscreen bitmap");
    assert_eq!(at(120 + 40, 10 + 25), (20, 60, 200), "StretchBlt (half size)");
    assert_eq!(at(120 + 60, 10 + 25), (255, 255, 255), "outside the stretched rect stays background");
    let (r, g, b) = at(10 + 30, 90 + 16);
    assert!((r - 127).abs() <= 2 && (g - 255).abs() <= 2 && (b - 127).abs() <= 2, "AlphaBlend over white: got {:?}", (r, g, b));
}
