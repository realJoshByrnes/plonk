//! Linux i386 personality: ELF loader + `int 0x80` syscall layer, so real Wine Unix-side binaries can run in the interpreter.
//! A small kernel: processes (fork/exec/wait) and threads (clone/futex) scheduled round robin, static and dynamic ELFs,
//! a writable file tree, Unix stream sockets and pipes, poll. The running process/thread has its state swapped into the
//! top-level fields (`mem`, `cpu`, `fds`, ...); everything else is parked in `procs`.
use plonk_cpu::{CacheSlot, Cpu, Exit, EAX, EBP, EBX, ECX, EDI, EDX, ESI, ESP};
use plonk_mem::Mem;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::rc::Rc;

mod gl;
mod snap;

/// Environment for Wine processes. The overrides switch off parts of Wine's session start-up that cost a lot of emulated
/// time and that the desktop does not need yet: Mono/Gecko prompts, menu building, driver hosts, service hosts, COM activation.
pub const WINE_ENV: [&str; 7] = ["HOME=/home/user", "WINEPREFIX=/home/user/.wine", "PATH=/usr/local/bin:/usr/bin:/bin", "WINELOADERNOEXEC=1", "USER=user",
    "WINEDLLOVERRIDES=mscoree,mshtml,winemenubuilder.exe,winedevice.exe,svchost.exe,rpcss.exe=d", "WINEDEBUG=-all"];

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

enum Kind { File { data: Data, pos: u64 }, Dir { pos: usize }, Null, Tty, Sock(usize), Events, Dsp, Mixer }

/// The emulated OSS sound card: PCM written to /dev/dsp drains at the guest clock's pace into `out` (s16le), which the page plays.
pub struct Audio { pub rate: u32, pub ch: u32, fmt: u32, q: VecDeque<u8>, last: u64, pub out: Vec<u8> }
const AUDIO_CAP: usize = 32768;
const AUDIO_FRAG: usize = 2048;

/// A top-level window as reported by the Wine display driver (wineplonk.drv), with the pixels it last flushed.
#[derive(Default, Clone)]
pub struct HostWin {
    pub hwnd: u32, pub owner: u32,
    /// Window and client rectangles in screen coordinates: x, y, width, height.
    pub rect: [i32; 4], pub client: [i32; 4],
    pub style: u32, pub ex_style: u32, pub visible: bool, pub title: String,
    /// Surface size and its BGRX pixels (top-down).
    pub sw: u32, pub sh: u32, pub pixels: Vec<u8>,
    /// The part of the surface changed since the page last took it (l, t, r, b; empty when r <= l).
    pub dirty: [i32; 4],
    /// The host-drawn title bar: flags (see PLONK_SYS_CAPTION), caption rectangle l, t, r, b (screen), button width; changes of the title.
    pub cap: [i32; 7], pub title_gen: u32,
}

/// An open file description (shared by dup'ed descriptors and descriptors passed over sockets).
pub struct Ofd { kind: Kind, flags: u32, path: String }

struct Msg { data: Vec<u8>, off: usize, fds: Vec<Fd> }

/// The page store of a file with shared mappings (pages are 4KB-aligned host blocks, created on first use, never moved).
struct SharedFile { data: Data, pages: Vec<*mut u8> }

/// One end of a Unix stream socket or pipe.
#[derive(Default)]
struct Sock { peer: Option<usize>, rx: VecDeque<Msg>, listening: bool, backlog: VecDeque<usize>, closed: bool, shut_wr: bool }

#[derive(Clone, Copy, PartialEq)]
enum St { Run, Futex(u32), Vfork(usize), Dead }

struct Thread { cpu: Cpu, tid: u32, poll_deadline: Option<u64>, clear_tid: u32, state: St, wake_at: Option<u64> }

/// A parked process. For the running one, the swapped fields hold placeholders.
struct Proc {
    pid: u32, ppid: u32, mem: Mem, fds: HashMap<u32, Fd>, cloexec: HashSet<u32>, cwd: String, brk: u32, brk_start: u32, maps: BTreeMap<u32, u32>, umask: u32,
    exe: String, cmd: String, sigact: HashMap<u32, [u32; 5]>, threads: Vec<Thread>, cache: CacheSlot, exit: Option<u32>, reaped: bool,
}

pub struct LinuxProc {
    /// Clock polling detector (busy-waits on timeGetTime): (thread, retired at the last read, reads in a row).
    clk_poll: ((usize, usize), u64, u32),
    /// Profiling (linuxrun PROF=1): samples of (pid, eip) and system call counts (pid, nr).
    pub prof: Option<(HashMap<(u32, u32), u64>, HashMap<(u32, u32), u64>)>,
    pub mem: Mem,
    pub cpu: Cpu,
    pub stdout: Vec<u8>,
    /// Read-only files visible to the guest (absolute path -> bytes).
    pub files: HashMap<String, Vec<u8>>,
    /// Syscalls answered with ENOSYS, for finding gaps.
    pub unknown: Vec<String>,
    pub trace: Vec<String>,
    /// Processes that were killed by a CPU fault (description each).
    pub faults: Vec<String>,
    /// Maximum number of trace lines kept (0 = tracing off).
    pub trace_cap: usize,
    pub exit_code: Option<u32>,
    /// The process is waiting (poll/read/accept) for something that only time or another party can provide.
    pub blocked: bool,
    vfs: BTreeMap<String, Node>,
    /// Hard links into `files` (path -> path): lets a saved Wine prefix share the DLLs of the install instead of holding copies.
    aliases: HashMap<String, String>,
    /// Files of the read-only layer that are known (path -> size) but whose contents have not been supplied yet
    /// (`add_index` / `provide`). A system call that needs one is retried until the host has provided it.
    lazy: HashMap<String, u32>,
    /// The not-yet-supplied files that something asked for (see `take_wanted`).
    wanted: RefCell<Vec<String>>,
    /// Set when the running system call hit such a file: it is then retried instead of returning.
    lazy_hit: std::cell::Cell<bool>,
    /// Tree files that came from the read-only layer and were never opened for writing (snapshots store them by name).
    pristine: HashSet<String>,
    /// Fingerprint of everything loaded with `add_tar` (names and sizes): a snapshot only fits the archives it was taken with.
    layer_sig: u64,
    /// Paths that came from `add_tar` (only those may be stored by name in a snapshot).
    layer_names: HashSet<String>,
    cwd: String,
    fds: HashMap<u32, Fd>,
    /// Descriptors of the running process that are closed on exec.
    cloexec: HashSet<u32>,
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
    /// Display driver state: screen size, windows by handle, z-order (top first), pending input records, cursor kind.
    pub screen: (u32, u32),
    pub windows: BTreeMap<u32, HostWin>,
    pub zorder: Vec<u32>,
    events: VecDeque<[u32; 8]>,
    pub audio: Audio,
    pub cursor: i32,
    /// OpenGL: the command stream for the page (see linux/gl.rs) and where the GL window is: top-level hwnd, hwnd, client x, y, w, h.
    pub gl: gl::Gl,
    pub gl_rect: [i32; 6],
    /// Files that are mapped MAP_SHARED: their pages live here and every mapping aliases them (Wine's server shares its
    /// clock/session pages with every process this way).
    shared: Vec<SharedFile>,
    /// Read-only page copies of files that are mapped privately (keyed by the address of the file's contents): every such
    /// mapping in every process refers to these pages until it writes to one (copy on write). Dropped from here when the
    /// file is written to; the pages themselves are never freed (mappings may still refer to them).
    cow: HashMap<usize, SharedFile>,
    /// The page contents of the restored snapshot: processes refer to them copy-on-write. (address, pages)
    snap_pool: (usize, usize),
    /// Set when a GL frame was presented: `run` returns so the page can draw it.
    frame_break: bool,
    gl_pid: u32,
    /// Bumped whenever a window changes or pixels are flushed.
    pub frame: u64,
    cur: (usize, usize),
    next_pid: u32,
    resched: bool,
    /// When every thread is waiting, jump the guest clock to the next timeout instead of reporting `blocked`.
    pub auto_time: bool,
    /// Guest time added by every clock read. Non-zero keeps programs that poll the clock moving when the host does not
    /// advance time itself (tests); hosts that follow the real clock set it to 0.
    pub clock_step_ns: u64,
    /// Keep running after the first process has exited, until every process is gone.
    pub wait_all: bool,
}

fn r16(d: &[u8], o: usize) -> u32 { u16::from_le_bytes([d[o], d[o + 1]]) as u32 }
fn r32(d: &[u8], o: usize) -> u32 { u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) }
fn ino(path: &str) -> u32 { path.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193)) | 0x1000 }
fn parent(path: &str) -> &str { match path.rfind('/') { Some(0) | None => "/", Some(i) => &path[..i] } }

