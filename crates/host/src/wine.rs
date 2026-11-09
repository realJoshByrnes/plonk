//! Runs a PE through Wine's unmodified PE ntdll/kernel32/...: this module plays the role of
//! Wine's Unix side (ntdll.so) and wineserver. Syscalls and unix calls arrive as thunk traps.
use crate::sysnames::{NT, WIN32U};
use plonk_cpu::{Cpu, Exit, EAX, EBP, EBX, ECX, EDI, EDX, ESI, ESP};
use plonk_mem::Mem;

mod win32u;
pub use win32u::Wm;
use std::collections::{HashMap, VecDeque};

pub const GUEST_SIZE: u32 = 0x8000_0000;
const SYSCALL_THUNK: u32 = 0xF000_0000;
const UNIXCALL_THUNK: u32 = 0xF000_0010;
pub(crate) const CALL_RETURN_THUNK: u32 = 0xF000_0020;
const KUSER: u32 = 0x7ffe_0000;
const PEB: u32 = 0x7ffd_f000;
const TEB: u32 = 0x7ffd_d000;
const SCRATCH: u32 = 0x0010_0000; // bump arena for startup structures
const SCRATCH_SIZE: u32 = 0x0010_0000;
const STACK_SIZE: u32 = 0x0010_0000;
const VM_LO: u32 = 0x0020_0000;
const VM_HI: u32 = 0x7ffd_0000;

pub const STATUS_SUCCESS: u32 = 0;
pub const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;
pub const STATUS_INVALID_HANDLE: u32 = 0xC000_0008;
pub const STATUS_INVALID_PARAMETER: u32 = 0xC000_000D;
pub const STATUS_NO_MEMORY: u32 = 0xC000_0017;
pub const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xC000_0034;
pub const STATUS_CONFLICTING_ADDRESSES: u32 = 0xC000_0018;

pub enum Obj {
    File { path: String, pos: u32, dir: bool },
    Section { path: String, image: bool, size: u32 },
    Stdout,
    Key(String),
}

pub struct WineProc {
    pub mem: Mem,
    pub cpu: Cpu,
    /// Virtual filesystem: lowercase NT-ish path ("c:\\windows\\system32\\ntdll.dll") -> bytes.
    pub files: HashMap<String, Vec<u8>>,
    pub stdout: Vec<u8>,
    pub trace: VecDeque<String>,
    regions: Vec<(u32, u32)>, // sorted (start, size)
    handles: HashMap<u32, Obj>,
    next_handle: u32,
    scratch: u32,
    exit: Option<u32>,
    pub ntdll_base: u32,
    images: Vec<(u32, String)>,
    keys: std::collections::HashSet<String>,
    exe_path: String,
    dcattr_next: u32,
    gdi_table: u32,
    /// Debug: log eip + eax/ebx/esi/esp for every instruction executed with eip in [lo, hi).
    pub step_trace: Option<(u32, u32)>,
    pub wm: Wm,
    guest_returned: bool,
    ki_user_callback: u32,
    callback_ret: Option<(u32, u32, u32)>,
    /// Debug: win32u syscalls with no handler return 0 and are recorded here instead of aborting.
    pub lenient: bool,
    pub stubbed: Vec<String>,
    values: HashMap<(String, String), (u32, Vec<u8>)>,
    ticks: u64,
    nls: HashMap<String, (u32, u32)>,
}

type Res<T> = Result<T, String>;

fn align(v: u32, a: u32) -> u32 { (v + a - 1) & !(a - 1) }

impl WineProc {
    pub fn new() -> Self {
        let mut p = WineProc {
            mem: Mem::new_owned(GUEST_SIZE), cpu: Cpu::new(), files: HashMap::new(), stdout: Vec::new(),
            trace: VecDeque::new(), regions: Vec::new(), handles: HashMap::new(), next_handle: 0x40,
            scratch: SCRATCH, exit: None, ntdll_base: 0, images: Vec::new(), keys: Default::default(), exe_path: String::new(), dcattr_next: 0, gdi_table: 0, step_trace: None, wm: Wm::new(), guest_returned: false, ki_user_callback: 0, callback_ret: None, lenient: false, stubbed: Vec::new(), values: HashMap::new(), ticks: 0, nls: HashMap::new(),
        };
        p.regions.push((SCRATCH, SCRATCH_SIZE));
        p
    }

    /// Native convenience: load every *.dll from `dir` as c:\windows\system32\<name> and
    /// every file in `dir/nls` as nls\<name>.
    pub fn load_dir(&mut self, dir: &str) {
        let rd = |p: &str| std::fs::read_dir(p).into_iter().flatten().flatten().collect::<Vec<_>>();
        for e in rd(dir) {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            if n.ends_with(".dll") { self.files.insert(format!("c:\\windows\\system32\\{n}"), std::fs::read(e.path()).unwrap()); }
        }
        for e in rd(&format!("{dir}/nls")) {
            self.files.insert(format!("nls\\{}", e.file_name().to_string_lossy()), std::fs::read(e.path()).unwrap());
        }
    }

    /// Debug: which loaded image (base, name) contains `addr`.
    pub fn module_at(&self, addr: u32) -> Option<(u32, &str)> {
        self.images.iter().filter(|(b, _)| *b <= addr).max_by_key(|(b, _)| *b).map(|(b, n)| (*b, n.as_str()))
    }

