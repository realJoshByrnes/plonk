//! DIB orientation: StretchDIBits / SetDIBitsToDevice with bottom-up and top-down DIBs must put the image's top row on top.
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn gdi_batch_state() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("gdi32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.lenient = true;
    p.files.insert("c:\\app.exe".into(), include_bytes!("../../../tests/guests/gbstate.exe").to_vec());
    p.boot("c:\\app.exe").unwrap();
    let mut st = p.run_until_idle(50_000_000);
    while st == Ok(RunState::Busy) { st = p.run_until_idle(50_000_000); }
    assert_eq!(st, Ok(RunState::Exited(0)));
    let out = String::from_utf8_lossy(&p.stdout).to_string();
    eprintln!("{out}");
    for k in ["gm=1", "setwt=1", "wt=2,0,0,3,5,7", "rightmul=1", "wt2=2,0,0,3,15,7", "ident=1", "wt3=1,0,0,1,0,0", "scalevp=1 old=200,200 new=100,100", "scalewin=1 win=200,200", "curpos=7,9", "bk=030201", "setbounds1=8", "setbounds2=4", "getbounds=2 rect=1,2,5,6", "getbounds_reset=1", "getbits=32 first=112233 last=000004 size=32", "setbits=32", "setbits_px=0000ff,00ff00", "setdim=1 prev=0,0 getdim=1 cur=10,20", "palget=4 e1=255,0,0", "palset=1 e2=9,8,7", "palsize=4 resize=1 palsize2=8 nearest=2", "raster=1 size=6 flags=3", "addfont=1", "memfont=1 cnt=1 remove=1", "justify=1", "glyph metrics=1 bitmap=1 box=1 adv=1", "gamma=1 r0=0 r255=65535", "cadj=1 size=24", "flush=1", "startdoc_fails=1", "enhmf=1", "done"] {
        assert!(out.lines().any(|l| l == k), "missing {k}
{out}");
    }
}
