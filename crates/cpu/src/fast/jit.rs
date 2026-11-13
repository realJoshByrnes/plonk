//! Hot blocks translated to WebAssembly.
//!
//! A block that has run often enough is translated into one wasm function `(cpu, tbl, cf, lim) -> done`: the registers and
//! flags are read and written in the `Cpu` it is given, guest memory is reached through the paged memory's table (`tbl`,
//! one host address per 4KB page with flag bits; `cf` the per-page "holds cached code" bytes; `lim` the size of the address
//! space). The host (the web page) compiles the functions of a batch as one module, puts them into the function table of
//! this program and reports back (`installed`); from then on the block is called instead of interpreted.
//!
//! Exactness: a translated function runs the first `jn` instructions of its block. Whenever one of them cannot be done
//! on the fast path (page not present, access crossing a page, write to a copy-on-write page or to a page with cached
//! code) the function returns the number of instructions completed, having changed nothing for the one it stopped at; the
//! interpreter carries on from there. Instructions the translator does not know end the translated part of the block.
//!
//! Translations are kept by (address, code bytes), so every process that runs the same code at the same address shares
//! them, and they survive a flush of the decoded-code cache.
use super::{Insn, H};
use crate::{Cpu, EAX, EBP, EDX, ESP};
use core::mem::offset_of;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

const O_R: u32 = offset_of!(Cpu, r) as u32;
const O_EIP: u32 = offset_of!(Cpu, eip) as u32;
const O_EF: u32 = offset_of!(Cpu, eflags) as u32;
const O_FS: u32 = offset_of!(Cpu, fs_base) as u32;
const O_GS: u32 = offset_of!(Cpu, gs_base) as u32;
const O_FST: u32 = offset_of!(Cpu, fst) as u32;
const O_FTOP: u32 = offset_of!(Cpu, ftop) as u32;
const O_FSW: u32 = offset_of!(Cpu, fsw) as u32;
const O_FCW: u32 = offset_of!(Cpu, fcw) as u32;

// parameters and locals of a translated function
const CPU: u32 = 0;
const TBL: u32 = 1;
const CFL: u32 = 2;
const LIM: u32 = 3;
const A: u32 = 4; // guest address
const E: u32 = 5; // page table entry
const HP: u32 = 6; // host address of a checked write
const X: u32 = 7;
const Y: u32 = 8;
const R: u32 = 9;
const T: u32 = 10;
const EF: u32 = 11;
const K: u32 = 12; // index of the instruction being executed (what a bail-out returns)
const P64: u32 = 13; // the one i64 local
const FA: u32 = 14; // the two f64 locals of the x87 code
const FB: u32 = 15;

/// CF PF AF ZF SF OF
const FLAGS6: u32 = 0x8D5;

/// Byte code of one function body under construction.
struct Fx { b: Vec<u8>, depth: u32 }

impl Fx {
    fn op(&mut self, o: u8) { self.b.push(o); }
    fn uleb(&mut self, mut v: u32) { loop { let c = (v & 0x7F) as u8; v >>= 7; if v == 0 { self.b.push(c); break; } self.b.push(c | 0x80); } }
    fn c(&mut self, v: u32) { // i32.const
        self.b.push(0x41);
        let mut v = v as i32;
        loop { let c = (v & 0x7F) as u8; v >>= 7; if (v == 0 && c & 0x40 == 0) || (v == -1 && c & 0x40 != 0) { self.b.push(c); break; } self.b.push(c | 0x80); }
    }
    fn get(&mut self, l: u32) { self.op(0x20); self.uleb(l); }
    fn set(&mut self, l: u32) { self.op(0x21); self.uleb(l); }
    fn tee(&mut self, l: u32) { self.op(0x22); self.uleb(l); }
    fn mem(&mut self, o: u8, off: u32) { self.op(o); self.op(0); self.uleb(off); }
    fn load(&mut self, w: u32, off: u32) { self.mem(match w { 8 => 0x2D, 16 => 0x2F, _ => 0x28 }, off); }
    fn store(&mut self, w: u32, off: u32) { self.mem(match w { 8 => 0x3A, 16 => 0x3B, _ => 0x36 }, off); }
    fn add(&mut self) { self.op(0x6A); }
    fn sub(&mut self) { self.op(0x6B); }
    fn and(&mut self) { self.op(0x71); }
    fn or(&mut self) { self.op(0x72); }
    fn xor(&mut self) { self.op(0x73); }
    fn shl(&mut self) { self.op(0x74); }
    fn shr_s(&mut self) { self.op(0x75); }
    fn shr_u(&mut self) { self.op(0x76); }
    fn eqz(&mut self) { self.op(0x45); }
    /// Leave the function, returning K, if the value on the stack is not zero.
    fn bail_if(&mut self) { self.op(0x0D); self.uleb(self.depth - 1); }

    /// Offset of a register operand of width `w` inside the Cpu (8-bit: al cl dl bl ah ch dh bh).
    fn roff(i: u8, w: u32) -> u32 { if w == 8 { O_R + 4 * (i as u32 & 3) + (i as u32 >> 2 & 1) } else { O_R + 4 * (i as u32 & 7) } }
    fn getr(&mut self, i: u8, w: u32) { self.get(CPU); self.load(w, Self::roff(i, w)); }
    /// register <- local
    fn setr(&mut self, i: u8, w: u32, l: u32) { self.get(CPU); self.get(l); self.store(w, Self::roff(i, w)); }

