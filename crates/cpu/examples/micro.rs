//! Per-instruction cost of the interpreter core on tiny loops: cargo run --release -p plonk-cpu --example micro
use plonk_cpu::{Cpu, Exit};
use plonk_mem::Mem;
use std::time::Instant;

fn run(name: &str, code: &[u8], iters: u32, insns_per_iter: u32) {
    let mut mem = Mem::new_owned(1 << 20);
    mem.write_bytes(0x1000, code).unwrap();
    let mut cpu = Cpu::new();
    cpu.eip = 0x1000;
    cpu.r[1] = iters; // ecx
    cpu.r[4] = 0x8000; // esp
    cpu.r[6] = 0x4000; // esi (scratch memory)
    let t = Instant::now();
    let e = cpu.run(&mut mem, iters as u64 * insns_per_iter as u64 + 10);
    let dt = t.elapsed().as_secs_f64();
    let n = cpu.retired as f64;
    println!("{name:28} {:>6.1} MIPS ({:?}, {:.1} ns/insn)", n / 1e6 / dt, matches!(e, Exit::StepLimit | Exit::Halt), dt * 1e9 / n);
}

fn steps(name: &str, code: &[u8], n: u64) {
    let mut mem = Mem::new_owned(1 << 20);
    mem.write_bytes(0x1000, code).unwrap();
    let mut cpu = Cpu::new();
    cpu.eip = 0x1000; cpu.r[4] = 0x8000; cpu.r[6] = 0x4000;
    let t = Instant::now();
    let _ = cpu.run(&mut mem, n);
    let dt = t.elapsed().as_secs_f64();
    println!("{name:28} {:>6.1} MIPS ({:.1} ns/insn)", n as f64 / 1e6 / dt, dt * 1e9 / n as f64);
}

fn main() {
    { let mut v = vec![0x90u8; 60]; v.extend_from_slice(&[0xEB, 0xC2]); steps("60 nops; jmp", &v, 60_000_000); }
    { let mut v = Vec::new(); for _ in 0..30 { v.extend_from_slice(&[0x89, 0xD8]); } v.extend_from_slice(&[0xEB, 0xC2]); steps("30 mov eax,ebx; jmp", &v, 60_000_000); }
    steps("jmp short self (0xEB 0xFE)", &[0xEB, 0xFE], 50_000_000);
    steps("inc eax; jmp -3", &[0x40, 0xEB, 0xFD], 50_000_000);
    steps("add eax,ebx; jmp", &[0x01, 0xD8, 0xEB, 0xFC], 50_000_000);
    steps("mov eax,ebx; jmp", &[0x89, 0xD8, 0xEB, 0xFC], 50_000_000);
    steps("mov eax,[esi]; jmp", &[0x8B, 0x06, 0xEB, 0xFC], 50_000_000);
    steps("mov eax,5; jmp", &[0xB8, 5, 0, 0, 0, 0xEB, 0xF9], 50_000_000);

    // add eax,ebx ; dec ecx ; jnz -5
    run("alu reg (add/dec/jnz)", &[0x01, 0xD8, 0x49, 0x75, 0xFB, 0xF4], 30_000_000, 3);
    // mov eax,[esi] ; add eax,1 ; mov [esi],eax ; dec ecx ; jnz
    run("mem load/add/store", &[0x8B, 0x06, 0x83, 0xC0, 0x01, 0x89, 0x06, 0x49, 0x75, 0xF6, 0xF4], 20_000_000, 5);
    // push eax ; pop eax ; dec ecx ; jnz
    run("push/pop", &[0x50, 0x58, 0x49, 0x75, 0xFB, 0xF4], 30_000_000, 4);
    // nop x4 ; dec ecx ; jnz
    run("nop", &[0x90, 0x90, 0x90, 0x90, 0x49, 0x75, 0xF9, 0xF4], 30_000_000, 6);
}
