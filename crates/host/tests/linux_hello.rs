use plonk_host::linux::LinuxProc;

#[test]
fn static_glibc_hello() {
    let elf = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/linux/hello")).unwrap();
    let mut p = LinuxProc::new();
    p.load(&elf, &["/bin/app"], &["HOME=/"]).unwrap();
    let r = p.run(200_000_000);
    let out = String::from_utf8_lossy(&p.stdout).into_owned();
    assert_eq!(r, Ok(Some(42)), "stdout {out:?} unknown {:?} tail {:?}", p.unknown, p.trace.iter().rev().take(8).collect::<Vec<_>>());
    assert_eq!(out, "hello from linux /bin/app 1 x 3.14\n");
}
