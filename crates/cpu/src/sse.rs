//! SSE / SSE2 (no MMX registers, no SSE3+). XMM registers are u128; lane helpers below.
use super::{Cpu, Exit, Op, AF, CF, OF, PF, R, SF, ZF};
use plonk_mem::Mem;

fn mask(w: u32) -> u128 { if w >= 128 { u128::MAX } else { (1u128 << w) - 1 } }
fn lane(v: u128, i: u32, w: u32) -> u64 { ((v >> (i * w)) & mask(w)) as u64 }
fn put(v: &mut u128, i: u32, w: u32, x: u64) {
    let m = mask(w) << (i * w);
    *v = (*v & !m) | (((x as u128) & mask(w)) << (i * w));
}
fn sx(x: u64, w: u32) -> i64 { let s = 64 - w; ((x << s) as i64) >> s }
fn zip(a: u128, b: u128, w: u32, f: impl Fn(u64, u64) -> u64) -> u128 {
    let mut r = 0;
    for i in 0..128 / w { put(&mut r, i, w, f(lane(a, i, w), lane(b, i, w))); }
    r
}
fn map(a: u128, w: u32, f: impl Fn(u64) -> u64) -> u128 {
    let mut r = 0;
    for i in 0..128 / w { put(&mut r, i, w, f(lane(a, i, w))); }
    r
}
fn f32l(v: u128, i: u32) -> f32 { f32::from_bits(lane(v, i, 32) as u32) }
fn f64l(v: u128, i: u32) -> f64 { f64::from_bits(lane(v, i, 64)) }
fn pf32(v: &mut u128, i: u32, x: f32) { put(v, i, 32, x.to_bits() as u64) }
fn pf64(v: &mut u128, i: u32, x: f64) { put(v, i, 64, x.to_bits()) }
fn sat_s(x: i64, w: u32) -> u64 { let (lo, hi) = (-(1i64 << (w - 1)), (1i64 << (w - 1)) - 1); x.clamp(lo, hi) as u64 }
fn sat_u(x: i64, w: u32) -> u64 { x.clamp(0, (1i64 << w) - 1) as u64 }

fn fop32(op: u8, a: f32, b: f32) -> f32 {
    match op { 0x58 => a + b, 0x59 => a * b, 0x5C => a - b, 0x5E => a / b, 0x5D => if a < b { a } else { b }, 0x5F => if a > b { a } else { b }, _ => b.sqrt() }
}
fn fop64(op: u8, a: f64, b: f64) -> f64 {
    match op { 0x58 => a + b, 0x59 => a * b, 0x5C => a - b, 0x5E => a / b, 0x5D => if a < b { a } else { b }, 0x5F => if a > b { a } else { b }, _ => b.sqrt() }
}
fn cmp_pred(p: u8, a: f64, b: f64) -> bool {
    let un = a.is_nan() || b.is_nan();
    match p & 7 { 0 => !un && a == b, 1 => !un && a < b, 2 => !un && a <= b, 3 => un, 4 => un || a != b, 5 => un || !(a < b), 6 => un || !(a <= b), _ => !un }
}
fn to_i32(v: f64) -> u32 { if v.is_nan() || v >= 2147483648.0 || v < -2147483648.0 { 0x8000_0000 } else { v as i32 as u32 } }

impl Cpu {
    fn rint(&self, v: f64, trunc: bool) -> f64 {
        if trunc { return v.trunc(); }
        match (self.mxcsr >> 13) & 3 {
            0 => { let r = v.round(); if (v - v.trunc()).abs() == 0.5 && r % 2.0 != 0.0 { r - v.signum() } else { r } }
            1 => v.floor(),
            2 => v.ceil(),
            _ => v.trunc(),
        }
    }

    fn xsrc(&mut self, m: &Mem, rm: Op, bytes: u32) -> R<u128> {
        Ok(match rm {
            Op::R(i) => self.xmm[i],
            Op::M(a) => match bytes {
                4 => m.r32(a)? as u128,
                8 => m.r32(a)? as u128 | (m.r32(a + 4)? as u128) << 32,
                _ => (0..4).try_fold(0u128, |acc, k| m.r32(a + k * 4).map(|v| acc | (v as u128) << (k * 32)))?,
            },
        })
    }

