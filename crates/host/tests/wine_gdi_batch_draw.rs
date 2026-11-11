//! DIB orientation: StretchDIBits / SetDIBitsToDevice with bottom-up and top-down DIBs must put the image's top row on top.
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn gdi_batch_draw() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("gdi32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.lenient = true;
    p.files.insert("c:\\app.exe".into(), include_bytes!("../../../tests/guests/gbdraw.exe").to_vec());
    p.boot("c:\\app.exe").unwrap();
    let mut st = p.run_until_idle(50_000_000);
    while st == Ok(RunState::Busy) { st = p.run_until_idle(50_000_000); }
    assert_eq!(st, Ok(RunState::Exited(0)));
    let out = String::from_utf8_lossy(&p.stdout).to_string();
    eprintln!("{out}");
    for (k, v) in [
        ("endpath=", "1"), ("fillpath=", "1"), ("fill_in=", "0000ff"), ("fill_out=", "ffffff"), ("fillpath_again=", "0"),
        ("strokepath=", "1"), ("stroke_on=", "ff0000"), ("stroke_off=", "ffffff"),
        ("getpath n=3 m=3 t=6,2,2 p1=5,1", ""), ("getpath_after_abort=", "-1"),
        ("pathrgn=", "2,3,8,9"), ("clippath=", "1"), ("clip_in=", "0000ff"), ("clip_out=", "ffffff"),
        ("polydraw=", "1"), ("polydraw_px=", "0000ff"),
        ("flood=", "1"), ("flood_in=", "00ff00"), ("flood_out=", "ffffff"),
        ("frame_edge=", "0000ff"), ("frame_mid=", "ffffff"), ("invert_mid=", "000000"), ("ellrgn=", "1"),
        ("rgndata size=48 got=48 count=1 rect=1,2,5,7", ""),
        ("gradient=", "1"), ("maskblt=", "1"), ("mask_set=", "0000ff"), ("mask_clear=", "ffffff"),
        ("transparent=", "1"), ("tb_opaque=", "00ff00"), ("tb_key=", "ffffff"),
        ("plgblt=", "1"), ("plg_left=", "00ff00"), ("plg_right=", "ff00ff"),
        ("dibitmap top=0000ff bottom=ff0000", ""), ("dibbrush=1 dibbrush_px=00ff00", ""), ("hatch=1", ""), ("nearest=030201", ""), ("done", ""),
    ] {
        assert!(out.lines().any(|l| l.contains(&format!("{k}{v}"))), "missing {k}{v}
{out}");
    }
    let g: Vec<u32> = out.lines().find(|l| l.starts_with("grad_left=")).unwrap().split(|c: char| !c.is_ascii_digit()).filter_map(|t| t.parse().ok()).collect();
    assert!(g[0] < 40 && g[1] > 200, "gradient ramp {g:?}");
}