impl LinuxProc {
    pub fn new() -> Self {
        let mut vfs = BTreeMap::new();
        for d in ["/", "/tmp", "/dev", "/dev/plonk", "/home", "/home/user", "/etc", "/proc", "/proc/self"] { vfs.insert(d.to_string(), Node::Dir); }
        LinuxProc { mem: Mem::new_owned(0), cpu: Cpu::new(), stdout: Vec::new(), files: HashMap::new(), unknown: Vec::new(), trace: Vec::new(), faults: Vec::new(), trace_cap: 400_000, clk_poll: ((usize::MAX, 0), 0, 0), prof: None, exit_code: None, blocked: false,
            vfs, aliases: HashMap::new(), lazy: HashMap::new(), wanted: RefCell::new(Vec::new()), lazy_hit: std::cell::Cell::new(false), pristine: HashSet::new(), layer_sig: 0, layer_names: HashSet::new(), cwd: "/".into(), fds: HashMap::new(), cloexec: HashSet::new(), socks: Vec::new(), brk: 0, brk_start: 0, maps: BTreeMap::new(), clock_ns: 1_700_000_000_000_000_000, poll_deadline: None, umask: 0o022,
            exe: String::new(), sigact: HashMap::new(), procs: Vec::new(), screen: (1024, 768), windows: BTreeMap::new(), zorder: Vec::new(), events: VecDeque::new(), audio: Audio { rate: 44100, ch: 2, fmt: 16, q: VecDeque::new(), last: 0, out: Vec::new() }, cursor: 0, gl: gl::Gl::default(), gl_rect: [0; 6], shared: Vec::new(), cow: HashMap::new(), snap_pool: (0, 0), frame_break: false, gl_pid: 0, frame: 0, cur: (usize::MAX, 0), next_pid: 1000, resched: false, auto_time: false, clock_step_ns: 20_000, wait_all: false }
    }

    // ---- processes and threads ----

    fn swap_proc(&mut self) {
        use std::mem::swap;
        let p = &mut self.procs[self.cur.0];
        swap(&mut self.mem, &mut p.mem); swap(&mut self.fds, &mut p.fds); swap(&mut self.cloexec, &mut p.cloexec); swap(&mut self.cwd, &mut p.cwd); swap(&mut self.brk, &mut p.brk);
        swap(&mut self.brk_start, &mut p.brk_start); swap(&mut self.maps, &mut p.maps); swap(&mut self.umask, &mut p.umask);
        swap(&mut self.exe, &mut p.exe); swap(&mut self.sigact, &mut p.sigact);
    }
    fn swap_thread(&mut self) {
        let t = &mut self.procs[self.cur.0].threads[self.cur.1];
        std::mem::swap(&mut self.cpu, &mut t.cpu);
        std::mem::swap(&mut self.poll_deadline, &mut t.poll_deadline);
    }
    /// Make (process, thread) the running one. The decoded-code cache belongs to the process and travels with whichever thread runs.
    /// Debug: bytes of a process's memory (hex), for inspecting a hot spot found by the profiler.
    pub fn peek(&mut self, pid: u32, addr: u32, len: u32) -> String {
        let Some(pi) = self.procs.iter().position(|p| p.pid == pid) else { return "no such pid".into() };
        let ti = self.procs[pi].threads.iter().position(|t| t.state != St::Dead).unwrap_or(0);
        self.switch(pi, ti);
        if len == 0 { // list the PE images (MZ at a 64 KB boundary) from `addr` up to 2 GB with their SizeOfImage and the first name-like string of the export directory
            let mut out = String::new();
            let mut a = addr & !0xFFFF;
            while a < 0x8000_0000 {
                if self.mem.r16(a).map_or(false, |v| v == 0x5a4d) {
                    let pe = a + self.mem.r32(a + 0x3c).unwrap_or(0);
                    let size = self.mem.r32(pe + 0x50).unwrap_or(0);
                    let exp = self.mem.r32(pe + 0x78).unwrap_or(0);
                    let name = if exp != 0 { let n = self.mem.r32(a + exp + 12).unwrap_or(0); self.cstr(a + n) } else { String::new() };
                    out += &format!("{a:#x}+{size:#x} {name}
");
                    a += (size + 0xFFFF) & !0xFFFF; continue;
                }
                a += 0x10000;
            }
            return out;
        }
        (0..len).map(|i| format!("{:02x}", self.mem.r8(addr + i).unwrap_or(0))).collect::<Vec<_>>().join("")
    }
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
    fn tr(&mut self, s: String) { if self.trace.len() < self.trace_cap { self.trace.push(s); } }
    fn me(&mut self) -> &mut Thread { &mut self.procs[self.cur.0].threads[self.cur.1] }
    fn pid(&self) -> u32 { self.procs[self.cur.0].pid }
    fn new_cpu() -> Cpu { let mut c = Cpu::new(); c.linux = true; c }
    fn blank_proc(&mut self, ppid: u32) -> Proc {
        let pid = self.next_pid;
        self.next_pid += 1;
        let mut mem = Mem::new_sparse(MEM_SIZE);
        Proc { pid, ppid, mem, fds: HashMap::new(), cloexec: HashSet::new(), cwd: "/".into(), brk: 0, brk_start: 0, maps: BTreeMap::new(), umask: 0o022, exe: String::new(), cmd: String::new(), sigact: HashMap::new(),
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

    /// Start a new process from an executable in the guest file tree (argv[0] is its path).
    pub fn spawn_path(&mut self, argv: &[&str], envp: &[&str]) -> Result<u32, String> {
        let path = argv.first().ok_or("empty command")?;
        let elf = self.file_bytes(path).ok_or(format!("{path}: not found"))?;
        self.spawn(&elf, argv, envp)
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
        // give its memory back now: the entry stays (as a zombie, and so that process indices stay valid) but holds nothing
        self.mem = Mem::new_sparse(0); self.maps.clear(); self.sigact.clear(); drop(self.cpu.take_cache());
        if self.procs[pi].pid == self.gl_pid { self.gl = gl::Gl::default(); self.gl_rect = [0; 6]; self.gl_pid = 0; self.frame += 1; } // its GL window is gone
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
        p.fds = self.fds.clone(); p.cloexec = self.cloexec.clone(); p.cwd = self.cwd.clone(); p.brk = self.brk; p.brk_start = self.brk_start; p.maps = self.maps.clone();
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
        let data = self.file_bytes(&p);
        if self.lazy_target("/lib/ld-linux.so.2") { return ENOENT; } // the loader must be here before the old image goes
        let Some(data) = data else { self.tr(format!("execve {p} -> ENOENT")); return ENOENT };
        if data.len() < 52 || data[0] != 0x7F { return -8i32 as u32; } // ENOEXEC
        let (mut argv, envp) = (self.str_vec(argv), self.str_vec(envp));
        if argv.is_empty() { argv.push(p.clone()); }
        self.tr(format!("pid {} execve {p} {argv:?}", self.pid()));
        let pi0 = self.cur.0;
        self.procs[pi0].cmd = argv.iter().skip(2).map(|a| a.rsplit(['/', char::from(92)]).next().unwrap_or(a)).collect::<Vec<_>>().join(" ");
        let pi = self.cur.0;
        let ti = self.cur.1;
        for (i, t) in self.procs[pi].threads.iter_mut().enumerate() { if i != ti { t.state = St::Dead; } }
        for pr in self.procs.iter_mut() { for t in pr.threads.iter_mut() { if t.state == St::Vfork(pi) { t.state = St::Run; } } }
        self.me().clear_tid = 0;
        let gone: Vec<u32> = self.cloexec.iter().copied().collect();
        for fd in gone { self.close(fd); }
        let retired = self.cpu.retired;
        self.mem = Mem::new_sparse(MEM_SIZE);
        self.cpu = Self::new_cpu();
        self.cpu.retired = retired;
        self.sigact.clear();
        self.exe = p;
        self.resched = true;
        if let Err(e) = self.exec_image(&data, &argv, &envp) { self.tr(format!("execve failed after the point of no return: {e}")); self.proc_exit(127); }
        0
    }

    // ---- display driver (wineplonk.drv) ----

    /// Driver calls: see wine/plonkdrv/wineplonk.drv/plonk_abi.h.
    fn plonk_call(&mut self, n: u32, a: [u32; 5]) -> u32 {
        self.frame += 1;
        if let Some(pr) = self.prof.as_mut() { *pr.1.entry((self.procs[self.cur.0].pid, 100000 + n)).or_default() += 1; }
        match n {
            0 => { let _ = self.mem.w32(a[0], self.screen.0); let _ = self.mem.w32(a[0] + 4, self.screen.1); }
            1 => { // create or update a top-level window
                let g = |i: u32| self.mem.r32(a[0] + i * 4).unwrap_or(0);
                let (hwnd, after) = (g(0), g(13));
                let known = self.windows.contains_key(&hwnd);
                let w = self.windows.entry(hwnd).or_default();
                w.hwnd = hwnd; w.owner = g(1);
                w.rect = [g(2) as i32, g(3) as i32, g(4) as i32, g(5) as i32];
                w.client = [g(6) as i32, g(7) as i32, g(8) as i32, g(9) as i32];
                w.style = g(10); w.ex_style = g(11); w.visible = g(12) != 0;
                // z-order: 0 = top, otherwise directly behind `after`; an unknown `after` leaves a known window where it is
                let behind = self.zorder.iter().position(|&h| h == after && h != hwnd);
                if after == 0 || behind.is_some() || !known {
                    self.zorder.retain(|&h| h != hwnd);
                    let at = if after == 0 { 0 } else { self.zorder.iter().position(|&h| h == after).map_or(0, |i| i + 1) };
                    self.zorder.insert(at, hwnd);
                }
            }
            2 => { self.windows.remove(&a[0]); self.zorder.retain(|&h| h != a[0]); }
            3 => { // flush(hwnd, bits, width, height, dirty[l,t,r,b])
                let (sw, sh) = (a[2], a[3]);
                if sw == 0 || sh == 0 || sw > 16384 || sh > 16384 { return EINVAL; }
                let d = |i: u32| self.mem.r32(a[4] + i * 4).unwrap_or(0) as i32;
                let Some(w) = self.windows.get_mut(&a[0]) else { return 0 };
                let (mut l, mut t, mut r, mut b) = if a[4] != 0 { (d(0), d(1), d(2), d(3)) } else { (0, 0, sw as i32, sh as i32) };
                if w.sw != sw || w.sh != sh { w.sw = sw; w.sh = sh; w.pixels = vec![0; (sw * sh * 4) as usize]; (l, t, r, b) = (0, 0, sw as i32, sh as i32); }
                (l, t, r, b) = (l.max(0), t.max(0), r.min(sw as i32), b.min(sh as i32));
                if r > l && b > t {
                    let dd = &mut w.dirty;
                    *dd = if dd[2] <= dd[0] { [l, t, r, b] } else { [dd[0].min(l), dd[1].min(t), dd[2].max(r), dd[3].max(b)] };
                }
                for y in t..b.max(t) {
                    if r <= l { break; }
                    let off = (y as u32 * sw + l as u32) * 4;
                    let Ok(src) = self.mem.read_bytes(a[1] + off, (r - l) as u32 * 4) else { return EFAULT };
                    w.pixels[off as usize..off as usize + src.len()].copy_from_slice(src);
                }
            }
            7 => { // caption(hwnd, flags, l, t, r, b, button width)
                let v: Vec<i32> = (0..7).map(|i| self.mem.r32(a[0] + (i + 1) * 4).unwrap_or(0) as i32).collect();
                let hw = self.mem.r32(a[0]).unwrap_or(0);
                if let Some(w) = self.windows.get_mut(&hw) { w.cap.copy_from_slice(&v); w.title_gen += 1; }
                self.frame += 1;
            }
            4 => { // title(hwnd, utf16, len)
                let u: Vec<u16> = (0..a[2].min(1024)).map(|i| self.mem.r16(a[1] + i * 2).unwrap_or(0)).collect();
                if let Some(w) = self.windows.get_mut(&a[0]) { w.title = String::from_utf16_lossy(&u); w.title_gen += 1; }
                self.frame += 1;
            }
            5 => self.cursor = a[1] as i32,
            6 => { // where the GL window is, and whose it is
                let old = self.gl_rect;
                for i in 0..6 { self.gl_rect[i] = self.mem.r32(a[0] + i as u32 * 4).unwrap_or(0) as i32; }
                self.gl_pid = self.pid();
                if old == self.gl_rect { self.frame -= 1; } // sent with every frame: only a move is a change for the 2D screen
            }
            _ => return ENOSYS,
        }
        0
    }

    /// Queue an input record for the driver (types as in plonk_abi.h: 1 move, 2 button down, 3 button up, 4 wheel, 5 key down, 6 key up, 7 close, 8 resize).
    pub fn push_event(&mut self, ty: u32, hwnd: u32, x: i32, y: i32, data: u32, flags: u32) {
        self.events.push_back([ty, hwnd, x as u32, y as u32, data, flags, (self.clock_ns / 1_000_000) as u32, 0]);
    }

    /// The topmost visible window at a screen position (0 = none).
    pub fn window_at(&self, x: i32, y: i32) -> u32 {
        self.zorder.iter().copied().find(|h| self.windows.get(h).map_or(false, |w| w.visible && x >= w.rect[0] && y >= w.rect[1] && x < w.rect[0] + w.rect[2] && y < w.rect[1] + w.rect[3])).unwrap_or(0)
    }

    /// Mouse input at a screen position: kind 0 = left down, 1 = move, 2 = left up.
    pub fn mouse(&mut self, kind: u32, x: i32, y: i32) {
        let h = self.window_at(x, y);
        self.push_event(match kind { 0 => 2, 2 => 3, _ => 1 }, h, x, y, 0, 0);
    }

    /// Plain composition of the visible windows over a flat desktop colour: RGB, screen-sized (debugging / tests).
    pub fn compose_rgb(&self) -> Vec<u8> {
        let (sw, sh) = (self.screen.0 as i32, self.screen.1 as i32);
        let mut out: Vec<u8> = [58u8, 110, 165].iter().copied().cycle().take((sw * sh * 3) as usize).collect();
        for h in self.zorder.iter().rev() {
            let Some(w) = self.windows.get(h) else { continue };
            if !w.visible || w.pixels.is_empty() { continue; }
            for y in 0..w.sh as i32 {
                let dy = w.rect[1] + y;
                if dy < 0 || dy >= sh { continue; }
                for x in 0..w.sw as i32 {
                    let dx = w.rect[0] + x;
                    if dx < 0 || dx >= sw { continue; }
                    let (si, di) = (((y as u32 * w.sw + x as u32) * 4) as usize, ((dy * sw + dx) * 3) as usize);
                    out[di] = w.pixels[si + 2]; out[di + 1] = w.pixels[si + 1]; out[di + 2] = w.pixels[si];
                }
            }
        }
        out
    }

    // ---- address space ----

    fn maps_hit(&self, a: u32, e: u32) -> bool { self.maps.range(..e).next_back().map_or(false, |(_, &en)| en > a) }
    /// Is any part of [a, e) in use (a mapping or the heap)?
    fn mapped(&self, a: u32, e: u32) -> bool { self.maps_hit(a, e) || (a < ((self.brk + 0xFFF) & !0xFFF) && e > self.brk_start) }

    fn unmap(&mut self, a: u32, e: u32) {
        let hit: Vec<(u32, u32)> = self.maps.range(..e).filter(|(_, &en)| en > a).map(|(&s, &en)| (s, en)).collect();
        for (s, en) in hit {
            self.maps.remove(&s);
            if s < a { self.maps.insert(s, a); }
            if en > e { self.maps.insert(e, en); }
        }
    }

    /// Highest free gap of `len` bytes below the stack (the heap counts as in use).
    fn find_gap(&self, len: u32) -> Option<u32> {
        let mut used: Vec<(u32, u32)> = self.maps.range(..STACK_LOW).map(|(&s, &e)| (s, e)).collect();
        if self.brk_start != 0 { used.push((self.brk_start, (self.brk + 0xFFF) & !0xFFF)); }
        used.sort_unstable();
        let mut top = STACK_LOW;
        for &(s, e) in used.iter().rev() {
            if top >= e && top - e >= len { break; }
            top = top.min(s);
        }
        (top >= MMAP_MIN + len).then(|| top - len)
    }

    /// Unpack a tar archive (ustar/GNU: files, directories, symlinks, hard links, long names) into the guest file tree at `/`.
    pub fn add_tar(&mut self, tar: &[u8]) {
        let field = |b: &[u8]| String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]).into_owned();
        let (mut off, mut long): (usize, Option<String>) = (0, None);
        while off + 512 <= tar.len() && tar[off] != 0 {
            let h = &tar[off..off + 512];
            let size = usize::from_str_radix(field(&h[124..136]).trim(), 8).unwrap_or(0);
            let body = &tar[off + 512..(off + 512 + size).min(tar.len())];
            off += 512 + (size + 511) / 512 * 512;
            if h[156] == b'L' { long = Some(field(body)); continue; }
            let mut name = long.take().unwrap_or_else(|| { let (n, pre) = (field(&h[..100]), field(&h[345..500])); if pre.is_empty() { n } else { format!("{pre}/{n}") } });
            name = format!("/{}", name.trim_start_matches("./").trim_end_matches('/'));
            self.layer_sig = name.bytes().fold(self.layer_sig ^ size as u64, |h, b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)).rotate_left(7);
            match h[156] {
                b'0' | 0 => { self.layer_names.insert(name.clone()); self.files.insert(name, body.to_vec()); }
                b'5' => { self.vfs.insert(name, Node::Dir); }
                b'2' => { self.vfs.insert(name, Node::Link(field(&h[157..257]))); }
                b'1' => { self.layer_names.insert(name.clone()); self.aliases.insert(name, format!("/{}", field(&h[157..257]))); }
                _ => {}
            }
        }
    }

