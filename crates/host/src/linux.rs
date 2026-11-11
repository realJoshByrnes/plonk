//! Linux i386 personality: ELF loader + `int 0x80` syscall layer, so real Wine Unix-side binaries can run in the interpreter.
//! Milestone 1: static ELF executables, one process, one thread.
use plonk_cpu::{Cpu, Exit, EAX, EBP, EBX, ECX, EDI, EDX, ESI, ESP};
use plonk_mem::Mem;
use std::collections::HashMap;

const MEM_SIZE: u32 = 0x1000_0000; // 256 MB flat
const STACK_TOP: u32 = 0x0FF0_0000;
const MMAP_BASE: u32 = 0x0A00_0000;
const ENOENT: u32 = -2i32 as u32;
const EBADF: u32 = -9i32 as u32;
const ENOMEM: u32 = -12i32 as u32;
const EINVAL: u32 = -22i32 as u32;
const ENOTTY: u32 = -25i32 as u32;
const ENOSYS: u32 = -38i32 as u32;

struct File { data: Vec<u8>, pos: u64, path: String }

pub struct LinuxProc {
    pub mem: Mem,
    pub cpu: Cpu,
    pub stdout: Vec<u8>,
    /// Read-only files visible to the guest (absolute path -> bytes).
    pub files: HashMap<String, Vec<u8>>,
    /// Syscalls answered with ENOSYS (name or number), for finding gaps.
    pub unknown: Vec<String>,
    pub trace: Vec<String>,
    pub exit_code: Option<u32>,
    fds: HashMap<u32, File>,
    next_fd: u32,
    brk: u32,
    mmap_next: u32,
    clock_ns: u64,
}

fn r16(d: &[u8], o: usize) -> u32 { u16::from_le_bytes([d[o], d[o + 1]]) as u32 }
fn r32(d: &[u8], o: usize) -> u32 { u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) }

impl LinuxProc {
    pub fn new() -> Self {
        let mut mem = Mem::new_owned(MEM_SIZE);
        mem.set_guard(0x1000);
        let mut cpu = Cpu::new();
        cpu.linux = true;
        LinuxProc { mem, cpu, stdout: Vec::new(), files: HashMap::new(), unknown: Vec::new(), trace: Vec::new(), exit_code: None,
            fds: HashMap::new(), next_fd: 3, brk: 0, mmap_next: MMAP_BASE, clock_ns: 1_700_000_000_000_000_000 }
    }

    /// Load a static ET_EXEC ELF and build the initial stack (argc/argv/envp/auxv).
    pub fn load(&mut self, elf: &[u8], argv: &[&str], envp: &[&str]) -> Result<(), String> {
        if elf.len() < 52 || &elf[..4] != b"\x7fELF" || elf[4] != 1 || r16(elf, 18) != 3 { return Err("not an i386 ELF".into()); }
        if r16(elf, 16) != 2 { return Err("only static ET_EXEC supported so far".into()); }
        let (entry, phoff, phentsize, phnum) = (r32(elf, 24), r32(elf, 28) as usize, r16(elf, 42) as usize, r16(elf, 44) as usize);
        let (mut end, mut phdr_va) = (0u32, 0u32);
        for i in 0..phnum {
            let p = phoff + i * phentsize;
            let (ty, off, va, filesz, memsz) = (r32(elf, p), r32(elf, p + 4), r32(elf, p + 8), r32(elf, p + 16), r32(elf, p + 20));
            if ty != 1 { continue; }
            let src = elf.get(off as usize..(off + filesz) as usize).ok_or("ELF segment outside file")?;
            self.mem.write_bytes(va, src).map_err(|_| "ELF segment outside guest memory")?;
            end = end.max(va + memsz);
            if off as usize <= phoff && phoff < (off + filesz) as usize { phdr_va = va + (phoff as u32 - off); }
        }
        self.brk = (end + 0xFFF) & !0xFFF;
        // strings at the top of the stack, then the pointer vectors
        let mut sp = STACK_TOP;
        let mut push_str = |mem: &mut Mem, s: &[u8]| -> u32 { sp -= s.len() as u32 + 1; mem.write_bytes(sp, s).unwrap(); mem.w8(sp + s.len() as u32, 0).unwrap(); sp };
        let random = push_str(&mut self.mem, b"plonk-random-16b");
        let argp: Vec<u32> = argv.iter().map(|s| push_str(&mut self.mem, s.as_bytes())).collect();
        let envp: Vec<u32> = envp.iter().map(|s| push_str(&mut self.mem, s.as_bytes())).collect();
        let aux: [(u32, u32); 13] = [(3, phdr_va), (4, phentsize as u32), (5, phnum as u32), (6, 4096), (7, 0), (8, 0), (9, entry),
            (11, 1000), (12, 1000), (13, 1000), (14, 1000), (23, 0), (25, random)];
        let mut words: Vec<u32> = vec![argp.len() as u32];
        words.extend(&argp); words.push(0);
        words.extend(&envp); words.push(0);
        for (k, v) in aux { words.push(k); words.push(v); }
        words.extend([0, 0]);
        sp = (sp - words.len() as u32 * 4) & !0xF;
        for (i, w) in words.iter().enumerate() { self.mem.w32(sp + i as u32 * 4, *w).unwrap(); }
        self.cpu.r[ESP] = sp;
        self.cpu.eip = entry;
        Ok(())
    }

