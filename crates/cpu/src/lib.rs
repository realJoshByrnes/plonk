//! i386 interpreter core. 32-bit protected-mode flat model (+ FS/GS bases). No FPU/SSE yet.
use plonk_mem::{Fault, Mem};

mod fast;
mod fpu;
mod sse;

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
const FLAGS_MASK: u32 = CF | PF | AF | ZF | SF | 0x100 | 0x200 | DF | OF;

pub use fast::CacheSlot;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Exit {
    Halt,
    StepLimit,
    /// eip entered the thunk range; guest-to-host call. Value is the thunk address.
    Thunk(u32),
    /// `int n` executed; eip is already past the instruction.
    Interrupt(u8),
    Unimplemented { eip: u32, opcode: u8 },
    /// ud2 or invalid encoding
    Undefined(u32),
    DivideError(u32),
    Fault(u32),
}

impl From<Fault> for Exit {
    fn from(f: Fault) -> Self { Exit::Fault(f.0) }
}

#[derive(Clone, Copy)]
enum Op { R(usize), M(u32) }

#[derive(Clone, Debug, Default)]
pub struct Cpu {
    /// Instructions retired through `run` (benchmarking).
    pub retired: u64,
    pub r: [u32; 8],
    pub eip: u32,
    pub eflags: u32,
    /// Half-open eip range [lo, hi) that traps out as Exit::Thunk. Empty by default.
    pub thunk: (u32, u32),
    pub fs_base: u32,
    /// 0x67 address-size prefix seen on the current instruction (16-bit ModRM addressing).
    addr16: bool,
    /// Debugging: run the plain interpreter (no block cache) and remember the last 64 instruction addresses.
    pub debug_ring: bool,
    pub ring: Vec<u32>,
    pub ring_pos: usize,
    pub gs_base: u32,
    /// Linux personality: segment loads of fs/gs pick their base from `tls` (GDT entries 6..=8, set_thread_area).
    pub linux: bool,
    pub tls: [u32; 3],
    pub sel_fs: u32,
    pub sel_gs: u32,
    pub tsc: u64,
    /// Address of the instruction being executed (for exception records).
    pub insn_start: u32,
    pub fst: [f64; 8],
    pub ftop: u32,
    pub fsw: u32,
    pub fcw: u32,
    pub xmm: [u128; 8],
    pub mxcsr: u32,
    seg: u32,
    ow: u32,
    cache: fast::CacheSlot,
    pending: Option<Exit>,
    nsteps: u64,
}

type R<T> = Result<T, Exit>;

fn maskw(w: u32) -> u32 { if w == 32 { u32::MAX } else { (1u32 << w) - 1 } }
fn sx(v: u32, w: u32) -> i64 {
    match w { 8 => v as u8 as i8 as i64, 16 => v as u16 as i16 as i64, _ => v as i32 as i64 }
}

impl Cpu {
    /// Detach / attach the decoded-code cache (a process's threads share one).
    pub fn take_cache(&mut self) -> CacheSlot { std::mem::take(&mut self.cache) }
    pub fn put_cache(&mut self, c: CacheSlot) { self.cache = c; }
    pub fn new() -> Self { Cpu { eflags: 0x202, ow: 32, fcw: 0x37F, mxcsr: 0x1F80, ..Default::default() } }

    /// Reference interpreter loop (one `step` per instruction); `run` is the cached fast path.
    pub fn run_slow(&mut self, mem: &mut Mem, max_steps: u64) -> Exit {
        for i in 0..max_steps {
            if let Err(e) = self.step(mem) { self.retired += i; return e; }
        }
        self.retired += max_steps;
        Exit::StepLimit
    }

    fn f8(&mut self, m: &Mem) -> R<u8> { let v = m.r8(self.eip)?; self.eip = self.eip.wrapping_add(1); Ok(v) }
    fn f16(&mut self, m: &Mem) -> R<u16> { let v = m.r16(self.eip)?; self.eip = self.eip.wrapping_add(2); Ok(v) }
    fn f32(&mut self, m: &Mem) -> R<u32> { let v = m.r32(self.eip)?; self.eip = self.eip.wrapping_add(4); Ok(v) }
    /// Immediate of width w (8/16/32), zero-extended.
    fn imm(&mut self, m: &Mem, w: u32) -> R<u32> {
        Ok(match w { 8 => self.f8(m)? as u32, 16 => self.f16(m)? as u32, _ => self.f32(m)? })
    }

