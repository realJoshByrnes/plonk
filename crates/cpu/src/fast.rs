//! Predecoded basic-block cache + tight execution loop.
//!
//! Instructions are decoded once into compact `Insn`s (operands/addressing resolved) and grouped into
//! blocks keyed by start eip. Anything the predecoder does not handle natively is either `Step`
//! (length known, executed by the old byte-wise interpreter `Cpu::step`, block continues) or `Slow`
//! (length unknown, executed by `step`, block ends). The old interpreter is also the fault path: a
//! fast handler that faults has not modified any state, so the instruction is simply re-run through
//! `step`, which reproduces the exact architectural state / exit the old core left.
//!
//! Code-change safety: guest stores go through `Mem`, which flags pages holding cached code and sets
//! `Mem::smc`; the loop checks that after every instruction and flushes. Host-side writes (raw
//! pointers, `write_bytes` between runs) are covered by re-validating each block's bytes against
//! memory the first time it is used in every `run` call (`epoch`).
use crate::*;
use plonk_mem::Fault;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum H {
    Slow, Step, Nop,
    AluRmR, AluRRm, AluAI, AluRmI, Test, TestAI, TestRmI,
    IncR, DecR, IncDecRm,
    PushR, PopR, PushI, PushRm,
    MovRmR, MovRRm, MovRI, MovRmI, Lea, Xchg, XchgA, Movzx, Movsx, Imul2, Imul3,
    Shift, Not, Neg,
    Jcc, Jmp, Call, CallRm, JmpRm, Ret, RetN, Leave,
    Setcc, Cmov, Cwde, Cdq, Fpu,
}

#[derive(Clone, Copy)]
pub(crate) struct Insn {
    h: H,
    len: u8,
    w: u8,
    m: bool,
    reg: u8,
    rm: u8,
    aux: u8,
    base: u8,
    idx: u8,
    sc: u8,
    seg: u8,
    bm: u32,
    im: u32,
    disp: u32,
    imm: u32,
}

impl Insn {
    fn slow() -> Insn {
        Insn { h: H::Slow, len: 0, w: 32, m: false, reg: 0, rm: 0, aux: 0, base: 0, idx: 0, sc: 0, seg: 0, bm: 0, im: 0, disp: 0, imm: 0 }
    }
}