    // ---- helpers ----
    fn log(&mut self, s: String) {
        if self.trace.len() >= 400 { self.trace.pop_front(); }
        self.trace.push_back(s);
    }
    fn r32(&self, a: u32) -> Res<u32> { self.mem.r32(a).map_err(|_| format!("read fault @{a:#x}")) }
    fn w32(&mut self, a: u32, v: u32) -> Res<()> { self.mem.w32(a, v).map_err(|_| format!("write fault @{a:#x}")) }
    fn w16(&mut self, a: u32, v: u16) -> Res<()> { self.mem.w16(a, v).map_err(|_| format!("write fault @{a:#x}")) }
    fn arg(&self, i: u32) -> u32 { self.mem.r32(self.cpu.r[ESP].wrapping_add(4 + 4 * i)).unwrap_or(0) }
    /// Syscall args: [esp] = return into stub, [esp+4] = caller's return, args follow.
    fn sarg(&self, i: u32) -> u32 { self.arg(i + 1) }

    fn scratch_alloc(&mut self, n: u32) -> u32 {
        let a = align(self.scratch, 16);
        self.scratch = a + n;
        assert!(self.scratch <= SCRATCH + SCRATCH_SIZE, "scratch arena full");
        a
    }
    fn put_wstr(&mut self, s: &str, nul: bool) -> (u32, u32) {
        let u: Vec<u16> = s.encode_utf16().collect();
        let a = self.scratch_alloc((u.len() as u32 + 1) * 2);
        for (i, c) in u.iter().enumerate() { self.mem.w16(a + i as u32 * 2, *c).unwrap(); }
        let _ = nul;
        (a, u.len() as u32 * 2)
    }
    fn put_ustr(&mut self, at: u32, s: &str) {
        let (a, len) = self.put_wstr(s, true);
        self.mem.w16(at, len as u16).unwrap();
        self.mem.w16(at + 2, (len + 2) as u16).unwrap();
        self.mem.w32(at + 4, a).unwrap();
    }
    fn get_ustr(&self, at: u32) -> Res<String> {
        let (len, buf) = (self.mem.r16(at).map_err(|_| "ustr")? as u32, self.r32(at + 4)?);
        let mut v = Vec::new();
        for i in 0..len / 2 { v.push(self.mem.r16(buf + i * 2).map_err(|_| "ustr buf")?); }
        Ok(String::from_utf16_lossy(&v))
    }

    // ---- virtual address space ----
    fn region_free(&self, start: u32, size: u32) -> bool {
        let end = start as u64 + size as u64;
        self.regions.iter().all(|&(s, z)| end <= s as u64 || start as u64 >= s as u64 + z as u64)
    }
    fn vm_reserve(&mut self, base: Option<u32>, size: u32) -> Option<u32> {
        let size = align(size.max(1), 0x1000);
        let start = match base {
            Some(b) => { let b = b & !0xfff; if self.region_free(b, size) { b } else { return None } }
            None => {
                let mut c = VM_LO;
                loop {
                    if c as u64 + size as u64 > VM_HI as u64 { return None; }
                    match self.regions.iter().find(|&&(s, z)| (c as u64) < s as u64 + z as u64 && c as u64 + size as u64 > s as u64) {
                        Some(&(s, z)) => c = align(s + z, 0x10000),
                        None => break c,
                    }
                }
            }
        };
        let pos = self.regions.partition_point(|&(s, _)| s < start);
        self.regions.insert(pos, (start, size));
        Some(start)
    }
    fn vm_release(&mut self, base: u32) {
        if let Some(i) = self.regions.iter().position(|&(s, _)| s == base) {
            let (s, z) = self.regions.remove(i);
            let zeros = vec![0u8; z as usize];
            let _ = self.mem.write_bytes(s, &zeros);
        }
    }

    /// Resolve an OBJECT_ATTRIBUTES to a normalized VFS path.
    fn oa_path(&self, oa: u32) -> Res<String> {
        let (root, name) = (self.r32(oa + 4)?, self.r32(oa + 8)?);
        let n = if name != 0 { self.get_ustr(name)? } else { String::new() };
        let mut p = Self::nt_path(&n);
        if root != 0 {
            if let Some(Obj::File { path, .. }) = self.handles.get(&root) { p = format!("{}\\{}", path, p); }
        }
        Ok(p.trim_end_matches('\\').to_string())
    }

    /// SECTION_IMAGE_INFORMATION (0x30 bytes) from a PE in the VFS.
    fn fill_image_info(&mut self, path: &str, buf: u32) -> Res<()> {
        let d = self.files.get(path).cloned().ok_or("image info: no file")?;
        let pe = u32::from_le_bytes(d[0x3C..0x40].try_into().unwrap()) as usize;
        let o = pe + 24;
        let u16at = |x: usize| u16::from_le_bytes([d[x], d[x + 1]]) as u32;
        let u32at = |x: usize| u32::from_le_bytes(d[x..x + 4].try_into().unwrap());
        let vals: [(u32, u32); 12] = [
            (0, u32at(o + 28) + u32at(o + 16)), (4, 0), (8, u32at(o + 72)), (0xC, u32at(o + 80)), (0x10, u16at(o + 68)),
            (0x14, u16at(o + 48) | u16at(o + 50) << 16), (0x18, u16at(o + 40) | u16at(o + 42) << 16),
            (0x1C, u16at(pe + 22) | u16at(o + 70) << 16), (0x20, u16at(pe + 4) | 1 << 16), (0x24, 0), (0x28, d.len() as u32), (0x2C, u32at(o + 64)),
        ];
        for (off, v) in vals { self.w32(buf + off, v)?; }
        Ok(())
    }

