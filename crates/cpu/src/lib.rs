//! i386 interpreter core. 32-bit protected-mode flat model only (for now).
use plonk_mem::{Fault, Mem};

pub const EAX: usize = 0;
pub const ECX: usize = 1;
pub const EDX: usize = 2;
pub const EBX: usize = 3;
pub const ESP: usize = 4;
pub const EBP: usize = 5;
pub const ESI: usize = 6;
pub const EDI: usize = 7;

pub const CF: u32 = 1;
pub const PF: u32 = 4;
pub const AF: u32 = 0x10;
pub const ZF: u32 = 0x40;
pub const SF: u32 = 0x80;
pub const DF: u32 = 0x400;
pub const OF: u32 = 0x800;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Exit {
    Halt,
    StepLimit,
    Unimplemented { eip: u32, opcode: u8 },
    Fault(u32),
}

impl From<Fault> for Exit {
    fn from(f: Fault) -> Self { Exit::Fault(f.0) }
}

#[derive(Clone, Copy)]
enum Op { R(usize), M(u32) }

#[derive(Clone, Debug, Default)]
pub struct Cpu {
    pub r: [u32; 8],
    pub eip: u32,
    pub eflags: u32,
}

type R<T> = Result<T, Exit>;

impl Cpu {
    pub fn new() -> Self { Cpu { eflags: 2, ..Default::default() } }

    pub fn run(&mut self, mem: &mut Mem, max_steps: u64) -> Exit {
        for _ in 0..max_steps {
            if let Err(e) = self.step(mem) { return e; }
        }
        Exit::StepLimit
    }

    fn f8(&mut self, m: &Mem) -> R<u8> { let v = m.r8(self.eip)?; self.eip = self.eip.wrapping_add(1); Ok(v) }
    fn f32(&mut self, m: &Mem) -> R<u32> { let v = m.r32(self.eip)?; self.eip = self.eip.wrapping_add(4); Ok(v) }

    fn reg8(&self, i: usize) -> u32 { if i < 4 { self.r[i] & 0xff } else { (self.r[i - 4] >> 8) & 0xff } }
    fn set_reg8(&mut self, i: usize, v: u32) {
        let v = v & 0xff;
        if i < 4 { self.r[i] = (self.r[i] & !0xff) | v } else { self.r[i - 4] = (self.r[i - 4] & !0xff00) | (v << 8) }
    }
    fn getr(&self, i: usize, w: u32) -> u32 { if w == 8 { self.reg8(i) } else { self.r[i] } }
    fn setr(&mut self, i: usize, w: u32, v: u32) { if w == 8 { self.set_reg8(i, v) } else { self.r[i] = v } }

    fn rd(&self, m: &Mem, o: Op, w: u32) -> R<u32> {
        Ok(match o {
            Op::R(i) => self.getr(i, w),
            Op::M(a) => if w == 8 { m.r8(a)? as u32 } else { m.r32(a)? },
        })
    }
    fn wr(&mut self, m: &mut Mem, o: Op, w: u32, v: u32) -> R<()> {
        match o {
            Op::R(i) => self.setr(i, w, v),
            Op::M(a) => if w == 8 { m.w8(a, v as u8)? } else { m.w32(a, v)? },
        }
        Ok(())
    }

    /// Returns (reg field, r/m operand).
    fn modrm(&mut self, m: &Mem) -> R<(usize, Op)> {
        let b = self.f8(m)?;
        let (md, reg, rm) = (b >> 6, ((b >> 3) & 7) as usize, (b & 7) as usize);
        if md == 3 { return Ok((reg, Op::R(rm))); }
        let mut a: u32;
        if rm == 4 {
            let s = self.f8(m)?;
            let (sc, ix, bs) = (s >> 6, ((s >> 3) & 7) as usize, (s & 7) as usize);
            a = if ix != 4 { self.r[ix] << sc } else { 0 };
            a = a.wrapping_add(if bs == 5 && md == 0 { self.f32(m)? } else { self.r[bs] });
        } else if rm == 5 && md == 0 {
            a = self.f32(m)?;
        } else {
            a = self.r[rm];
        }
        match md {
            1 => a = a.wrapping_add(self.f8(m)? as i8 as i32 as u32),
            2 => a = a.wrapping_add(self.f32(m)?),
            _ => {}
        }
        Ok((reg, Op::M(a)))
    }