fn rd32(b: &[u8], p: &mut usize) -> Option<u32> {
    let s = b.get(*p..*p + 4)?;
    *p += 4;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn rd8(b: &[u8], p: &mut usize) -> Option<u32> { let v = *b.get(*p)?; *p += 1; Some(v as u32) }
fn rds8(b: &[u8], p: &mut usize) -> Option<u32> { Some(rd8(b, p)? as u8 as i8 as i32 as u32) }
fn rd16(b: &[u8], p: &mut usize) -> Option<u32> { let l = rd8(b, p)?; Some(l | rd8(b, p)? << 8) }
fn rdimm(b: &[u8], p: &mut usize, w: u32) -> Option<u32> { match w { 8 => rd8(b, p), 16 => rd16(b, p), _ => rd32(b, p) } }

/// Parse modrm (+sib/disp) at b[*p..]; fills the operand fields of `i`. Returns the reg field.
fn modrm(b: &[u8], p: &mut usize, i: &mut Insn) -> Option<u8> {
    let x = *b.get(*p)?;
    *p += 1;
    let (md, reg, rm) = (x >> 6, (x >> 3) & 7, x & 7);
    if md == 3 { i.m = false; i.rm = rm; return Some(reg); }
    i.m = true;
    if rm == 4 {
        let s = *b.get(*p)?;
        *p += 1;
        let (sc, ix, bs) = (s >> 6, (s >> 3) & 7, s & 7);
        if ix != 4 { i.idx = ix; i.im = !0; i.sc = sc; }
        if bs == 5 && md == 0 { i.disp = rd32(b, p)?; } else { i.base = bs; i.bm = !0; }
    } else if rm == 5 && md == 0 {
        i.disp = rd32(b, p)?;
    } else {
        i.base = rm; i.bm = !0;
    }
    match md {
        1 => i.disp = i.disp.wrapping_add(rds8(b, p)?),
        2 => i.disp = i.disp.wrapping_add(rd32(b, p)?),
        _ => {}
    }
    Some(reg)
}

/// Number of bytes up to and including the opcode byte (prefixes + opcode) of the instruction at b.
fn seg_end(b: &[u8], _pc: u32) -> u8 {
    let mut p = 0;
    while matches!(b[p], 0x66 | 0x64 | 0x65 | 0xF2 | 0xF3 | 0xF0 | 0x2E | 0x36 | 0x3E | 0x26) { p += 1; }
    (p + 1) as u8
}

fn decode(b: &[u8], pc: u32) -> Insn { decode_opt(b, pc).unwrap_or_else(Insn::slow) }

fn decode_opt(b: &[u8], pc: u32) -> Option<Insn> {
    let mut i = Insn::slow();
    let mut p = 0usize;
    let (mut ow, mut seg, mut rep) = (32u32, 0u8, false);
    loop {
        match *b.get(p)? {
            0x66 => ow = 16,
            0x64 => seg = 1,
            0x65 => seg = 2,
            0xF2 | 0xF3 => rep = true,
            0xF0 | 0x2E | 0x36 | 0x3E | 0x26 => {}
            _ => break,
        }
        p += 1;
        if p > 10 { return None; }
    }
    let op = b[p];
    p += 1;
    i.seg = seg;
    i.w = ow as u8;
    let bb = b;
    macro_rules! mr { () => { modrm(bb, &mut p, &mut i)? } }
    macro_rules! step_modrm { ($imm:expr) => {{ i.h = H::Step; modrm(bb, &mut p, &mut i)?; if $imm { rd8(bb, &mut p)?; } }} }
    match op {
        0x00..=0x3F if (op & 7) < 6 => {
            let (alu, form) = (op >> 3, op & 7);
            let w = if form & 1 == 0 { 8 } else { ow };
            i.w = w as u8;
            i.aux = alu;
            match form {
                0 | 1 => { i.reg = mr!(); i.h = H::AluRmR; }
                2 | 3 => { i.reg = mr!(); i.h = H::AluRRm; }
                _ => { i.imm = rdimm(b, &mut p, w)?; i.h = H::AluAI; }
            }
        }
        0x40..=0x4F => { i.h = if op < 0x48 { H::IncR } else { H::DecR }; i.rm = op & 7; }
        0x50..=0x57 => { i.h = H::PushR; i.rm = op & 7; }
        0x58..=0x5F => { i.h = H::PopR; i.rm = op & 7; }
        0x68 => { i.h = H::PushI; i.imm = rd32(b, &mut p)?; }
        0x6A => { i.h = H::PushI; i.imm = rds8(b, &mut p)?; }
        0x69 | 0x6B => {
            i.reg = mr!();
            i.imm = if op == 0x69 { rdimm(b, &mut p, ow)? } else { rds8(b, &mut p)? };
            i.h = H::Imul3;
        }
        0x70..=0x7F => {
            let d = rds8(b, &mut p)?;
            i.h = H::Jcc; i.aux = op & 0xF; i.imm = pc.wrapping_add(p as u32).wrapping_add(d);
        }
        0x80 | 0x81 | 0x83 => {
            i.w = if op == 0x80 { 8 } else { ow as u8 };
            i.aux = mr!();
            i.imm = match op { 0x81 => rdimm(b, &mut p, ow)?, 0x83 => rds8(b, &mut p)?, _ => rd8(b, &mut p)? };
            i.h = H::AluRmI;
        }
        0x84 | 0x85 => { i.w = if op == 0x84 { 8 } else { ow as u8 }; i.reg = mr!(); i.h = H::Test; }
        0x86 | 0x87 => { i.w = if op == 0x86 { 8 } else { ow as u8 }; i.reg = mr!(); i.h = H::Xchg; }
        0x88 | 0x89 => { i.w = if op == 0x88 { 8 } else { ow as u8 }; i.reg = mr!(); i.h = H::MovRmR; }
        0x8A | 0x8B => { i.w = if op == 0x8A { 8 } else { ow as u8 }; i.reg = mr!(); i.h = H::MovRRm; }
        0x8D => {
            i.seg = 0;
            i.reg = mr!();
            if !i.m { return None; }
            i.h = H::Lea;
        }
        0x8C | 0x8E | 0x8F => step_modrm!(false),
        0x90 => i.h = H::Nop,
        0x91..=0x97 => { i.h = H::XchgA; i.rm = op & 7; }
        0x98 => i.h = H::Cwde,
        0x99 => i.h = H::Cdq,
        0xA8 | 0xA9 => {
            let w = if op == 0xA8 { 8 } else { ow };
            i.w = w as u8; i.imm = rdimm(b, &mut p, w)?; i.h = H::TestAI;
        }
        0xB0..=0xB7 => { i.h = H::MovRI; i.w = 8; i.rm = op & 7; i.imm = rd8(b, &mut p)?; }
        0xB8..=0xBF => { i.h = H::MovRI; i.rm = op & 7; i.imm = rdimm(b, &mut p, ow)?; }
        0xC0 | 0xC1 | 0xD0..=0xD3 => {
            i.w = if op & 1 == 0 { 8 } else { ow as u8 };
            i.reg = mr!();
            match op { 0xC0 | 0xC1 => { i.aux = 0; i.imm = rd8(b, &mut p)?; } 0xD0 | 0xD1 => i.aux = 1, _ => i.aux = 2 }
            i.h = H::Shift;
        }
        0xC2 => { i.h = H::RetN; i.imm = rd16(b, &mut p)?; }
        0xC3 => i.h = H::Ret,
        0xC6 | 0xC7 => {
            let w = if op == 0xC6 { 8 } else { ow };
            i.w = w as u8;
            mr!();
            i.imm = rdimm(b, &mut p, w)?;
            i.h = H::MovRmI;
        }
        0xC9 => i.h = H::Leave,
        0xE8 => { let d = rd32(b, &mut p)?; i.h = H::Call; i.imm = pc.wrapping_add(p as u32).wrapping_add(d); }
        0xE9 => { let d = rd32(b, &mut p)?; i.h = H::Jmp; i.imm = pc.wrapping_add(p as u32).wrapping_add(d); }
        0xEB => { let d = rds8(b, &mut p)?; i.h = H::Jmp; i.imm = pc.wrapping_add(p as u32).wrapping_add(d); }
        0xF6 | 0xF7 => {
            let w = if op == 0xF6 { 8 } else { ow };
            i.w = w as u8;
            let sub = mr!();
            match sub {
                0 | 1 => { i.imm = rdimm(b, &mut p, w)?; i.h = H::TestRmI; }
                2 => i.h = H::Not,
                3 => i.h = H::Neg,
                _ => i.h = H::Step,
            }
        }
        0xFE => {
            let sub = mr!();
            if sub > 1 { return None; }
            i.w = 8; i.reg = sub; i.h = H::IncDecRm;
        }
        0xFF => {
            let sub = mr!();
            i.reg = sub;
            i.h = match sub { 0 | 1 => H::IncDecRm, 2 => H::CallRm, 4 => H::JmpRm, 6 => H::PushRm, _ => return None };
        }
        0xD8..=0xDF => { step_modrm!(false); i.h = H::Fpu; i.aux = op; i.reg = seg_end(b, pc); }
        0x60 | 0x61 | 0x9B | 0x9C..=0x9F | 0xA4..=0xA7 | 0xAA..=0xAF | 0xD7 | 0xF5 | 0xF8..=0xFD | 0xCC | 0xF4 => i.h = H::Step,
        0xCD | 0xE0..=0xE3 => { rd8(b, &mut p)?; i.h = H::Step; }
        0xA0..=0xA3 => { rd32(b, &mut p)?; i.h = H::Step; }
        0x0F => {
            let op2 = *b.get(p)?;
            p += 1;
            match op2 {
                0x0D | 0x18 | 0x1F => { mr!(); i.h = H::Nop; }
                0x40..=0x4F => { i.reg = mr!(); i.aux = op2 & 0xF; i.h = H::Cmov; }
                0x80..=0x8F => {
                    let d = rd32(b, &mut p)?;
                    i.h = H::Jcc; i.aux = op2 & 0xF; i.imm = pc.wrapping_add(p as u32).wrapping_add(d);
                }
                0x90..=0x9F => { mr!(); i.aux = op2 & 0xF; i.h = H::Setcc; }
                0xAF => { i.reg = mr!(); i.h = H::Imul2; }
                0xB6 | 0xB7 | 0xBE | 0xBF => {
                    i.reg = mr!();
                    i.aux = if op2 & 1 != 0 { 16 } else { 8 };
                    i.h = if op2 >= 0xBE { H::Movsx } else { H::Movzx };
                }
                0x0B | 0x31 | 0xA2 | 0xC8..=0xCF => i.h = H::Step,
                0xA3 | 0xAB | 0xB3 | 0xBB | 0xB0 | 0xB1 | 0xC0 | 0xC1 | 0xBC | 0xBD | 0xC7 | 0xA5 | 0xAD | 0xAE => step_modrm!(false),
                0xBA | 0xA4 | 0xAC => step_modrm!(true),
                0x10..=0x17 | 0x28..=0x2F | 0x50..=0x6F | 0x74..=0x76 | 0x7E | 0x7F | 0xC3 | 0xD0..=0xFE => step_modrm!(false),
                0x70..=0x73 | 0xC2 | 0xC4..=0xC6 => step_modrm!(true),
                _ => return None,
            }
        }
        _ => return None,
    }
    i.len = p as u8;
    // rep/repne prefixes are ignored by the old core except for string ops / SSE: let it handle them all.
    if rep && i.h != H::Step { i.h = H::Step; }
    Some(i)
}

fn is_term(h: H) -> bool {
    matches!(h, H::Jcc | H::Jmp | H::Call | H::CallRm | H::JmpRm | H::Ret | H::RetN)
}

#[derive(Clone, Copy)]
struct Block { pc: u32, epoch: u32, start: u32, n: u32, boff: u32, blen: u32, nx: [(u32, u32); 2] }

const MAP_BITS: u32 = 14;
const MAX_BLOCKS: usize = 1 << 15;
const MAX_INSNS: usize = 1 << 19;
const BLOCK_MAX: usize = 64;
const NONE: u32 = u32::MAX;

pub(crate) struct Cache {
    map: Vec<u32>,
    blocks: Vec<Block>,
    insns: Vec<Insn>,
    bytes: Vec<u8>,
    epoch: u32,
    thunk: (u32, u32),
    pub flushes: u64,
    blocks_run: u64,
    builds: u64,
}

impl Default for Cache {
    fn default() -> Self {
        Cache { map: vec![NONE; 1 << MAP_BITS], blocks: Vec::new(), insns: Vec::new(), bytes: Vec::new(), epoch: 1, thunk: (0, 0), flushes: 0, blocks_run: 0, builds: 0 }
    }
}

#[inline(always)]
fn hash(pc: u32) -> usize { (pc.wrapping_mul(0x9E37_79B1) >> (32 - MAP_BITS)) as usize }

impl Cache {
    fn flush(&mut self) {
        self.map.fill(NONE);
        self.blocks.clear();
        self.insns.clear();
        self.bytes.clear();
        self.flushes += 1;
    }

    #[inline(always)]
    fn lookup(&mut self, mem: &mut Mem, pc: u32) -> usize {
        let h = hash(pc);
        let bi = self.map[h];
        if bi != NONE {
            let b = &mut self.blocks[bi as usize];
            if b.pc == pc {
                if b.epoch == self.epoch { return bi as usize; }
                if self.verify(mem, bi as usize) { return bi as usize; }
            }
        }
        self.build(mem, pc, h)
    }

    #[cold]
    fn verify(&mut self, mem: &mut Mem, bi: usize) -> bool {
        let b = self.blocks[bi];
        let ok = match mem.read_bytes(b.pc, b.blen) {
            Ok(cur) => cur == &self.bytes[b.boff as usize..(b.boff + b.blen) as usize],
            Err(_) => false,
        };
        if ok {
            self.blocks[bi].epoch = self.epoch;
            mem.mark_code(b.pc, b.blen);
        }
        ok
    }

    #[cold]
    fn build(&mut self, mem: &mut Mem, pc: u32, h: usize) -> usize {
        if self.blocks.len() >= MAX_BLOCKS || self.insns.len() >= MAX_INSNS {
            self.flush();
            mem.clear_code();
        }
        self.builds += 1;
        let (start, boff) = (self.insns.len(), self.bytes.len());
        let mut p = pc;
        let mut n = 0;
        let (tlo, thi) = self.thunk;
        while n < BLOCK_MAX {
            if n > 0 && p >= tlo && p < thi { break; }
            let avail = mem.len().saturating_sub(p).min(16);
            let buf: &[u8] = mem.read_bytes(p, avail).unwrap_or(&[]);
            let ins = decode(buf, p);
            n += 1;
            if ins.h == H::Slow { self.insns.push(ins); break; }
            self.bytes.extend_from_slice(&buf[..ins.len as usize]);
            p = p.wrapping_add(ins.len as u32);
            let h = ins.h;
            self.insns.push(ins);
            if is_term(h) { break; }
        }
        let blen = p.wrapping_sub(pc);
        mem.mark_code(pc, blen);
        self.blocks.push(Block { pc, epoch: self.epoch, start: start as u32, n: n as u32, boff: boff as u32, blen, nx: [(NONE, 0); 2] });
        let bi = self.blocks.len() - 1;
        self.map[h] = bi as u32;
        bi
    }
}

#[derive(Default)]
pub struct CacheSlot(pub(crate) Option<Box<Cache>>);
impl Clone for CacheSlot { fn clone(&self) -> Self { CacheSlot(None) } }
impl std::fmt::Debug for CacheSlot { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("CacheSlot") } }