    /// Effective address of the memory operand, on the stack.
    fn ea(&mut self, i: &Insn) {
        let mut any = false;
        if i.bm != 0 { self.getr(i.base, 32); any = true; }
        if i.im != 0 { self.getr(i.idx, 32); if i.sc != 0 { self.c(i.sc as u32); self.shl(); } if any { self.add(); } any = true; }
        if i.disp != 0 || !any { self.c(i.disp); if any { self.add(); } }
        if i.seg != 0 { self.get(CPU); self.load(32, if i.seg == 1 { O_FS } else { O_GS }); self.add(); }
    }
    // ---- x87: the registers are f64 in the Cpu, exactly as in fpu.rs ----
    fn f64c(&mut self, v: f64) { self.op(0x44); self.b.extend_from_slice(&v.to_le_bytes()); }
    /// Address of ST(i) (the f64 is then at offset O_FST).
    fn st_addr(&mut self, i: u32) {
        self.get(CPU); self.get(CPU); self.load(32, O_FTOP);
        if i != 0 { self.c(i); self.add(); }
        self.c(7); self.and(); self.c(3); self.shl(); self.add();
    }
    fn fget(&mut self, i: u32) { self.st_addr(i); self.mem(0x2B, O_FST); }
    fn fset(&mut self, i: u32, v: impl FnOnce(&mut Fx)) { self.st_addr(i); v(self); self.mem(0x39, O_FST); }
    /// ftop = ftop + d (mod 8): 1 pops, 7 pushes
    fn ftop_add(&mut self, d: u32) { self.get(CPU); self.get(CPU); self.load(32, O_FTOP); self.c(d); self.add(); self.c(7); self.and(); self.store(32, O_FTOP); }
    /// FA op FB as the x87 arithmetic selector `sub` (0 add, 1 mul, 4 sub, 5 subr, 6 div, 7 divr)
    fn farith(&mut self, sub: u32) {
        match sub {
            0 => { self.get(FA); self.get(FB); self.op(0xA0); }
            1 => { self.get(FA); self.get(FB); self.op(0xA2); }
            4 => { self.get(FA); self.get(FB); self.op(0xA1); }
            5 => { self.get(FB); self.get(FA); self.op(0xA1); }
            6 => { self.get(FA); self.get(FB); self.op(0xA3); }
            _ => { self.get(FB); self.get(FA); self.op(0xA3); }
        }
    }
    /// 1 when FA or FB is a NaN
    fn unord(&mut self) { self.get(FA); self.get(FA); self.op(0x62); self.get(FB); self.get(FB); self.op(0x62); self.or(); }
    /// fcom: C3 C2 C0 of the status word from FA ? FB
    fn fcmp(&mut self) {
        self.get(CPU);
        self.get(CPU); self.load(32, O_FSW); self.c(!0x4500u32); self.and();
        self.get(FA); self.get(FB); self.op(0x63); self.c(8); self.shl(); self.or();
        self.get(FA); self.get(FB); self.op(0x61); self.c(14); self.shl(); self.or();
        self.unord(); self.c(0x4500); self.op(0x6C); self.or();
        self.store(32, O_FSW);
    }
    /// fcomi: ZF PF CF of the flags from FA ? FB
    fn fcomi(&mut self) {
        self.get(EF); self.c(!0x45u32); self.and();
        self.get(FA); self.get(FB); self.op(0x61); self.unord(); self.or(); self.c(6); self.shl(); self.or();
        self.unord(); self.c(2); self.shl(); self.or();
        self.get(FA); self.get(FB); self.op(0x63); self.unord(); self.or(); self.or();
        self.set(EF);
    }
    /// FA rounded as the control word says (or truncated), on the stack
    fn fround(&mut self, trunc: bool) {
        if trunc { self.get(FA); self.op(0x9D); return; }
        // select(a, b, c) is a when c is non-zero: each step keeps the value so far unless the mode asks for the other
        self.get(CPU); self.load(32, O_FCW); self.c(10); self.shr_u(); self.c(3); self.and(); self.set(X);
        self.get(FA); self.op(0x9D); self.get(FA); self.op(0x9B); self.get(X); self.c(2); self.op(0x47); self.op(0x1B);
        self.get(FA); self.op(0x9C); self.get(X); self.c(1); self.op(0x47); self.op(0x1B);
        self.get(FA); self.op(0x9E); self.get(X); self.op(0x1B);
    }
    /// fist/fistp: ST(0) rounded to a `bits` wide integer stored at HP (checked); out of range or NaN gives the "integer indefinite"
    fn fstore_int(&mut self, bits: u32, trunc: bool) {
        self.fget(0); self.set(FA); self.fround(trunc); self.set(FA);
        let (lo, hi) = match bits { 16 => (-32768.0, 32767.0), 32 => (-2147483648.0, 2147483647.0), _ => (-9.223372036854775808e18, 9.223372036854775807e18) };
        self.get(HP);
        self.get(FA);
        if bits == 64 { self.op(0xFC); self.op(0x06); self.op(0x42); self.b.extend_from_slice(&[0x80; 9]); self.b.push(0x7F); }
        else { self.op(0xFC); self.op(0x02); self.c(if bits == 16 { 0x8000 } else { 0x8000_0000 }); }
        self.get(FA); self.f64c(lo); self.op(0x66); self.get(FA); self.f64c(hi); self.op(0x65); self.and();
        self.op(0x1B);
        if bits == 64 { self.mem(0x37, 0); } else { self.store(bits, 0); }
    }
    /// The 64 bits at the guest address in A, as raw i64 (page checks as `read`).
    fn read64(&mut self) {
        self.page();
        self.get(E); self.c(1); self.and(); self.eqz(); self.bail_if();
        self.cross(64);
        self.host(); self.mem(0x29, 0);
    }
    /// Page lookup for the address in A: leaves the table entry in E. Bails when the address is outside the address space.
    fn page(&mut self) {
        self.get(A); self.get(LIM); self.op(0x4F); self.bail_if(); // ge_u
        self.get(TBL); self.get(A); self.c(12); self.shr_u(); self.c(2); self.shl(); self.add(); self.load(32, 0); self.set(E);
    }
    fn cross(&mut self, w: u32) {
        if w > 8 { self.get(A); self.c(0xFFF); self.and(); self.c(4096 - w / 8); self.op(0x4B); self.bail_if(); } // gt_u
    }
    fn host(&mut self) { self.get(E); self.c(!0xFFF); self.and(); self.get(A); self.c(0xFFF); self.and(); self.add(); }
    /// Read `w` bits at the guest address in A (zero-extended), value on the stack.
    fn read(&mut self, w: u32) {
        self.page();
        self.get(E); self.c(1); self.and(); self.eqz(); self.bail_if();
        self.cross(w);
        self.host(); self.load(w, 0);
    }
    /// Check that `w` bits at the guest address in A can be written in place; the host address is left in HP.
    fn wcheck(&mut self, w: u32) {
        self.page();
        self.get(E); self.c(5); self.and(); self.c(1); self.op(0x47); self.bail_if(); // present and not copy-on-write
        self.get(CFL); self.get(A); self.c(12); self.shr_u(); self.add(); self.load(8, 0); self.bail_if(); // no cached code there
        self.cross(w);
        self.host(); self.set(HP);
    }
    /// The r/m operand into local `to`. `write`: it will be written too (memory: HP is then valid).
    fn rm_in(&mut self, i: &Insn, w: u32, to: u32, write: bool) {
        if i.m {
            self.ea(i); self.set(A);
            if write { self.wcheck(w); self.get(HP); self.load(w, 0); } else { self.read(w); }
        } else { self.getr(i.rm, w); }
        self.set(to);
    }
    /// local -> the r/m operand (after `rm_in(.., write = true)`).
    fn rm_out(&mut self, i: &Insn, w: u32, from: u32) {
        if i.m { self.get(HP); self.get(from); self.store(w, 0); } else { self.setr(i.rm, w, from); }
    }
    fn push(&mut self, value: impl FnOnce(&mut Fx)) {
        self.getr(ESP as u8, 32); self.c(4); self.sub(); self.set(A);
        self.wcheck(32);
        self.get(HP); value(self); self.store(32, 0);
        self.setr(ESP as u8, 32, A);
    }
    fn set_eip(&mut self, value: impl FnOnce(&mut Fx)) { self.get(CPU); value(self); self.store(32, O_EIP); }