    /// Map an NLS data file (VFS key "nls\\<name>") into guest memory once; returns (addr, size).
    fn nls_map(&mut self, name: &str) -> Res<(u32, u32)> {
        let key = format!("nls\\{name}");
        if let Some(&r) = self.nls.get(&key) { return Ok(r); }
        let data = self.files.get(&key).cloned().ok_or(format!("missing NLS file {name}"))?;
        let b = self.vm_reserve(None, data.len() as u32).ok_or("nls: no VM")?;
        self.mem.write_bytes(b, &data).map_err(|_| "nls write")?;
        self.nls.insert(key, (b, data.len() as u32));
        Ok((b, data.len() as u32))
    }

    // ---- boot ----
    fn nt_path(p: &str) -> String {
        let p = p.strip_prefix("\\??\\").unwrap_or(p);
        p.replace('/', "\\").to_ascii_lowercase()
    }

    /// Map an image from the VFS (headers+sections+relocs), registering the VM region.
    fn map_image_file(&mut self, path: &str) -> Res<plonk_pe::Image> {
        let data = self.files.get(path).ok_or_else(|| format!("no such file {path}"))?.clone();
        let pref = {
            let pe = u32::from_le_bytes(data[0x3C..0x40].try_into().unwrap()) as usize;
            u32::from_le_bytes(data[pe + 24 + 28..pe + 24 + 32].try_into().unwrap())
        };
        let size = {
            let pe = u32::from_le_bytes(data[0x3C..0x40].try_into().unwrap()) as usize;
            u32::from_le_bytes(data[pe + 24 + 56..pe + 24 + 60].try_into().unwrap())
        };
        let base = match self.vm_reserve(Some(pref), size) { Some(b) => b, None => self.vm_reserve(None, size).ok_or("no VM for image")? };
        self.images.push((base, path.to_string()));
        plonk_pe::map_ex(&data, &mut self.mem, Some(base), false)
    }

    pub fn boot(&mut self, exe_path: &str) -> Res<()> {
        let exe_path = Self::nt_path(exe_path);
        let ntdll = self.map_image_file("c:\\windows\\system32\\ntdll.dll")?;
        self.ntdll_base = ntdll.base;
        let exp = |n: &str, p: &Self| plonk_pe::find_export(&p.mem, ntdll.base, n).ok_or(format!("ntdll export {n} missing"));
        let (sd, ud, uh) = (exp("__wine_syscall_dispatcher", self)?, exp("__wine_unix_call_dispatcher", self)?, exp("__wine_unixlib_handle", self)?);
        let (ldr_init, thread_start) = (exp("LdrInitializeThunk", self)?, exp("RtlUserThreadStart", self)?);
        self.ki_user_callback = exp("KiUserCallbackDispatcher", self)?;
        self.w32(sd, SYSCALL_THUNK)?;
        self.w32(ud, UNIXCALL_THUNK)?;
        self.w32(uh, 1)?;
        self.w32(uh + 4, 0)?;

        self.exe_path = exe_path.clone();
        let exe = self.map_image_file(&exe_path)?;

        // KUSER_SHARED_DATA
        self.vm_reserve(Some(KUSER), 0x1000).ok_or("kuser")?;
        self.w32(KUSER + 0x4, 0x0fa0_0000)?;
        let root: Vec<u16> = "C:\\windows".encode_utf16().collect();
        for (i, c) in root.iter().enumerate() { self.w16(KUSER + 0x30 + i as u32 * 2, *c)?; }
        self.w32(KUSER + 0x264, 1)?; // NtProductType
        self.w32(KUSER + 0x268, 1)?; // ProductTypeIsValid (BOOLEAN) low byte
        self.w32(KUSER + 0x26C, 10)?; // NtMajorVersion
        self.w32(KUSER + 0x270, 0)?;
        self.w32(KUSER + 0x2D4, 1)?; // ActiveProcessorCount-ish; harmless

        // PEB / TEB / stack
        self.vm_reserve(Some(PEB), 0x1000).ok_or("peb")?;
        self.vm_reserve(Some(TEB), 0x2000).ok_or("teb")?;
        let stack = self.vm_reserve(None, STACK_SIZE).ok_or("stack")?;
        let stack_top = stack + STACK_SIZE;
        self.w32(PEB + 0x08, exe.base)?;
        self.gdi_table = self.vm_reserve(None, 0x10000 * 24).ok_or("gdi table")?;
        self.w32(PEB + 0x94, self.gdi_table)?; // GdiSharedHandleTable
        self.w32(PEB + 0x64, 1)?; // NumberOfProcessors
        self.w32(PEB + 0xA4, 10)?;
        self.w32(PEB + 0xA8, 0)?;
        self.mem.w16(PEB + 0xAC, 19045).map_err(|_| "peb")?;
        self.w32(PEB + 0xB0, 2)?;
        self.w32(TEB, 0xFFFF_FFFF)?;
        self.w32(TEB + 0x04, stack_top)?;
        self.w32(TEB + 0x08, stack)?;
        self.w32(TEB + 0x18, TEB)?;
        self.w32(TEB + 0x20, 0x20)?; // pid
        self.w32(TEB + 0x24, 0x24)?; // tid
        self.w32(TEB + 0x30, PEB)?;
        self.w32(TEB + 0xC0, SYSCALL_THUNK)?;
        self.cpu.fs_base = TEB;

        // process parameters
        let std_out = self.new_handle(Obj::Stdout);
        let pp = self.scratch_alloc(0x400);
        self.w32(pp, 0x400)?;
        self.w32(pp + 4, 0x298)?;
        self.w32(pp + 8, 1)?; // PROCESS_PARAMS_FLAG_NORMALIZED
        self.w32(pp + 0x1C, std_out)?;
        self.w32(pp + 0x20, std_out)?;
        self.put_ustr(pp + 0x24, "C:\\");
        self.put_ustr(pp + 0x30, "C:\\windows\\system32");
        self.put_ustr(pp + 0x38, &format!("C:{}", &exe_path[2..]));
        self.put_ustr(pp + 0x40, &format!("C:{}", &exe_path[2..]));
        self.put_ustr(pp + 0x70, "");
        self.put_ustr(pp + 0x78, "");
        self.put_ustr(pp + 0x80, "");
        self.put_ustr(pp + 0x88, "");
        let env = "SystemRoot=C:\\windows\0SystemDrive=C:\0PATH=C:\\windows\\system32\0\0";
        let u: Vec<u16> = env.encode_utf16().collect();
        let ea = self.scratch_alloc(u.len() as u32 * 2);
        for (i, c) in u.iter().enumerate() { self.w16(ea + i as u32 * 2, *c)?; }
        self.w32(pp + 0x48, ea)?;
        self.w32(pp + 0x290, u.len() as u32 * 2)?;
        self.w32(PEB + 0x10, pp)?;

        // initial CONTEXT at top of stack, then enter LdrInitializeThunk like call_init_thunk does
        let esp0 = stack_top - 16;
        let ctx = (esp0 & !3) - 0x2CC;
        self.w32(ctx, 0x1_0007)?; // CONTEXT_i386|CONTROL|INTEGER|SEGMENTS
        for (off, v) in [(0x8C, 0x2B), (0x90, 0x53), (0x94, 0x2B), (0x98, 0x2B), (0xA4, 0), (0xB0, exe.entry), (0xB8, thread_start), (0xBC, 0x23), (0xC0, 0x202), (0xC4, esp0), (0xC8, 0x2B)] {
            self.w32(ctx + off, v)?;
        }
        let mut sp = ctx;
        for v in [0, 0, 0, ctx, 0xdead_babe] { sp -= 4; self.w32(sp, v)?; }
        self.cpu.r[ESP] = sp;
        self.cpu.eip = ldr_init;
        self.cpu.thunk = (SYSCALL_THUNK, SYSCALL_THUNK + 0x40);
        Ok(())
    }

