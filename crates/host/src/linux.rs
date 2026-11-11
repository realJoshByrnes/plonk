//! Linux i386 personality: ELF loader + `int 0x80` syscall layer, so real Wine Unix-side binaries can run in the interpreter.
//! A small kernel: processes (fork/exec/wait) and threads (clone/futex) scheduled round robin, static and dynamic ELFs,
//! a writable file tree, Unix stream sockets and pipes, poll. The running process/thread has its state swapped into the
//! top-level fields (`mem`, `cpu`, `fds`, ...); everything else is parked in `procs`.
use plonk_cpu::{CacheSlot, Cpu, Exit, EAX, EBP, EBX, ECX, EDI, EDX, ESI, ESP};
use plonk_mem::Mem;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::rc::Rc;

const MEM_SIZE: u32 = 0xC000_0000; // 3 GB sparse address space per process, like Linux on i386
const STACK_TOP: u32 = 0xBFF0_0000;
const STACK_LOW: u32 = STACK_TOP - 0x80_0000;
const MMAP_MIN: u32 = 0x1_0000;
const EXE_BIAS: u32 = 0x9000_0000;
const INTERP_BIAS: u32 = 0xB000_0000;
const SLICE: u64 = 200_000;
const ECHILD: u32 = -10i32 as u32;
const ESRCH: u32 = -3i32 as u32;
const ETIMEDOUT: u32 = -110i32 as u32;
const ENOENT: u32 = -2i32 as u32;
const EBADF: u32 = -9i32 as u32;
const EAGAIN: u32 = -11i32 as u32;
const ENOMEM: u32 = -12i32 as u32;
const EFAULT: u32 = -14i32 as u32;
const EEXIST: u32 = -17i32 as u32;
const ENOTDIR: u32 = -20i32 as u32;
const EINVAL: u32 = -22i32 as u32;
const ENOTTY: u32 = -25i32 as u32;
const EPIPE: u32 = -32i32 as u32;
const ENOSYS: u32 = -38i32 as u32;
const ENOTSOCK: u32 = -88i32 as u32;
const ECONNREFUSED: u32 = -111i32 as u32;
const O_NONBLOCK: u32 = 0x800;
const POLLIN: u16 = 1;
const POLLOUT: u16 = 4;
const POLLHUP: u16 = 0x10;
const POLLNVAL: u16 = 0x20;

type Data = Rc<RefCell<Vec<u8>>>;
type Fd = Rc<RefCell<Ofd>>;

/// Things created in the file tree at run time (the read-only `files` map sits underneath).
enum Node { File(Data), Dir, Sock(usize), Link(String) }

enum Kind { File { data: Data, pos: u64 }, Dir { pos: usize }, Null, Tty, Sock(usize) }

/// An open file description (shared by dup'ed descriptors and descriptors passed over sockets).
pub struct Ofd { kind: Kind, flags: u32, path: String }

struct Msg { data: Vec<u8>, off: usize, fds: Vec<Fd> }

/// One end of a Unix stream socket or pipe.
#[derive(Default)]
struct Sock { peer: Option<usize>, rx: VecDeque<Msg>, listening: bool, backlog: VecDeque<usize>, closed: bool, shut_wr: bool }

#[derive(Clone, Copy, PartialEq)]
enum St { Run, Futex(u32), Vfork(usize), Dead }

struct Thread { cpu: Cpu, tid: u32, poll_deadline: Option<u64>, clear_tid: u32, state: St, wake_at: Option<u64> }

/// A parked process. For the running one, the swapped fields hold placeholders.
struct Proc {
    pid: u32, ppid: u32, mem: Mem, fds: HashMap<u32, Fd>, cwd: String, brk: u32, brk_start: u32, maps: BTreeMap<u32, u32>, umask: u32,
    exe: String, sigact: HashMap<u32, [u32; 5]>, threads: Vec<Thread>, cache: CacheSlot, exit: Option<u32>, reaped: bool,
}

pub struct LinuxProc {
    pub mem: Mem,
    pub cpu: Cpu,
    pub stdout: Vec<u8>,
    /// Read-only files visible to the guest (absolute path -> bytes).
    pub files: HashMap<String, Vec<u8>>,
    /// Syscalls answered with ENOSYS, for finding gaps.
    pub unknown: Vec<String>,
    pub trace: Vec<String>,
    pub exit_code: Option<u32>,
    /// The process is waiting (poll/read/accept) for something that only time or another party can provide.
    pub blocked: bool,
    vfs: BTreeMap<String, Node>,
    cwd: String,
    fds: HashMap<u32, Fd>,
    socks: Vec<Sock>,
    brk: u32,
    brk_start: u32,
    /// Mapped ranges of the running process (start -> end).
    maps: BTreeMap<u32, u32>,
    clock_ns: u64,
    poll_deadline: Option<u64>,
    umask: u32,
    exe: String,
    sigact: HashMap<u32, [u32; 5]>,
    procs: Vec<Proc>,
    cur: (usize, usize),
    next_pid: u32,
    resched: bool,
    /// When every thread is waiting, jump the guest clock to the next timeout instead of reporting `blocked`.
    pub auto_time: bool,
}

fn r16(d: &[u8], o: usize) -> u32 { u16::from_le_bytes([d[o], d[o + 1]]) as u32 }
fn r32(d: &[u8], o: usize) -> u32 { u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) }
fn ino(path: &str) -> u32 { path.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193)) | 0x1000 }
fn parent(path: &str) -> &str { match path.rfind('/') { Some(0) | None => "/", Some(i) => &path[..i] } }

impl LinuxProc {
    pub fn new() -> Self {
        let mut vfs = BTreeMap::new();
        for d in ["/", "/tmp", "/dev", "/home", "/home/user", "/etc", "/proc", "/proc/self"] { vfs.insert(d.to_string(), Node::Dir); }
        LinuxProc { mem: Mem::new_owned(0), cpu: Cpu::new(), stdout: Vec::new(), files: HashMap::new(), unknown: Vec::new(), trace: Vec::new(), exit_code: None, blocked: false,
            vfs, cwd: "/".into(), fds: HashMap::new(), socks: Vec::new(), brk: 0, brk_start: 0, maps: BTreeMap::new(), clock_ns: 1_700_000_000_000_000_000, poll_deadline: None, umask: 0o022,
            exe: String::new(), sigact: HashMap::new(), procs: Vec::new(), cur: (usize::MAX, 0), next_pid: 1000, resched: false, auto_time: false }
    }

    // ---- processes and threads ----

    fn swap_proc(&mut self) {
        use std::mem::swap;
        let p = &mut self.procs[self.cur.0];
        swap(&mut self.mem, &mut p.mem); swap(&mut self.fds, &mut p.fds); swap(&mut self.cwd, &mut p.cwd); swap(&mut self.brk, &mut p.brk);
        swap(&mut self.brk_start, &mut p.brk_start); swap(&mut self.maps, &mut p.maps); swap(&mut self.umask, &mut p.umask);
        swap(&mut self.exe, &mut p.exe); swap(&mut self.sigact, &mut p.sigact);
    }
    fn swap_thread(&mut self) {
        let t = &mut self.procs[self.cur.0].threads[self.cur.1];
        std::mem::swap(&mut self.cpu, &mut t.cpu);
        std::mem::swap(&mut self.poll_deadline, &mut t.poll_deadline);
    }
    /// Make (process, thread) the running one. The decoded-code cache belongs to the process and travels with whichever thread runs.
    fn switch(&mut self, pi: usize, ti: usize) {
        if (pi, ti) == self.cur { return; }
        if self.cur.0 != usize::MAX {
            let c = self.cpu.take_cache();
            self.swap_thread();
            self.procs[self.cur.0].cache = c;
            if pi != self.cur.0 { self.swap_proc(); }
        }
        if pi != self.cur.0 { self.cur.0 = pi; self.swap_proc(); }
        self.cur.1 = ti;
        self.swap_thread();
        let c = std::mem::take(&mut self.procs[pi].cache);
        self.cpu.put_cache(c);
    }
    fn me(&mut self) -> &mut Thread { &mut self.procs[self.cur.0].threads[self.cur.1] }
    fn pid(&self) -> u32 { self.procs[self.cur.0].pid }
    fn new_cpu() -> Cpu { let mut c = Cpu::new(); c.linux = true; c }
    fn blank_proc(&mut self, ppid: u32) -> Proc {
        let pid = self.next_pid;
        self.next_pid += 1;
        let mut mem = Mem::new_sparse(MEM_SIZE);
        Proc { pid, ppid, mem, fds: HashMap::new(), cwd: "/".into(), brk: 0, brk_start: 0, maps: BTreeMap::new(), umask: 0o022, exe: String::new(), sigact: HashMap::new(),
            threads: vec![Thread { cpu: Self::new_cpu(), tid: pid, poll_deadline: None, clear_tid: 0, state: St::Run, wake_at: None }], cache: CacheSlot::default(), exit: None, reaped: false }
    }