    fn xstore(&mut self, m: &mut Mem, rm: Op, v: u128, bytes: u32) -> R<()> {
        match rm {
            Op::R(i) => { self.xmm[i] = if bytes >= 16 { v } else { v & mask(bytes * 8) } }
            Op::M(a) => for k in 0..bytes / 4 { m.w32(a + k * 4, (v >> (k * 32)) as u32)?; },
        }
        Ok(())
    }

    /// Scalar result: replace the low `w`-bit lane of the destination, keep the rest.
    fn merge_low(&mut self, reg: usize, w: u32, val: u64) { let mut d = self.xmm[reg]; put(&mut d, 0, w, val); self.xmm[reg] = d; }

    fn set_comis(&mut self, a: f64, b: f64) {
        let (z, p, c) = if a.is_nan() || b.is_nan() { (true, true, true) } else { (a == b, false, a < b) };
        self.eflags = (self.eflags & !(ZF | PF | CF | OF | SF | AF)) | if z { ZF } else { 0 } | if p { PF } else { 0 } | if c { CF } else { 0 };
    }

    pub(crate) fn sse(&mut self, m: &mut Mem, op2: u8, start: u32, rep: u8) -> R<()> {
        if op2 == 0x77 { return Ok(()); } // emms
        let pfx = if rep == 0xF3 { 3 } else if rep == 0xF2 { 2 } else if self.ow == 16 { 1 } else { 0 };
        let bad = Exit::Unimplemented { eip: start, opcode: op2 };
        let (reg, rm) = self.modrm(m)?;
        let a = self.xmm[reg];
        match op2 {
            // ---- moves ----
            0x10 => match pfx {
                0 | 1 => self.xmm[reg] = self.xsrc(m, rm, 16)?,
                _ => {
                    let w = if pfx == 3 { 32 } else { 64 };
                    let v = self.xsrc(m, rm, w / 8)?;
                    if matches!(rm, Op::R(_)) { self.merge_low(reg, w, v as u64) } else { self.xmm[reg] = v }
                }
            },
            0x11 => match pfx {
                0 | 1 => self.xstore(m, rm, a, 16)?,
                _ => {
                    let w = if pfx == 3 { 32 } else { 64 };
                    match rm { Op::R(i) => { let mut d = self.xmm[i]; put(&mut d, 0, w, lane(a, 0, w)); self.xmm[i] = d } Op::M(_) => self.xstore(m, rm, a, w / 8)? }
                }
            },
            0x12 | 0x13 | 0x16 | 0x17 => {
                let high = op2 >= 0x16;
                if op2 & 1 == 1 { // store half to memory
                    let v = if high { a >> 64 } else { a };
                    self.xstore(m, rm, v, 8)?;
                } else {
                    let (src, mut d) = (self.xsrc(m, rm, 8)?, a);
                    match (high, matches!(rm, Op::R(_))) {
                        (false, true) => put(&mut d, 0, 64, lane(src, 1, 64)), // movhlps
                        (false, false) => put(&mut d, 0, 64, lane(src, 0, 64)),
                        (true, true) => put(&mut d, 1, 64, lane(src, 0, 64)), // movlhps
                        (true, false) => put(&mut d, 1, 64, lane(src, 0, 64)),
                    }
                    self.xmm[reg] = d;
                }
            }
            0x14 | 0x15 => {
                let s = self.xsrc(m, rm, 16)?;
                let (w, h) = if pfx == 1 { (64, 1) } else { (32, 2) };
                let off = if op2 == 0x15 { h } else { 0 };
                let mut r = 0;
                for i in 0..h { put(&mut r, 2 * i, w, lane(a, off + i, w)); put(&mut r, 2 * i + 1, w, lane(s, off + i, w)); }
                self.xmm[reg] = r;
            }
            0x28 => self.xmm[reg] = self.xsrc(m, rm, 16)?,
            0x29 | 0x2B | 0x7F | 0xE7 => self.xstore(m, rm, a, 16)?,
            0x6F => self.xmm[reg] = if pfx >= 1 { self.xsrc(m, rm, 16)? } else { return Err(bad) },
            0x6E if pfx == 1 => { let v = match rm { Op::R(i) => self.r[i], Op::M(ad) => m.r32(ad)? }; self.xmm[reg] = v as u128 }
            0x7E if pfx == 1 => { let v = a as u32; match rm { Op::R(i) => self.r[i] = v, Op::M(ad) => m.w32(ad, v)? } }
            0x7E if pfx == 3 => self.xmm[reg] = self.xsrc(m, rm, 8)? & mask(64),
            0xD6 if pfx == 1 => self.xstore(m, rm, a, 8)?,
            0xC3 => { let v = self.r[reg]; if let Op::M(ad) = rm { m.w32(ad, v)? } }
            // ---- floating point arithmetic ----
            0x51 | 0x58 | 0x59 | 0x5C | 0x5D | 0x5E | 0x5F => match pfx {
                0 => { let s = self.xsrc(m, rm, 16)?; let mut r = 0; for i in 0..4 { pf32(&mut r, i, fop32(op2, f32l(a, i), f32l(s, i))) } self.xmm[reg] = r }
                1 => { let s = self.xsrc(m, rm, 16)?; let mut r = 0; for i in 0..2 { pf64(&mut r, i, fop64(op2, f64l(a, i), f64l(s, i))) } self.xmm[reg] = r }
                3 => { let s = self.xsrc(m, rm, 4)?; self.merge_low(reg, 32, fop32(op2, f32l(a, 0), f32l(s, 0)).to_bits() as u64) }
                _ => { let s = self.xsrc(m, rm, 8)?; self.merge_low(reg, 64, fop64(op2, f64l(a, 0), f64l(s, 0)).to_bits()) }
            },
            0x54..=0x57 => {
                let s = self.xsrc(m, rm, 16)?;
                self.xmm[reg] = match op2 { 0x54 => a & s, 0x55 => !a & s, 0x56 => a | s, _ => a ^ s };
            }
            0xC2 => {
                let w = match pfx { 0 | 3 => 32, _ => 64 };
                let scalar = pfx >= 2;
                let s = self.xsrc(m, rm, if scalar { w / 8 } else { 16 })?;
                let p = m.r8(self.eip)?; self.eip += 1;
                let n = if scalar { 1 } else { 128 / w };
                let mut r = a;
                for i in 0..n {
                    let (x, y) = if w == 32 { (f32l(a, i) as f64, f32l(s, i) as f64) } else { (f64l(a, i), f64l(s, i)) };
                    put(&mut r, i, w, if cmp_pred(p, x, y) { mask(w) as u64 } else { 0 });
                }
                self.xmm[reg] = r;
            }
            0x2E | 0x2F => {
                let (x, y) = if pfx == 1 { (f64l(a, 0), f64l(self.xsrc(m, rm, 8)?, 0)) } else { (f32l(a, 0) as f64, f32l(self.xsrc(m, rm, 4)?, 0) as f64) };
                self.set_comis(x, y);
            }
            // ---- conversions ----
            0x2A => { // cvtsi2ss / cvtsi2sd
                let v = match rm { Op::R(i) => self.r[i], Op::M(ad) => m.r32(ad)? } as i32;
                if pfx == 3 { self.merge_low(reg, 32, (v as f32).to_bits() as u64) } else if pfx == 2 { self.merge_low(reg, 64, (v as f64).to_bits()) } else { return Err(bad) }
            }
            0x2C | 0x2D => {
                let v = if pfx == 3 { f32l(self.xsrc(m, rm, 4)?, 0) as f64 } else if pfx == 2 { f64l(self.xsrc(m, rm, 8)?, 0) } else { return Err(bad) };
                self.r[reg] = to_i32(self.rint(v, op2 == 0x2C));
            }
            0x50 => {
                let s = match rm { Op::R(i) => self.xmm[i], _ => return Err(bad) };
                let w = if pfx == 1 { 64 } else { 32 };
                self.r[reg] = (0..128 / w).fold(0, |acc, i| acc | ((lane(s, i, w) >> (w - 1)) as u32) << i);
            }
            0x5A => match pfx {
                0 => { let s = self.xsrc(m, rm, 8)?; let mut r = 0; for i in 0..2 { pf64(&mut r, i, f32l(s, i) as f64) } self.xmm[reg] = r }
                1 => { let s = self.xsrc(m, rm, 16)?; let mut r = 0; for i in 0..2 { pf32(&mut r, i, f64l(s, i) as f32) } self.xmm[reg] = r }
                3 => { let s = self.xsrc(m, rm, 4)?; self.merge_low(reg, 64, (f32l(s, 0) as f64).to_bits()) }
                _ => { let s = self.xsrc(m, rm, 8)?; self.merge_low(reg, 32, (f64l(s, 0) as f32).to_bits() as u64) }
            },
            0x5B => {
                let s = self.xsrc(m, rm, 16)?;
                let mut r = 0;
                for i in 0..4 {
                    match pfx {
                        0 => pf32(&mut r, i, lane(s, i, 32) as u32 as i32 as f32),
                        _ => put(&mut r, i, 32, to_i32(self.rint(f32l(s, i) as f64, pfx == 3)) as u64),
                    }
                }
                self.xmm[reg] = r;
            }
            0xE6 => {
                let s = self.xsrc(m, rm, if pfx == 3 { 8 } else { 16 })?;
                let mut r = 0;
                match pfx {
                    3 => for i in 0..2 { pf64(&mut r, i, lane(s, i, 32) as u32 as i32 as f64) },
                    1 | 2 => for i in 0..2 { put(&mut r, i, 32, to_i32(self.rint(f64l(s, i), pfx == 1)) as u64) },
                    _ => return Err(bad),
                }
                self.xmm[reg] = r;
            }
            // ---- shuffles / unpacks ----
            0x70 => {
                let s = self.xsrc(m, rm, 16)?;
                let imm = m.r8(self.eip)? as u32; self.eip += 1;
                let mut r = s;
                match pfx {
                    1 => for i in 0..4 { put(&mut r, i, 32, lane(s, (imm >> (2 * i)) & 3, 32)) },
                    2 => for i in 0..4 { put(&mut r, i, 16, lane(s, (imm >> (2 * i)) & 3, 16)) },
                    3 => for i in 0..4 { put(&mut r, 4 + i, 16, lane(s, 4 + ((imm >> (2 * i)) & 3), 16)) },
                    _ => return Err(bad),
                }
                self.xmm[reg] = r;
            }
            0xC6 => {
                let s = self.xsrc(m, rm, 16)?;
                let imm = m.r8(self.eip)? as u32; self.eip += 1;
                let mut r = 0;
                if pfx == 1 { put(&mut r, 0, 64, lane(a, imm & 1, 64)); put(&mut r, 1, 64, lane(s, (imm >> 1) & 1, 64)); }
                else { for i in 0..2 { put(&mut r, i, 32, lane(a, (imm >> (2 * i)) & 3, 32)); put(&mut r, 2 + i, 32, lane(s, (imm >> (4 + 2 * i)) & 3, 32)); } }
                self.xmm[reg] = r;
            }
            0x60..=0x62 | 0x68..=0x6A | 0x6C | 0x6D => {
                let s = self.xsrc(m, rm, 16)?;
                let w = match op2 & 7 { 0 => 8, 1 => 16, 2 => 32, _ => 64 };
                let n = 128 / w;
                let off = if op2 >= 0x68 && op2 != 0x6C { n / 2 } else if op2 == 0x6D { n / 2 } else { 0 };
                let mut r = 0;
                for i in 0..n / 2 { put(&mut r, 2 * i, w, lane(a, off + i, w)); put(&mut r, 2 * i + 1, w, lane(s, off + i, w)); }
                self.xmm[reg] = r;
            }
            0x63 | 0x67 | 0x6B => {
                let s = self.xsrc(m, rm, 16)?;
                let (w, nw) = if op2 == 0x6B { (32, 16) } else { (16, 8) };
                let mut r = 0;
                for i in 0..128 / w {
                    let (x, y) = (sx(lane(a, i, w), w), sx(lane(s, i, w), w));
                    let f = |v: i64| if op2 == 0x67 { sat_u(v, nw) } else { sat_s(v, nw) };
                    put(&mut r, i, nw, f(x));
                    put(&mut r, 128 / w + i, nw, f(y));
                }
                self.xmm[reg] = r;
            }
            // ---- shifts ----
            0x71..=0x73 => {
                let Op::R(i) = rm else { return Err(bad) };
                let n = m.r8(self.eip)? as u64; self.eip += 1;
                let w = match op2 { 0x71 => 16, 0x72 => 32, _ => 64 };
                let v = self.xmm[i];
                self.xmm[i] = match (op2, reg) {
                    (0x73, 3) => if n >= 16 { 0 } else { v >> (8 * n) },
                    (0x73, 7) => if n >= 16 { 0 } else { v << (8 * n) },
                    (_, 2) => map(v, w, |x| if n >= w as u64 { 0 } else { x >> n }),
                    (_, 4) => map(v, w, |x| (sx(x, w) >> n.min(w as u64 - 1)) as u64),
                    (_, 6) => map(v, w, |x| if n >= w as u64 { 0 } else { x << n }),
                    _ => return Err(bad),
                };
            }
            0xD1..=0xD3 | 0xE1 | 0xE2 | 0xF1..=0xF3 => {
                let n = self.xsrc(m, rm, 16)? as u64;
                let w = match op2 & 3 { 1 => 16, 2 => 32, _ => 64 };
                self.xmm[reg] = match op2 >> 4 {
                    0xD => map(a, w, |x| if n >= w as u64 { 0 } else { x >> n }),
                    0xE => map(a, w, |x| (sx(x, w) >> n.min(w as u64 - 1)) as u64),
                    _ => map(a, w, |x| if n >= w as u64 { 0 } else { x << n }),
                };
            }
            // ---- integer arithmetic / compare / logic ----
            0x64..=0x66 | 0x74..=0x76 => {
                let s = self.xsrc(m, rm, 16)?;
                let w = match op2 { 0x64 | 0x74 => 8, 0x65 | 0x75 => 16, _ => 32 };
                self.xmm[reg] = if op2 >= 0x74 { zip(a, s, w, |x, y| if x == y { u64::MAX } else { 0 }) } else { zip(a, s, w, |x, y| if sx(x, w) > sx(y, w) { u64::MAX } else { 0 }) };
            }
            0xD4 | 0xD5 | 0xD8 | 0xD9 | 0xDA | 0xDB | 0xDC | 0xDD | 0xDE | 0xDF | 0xE0 | 0xE3 | 0xE4 | 0xE5 | 0xE8 | 0xE9 | 0xEA | 0xEB | 0xEC | 0xED | 0xEE | 0xEF
            | 0xF8..=0xFE => {
                let s = self.xsrc(m, rm, 16)?;
                self.xmm[reg] = match op2 {
                    0xDB => a & s, 0xDF => !a & s, 0xEB => a | s, 0xEF => a ^ s,
                    0xFC => zip(a, s, 8, |x, y| x.wrapping_add(y)), 0xFD => zip(a, s, 16, |x, y| x.wrapping_add(y)),
                    0xFE => zip(a, s, 32, |x, y| x.wrapping_add(y)), 0xD4 => zip(a, s, 64, |x, y| x.wrapping_add(y)),
                    0xF8 => zip(a, s, 8, |x, y| x.wrapping_sub(y)), 0xF9 => zip(a, s, 16, |x, y| x.wrapping_sub(y)),
                    0xFA => zip(a, s, 32, |x, y| x.wrapping_sub(y)), 0xFB => zip(a, s, 64, |x, y| x.wrapping_sub(y)),
                    0xD5 => zip(a, s, 16, |x, y| x.wrapping_mul(y)),
                    0xE5 => zip(a, s, 16, |x, y| ((sx(x, 16) * sx(y, 16)) >> 16) as u64),
                    0xE4 => zip(a, s, 16, |x, y| (x * y) >> 16),
                    0xDA => zip(a, s, 8, |x, y| x.min(y)), 0xDE => zip(a, s, 8, |x, y| x.max(y)),
                    0xEA => zip(a, s, 16, |x, y| sx(x, 16).min(sx(y, 16)) as u64), 0xEE => zip(a, s, 16, |x, y| sx(x, 16).max(sx(y, 16)) as u64),
                    0xDC => zip(a, s, 8, |x, y| (x + y).min(255)), 0xDD => zip(a, s, 16, |x, y| (x + y).min(65535)),
                    0xD8 => zip(a, s, 8, |x, y| x.saturating_sub(y)), 0xD9 => zip(a, s, 16, |x, y| x.saturating_sub(y)),
                    0xEC => zip(a, s, 8, |x, y| sat_s(sx(x, 8) + sx(y, 8), 8)), 0xED => zip(a, s, 16, |x, y| sat_s(sx(x, 16) + sx(y, 16), 16)),
                    0xE8 => zip(a, s, 8, |x, y| sat_s(sx(x, 8) - sx(y, 8), 8)), 0xE9 => zip(a, s, 16, |x, y| sat_s(sx(x, 16) - sx(y, 16), 16)),
                    0xE0 => zip(a, s, 8, |x, y| (x + y + 1) >> 1), 0xE3 => zip(a, s, 16, |x, y| (x + y + 1) >> 1),
                    _ => return Err(bad),
                };
            }
            0xF4 => { let s = self.xsrc(m, rm, 16)?; let mut r = 0; for i in 0..2 { put(&mut r, i, 64, lane(a, 2 * i, 32) * lane(s, 2 * i, 32)); } self.xmm[reg] = r }
            0xF5 => {
                let s = self.xsrc(m, rm, 16)?;
                let mut r = 0;
                for i in 0..4 { put(&mut r, i, 32, (sx(lane(a, 2 * i, 16), 16) * sx(lane(s, 2 * i, 16), 16) + sx(lane(a, 2 * i + 1, 16), 16) * sx(lane(s, 2 * i + 1, 16), 16)) as u64); }
                self.xmm[reg] = r;
            }
            0xF6 => {
                let s = self.xsrc(m, rm, 16)?;
                let mut r = 0;
                for h in 0..2 { put(&mut r, h, 64, (0..8).map(|i| (lane(a, h * 8 + i, 8) as i64 - lane(s, h * 8 + i, 8) as i64).unsigned_abs()).sum()); }
                self.xmm[reg] = r;
            }
            0xD7 => {
                let Op::R(i) = rm else { return Err(bad) };
                let v = self.xmm[i];
                self.r[reg] = (0..16).fold(0, |acc, k| acc | ((lane(v, k, 8) >> 7) as u32) << k);
            }
            // ---- state ----
            0xAE => {
                let Op::M(ad) = rm else { return Err(bad) };
                match reg {
                    0 => self.fxsave(m, ad)?,
                    1 => self.fxrstor(m, ad)?,
                    2 => self.mxcsr = m.r32(ad)?,
                    3 => m.w32(ad, self.mxcsr)?,
                    _ => {} // clflush
                }
            }
            _ => return Err(bad),
        }
        Ok(())
    }

