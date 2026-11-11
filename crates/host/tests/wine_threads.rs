//! Cooperative threads: CreateThread, critical sections, interlocked ops, events, timeouts, suspended threads, TLS.
use plonk_host::wine::WineProc;
use std::path::PathBuf;

#[test]
fn threads_run_and_synchronise() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("msvcrt.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\\threads.exe".into(), include_bytes!("../../../tests/guests/threads.exe").to_vec());
    p.boot("c:\\threads.exe").unwrap();
    assert_eq!(p.run(20_000_000), Ok(0));
    assert_eq!(
        String::from_utf8_lossy(&p.stdout),
        "wait=0 counter=3000 total=3000 exits=12,14,16\r\nmain tls=0\r\nbefore=258\r\nafter=0\r\nt2=7\r\nsusp running=1\r\nsusp=28\r\n"
    );
}
