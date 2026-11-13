//! x87 FPU. Registers are held as f64 (not 80-bit): good enough for CRT/GDI math, not bit-exact.
use super::{Cpu, Exit, Op, CF, PF, R, ZF};
use plonk_mem::Mem;

pub(crate) fn ext_to_f64(lo: u64, se: u16) -> f64 {
    let sign = if se & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = (se & 0x7fff) as i32;
    if exp == 0 && lo == 0 { return sign * 0.0; }
    if exp == 0x7fff { return if lo << 1 == 0 { sign * f64::INFINITY } else { f64::NAN }; }
    let e = exp - 16383 - 63;
    let (h, r) = (e / 2, e - e / 2);
    sign * (lo as f64) * 2f64.powi(h) * 2f64.powi(r)
}

pub(crate) fn f64_to_ext(v: f64) -> (u64, u16) {
    let sign: u16 = if v.is_sign_negative() { 0x8000 } else { 0 };
    if v == 0.0 { return (0, sign); }
    if v.is_infinite() { return (1 << 63, sign | 0x7fff); }
    if v.is_nan() { return (0xC000_0000_0000_0000, 0x7fff); }
    let bits = v.to_bits();
    let (mut e, mut m) = (((bits >> 52) & 0x7ff) as i32, bits & ((1 << 52) - 1));
    if e == 0 {
        e = 1;
        while m & (1 << 52) == 0 { m <<= 1; e -= 1; }
    } else {
        m |= 1 << 52;
    }
    (m << 11, sign | (e - 1023 + 16383) as u16)
}

impl Cpu {
    pub(crate) fn st(&self, i: u32) -> f64 { self.fst[((self.ftop + i) & 7) as usize] }
    pub(crate) fn set_st(&mut self, i: u32, v: f64) { self.fst[((self.ftop + i) & 7) as usize] = v; }
    fn fpush(&mut self, v: f64) { self.ftop = (self.ftop.wrapping_sub(1)) & 7; self.set_st(0, v); }
    fn fpop(&mut self) { self.ftop = (self.ftop + 1) & 7; }
    pub(crate) fn fsw_get(&self) -> u32 { (self.fsw & !0x3800) | (self.ftop << 11) }

    fn fcc(&mut self, c3: bool, c2: bool, c0: bool) {
        self.fsw = (self.fsw & !0x4500) | (c3 as u32) << 14 | (c2 as u32) << 10 | (c0 as u32) << 8;
    }
    fn fcmp(&mut self, a: f64, b: f64) {
        if a.is_nan() || b.is_nan() { self.fcc(true, true, true) }
        else if a > b { self.fcc(false, false, false) }
        else if a < b { self.fcc(false, false, true) }
        else { self.fcc(true, false, false) }
    }
    fn fcomi(&mut self, a: f64, b: f64) {
        let (z, p, c) = if a.is_nan() || b.is_nan() { (true, true, true) } else { (a == b, false, a < b) };
        self.eflags = (self.eflags & !(ZF | PF | CF)) | if z { ZF } else { 0 } | if p { PF } else { 0 } | if c { CF } else { 0 };
    }
    fn round(&self, v: f64, trunc: bool) -> f64 {
        if trunc { return v.trunc(); }
        match (self.fcw >> 10) & 3 {
            0 => v.round_ties_even(),
            1 => v.floor(),
            2 => v.ceil(),
            _ => v.trunc(),
        }
    }
    fn arith(sub: u32, a: f64, b: f64) -> f64 {
        match sub { 0 => a + b, 1 => a * b, 4 => a - b, 5 => b - a, 6 => a / b, _ => b / a }
    }
    fn store_int(&mut self, m: &mut Mem, addr: u32, bits: u32, trunc: bool) -> R<()> {
        let v = self.round(self.st(0), trunc);
        let (lo, hi) = match bits { 16 => (-32768.0, 32767.0), 32 => (-2147483648.0, 2147483647.0), _ => (-9.223372036854775808e18, 9.223372036854775807e18) };
        let i: i64 = if v.is_nan() || v < lo || v > hi { i64::MIN >> (64 - bits) } else { v as i64 };
        match bits { 16 => m.w16(addr, i as u16)?, 32 => m.w32(addr, i as u32)?, _ => { m.w32(addr, i as u32)?; m.w32(addr + 4, (i >> 32) as u32)?; } }
        Ok(())
    }