    fn new_handle(&mut self, o: Obj) -> u32 {
        let h = self.next_handle;
        self.next_handle += 4;
        self.handles.insert(h, o);
        h
    }

    // ---- run loop ----
    /// Run the guest until process exit. Each loop iteration handles one guest->host trap.
    pub fn run(&mut self, max_syscalls: u64) -> Res<u32> {
        for _ in 0..max_syscalls {
            self.pump()?;
            if let Some(c) = self.exit { return Ok(c); }
        }
        Err("syscall budget exhausted".into())
    }

    /// Execute guest code until the next trap and handle it.
    fn pump(&mut self) -> Res<()> {
        let exit = if let Some((lo, hi)) = self.step_trace {
            loop {
                if self.cpu.eip >= lo && self.cpu.eip < hi {
                    let r = self.cpu.r;
                    eprintln!("@{:x} eax={:x} ebx={:x} esi={:x} esp={:x}", self.cpu.eip, r[0], r[3], r[6], r[4]);
                }
                if let Err(e) = self.cpu.step(&mut self.mem) { break e; }
            }
        } else {
            self.cpu.run(&mut self.mem, 500_000_000)
        };
        match exit {
            Exit::Thunk(SYSCALL_THUNK) => self.syscall(),
            Exit::Thunk(UNIXCALL_THUNK) => self.unixcall(),
            Exit::Thunk(CALL_RETURN_THUNK) => { self.guest_returned = true; Ok(()) }
            e => Err(format!("cpu stopped: {:?} eip={:#x}", e, self.cpu.eip)),
        }
    }

    /// Call into guest user32 like win32u's KeUserModeCallback: runs KiUserCallbackDispatcher(id, args, len)
    /// on the current guest stack until it executes NtCallbackReturn. Returns (return bytes, status).
    pub(crate) fn user_callback(&mut self, id: u32, args: &[u8]) -> Res<(Vec<u8>, u32)> {
        let saved = self.cpu.clone();
        let mut sp = (self.cpu.r[ESP].wrapping_sub(0x400)) & !15;
        sp -= align(args.len() as u32, 16);
        let args_ptr = sp;
        self.mem.write_bytes(args_ptr, args).map_err(|_| "callback args")?;
        for v in [args.len() as u32, args_ptr, id, 0] { sp -= 4; self.w32(sp, v)?; }
        self.cpu.r[ESP] = sp;
        self.cpu.eip = self.ki_user_callback;
        self.callback_ret = None;
        while self.callback_ret.is_none() {
            self.pump()?;
            if self.exit.is_some() { return Err("process exited inside callback".into()); }
        }
        let (ptr, len, status) = self.callback_ret.take().unwrap();
        let ret = if len > 0 { self.mem.read_bytes(ptr, len).map_err(|_| "callback ret")?.to_vec() } else { Vec::new() };
        self.cpu = saved;
        Ok((ret, status))
    }

    /// wineserver request issued by PE code (`wine_server_call`). Only what the GUI path needs.
    fn server_call(&mut self, req: u32) -> Res<()> {
        let id = self.r32(req)? as usize;
        let name = crate::reqnames::REQUESTS.get(id).copied().unwrap_or("?");
        self.log(format!("  (server request {name})"));
        match name {
            "post_quit_message" => { let c = self.r32(req + 12)?; self.post_quit(c); }
            _ => return Err(format!("unimplemented server request {name}")),
        }
        self.w32(req, 0)?; // reply.error = STATUS_SUCCESS
        self.w32(req + 4, 0)?;
        Ok(())
    }

