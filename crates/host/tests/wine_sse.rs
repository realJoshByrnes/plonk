//! A program compiled with real SSE2 code generation (doubles, floats, SIMD intrinsics) through Wine's CRT.
use plonk_host::wine::WineProc;
use std::path::PathBuf;

#[test]
fn sse2_exe_runs() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("msvcrt.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\\sse.exe".into(), include_bytes!("../../../tests/guests/sse_test.exe").to_vec());
    p.boot("c:\\sse.exe").unwrap();
    assert_eq!(p.run(5_000_000), Ok(5));
    assert_eq!(String::from_utf8_lossy(&p.stdout), "dot=70.00 sqrt=1.4142\r\nf=85.998 int=318\r\nsimd=11,12,13,14\r\npd=1.0,1.5\r\nlen=31\r\n");
}