    /// Run until the process exits or `max_steps` instructions were executed. Returns the exit code if it ended.
    pub fn run(&mut self, max_steps: u64) -> Result<Option<u32>, String> {
        let start = self.cpu.retired;
        while self.exit_code.is_none() {
            let left = max_steps.saturating_sub(self.cpu.retired - start);
            if left == 0 { return Ok(None); }
            match self.cpu.run(&mut self.mem, left) {
                Exit::Interrupt(0x80) => self.syscall(),
                Exit::StepLimit => return Ok(None),
                e => return Err(format!("cpu stopped: {e:?} at eip {:#x}", self.cpu.eip)),
            }
        }
        Ok(self.exit_code)
    }

    fn cstr(&self, a: u32) -> String {
        let mut v = Vec::new();
        let mut p = a;
        while let Ok(b) = self.mem.r8(p) { if b == 0 || v.len() > 4096 { break; } v.push(b); p += 1; }
        String::from_utf8_lossy(&v).into_owned()
    }

    fn open(&mut self, path: &str) -> u32 {
        let Some(d) = self.files.get(path) else { self.trace.push(format!("open {path} -> ENOENT")); return ENOENT };
        let fd = self.next_fd;
        self.next_fd += 1;
        self.fds.insert(fd, File { data: d.clone(), pos: 0, path: path.to_string() });
        fd
    }

    fn write_fd(&mut self, fd: u32, buf: u32, n: u32) -> u32 {
        if fd != 1 && fd != 2 { return EBADF; }
        match self.mem.read_bytes(buf, n) { Ok(b) => { self.stdout.extend_from_slice(b); n } Err(_) => -14i32 as u32 }
    }

    /// stat64 for regular files / the tty (st_mode at 16, st_size at 44, st_blksize at 52).
    fn stat64(&mut self, buf: u32, mode: u32, size: u64) -> u32 {
        let _ = self.mem.write_bytes(buf, &[0u8; 96]);
        let _ = self.mem.w32(buf + 16, mode);
        let _ = self.mem.w32(buf + 20, 1);
        let _ = self.mem.w32(buf + 44, size as u32);
        let _ = self.mem.w32(buf + 52, 4096);
        0
    }

    fn mmap(&mut self, addr: u32, len: u32, flags: u32, fd: u32, off: u64) -> u32 {
        let len = (len + 0xFFF) & !0xFFF;
        let a = if flags & 0x10 != 0 { addr } else { // MAP_FIXED
            let a = self.mmap_next;
            if a.checked_add(len).map_or(true, |e| e > STACK_TOP - 0x80_0000) { return ENOMEM; }
            self.mmap_next += len;
            a
        };
        if self.mem.write_bytes(a, &vec![0u8; len as usize]).is_err() { return ENOMEM; }
        if flags & 0x20 == 0 { // file-backed
            let Some(f) = self.fds.get(&fd) else { return EBADF };
            let s = (off as usize).min(f.data.len());
            let e = (s + len as usize).min(f.data.len());
            let d = f.data[s..e].to_vec();
            let _ = self.mem.write_bytes(a, &d);
        }
        a
    }

