//! Snippet harness: run hand-assembled x86 and compare regs/flags/memory.
use plonk_cpu::{Cpu, Exit, ESP};
use plonk_mem::Mem;

pub const CODE: u32 = 0x1000;
pub const STACK: u32 = 0x8000;

pub struct Case {
    pub name: &'static str,
    pub code: &'static [u8],
    pub init: &'static [(usize, u32)],
    pub exp: &'static [(usize, u32)],
    pub mem_init: &'static [(u32, &'static [u8])],
    pub mem_exp: &'static [(u32, &'static [u8])],
    /// (mask, expected value) over eflags; mask 0 = unchecked
    pub flags: (u32, u32),
    pub xmm_init: &'static [(usize, u128)],
    pub xmm_exp: &'static [(usize, u128)],
}

impl Case {
    pub fn new(name: &'static str, code: &'static [u8], init: &'static [(usize, u32)], exp: &'static [(usize, u32)]) -> Self {
        Case { name, code, init, exp, mem_init: &[], mem_exp: &[], flags: (0, 0), xmm_init: &[], xmm_exp: &[] }
    }
    pub fn xmm(mut self, init: &'static [(usize, u128)], exp: &'static [(usize, u128)]) -> Self { self.xmm_init = init; self.xmm_exp = exp; self }
    pub fn flags(mut self, mask: u32, val: u32) -> Self { self.flags = (mask, val); self }
    pub fn mem_init(mut self, m: &'static [(u32, &'static [u8])]) -> Self { self.mem_init = m; self }
    pub fn mem_exp(mut self, m: &'static [(u32, &'static [u8])]) -> Self { self.mem_exp = m; self }
}

pub fn run_case(c: &Case) -> Result<(), String> {
    let mut m = Mem::new_owned(0x10000);
    let f = |e| format!("{}: setup fault {:?}", c.name, e);
    m.write_bytes(CODE, c.code).map_err(f)?;
    m.w8(CODE + c.code.len() as u32, 0xF4).map_err(f)?; // trailing hlt
    for (a, b) in c.mem_init { m.write_bytes(*a, b).map_err(f)?; }
    let mut cpu = Cpu::new();
    cpu.eip = CODE;
    cpu.r[ESP] = STACK;
    for (r, v) in c.init { cpu.r[*r] = *v; }
    for (r, v) in c.xmm_init { cpu.xmm[*r] = *v; }
    let e = cpu.run(&mut m, 10_000);
    if e != Exit::Halt { return Err(format!("{}: exit {:?}", c.name, e)); }
    for (r, v) in c.exp {
        if cpu.r[*r] != *v { return Err(format!("{}: reg{} = {:#x}, want {:#x}", c.name, r, cpu.r[*r], v)); }
    }
    for (r, v) in c.xmm_exp {
        if cpu.xmm[*r] != *v { return Err(format!("{}: xmm{} = {:#x}, want {:#x}", c.name, r, cpu.xmm[*r], v)); }
    }
    let (mask, val) = c.flags;
    if cpu.eflags & mask != val { return Err(format!("{}: flags {:#x}&{:#x} = {:#x}, want {:#x}", c.name, cpu.eflags, mask, cpu.eflags & mask, val)); }
    for (a, b) in c.mem_exp {
        let got = m.read_bytes(*a, b.len() as u32).map_err(f)?;
        if got != *b { return Err(format!("{}: mem@{:#x} = {:02x?}, want {:02x?}", c.name, a, got, b)); }
    }
    Ok(())
}

/// Prints one summary line plus up to 3 failures; returns failure count.
pub fn run_all(cases: &[Case]) -> usize {
    let fails: Vec<String> = cases.iter().filter_map(|c| run_case(c).err()).collect();
    println!("x86 snippets: {} passed, {} failed", cases.len() - fails.len(), fails.len());
    for f in fails.iter().take(3) { println!("  FAIL {}", f); }
    fails.len()
}