    pub(crate) fn ret_stdcall(&mut self, status: u32) -> Res<()> {
        let ra = self.r32(self.cpu.r[ESP])?;
        self.cpu.r[ESP] = self.cpu.r[ESP].wrapping_add(4);
        self.cpu.r[EAX] = status;
        self.cpu.eip = ra;
        Ok(())
    }

    fn unixcall(&mut self) -> Res<()> {
        let (hlo, code, args) = (self.arg(0), self.arg(2), self.arg(3));
        let module = self.module_at(hlo).map(|(_, n)| n.to_string()).unwrap_or_default();
        self.log(format!("unixcall {module} code={code} args={args:#x}"));
        let st = if module.ends_with("win32u.dll") && code == 0 { 0 }
        else if hlo == 1 && code == 3 { self.server_call(args)?; 0 }
        else if hlo == 1 && code == 2 { 0 } // wine_dbg_write
        else {
            return Err(format!("unimplemented unix call {module} code={code}"));
        };
        // stdcall: 4 dword args (u64 handle, code, args) cleaned by the callee
        let ra = self.r32(self.cpu.r[ESP])?;
        self.cpu.r[ESP] = self.cpu.r[ESP].wrapping_add(4 + 16);
        self.cpu.r[EAX] = st;
        self.cpu.eip = ra;
        Ok(())
    }

    fn syscall(&mut self) -> Res<()> {
        let id = self.cpu.r[EAX];
        let (table, num) = ((id >> 12) & 3, (id & 0xfff) as usize);
        let (name, nbytes) = match if table == 0 { NT.get(num) } else { WIN32U.get(num) } {
            Some(&(n, b)) => (n, b as u32),
            None => return Err(format!("bad syscall id {id:#x}")),
        };
        let args: Vec<u32> = (0..nbytes / 4).map(|i| self.sarg(i)).collect();
        let r = self.dispatch(name, &args);
        let st = match &r { Ok(Some(s)) => *s, Ok(None) => return Ok(()), Err(_) => 0xDEAD_0000 };
        let a: Vec<String> = args.iter().map(|a| format!("{a:x}")).collect();
        self.log(format!("{name}({}) = {st:#x}", a.join(",")));
        match r {
            Err(e) => {
                let ra = self.arg(0);
                let where_ = self.module_at(ra).map(|(b, n)| format!("{n}+{:#x}", ra - b)).unwrap_or_default();
                Err(format!("{name}: {e} [caller {ra:#x} {where_}]"))
            }
            Ok(_) => self.ret_stdcall(st),
        }
    }

