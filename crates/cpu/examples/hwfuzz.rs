//! Differential test of the interpreter against real hardware results (see tools/hwfuzz/README.md).
//! cargo run --release -p plonk-cpu --example hwfuzz -- tools/hwfuzz/out [--slow] [--all] [--filter SUBSTR]
use plonk_cpu::{Cpu, Exit};
use plonk_mem::Mem;

const BASE: u32 = 0x210000;
const TABLE: u32 = 0x220000;
const CODE: u32 = 0x230000;
const SCR: usize = 512;
const STACK: u32 = 0x3F0000;
const RN: [&str; 8] = ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi"];

struct Rd<'a>(&'a [u8], usize);
impl<'a> Rd<'a> {
    fn u32(&mut self) -> u32 { let v = u32::from_le_bytes(self.0[self.1..self.1 + 4].try_into().unwrap()); self.1 += 4; v }
    fn bytes(&mut self, n: usize) -> &'a [u8] { let s = &self.0[self.1..self.1 + n]; self.1 += n; s }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).map(String::as_str).unwrap_or("tools/hwfuzz/out");
    let slow = args.iter().any(|a| a == "--slow");
    let all = args.iter().any(|a| a == "--all");
    let filter = args.iter().position(|a| a == "--filter").and_then(|i| args.get(i + 1)).cloned();
    let cases = std::fs::read(format!("{dir}/cases.bin")).expect("cases.bin");
    let hw = std::fs::read(format!("{dir}/hw.bin")).expect("hw.bin (run harness.exe first)");
    let code = std::fs::read(format!("{dir}/code.bin")).expect("code.bin");
    let (mut c, mut h, mut k) = (Rd(&cases, 4), Rd(&hw, 0), Rd(&code, 0));
    let nt = c.u32();
    assert_eq!(k.u32(), nt);
    let mut mem = Mem::new_owned(0x400000);
    for i in 0..256u32 {
        let mut v = i;
        for _ in 0..8 { v = if v & 1 != 0 { (v >> 1) ^ 0xEDB88320 } else { v >> 1 }; }
        mem.w32(TABLE + i * 4, v).unwrap();
    }
    let mut rows: Vec<(usize, String, String)> = Vec::new(); // (count, name, detail)
    let (mut total, mut bad, mut skipped, mut bad_t) = (0usize, 0usize, 0usize, 0usize);
    for _t in 0..nt {
        let nl = c.u32() as usize;
        let name = String::from_utf8_lossy(c.bytes(nl)).to_string();
        let has_mem = c.u32() != 0;
        let n = c.u32();
        let len = k.u32() as usize;
        let tcode = k.bytes(len).to_vec();
        let skip_t = filter.as_ref().map_or(false, |f| !name.contains(f.as_str()));
        let mut cbuf = tcode.clone();
        cbuf.push(0xF4);
        mem.write_bytes(CODE, &cbuf).unwrap();
        let (mut nbad, mut first) = (0usize, String::new());
        for _ in 0..n {
            let mut r = [0u32; 8];
            for x in r.iter_mut() { *x = c.u32(); }
            let fl = c.u32();
            let mask = c.u32();
            let skip = c.u32();
            let buf = if has_mem { c.bytes(SCR) } else { &[][..] };
            let faulted = h.u32();
            let mut er = [0u32; 8];
            for x in er.iter_mut() { *x = h.u32(); }
            let efl = h.u32();
            let ebuf = if has_mem { h.bytes(SCR) } else { &[][..] };
            if skip_t { continue; }
            if faulted != 0 { skipped += 1; continue; }
            total += 1;
            if has_mem { mem.write_bytes(BASE, buf).unwrap(); }
            let mut cpu = Cpu::new();
            cpu.r = r;
            cpu.r[4] = STACK;
            cpu.eflags = (fl & 0xCD5) | 0x202;
            cpu.eip = CODE;
            let e = if slow { cpu.run_slow(&mut mem, 200_000) } else { cpu.run(&mut mem, 200_000) };
            let mut diffs: Vec<String> = Vec::new();
            if !matches!(e, Exit::Halt) { diffs.push(format!("exit={:?}", e)); }
            else {
                for i in 0..8 {
                    if skip & (1 << i) != 0 { continue; }
                    let got = if i == 4 { cpu.r[4].wrapping_sub(STACK) } else { cpu.r[i] };
                    if got != er[i] { diffs.push(format!("{} exp={:#x} got={:#x}", RN[i], er[i], got)); }
                }
                if (cpu.eflags ^ efl) & mask != 0 {
                    diffs.push(format!("flags exp={} got={} (xor {:#x})", fs(efl & mask), fs(cpu.eflags & mask), (cpu.eflags ^ efl) & mask));
                }
                if has_mem {
                    let got = mem.read_bytes(BASE, SCR as u32).unwrap();
                    let d: Vec<usize> = (0..SCR).filter(|&i| got[i] != ebuf[i]).collect();
                    if !d.is_empty() {
                        let s: Vec<String> = d.iter().take(4).map(|&i| format!("[{i}] exp={:02x} got={:02x}", ebuf[i], got[i])).collect();
                        diffs.push(format!("mem {} bytes differ: {}", d.len(), s.join(", ")));
                    }
                }
            }
            if !diffs.is_empty() {
                nbad += 1;
                if first.is_empty() {
                    let ins: Vec<String> = (0..8).filter(|&i| i != 4).map(|i| format!("{}={:08x}", RN[i], r[i])).collect();
                    first = format!("    in : {} fl={}{}\n    {}", ins.join(" "), fs(fl & 0xCD5),
                        if has_mem { format!(" (esi/edi/ebx rel {:#x}/{:#x}/{:#x})", r[6].wrapping_sub(BASE), r[7].wrapping_sub(BASE), r[3].wrapping_sub(BASE)) } else { String::new() },
                        diffs.join("\n    "));
                }
            }
            bad += (nbad > 0 && false) as usize;
        }
        if nbad > 0 { bad += nbad; bad_t += 1; rows.push((nbad, name, first)); }
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    let lim = if all { usize::MAX } else { 40 };
    for (n, name, d) in rows.iter().take(lim) { println!("{n:4} mismatches  {name}\n{d}"); }
    if rows.len() > lim { println!("... {} more failing templates (use --all)", rows.len() - lim); }
    println!("\nSUMMARY: {nt} templates, {total} cases compared ({skipped} skipped: faulted on hw), {bad} mismatching cases in {bad_t} templates{}", if slow { " [run_slow]" } else { "" });
    if bad > 0 { std::process::exit(1); }
}

fn fs(f: u32) -> String {
    let mut s = String::new();
    for (b, c) in [(0x800, 'O'), (0x400, 'D'), (0x80, 'S'), (0x40, 'Z'), (0x10, 'A'), (0x4, 'P'), (0x1, 'C')] {
        if f & b != 0 { s.push(c) } else { s.push('.') }
    }
    s
}