static PARITY: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 { if (i as u8).count_ones() % 2 == 0 { t[i] = PF as u8; } i += 1; }
    t
};

#[inline(always)]
fn ld(mem: &Mem, a: u32, w: u32) -> Result<u32, Fault> {
    match w { 8 => mem.r8(a).map(|v| v as u32), 16 => mem.r16(a).map(|v| v as u32), _ => mem.r32(a) }
}
#[inline(always)]
fn stv(mem: &mut Mem, a: u32, w: u32, v: u32) -> Result<(), Fault> {
    match w { 8 => mem.w8(a, v as u8), 16 => mem.w16(a, v as u16), _ => mem.w32(a, v) }
}

impl Cpu {
    #[inline(always)]
    fn ea(&self, i: &Insn) -> u32 {
        let mut a = i.disp
            .wrapping_add(self.r[i.base as usize & 7] & i.bm)
            .wrapping_add((self.r[i.idx as usize & 7] & i.im) << i.sc);
        if i.seg != 0 { a = a.wrapping_add(if i.seg == 1 { self.fs_base } else { self.gs_base }); }
        a
    }
    #[inline(always)]
    fn rdrm(&self, mem: &Mem, i: &Insn, w: u32) -> Result<u32, Fault> {
        if i.m { ld(mem, self.ea(i), w) } else { Ok(self.getr(i.rm as usize, w)) }
    }
    #[inline(always)]
    fn wrrm(&mut self, mem: &mut Mem, i: &Insn, w: u32, v: u32) -> Result<(), Fault> {
        if i.m { stv(mem, self.ea(i), w, v) } else { self.setr(i.rm as usize, w, v); Ok(()) }
    }