    /// Start a new process from an ELF image (a child of "init"). Returns its pid. It becomes the running process.
    pub fn spawn(&mut self, elf: &[u8], argv: &[&str], envp: &[&str]) -> Result<u32, String> {
        let mut p = self.blank_proc(1);
        for i in 0..3 { p.fds.insert(i, Rc::new(RefCell::new(Ofd { kind: Kind::Tty, flags: 2, path: "/dev/tty".into() }))); }
        let pid = p.pid;
        self.procs.push(p);
        self.switch(self.procs.len() - 1, 0);
        let (argv, envp): (Vec<String>, Vec<String>) = (argv.iter().map(|s| s.to_string()).collect(), envp.iter().map(|s| s.to_string()).collect());
        self.exec_image(elf, &argv, &envp)?;
        Ok(pid)
    }

    /// Load an ELF executable as the first process.
    pub fn load(&mut self, elf: &[u8], argv: &[&str], envp: &[&str]) -> Result<(), String> { self.spawn(elf, argv, envp).map(|_| ()) }

    fn futex_wake(&mut self, addr: u32, n: u32) -> u32 {
        let mut k = 0;
        for t in self.procs[self.cur.0].threads.iter_mut() {
            if k < n && t.state == St::Futex(addr) { t.state = St::Run; t.wake_at = None; k += 1; }
        }
        k
    }

    fn thread_exit(&mut self, code: u32) {
        let ct = self.me().clear_tid;
        self.me().state = St::Dead;
        self.resched = true;
        if ct != 0 { let _ = self.mem.w32(ct, 0); self.futex_wake(ct, 1); }
        if self.procs[self.cur.0].threads.iter().all(|t| t.state == St::Dead) { self.proc_exit(code); }
    }

    fn proc_exit(&mut self, code: u32) {
        let pi = self.cur.0;
        for t in self.procs[pi].threads.iter_mut() { t.state = St::Dead; }
        let fds: Vec<u32> = self.fds.keys().copied().collect();
        for fd in fds { self.close(fd); }
        self.procs[pi].exit = Some(code);
        if self.procs[pi].ppid == 1 { self.procs[pi].reaped = true; }
        for p in self.procs.iter_mut() { for t in p.threads.iter_mut() { if t.state == St::Vfork(pi) { t.state = St::Run; } } }
        if pi == 0 { self.exit_code = Some(code); }
        self.resched = true;
    }

    /// fork / vfork / clone without CLONE_THREAD: a new process with a copy of the mapped memory.
    fn fork(&mut self, stack: u32, wait_exec: bool) -> u32 {
        let mut p = self.blank_proc(self.pid());
        for (&s, &e) in self.maps.iter() { p.mem.copy_pages_from(&self.mem, s, e - s); }
        if self.brk > self.brk_start { p.mem.copy_pages_from(&self.mem, self.brk_start, self.brk - self.brk_start); }
        p.fds = self.fds.clone(); p.cwd = self.cwd.clone(); p.brk = self.brk; p.brk_start = self.brk_start; p.maps = self.maps.clone();
        p.umask = self.umask; p.exe = self.exe.clone(); p.sigact = self.sigact.clone();
        let mut cpu = self.cpu.clone();
        cpu.r[EAX] = 0;
        if stack != 0 { cpu.r[ESP] = stack; }
        p.threads[0].cpu = cpu;
        let pid = p.pid;
        self.procs.push(p);
        if wait_exec { let ci = self.procs.len() - 1; self.me().state = St::Vfork(ci); self.resched = true; }
        pid
    }

    fn clone_call(&mut self, flags: u32, stack: u32, ptid: u32, tls: u32, ctid: u32) -> u32 {
        if flags & 0x10000 == 0 { // not CLONE_THREAD
            let pid = self.fork(stack, flags & 0x4000 != 0); // CLONE_VFORK
            if flags & 0x0010_0000 != 0 { let _ = self.mem.w32(ptid, pid); }
            if flags & 0x0100_0000 != 0 { let _ = self.procs.last_mut().unwrap().mem.w32(ctid, pid); }
            if flags & 0x0020_0000 != 0 { self.procs.last_mut().unwrap().threads[0].clear_tid = ctid; }
            return pid;
        }
        let tid = self.next_pid;
        self.next_pid += 1;
        let mut cpu = self.cpu.clone();
        cpu.r[EAX] = 0;
        cpu.r[ESP] = stack;
        if flags & 0x80000 != 0 { // CLONE_SETTLS
            let (entry, base) = (self.mem.r32(tls).unwrap_or(0), self.mem.r32(tls + 4).unwrap_or(0));
            if (6..=8).contains(&entry) {
                cpu.tls[(entry - 6) as usize] = base;
                if cpu.sel_gs >> 3 == entry { cpu.gs_base = base; }
                if cpu.sel_fs >> 3 == entry { cpu.fs_base = base; }
            }
        }
        if flags & 0x0010_0000 != 0 { let _ = self.mem.w32(ptid, tid); }
        if flags & 0x0100_0000 != 0 { let _ = self.mem.w32(ctid, tid); }
        let clear_tid = if flags & 0x0020_0000 != 0 { ctid } else { 0 };
        self.procs[self.cur.0].threads.push(Thread { cpu, tid, poll_deadline: None, clear_tid, state: St::Run, wake_at: None });
        tid
    }

    fn str_vec(&self, mut p: u32) -> Vec<String> {
        let mut v = Vec::new();
        while p != 0 { match self.mem.r32(p) { Ok(a) if a != 0 && v.len() < 4096 => v.push(self.cstr(a)), _ => break } p += 4; }
        v
    }

    fn execve(&mut self, path: u32, argv: u32, envp: u32) -> u32 {
        let raw = self.cstr(path);
        let p = self.resolve(-100i32 as u32, &raw, true);
        let data = match self.vfs.get(&p) { Some(Node::File(d)) => Some(d.borrow().clone()), _ => self.files.get(&p).cloned() };
        let Some(data) = data else { self.trace.push(format!("execve {p} -> ENOENT")); return ENOENT };
        if data.len() < 52 || data[0] != 0x7F { return -8i32 as u32; } // ENOEXEC
        let (mut argv, envp) = (self.str_vec(argv), self.str_vec(envp));
        if argv.is_empty() { argv.push(p.clone()); }
        self.trace.push(format!("pid {} execve {p} {argv:?}", self.pid()));
        let pi = self.cur.0;
        let ti = self.cur.1;
        for (i, t) in self.procs[pi].threads.iter_mut().enumerate() { if i != ti { t.state = St::Dead; } }
        for pr in self.procs.iter_mut() { for t in pr.threads.iter_mut() { if t.state == St::Vfork(pi) { t.state = St::Run; } } }
        self.me().clear_tid = 0;
        let retired = self.cpu.retired;
        self.mem = Mem::new_sparse(MEM_SIZE);
        self.cpu = Self::new_cpu();
        self.cpu.retired = retired;
        self.sigact.clear();
        self.exe = p;
        self.resched = true;
        if let Err(e) = self.exec_image(&data, &argv, &envp) { self.trace.push(format!("execve failed after the point of no return: {e}")); self.proc_exit(127); }
        0
    }

    // ---- address space ----

    fn mapped(&self, a: u32, e: u32) -> bool { self.maps.range(..e).next_back().map_or(false, |(_, &en)| en > a) }

    fn unmap(&mut self, a: u32, e: u32) {
        let hit: Vec<(u32, u32)> = self.maps.range(..e).filter(|(_, &en)| en > a).map(|(&s, &en)| (s, en)).collect();
        for (s, en) in hit {
            self.maps.remove(&s);
            if s < a { self.maps.insert(s, a); }
            if en > e { self.maps.insert(e, en); }
        }
    }

    /// Highest free gap of `len` bytes below the stack.
    fn find_gap(&self, len: u32) -> Option<u32> {
        let mut top = STACK_LOW;
        for (&s, &e) in self.maps.range(..STACK_LOW).rev() {
            if top >= e && top - e >= len { break; }
            top = top.min(s);
        }
        (top >= MMAP_MIN + len).then(|| top - len)
    }

