//! hello.exe through Wine's real PE ntdll/kernel32/kernelbase. Skipped if wine/out is missing
//! (build it with wine/build.sh; the DLLs are git-ignored).
use plonk_host::wine::WineProc;
use std::path::PathBuf;

#[test]
fn hello_through_wine() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("ntdll.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\\hello.exe".into(), include_bytes!("../../../tests/guests/hello.exe").to_vec());
    let r = p.boot("c:\\hello.exe").and_then(|_| p.run(100_000));
    let tail: Vec<_> = p.trace.iter().rev().take(5).collect();
    assert_eq!(r, Ok(0), "last syscalls: {tail:?}");
    assert_eq!(p.stdout, b"Hello from Plonk\r\n");
}