    fn reg8(&self, i: usize) -> u32 { if i < 4 { self.r[i] & 0xff } else { (self.r[i - 4] >> 8) & 0xff } }
    fn set_reg8(&mut self, i: usize, v: u32) {
        let v = v & 0xff;
        if i < 4 { self.r[i] = (self.r[i] & !0xff) | v } else { self.r[i - 4] = (self.r[i - 4] & !0xff00) | (v << 8) }
    }
    fn getr(&self, i: usize, w: u32) -> u32 {
        match w { 8 => self.reg8(i), 16 => self.r[i] & 0xffff, _ => self.r[i] }
    }
    fn setr(&mut self, i: usize, w: u32, v: u32) {
        match w { 8 => self.set_reg8(i, v), 16 => self.r[i] = (self.r[i] & 0xffff_0000) | (v & 0xffff), _ => self.r[i] = v }
    }

    fn rd(&self, m: &Mem, o: Op, w: u32) -> R<u32> {
        Ok(match o {
            Op::R(i) => self.getr(i, w),
            Op::M(a) => match w { 8 => m.r8(a)? as u32, 16 => m.r16(a)? as u32, _ => m.r32(a)? },
        })
    }
    fn wr(&mut self, m: &mut Mem, o: Op, w: u32, v: u32) -> R<()> {
        match o {
            Op::R(i) => self.setr(i, w, v),
            Op::M(a) => match w { 8 => m.w8(a, v as u8)?, 16 => m.w16(a, v as u16)?, _ => m.w32(a, v)? },
        }
        Ok(())
    }

    /// Returns (reg field, r/m operand). Memory operands include the active segment base.
    fn modrm(&mut self, m: &Mem) -> R<(usize, Op)> {
        let b = self.f8(m)?;
        let (md, reg, rm) = (b >> 6, ((b >> 3) & 7) as usize, (b & 7) as usize);
        if md == 3 { return Ok((reg, Op::R(rm))); }
        if self.addr16 {
            // 16-bit addressing: [bx+si] [bx+di] [bp+si] [bp+di] [si] [di] [bp]/disp16 [bx], wrapped to 16 bits
            let g = |i: usize| self.r[i] & 0xffff;
            let mut a = match rm {
                0 => g(EBX) + g(ESI), 1 => g(EBX) + g(EDI), 2 => g(EBP) + g(ESI), 3 => g(EBP) + g(EDI),
                4 => g(ESI), 5 => g(EDI), 6 => if md == 0 { self.f16(m)? as u32 } else { g(EBP) }, _ => g(EBX),
            };
            match md {
                1 => a = a.wrapping_add(self.f8(m)? as i8 as i16 as u16 as u32),
                2 => a = a.wrapping_add(self.f16(m)? as u32),
                _ => {}
            }
            return Ok((reg, Op::M((a & 0xffff).wrapping_add(self.seg))));
        }
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
        Ok((reg, Op::M(a.wrapping_add(self.seg))))
    }

    fn setf(&mut self, f: u32, on: bool) { if on { self.eflags |= f } else { self.eflags &= !f } }
    fn szp(&mut self, r: u32, w: u32) {
        self.setf(ZF, r == 0);
        self.setf(SF, r & (1 << (w - 1)) != 0);
        self.setf(PF, (r as u8).count_ones() % 2 == 0);
    }

    /// op: 0 add,1 or,2 adc,3 sbb,4 and,5 sub,6 xor,7 cmp. Returns result; sets flags.
    fn alu(&mut self, op: u8, a: u32, b: u32, w: u32) -> u32 {
        let mask = maskw(w) as u64;
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
        let r = (r & mask) as u32;
        self.setf(CF, cf);
        self.setf(OF, of);
        self.szp(r, w);
        self.setf(AF, (a ^ b ^ r as u64) & 0x10 != 0);
        r
    }

