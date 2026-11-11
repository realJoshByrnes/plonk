//! OpenGL guest app: opengl32 stub traps into the host, which must emit a CREATE, state/geometry commands and PRESENTs.
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn gl_cube_emits_command_stream() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("opengl32.dll").exists() {
        eprintln!("skipped: opengl32.dll not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.lenient = true;
    p.files.insert("c:\\gl.exe".into(), include_bytes!("../../../tests/guests/gl_cube.exe").to_vec());
    p.boot("c:\\gl.exe").unwrap();
    let mut st = p.run_until_idle(20_000_000);
    while st == Ok(RunState::Busy) && p.wm.gl.presents < 2 { st = p.run_until_idle(20_000_000); }
    for _ in 0..5 {
        p.tick(20);
        let mut st = p.run_until_idle(20_000_000);
        while st == Ok(RunState::Busy) { st = p.run_until_idle(20_000_000); if p.wm.gl.presents > 12 { break; } } // free-running demo yields once per frame
        assert!(matches!(st, Ok(RunState::Idle) | Ok(RunState::Busy)), "{st:?}");
    }
    let c = &p.wm.gl.cmds;
    let mut ops = Vec::new();
    let mut i = 0;
    while i + 1 < c.len() { ops.push(c[i]); i += 2 + c[i + 1] as usize; }
    assert_eq!(i, c.len(), "well-formed stream");
    assert_eq!(ops[0], 1, "CREATE first");
    assert!(ops.iter().filter(|&&o| o == 49).count() >= 3, "presents: {}", ops.iter().filter(|&&o| o == 49).count());
    for need in [3u32, 6, 22, 24, 27, 29, 28, 39, 34] { assert!(ops.contains(&need), "missing op {need}"); }
    eprintln!("gl stubs: {:?}", p.wm.gl.stubbed);
}
