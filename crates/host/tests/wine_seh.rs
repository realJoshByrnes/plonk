//! Exceptions: access violation, divide error and RaiseException dispatched through Wine's ntdll to a vectored handler.
use plonk_host::wine::WineProc;
use std::path::PathBuf;

#[test]
fn exceptions_dispatch() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("msvcrt.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\seh.exe".into(), include_bytes!("../../../tests/guests/seh.exe").to_vec());
    p.boot("c:\seh.exe").unwrap();
    assert_eq!(p.run(5_000_000), Ok(0));
    assert_eq!(String::from_utf8_lossy(&p.stdout), "av addr=10\r\ndiv=1\r\nraise arg=42\r\nend\r\n");
}