    /// X op Y -> R for width `w` (operands zero-extended), and the six flags into EF when `flags`. `keep_cf`: inc/dec.
    fn alu(&mut self, op: u8, w: u32, flags: bool, keep_cf: bool) {
        let mask = if w == 32 { u32::MAX } else { (1 << w) - 1 };
        self.get(X); self.get(Y);
        match op { 0 => self.add(), 5 | 7 => self.sub(), 1 => self.or(), 4 => self.and(), _ => self.xor() }
        if w != 32 { self.c(mask); self.and(); }
        self.set(R);
        if !flags { return; }
        self.get(EF); self.c(!(if keep_cf { FLAGS6 & !1 } else { FLAGS6 })); self.and();
        if !keep_cf {
            match op {
                0 if w == 32 => { self.get(R); self.get(X); self.op(0x49); self.or(); } // carry: r <u a
                0 => { self.get(X); self.get(Y); self.add(); self.c(w); self.shr_u(); self.or(); }
                5 | 7 => { self.get(X); self.get(Y); self.op(0x49); self.or(); } // borrow: a <u b
                _ => {}
            }
        }
        if matches!(op, 0 | 5 | 7) { // overflow
            if op == 0 { self.get(X); self.get(R); self.xor(); self.get(Y); self.get(R); self.xor(); self.and(); }
            else { self.get(X); self.get(Y); self.xor(); self.get(X); self.get(R); self.xor(); self.and(); }
            self.c(w - 1); self.shr_u(); self.c(1); self.and(); self.c(11); self.shl(); self.or();
        }
        self.get(X); self.get(Y); self.xor(); self.get(R); self.xor(); self.c(0x10); self.and(); self.or(); // AF
        self.szp(w);
        self.set(EF);
    }
    /// OR the ZF, SF and PF of R (width `w`) into the flags word on the stack.
    fn szp(&mut self, w: u32) {
        self.get(R); self.eqz(); self.c(6); self.shl(); self.or();
        self.get(R); self.c(w - 1); self.shr_u(); self.c(7); self.shl(); self.or();
        self.get(R); self.c(0xFF); self.and(); self.op(0x69); self.c(1); self.and(); self.c(1); self.xor(); self.c(2); self.shl(); self.or(); // popcnt
    }
    /// The condition `cc` as a zero / non-zero value on the stack.
    fn cond(&mut self, cc: u8) {
        let sf_ne_of = |f: &mut Fx| { f.get(EF); f.c(7); f.shr_u(); f.get(EF); f.c(11); f.shr_u(); f.xor(); f.c(1); f.and(); };
        match cc >> 1 {
            0 => { self.get(EF); self.c(0x800); self.and(); }
            1 => { self.get(EF); self.c(1); self.and(); }
            2 => { self.get(EF); self.c(0x40); self.and(); }
            3 => { self.get(EF); self.c(0x41); self.and(); }
            4 => { self.get(EF); self.c(0x80); self.and(); }
            5 => { self.get(EF); self.c(4); self.and(); }
            6 => sf_ne_of(self),
            _ => { sf_ne_of(self); self.get(EF); self.c(0x40); self.and(); self.or(); }
        }
        if cc & 1 != 0 { self.eqz(); }
    }
}