    /// Declare the contents of a tar archive without its file bodies: one line per entry, in archive order,
    /// `type<TAB>size<TAB>name<TAB>link target` (type as in tar: 0 file, 1 hard link, 2 symlink, 5 directory). Files are
    /// asked for through `take_wanted` when first used and supplied with `provide`.
    pub fn add_index(&mut self, index: &str) {
        for line in index.lines() {
            let mut f = line.split(char::from(9));
            let (Some(ty), Some(size), Some(name)) = (f.next(), f.next().and_then(|s| s.parse::<u32>().ok()), f.next()) else { continue };
            let link = f.next().unwrap_or("");
            let name = format!("/{}", name.trim_start_matches("./").trim_end_matches('/'));
            self.layer_sig = name.bytes().fold(self.layer_sig ^ size as u64, |h, b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)).rotate_left(7);
            match ty {
                "0" => { self.layer_names.insert(name.clone()); if !self.files.contains_key(&name) { self.lazy.insert(name, size); } }
                "5" => { self.vfs.insert(name, Node::Dir); }
                "2" => { self.vfs.insert(name, Node::Link(link.to_string())); }
                "1" => { self.layer_names.insert(name.clone()); self.aliases.insert(name, format!("/{link}")); }
                _ => {}
            }
        }
    }

    /// Supply a file: the contents of one declared by `add_index`, or a new file of the read-only layer.
    pub fn provide(&mut self, name: String, data: Vec<u8>) {
        self.lazy.remove(&name);
        self.wanted.borrow_mut().retain(|w| *w != name);
        self.files.insert(name, data);
    }

    /// Supply the files of a tar archive (regular files only; see `provide`).
    pub fn provide_tar(&mut self, tar: &[u8]) {
        let field = |b: &[u8]| String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]).into_owned();
        let (mut off, mut long): (usize, Option<String>) = (0, None);
        while off + 512 <= tar.len() && tar[off] != 0 {
            let h = &tar[off..off + 512];
            let size = usize::from_str_radix(field(&h[124..136]).trim(), 8).unwrap_or(0);
            let body = &tar[off + 512..(off + 512 + size).min(tar.len())];
            off += 512 + (size + 511) / 512 * 512;
            if h[156] == b'L' { long = Some(field(body)); continue; }
            let name = long.take().unwrap_or_else(|| { let (n, pre) = (field(&h[..100]), field(&h[345..500])); if pre.is_empty() { n } else { format!("{pre}/{n}") } });
            if matches!(h[156], b'0' | 0) { self.provide(format!("/{}", name.trim_start_matches("./")), body.to_vec()); }
        }
    }

    /// The declared files that the guest (or `spawn_path`) needs now. Whatever asked is waiting for `provide`.
    pub fn take_wanted(&mut self) -> Vec<String> { self.wanted.borrow().clone() }

    /// The files of the read-only layer that are in use unmodified (what a snapshot refers to by name).
    pub fn pristine_files(&self) -> Vec<String> { let mut v: Vec<String> = self.pristine.iter().cloned().collect(); v.sort(); v }

    /// True (and the file is asked for) when `p` is, or is a hard link to, a file whose contents are not here yet.
    fn lazy_target(&self, p: &str) -> bool {
        let mut t = p;
        for _ in 0..8 { match self.aliases.get(t) { Some(a) if !self.vfs.contains_key(t) => t = a, _ => break } }
        if self.vfs.contains_key(t) || !self.lazy.contains_key(t) { return false; }
        self.lazy_hit.set(true);
        let mut w = self.wanted.borrow_mut();
        if !w.iter().any(|x| x == t) { w.push(t.to_string()); }
        true
    }

    /// One line per live process: pid, parent, executable, and what each thread is doing (debugging).
    pub fn ps(&self) -> String {
        let mut out = String::new();
        for (pi, p) in self.procs.iter().enumerate() {
            let exe = if pi == self.cur.0 { &self.exe } else { &p.exe };
            let mi: u64 = p.threads.iter().enumerate().map(|(ti, t)| if (pi, ti) == self.cur { self.cpu.retired } else { t.cpu.retired }).sum::<u64>() / 1_000_000;
            if let Some(c) = p.exit { out.push_str(&format!("pid {} ppid {} {exe} {} [{mi} M instr] exited {c}{}", p.pid, p.ppid, p.cmd, char::from(10))); continue; }
            out.push_str(&format!("pid {} ppid {} {exe} {} [{mi} M instr, {} MB]:", p.pid, p.ppid, p.cmd, (if pi == self.cur.0 { &self.mem } else { &p.mem }).own_bytes() >> 20));
            for (ti, t) in p.threads.iter().enumerate() {
                let cpu = if (pi, ti) == self.cur { &self.cpu } else { &t.cpu };
                match t.state {
                    St::Dead => {}
                    St::Run => out.push_str(&format!(" [{} sys {} ({:#x},{:#x},{:#x}) eip {:#x}]", t.tid, cpu.r[EAX], cpu.r[EBX], cpu.r[ECX], cpu.r[EDX], cpu.eip)),
                    St::Futex(a) => out.push_str(&format!(" [{} futex {a:#x}{}]", t.tid, if t.wake_at.is_some() { " timed" } else { "" })),
                    St::Vfork(c) => out.push_str(&format!(" [{} vfork-wait {}]", t.tid, self.procs[c].pid)),
                }
            }
            out.push(char::from(10));
        }
        out
    }

    /// Pack everything created at run time under `prefix` (files, directories, symlinks) into a tar archive that `add_tar` can load.
    pub fn export_tar(&self, prefix: &str) -> Vec<u8> {
        fn entry(out: &mut Vec<u8>, name: &str, ty: u8, link: &str, data: &[u8]) {
            let name = name.trim_start_matches('/');
            if name.len() > 99 { entry(out, "././@LongLink", b'L', "", format!("{name}{}", char::from(0)).as_bytes()); }
            let mut h = [0u8; 512];
            let n = name.as_bytes();
            h[..n.len().min(99)].copy_from_slice(&n[..n.len().min(99)]);
            h[100..107].copy_from_slice(if ty == b'5' { b"0000755" } else { b"0000644" });
            h[108..115].copy_from_slice(b"0001750");
            h[116..123].copy_from_slice(b"0001750");
            h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
            h[136..147].copy_from_slice(b"00000000000");
            h[156] = ty;
            h[157..157 + link.len().min(99)].copy_from_slice(&link.as_bytes()[..link.len().min(99)]);
            h[257..263].copy_from_slice(b"ustar ");
            h[263] = b' ';
            h[148..156].fill(b' ');
            let sum: u32 = h.iter().map(|&b| b as u32).sum();
            h[148..154].copy_from_slice(format!("{sum:06o}").as_bytes());
            h[154] = 0;
            out.extend_from_slice(&h);
            out.extend_from_slice(data);
            out.resize(out.len() + (512 - data.len() % 512) % 512, 0);
        }
        let mut out = Vec::new();
        let pre = format!("{}/", prefix.trim_end_matches('/'));
        for (k, n) in self.vfs.iter().filter(|(k, _)| k.starts_with(&pre) || k.as_str() == prefix) {
            match n {
                Node::Dir => entry(&mut out, k, b'5', "", &[]),
                Node::Link(t) => entry(&mut out, k, b'2', t, &[]),
                Node::File(d) => match self.install_twin(k, &d.borrow()) { Some(t) => entry(&mut out, k, b'1', t.trim_start_matches('/'), &[]), None => entry(&mut out, k, b'0', "", &d.borrow()) },
                Node::Sock(_) => {}
            }
        }
        for (k, t) in self.aliases.iter().filter(|(k, _)| k.starts_with(&pre)) { entry(&mut out, k, b'1', t.trim_start_matches('/'), &[]); }
        out.resize(out.len() + 1024, 0);
        out
    }

    /// The install's file with the same name and contents as this prefix file, if any (Wine copies its DLLs into the prefix).
    fn install_twin(&self, path: &str, data: &[u8]) -> Option<String> {
        let base = path.rsplit('/').next()?;
        for dir in ["/usr/local/lib/wine/i386-windows", "/usr/local/share/wine/fonts", "/usr/local/share/wine/nls"] {
            let t = format!("{dir}/{base}");
            let same = match self.vfs.get(&t) { Some(Node::File(d)) => d.borrow().as_slice() == data, _ => self.files.get(&t).map_or(false, |d| d.as_slice() == data) };
            if same && t.len() < 100 { return Some(t); }
        }
        None
    }

    /// Move a file of the read-only layer into the tree (shared, no copy) and return it. `private`: a hard link gets its own copy.
    fn promote(&mut self, p: &str, private: bool) -> Option<Data> {
        if let Some(t) = self.aliases.get(p).cloned() {
            let target = match self.vfs.get(&t) { Some(Node::File(d)) => d.clone(), _ => self.promote(&t, false)? };
            let data = if private { Rc::new(RefCell::new(target.borrow().clone())) } else { target };
            self.aliases.remove(p);
            self.vfs.insert(p.to_string(), Node::File(data.clone()));
            if !private && self.layer_names.contains(p) { self.pristine.insert(p.to_string()); }
            return Some(data);
        }
        if self.lazy_target(p) { return None; }
        let d = self.files.remove(p)?;
        let data: Data = Rc::new(RefCell::new(d));
        self.vfs.insert(p.to_string(), Node::File(data.clone()));
        if !private && self.layer_names.contains(p) { self.pristine.insert(p.to_string()); }
        Some(data)
    }

    /// Contents of a regular file anywhere in the tree (a copy).
    fn file_bytes(&self, p: &str) -> Option<Vec<u8>> {
        match self.vfs.get(p) {
            Some(Node::File(d)) => Some(d.borrow().clone()),
            Some(_) => None,
            None if self.lazy_target(p) => None,
            None => self.files.get(p).cloned().or_else(|| self.aliases.get(p).and_then(|t| self.file_bytes(t))),
        }
    }

    /// Create a directory in the guest file tree.
    pub fn mkdir(&mut self, path: &str) { self.vfs.insert(path.to_string(), Node::Dir); }

    /// Paths created in the guest file tree at run time.
    pub fn created(&self) -> Vec<String> { self.vfs.keys().cloned().collect() }

    /// Advance the guest clock (lets poll timeouts expire).
    pub fn advance(&mut self, ms: u64) { self.clock_ns += ms * 1_000_000; }
    pub fn advance_ns(&mut self, ns: u64) { self.clock_ns += ns; self.audio_pump(); }
    /// Move the PCM the sound card would have played by now from its queue to `audio.out`.
    fn audio_pump(&mut self) {
        let a = &mut self.audio;
        let bps = (a.rate as u64 * a.ch as u64 * 2).max(1);
        if a.q.is_empty() { a.last = self.clock_ns; return; }
        let n = (((self.clock_ns - a.last) * bps / 1_000_000_000) as usize & !3).min(a.q.len());
        if n == 0 { return; }
        a.last += n as u64 * 1_000_000_000 / bps;
        let v: Vec<u8> = a.q.drain(..n).collect();
        a.out.extend_from_slice(&v);
    }
    fn audio_write(&mut self, src: &[u8]) {
        self.audio_pump();
        let a = &mut self.audio;
        if a.q.is_empty() { a.last = self.clock_ns; }
        if a.fmt == 8 { for &b in src { let v = ((b as i16 - 128) << 8) as u16; a.q.extend(v.to_le_bytes()); } } else { a.q.extend(src.iter().copied()); }
    }
    fn ioctl(&mut self, fd: u32, req: u32, arg: u32) -> u32 {
        let Some(o) = self.fds.get(&fd).cloned() else { return EBADF };
        let k = match o.borrow().kind { Kind::Dsp => 1, Kind::Mixer => 2, _ => 0 };
        if k == 0 { return ENOTTY; }
        let (ty, nr) = ((req >> 8) & 0xFF, req & 0xFF);
        let ok = 0u32;
        match (ty as u8, nr) {
            (b'X', 1) => { // SNDCTL_SYSINFO
                let mut b = vec![0u8; 1248];
                b[..9].copy_from_slice(b"Plonk OSS"); b[32..37].copy_from_slice(b"4.2.0");
                b[64..68].copy_from_slice(&0x040200u32.to_le_bytes());
                for off in [196usize, 248, 284, 288] { b[off..off + 4].copy_from_slice(&1u32.to_le_bytes()); } // numaudios, nummixers, numcards, numaudioengines
                if self.mem.write_bytes(arg, &b).is_err() { return EFAULT; }
                ok
            }
            (b'X', 7) | (b'X', 12) => { // SNDCTL_AUDIOINFO / ENGINEINFO
                let mut b = vec![0u8; 1180];
                let mut put = |off: usize, v: u32| b[off..off + 4].copy_from_slice(&v.to_le_bytes());
                put(76, 0x0002_0000); put(84, 0x18); put(172, 1); // caps: output; oformats: S16_LE | U8; enabled
                put(180, 8000); put(184, 48000); put(188, 1); put(192, 2);
                b[4..14].copy_from_slice(b"Plonk Wave");
                b[404..412].copy_from_slice(b"/dev/dsp");
                if self.mem.write_bytes(arg, &b).is_err() { return EFAULT; }
                ok
            }
            (b'M', _) => { if req & 0x4000_0000 != 0 { let _ = self.mem.w32(arg, 0x6464); } ok }
            (b'P', 5) => { // SETFMT
                let v = self.mem.r32(arg).unwrap_or(0);
                let f = if v == 0x8 { 8 } else { 16 };
                self.audio.fmt = f;
                let _ = self.mem.w32(arg, if f == 8 { 0x8 } else { 0x10 });
                ok
            }
            (b'P', 2) => { let v = self.mem.r32(arg).unwrap_or(44100).clamp(8000, 48000); self.audio_pump(); self.audio.rate = v; let _ = self.mem.w32(arg, v); ok }
            (b'P', 6) => { let v = self.mem.r32(arg).unwrap_or(2).clamp(1, 2); self.audio.ch = v; let _ = self.mem.w32(arg, v); ok }
            (b'P', 3) => { let v = self.mem.r32(arg).unwrap_or(1); self.audio.ch = if v != 0 { 2 } else { 1 }; ok }
            (b'P', 11) => { let _ = self.mem.w32(arg, 0x18); ok } // GETFMTS
            (b'P', 15) => { let _ = self.mem.w32(arg, 0x0000_1000); ok } // GETCAPS: revision 1, no trigger
            (b'P', 10) => ok, // SETFRAGMENT
            (b'P', 1) | (b'P', 0) | (b'P', 8) | (b'P', 4) | (b'P', 14) | (b'P', 16) => ok, // SYNC, RESET, POST, ... NONBLOCK
            (b'P', 12) | (b'P', 13) => { // GETOSPACE / GETISPACE
                self.audio_pump();
                let used = self.audio.q.len().min(AUDIO_CAP);
                let free = if nr == 12 { AUDIO_CAP - used } else { 0 };
                for (i, v) in [free / AUDIO_FRAG, AUDIO_CAP / AUDIO_FRAG, AUDIO_FRAG, free].iter().enumerate() { let _ = self.mem.w32(arg + i as u32 * 4, *v as u32); }
                ok
            }
            (b'P', 23) => { self.audio_pump(); let _ = self.mem.w32(arg, self.audio.q.len() as u32); ok } // GETODELAY
            _ => ENOTTY,
        }
    }
    /// Is this process still running?
    pub fn alive(&self, pid: u32) -> bool { self.procs.iter().any(|p| p.pid == pid && p.exit.is_none()) }
    /// Instructions executed by all threads of all processes.
    pub fn retired(&self) -> u64 {
        let mut n = self.cpu.retired;
        for (pi, p) in self.procs.iter().enumerate() { for (ti, t) in p.threads.iter().enumerate() { if (pi, ti) != self.cur { n += t.cpu.retired; } } }
        n
    }

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
        self.brk = 0;
        self.brk_start = 0;
        if self.exe.is_empty() { self.exe = argv.first().cloned().unwrap_or_default(); }
        if elf.len() < 52 { return Err("not an i386 ELF".into()); }
        let bias = if r16(elf, 16) == 3 { EXE_BIAS } else { 0 }; // ET_DYN (PIE)
        let (exe_entry, phdr_va, phentsize, phnum, end, interp) = self.map_elf(elf, bias)?;
        let (mut entry, mut at_base) = (exe_entry, 0);
        if let Some(path) = interp {
            let ld = self.file_bytes(&path).ok_or(format!("interpreter {path} not in files"))?;
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
            let chunk = if self.prof.is_some() { (steps - used).min(997) } else { steps - used };
            let (pk, r0) = ((self.procs[self.cur.0].pid, self.cpu.eip), self.cpu.retired);
            let ex = self.cpu.run(&mut self.mem, chunk);
            if let Some(pr) = self.prof.as_mut() { *pr.0.entry(pk).or_default() += self.cpu.retired - r0; }
            match ex {
                Exit::Interrupt(0x80) => { self.syscall(); if self.blocked || self.resched { break; } }
                Exit::StepLimit => break,
                e => { // no signal delivery yet: a CPU fault ends this process only (as an unhandled SIGSEGV would)
                    let at = self.cpu.insn_start;
                    let bytes: Vec<String> = (0..12).map(|i| format!("{:02x}", self.mem.r8(at + i).unwrap_or(0))).collect();
                    let msg = format!("plonk: pid {} ({}) killed: {e:?} at eip {:#x} insn {at:#x} [{}] regs {:x?} stack {:x?}", self.pid(), self.exe, self.cpu.eip, bytes.join(" "), self.cpu.r, (0..8).map(|i| self.mem.r32(self.cpu.r[4] + i * 4).unwrap_or(0)).collect::<Vec<_>>());
                    self.stdout.extend_from_slice(msg.as_bytes());
                    self.stdout.push(10);
                    self.faults.push(msg);
                    self.proc_exit(139);
                    break;
                }
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
                    if ti >= self.procs[pi].threads.len() { break; }
                    if self.exit_code.is_some() && !self.wait_all { return Ok(self.exit_code); }
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
                    if self.frame_break { self.frame_break = false; self.blocked = false; return Ok(self.exit_code); }
                    if !(blocked && used <= 2) { progress = true; }
                    budget = budget.saturating_sub(used.max(1));
                    if budget == 0 { self.blocked = false; return Ok(self.exit_code); }
                }
            }
            if (self.exit_code.is_some() && !self.wait_all) || !alive { return Ok(self.exit_code); }
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
        self.files.keys().chain(self.aliases.keys()).chain(self.lazy.keys()).any(|k| k.starts_with(&pre)) || self.vfs.range(pre.clone()..).next().map_or(false, |(k, _)| k.starts_with(&pre))
    }

    /// (mode, size) of a path, if it exists.
    fn lookup(&self, p: &str) -> Option<(u32, u64)> {
        match self.vfs.get(p) {
            Some(Node::File(d)) => Some((0o100644, d.borrow().len() as u64)),
            Some(Node::Dir) => Some((0o040700, 0)),
            Some(Node::Sock(_)) => Some((0o140755, 0)),
            Some(Node::Link(t)) => Some((0o120777, t.len() as u64)),
            None => match p {
                "/dev/null" | "/dev/tty" | "/dev/plonk/events" | "/dev/dsp" | "/dev/mixer" => Some((0o020666, 0)),
                _ => self.files.get(p).map(|d| (0o100755, d.len() as u64))
                    .or_else(|| self.lazy.get(p).map(|&n| (0o100755, n as u64)))
                    .or_else(|| self.aliases.get(p).and_then(|t| self.lookup(t)).map(|(_, n)| (0o100755, n)))
                    .or_else(|| self.is_dir(p).then_some((0o040755, 0))),
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
        for k in self.files.keys().chain(self.aliases.keys()).chain(self.lazy.keys()).filter(|k| k.starts_with(&pre)) {
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
        if self.trace_cap > 0 { self.tr(format!("path open {p} flags {flags:#x}")); }
        if self.lazy_target(&p) { return 0; } // not here yet: the call is retried
        let kind = match self.vfs.get(&p) {
            Some(Node::File(d)) => {
                if flags & 0xC0 == 0xC0 { return EEXIST; } // O_CREAT|O_EXCL
                if flags & 0x200 != 0 { d.borrow_mut().clear(); self.cow.remove(&(Rc::as_ptr(d) as usize)); } // O_TRUNC
                let d = d.clone();
                if flags & 0x203 != 0 { self.pristine.remove(&p); }
                Kind::File { data: d, pos: 0 }
            }
            Some(Node::Dir) => Kind::Dir { pos: 0 },
            Some(_) => return -6i32 as u32, // ENXIO
            None => match p.as_str() {
                "/dev/null" => Kind::Null,
                "/dev/tty" => Kind::Tty,
                "/dev/plonk/events" => Kind::Events,
                "/dev/dsp" => Kind::Dsp,
                "/dev/mixer" => Kind::Mixer,
                _ => {
                    if let Some(data) = self.promote(&p, flags & 3 != 0) {
                        if flags & 0x200 != 0 { data.borrow_mut().clear(); self.cow.remove(&(Rc::as_ptr(&data) as usize)); }
                        Kind::File { data, pos: 0 }
                    } else if self.is_dir(&p) { Kind::Dir { pos: 0 } }
                    else if flags & 0x40 != 0 && self.is_dir(parent(&p)) { // O_CREAT
                        let data: Data = Rc::new(RefCell::new(Vec::new()));
                        self.vfs.insert(p.clone(), Node::File(data.clone()));
                        Kind::File { data, pos: 0 }
                    } else { self.tr(format!("open {p} -> ENOENT")); return ENOENT; }
                }
            },
        };
        if flags & 0x10000 != 0 && !matches!(kind, Kind::Dir { .. }) { return ENOTDIR; } // O_DIRECTORY
        let fd = self.new_fd(Rc::new(RefCell::new(Ofd { kind, flags, path: p })), 0);
        if flags & 0x80000 != 0 { self.cloexec.insert(fd); } // O_CLOEXEC
        fd
    }

    fn close(&mut self, fd: u32) -> u32 {
        let Some(o) = self.fds.remove(&fd) else { return EBADF };
        self.cloexec.remove(&fd);
        if Rc::strong_count(&o) == 1 { if let Kind::Sock(s) = o.borrow().kind { self.socks[s].closed = true; self.socks[s].rx.clear(); } }
        0
    }

    fn fill_stat(&mut self, buf: u32, path: &str, mode: u32, size: u64) -> u32 {
        let mut s = [0u8; 96];
        let mut put = |o: usize, v: u32| s[o..o + 4].copy_from_slice(&v.to_le_bytes());
        let i = ino(path);
        put(0, 0x801); put(12, i); put(16, mode); put(20, 1); put(24, 1000); put(28, 1000);
        put(32, match path { "/dev/null" => 0x103, "/dev/tty" => 0x8800, _ => 0 }); // st_rdev: Wine compares stderr against /dev/null
        put(44, size as u32); put(52, 4096); put(56, ((size + 511) / 512) as u32);
        let t = 1_600_000_000u32; // fixed timestamps: Wine re-runs its prefix update when wine.inf looks newer than the prefix
        put(64, t); put(72, t); put(80, t); put(88, i);
        if self.mem.write_bytes(buf, &s).is_ok() { 0 } else { EFAULT }
    }

    fn stat_path(&mut self, dirfd: u32, path: u32, buf: u32, follow: bool) -> u32 {
        let raw = self.cstr(path);
        let p = self.resolve(dirfd, &raw, follow);
        if self.trace_cap > 0 { self.tr(format!("path stat {p} found={}", self.lookup(&p).is_some())); }
        match self.lookup(&p) { Some((mode, size)) => self.fill_stat(buf, &p, mode, size), None => ENOENT }
    }

    fn fstat(&mut self, fd: u32, buf: u32) -> u32 {
        let Some(o) = self.fds.get(&fd).cloned() else { return EBADF };
        let o = o.borrow();
        let (mode, size) = match &o.kind {
            Kind::File { data, .. } => (0o100644, data.borrow().len() as u64),
            Kind::Dir { .. } => (0o040700, 0),
            Kind::Null | Kind::Tty | Kind::Events | Kind::Dsp | Kind::Mixer => (0o020620, 0),
            Kind::Sock(_) => (0o140777, 0),
        };
        let path = if let Kind::Sock(s) = o.kind { format!("socket:{s}") } else { o.path.clone() };
        self.fill_stat(buf, &path, mode, size)
    }

    // ---- sockets and pipes ----

    fn new_sock(&mut self) -> usize { self.socks.push(Sock::default()); self.socks.len() - 1 }
    /// `flags`: access mode plus O_NONBLOCK / O_CLOEXEC bits (the SOCK_* flags have the same values).
    fn sock_fd(&mut self, s: usize, flags: u32) -> u32 {
        let fd = self.new_fd(Rc::new(RefCell::new(Ofd { kind: Kind::Sock(s), flags: flags & !0x80000, path: String::new() })), 0);
        if flags & 0x80000 != 0 { self.cloexec.insert(fd); }
        fd
    }
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
            Kind::Events => if self.events.is_empty() { 0 } else { events & POLLIN },
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
            1 => { let s = self.new_sock(); self.sock_fd(s, 2 | (a[1] & (O_NONBLOCK | 0x80000))) } // socket
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
                self.sock_fd(c, 2 | if call == 18 { a[3] & (O_NONBLOCK | 0x80000) } else { 0 })
            }
            6 | 7 => { if a[1] != 0 { let _ = self.mem.w16(a[1], 1); } w32(&mut self.mem, a[2], 2); 0 } // getsockname, getpeername
            8 => { // socketpair
                let (x, y) = (self.new_sock(), self.new_sock());
                self.socks[x].peer = Some(y);
                self.socks[y].peer = Some(x);
                let fl = 2 | (a[1] & (O_NONBLOCK | 0x80000));
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
                    for (i, o) in fds.into_iter().take(n).enumerate() {
                        let fd = self.new_fd(o, 0);
                        if a[2] & 0x4000_0000 != 0 { self.cloexec.insert(fd); } // MSG_CMSG_CLOEXEC
                        let _ = self.mem.w32(ctl + 12 + i as u32 * 4, fd);
                    }
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
            Kind::Null | Kind::Tty | Kind::Mixer => return Some(0),
            Kind::Dsp => return Some(if nb { EAGAIN } else { 0 }),
            Kind::Events => { // whole 32-byte input records
                let k = (n as usize / 32).min(self.events.len());
                if k == 0 { return if nb { Some(EAGAIN) } else { None }; }
                for i in 0..k { let e = self.events.pop_front().unwrap(); for (j, w) in e.iter().enumerate() { let _ = self.mem.w32(buf + (i * 32 + j * 4) as u32, *w); } }
                return Some(k as u32 * 32);
            }
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
                self.cow.remove(&(Rc::as_ptr(data) as usize));
                let mut d = data.borrow_mut();
                let s = if append { d.len() } else { at.unwrap_or(*pos) as usize };
                if d.len() < s + src.len() { d.resize(s + src.len(), 0); }
                d[s..s + src.len()].copy_from_slice(&src);
                if at.is_none() { *pos = (s + src.len()) as u64; }
                return n;
            }
            Kind::Tty => { self.stdout.extend_from_slice(&src); return n; }
            Kind::Null | Kind::Mixer => return n,
            Kind::Dsp => { self.audio_write(&src); return n; }
            Kind::Sock(s) => *s,
            Kind::Dir { .. } | Kind::Events => return EBADF,
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
        if flags & 0x21 == 1 { // MAP_SHARED of a file: alias the file's shared pages (not written back to the file contents)
            let Some(o) = self.fds.get(&fd).cloned() else { return EBADF };
            let file = match &o.borrow().kind { Kind::File { data, .. } => Some(data.clone()), _ => None };
            if let Some(data) = file {
                let si = match self.shared.iter().position(|s| Rc::ptr_eq(&s.data, &data)) { Some(i) => i, None => { self.shared.push(SharedFile { data: data.clone(), pages: Vec::new() }); self.shared.len() - 1 } };
                for i in 0..len / 4096 {
                    let pg = (off / 4096) as usize + i as usize;
                    let ptr = self.shared_page(si, pg);
                    self.mem.map_ext(a + i * 4096, ptr, prot != 0);
                }
                return a;
            }
        }
        if flags & 0x21 == 0 && off & 0xFFF == 0 { // MAP_PRIVATE of a file: the file's pages, each copied when first written to
            let Some(o) = self.fds.get(&fd).cloned() else { return EBADF };
            let data = match &o.borrow().kind { Kind::File { data, .. } => Some(data.clone()), _ => None };
            if let Some(data) = &data {
                let s = self.cow.entry(Rc::as_ptr(data) as usize).or_insert_with(|| SharedFile { data: data.clone(), pages: Vec::new() });
                let (flen, first) = (data.borrow().len(), (off / 4096) as usize);
                let n = ((flen + 4095) / 4096).saturating_sub(first).min((len / 4096) as usize);
                if s.pages.len() < first + n { s.pages.resize(first + n, core::ptr::null_mut()); }
                for i in 0..n {
                    let pg = first + i;
                    if s.pages[pg].is_null() {
                        let p = unsafe { std::alloc::alloc_zeroed(std::alloc::Layout::from_size_align(4096, 4096).unwrap()) };
                        assert!(!p.is_null(), "out of host memory");
                        let d = data.borrow();
                        unsafe { core::ptr::copy_nonoverlapping(d[pg * 4096..].as_ptr(), p, (flen - pg * 4096).min(4096)) };
                        s.pages[pg] = p;
                    }
                    self.mem.map_cow(a + (i as u32) * 4096, s.pages[pg], true);
                }
                let done = (n as u32) * 4096;
                self.mem.map(a + done, len - done, true); // past the end of the file: zero pages
                return a;
            }
        }
        self.mem.map(a, len, prot != 0 || flags & 0x20 == 0);
        if flags & 0x20 == 0 { // file-backed, private at an odd offset: a copy
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

    /// Page `pg` of a shared file's page store, created from the file contents on first use.
    fn shared_page(&mut self, si: usize, pg: usize) -> *mut u8 {
        let s = &mut self.shared[si];
        while s.pages.len() <= pg {
            let p = unsafe { std::alloc::alloc_zeroed(std::alloc::Layout::from_size_align(4096, 4096).unwrap()) };
            assert!(!p.is_null(), "out of host memory");
            let d = s.data.borrow();
            let at = s.pages.len() * 4096;
            if at < d.len() { let k = (d.len() - at).min(4096); unsafe { core::ptr::copy_nonoverlapping(d[at..].as_ptr(), p, k) }; }
            drop(d);
            s.pages.push(p);
        }
        s.pages[pg]
    }

    fn now(&mut self) -> (u64, u32) {
        self.clock_ns += self.clock_step_ns;
        (self.clock_ns / 1_000_000_000, (self.clock_ns % 1_000_000_000) as u32)
    }

    /// A thread that reads the clock again and again with almost nothing in between is busy-waiting for time to pass (Windows
    /// timeGetTime/QueryPerformanceCounter loops). After a few such reads, deschedule it for a millisecond of guest time:
    /// the guest then costs no host time while waiting. Returns true when the call has to be retried later.
    fn clock_poll_yield(&mut self) -> bool {
        let (me, now) = (self.cur, self.cpu.retired);
        if let Some(dl) = self.poll_deadline { // retry after a yield: go on once the millisecond has passed
            if self.clk_poll.2 >= 6 { if self.clock_ns >= dl { self.poll_deadline = None; self.clk_poll = (me, now, 0); return false; } return true; }
        }
        let c = &mut self.clk_poll;
        if c.0 == me && now.saturating_sub(c.1) < 3000 { c.2 += 1 } else { c.2 = 0 }
        c.0 = me; c.1 = now;
        if c.2 >= 6 { self.poll_deadline = Some(self.clock_ns + 1_000_000); return true; }
        false
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
        self.lazy_hit.set(false);
        if let Some(pr) = self.prof.as_mut() { *pr.1.entry((self.procs[self.cur.0].pid, nr)).or_default() += 1; }
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
                self.pristine.remove(&p);
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
                self.pristine.remove(&from);
                self.pristine.remove(&to);
                Some(match self.vfs.remove(&from) { Some(n) => { self.vfs.insert(to, n); 0 } None => ENOENT })
            }
            39 | 296 => { // mkdir, mkdirat
                let p = if nr == 39 { let s = self.cstr(a); self.resolve(CWD, &s, false) } else { let s = self.cstr(b); self.resolve(a, &s, false) };
                Some(if self.lookup(&p).is_some() { EEXIST } else if !self.is_dir(parent(&p)) { ENOENT } else { self.vfs.insert(p, Node::Dir); 0 })
            }
            41 => Some(match self.fds.get(&a).cloned() { Some(o) => self.new_fd(o, 0), None => EBADF }), // dup
            63 | 330 => Some(match self.fds.get(&a).cloned() { Some(o) => { if a != b { self.close(b); self.fds.insert(b, o); if nr == 330 && c & 0x80000 != 0 { self.cloexec.insert(b); } } b } None => EBADF }), // dup2, dup3
            42 | 331 => { // pipe, pipe2: a one-way socket pair
                let (x, y) = (self.new_sock(), self.new_sock());
                self.socks[x].peer = Some(y);
                self.socks[y].peer = Some(x);
                let fl = if nr == 331 { b & (O_NONBLOCK | 0x80000) } else { 0 };
                let (fx, fy) = (self.sock_fd(x, fl), self.sock_fd(y, fl | 1));
                let _ = self.mem.w32(a, fx);
                let _ = self.mem.w32(a + 4, fy);
                Some(0)
            }
            45 => { // brk
                if a >= self.brk_start && a < self.brk_start + 0x0800_0000 && (a <= self.brk || !self.maps_hit((self.brk + 0xFFF) & !0xFFF, a)) {
                    let (lo, hi) = ((self.brk + 0xFFF) & !0xFFF, (a + 0xFFF) & !0xFFF);
                    if hi > lo { self.mem.map(lo, hi - lo, true); }
                    self.brk = a;
                }
                Some(self.brk)
            }
            54 => Some(self.ioctl(a, b, c)),
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
                    Some(o) => { if let Kind::File { data, .. } = &o.borrow().kind { data.borrow_mut().resize(b as usize, 0); self.cow.remove(&(Rc::as_ptr(data) as usize)); 0 } else { EINVAL } }
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
            268 | 269 => { // statfs/fstatfs(64): a roomy disk (4 GB, 2 GB free) so installers' free-space checks pass
                let buf = c;
                let mut st = [0u8; 84];
                let put = |st: &mut [u8], off: usize, v: u64| st[off..off + 8].copy_from_slice(&v.to_le_bytes());
                st[0..4].copy_from_slice(&0xEF53u32.to_le_bytes());
                st[4..8].copy_from_slice(&4096u32.to_le_bytes());
                put(&mut st, 8, 1 << 20); put(&mut st, 16, 1 << 19); put(&mut st, 24, 1 << 19); put(&mut st, 32, 1 << 20); put(&mut st, 40, 1 << 19);
                st[56..60].copy_from_slice(&255u32.to_le_bytes());
                st[60..64].copy_from_slice(&4096u32.to_le_bytes());
                let _ = self.mem.write_bytes(buf, &st);
                Some(0)
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
            142 | 308 | 413 => { // _newselect, pselect6(_time64): only the "no descriptors" form, i.e. a sleep (Wine's infinite Sleep)
                if a != 0 && (b != 0 || c != 0 || d != 0) { Some(ENOSYS) } else {
                    let ms = if e == 0 { -1 } else {
                        let (s0, s1, s2) = (self.mem.r32(e).unwrap_or(0) as i64, self.mem.r32(e + 4).unwrap_or(0) as i64, self.mem.r32(e + 8).unwrap_or(0) as i64);
                        match nr { 142 => s0 * 1000 + s1 / 1000, 308 => s0 * 1000 + s1 / 1_000_000, _ => s0 * 1000 + s2 / 1_000_000 }
                    };
                    self.poll(0, 0, if ms < 0 { -1 } else { ms.max(1) })
                }
            }
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
            37 => { // kill: existence check, and SIGKILL (how wineserver ends a process); other signals are not delivered yet
                match self.procs.iter().position(|p| p.pid == a && p.exit.is_none()) {
                    Some(pi) => {
                        if b == 9 {
                            let me = self.cur;
                            self.switch(pi, 0);
                            self.proc_exit(137);
                            self.switch(me.0, me.1);
                        }
                        Some(0)
                    }
                    None => Some(if (a as i32) <= 0 { 0 } else { ESRCH }),
                }
            }
            238 | 270 => { // tkill, tgkill: signals are not delivered yet, but a fatal one aimed at ourselves ends the process
                let (tid, sig) = if nr == 238 { (a, b) } else { (b, c) };
                if matches!(sig, 6 | 9 | 15) && self.procs[self.cur.0].threads.iter().any(|t| t.tid == tid) { self.tr(format!("pid {} killed by signal {sig}", self.pid())); self.proc_exit(128 + sig); }
                Some(0)
            }
            175 | 186 => Some(0), // rt_sigprocmask, sigaltstack
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
                    0 | 1030 => { let fd = self.new_fd(o, c); if b == 1030 { self.cloexec.insert(fd); } fd } // F_DUPFD, F_DUPFD_CLOEXEC
                    1 => self.cloexec.contains(&a) as u32, // F_GETFD
                    2 => { if c & 1 != 0 { self.cloexec.insert(a); } else { self.cloexec.remove(&a); } 0 } // F_SETFD
                    3 => o.borrow().flags, // F_GETFL
                    4 => { let mut o = o.borrow_mut(); o.flags = (o.flags & 3) | (c & !3); 0 } // F_SETFL
                    5 | 12 => { let _ = self.mem.w16(c, 2); 0 } // F_GETLK(64): nothing is locked (F_UNLCK)
                    _ => 0, // locks
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
                if entry == u32::MAX { entry = 6 + self.cpu.tls.iter().position(|&b| b == 0).unwrap_or(3) as u32; let _ = self.mem.w32(a, entry); } // first free GDT TLS slot
                Some(if (6..=8).contains(&entry) {
                    self.cpu.tls[(entry - 6) as usize] = base;
                    if self.cpu.sel_gs >> 3 == entry { self.cpu.gs_base = base; }
                    if self.cpu.sel_fs >> 3 == entry { self.cpu.fs_base = base; }
                    0
                } else { EINVAL })
            }
            258 => { self.me().clear_tid = a; Some(self.me().tid) } // set_tid_address
            265 | 403 => { // clock_gettime (32-bit / 64-bit timespec)
                if self.clock_poll_yield() { None } else {
                let (s, ns) = self.now();
                let _ = self.mem.w32(b, s as u32);
                if nr == 403 { let _ = self.mem.w32(b + 4, (s >> 32) as u32); let _ = self.mem.w32(b + 8, ns); } else { let _ = self.mem.w32(b + 4, ns); }
                Some(0) }
            }
            311 => Some(0), // set_robust_list
            340 => { // prlimit64: report "unlimited"
                if d != 0 { for i in 0..4 { let _ = self.mem.w32(d + i * 4, u32::MAX); } }
                Some(0)
            }
            191 => { let _ = self.mem.w32(b, u32::MAX); let _ = self.mem.w32(b + 4, u32::MAX); Some(0) } // ugetrlimit
            0x504C_0000..=0x504C_00FF => Some(self.plonk_call(nr & 0xFF, [a, b, c, d, e])),
            0x504C_1000..=0x504C_1FFF => { // OpenGL: Plonk's opengl32.dll traps here, ebx -> the stdcall arguments
                match Self::gl_name((nr & 0xFFF) as usize) {
                    Some((name, nbytes)) => {
                        let args: Vec<u32> = (0..nbytes / 4).map(|i| self.mem.r32(a + i * 4).unwrap_or(0)).collect(); // a = ebx
                        match self.gl_call(name, &args) {
                            Ok(v) => Some(v.unwrap_or(0)),
                            Err(e) => { self.tr(format!("gl {name}: {e}")); Some(0) }
                        }
                    }
                    None => Some(0),
                }
            }
            355 => { for i in 0..b { let _ = self.mem.w8(a + i, (i * 37 + 11) as u8); } Some(b) } // getrandom
            _ => {
                let s = format!("syscall {nr}");
                if !self.unknown.contains(&s) { self.unknown.push(s); }
                Some(ENOSYS)
            }
        };
        let ret = if self.lazy_hit.replace(false) { None } else { ret }; // needs a file that is still to be supplied: retry
        match ret {
            Some(ret) => {
                if self.trace_cap > 0 { let pid = self.pid(); self.tr(format!("{pid} sys {nr}({a:#x}, {b:#x}, {c:#x}, {d:#x}) = {ret:#x}")); }
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

// ---- shortcuts (.lnk) in the Wine prefix: listing and parsing for the page's desktop icons and Start menu ----
const DRIVE_C: &str = "/home/user/.wine/drive_c/";
const SM_PROGRAMS: &str = "/start menu/programs/";

fn lnk_win_to_guest(w: &str) -> String {
    let b = w.as_bytes();
    if b.len() >= 2 && b[1] == b':' {
        let rest = w[2..].replace('\\', "/");
        let rest = rest.trim_start_matches('/');
        return if b[0].eq_ignore_ascii_case(&b'z') { format!("/{rest}") } else { format!("{DRIVE_C}{rest}") };
    }
    w.replace('\\', "/")
}

fn is_lnk(k: &str) -> bool {
    k.starts_with(DRIVE_C) && k.len() > 4 && k.is_char_boundary(k.len() - 4) && k[k.len() - 4..].eq_ignore_ascii_case(".lnk")
}

/// Parse an MS-SHLLINK file: (target, arguments, working dir, icon location).
fn parse_lnk(d: &[u8]) -> Option<(String, String, String, String)> {
    let u16at = |o: usize| d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32at = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    if u32at(0)? != 0x4c { return None; }
    let flags = u32at(0x14)?;
    let uni = flags & 0x80 != 0;
    let cstr = |o: usize| -> String { let s = d.get(o..).unwrap_or(&[]); String::from_utf8_lossy(&s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]).into_owned() };
    let wstr = |o: usize, max: usize| -> String {
        let mut v = Vec::new();
        for i in 0..max { match u16at(o + i * 2) { Some(0) | None => break, Some(c) => v.push(c as u16) } }
        String::from_utf16_lossy(&v)
    };
    let mut off = 0x4c;
    if flags & 1 != 0 { off += 2 + u16at(off)?; }
    let mut target = String::new();
    if flags & 2 != 0 {
        let sz = u32at(off)?;
        let hdr = u32at(off + 4)?;
        let lf = u32at(off + 8)?;
        if lf & 1 != 0 {
            let (base, suf) = if hdr >= 0x24 && u32at(off + 28)? != 0 {
                (wstr(off + u32at(off + 28)?, 520), wstr(off + u32at(off + 32)?, 520))
            } else { (cstr(off + u32at(off + 16)?), cstr(off + u32at(off + 24)?)) };
            target = if suf.is_empty() { base } else { format!("{}\\{}", base.trim_end_matches('\\'), suf) };
        }
        off += sz;
    }
    let mut rd = |present: bool| -> Option<String> {
        if !present { return Some(String::new()); }
        let n = u16at(off)?;
        off += 2;
        let s = if uni { let s = wstr(off, n); off += n * 2; s } else { let s = String::from_utf8_lossy(d.get(off..off + n)?).into_owned(); off += n; s };
        Some(s)
    };
    let _name = rd(flags & 4 != 0)?;
    let rel = rd(flags & 8 != 0)?;
    let cwd = rd(flags & 0x10 != 0)?;
    let args = rd(flags & 0x20 != 0)?;
    let icon = rd(flags & 0x40 != 0)?;
    // extra data: the environment-variable block carries the target as a plain path
    while let Some(sz) = u32at(off) {
        if sz < 8 { break; }
        if u32at(off + 4) == Some(0xA000_0001) && target.is_empty() { target = wstr(off + 268, 260); if target.is_empty() { target = cstr(off + 8); } }
        off += sz;
    }
    if target.is_empty() && !rel.is_empty() { target = rel; }
    if target.is_empty() { return None; }
    Some((target, args, cwd, icon))
}

impl LinuxProc {
    /// Every .lnk of the prefix: (guest path, contents).
    fn lnk_files(&self) -> BTreeMap<String, Vec<u8>> {
        let mut out: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for (k, d) in self.files.iter() { if is_lnk(k) && !self.vfs.contains_key(k) { out.insert(k.clone(), d.clone()); } }
        for (k, n) in self.vfs.range(DRIVE_C.to_string()..) {
            if !k.starts_with(DRIVE_C) { break; }
            if let (true, Node::File(d)) = (is_lnk(k), n) { out.insert(k.clone(), d.borrow().clone()); }
        }
        out
    }

    /// Cheap change counter for the shortcut files (hash of the paths and sizes).
    pub fn lnk_generation(&self) -> u32 {
        let mut h: u64 = 0;
        let mut add = |k: &str, n: usize| { h ^= k.bytes().fold(n as u64 ^ 1469598103934665603, |a, b| (a ^ b as u64).wrapping_mul(1099511628211)); };
        for (k, d) in self.files.iter() { if is_lnk(k) && !self.vfs.contains_key(k) { add(k, d.len()); } }
        for (k, n) in self.vfs.range(DRIVE_C.to_string()..) {
            if !k.starts_with(DRIVE_C) { break; }
            if let (true, Node::File(d)) = (is_lnk(k), n) { add(k, d.borrow().len()); }
        }
        (h ^ (h >> 32)) as u32
    }

    /// Case-insensitive lookup of a file (Windows paths are); returns the real guest path.
    pub fn find_ci(&self, p: &str) -> Option<String> {
        if self.vfs.contains_key(p) || self.files.contains_key(p) || self.lazy.contains_key(p) { return Some(p.to_string()); }
        self.vfs.iter().filter(|(_, n)| matches!(n, Node::File(_))).map(|(k, _)| k).chain(self.files.keys()).chain(self.lazy.keys()).find(|k| k.eq_ignore_ascii_case(p)).cloned()
    }

    /// Bytes of a file by (case-insensitive) guest path; a lazily-declared file is asked for and `None` is returned.
    pub fn read_ci(&self, p: &str) -> Option<Vec<u8>> { self.find_ci(p).and_then(|k| self.file_bytes(&k)) }

    /// One line per shortcut: kind, name, target (guest path), args, cwd (guest), icon source (guest), target (windows), group, .lnk (windows), tab separated.
    pub fn list_shortcuts(&self) -> String {
        let mut rows: Vec<String> = Vec::new();
        let pd = "programdata/microsoft/windows/start menu/programs/";
        for (path, data) in self.lnk_files() {
            let rel = &path[DRIVE_C.len()..];
            let low = rel.to_ascii_lowercase();
            let parts: Vec<&str> = rel.split('/').collect();
            let after = if let Some(i) = low.find(SM_PROGRAMS) { Some(&rel[i + SM_PROGRAMS.len()..]) } else if low.starts_with(pd) { Some(&rel[pd.len()..]) } else { None };
            let (kind, group) = if let Some(a) = after { ("start", a.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default()) }
                else if parts.len() == 4 && low.starts_with("users/") && parts[2].eq_ignore_ascii_case("desktop") { ("desktop", String::new()) } else { continue };
            let Some((target, args, cwd, icon)) = parse_lnk(&data) else { continue };
            let file = parts[parts.len() - 1];
            let name = &file[..file.len() - 4];
            let wlnk = format!("C:\\{}", rel.replace('/', "\\"));
            let gt = lnk_win_to_guest(&target);
            let ico = if icon.is_empty() { gt.clone() } else { lnk_win_to_guest(&icon) };
            let t = |s: &str| s.replace(['\t', '\n'], " ");
            rows.push(format!("{kind}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", t(name), t(&gt), t(&args), t(&lnk_win_to_guest(&cwd)), t(&ico), t(&target), t(&group), t(&wlnk)));
        }
        rows.join("\n")
    }
}