    fn fxsave(&mut self, m: &mut Mem, a: u32) -> R<()> {
        m.w16(a, self.fcw as u16)?;
        m.w16(a + 2, self.fsw_get() as u16)?;
        m.w16(a + 4, 0xFF)?; // abridged tag: all valid (we do not track emptiness)
        m.w32(a + 24, self.mxcsr)?;
        m.w32(a + 28, 0xFFFF)?;
        for i in 0..8u32 {
            let (lo, se) = super::fpu::f64_to_ext(self.st(i));
            m.w32(a + 32 + i * 16, lo as u32)?;
            m.w32(a + 36 + i * 16, (lo >> 32) as u32)?;
            m.w32(a + 40 + i * 16, se as u32)?;
            m.w32(a + 44 + i * 16, 0)?;
        }
        for i in 0..8u32 { self.xstore(m, Op::M(a + 160 + i * 16), self.xmm[i as usize], 16)?; }
        Ok(())
    }

    fn fxrstor(&mut self, m: &mut Mem, a: u32) -> R<()> {
        self.fcw = m.r16(a)? as u32;
        self.fsw = m.r16(a + 2)? as u32;
        self.ftop = (self.fsw >> 11) & 7;
        self.mxcsr = m.r32(a + 24)?;
        for i in 0..8u32 {
            let lo = m.r32(a + 32 + i * 16)? as u64 | (m.r32(a + 36 + i * 16)? as u64) << 32;
            let v = super::fpu::ext_to_f64(lo, m.r16(a + 40 + i * 16)?);
            self.set_st(i, v);
        }
        for i in 0..8u32 { self.xmm[i as usize] = self.xsrc(m, Op::M(a + 160 + i * 16), 16)?; }
        Ok(())
    }
}