    /// Create a directory in the guest file tree.
    pub fn mkdir(&mut self, path: &str) { self.vfs.insert(path.to_string(), Node::Dir); }

    /// Paths created in the guest file tree at run time.
    pub fn created(&self) -> Vec<String> { self.vfs.keys().cloned().collect() }

    /// Advance the guest clock (lets poll timeouts expire).
    pub fn advance(&mut self, ms: u64) { self.clock_ns += ms * 1_000_000; }

    /// Map an ELF image's PT_LOAD segments at `bias`. Returns (entry, phdr address, phentsize, phnum, end, interpreter path).
    fn map_elf(&mut self, elf: &[u8], bias: u32) -> Result<(u32, u32, u32, u32, u32, Option<String>), String> {
        if elf.len() < 52 || elf[0] != 0x7F || &elf[1..4] != b"ELF" || elf[4] != 1 || r16(elf, 18) != 3 { return Err("not an i386 ELF".into()); }
        let (entry, phoff, phentsize, phnum) = (r32(elf, 24), r32(elf, 28) as usize, r16(elf, 42) as usize, r16(elf, 44) as usize);
        let (mut end, mut phdr_va, mut interp) = (0u32, 0u32, None);
        for i in 0..phnum {
            let p = phoff + i * phentsize;
            let (ty, off, va, filesz, memsz) = (r32(elf, p), r32(elf, p + 4), r32(elf, p + 8) + bias, r32(elf, p + 16), r32(elf, p + 20));
            let src = elf.get(off as usize..(off + filesz) as usize).ok_or("ELF segment outside file")?;
            if ty == 3 { interp = Some(String::from_utf8_lossy(src).trim_end_matches(char::from(0)).to_string()); }
            if ty != 1 { continue; }
            let (lo, hi) = (va & !0xFFF, (va + memsz + 0xFFF) & !0xFFF);
            if hi > MEM_SIZE || hi <= lo { return Err("ELF segment outside guest memory".into()); }
            for pg in (lo..hi).step_by(4096) { if !self.mapped(pg, pg + 4096) { self.mem.map(pg, 4096, true); } } // segments may share a page
            self.unmap(lo, hi);
            self.maps.insert(lo, hi);
            self.mem.write_bytes(va, src).map_err(|_| "ELF segment outside guest memory")?;
            end = end.max(va + memsz);
            if off as usize <= phoff && phoff < (off + filesz) as usize { phdr_va = va + (phoff as u32 - off); }
        }
        Ok((entry + bias, phdr_va, phentsize as u32, phnum as u32, end, interp))
    }