    fn shift(&mut self, sub: usize, a: u32, cnt: u32, w: u32) -> u32 {
        let c = cnt & 0x1f;
        if c == 0 { return a; }
        let mask = maskw(w) as u64;
        let a64 = a as u64 & mask;
        let msb = |v: u64| (v >> (w - 1)) & 1 != 0;
        let (r, cf, of) = match sub {
            0 => {
                let k = c % w;
                let r = if k == 0 { a64 } else { ((a64 << k) | (a64 >> (w - k))) & mask };
                let cf = r & 1 != 0;
                (r, cf, msb(r) ^ cf)
            }
            1 => {
                let k = c % w;
                let r = if k == 0 { a64 } else { ((a64 >> k) | (a64 << (w - k))) & mask };
                (r, msb(r), msb(r) ^ ((r >> (w - 2)) & 1 != 0))
            }
            2 => {
                let (mut v, mut cf) = (a64, self.eflags & CF != 0);
                for _ in 0..c { let b = msb(v); v = ((v << 1) | cf as u64) & mask; cf = b; }
                (v, cf, msb(v) ^ cf)
            }
            3 => {
                let (mut v, mut cf) = (a64, self.eflags & CF != 0);
                for _ in 0..c { let b = v & 1 != 0; v = (v >> 1) | ((cf as u64) << (w - 1)); cf = b; }
                (v, cf, msb(v) ^ ((v >> (w - 2)) & 1 != 0))
            }
            4 | 6 => {
                if c > w { (0, false, false) } else {
                    let r = (a64 << c) & mask;
                    let cf = (a64 >> (w - c)) & 1 != 0;
                    (r, cf, msb(r) ^ cf)
                }
            }
            5 => ((a64 >> c), (a64 >> (c - 1)) & 1 != 0, msb(a64)),
            _ => {
                let s = sx(a, w);
                (((s >> c.min(63)) as u64) & mask, (s >> (c - 1).min(63)) & 1 != 0, false)
            }
        };
        let r = r as u32;
        self.setf(CF, cf);
        self.setf(OF, of);
        if sub >= 4 { self.szp(r, w); }
        r
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

    /// mul/imul/div/idiv on the accumulator. sub: 4 mul,5 imul,6 div,7 idiv.
    fn muldiv(&mut self, sub: usize, v: u32, w: u32, start: u32) -> R<()> {
        let mask = maskw(w) as u64;
        let acc = self.getr(EAX, w) as u64;
        match sub {
            4 | 5 => {
                let (p, fits) = if sub == 4 {
                    let p = acc * (v as u64 & mask);
                    (p, p >> w == 0)
                } else {
                    let p = sx(acc as u32, w).wrapping_mul(sx(v, w));
                    (p as u64, p == sx((p as u64 & mask) as u32, w))
                };
                if w == 8 { self.setr(EAX, 16, p as u32) } else {
                    self.setr(EAX, w, p as u32);
                    self.setr(EDX, w, (p >> w) as u32);
                }
                self.setf(CF, !fits);
                self.setf(OF, !fits);
            }
            _ => {
                let d = v as u64 & mask;
                if d == 0 { return Err(Exit::DivideError(start)); }
                let dividend: u64 = match w { 8 => self.getr(EAX, 16) as u64, 16 => ((self.getr(EDX, 16) as u64) << 16) | acc, _ => ((self.r[EDX] as u64) << 32) | acc };
                let (q, rem) = if sub == 6 {
                    (dividend / d, dividend % d)
                } else {
                    let dv = match w { 8 => dividend as u16 as i16 as i64, 16 => dividend as u32 as i32 as i64, _ => dividend as i64 };
                    let ds = sx(v, w);
                    let (q, r) = (dv.wrapping_div(ds), dv.wrapping_rem(ds));
                    let lim = 1i64 << (w - 1);
                    if q >= lim || q < -lim { return Err(Exit::DivideError(start)); }
                    (q as u64 & mask, r as u64 & mask)
                };
                if q > mask { return Err(Exit::DivideError(start)); }
                match w {
                    8 => { self.set_reg8(0, q as u32); self.set_reg8(4, rem as u32); }
                    _ => { self.setr(EAX, w, q as u32); self.setr(EDX, w, rem as u32); }
                }
            }
        }
        Ok(())
    }

    /// One string-op iteration. Returns true if the op sets ZF from a compare.
    fn strop(&mut self, m: &mut Mem, op: u8, w: u32) -> R<bool> {
        let n = w / 8;
        let d = if self.eflags & DF != 0 { 0u32.wrapping_sub(n) } else { n };
        let (si, di) = (self.r[ESI].wrapping_add(self.seg), self.r[EDI]);
        let mut cmp = false;
        match op {
            0xA4 | 0xA5 => { let v = self.rd(m, Op::M(si), w)?; self.wr(m, Op::M(di), w, v)?; self.r[ESI] = self.r[ESI].wrapping_add(d); self.r[EDI] = di.wrapping_add(d); }
            0xA6 | 0xA7 => { let (a, b) = (self.rd(m, Op::M(si), w)?, self.rd(m, Op::M(di), w)?); self.alu(7, a, b, w); self.r[ESI] = self.r[ESI].wrapping_add(d); self.r[EDI] = di.wrapping_add(d); cmp = true; }
            0xAA | 0xAB => { let v = self.getr(EAX, w); self.wr(m, Op::M(di), w, v)?; self.r[EDI] = di.wrapping_add(d); }
            0xAC | 0xAD => { let v = self.rd(m, Op::M(si), w)?; self.setr(EAX, w, v); self.r[ESI] = self.r[ESI].wrapping_add(d); }
            _ => { let (a, b) = (self.getr(EAX, w), self.rd(m, Op::M(di), w)?); self.alu(7, a, b, w); self.r[EDI] = di.wrapping_add(d); cmp = true; }
        }
        Ok(cmp)
    }

    #[inline(never)]
    pub fn step(&mut self, m: &mut Mem) -> R<()> {
        let start = self.eip;
        self.insn_start = start;
        if start >= self.thunk.0 && start < self.thunk.1 { return Err(Exit::Thunk(start)); }
        self.seg = 0;
        self.ow = 32;
        self.addr16 = false;
        if self.debug_ring { if self.ring.len() < 64 { self.ring.push(start); } else { let i = self.ring_pos & 63; self.ring[i] = start; } self.ring_pos += 1; }
        let mut rep = 0u8;
        let op = loop {
            let b = self.f8(m)?;
            match b {
                0x66 => self.ow = 16,
                0x64 => self.seg = self.fs_base,
                0x65 => self.seg = self.gs_base,
                0xF2 | 0xF3 => rep = b,
                0x67 => self.addr16 = true,
                0xF0 | 0x2E | 0x36 | 0x3E | 0x26 => {}
                _ => break b,
            }
        };
        let vw = self.ow;
        let unimpl = |o: u8| Err(Exit::Unimplemented { eip: start, opcode: o });
        match op {
            // ALU: add/or/adc/sbb/and/sub/xor/cmp, forms 0..5
            0x00..=0x3F if (op & 7) < 6 => {
                let (alu, form) = (op >> 3, op & 7);
                let w = if form & 1 == 0 { 8 } else { vw };
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
                        let imm = self.imm(m, w)?;
                        let a = self.getr(EAX, w);
                        let r = self.alu(alu, a, imm, w);
                        if alu != 7 { self.setr(EAX, w, r); }
                    }
                }
            }
            0x40..=0x4F => {
                let i = (op & 7) as usize;
                let cf = self.eflags & CF;
                let a = self.getr(i, vw);
                let r = self.alu(if op < 0x48 { 0 } else { 5 }, a, 1, vw);
                self.setr(i, vw, r);
                self.eflags = (self.eflags & !CF) | cf;
            }
            0x50..=0x57 => { let v = self.r[(op & 7) as usize]; self.push(m, v)?; }
            0x58..=0x5F => { let v = self.pop(m)?; self.r[(op & 7) as usize] = v; }
            0x60 => {
                let sp = self.r[ESP];
                for i in [EAX, ECX, EDX, EBX] { let v = self.r[i]; self.push(m, v)?; }
                self.push(m, sp)?;
                for i in [EBP, ESI, EDI] { let v = self.r[i]; self.push(m, v)?; }
            }
            0x61 => {
                for i in [EDI, ESI, EBP] { self.r[i] = self.pop(m)?; }
                self.pop(m)?;
                for i in [EBX, EDX, ECX, EAX] { self.r[i] = self.pop(m)?; }
            }
            0x68 => { let v = self.f32(m)?; self.push(m, v)?; }
            0x6A => { let v = self.f8(m)? as i8 as i32 as u32; self.push(m, v)?; }
            0x69 | 0x6B => {
                let (reg, rm) = self.modrm(m)?;
                let imm = if op == 0x69 { self.imm(m, vw)? } else { self.f8(m)? as i8 as i32 as u32 };
                let a = self.rd(m, rm, vw)?;
                let p = sx(a, vw).wrapping_mul(sx(imm, vw));
                let fits = p == sx((p as u64 & maskw(vw) as u64) as u32, vw);
                self.setr(reg, vw, p as u32);
                self.setf(CF, !fits);
                self.setf(OF, !fits);
            }
            0x70..=0x7F => {
                let d = self.f8(m)? as i8 as i32 as u32;
                if self.cond(op & 0xF) { self.eip = self.eip.wrapping_add(d); }
            }
            0x80 | 0x81 | 0x83 => {
                let w = if op == 0x80 { 8 } else { vw };
                let (alu, rm) = self.modrm(m)?;
                let imm = match op { 0x81 => self.imm(m, vw)?, 0x83 => self.f8(m)? as i8 as i32 as u32, _ => self.f8(m)? as u32 };
                let a = self.rd(m, rm, w)?;
                let r = self.alu(alu as u8, a, imm, w);
                if alu != 7 { self.wr(m, rm, w, r)?; }
            }
            0x84 | 0x85 => {
                let w = if op == 0x84 { 8 } else { vw };
                let (reg, rm) = self.modrm(m)?;
                let (a, b) = (self.rd(m, rm, w)?, self.getr(reg, w));
                self.alu(4, a, b, w);
            }
            0x86 | 0x87 => {
                let w = if op == 0x86 { 8 } else { vw };
                let (reg, rm) = self.modrm(m)?;
                let (a, b) = (self.rd(m, rm, w)?, self.getr(reg, w));
                self.wr(m, rm, w, b)?;
                self.setr(reg, w, a);
            }
            0x88 | 0x89 => {
                let w = if op == 0x88 { 8 } else { vw };
                let (reg, rm) = self.modrm(m)?;
                let v = self.getr(reg, w);
                self.wr(m, rm, w, v)?;
            }
            0x8A | 0x8B => {
                let w = if op == 0x8A { 8 } else { vw };
                let (reg, rm) = self.modrm(m)?;
                let v = self.rd(m, rm, w)?;
                self.setr(reg, w, v);
            }
            0x8C => {
                let (reg, rm) = self.modrm(m)?;
                let mut sel = [0x2B, 0x23, 0x2B, 0x2B, 0x53, 0x2B, 0, 0][reg];
                if self.linux { sel = match reg { 1 => 0x73, 4 => self.sel_fs, 5 => self.sel_gs, _ => 0x7B }; }
                self.wr(m, rm, if matches!(rm, Op::R(_)) { vw } else { 16 }, sel)?;
            }
            0x8E => {
                let (reg, rm) = self.modrm(m)?;
                if self.linux && (reg == 4 || reg == 5) {
                    let sel = self.rd(m, rm, 16)? & 0xFFFF;
                    let base = match sel >> 3 { i @ 6..=8 if sel & 4 == 0 => self.tls[(i - 6) as usize], _ => 0 };
                    if reg == 4 { self.sel_fs = sel; self.fs_base = base; } else { self.sel_gs = sel; self.gs_base = base; }
                }
            }
            0x8D => {
                self.seg = 0;
                match self.modrm(m)? {
                    (reg, Op::M(a)) => self.setr(reg, vw, a),
                    _ => return Err(Exit::Undefined(start)),
                }
            }
            0x8F => { let v = self.pop(m)?; let (_, rm) = self.modrm(m)?; self.wr(m, rm, 32, v)?; }
            0x90 => {}
            0x91..=0x97 => { let i = (op & 7) as usize; let (a, b) = (self.getr(EAX, vw), self.getr(i, vw)); self.setr(EAX, vw, b); self.setr(i, vw, a); }
            0x98 => if vw == 32 { self.r[EAX] = self.r[EAX] as u16 as i16 as i32 as u32 } else { let v = self.reg8(0) as u8 as i8 as i32 as u32; self.setr(EAX, 16, v) },
            0x99 => if vw == 32 { self.r[EDX] = ((self.r[EAX] as i32) >> 31) as u32 } else { let v = ((self.r[EAX] as u16 as i16) >> 15) as u32; self.setr(EDX, 16, v) },
            0x9B => {}
            0x9C => { let v = self.eflags & FLAGS_MASK | 2; self.push(m, v)?; }
            0x9D => { let v = self.pop(m)?; self.eflags = (v & FLAGS_MASK) | 2; }
            0x9E => { let ah = self.reg8(4); self.eflags = (self.eflags & !(SF | ZF | AF | PF | CF)) | (ah & (SF | ZF | AF | PF | CF)); }
            0x9F => { let v = (self.eflags & (SF | ZF | AF | PF | CF)) | 2; self.set_reg8(4, v); }
            0xA0..=0xA3 => {
                // moffs is 16 bits wide under the 0x67 address-size prefix (MSVC's `mov fs:[0], eax` SEH frames use it)
                let off = if self.addr16 { self.f16(m)? as u32 } else { self.f32(m)? };
                let a = off.wrapping_add(self.seg);
                let w = if op & 1 == 0 { 8 } else { vw };
                if op < 0xA2 { let v = self.rd(m, Op::M(a), w)?; self.setr(EAX, w, v); } else { let v = self.getr(EAX, w); self.wr(m, Op::M(a), w, v)?; }
            }
            0xA4..=0xA7 | 0xAA..=0xAF => {
                let w = if op & 1 == 0 { 8 } else { vw };
                if rep != 0 && matches!(op, 0xA4 | 0xA5 | 0xAA | 0xAB) && (self.r[ECX] as u64) * (w as u64 / 8) > 0x1000_0000 {
                    return Err(Exit::Fault(self.r[EDI])); // a >256MB string op is a bug in the guest; fault instead of grinding through it
                }
                if rep == 0 { self.strop(m, op, w)?; } else {
                    while self.r[ECX] != 0 {
                        let cmp = self.strop(m, op, w)?;
                        self.r[ECX] -= 1;
                        if cmp && ((rep == 0xF3) != (self.eflags & ZF != 0)) { break; }
                    }
                }
            }
            0xA8 | 0xA9 => { let w = if op == 0xA8 { 8 } else { vw }; let i = self.imm(m, w)?; let a = self.getr(EAX, w); self.alu(4, a, i, w); }
            0xB0..=0xB7 => { let v = self.f8(m)? as u32; self.set_reg8((op & 7) as usize, v); }
            0xB8..=0xBF => { let v = self.imm(m, vw)?; self.setr((op & 7) as usize, vw, v); }
            0xC0 | 0xC1 | 0xD0..=0xD3 => {
                let w = if op & 1 == 0 { 8 } else { vw };
                let (sub, rm) = self.modrm(m)?;
                let cnt = match op { 0xC0 | 0xC1 => self.f8(m)? as u32, 0xD0 | 0xD1 => 1, _ => self.r[ECX] & 0xff };
                let a = self.rd(m, rm, w)?;
                let r = self.shift(sub, a, cnt, w);
                self.wr(m, rm, w, r)?;
            }
            0xC2 => { let n = self.f16(m)? as u32; self.eip = self.pop(m)?; self.r[ESP] = self.r[ESP].wrapping_add(n); }
            0xC3 => self.eip = self.pop(m)?,
            0xC6 | 0xC7 => {
                let w = if op == 0xC6 { 8 } else { vw };
                let (_, rm) = self.modrm(m)?;
                let v = self.imm(m, w)?;
                self.wr(m, rm, w, v)?;
            }
            0xC9 => { self.r[ESP] = self.r[EBP]; self.r[EBP] = self.pop(m)?; }
            0xCC => return Err(Exit::Interrupt(3)),
            0xCD => { let n = self.f8(m)?; return Err(Exit::Interrupt(n)); }
            0xD7 => { let a = self.r[EBX].wrapping_add(self.reg8(0)).wrapping_add(self.seg); let v = m.r8(a)? as u32; self.set_reg8(0, v); }
            0xD8..=0xDF => self.fpu(m, op, start)?,
            0xE0..=0xE3 => {
                let d = self.f8(m)? as i8 as i32 as u32;
                let take = if op == 0xE3 { self.r[ECX] == 0 } else {
                    self.r[ECX] = self.r[ECX].wrapping_sub(1);
                    self.r[ECX] != 0 && match op { 0xE0 => self.eflags & ZF == 0, 0xE1 => self.eflags & ZF != 0, _ => true }
                };
                if take { self.eip = self.eip.wrapping_add(d); }
            }
            0xE8 => { let d = self.f32(m)?; let ret = self.eip; self.push(m, ret)?; self.eip = self.eip.wrapping_add(d); }
            0xE9 => { let d = self.f32(m)?; self.eip = self.eip.wrapping_add(d); }
            0xEB => { let d = self.f8(m)? as i8 as i32 as u32; self.eip = self.eip.wrapping_add(d); }
            0xF4 => return Err(Exit::Halt),
            0xF5 => self.eflags ^= CF,
            0xF6 | 0xF7 => {
                let w = if op == 0xF6 { 8 } else { vw };
                let (sub, rm) = self.modrm(m)?;
                match sub {
                    0 | 1 => { let i = self.imm(m, w)?; let a = self.rd(m, rm, w)?; self.alu(4, a, i, w); }
                    2 => { let a = self.rd(m, rm, w)?; self.wr(m, rm, w, !a)?; }
                    3 => { let a = self.rd(m, rm, w)?; let r = self.alu(5, 0, a, w); self.wr(m, rm, w, r)?; self.setf(CF, a != 0); }
                    _ => { let v = self.rd(m, rm, w)?; self.muldiv(sub, v, w, start)?; }
                }
            }
            0xF8 => self.setf(CF, false),
            0xF9 => self.setf(CF, true),
            0xFA | 0xFB => {}
            0xFC => self.setf(DF, false),
            0xFD => self.setf(DF, true),
            0xFE => {
                let (sub, rm) = self.modrm(m)?;
                if sub > 1 { return Err(Exit::Undefined(start)); }
                let cf = self.eflags & CF;
                let a = self.rd(m, rm, 8)?;
                let r = self.alu(if sub == 0 { 0 } else { 5 }, a, 1, 8);
                self.wr(m, rm, 8, r)?;
                self.eflags = (self.eflags & !CF) | cf;
            }
            0xFF => {
                let (sub, rm) = self.modrm(m)?;
                match sub {
                    0 | 1 => {
                        let cf = self.eflags & CF;
                        let a = self.rd(m, rm, vw)?;
                        let r = self.alu(if sub == 0 { 0 } else { 5 }, a, 1, vw);
                        self.wr(m, rm, vw, r)?;
                        self.eflags = (self.eflags & !CF) | cf;
                    }
                    2 => { let t = self.rd(m, rm, 32)?; let ret = self.eip; self.push(m, ret)?; self.eip = t; }
                    4 => self.eip = self.rd(m, rm, 32)?,
                    6 => { let v = self.rd(m, rm, 32)?; self.push(m, v)?; }
                    _ => return Err(Exit::Undefined(start)),
                }
            }
            0x0F => {
                let op2 = self.f8(m)?;
                match op2 {
                    0x0B => return Err(Exit::Undefined(start)),
                    0x0D | 0x18 | 0x1E | 0x1F => { self.modrm(m)?; }
                    0x31 => { self.tsc += 1000; self.r[EAX] = self.tsc as u32; self.r[EDX] = (self.tsc >> 32) as u32; }
                    0xA2 => {
                        let leaf = self.r[EAX];
                        let (a, b, c, d) = match leaf {
                            0 => (1, 0x756e_6547, 0x6c65_746e, 0x4965_6e69), // "GenuineIntel"
                            1 => (0x0000_0633, 0, 0, 0x0700_8111),            // FPU TSC CX8 CMOV FXSR SSE SSE2
                            _ => (0, 0, 0, 0),
                        };
                        self.r[EAX] = a; self.r[EBX] = b; self.r[ECX] = c; self.r[EDX] = d;
                    }
                    0x40..=0x4F => {
                        let (reg, rm) = self.modrm(m)?;
                        let v = self.rd(m, rm, vw)?;
                        if self.cond(op2 & 0xF) { self.setr(reg, vw, v); }
                    }
                    0x80..=0x8F => {
                        let d = self.f32(m)?;
                        if self.cond(op2 & 0xF) { self.eip = self.eip.wrapping_add(d); }
                    }
                    0x90..=0x9F => {
                        let (_, rm) = self.modrm(m)?;
                        let v = self.cond(op2 & 0xF) as u32;
                        self.wr(m, rm, 8, v)?;
                    }
                    0xA3 | 0xAB | 0xB3 | 0xBB | 0xBA => {
                        let (reg, rm) = self.modrm(m)?;
                        let (kind, off) = if op2 == 0xBA { (reg, self.f8(m)? as u32) } else { (((op2 >> 3) & 3) as usize + 4, self.r[reg]) };
                        let (rm, bit) = match rm {
                            Op::M(a) if op2 != 0xBA => (Op::M(a.wrapping_add(((off as i32) >> 5) as u32 * 4)), off & 31),
                            _ => (rm, off & 31),
                        };
                        let v = self.rd(m, rm, 32)?;
                        self.setf(CF, v >> bit & 1 != 0);
                        // kind: 4 bt, 5 bts, 6 btr, 7 btc ; for 0xA3/AB/B3/BB: (op2>>3)&3 = 0,1,2,3 -> 4..7
                        let nv = match kind { 5 => v | 1 << bit, 6 => v & !(1 << bit), 7 => v ^ 1 << bit, _ => v };
                        if kind != 4 { self.wr(m, rm, 32, nv)?; }
                    }
                    0xA4 | 0xA5 | 0xAC | 0xAD => {
                        let (reg, rm) = self.modrm(m)?;
                        let c = (if op2 & 1 == 0 { self.f8(m)? as u32 } else { self.r[ECX] }) & 0x1f;
                        if c != 0 {
                            let (d, s) = (self.rd(m, rm, vw)? as u64, self.getr(reg, vw) as u64);
                            let mask = maskw(vw) as u64;
                            let (r, cf) = if op2 < 0xA8 {
                                (((d << c) | (s >> (vw - c))) & mask, (d >> (vw - c)) & 1 != 0)
                            } else {
                                (((d >> c) | (s << (vw - c))) & mask, (d >> (c - 1)) & 1 != 0)
                            };
                            self.setf(CF, cf);
                            self.szp(r as u32, vw);
                            self.wr(m, rm, vw, r as u32)?;
                        }
                    }
                    0xAE => {
                        let b = m.r8(self.eip)?;
                        if b >> 6 != 3 { return self.sse(m, op2, start, rep); }
                        self.eip += 1; // fences
                    }
                    0xAF => {
                        let (reg, rm) = self.modrm(m)?;
                        let (a, b) = (self.getr(reg, vw), self.rd(m, rm, vw)?);
                        let p = sx(a, vw).wrapping_mul(sx(b, vw));
                        let fits = p == sx((p as u64 & maskw(vw) as u64) as u32, vw);
                        self.setr(reg, vw, p as u32);
                        self.setf(CF, !fits);
                        self.setf(OF, !fits);
                    }
                    0xB0 | 0xB1 => {
                        let w = if op2 == 0xB0 { 8 } else { vw };
                        let (reg, rm) = self.modrm(m)?;
                        let (d, acc) = (self.rd(m, rm, w)?, self.getr(EAX, w));
                        self.alu(7, acc, d, w);
                        if acc == d { let s = self.getr(reg, w); self.wr(m, rm, w, s)?; } else { self.setr(EAX, w, d); }
                    }
                    0xC0 | 0xC1 => {
                        let w = if op2 == 0xC0 { 8 } else { vw };
                        let (reg, rm) = self.modrm(m)?;
                        let (d, s) = (self.rd(m, rm, w)?, self.getr(reg, w));
                        let r = self.alu(0, d, s, w);
                        self.setr(reg, w, d);
                        self.wr(m, rm, w, r)?;
                    }
                    0xB6 | 0xB7 | 0xBE | 0xBF => {
                        let (reg, rm) = self.modrm(m)?;
                        let sw = if op2 & 1 != 0 { 16 } else { 8 };
                        let v = self.rd(m, rm, sw)?;
                        let v = if op2 >= 0xBE { sx(v, sw) as u32 } else { v };
                        self.setr(reg, vw, v);
                    }
                    0xBC | 0xBD => {
                        let (reg, rm) = self.modrm(m)?;
                        let v = self.rd(m, rm, vw)?;
                        if v == 0 { self.setf(ZF, true); } else {
                            self.setf(ZF, false);
                            let i = if op2 == 0xBC { v.trailing_zeros() } else { 31 - v.leading_zeros() };
                            self.setr(reg, vw, i);
                        }
                    }
                    0xC7 => { // cmpxchg8b m64
                        let (sub, rm) = self.modrm(m)?;
                        let Op::M(a) = rm else { return unimpl(op2) };
                        if sub != 1 { return unimpl(op2); }
                        let cur = m.r32(a)? as u64 | (m.r32(a + 4)? as u64) << 32;
                        let want = (self.r[EDX] as u64) << 32 | self.r[EAX] as u64;
                        if cur == want {
                            m.w32(a, self.r[EBX])?; m.w32(a + 4, self.r[ECX])?;
                            self.setf(ZF, true);
                        } else {
                            self.r[EAX] = cur as u32; self.r[EDX] = (cur >> 32) as u32;
                            self.setf(ZF, false);
                        }
                    }
                    0xC8..=0xCF => { let i = (op2 & 7) as usize; self.r[i] = self.r[i].swap_bytes(); }
                    _ => return self.sse(m, op2, start, rep),
                }
            }
            _ => return unimpl(op),
        }
        Ok(())
    }
}