/// How an instruction uses the flags: 0 not at all, 1 reads or changes some, 2 replaces all six without reading.
fn flag_use(i: &Insn) -> u8 {
    match i.h {
        H::AluRmR | H::AluRRm | H::AluAI | H::AluRmI if i.aux != 2 && i.aux != 3 => 2,
        H::Test | H::TestAI | H::TestRmI | H::Neg => 2,
        H::Nop | H::PushR | H::PopR | H::PushI | H::PushRm | H::MovRmR | H::MovRRm | H::MovRI | H::MovRmI | H::Lea | H::Xchg | H::XchgA
        | H::Movzx | H::Movsx | H::Not | H::Cwde | H::Cdq | H::Leave | H::Jmp | H::Call | H::CallRm | H::JmpRm | H::Ret | H::RetN => 0,
        _ => 1, // anything else (also what the interpreter runs) may read the flags
    }
}

/// Translate one instruction. `next` is the address after it, `flags` whether its flag results are needed. False: unsupported.
fn insn(f: &mut Fx, i: &Insn, next: u32, flags: bool, side_exit: Option<u32>) -> bool {
    let w = i.w as u32;
    match i.h {
        H::Nop => {}
        H::AluRmR | H::AluRmI | H::AluRRm | H::AluAI => {
            let op = i.aux;
            if op == 2 || op == 3 { return false; } // adc / sbb
            let mask = if w == 32 { u32::MAX } else { (1 << w) - 1 };
            match i.h {
                H::AluRmR => { f.rm_in(i, w, X, op != 7); f.getr(i.reg, w); f.set(Y); }
                H::AluRmI => { f.rm_in(i, w, X, op != 7); f.c(i.imm & mask); f.set(Y); }
                H::AluRRm => { f.rm_in(i, w, Y, false); f.getr(i.reg, w); f.set(X); }
                _ => { f.getr(EAX as u8, w); f.set(X); f.c(i.imm & mask); f.set(Y); }
            }
            f.alu(op, w, flags, false);
            if op != 7 {
                match i.h { H::AluRmR | H::AluRmI => f.rm_out(i, w, R), H::AluRRm => f.setr(i.reg, w, R), _ => f.setr(EAX as u8, w, R) }
            }
        }
        H::Test | H::TestRmI | H::TestAI => {
            let mask = if w == 32 { u32::MAX } else { (1 << w) - 1 };
            if i.h == H::TestAI { f.getr(EAX as u8, w); f.set(X); } else { f.rm_in(i, w, X, false); }
            if i.h == H::Test { f.getr(i.reg, w); } else { f.c(i.imm & mask); }
            f.set(Y);
            f.alu(4, w, flags, false);
        }
        H::IncR | H::DecR => {
            f.getr(i.rm, w); f.set(X); f.c(1); f.set(Y);
            f.alu(if i.h == H::IncR { 0 } else { 5 }, w, true, true);
            f.setr(i.rm, w, R);
        }
        H::IncDecRm => {
            f.rm_in(i, w, X, true); f.c(1); f.set(Y);
            f.alu(if i.reg == 0 { 0 } else { 5 }, w, true, true);
            f.rm_out(i, w, R);
        }
        H::Neg => {
            f.rm_in(i, w, Y, true); f.c(0); f.set(X);
            f.alu(5, w, flags, false);
            f.rm_out(i, w, R);
        }
        H::Not => {
            f.rm_in(i, w, X, true);
            f.get(X); f.c(u32::MAX); f.xor(); f.set(R);
            f.rm_out(i, w, R);
        }
        H::PushR => f.push(|f| f.getr(i.rm, 32)),
        H::PushI => f.push(|f| f.c(i.imm)),
        H::PushRm => { f.rm_in(i, 32, T, false); f.push(|f| f.get(T)); }
        H::PopR => {
            f.getr(ESP as u8, 32); f.set(A);
            f.read(32); f.set(T);
            f.get(CPU); f.get(A); f.c(4); f.add(); f.store(32, Fx::roff(ESP as u8, 32));
            f.setr(i.rm, 32, T);
        }
        H::MovRmR => {
            if i.m { f.ea(i); f.set(A); f.wcheck(w); f.get(HP); f.getr(i.reg, w); f.store(w, 0); }
            else { f.get(CPU); f.getr(i.reg, w); f.store(w, Fx::roff(i.rm, w)); }
        }
        H::MovRRm => { f.rm_in(i, w, T, false); f.setr(i.reg, w, T); }
        H::MovRI => { f.get(CPU); f.c(i.imm); f.store(w, Fx::roff(i.rm, w)); }
        H::MovRmI => {
            if i.m { f.ea(i); f.set(A); f.wcheck(w); f.get(HP); f.c(i.imm); f.store(w, 0); }
            else { f.get(CPU); f.c(i.imm); f.store(w, Fx::roff(i.rm, w)); }
        }
        H::Lea => { f.get(CPU); f.ea(i); f.store(w, Fx::roff(i.reg, w)); }
        H::Movzx | H::Movsx => {
            let sw = i.aux as u32;
            f.rm_in(i, sw, T, false);
            f.get(CPU); f.get(T);
            if i.h == H::Movsx { f.op(if sw == 8 { 0xC0 } else { 0xC1 }); } // i32.extend8_s / extend16_s
            f.store(w, Fx::roff(i.reg, w));
        }
        H::Xchg if !i.m && w == 32 => {
            f.getr(i.rm, 32); f.set(T); f.getr(i.reg, 32); f.set(X);
            f.setr(i.rm, 32, X); f.setr(i.reg, 32, T);
        }
        H::XchgA if w == 32 => {
            f.getr(i.rm, 32); f.set(T); f.getr(EAX as u8, 32); f.set(X);
            f.setr(i.rm, 32, X); f.setr(EAX as u8, 32, T);
        }
        H::Imul2 | H::Imul3 if w == 32 => {
            f.rm_in(i, 32, X, false);
            f.get(X); f.op(0xAC); // i64.extend_i32_s
            if i.h == H::Imul2 { f.getr(i.reg, 32); } else { f.c(i.imm); }
            f.op(0xAC); f.op(0x7E); f.set(P64); // i64.mul
            f.get(P64); f.op(0xA7); f.set(R); // i32.wrap_i64
            f.setr(i.reg, 32, R);
            // CF and OF set when the product does not fit, the other flags stay
            f.get(EF); f.c(!0x801); f.and();
            f.get(R); f.op(0xAC); f.get(P64); f.op(0x52); f.c(0x801); f.op(0x6C); f.or(); // i64.ne, i32.mul
            f.set(EF);
        }
        H::Shift if w == 32 && i.aux < 2 && matches!(i.reg, 4..=7) => {
            let c = (if i.aux == 0 { i.imm } else { 1 }) & 0x1F;
            if c == 0 { if i.m { return false; } return true; } // nothing changes (a memory operand would still be accessed)
            f.rm_in(i, 32, X, true);
            f.get(X); f.c(c);
            match i.reg { 5 => f.shr_u(), 7 => f.shr_s(), _ => f.shl() }
            f.set(R);
            f.rm_out(i, 32, R);
            // CF, OF from the shift; ZF SF PF from the result; AF stays
            f.get(EF); f.c(!(FLAGS6 & !0x10)); f.and();
            match i.reg {
                5 => {
                    f.get(X); f.c(c - 1); f.shr_u(); f.c(1); f.and(); f.or();
                    f.get(X); f.c(31); f.shr_u(); f.c(11); f.shl(); f.or();
                }
                7 => { f.get(X); f.c(c - 1); f.shr_s(); f.c(1); f.and(); f.or(); }
                _ => {
                    f.get(X); f.c(32 - c); f.shr_u(); f.c(1); f.and(); f.tee(T); f.or();
                    f.get(R); f.c(31); f.shr_u(); f.get(T); f.xor(); f.c(11); f.shl(); f.or();
                }
            }
            f.szp(32);
            f.set(EF);
        }
        H::Setcc => {
            if i.m { f.ea(i); f.set(A); f.wcheck(8); }
            f.cond(i.aux); f.eqz(); f.eqz(); f.set(R);
            f.rm_out(i, 8, R);
        }
        H::Cmov if w == 32 => {
            f.rm_in(i, 32, T, false);
            f.get(CPU); f.get(T); f.getr(i.reg, 32); f.cond(i.aux); f.op(0x1B); f.store(32, Fx::roff(i.reg, 32)); // select
        }
        H::Cwde if w == 32 => { f.get(CPU); f.get(CPU); f.mem(0x2E, O_R); f.store(32, O_R); } // i32.load16_s
        H::Cdq if w == 32 => { f.get(CPU); f.getr(EAX as u8, 32); f.c(31); f.shr_s(); f.store(32, Fx::roff(EDX as u8, 32)); }
        H::Leave => {
            f.getr(EBP as u8, 32); f.set(A);
            f.read(32); f.set(T);
            f.get(CPU); f.get(A); f.c(4); f.add(); f.store(32, Fx::roff(ESP as u8, 32));
            f.setr(EBP as u8, 32, T);
        }
        H::Jcc if side_exit.is_some() => { // not the end of the block: taken leaves through a side exit, not taken falls through
            let k = side_exit.unwrap();
            f.cond(i.aux); f.op(0x04); f.op(0x40);
            f.set_eip(|f| f.c(i.imm));
            f.get(CPU); f.get(EF); f.store(32, O_EF);
            f.c((k + 1) | 0x8000_0000); f.op(0x0F);
            f.op(0x0B);
        }
        H::Jcc => f.set_eip(|f| { f.c(i.imm); f.c(next); f.cond(i.aux); f.op(0x1B); }),
        H::Jmp => f.set_eip(|f| f.c(i.imm)),
        H::Call => { f.push(|f| f.c(next)); f.set_eip(|f| f.c(i.imm)); }
        H::CallRm => { f.rm_in(i, 32, T, false); f.push(|f| f.c(next)); f.set_eip(|f| f.get(T)); }
        H::JmpRm => { f.rm_in(i, 32, T, false); f.set_eip(|f| f.get(T)); }
        H::Ret | H::RetN => {
            f.getr(ESP as u8, 32); f.set(A);
            f.read(32); f.set(T);
            f.get(CPU); f.get(A); f.c(4u32.wrapping_add(if i.h == H::RetN { i.imm } else { 0 })); f.add(); f.store(32, Fx::roff(ESP as u8, 32));
            f.set_eip(|f| f.get(T));
        }
        H::Fpu => return fpu(f, i),
        _ => return false,
    }
    true
}

