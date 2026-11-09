//! Host: process (memory + cpu + thunk dispatch) and wasm exports.
pub mod ntuser_codes;
pub mod reqnames;
pub mod sysnames;
pub mod wasm_wine;
pub mod wine;
use plonk_cpu::{Cpu, Exit, EAX, ESP};
use plonk_mem::Mem;

pub const GUEST_SIZE: u32 = 0x80_0000; // 8 MB
pub const THUNK_BASE: u32 = 0xF000_0000; // magic eip range, not backed by memory
const THUNK_SLOT: u32 = 4;
const STACK_TOP: u32 = 0x7F_0000;

pub struct Process {
    pub mem: Mem,
    pub cpu: Cpu,
    pub stdout: Vec<u8>,
    thunks: Vec<String>,
}

impl Process {
    pub fn new() -> Self {
        Process { mem: Mem::new_owned(GUEST_SIZE), cpu: Cpu::new(), stdout: Vec::new(), thunks: vec!["<return>".into()] }
    }

    /// Load a PE and run it to ExitProcess / return from entry. Returns the exit code.
    pub fn run_pe(&mut self, file: &[u8]) -> Result<u32, String> {
        let mut thunks = std::mem::take(&mut self.thunks);
        let img = {
            let mut res = |dll: &str, name: &str| {
                let key = format!("{}!{}", dll.to_ascii_lowercase(), name);
                let i = thunks.iter().position(|n| *n == key).unwrap_or_else(|| { thunks.push(key); thunks.len() - 1 });
                THUNK_BASE + i as u32 * THUNK_SLOT
            };
            plonk_pe::load(file, &mut self.mem, None, &mut res)
        };
        self.thunks = thunks;
        let img = img?;
        self.cpu.thunk = (THUNK_BASE, THUNK_BASE + self.thunks.len() as u32 * THUNK_SLOT);
        self.cpu.r[ESP] = STACK_TOP - 4;
        self.mem.w32(STACK_TOP - 4, THUNK_BASE).map_err(|_| "stack")?; // return address -> <return>
        self.cpu.eip = img.entry;
        loop {
            match self.cpu.run(&mut self.mem, u64::MAX) {
                Exit::Thunk(addr) => {
                    let name = self.thunks[((addr - THUNK_BASE) / THUNK_SLOT) as usize].clone();
                    if let Some(code) = self.call_thunk(&name)? { return Ok(code); }
                }
                e => return Err(format!("cpu stopped: {:?}", e)),
            }
        }
    }

    fn arg(&self, i: u32) -> u32 { self.mem.r32(self.cpu.r[ESP].wrapping_add(4 + 4 * i)).unwrap_or(0) }

    /// stdcall dispatch. Returns Some(exit_code) when the process ends.
    fn call_thunk(&mut self, name: &str) -> Result<Option<u32>, String> {
        if name == "<return>" { return Ok(Some(self.cpu.r[EAX])); }
        let (nargs, ret) = match name {
            "kernel32.dll!GetStdHandle" => (1, match self.arg(0) { 0xFFFF_FFF6 => 0x10, 0xFFFF_FFF5 => 0x11, 0xFFFF_FFF4 => 0x12, _ => 0xFFFF_FFFF }),
            "kernel32.dll!WriteFile" => {
                let (h, buf, n, pw) = (self.arg(0), self.arg(1), self.arg(2), self.arg(3));
                if h != 0x11 && h != 0x12 { (5, 0) } else {
                    let data = self.mem.read_bytes(buf, n).map_err(|_| "WriteFile: bad buffer")?.to_vec();
                    self.stdout.extend_from_slice(&data);
                    if pw != 0 { self.mem.w32(pw, n).map_err(|_| "WriteFile: bad pw")?; }
                    (5, 1)
                }
            }
            "kernel32.dll!ExitProcess" => return Ok(Some(self.arg(0))),
            _ => return Err(format!("unimplemented import {name}")),
        };
        let ra = self.mem.r32(self.cpu.r[ESP]).map_err(|_| "bad stack")?;
        self.cpu.r[ESP] = self.cpu.r[ESP].wrapping_add(4 + 4 * nargs);
        self.cpu.r[EAX] = ret;
        self.cpu.eip = ra;
        Ok(None)
    }
}

impl Default for Process { fn default() -> Self { Self::new() } }

// ---- wasm exports (also callable natively) ----
static mut LAST: Option<Process> = None;
static mut ERR: String = String::new();

#[no_mangle]
pub extern "C" fn plonk_alloc(n: u32) -> *mut u8 {
    let mut v = vec![0u8; n as usize];
    let p = v.as_mut_ptr();
    core::mem::forget(v);
    p
}

/// Runs a PE image at ptr/len. Returns exit code, or 0xFFFF_FFFF on error (see plonk_error_*).
///
/// # Safety
/// `ptr..ptr+len` must be readable.
#[no_mangle]
pub unsafe extern "C" fn plonk_run_pe(ptr: *const u8, len: u32) -> u32 {
    let file = core::slice::from_raw_parts(ptr, len as usize);
    let mut p = Process::new();
    let r = p.run_pe(file);
    LAST = Some(p);
    match r { Ok(c) => c, Err(e) => { ERR = e; 0xFFFF_FFFF } }
}
/// # Safety
/// Single-threaded wasm only.
#[no_mangle]
pub unsafe extern "C" fn plonk_stdout_ptr() -> *const u8 { (*core::ptr::addr_of!(LAST)).as_ref().map_or(core::ptr::null(), |p| p.stdout.as_ptr()) }
/// # Safety
/// Single-threaded wasm only.
#[no_mangle]
pub unsafe extern "C" fn plonk_stdout_len() -> u32 { (*core::ptr::addr_of!(LAST)).as_ref().map_or(0, |p| p.stdout.len() as u32) }
/// # Safety
/// Single-threaded wasm only.
#[no_mangle]
pub unsafe extern "C" fn plonk_error_ptr() -> *const u8 { (*core::ptr::addr_of!(ERR)).as_str().as_ptr() }
/// # Safety
/// Single-threaded wasm only.
#[no_mangle]
pub unsafe extern "C" fn plonk_error_len() -> u32 { (*core::ptr::addr_of!(ERR)).as_str().len() as u32 }

/// Runs `mov eax,2; add eax,3; hlt` inside wasm. Returns EAX (5 on success) or 0xDEAD.
#[no_mangle]
pub extern "C" fn plonk_selftest() -> u32 {
    let mut m = Mem::new_owned(0x10000);
    m.write_bytes(0x1000, &[0xB8, 2, 0, 0, 0, 0x83, 0xC0, 3, 0xF4]).unwrap();
    let mut c = Cpu::new();
    c.eip = 0x1000;
    if c.run(&mut m, 100) == Exit::Halt { c.r[EAX] } else { 0xDEAD }
}
