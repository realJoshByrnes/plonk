//! Kitchen-sink Win32 program: TLS, env, time, heap, metrics, writable files, registry, timers, GDI shapes + fonts.
use plonk_host::wine::{RunState, WineProc};
use std::path::PathBuf;

#[test]
fn kitchen_sink() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("user32.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\\sink.exe".into(), include_bytes!("../../../tests/guests/sink.exe").to_vec());
    p.boot("c:\\sink.exe").unwrap();
    // the app arms a 200 ms timer that closes its window; drive the shell clock until it exits
    let mut state = p.run_until_idle(5_000_000);
    for _ in 0..40 {
        if matches!(state, Ok(RunState::Exited(_))) { break; }
        p.tick(50);
        state = p.run_until_idle(5_000_000);
    }
    assert_eq!(state, Ok(RunState::Exited(0)), "last: {:?}", p.trace.iter().rev().take(4).collect::<Vec<_>>());
    let out = String::from_utf8_lossy(&p.stdout).to_string();
    for want in ["tls=42", "env=C:\\windows", "file=hello file size=10", "attrs=80", "reg=7", "done"] {
        assert!(out.contains(want), "missing {want:?} in {out:?}");
    }
}