/// Translate the start of a block at `pc`. Returns the function body and how many instructions it covers (None: not worth it).
fn translate(insns: &[Insn], pc: u32) -> Option<(Vec<u8>, u32)> {
    let mut f = Fx { b: Vec::new(), depth: 1 };
    // locals: 9 x i32 (A..K), 1 x i64
    f.b.extend_from_slice(&[3, 9, 0x7F, 1, 0x7E, 2, 0x7C]);
    f.get(CPU); f.load(32, O_EF); f.set(EF);
    f.op(0x02); f.op(0x40); // block: a branch out of it is a bail-out
    let (mut at, mut jn, mut ended) = (pc, 0u32, false);
    for (k, i) in insns.iter().enumerate() {
        // the flags of this instruction are dead when the next one that touches flags replaces them all
        let needed = insns[k + 1..].iter().map(flag_use).find(|&u| u != 0).map_or(true, |u| u == 1);
        let mark = f.b.len();
        f.c(k as u32); f.set(K);
        let next = at.wrapping_add(i.len as u32);
        let last = k + 1 == insns.len();
        if !insn(&mut f, i, next, needed, (!last && i.h == H::Jcc).then_some(k as u32)) { f.b.truncate(mark); break; }
        at = next;
        jn += 1;
        if super::is_term(i.h) && (last || i.h != H::Jcc) { ended = true; break; }
    }
    finish(f, insns, at, jn, ended)
}

