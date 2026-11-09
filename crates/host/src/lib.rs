//! wasm entry points. Guest memory is a region of this module's linear memory.
use plonk_cpu::{Cpu, Exit, EAX};
use plonk_mem::Mem;

const GUEST_SIZE: usize = 0x10000;
static mut GUEST: [u8; GUEST_SIZE] = [0; GUEST_SIZE];

/// Runs `mov eax,2; add eax,3; hlt` inside wasm. Returns EAX (5 on success) or 0xDEAD.
#[no_mangle]
pub extern "C" fn plonk_selftest() -> u32 {
    let mut m = unsafe { Mem::from_raw(core::ptr::addr_of_mut!(GUEST) as *mut u8, GUEST_SIZE as u32) };
    let code = [0xB8, 2, 0, 0, 0, 0x83, 0xC0, 3, 0xF4];
    m.write_bytes(0x1000, &code).unwrap();
    let mut c = Cpu::new();
    c.eip = 0x1000;
    if c.run(&mut m, 100) == Exit::Halt { c.r[EAX] } else { 0xDEAD }
}
