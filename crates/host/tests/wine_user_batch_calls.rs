//! class/window words, layered, regions, caret, system menu, accelerators, icons, ScrollDC, deferred pos, input (user_batch.rs).
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn user_batch_calls() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("user32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.lenient = true;
    p.files.insert("c:\\app.exe".into(), include_bytes!("../../../tests/guests/user_batch.exe").to_vec());
    p.boot("c:\\app.exe").unwrap();
    let mut st = p.run_until_idle(50_000_000);
    let mut n = 0;
    while matches!(st, Ok(RunState::Busy) | Ok(RunState::Idle)) && n < 200 { st = p.run_until_idle(50_000_000); n += 1; }
    assert_eq!(st, Ok(RunState::Exited(0)));
    let out = String::from_utf8_lossy(&p.stdout).to_string();
    for want in [
        "classlong 1234 5678",
        "classstyle old=0 new=2",
        "winword beef",
        "unreg busy=0",
        "layered 1 1 key=30201 alpha=128 flags=3",
        "getrgn type=2 box=1,2,30,40",
        "caretpos 5,6 blink=530",
        "threadmsg got=1 msg=8001 w=7 l=8",
        "sysmenu=1 count=7 close=1 bogus=0",
        "sysrevert=0",
        "menuinfo style=4000000 data=77 help=42",
        "accel h=1 count=3 copy=3 first=9/83/100",
        "acc plainS=0 ctrlS=1 cmds=1 w=10064",
        "acc F5=1 cmds=2 w=10065",
        "acc ctrlZ=0 ctrlshiftZ=1 cmds=3 w=10066",
        "acc keyup=0 destroy=1",
        "acc afterdestroy=0",
        "acc sys=1 syscmds=1 w=f060",
        "iconinfo ok=1 icon=1 hot=8,8 colorw=16 distinct=1",
        "drawicon=1",
        "pix left=ff right=ffffff outside=ffffff",
        "scroll px8=28 upd=0,0,4,16",
        "deferred rect=10,20,310,220",
        "wfromdc=1",
        "lock=1 unlock=1",
        "realchild in=1 out=1",
        "titlebar=1 wide=1",
        "showasync=1 vis=0",
        "syscolor 30201",
        "palette old=1 back=1",
        "sendinput=2",
        "cursor 1 1",
        "ctrl=1",
        "ctrlup=0",
        "query flags=2",
        "leaves before=0",
        "leaves after=1",
        "unreg=1",
        "done",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
}