fn finish(mut f: Fx, insns: &[Insn], at: u32, jn: u32, ended: bool) -> Option<(Vec<u8>, u32)> {
    if jn < 2 && (jn as usize) < insns.len() { return None; }
    if jn == 0 { return None; }
    // done: where execution continues (a terminator has stored it already), the flags, and the count
    if !ended { f.set_eip(|f| f.c(at)); }
    f.get(CPU); f.get(EF); f.store(32, O_EF);
    f.c(jn); f.op(0x0F); // return
    f.op(0x0B); // end of the block: bailed out before instruction K
    f.get(CPU); f.get(EF); f.store(32, O_EF);
    f.get(K);
    f.op(0x0B);
    Some((f.b, jn))
}

type Key = (u32, u64);

#[derive(Default)]
struct Jit {
    on: bool,
    /// (address, hash of the code bytes) -> (function table index, instructions covered); index 0: not translatable.
    map: HashMap<Key, (u32, u32)>,
    asked: HashSet<Key>,
    pending: Vec<(Key, u32, Vec<u8>)>,
    batches: HashMap<u32, Vec<(Key, u32)>>,
    next_batch: u32,
    stats: [u64; 8],
}

thread_local! { static JIT: RefCell<Jit> = RefCell::new(Jit::default()); }

/// Turn translation on (the host must then collect modules with `take_module` and report them with `installed`).
pub fn enable(on: bool) { JIT.with(|j| j.borrow_mut().on = on); }
pub(super) fn enabled() -> bool { JIT.with(|j| j.borrow().on) }

/// (functions installed, functions waiting to be compiled, blocks found untranslatable)
pub fn stats() -> (u64, u64, u64) { JIT.with(|j| { let j = j.borrow(); (j.stats[0], j.pending.len() as u64, j.stats[2]) }) }

/// A block got hot (or is asking again): its translation if there is one, else queue it for translation.
pub(super) fn want(pc: u32, bytes: &[u8], insns: &[Insn]) -> Option<(u32, u32)> {
    JIT.with(|j| {
        let mut j = j.borrow_mut();
        if !j.on { return None; }
        let key = (pc, bytes.iter().fold(0xcbf2_9ce4_8422_2325u64 ^ insns.len() as u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)));
        if let Some(&(idx, jn)) = j.map.get(&key) { return (idx != 0).then_some((idx, jn)); }
        if !j.asked.insert(key) { return None; }
        match translate(insns, pc) {
            Some((body, jn)) => j.pending.push((key, jn, body)),
            None => { j.map.insert(key, (0, 0)); j.stats[2] += 1; }
        }
        None
    })
}

/// The queued translations as one wasm module (functions exported as "0", "1", ...; it imports the memory as "e"."m").
/// Returns (batch id, number of functions, module bytes).
pub fn take_module() -> Option<(u32, u32, Vec<u8>)> {
    JIT.with(|j| {
        let mut j = j.borrow_mut();
        if j.pending.is_empty() { return None; }
        let fns: Vec<(Key, u32, Vec<u8>)> = j.pending.drain(..).collect();
        let n = fns.len() as u32;
        let leb = |mut v: u32, out: &mut Vec<u8>| loop { let c = (v & 0x7F) as u8; v >>= 7; if v == 0 { out.push(c); break; } out.push(c | 0x80); };
        let mut m = vec![0, 0x61, 0x73, 0x6D, 1, 0, 0, 0];
        let section = |id: u8, body: Vec<u8>, m: &mut Vec<u8>| { m.push(id); leb(body.len() as u32, m); m.extend_from_slice(&body); };
        section(1, vec![1, 0x60, 4, 0x7F, 0x7F, 0x7F, 0x7F, 1, 0x7F], &mut m); // type 0: (i32 i32 i32 i32) -> i32
        section(2, vec![1, 1, b'e', 1, b'm', 2, 0, 0], &mut m); // import memory e.m, at least 0 pages
        let mut s = Vec::new();
        leb(n, &mut s);
        for _ in 0..n { s.push(0); }
        section(3, s, &mut m);
        let mut s = Vec::new();
        leb(n, &mut s);
        for i in 0..n { let name = i.to_string(); leb(name.len() as u32, &mut s); s.extend_from_slice(name.as_bytes()); s.push(0); leb(i, &mut s); }
        section(7, s, &mut m);
        let mut s = Vec::new();
        leb(n, &mut s);
        for (_, _, body) in &fns { leb(body.len() as u32, &mut s); s.extend_from_slice(body); }
        section(10, s, &mut m);
        let id = j.next_batch;
        j.next_batch += 1;
        j.batches.insert(id, fns.into_iter().map(|(k, jn, _)| (k, jn)).collect());
        Some((id, n, m))
    })
}

/// The host compiled batch `id` and put its functions into the function table at `first`, `first + 1`, ...
pub fn installed(id: u32, first: u32) {
    JIT.with(|j| {
        let mut j = j.borrow_mut();
        let Some(fns) = j.batches.remove(&id) else { return };
        for (i, (key, jn)) in fns.into_iter().enumerate() { j.map.insert(key, (first + i as u32, jn)); j.stats[0] += 1; }
    })
}

/// Call a translated function: (instructions completed).
#[cfg(target_arch = "wasm32")]
#[inline(always)]
pub(super) unsafe fn call(idx: u32, cpu: *mut Cpu, tbl: usize, cf: usize, lim: u32) -> u32 {
    let f: extern "C" fn(*mut Cpu, usize, usize, u32) -> u32 = unsafe { core::mem::transmute(idx as usize) };
    f(cpu, tbl, cf, lim)
}
#[cfg(not(target_arch = "wasm32"))]
pub(super) unsafe fn call(_idx: u32, _cpu: *mut Cpu, _tbl: usize, _cf: usize, _lim: u32) -> u32 { 0 }