    pub(crate) fn fpu(&mut self, m: &mut Mem, op: u8, start: u32) -> R<()> {
        let b = m.r8(self.eip)?;
        let unimpl = Err(Exit::Unimplemented { eip: start, opcode: op });
        if b >> 6 != 3 {
            let (reg, rm) = self.modrm(m)?;
            let Op::M(a) = rm else { return unimpl };
            let reg = reg as u32;
            match op {
                0xD8 | 0xDC | 0xDA | 0xDE => {
                    let v = match op {
                        0xD8 => f32::from_bits(m.r32(a)?) as f64,
                        0xDC => f64::from_bits(m.r32(a)? as u64 | (m.r32(a + 4)? as u64) << 32),
                        0xDA => m.r32(a)? as i32 as f64,
                        _ => m.r16(a)? as i16 as f64,
                    };
                    if reg == 2 || reg == 3 { let s0 = self.st(0); self.fcmp(s0, v); if reg == 3 { self.fpop(); } }
                    else { let r = Self::arith(reg, self.st(0), v); self.set_st(0, r); }
                }
                0xD9 => match reg {
                    0 => { let v = f32::from_bits(m.r32(a)?) as f64; self.fpush(v); }
                    2 | 3 => { m.w32(a, (self.st(0) as f32).to_bits())?; if reg == 3 { self.fpop(); } }
                    4 => { self.fcw = m.r16(a)? as u32; self.fsw = m.r16(a + 4)? as u32; self.ftop = (self.fsw >> 11) & 7; }
                    5 => self.fcw = m.r16(a)? as u32,
                    6 => { m.w32(a, self.fcw | 0xFFFF_0000)?; m.w32(a + 4, self.fsw_get() | 0xFFFF_0000)?; m.w32(a + 8, 0xFFFF_FFFF)?; for i in 3..7 { m.w32(a + i * 4, 0)?; } }
                    7 => m.w16(a, self.fcw as u16)?,
                    _ => return unimpl,
                },
                0xDB => match reg {
                    0 => { let v = m.r32(a)? as i32 as f64; self.fpush(v); }
                    1 => { self.store_int(m, a, 32, true)?; self.fpop(); }
                    2 | 3 => { self.store_int(m, a, 32, false)?; if reg == 3 { self.fpop(); } }
                    5 => { let (lo, se) = (m.r32(a)? as u64 | (m.r32(a + 4)? as u64) << 32, m.r16(a + 8)?); self.fpush(ext_to_f64(lo, se)); }
                    7 => { let (lo, se) = f64_to_ext(self.st(0)); m.w32(a, lo as u32)?; m.w32(a + 4, (lo >> 32) as u32)?; m.w16(a + 8, se)?; self.fpop(); }
                    _ => return unimpl,
                },
                0xDD => match reg {
                    0 => { let v = f64::from_bits(m.r32(a)? as u64 | (m.r32(a + 4)? as u64) << 32); self.fpush(v); }
                    1 => { self.store_int(m, a, 64, true)?; self.fpop(); }
                    2 | 3 => { let bits = self.st(0).to_bits(); m.w32(a, bits as u32)?; m.w32(a + 4, (bits >> 32) as u32)?; if reg == 3 { self.fpop(); } }
                    4 => { // frstor: 28-byte environment, then ST0..ST7 as 80-bit
                        self.fcw = m.r16(a)? as u32; self.fsw = m.r16(a + 4)? as u32; self.ftop = (self.fsw >> 11) & 7;
                        for i in 0..8u32 {
                            let o = a + 28 + i * 10;
                            let lo = m.r32(o)? as u64 | (m.r32(o + 4)? as u64) << 32;
                            let v = ext_to_f64(lo, m.r16(o + 8)?);
                            self.set_st(i, v);
                        }
                    }
                    6 => { // fnsave: store state, then reinitialise the FPU
                        m.w32(a, self.fcw | 0xFFFF_0000)?; m.w32(a + 4, self.fsw_get() | 0xFFFF_0000)?; m.w32(a + 8, 0xFFFF_FFFF)?;
                        for o in [12u32, 16, 20, 24] { m.w32(a + o, 0)?; }
                        for i in 0..8u32 {
                            let (lo, se) = f64_to_ext(self.st(i));
                            let o = a + 28 + i * 10;
                            m.w32(o, lo as u32)?; m.w32(o + 4, (lo >> 32) as u32)?; m.w16(o + 8, se)?;
                        }
                        self.fcw = 0x37F; self.fsw = 0; self.ftop = 0;
                    }
                    7 => m.w16(a, self.fsw_get() as u16)?,
                    _ => return unimpl,
                },
                _ => match reg { // 0xDF
                    0 => { let v = m.r16(a)? as i16 as f64; self.fpush(v); }
                    1 => { self.store_int(m, a, 16, true)?; self.fpop(); }
                    2 | 3 => { self.store_int(m, a, 16, false)?; if reg == 3 { self.fpop(); } }
                    5 => { let v = (m.r32(a)? as u64 | (m.r32(a + 4)? as u64) << 32) as i64 as f64; self.fpush(v); }
                    7 => { self.store_int(m, a, 64, false)?; self.fpop(); }
                    _ => return unimpl,
                },
            }
            return Ok(());
        }
        self.eip += 1;
        let (reg, i) = (((b >> 3) & 7) as u32, (b & 7) as u32);
        let s0 = self.st(0);
        match op {
            0xD8 => {
                if reg == 2 || reg == 3 { let si = self.st(i); self.fcmp(s0, si); if reg == 3 { self.fpop(); } }
                else { let r = Self::arith(reg, s0, self.st(i)); self.set_st(0, r); }
            }
            0xDC | 0xDE => {
                if op == 0xDE && b == 0xD9 { let s1 = self.st(1); self.fcmp(s0, s1); self.fpop(); self.fpop(); return Ok(()); }
                // reversed sub/div encoding for the ST(i) destination forms
                let sub = match reg { 4 => 5, 5 => 4, 6 => 7, 7 => 6, r => r };
                if reg == 2 || reg == 3 { return unimpl; }
                let r = Self::arith(sub, self.st(i), s0);
                self.set_st(i, r);
                if op == 0xDE { self.fpop(); }
            }
            0xD9 => match (reg, i) {
                (0, _) => { let v = self.st(i); self.fpush(v); }
                (1, _) => { let (a, c) = (self.st(0), self.st(i)); self.set_st(0, c); self.set_st(i, a); }
                (2, 0) => {}
                (4, 0) => self.set_st(0, -s0),
                (4, 1) => self.set_st(0, s0.abs()),
                (4, 4) => self.fcmp(s0, 0.0),
                (4, 5) => {
                    let (c3, c2, c0) = if s0.is_nan() { (false, false, true) } else if s0.is_infinite() { (false, true, true) } else if s0 == 0.0 { (true, false, false) } else { (false, true, false) };
                    self.fcc(c3, c2, c0);
                    self.fsw = (self.fsw & !0x200) | (s0.is_sign_negative() as u32) << 9;
                }
                (5, 0) => self.fpush(1.0),
                (5, 1) => self.fpush(10f64.log2()),
                (5, 2) => self.fpush(std::f64::consts::LOG2_E),
                (5, 3) => self.fpush(std::f64::consts::PI),
                (5, 4) => self.fpush(2f64.log10()),
                (5, 5) => self.fpush(std::f64::consts::LN_2),
                (5, 6) => self.fpush(0.0),
                (6, 0) => self.set_st(0, 2f64.powf(s0) - 1.0),
                (6, 1) => { let r = self.st(1) * s0.log2(); self.set_st(1, r); self.fpop(); }
                (6, 2) => { self.set_st(0, s0.tan()); self.fpush(1.0); self.fsw &= !0x400; }
                (6, 3) => { let r = self.st(1).atan2(s0); self.set_st(1, r); self.fpop(); }
                (6, 4) => { let e = s0.abs().log2().floor(); let sig = s0 / 2f64.powf(e); self.set_st(0, e); self.fpush(sig); }
                (6, 5) | (7, 0) => { let d = self.st(1); let q = (s0 / d).trunc(); self.set_st(0, s0 - q * d); self.fsw &= !0x400; }
                (6, 6) => self.ftop = (self.ftop.wrapping_sub(1)) & 7,
                (6, 7) => self.ftop = (self.ftop + 1) & 7,
                (7, 1) => { let r = self.st(1) * (s0 + 1.0).log2(); self.set_st(1, r); self.fpop(); }
                (7, 2) => self.set_st(0, s0.sqrt()),
                (7, 3) => { self.set_st(0, s0.sin()); self.fpush(s0.cos()); self.fsw &= !0x400; }
                (7, 4) => { let r = self.round(s0, false); self.set_st(0, r); }
                (7, 5) => { let r = s0 * 2f64.powf(self.st(1).trunc()); self.set_st(0, r); }
                (7, 6) => { self.set_st(0, s0.sin()); self.fsw &= !0x400; }
                (7, 7) => { self.set_st(0, s0.cos()); self.fsw &= !0x400; }
                _ => return unimpl,
            },
            0xDA => match (reg, i) {
                (0..=3, _) => {
                    let take = match reg { 0 => self.eflags & CF != 0, 1 => self.eflags & ZF != 0, 2 => self.eflags & (CF | ZF) != 0, _ => self.eflags & PF != 0 };
                    if take { let v = self.st(i); self.set_st(0, v); }
                }
                (5, 1) => { let s1 = self.st(1); self.fcmp(s0, s1); self.fpop(); self.fpop(); }
                _ => return unimpl,
            },
            0xDB => match (reg, i) {
                (0..=3, _) => {
                    let take = match reg { 0 => self.eflags & CF == 0, 1 => self.eflags & ZF == 0, 2 => self.eflags & (CF | ZF) == 0, _ => self.eflags & PF == 0 };
                    if take { let v = self.st(i); self.set_st(0, v); }
                }
                (4, 2) => self.fsw &= !0x80FF,
                (4, 3) => { self.fcw = 0x37F; self.fsw = 0; self.ftop = 0; }
                (5, _) | (6, _) => { let si = self.st(i); self.fcomi(s0, si); }
                _ => return unimpl,
            },
            0xDD => match reg {
                0 => {}
                2 | 3 => { self.set_st(i, s0); if reg == 3 { self.fpop(); } }
                4 | 5 => { let si = self.st(i); self.fcmp(s0, si); if reg == 5 { self.fpop(); } }
                _ => return unimpl,
            },
            _ => match (reg, i) { // 0xDF
                (4, 0) => { let sw = self.fsw_get(); self.r[0] = (self.r[0] & 0xffff_0000) | sw; }
                (5, _) | (6, _) => { let si = self.st(i); self.fcomi(s0, si); self.fpop(); }
                _ => return unimpl,
            },
        }
        Ok(())
    }
}