    /// Ok(Some(status)) = return to caller; Ok(None) = handler already set the cpu state.
    fn dispatch(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        if name.starts_with("NtUser") || name.starts_with("NtGdi") {
            if let Some(r) = self.win32u_call(name, a)? { return Ok(Some(r)); }
        }
        Ok(Some(match name {
            "NtAllocateVirtualMemory" => {
                let (pbase, psize) = (a[1], a[3]);
                let (base, size) = (self.r32(pbase)?, self.r32(psize)?);
                let typ = a[4];
                let start = if base != 0 && typ & 0x2000 == 0 { Some(base) } else if base != 0 { Some(base) } else { None };
                // committing inside an existing reservation is a no-op
                if let Some(b) = start {
                    if let Some(&(s, z)) = self.regions.iter().find(|&&(s, z)| b >= s && b < s + z) {
                        if typ & 0x1000 != 0 && typ & 0x2000 == 0 { self.w32(pbase, b & !0xfff)?; self.w32(psize, align(size, 0x1000))?; let _ = (s, z); return Ok(Some(0)); }
                    }
                }
                match self.vm_reserve(start, size) {
                    Some(b) => { self.w32(pbase, b)?; self.w32(psize, align(size, 0x1000))?; 0 }
                    None => if start.is_some() { STATUS_CONFLICTING_ADDRESSES } else { STATUS_NO_MEMORY },
                }
            }
            "NtFreeVirtualMemory" => {
                let base = self.r32(a[1])?;
                if a[3] & 0x8000 != 0 { self.vm_release(base); }
                0
            }
            "NtProtectVirtualMemory" => { if a[4] != 0 { self.w32(a[4], 4)?; } 0 }
            "NtContinue" => {
                let c = a[0];
                let rd = |p: &Self, o: u32| p.r32(c + o);
                for (i, off) in [(EDI, 0x9C), (ESI, 0xA0), (EBX, 0xA4), (EDX, 0xA8), (ECX, 0xAC), (EAX, 0xB0), (EBP, 0xB4), (ESP, 0xC4)] {
                    self.cpu.r[i] = rd(self, off)?;
                }
                self.cpu.eip = rd(self, 0xB8)?;
                self.cpu.eflags = rd(self, 0xC0)? & 0xfff | 2;
                let s = format!("NtContinue -> eip={:#x} esp={:#x}", self.cpu.eip, self.cpu.r[ESP]);
                self.log(s);
                return Ok(None);
            }
            "NtQueryVirtualMemory" => {
                let (addr, class, buf, len, retlen) = (a[1], a[2], a[3], a[4], a[5]);
                if class == 1000 {
                    // MemoryWineUnixFuncs: the unixlib handle for a module; we use its base address.
                    let name = self.module_at(addr).map(|(_, n)| n.to_string()).unwrap_or_default();
                    self.log(format!("  (unixlib handle for {name})"));
                    self.w32(buf, addr)?;
                    self.w32(buf + 4, 0)?;
                    return Ok(Some(0));
                }
                if class != 0 { return Err(format!("QueryVirtualMemory class {class}")); }
                if len < 28 { return Ok(Some(0xC000_0004)); }
                let page = addr & !0xfff;
                let hit = self.regions.iter().copied().find(|&(s, z)| page >= s && page < s + z);
                let (base, alloc_base, prot, size, state, typ) = match hit {
                    Some((s, z)) => {
                        let img = self.images.iter().any(|(b, _)| *b == s);
                        (page, s, if img { 0x40 } else { 4 }, s + z - page, 0x1000, if img { 0x100_0000 } else { 0x2_0000 })
                    }
                    None => {
                        let next = self.regions.iter().map(|&(s, _)| s).find(|&s| s > page).unwrap_or(0x8000_0000);
                        (page, 0, 1, next - page, 0x1_0000, 0)
                    }
                };
                for (i, v) in [base, alloc_base, if alloc_base != 0 { prot } else { 0 }, size, state, prot, typ].iter().enumerate() { self.w32(buf + i as u32 * 4, *v)?; }
                if retlen != 0 { self.w32(retlen, 28)?; }
                0
            }
            "NtOpenFile" | "NtCreateFile" => {
                let (hp, oa, iosb) = (a[0], a[2], a[3]);
                let path = self.oa_path(oa)?;
                self.log(format!("  (open {path})"));
                let is_dir = self.files.keys().any(|k| k.starts_with(&format!("{path}\\")));
                if !self.files.contains_key(&path) && !is_dir && !path.is_empty() && path != "c:" {
                    self.w32(iosb, STATUS_OBJECT_NAME_NOT_FOUND)?;
                    return Ok(Some(STATUS_OBJECT_NAME_NOT_FOUND));
                }
                let h = self.new_handle(Obj::File { path, pos: 0, dir: is_dir || self.files.get("").is_some() });
                self.w32(hp, h)?;
                self.w32(iosb, 0)?;
                self.w32(iosb + 4, 1)?;
                0
            }
            "NtCreateSection" => {
                let (hp, attrs, fh) = (a[0], a[5], a[6]);
                let (path, size) = match self.handles.get(&fh) {
                    Some(Obj::File { path, .. }) => (path.clone(), self.files.get(path).map_or(0, |f| f.len() as u32)),
                    _ => return Ok(Some(STATUS_INVALID_HANDLE)),
                };
                let h = self.new_handle(Obj::Section { path, image: attrs & 0x0100_0000 != 0, size });
                self.w32(hp, h)?;
                0
            }
            "NtMapViewOfSection" => {
                let (sec, pbase, psize) = (a[0], a[2], a[6]);
                let (path, image, size) = match self.handles.get(&sec) {
                    Some(Obj::Section { path, image, size }) => (path.clone(), *image, *size),
                    _ => return Ok(Some(STATUS_INVALID_HANDLE)),
                };
                if image {
                    let img = self.map_image_file(&path)?;
                    self.w32(pbase, img.base)?;
                    self.w32(psize, img.size)?;
                    if img.base != img.pref_base { 0x4000_0003 } else { 0 }
                } else {
                    let want = self.r32(pbase)?;
                    let off = if a[5] != 0 { self.r32(a[5])? } else { 0 };
                    let vs = self.r32(psize)?;
                    let len = if vs == 0 { size.saturating_sub(off) } else { vs };
                    let b = match self.vm_reserve(if want != 0 { Some(want) } else { None }, len) { Some(b) => b, None => return Ok(Some(STATUS_NO_MEMORY)) };
                    let data = self.files.get(&path).cloned().unwrap_or_default();
                    let n = (len as usize).min(data.len().saturating_sub(off as usize));
                    self.mem.write_bytes(b, &data[off as usize..off as usize + n]).map_err(|_| "map data")?;
                    self.w32(pbase, b)?;
                    self.w32(psize, align(len, 0x1000))?;
                    0
                }
            }
            "NtQuerySection" => {
                let (sec, class, buf, len, retlen) = (a[0], a[1], a[2], a[3], a[4]);
                let path = match self.handles.get(&sec) { Some(Obj::Section { path, .. }) => path.clone(), _ => return Ok(Some(STATUS_INVALID_HANDLE)) };
                if class != 1 { return Err(format!("QuerySection class {class}")); }
                if len < 0x30 { return Ok(Some(0xC000_0004)); }
                self.fill_image_info(&path, buf)?;
                if retlen != 0 { self.w32(retlen, 0x30)?; }
                0
            }
            "NtQueryInformationProcess" => {
                let (class, buf, len, retlen) = (a[1], a[2], a[3], a[4]);
                match class {
                    37 => {
                        if len < 0x30 { return Ok(Some(0xC000_0004)); }
                        let exe = self.exe_path.clone();
                        self.fill_image_info(&exe, buf)?;
                        if retlen != 0 { self.w32(retlen, 0x30)?; }
                    }
                    26 | 7 => { self.w32(buf, 0)?; if retlen != 0 { self.w32(retlen, 4)?; } }
                    31 => { self.w32(buf, 1)?; if retlen != 0 { self.w32(retlen, 4)?; } }
                    0 => {
                        for (i, v) in [0u32, PEB, 1, 8, 0x20, 0].iter().enumerate() { self.w32(buf + i as u32 * 4, *v)?; }
                        if retlen != 0 { self.w32(retlen, 24)?; }
                    }
                    _ => return Err(format!("QueryInformationProcess class {class}")),
                }
                0
            }
            "NtQueryInformationFile" => {
                let (h, iosb, buf, len, class) = (a[0], a[1], a[2], a[3], a[4]);
                let (path, pos) = match self.handles.get(&h) { Some(Obj::File { path, pos, .. }) => (path.clone(), *pos), _ => return Ok(Some(STATUS_INVALID_HANDLE)) };
                let fsz = self.files.get(&path).map_or(0, |f| f.len() as u32);
                let n = match class {
                    5 => { for i in 0..6 { self.w32(buf + i * 4, 0)?; } self.w32(buf + 8, fsz)?; self.w32(buf, fsz)?; self.w32(buf + 16, 1)?; 24 }
                    14 => { self.w32(buf, pos)?; self.w32(buf + 4, 0)?; 8 }
                    _ => return Err(format!("QueryInformationFile class {class}")),
                };
                if len < n { return Ok(Some(0xC000_0004)); }
                self.w32(iosb, 0)?; self.w32(iosb + 4, n)?;
                0
            }
            "NtReadFile" => {
                let (h, iosb, buf, len, poff) = (a[0], a[4], a[5], a[6], a[7]);
                let off = if poff != 0 { Some(self.r32(poff)?) } else { None };
                let (path, pos) = match self.handles.get(&h) { Some(Obj::File { path, pos, .. }) => (path.clone(), *pos), _ => return Ok(Some(STATUS_INVALID_HANDLE)) };
                let data = self.files.get(&path).cloned().unwrap_or_default();
                let at = off.unwrap_or(pos) as usize;
                if at >= data.len() { self.w32(iosb, 0xC000_0011)?; self.w32(iosb + 4, 0)?; return Ok(Some(0xC000_0011)); }
                let n = (len as usize).min(data.len() - at);
                self.mem.write_bytes(buf, &data[at..at + n]).map_err(|_| "read buf")?;
                if let Some(Obj::File { pos, .. }) = self.handles.get_mut(&h) { *pos = (at + n) as u32; }
                self.w32(iosb, 0)?; self.w32(iosb + 4, n as u32)?;
                0
            }
            "NtQueryVolumeInformationFile" => {
                let (iosb, buf, class) = (a[1], a[2], a[4]);
                match class {
                    4 => { self.w32(buf, 7)?; self.w32(buf + 4, 0)?; self.w32(iosb, 0)?; self.w32(iosb + 4, 8)?; }
                    _ => return Err(format!("QueryVolumeInformationFile class {class}")),
                }
                0
            }
            "NtOpenKey" | "NtOpenKeyEx" | "NtCreateKey" => {
                let mut path = self.get_ustr(self.r32(a[2] + 8)?)?.to_ascii_lowercase();
                let root = self.r32(a[2] + 4)?;
                if root != 0 { if let Some(Obj::Key(r)) = self.handles.get(&root) { path = format!("{}\\{}", r, path.trim_start_matches('\\')); } }
                let path = path.trim_end_matches('\\').to_string();
                let existed = self.keys.contains(&path);
                if name != "NtCreateKey" && !existed {
                    self.log(format!("  (registry open {path} -> not found)"));
                    return Ok(Some(STATUS_OBJECT_NAME_NOT_FOUND));
                }
                if !existed {
                    let mut cur = String::new();
                    for part in path.split('\\').filter(|x| !x.is_empty()) { cur = format!("{cur}\\{part}"); self.keys.insert(cur.clone()); }
                }
                let h = self.new_handle(Obj::Key(path));
                self.w32(a[0], h)?;
                if name == "NtCreateKey" && a[6] != 0 { self.w32(a[6], if existed { 2 } else { 1 })?; }
                0
            }
            "NtSetValueKey" => {
                let kp = match self.handles.get(&a[0]) { Some(Obj::Key(k)) => k.clone(), _ => return Ok(Some(STATUS_INVALID_HANDLE)) };
                let vn = self.get_ustr(a[1])?.to_ascii_lowercase();
                let data = self.mem.read_bytes(a[4], a[5]).map_err(|_| "setvalue data")?.to_vec();
                self.values.insert((kp, vn), (a[3], data));
                0
            }
            "NtQueryValueKey" => {
                let kp = match self.handles.get(&a[0]) { Some(Obj::Key(k)) => k.clone(), _ => return Ok(Some(STATUS_INVALID_HANDLE)) };
                let vn = self.get_ustr(a[1])?.to_ascii_lowercase();
                let Some((typ, data)) = self.values.get(&(kp, vn)).cloned() else { return Ok(Some(STATUS_OBJECT_NAME_NOT_FOUND)) };
                if a[2] != 2 { return Err(format!("QueryValueKey class {}", a[2])); }
                let need = 12 + data.len() as u32;
                if a[4] != 0 { self.w32(a[5], need)?; }
                if a[4] < need { return Ok(Some(0x8000_0005)); }
                self.w32(a[3], 0)?;
                self.w32(a[3] + 4, typ)?;
                self.w32(a[3] + 8, data.len() as u32)?;
                self.mem.write_bytes(a[3] + 12, &data).map_err(|_| "qv buf")?;
                0
            }
            "NtEnumerateValueKey" | "NtEnumerateKey" => 0x8000_001A, // STATUS_NO_MORE_ENTRIES
            "NtFlushKey" => 0,
            "NtQuerySystemInformation" => {
                let (class, buf, len, retlen) = (a[0], a[1], a[2], a[3]);
                match class {
                    1000 => {
                        self.mem.write_bytes(buf, b"9.0\0").map_err(|_| "buf")?;
                        if retlen != 0 { self.w32(retlen, 4)?; }
                        let _ = len;
                    }
                    102 => {
                        for i in 0..len / 4 { self.w32(buf + i * 4, 0)?; }
                        if retlen != 0 { self.w32(retlen, 0x1B0)?; }
                    }
                    0 => {
                        let v = [0u32, 156250, 4096, 0x8000, 1, 0x7fff, 0x10000, 0x10000, 0x7ffe_ffff, 1, 1];
                        for (i, x) in v.iter().enumerate() { self.w32(buf + i as u32 * 4, *x)?; }
                        if retlen != 0 { self.w32(retlen, 0x2C)?; }
                    }
                    1 => {
                        self.w32(buf, 6 << 16)?; // arch=intel(0), level=6
                        self.w32(buf + 4, 1 << 16)?;
                        self.w32(buf + 8, 0)?;
                        if retlen != 0 { self.w32(retlen, 12)?; }
                    }
                    _ => return Err(format!("QuerySystemInformation class {class:#x}")),
                }
                0
            }
            "NtQueryInformationToken" => {
                let (class, buf, len, retlen) = (a[1], a[2], a[3], a[4]);
                match class {
                    1 => {
                        if len < 36 { self.w32(retlen, 36)?; return Ok(Some(0xC000_0023)); }
                        self.w32(buf, buf + 8)?;
                        self.w32(buf + 4, 0)?;
                        // S-1-5-21-0-0-0-1000
                        self.mem.write_bytes(buf + 8, &[1, 5, 0, 0, 0, 0, 0, 5]).map_err(|_| "sid")?;
                        for (i, v) in [21u32, 0, 0, 0, 1000].iter().enumerate() { self.w32(buf + 16 + i as u32 * 4, *v)?; }
                        self.w32(retlen, 36)?;
                    }
                    _ => return Err(format!("QueryInformationToken class {class}")),
                }
                0
            }
            "NtFsControlFile" => {
                self.log(format!("  (fsctl {:#x})", a[5]));
                0xC000_0275 // STATUS_NOT_A_REPARSE_POINT
            }
            "NtInitializeNlsFiles" => {
                let (p, sz) = self.nls_map("locale.nls")?;
                self.w32(a[0], p)?;
                self.w32(a[1], 0x409)?;
                self.w32(a[2], sz)?;
                self.w32(a[2] + 4, 0)?;
                0
            }
            "NtGetNlsSectionPtr" => {
                let (typ, id) = (a[0], a[1]);
                let name = match typ {
                    9 => "sortdefault.nls".to_string(),
                    10 => "l_intl.nls".to_string(),
                    11 => format!("c_{id:03}.nls"),
                    12 => match id { 1 => "normnfc.nls", 2 => "normnfd.nls", 5 => "normnfkc.nls", 6 => "normnfkd.nls", _ => "normidna.nls" }.to_string(),
                    _ => return Ok(Some(STATUS_INVALID_PARAMETER)),
                };
                if !self.files.contains_key(&format!("nls\\{name}")) { return Ok(Some(STATUS_OBJECT_NAME_NOT_FOUND)); }
                let (p, sz) = self.nls_map(&name)?;
                self.w32(a[3], p)?;
                self.w32(a[4], sz)?;
                0
            }
            "NtQueryDefaultLocale" => { self.w32(a[1], 0x409)?; 0 }
            "NtQueryDefaultUILanguage" | "NtQueryInstallUILanguage" => { self.w16(a[0], 0x409)?; 0 }
            "NtQuerySystemTime" => {
                self.ticks += 10_000;
                let t = 133_500_000_000_000_000u64 + self.ticks;
                self.w32(a[0], t as u32)?;
                self.w32(a[0] + 4, (t >> 32) as u32)?;
                0
            }
            "NtQueryPerformanceCounter" => {
                self.ticks += 10_000;
                self.w32(a[0], self.ticks as u32)?;
                self.w32(a[0] + 4, (self.ticks >> 32) as u32)?;
                if a[1] != 0 { self.w32(a[1], 10_000_000)?; self.w32(a[1] + 4, 0)?; }
                0
            }
            "NtWriteFile" => {
                let (h, iosb, buf, len) = (a[0], a[4], a[5], a[6]);
                match self.handles.get(&h) {
                    Some(Obj::Stdout) => {
                        let d = self.mem.read_bytes(buf, len).map_err(|_| "write buf")?.to_vec();
                        self.stdout.extend_from_slice(&d);
                        self.w32(iosb, 0)?;
                        self.w32(iosb + 4, len)?;
                        0
                    }
                    Some(_) => return Err("NtWriteFile on non-stdout handle".into()),
                    None => STATUS_INVALID_HANDLE,
                }
            }
            "NtWow64IsProcessorFeaturePresent" => 0, // no SSE/x87-extension features advertised
            "NtCallbackReturn" => { self.callback_ret = Some((a[0], a[1], a[2])); return Ok(None); }
            "NtTerminateProcess" => { self.exit = Some(a[1]); return Ok(None); }
            "NtClose" => if self.handles.remove(&a[0]).is_some() { 0 } else { STATUS_INVALID_HANDLE },
            _ if self.lenient && (name.starts_with("NtUser") || name.starts_with("NtGdi")) => {
                if !self.stubbed.iter().any(|n| n == name) { self.stubbed.push(name.to_string()); }
                0
            }
            _ => return Err("unimplemented syscall".into()),
        }))
    }
}

impl Default for WineProc { fn default() -> Self { Self::new() } }