// ---- profiling: which instructions take the slow path ----
thread_local! { static PROF: RefCell<Option<HashMap<String, u64>>> = RefCell::new(None); }
pub fn profile(on: bool) { PROF.with(|p| *p.borrow_mut() = on.then(HashMap::new)); }
pub(super) fn profiling() -> bool { PROF.with(|p| p.borrow().is_some()) }
pub(super) fn count_op(mem: &plonk_mem::Mem, pc: u32) {
    let mut b = [0u8; 4];
    for (k, x) in b.iter_mut().enumerate() { *x = mem.r8(pc.wrapping_add(k as u32)).unwrap_or(0); }
    let mut p = 0;
    let mut pre = String::new();
    while p < 3 && matches!(b[p], 0x66 | 0xF2 | 0xF3 | 0x64 | 0x65 | 0xF0 | 0x2E | 0x36 | 0x3E | 0x26) { if matches!(b[p], 0x66 | 0xF2 | 0xF3) { pre += &format!("{:02x} ", b[p]); } p += 1; }
    let key = if b[p] == 0x0F { format!("{pre}0f {:02x}", b[p + 1]) } else { format!("{pre}{:02x}", b[p]) };
    PROF.with(|m| if let Some(m) = m.borrow_mut().as_mut() { *m.entry(key).or_insert(0) += 1; });
}
/// The histogram, most frequent first, as lines "count opcode".
pub fn profile_report() -> String {
    PROF.with(|m| {
        let m = m.borrow();
        let Some(m) = m.as_ref() else { return String::new() };
        let mut v: Vec<_> = m.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        let st = JIT.with(|j| j.borrow().stats);
        format!("blocks {} (with translation {}, bailed {}), instructions translated {} interpreted {}
", st[7], st[6], st[5], st[3], st[4])
            + &v.iter().take(40).map(|(k, n)| format!("{n} {k}")).collect::<Vec<_>>().join("
")
    })
}
pub(super) fn tally(jit_insns: u64, interp_insns: u64, bailed: bool, has_jit: bool) {
    JIT.with(|j| { let mut j = j.borrow_mut(); j.stats[3] += jit_insns; j.stats[4] += interp_insns; j.stats[5] += bailed as u64; j.stats[6] += has_jit as u64; j.stats[7] += 1; });
}

/// x87 instruction (decoded with the ModRM byte in `imm`), mirroring fpu.rs. False: not handled here.
thread_local! { static FPU_OFF: std::cell::Cell<u32> = std::cell::Cell::new(0); }
/// Debugging: bit (opcode - 0xD8) * 2 + (register form) switches that class of x87 translation off.
pub fn fpu_off(mask: u32) { FPU_OFF.with(|m| m.set(mask)); }

fn fpu(f: &mut Fx, i: &Insn) -> bool {
    let (op, mb) = (i.aux, i.imm as u8);
    if FPU_OFF.with(|m| m.get()) >> ((op as u32 - 0xD8) * 2 + (!i.m) as u32) & 1 != 0 { return false; }
    let (reg, ri) = ((mb >> 3) & 7, (mb & 7) as u32);
    if i.m && FPU_OFF.with(|m| m.get()) >> (16 + reg) & 1 != 0 { return false; }
    if i.m {
        // memory operand: the address into A, every check before the first change of state
        let load_f32 = |f: &mut Fx| { f.ea(i); f.set(A); f.read(32); f.op(0xBE); f.op(0xBB); };
        let load_f64 = |f: &mut Fx| { f.ea(i); f.set(A); f.page(); f.get(E); f.c(1); f.and(); f.eqz(); f.bail_if(); f.cross(64); f.host(); f.mem(0x2B, 0); };
        let load_i32 = |f: &mut Fx| { f.ea(i); f.set(A); f.read(32); f.op(0xB7); };
        let load_i16 = |f: &mut Fx| { f.ea(i); f.set(A); f.read(16); f.op(0xC1); f.op(0xB7); };
        let load_i64 = |f: &mut Fx| { f.ea(i); f.set(A); f.read64(); f.op(0xB9); };
        let push = |f: &mut Fx| { f.ftop_add(7); f.fset(0, |f| f.get(FA)); };
        match (op, reg) {
            (0xD8, _) | (0xDC, _) | (0xDA, _) | (0xDE, _) => {
                match op { 0xD8 => load_f32(f), 0xDC => load_f64(f), 0xDA => load_i32(f), _ => load_i16(f) }
                f.set(FB); f.fget(0); f.set(FA);
                if reg == 2 || reg == 3 { f.fcmp(); if reg == 3 { f.ftop_add(1); } } else { f.fset(0, |f| f.farith(reg as u32)); }
            }
            (0xD9, 0) => { load_f32(f); f.set(FA); push(f); }
            (0xD9, 2) | (0xD9, 3) => {
                f.ea(i); f.set(A); f.wcheck(32);
                f.get(HP); f.fget(0); f.op(0xB6); f.mem(0x38, 0);
                if reg == 3 { f.ftop_add(1); }
            }
            (0xD9, 5) => { f.get(CPU); f.ea(i); f.set(A); f.read(16); f.store(32, O_FCW); }
            (0xD9, 7) => { f.ea(i); f.set(A); f.wcheck(16); f.get(HP); f.get(CPU); f.load(32, O_FCW); f.store(16, 0); }
            (0xDB, 0) => { load_i32(f); f.set(FA); push(f); }
            (0xDB, 1) | (0xDB, 2) | (0xDB, 3) => { f.ea(i); f.set(A); f.wcheck(32); f.fstore_int(32, reg == 1); if reg != 2 { f.ftop_add(1); } }
            (0xDD, 0) => { load_f64(f); f.set(FA); push(f); }
            (0xDD, 1) => { f.ea(i); f.set(A); f.wcheck(64); f.fstore_int(64, true); f.ftop_add(1); }
            (0xDD, 2) | (0xDD, 3) => {
                f.ea(i); f.set(A); f.wcheck(64);
                f.get(HP); f.fget(0); f.mem(0x39, 0);
                if reg == 3 { f.ftop_add(1); }
            }
            (0xDD, 7) => {
                f.ea(i); f.set(A); f.wcheck(16);
                f.get(HP); f.get(CPU); f.load(32, O_FSW); f.c(!0x3800u32); f.and(); f.get(CPU); f.load(32, O_FTOP); f.c(11); f.shl(); f.or(); f.store(16, 0);
            }
            (0xDF, 0) => { load_i16(f); f.set(FA); push(f); }
            (0xDF, 1) | (0xDF, 2) | (0xDF, 3) => { f.ea(i); f.set(A); f.wcheck(16); f.fstore_int(16, reg == 1); if reg != 2 { f.ftop_add(1); } }
            (0xDF, 5) => { load_i64(f); f.set(FA); push(f); }
            (0xDF, 7) => { f.ea(i); f.set(A); f.wcheck(64); f.fstore_int(64, false); f.ftop_add(1); }
            _ => return false,
        }
        return true;
    }
    let cmov = |f: &mut Fx, neg: bool| {
        let mask = match reg { 0 => 1, 1 => 0x40, 2 => 0x41, _ => 4 };
        f.fset(0, |f| { f.fget(ri); f.fget(0); f.get(EF); f.c(mask); f.and(); if neg { f.eqz(); } f.op(0x1B); });
    };
    match (op, reg, ri) {
        (0xD8, _, _) => {
            f.fget(0); f.set(FA); f.fget(ri); f.set(FB);
            if reg == 2 || reg == 3 { f.fcmp(); if reg == 3 { f.ftop_add(1); } } else { f.fset(0, |f| f.farith(reg as u32)); }
        }
        (0xDE, 3, 1) => { f.fget(0); f.set(FA); f.fget(1); f.set(FB); f.fcmp(); f.ftop_add(1); f.ftop_add(1); }
        (0xDC, 2..=3, _) | (0xDE, 2..=3, _) => return false,
        (0xDC, _, _) | (0xDE, _, _) => {
            let sub = match reg { 4 => 5, 5 => 4, 6 => 7, 7 => 6, r => r } as u32;
            f.fget(ri); f.set(FA); f.fget(0); f.set(FB);
            f.fset(ri, |f| f.farith(sub));
            if op == 0xDE { f.ftop_add(1); }
        }
        (0xD9, 0, _) => { f.fget(ri); f.set(FA); f.ftop_add(7); f.fset(0, |f| f.get(FA)); }
        (0xD9, 1, _) => { f.fget(0); f.set(FA); f.fget(ri); f.set(FB); f.fset(0, |f| f.get(FB)); f.fset(ri, |f| f.get(FA)); }
        (0xD9, 2, 0) => {}
        (0xD9, 4, 0) => f.fset(0, |f| { f.fget(0); f.op(0x9A); }),
        (0xD9, 4, 1) => f.fset(0, |f| { f.fget(0); f.op(0x99); }),
        (0xD9, 4, 4) => { f.fget(0); f.set(FA); f.f64c(0.0); f.set(FB); f.fcmp(); }
        (0xD9, 5, k) if k != 7 => {
            let v = [1.0, 10f64.log2(), std::f64::consts::LOG2_E, std::f64::consts::PI, 2f64.log10(), std::f64::consts::LN_2, 0.0][k as usize];
            f.ftop_add(7); f.fset(0, |f| f.f64c(v));
        }
        (0xD9, 6, 6) => f.ftop_add(7),
        (0xD9, 6, 7) => f.ftop_add(1),
        (0xD9, 7, 2) => f.fset(0, |f| { f.fget(0); f.op(0x9F); }),
        (0xD9, 7, 4) => { f.fget(0); f.set(FA); f.fset(0, |f| f.fround(false)); }
        (0xDA, 0..=3, _) => cmov(f, false),
        (0xDA, 5, 1) => { f.fget(0); f.set(FA); f.fget(1); f.set(FB); f.fcmp(); f.ftop_add(1); f.ftop_add(1); }
        (0xDB, 0..=3, _) => cmov(f, true),
        (0xDB, 4, 2) => { f.get(CPU); f.get(CPU); f.load(32, O_FSW); f.c(!0x80FFu32); f.and(); f.store(32, O_FSW); }
        (0xDB, 4, 3) => {
            f.get(CPU); f.c(0x37F); f.store(32, O_FCW);
            f.get(CPU); f.c(0); f.store(32, O_FSW);
            f.get(CPU); f.c(0); f.store(32, O_FTOP);
        }
        (0xDB, 5..=6, _) => { f.fget(0); f.set(FA); f.fget(ri); f.set(FB); f.fcomi(); }
        (0xDD, 0, _) => {}
        (0xDD, 2..=3, _) => { f.fget(0); f.set(FA); f.fset(ri, |f| f.get(FA)); if reg == 3 { f.ftop_add(1); } }
        (0xDD, 4..=5, _) => { f.fget(0); f.set(FA); f.fget(ri); f.set(FB); f.fcmp(); if reg == 5 { f.ftop_add(1); } }
        (0xDF, 4, 0) => {
            f.get(CPU);
            f.get(CPU); f.load(32, O_R); f.c(0xFFFF_0000); f.and();
            f.get(CPU); f.load(32, O_FSW); f.c(!0x3800u32); f.and(); f.get(CPU); f.load(32, O_FTOP); f.c(11); f.shl(); f.or();
            f.or(); f.store(32, O_R);
        }
        (0xDF, 5..=6, _) => { f.fget(0); f.set(FA); f.fget(ri); f.set(FB); f.fcomi(); f.ftop_add(1); }
        _ => return false,
    }
    true
}
