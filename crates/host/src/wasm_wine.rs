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
static mut TRAP_BUDGET: u64 = 50_000_000;
/// Work per `plonk_wine_run` call: guest instructions per slice and syscalls per call. The page tunes these to ~10 ms per call
/// so a program that never idles (a game loop) cannot starve rendering and input.
#[no_mangle]
pub extern "C" fn plonk_wine_set_budget(steps: u32, traps: u32) { wine().slice = steps.max(10_000) as u64; unsafe { TRAP_BUDGET = traps.max(200) as u64; } }

#[no_mangle]
pub extern "C" fn plonk_wine_run() -> i32 {
    match wine().run_until_idle(unsafe { TRAP_BUDGET }) {
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
    unsafe {
        let new = wine().compose_rgba(w, h);
        // dirty rectangle vs the previous frame, so the page uploads only what changed
        let (wu, hu) = (w as usize, h as usize);
        let (mut x0, mut y0, mut x1, mut y1) = (wu, hu, 0usize, 0usize);
        if OUT.len() != new.len() { (x0, y0, x1, y1) = (0, 0, wu, hu); }
        else {
            for y in 0..hu {
                let (a, b) = (&OUT[y * wu * 4..(y + 1) * wu * 4], &new[y * wu * 4..(y + 1) * wu * 4]);
                if a == b { continue; }
                let l = a.chunks_exact(4).zip(b.chunks_exact(4)).position(|(p, q)| p != q).unwrap_or(0);
                let r = a.chunks_exact(4).zip(b.chunks_exact(4)).rposition(|(p, q)| p != q).unwrap_or(0);
                x0 = x0.min(l); x1 = x1.max(r + 1); y0 = y0.min(y); y1 = y1.max(y + 1);
            }
        }
        DIRTY = if x1 > x0 && y1 > y0 { [x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32] } else { [0; 4] };
        OUT = new;
        OUT.as_ptr()
    }
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

// ---- 3D: command stream out, rendered frames in (docs/GL_PROTOCOL.md) ----
/// Pointer to the pending GL command words (valid until `plonk_gl_clear`).
#[no_mangle]
pub extern "C" fn plonk_gl_ptr() -> *const u32 { wine().wm.gl.cmds.as_ptr() }
#[no_mangle]
pub extern "C" fn plonk_gl_len() -> u32 { wine().wm.gl.cmds.len() as u32 }
#[no_mangle]
pub extern "C" fn plonk_gl_clear() { wine().wm.gl.cmds.clear(); }
/// Number of SwapBuffers calls so far.
#[no_mangle]
pub extern "C" fn plonk_gl_presents() -> u32 { wine().wm.gl.presents as u32 }
/// Reserve a buffer for an incoming RGBA frame of `w*h` pixels; returns its address.
#[no_mangle]
pub extern "C" fn plonk_gl_frame_buf(w: u32, h: u32) -> *mut u32 { let g = &mut wine().wm.gl; g.frame.clear(); g.frame.resize((w * h) as usize, 0); g.frame.as_mut_ptr() }
/// Blit the frame previously written into `plonk_gl_frame_buf` into the GL window.
#[no_mangle]
pub extern "C" fn plonk_gl_frame_commit(w: u32, h: u32) { wine().gl_frame(w, h); }

// ---- GPU compositor feed: the page draws the layers itself (web/compositor_gpu.mjs) ----
static mut LAYER_INFO: Vec<u32> = Vec::new();
/// Fill and return a table of the visible surfaces, bottom to top, 12 u32 per layer:
/// [id, x(i32), y(i32), w, h, radius, blur, shadow, ver_lo, pix_ptr (ARGB u32 pixels, w*h), ver_hi, 0]. Length via `plonk_layers_count`.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_layers() -> *const u32 {
    unsafe {
        LAYER_INFO.clear();
        for (id, s) in wine().wm.backend.layers() {
            LAYER_INFO.extend_from_slice(&[id, s.x as u32, s.y as u32, s.w, s.h, s.radius as u32, s.blur as u32, s.shadow as u32, s.ver as u32, s.pix.as_ptr() as u32, (s.ver >> 32) as u32, 0]);
        }
        LAYER_INFO.as_ptr()
    }
}
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_layers_count() -> u32 { unsafe { (LAYER_INFO.len() / 12) as u32 } }

/// Screen rectangle (x, y, w, h as 4 u32) of the client area the GL context draws to, and whether nothing is stacked over it
/// (flag in the 5th u32: 1 = the GL window is topmost and unobscured, so the page may show the GL canvas directly).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_gl_rect() -> *const u32 {
    static mut R: [u32; 5] = [0; 5];
    unsafe { R = wine().gl_screen_rect(); R.as_ptr() }
}

static mut DIRTY: [u32; 4] = [0; 4];
/// [x, y, w, h] changed since the previous `plonk_wine_compose` (w == 0: nothing changed).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_wine_dirty() -> *const u32 { unsafe { DIRTY.as_ptr() } }

/// Set the desktop wallpaper from `w*h` RGBA bytes already written at `ptr` (CPU compositor path; the GPU path gets the image directly).
#[no_mangle]
pub extern "C" fn plonk_wine_set_wallpaper(ptr: *const u8, w: u32, h: u32) {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, (w * h * 4) as usize) };
    let px = bytes.chunks_exact(4).map(|c| 0xFF00_0000 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32).collect();
    let b = &wine().wm.backend;
    b.set_wallpaper(w, h, px);
    let _ = b;
    wine().wm.backend.gen += 1;
}

/// Set the screen size (the page's available desktop area); call after `plonk_wine_reset` and before booting the program.
#[no_mangle]
pub extern "C" fn plonk_wine_set_screen(w: u32, h: u32) { wine().wm.set_screen(w.clamp(320, 8192) as i32, h.clamp(240, 8192) as i32); }

/// Guest instructions retired so far (all threads' CPU state is summed by the host as it switches; this is the running CPU's counter).
#[no_mangle]
pub extern "C" fn plonk_wine_retired() -> f64 { wine().total_retired() as f64 }

// ---- session disk: the page snapshots user files from one run and restores them into the next (no persistence across page loads) ----
static mut OUT_LIST: String = String::new();
/// Tab-separated "path\tsize" lines of the user's virtual disk (size -1 = directory); pointer valid until the next call.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_vfs_list() -> *const u8 { unsafe { OUT_LIST = wine().list_user_files(); OUT_LIST.as_ptr() } }
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_vfs_list_len() -> u32 { unsafe { OUT_LIST.len() as u32 } }
/// Pointer to a file's bytes (0 if missing); length via `plonk_vfs_get_len`. Valid until the file changes.
#[no_mangle]
pub extern "C" fn plonk_vfs_get(p: *const u8, n: u32) -> *const u8 {
    let path = unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(p, n as usize)).to_string() };
    wine().files.get(&path).map_or(std::ptr::null(), |v| v.as_ptr())
}
#[no_mangle]
pub extern "C" fn plonk_vfs_get_len(p: *const u8, n: u32) -> u32 {
    let path = unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(p, n as usize)).to_string() };
    wine().files.get(&path).map_or(0, |v| v.len() as u32)
}
/// Restore a directory entry of the session disk.
#[no_mangle]
pub extern "C" fn plonk_vfs_mkdir(p: *const u8, n: u32) {
    let path = unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(p, n as usize)).to_string() };
    wine().add_dir(&path);
}
