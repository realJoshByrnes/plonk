//! wasm/FFI entry points for the Wine path: JS feeds files, boots an exe, pumps it, and reads
//! back the composed desktop as RGBA.
use crate::wine::{RunState, WineProc};

static mut WINE: Option<WineProc> = None;
static mut OUT: Vec<u8> = Vec::new();
static mut MSG: String = String::new();

#[allow(static_mut_refs)]
fn wine() -> &'static mut WineProc {
    unsafe { WINE.get_or_insert_with(|| { let mut p = WineProc::new(); p.lenient = true; p }) }
}
unsafe fn bytes<'a>(p: *const u8, n: u32) -> &'a [u8] { core::slice::from_raw_parts(p, n as usize) }

/// # Safety
/// Pointers must be valid for the given lengths.
#[no_mangle]
pub unsafe extern "C" fn plonk_wine_add_file(name: *const u8, nlen: u32, data: *const u8, dlen: u32) {
    let n = String::from_utf8_lossy(bytes(name, nlen)).into_owned();
    wine().files.insert(n, bytes(data, dlen).to_vec());
}

/// # Safety
/// Pointer must be valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn plonk_wine_boot(path: *const u8, len: u32) -> i32 {
    let p = String::from_utf8_lossy(bytes(path, len)).into_owned();
    match wine().boot(&p) { Ok(()) => 0, Err(e) => { MSG = e; -1 } }
}

/// Runs until the guest exits or waits for input. Returns 0 = idle, 1 = exited (see exit code), -1 = error.
#[no_mangle]
pub extern "C" fn plonk_wine_run() -> i32 {
    match wine().run_until_idle(50_000_000) {
        Ok(RunState::Idle) => 0,
        Ok(RunState::Exited(c)) => { unsafe { MSG = format!("{c}"); } 1 }
        Err(e) => { unsafe { MSG = e; } -1 }
    }
}

/// kind: 0 = mouse down, 1 = move, 2 = up (desktop coordinates).
#[no_mangle]
pub extern "C" fn plonk_wine_mouse(kind: u32, x: i32, y: i32) { wine().mouse(kind, x, y); }

#[no_mangle]
pub extern "C" fn plonk_wine_click(x: i32, y: i32) { wine().click(x, y); }

/// Compose the desktop (w x h); returns a pointer to w*h*4 RGBA bytes valid until the next call.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_wine_compose(w: u32, h: u32) -> *const u8 {
    unsafe { OUT = wine().compose_rgba(w, h); OUT.as_ptr() }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_wine_msg_ptr() -> *const u8 { unsafe { MSG.as_ptr() } }
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_wine_msg_len() -> u32 { unsafe { MSG.len() as u32 } }

#[no_mangle]
pub extern "C" fn plonk_wine_stdout_ptr() -> *const u8 { wine().stdout.as_ptr() }
#[no_mangle]
pub extern "C" fn plonk_wine_stdout_len() -> u32 { wine().stdout.len() as u32 }

/// Space-separated NtUser*/NtGdi* syscalls that had no handler and were answered with 0.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_wine_stubbed_ptr() -> *const u8 { unsafe { OUT_STUB = wine().stubbed.join(" "); OUT_STUB.as_ptr() } }
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_wine_stubbed_len() -> u32 { unsafe { OUT_STUB.len() as u32 } }
static mut OUT_STUB: String = String::new();
