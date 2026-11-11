//! Kernel services: drives, volume info, tokens, waitable timers, handle duplication, directories, file ops, locale.
use plonk_host::wine::WineProc;
use std::path::PathBuf;

#[test]
fn kernel_services() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wine/out");
    if !dir.join("msvcrt.dll").exists() {
        eprintln!("skipped: wine/out not built");
        return;
    }
    let mut p = WineProc::new();
    p.load_dir(dir.to_str().unwrap());
    p.files.insert("c:\\sink3.exe".into(), include_bytes!("../../../tests/guests/sink3.exe").to_vec());
    p.boot("c:\\sink3.exe").unwrap();
    assert_eq!(p.run(5_000_000), Ok(0));
    let out = String::from_utf8_lossy(&p.stdout).to_string();
    for want in ["drives=4", "fs=NTFS", "type=3", "token=1", "timer=0", "dup=1", "mkdir=1", "copy=1", "move=1", "entries=2", "delete=1", "temp=C:\\temp\\", "lib=1 proc=1", "cpus=1 page=4096", "mem>0:1", "computer=1", "cmp=2"] {
        assert!(out.contains(want), "missing {want:?} in {out:?}");
    }
}
