//! A Rust std hello world built with the default i686-pc-windows-msvc toolchain (UCRT apiset imports, vcruntime, cmpxchg8b).
use plonk_host::wine::WineProc;
use std::path::PathBuf;

#[test]
fn rust_msvc_hello() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("vcruntime140.dll").exists() {
        eprintln!("skipped: wine/out not built (needs vcruntime140, apisetschema)");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\rust.exe".into(), include_bytes!("../../../tests/guests/rust_hello.exe").to_vec());
    p.boot("c:\rust.exe").unwrap();
    assert_eq!(p.run(20_000_000), Ok(0));
    assert_eq!(String::from_utf8_lossy(&p.stdout), "Hello from Rust! 55\n");
}