    /// Pure ALU: returns (result, new eflags). Same semantics as `alu` (differentially tested).
    #[inline(always)]
    fn alu_f(&self, op: u8, a: u32, b: u32, w: u32) -> (u32, u32) {
        let mask = maskw(w);
        let (a, b) = (a & mask, b & mask);
        let c = (self.eflags & CF) as u64;
        let (r, cf, of);
        match op {
            0 | 2 => {
                let sum = a as u64 + b as u64 + if op == 2 { c } else { 0 };
                r = sum as u32 & mask;
                cf = (sum >> w) as u32 & 1;
                of = ((a ^ r) & (b ^ r)) >> (w - 1) & 1;
            }
            3 | 5 | 7 => {
                let d = (a as u64).wrapping_sub(b as u64).wrapping_sub(if op == 3 { c } else { 0 });
                r = d as u32 & mask;
                cf = (d >> w) as u32 & 1;
                of = ((a ^ b) & (a ^ r)) >> (w - 1) & 1;
            }
            1 => { r = a | b; cf = 0; of = 0; }
            4 => { r = a & b; cf = 0; of = 0; }
            _ => { r = a ^ b; cf = 0; of = 0; }
        }
        let f = (self.eflags & !(CF | PF | AF | ZF | SF | OF))
            | cf
            | (PARITY[(r & 0xff) as usize] as u32)
            | ((a ^ b ^ r) & AF)
            | (((r == 0) as u32) << 6)
            | ((r >> (w - 1) & 1) << 7)
            | (of << 11);
        (r, f)
    }
    #[inline(always)]
    fn alu_w(&self, op: u8, a: u32, b: u32, w: u32) -> (u32, u32) {
        match w { 32 => self.alu_f(op, a, b, 32), 8 => self.alu_f(op, a, b, 8), _ => self.alu_f(op, a, b, 16) }
    }