    /// Replace the running process's image: map the ELF (and its interpreter, taken from `files`) and build the initial stack.
    fn exec_image(&mut self, elf: &[u8], argv: &[String], envp: &[String]) -> Result<(), String> {
        self.maps.clear();
        if self.exe.is_empty() { self.exe = argv.first().cloned().unwrap_or_default(); }
        if elf.len() < 52 { return Err("not an i386 ELF".into()); }
        let bias = if r16(elf, 16) == 3 { EXE_BIAS } else { 0 }; // ET_DYN (PIE)
        let (exe_entry, phdr_va, phentsize, phnum, end, interp) = self.map_elf(elf, bias)?;
        let (mut entry, mut at_base) = (exe_entry, 0);
        if let Some(path) = interp {
            let ld = self.files.get(&path).cloned().ok_or(format!("interpreter {path} not in files"))?;
            entry = self.map_elf(&ld, INTERP_BIAS)?.0;
            at_base = INTERP_BIAS;
        }
        self.brk = (end + 0xFFF) & !0xFFF;
        self.brk_start = self.brk;
        self.maps.insert(STACK_LOW, STACK_TOP);
        self.mem.map(STACK_LOW, STACK_TOP - STACK_LOW, true);
        // strings at the top of the stack, then the pointer vectors
        let mut sp = STACK_TOP;
        let mut push_str = |mem: &mut Mem, s: &[u8]| -> u32 { sp -= s.len() as u32 + 1; mem.write_bytes(sp, s).unwrap(); mem.w8(sp + s.len() as u32, 0).unwrap(); sp };
        let random = push_str(&mut self.mem, b"plonk-random-16b");
        let argp: Vec<u32> = argv.iter().map(|s| push_str(&mut self.mem, s.as_bytes())).collect();
        let envp: Vec<u32> = envp.iter().map(|s| push_str(&mut self.mem, s.as_bytes())).collect();
        let aux: [(u32, u32); 13] = [(3, phdr_va), (4, phentsize), (5, phnum), (6, 4096), (7, at_base), (8, 0), (9, exe_entry),
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

    /// Run the current thread for up to `steps` instructions. Returns (instructions used, whether it blocked).
    fn run_slice(&mut self, steps: u64) -> Result<(u64, bool), String> {
        let start = self.cpu.retired;
        self.blocked = false;
        self.resched = false;
        loop {
            let used = self.cpu.retired.saturating_sub(start);
            if used >= steps { break; }
            match self.cpu.run(&mut self.mem, steps - used) {
                Exit::Interrupt(0x80) => { self.syscall(); if self.blocked || self.resched { break; } }
                Exit::StepLimit => break,
                e => return Err(format!("pid {} ({}) cpu stopped: {e:?} at eip {:#x}", self.pid(), self.exe, self.cpu.eip)),
            }
        }
        Ok((self.cpu.retired.saturating_sub(start), self.blocked))
    }

    fn next_deadline(&self) -> Option<u64> {
        let mut d = self.poll_deadline;
        for (pi, p) in self.procs.iter().enumerate() {
            for (ti, t) in p.threads.iter().enumerate() {
                if t.state == St::Dead { continue; }
                let pd = if (pi, ti) == self.cur { None } else { t.poll_deadline };
                for x in [pd, t.wake_at].into_iter().flatten() { d = Some(d.map_or(x, |y: u64| y.min(x))); }
            }
        }
        d
    }

    /// Run all processes until the first one exits, every thread is waiting, or about `max_steps` instructions were executed.
    /// Returns the first process's exit code if it ended; `blocked` tells whether everything is waiting.
    pub fn run(&mut self, max_steps: u64) -> Result<Option<u32>, String> {
        let mut budget = max_steps;
        loop {
            let (mut progress, mut alive) = (false, false);
            for pi in 0..self.procs.len() {
                for ti in 0..self.procs[pi].threads.len() {
                    if self.exit_code.is_some() { return Ok(self.exit_code); }
                    let t = &mut self.procs[pi].threads[ti];
                    match t.state {
                        St::Dead => continue,
                        St::Vfork(_) => { alive = true; continue; }
                        St::Futex(_) => {
                            alive = true;
                            if t.wake_at.map_or(true, |d| self.clock_ns < d) { continue; }
                            t.state = St::Run;
                            t.wake_at = None;
                            if (pi, ti) == self.cur { self.cpu.r[EAX] = ETIMEDOUT; } else { t.cpu.r[EAX] = ETIMEDOUT; }
                        }
                        St::Run => {}
                    }
                    alive = true;
                    self.switch(pi, ti);
                    let (used, blocked) = self.run_slice(SLICE.min(budget))?;
                    if !(blocked && used <= 2) { progress = true; }
                    budget = budget.saturating_sub(used.max(1));
                    if budget == 0 { self.blocked = false; return Ok(self.exit_code); }
                }
            }
            if self.exit_code.is_some() || !alive { return Ok(self.exit_code); }
            if !progress {
                match self.next_deadline() {
                    Some(d) if self.auto_time => { self.clock_ns = self.clock_ns.max(d); }
                    _ => { self.blocked = true; return Ok(None); }
                }
            }
        }
    }

    fn cstr(&self, a: u32) -> String {
        let mut v = Vec::new();
        let mut p = a;
        while let Ok(b) = self.mem.r8(p) { if b == 0 || v.len() > 4096 { break; } v.push(b); p += 1; }
        String::from_utf8_lossy(&v).into_owned()
    }

    // ---- file tree ----

    /// Absolute, normalized path (symlinks in the tree are followed; `follow` controls the last component).
    fn resolve(&self, dirfd: u32, path: &str, follow: bool) -> String {
        let base = if path.starts_with('/') { String::new() } else if dirfd == -100i32 as u32 { self.cwd.clone() } else { self.fds.get(&dirfd).map(|f| f.borrow().path.clone()).unwrap_or_else(|| self.cwd.clone()) };
        let mut todo: Vec<String> = format!("{base}/{path}").split('/').rev().map(|s| s.to_string()).collect();
        let mut out: Vec<String> = Vec::new();
        let mut hops = 0;
        while let Some(c) = todo.pop() {
            match c.as_str() {
                "" | "." => {}
                ".." => { out.pop(); }
                _ => {
                    out.push(c);
                    let cur = format!("/{}", out.join("/"));
                    if let Some(Node::Link(t)) = self.vfs.get(&cur) {
                        if (follow || !todo.is_empty()) && hops < 16 {
                            hops += 1;
                            out.pop();
                            if t.starts_with('/') { out.clear(); }
                            todo.extend(t.split('/').rev().map(|s| s.to_string()));
                        }
                    }
                }
            }
        }
        format!("/{}", out.join("/"))
    }

    fn is_dir(&self, p: &str) -> bool {
        if matches!(self.vfs.get(p), Some(Node::Dir)) { return true; }
        let pre = format!("{}/", p.trim_end_matches('/'));
        self.files.keys().any(|k| k.starts_with(&pre))
    }

    /// (mode, size) of a path, if it exists.
    fn lookup(&self, p: &str) -> Option<(u32, u64)> {
        match self.vfs.get(p) {
            Some(Node::File(d)) => Some((0o100644, d.borrow().len() as u64)),
            Some(Node::Dir) => Some((0o040700, 0)),
            Some(Node::Sock(_)) => Some((0o140755, 0)),
            Some(Node::Link(t)) => Some((0o120777, t.len() as u64)),
            None => match p {
                "/dev/null" | "/dev/tty" => Some((0o020666, 0)),
                _ => self.files.get(p).map(|d| (0o100755, d.len() as u64)).or_else(|| self.is_dir(p).then_some((0o040755, 0))),
            },
        }
    }

    fn children(&self, dir: &str) -> Vec<(String, u8)> {
        let pre = format!("{}/", dir.trim_end_matches('/'));
        let mut v: BTreeMap<String, u8> = BTreeMap::new();
        for (k, n) in self.vfs.range(pre.clone()..).take_while(|(k, _)| k.starts_with(&pre)) {
            let rest = &k[pre.len()..];
            if rest.is_empty() || rest.contains('/') { continue; }
            v.insert(rest.to_string(), match n { Node::Dir => 4, Node::File(_) => 8, Node::Link(_) => 10, Node::Sock(_) => 12 });
        }
        for k in self.files.keys().filter(|k| k.starts_with(&pre)) {
            let rest = &k[pre.len()..];
            match rest.split_once('/') { Some((d, _)) => { v.entry(d.to_string()).or_insert(4); } None => { v.entry(rest.to_string()).or_insert(8); } }
        }
        v.into_iter().collect()
    }

    fn new_fd(&mut self, o: Fd, min: u32) -> u32 {
        let mut fd = min;
        while self.fds.contains_key(&fd) { fd += 1; }
        self.fds.insert(fd, o);
        fd
    }

    fn open(&mut self, dirfd: u32, path: u32, flags: u32) -> u32 {
        let raw = self.cstr(path);
        let p = self.resolve(dirfd, &raw, flags & 0x20000 == 0); // O_NOFOLLOW
        let kind = match self.vfs.get(&p) {
            Some(Node::File(d)) => {
                if flags & 0xC0 == 0xC0 { return EEXIST; } // O_CREAT|O_EXCL
                if flags & 0x200 != 0 { d.borrow_mut().clear(); } // O_TRUNC
                Kind::File { data: d.clone(), pos: 0 }
            }
            Some(Node::Dir) => Kind::Dir { pos: 0 },
            Some(_) => return -6i32 as u32, // ENXIO
            None => match p.as_str() {
                "/dev/null" => Kind::Null,
                "/dev/tty" => Kind::Tty,
                _ => {
                    if let Some(d) = self.files.get(&p) {
                        if flags & 3 != 0 { // opened for writing: copy up into the writable tree
                            let data: Data = Rc::new(RefCell::new(if flags & 0x200 != 0 { Vec::new() } else { d.clone() }));
                            self.vfs.insert(p.clone(), Node::File(data.clone()));
                            Kind::File { data, pos: 0 }
                        } else { Kind::File { data: Rc::new(RefCell::new(d.clone())), pos: 0 } }
                    } else if self.is_dir(&p) { Kind::Dir { pos: 0 } }
                    else if flags & 0x40 != 0 && self.is_dir(parent(&p)) { // O_CREAT
                        let data: Data = Rc::new(RefCell::new(Vec::new()));
                        self.vfs.insert(p.clone(), Node::File(data.clone()));
                        Kind::File { data, pos: 0 }
                    } else { self.trace.push(format!("open {p} -> ENOENT")); return ENOENT; }
                }
            },
        };
        if flags & 0x10000 != 0 && !matches!(kind, Kind::Dir { .. }) { return ENOTDIR; } // O_DIRECTORY
        self.new_fd(Rc::new(RefCell::new(Ofd { kind, flags, path: p })), 0)
    }

    fn close(&mut self, fd: u32) -> u32 {
        let Some(o) = self.fds.remove(&fd) else { return EBADF };
        if Rc::strong_count(&o) == 1 { if let Kind::Sock(s) = o.borrow().kind { self.socks[s].closed = true; self.socks[s].rx.clear(); } }
        0
    }

    fn fill_stat(&mut self, buf: u32, path: &str, mode: u32, size: u64) -> u32 {
        let mut s = [0u8; 96];
        let mut put = |o: usize, v: u32| s[o..o + 4].copy_from_slice(&v.to_le_bytes());
        let i = ino(path);
        put(0, 0x801); put(12, i); put(16, mode); put(20, 1); put(24, 1000); put(28, 1000);
        put(44, size as u32); put(52, 4096); put(56, ((size + 511) / 512) as u32);
        let t = (self.clock_ns / 1_000_000_000) as u32;
        put(64, t); put(72, t); put(80, t); put(88, i);
        if self.mem.write_bytes(buf, &s).is_ok() { 0 } else { EFAULT }
    }

    fn stat_path(&mut self, dirfd: u32, path: u32, buf: u32, follow: bool) -> u32 {
        let raw = self.cstr(path);
        let p = self.resolve(dirfd, &raw, follow);
        match self.lookup(&p) { Some((mode, size)) => self.fill_stat(buf, &p, mode, size), None => ENOENT }
    }

    fn fstat(&mut self, fd: u32, buf: u32) -> u32 {
        let Some(o) = self.fds.get(&fd).cloned() else { return EBADF };
        let o = o.borrow();
        let (mode, size) = match &o.kind {
            Kind::File { data, .. } => (0o100644, data.borrow().len() as u64),
            Kind::Dir { .. } => (0o040700, 0),
            Kind::Null | Kind::Tty => (0o020620, 0),
            Kind::Sock(_) => (0o140777, 0),
        };
        let path = if let Kind::Sock(s) = o.kind { format!("socket:{s}") } else { o.path.clone() };
        self.fill_stat(buf, &path, mode, size)
    }

    // ---- sockets and pipes ----

    fn new_sock(&mut self) -> usize { self.socks.push(Sock::default()); self.socks.len() - 1 }
    fn sock_fd(&mut self, s: usize, flags: u32) -> u32 { self.new_fd(Rc::new(RefCell::new(Ofd { kind: Kind::Sock(s), flags, path: String::new() })), 0) }
    fn sock_of(&self, fd: u32) -> Result<(usize, bool), u32> {
        let o = self.fds.get(&fd).ok_or(EBADF)?.borrow();
        match o.kind { Kind::Sock(s) => Ok((s, o.flags & O_NONBLOCK != 0)), _ => Err(ENOTSOCK) }
    }

    fn send(&mut self, s: usize, data: Vec<u8>, fds: Vec<Fd>) -> u32 {
        let Some(peer) = self.socks[s].peer else { return EPIPE };
        if self.socks[peer].closed || self.socks[s].shut_wr { return EPIPE; }
        let n = data.len() as u32;
        self.socks[peer].rx.push_back(Msg { data, off: 0, fds });
        n
    }

    /// Read up to `n` bytes: Some((bytes, passed descriptors)), or None when it would block.
    fn recv(&mut self, s: usize, n: usize) -> Option<(Vec<u8>, Vec<Fd>)> {
        if self.socks[s].rx.is_empty() {
            let eof = self.socks[s].peer.map_or(true, |p| self.socks[p].closed || self.socks[p].shut_wr);
            return eof.then(|| (Vec::new(), Vec::new()));
        }
        let sk = &mut self.socks[s];
        let m = sk.rx.front_mut().unwrap();
        let k = n.min(m.data.len() - m.off);
        let out = m.data[m.off..m.off + k].to_vec();
        let fds = std::mem::take(&mut m.fds);
        m.off += k;
        if m.off >= m.data.len() { sk.rx.pop_front(); }
        Some((out, fds))
    }

    fn poll_one(&self, fd: u32, events: u16) -> u16 {
        let Some(o) = self.fds.get(&fd) else { return POLLNVAL };
        let o = o.borrow();
        match o.kind {
            Kind::Sock(s) => {
                let sk = &self.socks[s];
                let peer_gone = sk.peer.map_or(false, |p| self.socks[p].closed);
                let peer_eof = sk.peer.map_or(false, |p| self.socks[p].shut_wr);
                let mut r = 0;
                if !sk.rx.is_empty() || !sk.backlog.is_empty() || peer_eof || peer_gone { r |= POLLIN; }
                if sk.peer.is_some() && !peer_gone && !sk.shut_wr { r |= POLLOUT; }
                r &= events;
                if peer_gone || (peer_eof && sk.shut_wr) { r |= POLLHUP; }
                r
            }
            Kind::Tty => if fd == 0 { 0 } else { events & POLLOUT },
            _ => events & (POLLIN | POLLOUT),
        }
    }

    fn sockaddr_path(&self, addr: u32, len: u32) -> String {
        let b = self.mem.read_bytes(addr + 2, len.saturating_sub(2).min(108)).unwrap_or(&[]);
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        self.resolve(-100i32 as u32, &String::from_utf8_lossy(&b[..end]), true)
    }

    /// The socket calls, numbered as in socketcall(2). None = would block.
    fn sock_call(&mut self, call: u32, a: [u32; 6]) -> Option<u32> {
        let w32 = |m: &mut Mem, p: u32, v: u32| { if p != 0 { let _ = m.w32(p, v); } };
        Some(match call {
            1 => { let s = self.new_sock(); self.sock_fd(s, 2 | (a[1] & O_NONBLOCK)) } // socket
            2 => { // bind
                let (s, _) = match self.sock_of(a[0]) { Ok(v) => v, Err(e) => return Some(e) };
                let p = self.sockaddr_path(a[1], a[2]);
                if self.lookup(&p).is_some() { return Some(-98i32 as u32); } // EADDRINUSE
                self.vfs.insert(p, Node::Sock(s));
                0
            }
            3 => { // connect
                let (s, _) = match self.sock_of(a[0]) { Ok(v) => v, Err(e) => return Some(e) };
                let p = self.sockaddr_path(a[1], a[2]);
                let Some(&Node::Sock(l)) = self.vfs.get(&p) else { return Some(ENOENT) };
                if !self.socks[l].listening || self.socks[l].closed { return Some(ECONNREFUSED); }
                let srv = self.new_sock();
                self.socks[srv].peer = Some(s);
                self.socks[s].peer = Some(srv);
                self.socks[l].backlog.push_back(srv);
                0
            }
            4 => match self.sock_of(a[0]) { Ok((s, _)) => { self.socks[s].listening = true; 0 } Err(e) => e }, // listen
            5 | 18 => { // accept, accept4
                let (s, nb) = match self.sock_of(a[0]) { Ok(v) => v, Err(e) => return Some(e) };
                let Some(c) = self.socks[s].backlog.pop_front() else { return if nb { Some(EAGAIN) } else { None } };
                w32(&mut self.mem, a[2], 2);
                if a[1] != 0 { let _ = self.mem.w16(a[1], 1); }
                self.sock_fd(c, 2 | if call == 18 { a[3] & O_NONBLOCK } else { 0 })
            }
            6 | 7 => { if a[1] != 0 { let _ = self.mem.w16(a[1], 1); } w32(&mut self.mem, a[2], 2); 0 } // getsockname, getpeername
            8 => { // socketpair
                let (x, y) = (self.new_sock(), self.new_sock());
                self.socks[x].peer = Some(y);
                self.socks[y].peer = Some(x);
                let fl = 2 | (a[1] & O_NONBLOCK);
                let (fx, fy) = (self.sock_fd(x, fl), self.sock_fd(y, fl));
                let _ = self.mem.w32(a[3], fx);
                let _ = self.mem.w32(a[3] + 4, fy);
                0
            }
            9 | 11 => { // send, sendto
                let (s, _) = match self.sock_of(a[0]) { Ok(v) => v, Err(e) => return Some(e) };
                let Ok(d) = self.mem.read_bytes(a[1], a[2]).map(|b| b.to_vec()) else { return Some(EFAULT) };
                self.send(s, d, Vec::new())
            }
            10 | 12 => { // recv, recvfrom
                let (s, nb) = match self.sock_of(a[0]) { Ok(v) => v, Err(e) => return Some(e) };
                let Some((d, _)) = self.recv(s, a[2] as usize) else { return if nb || a[3] & 0x40 != 0 { Some(EAGAIN) } else { None } };
                if self.mem.write_bytes(a[1], &d).is_ok() { d.len() as u32 } else { EFAULT }
            }
            13 => match self.sock_of(a[0]) { Ok((s, _)) => { if a[1] != 0 { self.socks[s].shut_wr = true; } 0 } Err(e) => e }, // shutdown
            14 => 0, // setsockopt
            15 => { // getsockopt: SO_PEERCRED gives {pid, uid, gid}, everything else 0
                if a[1] == 1 && a[2] == 17 { for (i, v) in [1000u32, 1000, 1000].iter().enumerate() { let _ = self.mem.w32(a[3] + i as u32 * 4, *v); } w32(&mut self.mem, a[4], 12); }
                else { w32(&mut self.mem, a[3], 0); w32(&mut self.mem, a[4], 4); }
                0
            }
            16 => { // sendmsg(fd, msghdr, flags)
                let (s, _) = match self.sock_of(a[0]) { Ok(v) => v, Err(e) => return Some(e) };
                let g = |m: &Mem, i: u32| m.r32(a[1] + i * 4).unwrap_or(0);
                let (iov, iovlen, ctl, ctllen) = (g(&self.mem, 2), g(&self.mem, 3), g(&self.mem, 4), g(&self.mem, 5));
                let mut data = Vec::new();
                for i in 0..iovlen {
                    let (b, l) = (self.mem.r32(iov + i * 8).unwrap_or(0), self.mem.r32(iov + i * 8 + 4).unwrap_or(0));
                    match self.mem.read_bytes(b, l) { Ok(x) => data.extend_from_slice(x), Err(_) => return Some(EFAULT) }
                }
                let mut fds = Vec::new();
                let mut c = ctl;
                while ctl != 0 && c + 12 <= ctl + ctllen {
                    let (len, level, ty) = (self.mem.r32(c).unwrap_or(0), self.mem.r32(c + 4).unwrap_or(0), self.mem.r32(c + 8).unwrap_or(0));
                    if len < 12 { break; }
                    if level == 1 && ty == 1 { // SOL_SOCKET, SCM_RIGHTS
                        for i in 0..(len - 12) / 4 { if let Some(o) = self.fds.get(&self.mem.r32(c + 12 + i * 4).unwrap_or(u32::MAX)) { fds.push(o.clone()); } }
                    }
                    c += (len + 3) & !3;
                }
                self.send(s, data, fds)
            }
            17 => { // recvmsg(fd, msghdr, flags)
                let (s, nb) = match self.sock_of(a[0]) { Ok(v) => v, Err(e) => return Some(e) };
                let g = |m: &Mem, i: u32| m.r32(a[1] + i * 4).unwrap_or(0);
                let (iov, iovlen, ctl, ctllen) = (g(&self.mem, 2), g(&self.mem, 3), g(&self.mem, 4), g(&self.mem, 5));
                let total: u32 = (0..iovlen).map(|i| self.mem.r32(iov + i * 8 + 4).unwrap_or(0)).sum();
                let Some((d, fds)) = self.recv(s, total as usize) else { return if nb || a[2] & 0x40 != 0 { Some(EAGAIN) } else { None } };
                let mut off = 0usize;
                for i in 0..iovlen {
                    let (b, l) = (self.mem.r32(iov + i * 8).unwrap_or(0), self.mem.r32(iov + i * 8 + 4).unwrap_or(0) as usize);
                    let k = l.min(d.len() - off);
                    let _ = self.mem.write_bytes(b, &d[off..off + k]);
                    off += k;
                }
                let room = (ctllen.saturating_sub(12) / 4) as usize;
                let n = fds.len().min(room);
                if ctl != 0 && n > 0 {
                    let _ = self.mem.w32(ctl, 12 + 4 * n as u32);
                    let _ = self.mem.w32(ctl + 4, 1);
                    let _ = self.mem.w32(ctl + 8, 1);
                    for (i, o) in fds.into_iter().take(n).enumerate() { let fd = self.new_fd(o, 0); let _ = self.mem.w32(ctl + 12 + i as u32 * 4, fd); }
                    let _ = self.mem.w32(a[1] + 20, 12 + 4 * n as u32);
                } else { let _ = self.mem.w32(a[1] + 20, 0); }
                let _ = self.mem.w32(a[1] + 24, 0);
                d.len() as u32
            }
            _ => ENOSYS,
        })
    }

    // ---- read / write ----

    fn read(&mut self, fd: u32, buf: u32, n: u32, at: Option<u64>) -> Option<u32> {
        let Some(o) = self.fds.get(&fd).cloned() else { return Some(EBADF) };
        let mut o = o.borrow_mut();
        let nb = o.flags & O_NONBLOCK != 0;
        let sock = match &mut o.kind {
            Kind::File { data, pos } => {
                let d = data.borrow();
                let s = (at.unwrap_or(*pos) as usize).min(d.len());
                let k = (n as usize).min(d.len() - s);
                if self.mem.write_bytes(buf, &d[s..s + k]).is_err() { return Some(EFAULT); }
                if at.is_none() { *pos += k as u64; }
                return Some(k as u32);
            }
            Kind::Sock(s) => *s,
            Kind::Dir { .. } => return Some(-21i32 as u32), // EISDIR
            Kind::Null | Kind::Tty => return Some(0),
        };
        drop(o);
        let Some((d, _)) = self.recv(sock, n as usize) else { return if nb { Some(EAGAIN) } else { None } };
        Some(if self.mem.write_bytes(buf, &d).is_ok() { d.len() as u32 } else { EFAULT })
    }

    fn write(&mut self, fd: u32, buf: u32, n: u32, at: Option<u64>) -> u32 {
        let Some(o) = self.fds.get(&fd).cloned() else { return EBADF };
        let Ok(src) = self.mem.read_bytes(buf, n).map(|b| b.to_vec()) else { return EFAULT };
        let mut o = o.borrow_mut();
        let append = o.flags & 0x400 != 0;
        let sock = match &mut o.kind {
            Kind::File { data, pos } => {
                let mut d = data.borrow_mut();
                let s = if append { d.len() } else { at.unwrap_or(*pos) as usize };
                if d.len() < s + src.len() { d.resize(s + src.len(), 0); }
                d[s..s + src.len()].copy_from_slice(&src);
                if at.is_none() { *pos = (s + src.len()) as u64; }
                return n;
            }
            Kind::Tty => { self.stdout.extend_from_slice(&src); return n; }
            Kind::Null => return n,
            Kind::Sock(s) => *s,
            Kind::Dir { .. } => return EBADF,
        };
        drop(o);
        self.send(sock, src, Vec::new())
    }

    fn mmap(&mut self, addr: u32, len: u32, prot: u32, flags: u32, fd: u32, off: u64) -> u32 {
        let len = (len + 0xFFF) & !0xFFF;
        if len == 0 { return EINVAL; }
        let fits = |a: u32| a >= 0x1000 && a & 0xFFF == 0 && a.checked_add(len).map_or(false, |e| e <= MEM_SIZE);
        let a = if flags & 0x10 != 0 { // MAP_FIXED
            if !fits(addr) { return ENOMEM; }
            self.unmap(addr, addr + len);
            addr
        } else if flags & 0x10_0000 != 0 { // MAP_FIXED_NOREPLACE
            if !fits(addr) { return ENOMEM; }
            if self.mapped(addr, addr + len) { return EEXIST; }
            addr
        } else if fits(addr) && !self.mapped(addr, addr + len) { addr } else {
            match self.find_gap(len) { Some(a) => a, None => return ENOMEM }
        };
        self.maps.insert(a, a + len);
        self.mem.map(a, len, prot != 0 || flags & 0x20 == 0);
        if flags & 0x20 == 0 { // file-backed (a private copy; shared writable mappings are not written back yet)
            let Some(o) = self.fds.get(&fd).cloned() else { return EBADF };
            let o = o.borrow();
            if let Kind::File { data, .. } = &o.kind {
                let d = data.borrow();
                let s = (off as usize).min(d.len());
                let e = (s + len as usize).min(d.len());
                let _ = self.mem.write_bytes(a, &d[s..e]);
            }
        }
        a
    }

    fn now(&mut self) -> (u64, u32) {
        self.clock_ns += 20_000;
        (self.clock_ns / 1_000_000_000, (self.clock_ns % 1_000_000_000) as u32)
    }

    fn poll(&mut self, fds: u32, n: u32, timeout_ms: i64) -> Option<u32> {
        let mut ready = 0;
        for i in 0..n {
            let p = fds + i * 8;
            let (fd, ev) = (self.mem.r32(p).unwrap_or(u32::MAX), self.mem.r16(p + 4).unwrap_or(0));
            let r = if (fd as i32) < 0 { 0 } else { self.poll_one(fd, ev) };
            let _ = self.mem.w16(p + 6, r);
            if r != 0 { ready += 1; }
        }
        if ready > 0 || timeout_ms == 0 { self.poll_deadline = None; return Some(ready); }
        if timeout_ms > 0 {
            let dl = *self.poll_deadline.get_or_insert(self.clock_ns + timeout_ms as u64 * 1_000_000);
            if self.clock_ns >= dl { self.poll_deadline = None; return Some(0); }
        }
        None
    }

    fn syscall(&mut self) {
        let r = self.cpu.r;
        let (nr, a, b, c, d, e, f) = (r[EAX], r[EBX], r[ECX], r[EDX], r[ESI], r[EDI], r[EBP]);
        const CWD: u32 = -100i32 as u32;
        let ret: Option<u32> = match nr {
            1 => { self.thread_exit(a); Some(0) } // exit (this thread)
            252 => { self.proc_exit(a); Some(0) } // exit_group
            2 => Some(self.fork(0, false)), // fork
            190 => Some(self.fork(0, true)), // vfork
            120 => Some(self.clone_call(a, b, c, d, e)), // clone(flags, stack, ptid, tls, ctid)
            11 => { let r = self.execve(a, b, c); if r == 0 { return; } Some(r) }
            7 | 114 => { // waitpid, wait4(pid, status, options)
                let me = self.pid();
                let want = a as i32;
                let kids: Vec<usize> = (0..self.procs.len()).filter(|&i| self.procs[i].ppid == me && !self.procs[i].reaped && (want <= 0 || self.procs[i].pid == a)).collect();
                match kids.iter().find(|&&i| self.procs[i].exit.is_some()) {
                    Some(&i) => {
                        self.procs[i].reaped = true;
                        if b != 0 { let _ = self.mem.w32(b, (self.procs[i].exit.unwrap() & 0xFF) << 8); }
                        Some(self.procs[i].pid)
                    }
                    None if kids.is_empty() => Some(ECHILD),
                    None if c & 1 != 0 => Some(0), // WNOHANG
                    None => None,
                }
            }
            158 => { self.resched = true; Some(0) } // sched_yield
            3 => self.read(a, b, c, None),
            4 => Some(self.write(a, b, c, None)),
            5 => Some(self.open(CWD, a, b)),
            295 => Some(self.open(a, b, c)), // openat
            6 => Some(self.close(a)),
            10 | 40 | 301 => { // unlink, rmdir, unlinkat
                let p = if nr == 301 { let s = self.cstr(b); self.resolve(a, &s, false) } else { let s = self.cstr(a); self.resolve(CWD, &s, false) };
                Some(if self.vfs.remove(&p).is_some() { 0 } else { ENOENT })
            }
            12 => { let s = self.cstr(a); let p = self.resolve(CWD, &s, true); Some(if self.is_dir(&p) { self.cwd = p; 0 } else { ENOENT }) } // chdir
            133 => Some(match self.fds.get(&a).map(|o| o.borrow().path.clone()) { Some(p) => { self.cwd = p; 0 } None => EBADF }), // fchdir
            13 => { let (s, _) = self.now(); if a != 0 { let _ = self.mem.w32(a, s as u32); } Some(s as u32) } // time
            15 | 94 | 306 | 207 | 212 | 198 => Some(0), // chmod, fchmod, fchmodat, fchown32, chown32, lchown32
            20 => Some(self.pid()),
            224 => Some(self.me().tid), // gettid
            64 => Some(self.procs[self.cur.0].ppid), // getppid
            33 | 307 => { // access, faccessat
                let p = if nr == 33 { let s = self.cstr(a); self.resolve(CWD, &s, true) } else { let s = self.cstr(b); self.resolve(a, &s, true) };
                Some(if self.lookup(&p).is_some() { 0 } else { ENOENT })
            }
            38 | 302 => { // rename, renameat
                let (from, to) = if nr == 38 { (self.cstr(a), self.cstr(b)) } else { (self.cstr(b), self.cstr(d)) };
                let (from, to) = (self.resolve(CWD, &from, false), self.resolve(CWD, &to, false));
                Some(match self.vfs.remove(&from) { Some(n) => { self.vfs.insert(to, n); 0 } None => ENOENT })
            }
            39 | 296 => { // mkdir, mkdirat
                let p = if nr == 39 { let s = self.cstr(a); self.resolve(CWD, &s, false) } else { let s = self.cstr(b); self.resolve(a, &s, false) };
                Some(if self.lookup(&p).is_some() { EEXIST } else if !self.is_dir(parent(&p)) { ENOENT } else { self.vfs.insert(p, Node::Dir); 0 })
            }
            41 => Some(match self.fds.get(&a).cloned() { Some(o) => self.new_fd(o, 0), None => EBADF }), // dup
            63 | 330 => Some(match self.fds.get(&a).cloned() { Some(o) => { if a != b { self.close(b); self.fds.insert(b, o); } b } None => EBADF }), // dup2, dup3
            42 | 331 => { // pipe, pipe2: a one-way socket pair
                let (x, y) = (self.new_sock(), self.new_sock());
                self.socks[x].peer = Some(y);
                self.socks[y].peer = Some(x);
                let fl = if nr == 331 { b & O_NONBLOCK } else { 0 };
                let (fx, fy) = (self.sock_fd(x, fl), self.sock_fd(y, fl | 1));
                let _ = self.mem.w32(a, fx);
                let _ = self.mem.w32(a + 4, fy);
                Some(0)
            }
            45 => { // brk
                if a >= self.brk_start && a < self.brk_start + 0x0800_0000 && (a <= self.brk || !self.mapped(self.brk, a)) {
                    let (lo, hi) = ((self.brk + 0xFFF) & !0xFFF, (a + 0xFFF) & !0xFFF);
                    if hi > lo { self.mem.map(lo, hi - lo, true); }
                    self.brk = a;
                }
                Some(self.brk)
            }
            54 => Some(ENOTTY), // ioctl
            60 => { let old = self.umask; self.umask = a & 0o777; Some(old) } // umask
            66 | 57 => Some(0), // setsid, setpgid
            65 | 132 => Some(1000), // getpgrp, getpgid
            78 => { // gettimeofday
                let (s, ns) = self.now();
                if a != 0 { let _ = self.mem.w32(a, s as u32); let _ = self.mem.w32(a + 4, ns / 1000); }
                Some(0)
            }
            83 | 304 => { // symlink, symlinkat
                let (target, link) = if nr == 83 { (self.cstr(a), self.cstr(b)) } else { (self.cstr(a), self.cstr(c)) };
                let p = self.resolve(if nr == 83 { CWD } else { b }, &link, false);
                Some(if self.lookup(&p).is_some() { EEXIST } else { self.vfs.insert(p, Node::Link(target)); 0 })
            }
            85 | 305 => { // readlink, readlinkat
                let (raw, buf, n) = if nr == 85 { (self.cstr(a), b, c) } else { (self.cstr(b), c, d) };
                let p = self.resolve(if nr == 85 { CWD } else { a }, &raw, false);
                let t = match self.vfs.get(&p) { Some(Node::Link(t)) => Some(t.clone()), _ if p == "/proc/self/exe" => Some(self.exe.clone()), _ => None };
                Some(match t { Some(t) => { let k = t.len().min(n as usize); let _ = self.mem.write_bytes(buf, &t.as_bytes()[..k]); k as u32 } None => EINVAL })
            }
            91 => { // munmap
                let e = a.saturating_add((b + 0xFFF) & !0xFFF).min(MEM_SIZE);
                if a & 0xFFF == 0 && e > a { self.unmap(a, e); self.mem.map(a, e - a, false); }
                Some(0)
            }
            125 => { self.mem.protect(a & !0xFFF, (b + (a & 0xFFF) + 0xFFF) & !0xFFF, c != 0); Some(0) } // mprotect
            219 | 118 | 148 | 36 => Some(0), // madvise, fsync, fdatasync, sync
            90 => { // old mmap: args in a struct
                let g = |i: u32| self.mem.r32(a + i * 4).unwrap_or(0);
                let (addr, len, prot, flags, fd, off) = (g(0), g(1), g(2), g(3), g(4), g(5));
                Some(self.mmap(addr, len, prot, flags, fd, off as u64))
            }
            192 => Some(self.mmap(a, b, c, d, e, (f as u64) << 12)), // mmap2
            93 | 194 => { // ftruncate, ftruncate64
                Some(match self.fds.get(&a).cloned() {
                    Some(o) => { if let Kind::File { data, .. } = &o.borrow().kind { data.borrow_mut().resize(b as usize, 0); 0 } else { EINVAL } }
                    None => EBADF,
                })
            }
            102 => { // socketcall(call, args)
                let mut args = [0u32; 6];
                for (i, x) in args.iter_mut().enumerate() { *x = self.mem.r32(b + i as u32 * 4).unwrap_or(0); }
                self.sock_call(a, args)
            }
            359..=373 => { // the direct socket syscalls, mapped onto socketcall numbers
                let call = [1, 8, 2, 3, 4, 18, 15, 14, 6, 7, 11, 16, 12, 17, 13][(nr - 359) as usize];
                self.sock_call(call, [a, b, c, d, e, f])
            }
            122 => { // uname: six 65-byte fields
                let _ = self.mem.write_bytes(a, &[0u8; 390]);
                for (i, s) in ["Linux", "plonk", "6.1.0", "#1", "i686", "(none)"].iter().enumerate() { let _ = self.mem.write_bytes(a + i as u32 * 65, s.as_bytes()); }
                Some(0)
            }
            140 => Some(match self.fds.get(&a).cloned() { // _llseek(fd, hi, lo, result, whence)
                Some(o) => {
                    let mut o = o.borrow_mut();
                    match &mut o.kind {
                        Kind::File { data, pos } => {
                            let off = (((b as u64) << 32) | c as u64) as i64;
                            let base = match e { 0 => 0, 1 => *pos as i64, _ => data.borrow().len() as i64 };
                            *pos = (base + off).max(0) as u64;
                            let _ = self.mem.w32(d, *pos as u32);
                            let _ = self.mem.w32(d + 4, (*pos >> 32) as u32);
                            0
                        }
                        Kind::Dir { pos } => { *pos = 0; let _ = self.mem.w32(d, 0); let _ = self.mem.w32(d + 4, 0); 0 }
                        _ => -29i32 as u32, // ESPIPE
                    }
                }
                None => EBADF,
            }),
            19 => Some(match self.fds.get(&a).cloned() { // lseek
                Some(o) => {
                    let mut o = o.borrow_mut();
                    match &mut o.kind {
                        Kind::File { data, pos } => { let base = match c { 0 => 0, 1 => *pos as i64, _ => data.borrow().len() as i64 }; *pos = (base + b as i32 as i64).max(0) as u64; *pos as u32 }
                        Kind::Dir { pos } => { *pos = 0; 0 }
                        _ => -29i32 as u32,
                    }
                }
                None => EBADF,
            }),
            145 | 146 => { // readv, writev
                let mut total = Some(0u32);
                for i in 0..c {
                    let (base, len) = (self.mem.r32(b + i * 8).unwrap_or(0), self.mem.r32(b + i * 8 + 4).unwrap_or(0));
                    let w = if nr == 146 { Some(self.write(a, base, len, None)) } else { self.read(a, base, len, None) };
                    match w {
                        Some(w) if (w as i32) >= 0 => { total = total.map(|t| t + w); if w < len { break; } }
                        other => { if total == Some(0) { total = other; } break; }
                    }
                }
                total
            }
            162 => { // nanosleep
                let ms = self.mem.r32(a).unwrap_or(0) as i64 * 1000 + (self.mem.r32(a + 4).unwrap_or(0) / 1_000_000) as i64;
                self.poll(0, 0, ms.max(1))
            }
            168 => self.poll(a, b, c as i32 as i64), // poll
            309 | 414 => { // ppoll, ppoll_time64 (timespec pointer in c)
                let t = if c == 0 { -1 } else if nr == 309 { self.mem.r32(c).unwrap_or(0) as i64 * 1000 + (self.mem.r32(c + 4).unwrap_or(0) / 1_000_000) as i64 }
                    else { self.mem.r32(c).unwrap_or(0) as i64 * 1000 + (self.mem.r32(c + 8).unwrap_or(0) / 1_000_000) as i64 };
                self.poll(a, b, t)
            }
            174 => { // rt_sigaction(sig, act, oldact): remembered, not delivered yet
                let old = self.sigact.get(&a).copied().unwrap_or([0; 5]);
                if b != 0 { let mut n = [0u32; 5]; for (i, x) in n.iter_mut().enumerate() { *x = self.mem.r32(b + i as u32 * 4).unwrap_or(0); } self.sigact.insert(a, n); }
                if c != 0 { for (i, x) in old.iter().enumerate() { let _ = self.mem.w32(c + i as u32 * 4, *x); } }
                Some(0)
            }
            37 => Some(if self.procs.iter().any(|p| p.pid == a && p.exit.is_none()) || (a as i32) <= 0 { 0 } else { ESRCH }), // kill: existence check only
            175 | 186 | 238 | 270 => Some(0), // rt_sigprocmask, sigaltstack, tkill, tgkill
            180 => self.read(a, b, c, Some(d as u64 | (e as u64) << 32)), // pread64
            181 => Some(self.write(a, b, c, Some(d as u64 | (e as u64) << 32))), // pwrite64
            183 => { // getcwd
                let s = self.cwd.clone();
                Some(if s.len() as u32 + 1 > b { -34i32 as u32 } else { let _ = self.mem.write_bytes(a, s.as_bytes()); let _ = self.mem.w8(a + s.len() as u32, 0); s.len() as u32 + 1 })
            }
            195 => Some(self.stat_path(CWD, a, b, true)), // stat64
            196 => Some(self.stat_path(CWD, a, b, false)), // lstat64
            197 => Some(self.fstat(a, b)), // fstat64
            300 => { // fstatat64(dirfd, path, buf, flags)
                if self.mem.r8(b).unwrap_or(0) == 0 && d & 0x1000 != 0 { Some(self.fstat(a, c)) } else { Some(self.stat_path(a, b, c, d & 0x100 == 0)) }
            }
            199..=202 => Some(1000), // getuid32, getgid32, geteuid32, getegid32
            220 => Some(match self.fds.get(&a).cloned() { // getdents64
                Some(o) => {
                    let mut o = o.borrow_mut();
                    let path = o.path.clone();
                    if let Kind::Dir { pos } = &mut o.kind {
                        let mut all = vec![(".".to_string(), 4u8), ("..".to_string(), 4)];
                        all.extend(self.children(&path));
                        let mut out = Vec::new();
                        while *pos < all.len() {
                            let (name, ty) = &all[*pos];
                            let reclen = (19 + name.len() + 1 + 7) & !7;
                            if out.len() + reclen > c as usize { break; }
                            let start = out.len();
                            out.extend_from_slice(&(ino(&format!("{path}/{name}")) as u64).to_le_bytes());
                            out.extend_from_slice(&(*pos as u64 + 1).to_le_bytes());
                            out.extend_from_slice(&(reclen as u16).to_le_bytes());
                            out.push(*ty);
                            out.extend_from_slice(name.as_bytes());
                            out.resize(start + reclen, 0);
                            *pos += 1;
                        }
                        let _ = self.mem.write_bytes(b, &out);
                        out.len() as u32
                    } else { ENOTDIR }
                }
                None => EBADF,
            }),
            55 | 221 => Some(match self.fds.get(&a).cloned() { // fcntl, fcntl64
                Some(o) => match b {
                    0 | 1030 => self.new_fd(o, c), // F_DUPFD, F_DUPFD_CLOEXEC
                    3 => o.borrow().flags, // F_GETFL
                    4 => { let mut o = o.borrow_mut(); o.flags = (o.flags & 3) | (c & !3); 0 } // F_SETFL
                    5 | 12 => { let _ = self.mem.w16(c, 2); 0 } // F_GETLK(64): nothing is locked (F_UNLCK)
                    _ => 0, // F_GETFD, F_SETFD, locks
                },
                None => EBADF,
            }),
            240 | 422 => match b & 0x7F { // futex(addr, op, val, timeout)
                0 | 9 => { // FUTEX_WAIT (relative timeout), FUTEX_WAIT_BITSET (absolute)
                    if self.mem.r32(a).unwrap_or(0) != c { Some(EAGAIN) } else {
                        let ts = if d == 0 { None } else if nr == 240 { Some(self.mem.r32(d).unwrap_or(0) as u64 * 1_000_000_000 + self.mem.r32(d + 4).unwrap_or(0) as u64) }
                            else { Some((self.mem.r32(d).unwrap_or(0) as u64 | (self.mem.r32(d + 4).unwrap_or(0) as u64) << 32) * 1_000_000_000 + self.mem.r32(d + 8).unwrap_or(0) as u64) };
                        let now = self.clock_ns;
                        let t = self.me();
                        t.state = St::Futex(a);
                        t.wake_at = ts.map(|x| if b & 0x7F == 0 { now + x } else { x });
                        self.resched = true;
                        Some(0)
                    }
                }
                1 | 10 => Some(self.futex_wake(a, c)), // FUTEX_WAKE, FUTEX_WAKE_BITSET
                3 | 4 => Some(self.futex_wake(a, u32::MAX)), // (CMP_)REQUEUE: wake everyone instead (spurious wakeups are allowed)
                _ => Some(ENOSYS),
            },
            243 => { // set_thread_area(struct user_desc *)
                let mut entry = self.mem.r32(a).unwrap_or(0);
                let base = self.mem.r32(a + 4).unwrap_or(0);
                if entry == u32::MAX { entry = 6; let _ = self.mem.w32(a, entry); }
                Some(if (6..=8).contains(&entry) {
                    self.cpu.tls[(entry - 6) as usize] = base;
                    if self.cpu.sel_gs >> 3 == entry { self.cpu.gs_base = base; }
                    if self.cpu.sel_fs >> 3 == entry { self.cpu.fs_base = base; }
                    0
                } else { EINVAL })
            }
            258 => { self.me().clear_tid = a; Some(self.me().tid) } // set_tid_address
            265 | 403 => { // clock_gettime (32-bit / 64-bit timespec)
                let (s, ns) = self.now();
                let _ = self.mem.w32(b, s as u32);
                if nr == 403 { let _ = self.mem.w32(b + 4, (s >> 32) as u32); let _ = self.mem.w32(b + 8, ns); } else { let _ = self.mem.w32(b + 4, ns); }
                Some(0)
            }
            311 => Some(0), // set_robust_list
            340 => { // prlimit64: report "unlimited"
                if d != 0 { for i in 0..4 { let _ = self.mem.w32(d + i * 4, u32::MAX); } }
                Some(0)
            }
            191 => { let _ = self.mem.w32(b, u32::MAX); let _ = self.mem.w32(b + 4, u32::MAX); Some(0) } // ugetrlimit
            355 => { for i in 0..b { let _ = self.mem.w8(a + i, (i * 37 + 11) as u8); } Some(b) } // getrandom
            _ => {
                let s = format!("syscall {nr}");
                if !self.unknown.contains(&s) { self.unknown.push(s); }
                Some(ENOSYS)
            }
        };
        match ret {
            Some(ret) => {
                if self.trace.len() < 400_000 { let pid = self.pid(); self.trace.push(format!("{pid} sys {nr}({a:#x}, {b:#x}, {c:#x}, {d:#x}) = {ret:#x}")); }
                self.cpu.r[EAX] = ret;
            }
            None => { // would block: re-execute the `int 0x80` when the process is resumed
                self.cpu.eip = self.cpu.eip.wrapping_sub(2);
                self.blocked = true;
            }
        }
    }
}

impl Default for LinuxProc {
    fn default() -> Self { Self::new() }
}