    fn setf(&mut self, f: u32, on: bool) { if on { self.eflags |= f } else { self.eflags &= !f } }

    /// op: 0 add,1 or,2 adc,3 sbb,4 and,5 sub,6 xor,7 cmp. Returns result; sets flags.
    fn alu(&mut self, op: u8, a: u32, b: u32, w: u32) -> u32 {
        let mask: u64 = if w == 8 { 0xff } else { 0xffff_ffff };
        let sign: u64 = 1 << (w - 1);
        let (a, b) = (a as u64 & mask, b as u64 & mask);
        let c = (self.eflags & CF) as u64;
        let (r, cf, of) = match op {
            0 | 2 => {
                let r = a + b + if op == 2 { c } else { 0 };
                (r, r > mask, (!(a ^ b) & (a ^ r)) & sign != 0)
            }
            3 => {
                let r = a.wrapping_sub(b).wrapping_sub(c);
                (r, a < b + c, ((a ^ b) & (a ^ r)) & sign != 0)
            }
            5 | 7 => {
                let r = a.wrapping_sub(b);
                (r, a < b, ((a ^ b) & (a ^ r)) & sign != 0)
            }
            1 => (a | b, false, false),
            4 => (a & b, false, false),
            _ => (a ^ b, false, false),
        };
        let r = r & mask;
        self.setf(CF, cf);
        self.setf(OF, of);
        self.setf(ZF, r == 0);
        self.setf(SF, r & sign != 0);
        self.setf(PF, (r as u8).count_ones() % 2 == 0);
        self.setf(AF, (a ^ b ^ r) & 0x10 != 0);
        r as u32
    }

    fn cond(&self, cc: u8) -> bool {
        let f = self.eflags;
        let (cf, zf, sf, of, pf) = (f & CF != 0, f & ZF != 0, f & SF != 0, f & OF != 0, f & PF != 0);
        let v = match cc >> 1 {
            0 => of,
            1 => cf,
            2 => zf,
            3 => cf || zf,
            4 => sf,
            5 => pf,
            6 => sf != of,
            _ => zf || sf != of,
        };
        v ^ (cc & 1 != 0)
    }

    fn push(&mut self, m: &mut Mem, v: u32) -> R<()> {
        let sp = self.r[ESP].wrapping_sub(4);
        m.w32(sp, v)?;
        self.r[ESP] = sp;
        Ok(())
    }
    fn pop(&mut self, m: &Mem) -> R<u32> {
        let v = m.r32(self.r[ESP])?;
        self.r[ESP] = self.r[ESP].wrapping_add(4);
        Ok(v)
    }

