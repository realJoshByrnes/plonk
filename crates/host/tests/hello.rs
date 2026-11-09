use plonk_host::Process;

#[test]
fn hello_exe() {
    let mut p = Process::new();
    let r = p.run_pe(include_bytes!("../../../tests/guests/hello.exe"));
    assert_eq!(r, Ok(0), "run_pe");
    assert_eq!(p.stdout, b"Hello from Plonk\r\n");
}
