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

#[test]
fn dynamic_glibc_hello() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/linux/");
    let Ok(ld) = std::fs::read(format!("{root}root/ld-linux.so.2")) else { eprintln!("skipped: run tests/linux/build.sh"); return };
    let mut p = LinuxProc::new();
    p.files.insert("/lib/ld-linux.so.2".into(), ld);
    p.files.insert("/lib/libc.so.6".into(), std::fs::read(format!("{root}root/libc.so.6")).unwrap());
    let elf = std::fs::read(format!("{root}hello_dyn")).unwrap();
    p.load(&elf, &["/bin/app"], &["HOME=/"]).unwrap();
    let r = p.run(400_000_000);
    let out = String::from_utf8_lossy(&p.stdout).into_owned();
    assert_eq!(r, Ok(Some(42)), "stdout {out:?} unknown {:?} tail {:#?}", p.unknown, p.trace);
    assert_eq!(out, "hello from linux /bin/app 1 x 3.14
");
}

#[test]
fn fork_exec_threads() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/linux/");
    let Ok(ld) = std::fs::read(format!("{root}root/ld-linux.so.2")) else { eprintln!("skipped: run tests/linux/build.sh"); return };
    let mut p = LinuxProc::new();
    p.auto_time = true;
    p.files.insert("/lib/ld-linux.so.2".into(), ld);
    p.files.insert("/lib/libc.so.6".into(), std::fs::read(format!("{root}root/libc.so.6")).unwrap());
    let elf = std::fs::read(format!("{root}multi")).unwrap();
    p.files.insert("/bin/app".into(), elf.clone());
    p.load(&elf, &["/bin/app"], &["HOME=/"]).unwrap();
    let r = p.run(2_000_000_000);
    let out = String::from_utf8_lossy(&p.stdout).into_owned();
    assert_eq!(r, Ok(Some(0)), "stdout {out:?} unknown {:?} tail {:#?}", p.unknown, p.trace.iter().rev().take(30).collect::<Vec<_>>());
    assert_eq!(out, "child said ping (4) exit 5\nthreads counter 6000 sum 6\nexec child x\nexec exit 7\n");
}
