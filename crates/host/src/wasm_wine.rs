//! wasm/FFI entry points for the Wine path: JS feeds files, boots an exe, pumps it, and reads
//! back the composed desktop as RGBA.
use crate::wine::{RunState, WineProc};

static mut WINE: Option<WineProc> = None;
static mut OUT: Vec<u8> = Vec::new();
static mut MSG: String = String::new();

#[allow(static_mut_refs)]
fn wine() -> &'static mut WineProc {
    unsafe { WINE.get_or_insert_with(|| { drop(Vec::<u8>::with_capacity(160 << 20)); let mut p = WineProc::new(); p.lenient = true; p }) }
}
unsafe fn bytes<'a>(p: *const u8, n: u32) -> &'a [u8] { core::slice::from_raw_parts(p, n as usize) }

/// Takes ownership of both buffers, which must come from `plonk_alloc` (no copy, no leak).
///
/// # Safety
/// `name`/`data` must be exact-size allocations returned by `plonk_alloc` and not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn plonk_wine_add_file(name: *mut u8, nlen: u32, data: *mut u8, dlen: u32) {
    let n = String::from_utf8_lossy(&Vec::from_raw_parts(name, nlen as usize, nlen as usize)).into_owned();
    wine().files.insert(n, Vec::from_raw_parts(data, dlen as usize, dlen as usize));
}

/// # Safety
/// Pointer must be valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn plonk_wine_boot(path: *const u8, len: u32) -> i32 {
    let p = String::from_utf8_lossy(bytes(path, len)).into_owned();
    match wine().boot(&p) { Ok(()) => 0, Err(e) => { MSG = e; -1 } }
}

/// Runs a slice. Returns 0 = idle, 1 = exited (see exit code), 2 = busy (call again),
/// 3 = needs a file (VFS key in the message; add it, then call again), -1 = error.
#[no_mangle]
pub extern "C" fn plonk_wine_run() -> i32 {
    match wine().run_until_idle(50_000_000) {
        Ok(RunState::Idle) => 0,
        Ok(RunState::Busy) => 2,
        Ok(RunState::Need(k)) => { unsafe { MSG = k; } 3 }
        Ok(RunState::Exited(c)) => { unsafe { MSG = format!("{c}"); } 1 }
        Err(e) => { unsafe { MSG = e; } -1 }
    }
}

/// kind: 0 = mouse down, 1 = move, 2 = up (desktop coordinates).
#[no_mangle]
pub extern "C" fn plonk_wine_mouse(kind: u32, x: i32, y: i32) { wine().mouse(kind, x, y); }

/// kind: 0 = key down, 1 = key up; ch = translated character or 0.
#[no_mangle]
pub extern "C" fn plonk_wine_key(kind: u32, vk: u32, ch: u32) { wine().key(kind, vk, ch); }

/// Advance the guest clock by dt_ms and run it (call from a ~16-50ms JS interval).
#[no_mangle]
pub extern "C" fn plonk_wine_tick(dt_ms: u32) { wine().tick(dt_ms); }

/// Drop the current process (frees the 2GB guest) so the next boot starts clean.
#[no_mangle]
pub extern "C" fn plonk_wine_reset() { unsafe { WINE = None; } }

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

/// Last 80 trace lines (debugging aid for the browser console / e2e).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_wine_trace_ptr() -> *const u8 {
    unsafe {
        let t = &wine().trace;
        OUT_TRACE = t.iter().skip(t.len().saturating_sub(80)).cloned().collect::<Vec<_>>().join("\n");
        OUT_TRACE.as_ptr()
    }
}
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_wine_trace_len() -> u32 { unsafe { OUT_TRACE.len() as u32 } }
static mut OUT_TRACE: String = String::new();

/// Display generation (low 32 bits); changes whenever anything on screen changed.
#[no_mangle]
pub extern "C" fn plonk_wine_gen() -> u32 { wine().wm.backend.gen as u32 }

// dlmalloc degrades badly next to the 2GB guest block (fontdue's many small reallocs took seconds);
// lol_alloc's free-list allocator keeps small allocations cheap.
#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOC: lol_alloc::AssumeSingleThreaded<lol_alloc::FreeListAllocator> =
    unsafe { lol_alloc::AssumeSingleThreaded::new(lol_alloc::FreeListAllocator::new()) };

/// Declare a file (basename) the page can supply lazily.
///
/// # Safety
/// Pointer must be valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn plonk_wine_add_remote(name: *const u8, len: u32) {
    wine().remote.insert(String::from_utf8_lossy(bytes(name, len)).to_ascii_lowercase());
}

/// Debug: top-level windows and pending invalid regions as text (see plonk_wine_msg_*).
#[no_mangle]
pub extern "C" fn plonk_wine_debug() {
    let w = wine();
    unsafe { MSG = format!("windows: {} | invalid: {} | stubbed: {}", w.debug_windows(), w.debug_invalid(), w.stubbed.join(" ")); }
}
