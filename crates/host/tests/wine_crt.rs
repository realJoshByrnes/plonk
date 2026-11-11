//! A mingw exe linked against the real msvcrt (printf, malloc, libm) runs through Wine.
use plonk_host::wine::WineProc;
use std::path::PathBuf;

#[test]
fn crt_exe_runs() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("msvcrt.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\crt.exe".into(), include_bytes!("../../../tests/guests/crt_hello.exe").to_vec());
    p.boot("c:\crt.exe").unwrap();
    assert_eq!(p.run(5_000_000), Ok(3));
    assert_eq!(p.stdout, b"CRT hello: sqrt(2)=1.4142 n=3\r\n");
}