    pub fn step(&mut self, m: &mut Mem) -> R<()> {
        let start = self.eip;
        let op = self.f8(m)?;
        match op {
            // ALU: add/or/adc/sbb/and/sub/xor/cmp, forms 0..5
            0x00..=0x3F if (op & 7) < 6 => {
                let (alu, form) = (op >> 3, op & 7);
                let w = if form & 1 == 0 { 8 } else { 32 };
                match form {
                    0 | 1 | 2 | 3 => {
                        let (reg, rm) = self.modrm(m)?;
                        if form < 2 {
                            let (a, b) = (self.rd(m, rm, w)?, self.getr(reg, w));
                            let r = self.alu(alu, a, b, w);
                            if alu != 7 { self.wr(m, rm, w, r)?; }
                        } else {
                            let (a, b) = (self.getr(reg, w), self.rd(m, rm, w)?);
                            let r = self.alu(alu, a, b, w);
                            if alu != 7 { self.setr(reg, w, r); }
                        }
                    }
                    _ => {
                        let imm = if w == 8 { self.f8(m)? as u32 } else { self.f32(m)? };
                        let a = self.getr(EAX, w);
                        let r = self.alu(alu, a, imm, w);
                        if alu != 7 { self.setr(EAX, w, r); }
                    }
                }
            }
            0x40..=0x4F => {
                let i = (op & 7) as usize;
                let cf = self.eflags & CF;
                let r = self.alu(if op < 0x48 { 0 } else { 5 }, self.r[i], 1, 32);
                self.r[i] = r;
                self.eflags = (self.eflags & !CF) | cf;
            }
            0x50..=0x57 => { let v = self.r[(op & 7) as usize]; self.push(m, v)?; }
            0x58..=0x5F => { let v = self.pop(m)?; self.r[(op & 7) as usize] = v; }
            0x68 => { let v = self.f32(m)?; self.push(m, v)?; }
            0x6A => { let v = self.f8(m)? as i8 as i32 as u32; self.push(m, v)?; }
            0x70..=0x7F => {
                let d = self.f8(m)? as i8 as i32 as u32;
                if self.cond(op & 0xF) { self.eip = self.eip.wrapping_add(d); }
            }
            0x80 | 0x81 | 0x83 => {
                let w = if op == 0x80 { 8 } else { 32 };
                let (alu, rm) = self.modrm(m)?;
                let imm = match op { 0x81 => self.f32(m)?, 0x83 => self.f8(m)? as i8 as i32 as u32, _ => self.f8(m)? as u32 };
                let a = self.rd(m, rm, w)?;
                let r = self.alu(alu as u8, a, imm, w);
                if alu != 7 { self.wr(m, rm, w, r)?; }
            }
            0x84 | 0x85 => {
                let w = if op == 0x84 { 8 } else { 32 };
                let (reg, rm) = self.modrm(m)?;
                let (a, b) = (self.rd(m, rm, w)?, self.getr(reg, w));
                self.alu(4, a, b, w);
            }
            0x88 | 0x89 => {
                let w = if op == 0x88 { 8 } else { 32 };
                let (reg, rm) = self.modrm(m)?;
                let v = self.getr(reg, w);
                self.wr(m, rm, w, v)?;
            }
            0x8A | 0x8B => {
                let w = if op == 0x8A { 8 } else { 32 };
                let (reg, rm) = self.modrm(m)?;
                let v = self.rd(m, rm, w)?;
                self.setr(reg, w, v);
            }
            0x8D => {
                match self.modrm(m)? {
                    (reg, Op::M(a)) => self.r[reg] = a,
                    _ => return Err(Exit::Unimplemented { eip: start, opcode: op }),
                }
            }
            0x90 => {}
            0xB0..=0xB7 => { let v = self.f8(m)? as u32; self.set_reg8((op & 7) as usize, v); }
            0xB8..=0xBF => { let v = self.f32(m)?; self.r[(op & 7) as usize] = v; }
            0xC6 | 0xC7 => {
                let w = if op == 0xC6 { 8 } else { 32 };
                let (_, rm) = self.modrm(m)?;
                let v = if w == 8 { self.f8(m)? as u32 } else { self.f32(m)? };
                self.wr(m, rm, w, v)?;
            }
            0xC3 => self.eip = self.pop(m)?,
            0xE8 => { let d = self.f32(m)?; let ret = self.eip; self.push(m, ret)?; self.eip = self.eip.wrapping_add(d); }
            0xE9 => { let d = self.f32(m)?; self.eip = self.eip.wrapping_add(d); }
            0xEB => { let d = self.f8(m)? as i8 as i32 as u32; self.eip = self.eip.wrapping_add(d); }
            0xF4 => return Err(Exit::Halt),
            0xF8 => self.setf(CF, false),
            0xF9 => self.setf(CF, true),
            0xFC => self.setf(DF, false),
            0xFD => self.setf(DF, true),
            0x0F => {
                let op2 = self.f8(m)?;
                if (0x80..=0x8F).contains(&op2) {
                    let d = self.f32(m)?;
                    if self.cond(op2 & 0xF) { self.eip = self.eip.wrapping_add(d); }
                } else {
                    return Err(Exit::Unimplemented { eip: start, opcode: op2 });
                }
            }
            _ => return Err(Exit::Unimplemented { eip: start, opcode: op }),
        }
        Ok(())
    }
}
