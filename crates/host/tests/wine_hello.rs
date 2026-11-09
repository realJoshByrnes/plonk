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
    for d in ["ntdll", "kernel32", "kernelbase", "win32u", "user32", "gdi32"] {
        if let Ok(b) = std::fs::read(dir.join(format!("{d}.dll"))) {
            p.files.insert(format!("c:\\windows\\system32\\{d}.dll"), b);
        }
    }
    for e in std::fs::read_dir(dir.join("nls")).unwrap().flatten() {
        p.files.insert(format!("nls\\{}", e.file_name().to_string_lossy()), std::fs::read(e.path()).unwrap());
    }
    p.files.insert("c:\\hello.exe".into(), include_bytes!("../../../tests/guests/hello.exe").to_vec());
    let r = p.boot("c:\\hello.exe").and_then(|_| p.run(100_000));
    let tail: Vec<_> = p.trace.iter().rev().take(5).collect();
    assert_eq!(r, Ok(0), "last syscalls: {tail:?}");
    assert_eq!(p.stdout, b"Hello from Plonk\r\n");
}