    fn syscall(&mut self) {
        let r = self.cpu.r;
        let (nr, a, b, c, d, e, f) = (r[EAX], r[EBX], r[ECX], r[EDX], r[ESI], r[EDI], r[EBP]);
        let ret: u32 = match nr {
            1 | 252 => { self.exit_code = Some(a); 0 } // exit, exit_group
            3 => match self.fds.get_mut(&a) { // read
                Some(fl) => {
                    let s = (fl.pos as usize).min(fl.data.len());
                    let n = (c as usize).min(fl.data.len() - s);
                    let chunk = fl.data[s..s + n].to_vec();
                    fl.pos += n as u64;
                    if self.mem.write_bytes(b, &chunk).is_ok() { n as u32 } else { -14i32 as u32 }
                }
                None => if a == 0 { 0 } else { EBADF },
            },
            4 => self.write_fd(a, b, c),
            5 => { let p = self.cstr(a); self.open(&p) }
            295 => { let p = self.cstr(b); self.open(&p) } // openat
            6 => { self.fds.remove(&a); 0 }
            20 | 224 => 1000, // getpid, gettid
            33 => { let p = self.cstr(a); if self.files.contains_key(&p) { 0 } else { ENOENT } } // access
            45 => { // brk
                if a >= self.brk && a < MMAP_BASE { self.brk = a; }
                self.brk
            }
            54 => ENOTTY, // ioctl
            85 | 305 => { // readlink, readlinkat
                let (p, buf, n) = if nr == 85 { (self.cstr(a), b, c) } else { (self.cstr(b), c, d) };
                if p == "/proc/self/exe" { let s = b"/bin/app"; let k = s.len().min(n as usize); let _ = self.mem.write_bytes(buf, &s[..k]); k as u32 } else { EINVAL }
            }
            91 | 125 | 219 => 0, // munmap, mprotect, madvise
            90 => { // old mmap: args in a struct
                let g = |i: u32| self.mem.r32(a + i * 4).unwrap_or(0);
                let (addr, len, flags, fd, off) = (g(0), g(1), g(3), g(4), g(5));
                self.mmap(addr, len, flags, fd, off as u64)
            }
            192 => self.mmap(a, b, d, e, (f as u64) << 12), // mmap2
            122 => { // uname: six 65-byte fields
                let _ = self.mem.write_bytes(a, &[0u8; 390]);
                for (i, s) in ["Linux", "plonk", "6.1.0", "#1", "i686", "(none)"].iter().enumerate() { let _ = self.mem.write_bytes(a + i as u32 * 65, s.as_bytes()); }
                0
            }
            140 => match self.fds.get_mut(&a) { // _llseek(fd, hi, lo, result, whence)
                Some(fl) => {
                    let off = (((b as u64) << 32) | c as u64) as i64;
                    let base = match e { 0 => 0, 1 => fl.pos as i64, _ => fl.data.len() as i64 };
                    fl.pos = (base + off).max(0) as u64;
                    let p = fl.pos;
                    let _ = self.mem.w32(d, p as u32);
                    let _ = self.mem.w32(d + 4, (p >> 32) as u32);
                    0
                }
                None => EBADF,
            },
            146 => { // writev
                let mut total = 0u32;
                for i in 0..c {
                    let (base, len) = (self.mem.r32(b + i * 8).unwrap_or(0), self.mem.r32(b + i * 8 + 4).unwrap_or(0));
                    let w = self.write_fd(a, base, len);
                    if (w as i32) < 0 { total = w; break; }
                    total += w;
                }
                total
            }
            174 | 175 | 186 => 0, // rt_sigaction, rt_sigprocmask, sigaltstack
            195 | 196 => { let p = self.cstr(a); match self.files.get(&p).map(|d| d.len()) { Some(n) => self.stat64(b, 0o100644, n as u64), None => ENOENT } } // stat64, lstat64
            197 => { // fstat64
                if a < 3 { self.stat64(b, 0o020620, 0) } else { match self.fds.get(&a).map(|f| f.data.len()) { Some(n) => self.stat64(b, 0o100644, n as u64), None => EBADF } }
            }
            199..=202 => 1000, // getuid32, getgid32, geteuid32, getegid32
            221 => 0, // fcntl64
            240 | 422 => 0, // futex (single thread: nothing to wait for)
            243 => { // set_thread_area(struct user_desc *)
                let mut entry = self.mem.r32(a).unwrap_or(0);
                let base = self.mem.r32(a + 4).unwrap_or(0);
                if entry == u32::MAX { entry = 6; let _ = self.mem.w32(a, entry); }
                if (6..=8).contains(&entry) {
                    self.cpu.tls[(entry - 6) as usize] = base;
                    if self.cpu.sel_gs >> 3 == entry { self.cpu.gs_base = base; }
                    if self.cpu.sel_fs >> 3 == entry { self.cpu.fs_base = base; }
                    0
                } else { EINVAL }
            }
            258 => 1000, // set_tid_address
            265 | 403 => { // clock_gettime (32-bit / 64-bit timespec)
                self.clock_ns += 1_000_000;
                let (s, ns) = (self.clock_ns / 1_000_000_000, (self.clock_ns % 1_000_000_000) as u32);
                let _ = self.mem.w32(b, s as u32);
                if nr == 403 { let _ = self.mem.w32(b + 4, (s >> 32) as u32); let _ = self.mem.w32(b + 8, ns); } else { let _ = self.mem.w32(b + 4, ns); }
                0
            }
            311 => 0, // set_robust_list
            340 => { // prlimit64: report "unlimited"
                if d != 0 { for i in 0..4 { let _ = self.mem.w32(d + i * 4, u32::MAX); } }
                0
            }
            355 => { for i in 0..b { let _ = self.mem.w8(a + i, (i * 37 + 11) as u8); } b } // getrandom
            _ => {
                let s = format!("syscall {nr}");
                if !self.unknown.contains(&s) { self.unknown.push(s); }
                ENOSYS
            }
        };
        if self.trace.len() < 100_000 { self.trace.push(format!("sys {nr}({a:#x}, {b:#x}, {c:#x}) = {ret:#x}")); }
        self.cpu.r[EAX] = ret;
    }
}

impl Default for LinuxProc {
    fn default() -> Self { Self::new() }
}