    /// (cache flushes, instructions run via the byte-wise interpreter, blocks executed, blocks built); diagnostics.
    pub fn cache_stats(&self) -> (u64, u64, u64, u64) {
        self.cache.0.as_ref().map_or((0, 0, 0, 0), |c| (c.flushes, self.nsteps, c.blocks_run, c.builds))
    }

}

const ST_OK: u8 = 0;
const ST_STOP: u8 = 1;
const ST_FAULT: u8 = 2;
const ST_FLUSH: u8 = 3;

struct BlockOut { done: u32, pc: u32, last: u32, st: u8 }

impl Cpu {
    /// Execute up to `n` predecoded instructions starting at `pc0`.
    #[inline(never)]
    unsafe fn exec(&mut self, mem: &mut Mem, ip: *const Insn, n: usize, pc0: u32) -> BlockOut {
        let (mut pc, mut last, mut done) = (pc0, pc0, 0usize);
        let mut st = ST_OK;
        let mut fault = false;
        {
            'blk: while done < n {
                let i = unsafe { &*ip.add(done) };
                let w = i.w as u32;
                let mut next = pc.wrapping_add(i.len as u32);
                let mut brk = false;
                macro_rules! t { ($e:expr) => { match $e { Ok(v) => v, Err(_) => { fault = true; break 'blk } } } }
                match i.h {
                    H::Nop => {}
                    H::AluRmR => {
                        let (op, reg) = (i.aux, i.reg as usize);
                        let a = t!(self.rdrm(mem, i, w));
                        let (r, f) = self.alu_w(op, a, self.getr(reg, w), w);
                        if op != 7 { t!(self.wrrm(mem, i, w, r)); }
                        self.eflags = f;
                    }
                    H::AluRRm => {
                        let (op, reg) = (i.aux, i.reg as usize);
                        let b = t!(self.rdrm(mem, i, w));
                        let (r, f) = self.alu_w(op, self.getr(reg, w), b, w);
                        if op != 7 { self.setr(reg, w, r); }
                        self.eflags = f;
                    }
                    H::AluAI => {
                        let op = i.aux;
                        let (r, f) = self.alu_w(op, self.getr(EAX, w), i.imm, w);
                        if op != 7 { self.setr(EAX, w, r); }
                        self.eflags = f;
                    }
                    H::AluRmI => {
                        let op = i.aux;
                        let a = t!(self.rdrm(mem, i, w));
                        let (r, f) = self.alu_w(op, a, i.imm, w);
                        if op != 7 { t!(self.wrrm(mem, i, w, r)); }
                        self.eflags = f;
                    }
                    H::Test => {
                        let a = t!(self.rdrm(mem, i, w));
                        self.eflags = self.alu_w(4, a, self.getr(i.reg as usize, w), w).1;
                    }
                    H::TestAI => { self.eflags = self.alu_w(4, self.getr(EAX, w), i.imm, w).1; }
                    H::TestRmI => {
                        let a = t!(self.rdrm(mem, i, w));
                        self.eflags = self.alu_w(4, a, i.imm, w).1;
                    }
                    H::IncR | H::DecR => {
                        let n = i.rm as usize;
                        let (r, f) = self.alu_w(if i.h == H::IncR { 0 } else { 5 }, self.getr(n, w), 1, w);
                        self.setr(n, w, r);
                        self.eflags = (f & !CF) | (self.eflags & CF);
                    }
                    H::IncDecRm => {
                        let a = t!(self.rdrm(mem, i, w));
                        let (r, f) = self.alu_w(if i.reg == 0 { 0 } else { 5 }, a, 1, w);
                        t!(self.wrrm(mem, i, w, r));
                        self.eflags = (f & !CF) | (self.eflags & CF);
                    }
                    H::PushR => {
                        let sp = self.r[ESP].wrapping_sub(4);
                        t!(mem.w32(sp, self.r[i.rm as usize & 7]));
                        self.r[ESP] = sp;
                    }
                    H::PopR => {
                        let v = t!(mem.r32(self.r[ESP]));
                        self.r[ESP] = self.r[ESP].wrapping_add(4);
                        self.r[i.rm as usize & 7] = v;
                    }
                    H::PushI => {
                        let sp = self.r[ESP].wrapping_sub(4);
                        t!(mem.w32(sp, i.imm));
                        self.r[ESP] = sp;
                    }
                    H::PushRm => {
                        let v = t!(self.rdrm(mem, i, 32));
                        let sp = self.r[ESP].wrapping_sub(4);
                        t!(mem.w32(sp, v));
                        self.r[ESP] = sp;
                    }
                    H::MovRmR if w == 32 && !i.m => { self.r[i.rm as usize & 7] = self.r[i.reg as usize & 7]; }
                    H::MovRmR => { let v = self.getr(i.reg as usize, w); t!(self.wrrm(mem, i, w, v)); }
                    H::MovRRm => { let v = t!(self.rdrm(mem, i, w)); self.setr(i.reg as usize, w, v); }
                    H::MovRI => self.setr(i.rm as usize, w, i.imm),
                    H::MovRmI => { t!(self.wrrm(mem, i, w, i.imm)); }
                    H::Lea => { let a = self.ea(i); self.setr(i.reg as usize, w, a); }
                    H::Xchg => {
                        let a = t!(self.rdrm(mem, i, w));
                        let b = self.getr(i.reg as usize, w);
                        t!(self.wrrm(mem, i, w, b));
                        self.setr(i.reg as usize, w, a);
                    }
                    H::XchgA => {
                        let (a, b) = (self.getr(EAX, w), self.getr(i.rm as usize, w));
                        self.setr(EAX, w, b);
                        self.setr(i.rm as usize, w, a);
                    }
                    H::Movzx | H::Movsx => {
                        let sw = i.aux as u32;
                        let v = t!(self.rdrm(mem, i, sw));
                        let v = if i.h == H::Movsx { sx(v, sw) as u32 } else { v };
                        self.setr(i.reg as usize, w, v);
                    }
                    H::Imul2 | H::Imul3 => {
                        let (a, b) = if i.h == H::Imul2 {
                            (self.getr(i.reg as usize, w), t!(self.rdrm(mem, i, w)))
                        } else {
                            (t!(self.rdrm(mem, i, w)), i.imm)
                        };
                        let p = sx(a, w).wrapping_mul(sx(b, w));
                        let fits = p == sx((p as u64 & maskw(w) as u64) as u32, w);
                        self.setr(i.reg as usize, w, p as u32);
                        self.eflags = (self.eflags & !(CF | OF)) | if fits { 0 } else { CF | OF };
                    }
                    H::Shift => {
                        let cnt = match i.aux { 0 => i.imm, 1 => 1, _ => self.r[ECX] & 0xff };
                        let a = t!(self.rdrm(mem, i, w));
                        let ef = self.eflags;
                        let r = self.shift(i.reg as usize, a, cnt, w);
                        if self.wrrm(mem, i, w, r).is_err() { self.eflags = ef; fault = true; break 'blk; }
                    }
                    H::Not => { let a = t!(self.rdrm(mem, i, w)); t!(self.wrrm(mem, i, w, !a)); }
                    H::Neg => {
                        let a = t!(self.rdrm(mem, i, w));
                        let (r, f) = self.alu_w(5, 0, a, w);
                        t!(self.wrrm(mem, i, w, r));
                        self.eflags = (f & !CF) | if a != 0 { CF } else { 0 };
                    }
                    H::Jcc => { if self.cond(i.aux) { next = i.imm; } }
                    H::Jmp => next = i.imm,
                    H::Call => {
                        let sp = self.r[ESP].wrapping_sub(4);
                        t!(mem.w32(sp, next));
                        self.r[ESP] = sp;
                        next = i.imm;
                    }
                    H::CallRm => {
                        let tgt = t!(self.rdrm(mem, i, 32));
                        let sp = self.r[ESP].wrapping_sub(4);
                        t!(mem.w32(sp, next));
                        self.r[ESP] = sp;
                        next = tgt;
                    }
                    H::JmpRm => next = t!(self.rdrm(mem, i, 32)),
                    H::Ret => {
                        next = t!(mem.r32(self.r[ESP]));
                        self.r[ESP] = self.r[ESP].wrapping_add(4);
                    }
                    H::RetN => {
                        next = t!(mem.r32(self.r[ESP]));
                        self.r[ESP] = self.r[ESP].wrapping_add(4).wrapping_add(i.imm);
                    }
                    H::Leave => {
                        let v = t!(mem.r32(self.r[EBP]));
                        self.r[ESP] = self.r[EBP].wrapping_add(4);
                        self.r[EBP] = v;
                    }
                    H::Setcc => { let v = self.cond(i.aux) as u32; t!(self.wrrm(mem, i, 8, v)); }
                    H::Cmov => {
                        let v = t!(self.rdrm(mem, i, w));
                        if self.cond(i.aux) { self.setr(i.reg as usize, w, v); }
                    }
                    H::Cwde => {
                        if w == 32 { self.r[EAX] = self.r[EAX] as u16 as i16 as i32 as u32 } else { let v = self.reg8(0) as u8 as i8 as i32 as u32; self.setr(EAX, 16, v) }
                    }
                    H::Cdq => {
                        if w == 32 { self.r[EDX] = ((self.r[EAX] as i32) >> 31) as u32 } else { let v = ((self.r[EAX] as u16 as i16) >> 15) as u32; self.setr(EDX, 16, v) }
                    }

                    H::Fpu => {
                        if let Err(e) = self.do_fpu(mem, i, pc) { self.pending = Some(e); st = ST_STOP; break 'blk; }
                    }
                    H::Step | H::Slow => {
                        self.eip = pc;
                        if let Err(e) = self.do_step(mem) { self.pending = Some(e); st = ST_STOP; break 'blk; }
                        if i.h == H::Slow || self.eip != next { brk = true; }
                        next = self.eip;
                    }
                }
                last = pc;
                pc = next;
                done += 1;
                if mem.smc { st = ST_FLUSH; break; }
                if brk { break; }
            }
            if fault { st = ST_FAULT; }
            BlockOut { done: done as u32, pc, last, st }
        }
    }

    #[inline(never)]
    fn do_step(&mut self, mem: &mut Mem) -> R<()> { self.nsteps += 1; self.step(mem) }

    #[inline(never)]
    fn do_fpu(&mut self, mem: &mut Mem, i: &Insn, pc: u32) -> R<()> {
        self.insn_start = pc;
        self.seg = match i.seg { 0 => 0, 1 => self.fs_base, _ => self.gs_base };
        self.eip = pc.wrapping_add(i.reg as u32);
        self.fpu(mem, i.aux, pc)
    }

    /// Run up to `max_steps` instructions. Exactly equivalent to calling `step` that many times.
    pub fn run(&mut self, mem: &mut Mem, max_steps: u64) -> Exit {
        let mut cache = self.cache.0.take().unwrap_or_default();
        if cache.thunk != self.thunk {
            cache.thunk = self.thunk;
            cache.flush();
            mem.clear_code();
        }
        cache.epoch = cache.epoch.wrapping_add(1).max(1);
        mem.smc = false;
        let mut left = max_steps;
        let mut prev = usize::MAX;
        let exit = loop {
            if left == 0 { break Exit::StepLimit; }
            let pc0 = self.eip;
            // Chained successor of the previous block (blocks never exist at thunk addresses).
            let mut bi = usize::MAX;
            if prev != usize::MAX {
                let pb = &cache.blocks[prev];
                for k in 0..2 {
                    if pb.nx[k].0 == pc0 {
                        let c = pb.nx[k].1 as usize;
                        if cache.blocks[c].epoch == cache.epoch { bi = c; }
                        break;
                    }
                }
            }
            if bi == usize::MAX {
                if pc0 >= self.thunk.0 && pc0 < self.thunk.1 {
                    self.insn_start = pc0;
                    break Exit::Thunk(pc0);
                }
                let fl = cache.flushes;
                bi = cache.lookup(mem, pc0);
                if prev != usize::MAX && fl == cache.flushes {
                    let pb = &mut cache.blocks[prev];
                    pb.nx[1] = pb.nx[0];
                    pb.nx[0] = (pc0, bi as u32);
                }
            }
            cache.blocks_run += 1;
            let blk = cache.blocks[bi];
            let n = (blk.n as u64).min(left) as usize;
            let ip = unsafe { cache.insns.as_ptr().add(blk.start as usize) };
            let o = unsafe { self.exec(mem, ip, n, pc0) };
            left -= o.done as u64;
            match o.st {
                ST_STOP => break self.pending.take().unwrap(),
                ST_FAULT => {
                    // The fast handler touched nothing: replay through the byte-wise interpreter.
                    self.eip = o.pc;
                    match self.step(mem) {
                        Err(e) => break e,
                        Ok(()) => { left -= 1; prev = usize::MAX; continue; }
                    }
                }
                _ => {}
            }
            if o.done > 0 { self.insn_start = o.last; }
            self.eip = o.pc;
            prev = bi;
            if o.st == ST_FLUSH {
                cache.flush();
                mem.clear_code();
                prev = usize::MAX;
            }
        };
        self.retired += max_steps - left;
        self.cache.0 = Some(cache);
        exit
    }
}
