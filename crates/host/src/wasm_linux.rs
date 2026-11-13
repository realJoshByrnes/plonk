//! wasm/FFI entry points for the real-Wine path (the Linux personality in `linux.rs`): JS feeds the packed Wine install and a
//! saved prefix as tar archives, starts programs with `wine`, pumps the kernel each frame, sends input, and reads back the
//! composed screen as RGBA.
use crate::linux::LinuxProc;

static mut LX: Option<LinuxProc> = None;
static mut OUT: Vec<u8> = Vec::new();
static mut MSG: String = String::new();

#[allow(static_mut_refs)]
fn lx() -> &'static mut LinuxProc {
    unsafe {
        LX.get_or_insert_with(|| {
            let mut p = LinuxProc::new();
            p.wait_all = true; // the kernel keeps going after the first program ends
            p.trace_cap = 0;
            p.clock_step_ns = 0; // guest time is the page's real clock
            p.mkdir("/home/user/.wine");
            p
        })
    }
}

/// Unpack a tar archive into the guest file tree. Takes ownership of the buffer (from `plonk_alloc`).
///
/// # Safety
/// `data` must be an exact-size allocation returned by `plonk_alloc` and not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn plonk_lx_add_tar(data: *mut u8, len: u32) {
    let v = Vec::from_raw_parts(data, len as usize, len as usize);
    lx().add_tar(&v);
}

/// Add one file to the guest file tree. Takes ownership of both buffers (from `plonk_alloc`).
///
/// # Safety
/// `name`/`data` must be exact-size allocations returned by `plonk_alloc` and not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn plonk_lx_add_file(name: *mut u8, nlen: u32, data: *mut u8, dlen: u32) {
    let n = String::from_utf8_lossy(&Vec::from_raw_parts(name, nlen as usize, nlen as usize)).into_owned();
    lx().provide(n, Vec::from_raw_parts(data, dlen as usize, dlen as usize));
}

/// Declare an archive's contents without the file bodies (see `LinuxProc::add_index`). Takes ownership of the buffer.
///
/// # Safety
/// `data` must be an exact-size allocation returned by `plonk_alloc` and not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn plonk_lx_add_index(data: *mut u8, len: u32) {
    let v = Vec::from_raw_parts(data, len as usize, len as usize);
    lx().add_index(&String::from_utf8_lossy(&v));
}

/// Supply the regular files of a tar archive (see `LinuxProc::provide_tar`). Takes ownership of the buffer.
///
/// # Safety
/// `data` must be an exact-size allocation returned by `plonk_alloc` and not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn plonk_lx_provide_tar(data: *mut u8, len: u32) {
    let v = Vec::from_raw_parts(data, len as usize, len as usize);
    lx().provide_tar(&v);
}

/// The files the guest is waiting for, one path per line, as the message (`plonk_lx_msg`). Returns how many.
#[no_mangle]
pub unsafe extern "C" fn plonk_lx_wanted() -> u32 {
    let w = lx().take_wanted();
    MSG = w.join("\n");
    w.len() as u32
}

/// Start a program: `cmd` is the argument vector separated by newlines, argv[0] being a guest path (normally
/// /usr/local/bin/wine). Returns the pid, or -1 (see the message).
///
/// # Safety
/// `cmd` must be valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn plonk_lx_spawn(cmd: *const u8, len: u32) -> i32 {
    let s = String::from_utf8_lossy(core::slice::from_raw_parts(cmd, len as usize)).into_owned();
    let argv: Vec<&str> = s.split('\n').filter(|a| !a.is_empty()).collect();
    let env = crate::linux::WINE_ENV;
    match lx().spawn_path(&argv, &env) { Ok(pid) => pid as i32, Err(e) => { MSG = e; -1 } }
}

/// Replace the kernel state with a snapshot taken by `linuxrun` (SNAP=...): a booted, idle Wine session. The tar archives
/// must be loaded first. Returns 0, or -1 (see the message). Takes ownership of the buffer (from `plonk_alloc`).
///
/// # Safety
/// `data` must be an exact-size allocation returned by `plonk_alloc` and not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn plonk_lx_restore(data: *mut u8, len: u32) -> i32 {
    let v = Vec::from_raw_parts(data, len as usize, len as usize);
    match lx().restore(&v) { Ok(()) => 0, Err(e) => { MSG = e; -1 } }
}

/// Let `dt_us` microseconds of guest time pass, then run up to `steps` instructions.
/// Returns 0 = still busy (call again soon), 1 = everything is waiting (for time or input), -1 = error (see the message).
#[no_mangle]
pub extern "C" fn plonk_lx_run(steps: u32, dt_us: u32) -> i32 {
    let p = lx();
    p.advance_ns(dt_us as u64 * 1000);
    match p.run(steps as u64) {
        Ok(_) => p.blocked as i32,
        Err(e) => { unsafe { MSG = e; } -1 }
    }
}

/// 1 while the process with this pid (from `plonk_lx_spawn`) is running.
#[no_mangle]
pub extern "C" fn plonk_lx_alive(pid: u32) -> u32 { lx().alive(pid) as u32 }

/// Changes whenever a window moved or new pixels arrived.
#[no_mangle]
pub extern "C" fn plonk_lx_frame() -> u32 { lx().frame as u32 }

#[no_mangle]
pub extern "C" fn plonk_lx_set_screen(w: u32, h: u32) {
    let p = lx();
    if p.screen != (w, h) { p.screen = (w, h); p.push_event(8, 0, w as i32, h as i32, 0, 0); }
}

static mut SCENE: Vec<u32> = Vec::new();
/// The windows for a taskbar, one per line "hwnd<TAB>state<TAB>title" ordered by creation (state: 2 minimized, 4 the front one), as the
/// message (`plonk_lx_msg`).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_lx_tasks() {
    let p = lx();
    let task = |w: &crate::linux::HostWin| w.visible && w.owner == 0 && w.cap[0] & 1 != 0 && !w.title.is_empty() && (w.ex_style & 0x80 == 0 || w.ex_style & 0x4_0000 != 0);
    let front = p.zorder.iter().find(|h| p.windows.get(h).map_or(false, |w| task(w) && w.style & 0x2000_0000 == 0)).copied();
    let mut out = String::new();
    for w in p.windows.values().filter(|w| task(w)) {
        let st = if w.style & 0x2000_0000 != 0 { 2 } else { 0 } | if Some(w.hwnd) == front { 4 } else { 0 };
        out.push_str(&format!("{}\t{}\t{}\n", w.hwnd, st, w.title.replace(['\t', '\n'], " ")));
    }
    unsafe { MSG = out; }
}
/// Bring a window to the front (action 1) or minimize it (2).
#[no_mangle]
pub extern "C" fn plonk_lx_activate(hwnd: u32, action: u32) { lx().push_event(9, hwnd, 0, 0, action, 0); }
/// The title of a window, as the message (`plonk_lx_msg`).
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_lx_title(hwnd: u32) { unsafe { MSG = lx().windows.get(&hwnd).map(|w| w.title.clone()).unwrap_or_default(); } }
/// The windows to draw, bottom first, for a GPU compositor: [count, then per window: hwnd, x, y, width, height, address of the
/// BGRX pixels, dirty left, top, right, bottom (surface coordinates; right <= left: unchanged)]. Valid until the next call.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_lx_scene() -> *const u32 {
    let p = lx();
    unsafe {
        SCENE.clear();
        SCENE.push(0);
        let mut n = 0;
        for h in p.zorder.iter().rev() {
            let Some(w) = p.windows.get(h) else { continue };
            if !w.visible || w.pixels.is_empty() || w.style & 0x2000_0000 != 0 { continue; } // minimized windows are not drawn
            SCENE.extend_from_slice(&[w.hwnd, w.rect[0] as u32, w.rect[1] as u32, w.sw, w.sh, w.pixels.as_ptr() as u32,
                w.dirty[0] as u32, w.dirty[1] as u32, w.dirty[2] as u32, w.dirty[3] as u32,
                w.cap[0] as u32, w.cap[1] as u32, w.cap[2] as u32, w.cap[3] as u32, w.cap[4] as u32, w.cap[5] as u32, w.title_gen]);
            n += 1;
        }
        SCENE[0] = n;
        SCENE.as_ptr()
    }
}
/// The page has taken the pending changes of every window.
#[no_mangle]
pub extern "C" fn plonk_lx_scene_done() { for w in lx().windows.values_mut() { w.dirty = [0; 4]; } }

/// Compose the screen; returns a pointer to width*height*4 RGBA bytes valid until the next call.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_lx_compose() -> *const u8 {
    let p = lx();
    let (sw, sh) = (p.screen.0 as i32, p.screen.1 as i32);
    unsafe {
        OUT.clear();
        OUT.resize((sw * sh * 4) as usize, 0);
        for px in OUT.chunks_exact_mut(4) { px.copy_from_slice(&[58, 110, 165, 255]); }
        for h in p.zorder.iter().rev() {
            let Some(w) = p.windows.get(h) else { continue };
            if !w.visible || w.pixels.is_empty() { continue; }
            let (x0, x1) = (w.rect[0].max(0), (w.rect[0] + w.sw as i32).min(sw));
            if x1 <= x0 { continue; }
            for y in 0..w.sh as i32 {
                let dy = w.rect[1] + y;
                if dy < 0 || dy >= sh { continue; }
                let src = &w.pixels[((y as u32 * w.sw) as usize + (x0 - w.rect[0]) as usize) * 4..];
                let dst = &mut OUT[((dy * sw + x0) * 4) as usize..((dy * sw + x1) * 4) as usize];
                for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) { d[0] = s[2]; d[1] = s[1]; d[2] = s[0]; }
            }
        }
        OUT.as_ptr()
    }
}

/// kind: 0 = button down, 1 = move, 2 = button up; button: 0 left, 1 right, 2 middle. Screen coordinates.
#[no_mangle]
pub extern "C" fn plonk_lx_mouse(kind: u32, x: i32, y: i32, button: u32) {
    let p = lx();
    let h = p.window_at(x, y);
    p.push_event(match kind { 0 => 2, 2 => 3, _ => 1 }, h, x, y, button, 0);
}

#[no_mangle]
pub extern "C" fn plonk_lx_wheel(x: i32, y: i32, delta: i32) {
    let p = lx();
    let h = p.window_at(x, y);
    p.push_event(4, h, x, y, delta as u32, 0);
}

/// down: 1 = key down, 0 = key up; vk = Windows virtual-key code; scan = hardware scan code (bit 8 = extended key).
#[no_mangle]
pub extern "C" fn plonk_lx_key(down: u32, vk: u32, scan: u32) { lx().push_event(if down != 0 { 5 } else { 6 }, 0, 0, 0, vk, scan); }

/// Current cursor kind as set by the driver (0 arrow, 1 ibeam, ... -1 hidden).
#[no_mangle]
pub extern "C" fn plonk_lx_cursor() -> i32 { lx().cursor }

/// Translation of hot code to wasm: turn it on, collect the next module of translated blocks (length, 0 = none; then
/// `plonk_lx_jit_ptr` / `plonk_lx_jit_id` / `plonk_lx_jit_count`), and report it installed in the function table.
#[no_mangle]
pub extern "C" fn plonk_lx_jit_enable(on: u32) { plonk_cpu::jit::enable(on != 0); }
static mut JIT_MOD: (u32, u32, Vec<u8>) = (0, 0, Vec::new());
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_lx_jit_take() -> u32 {
    unsafe { match plonk_cpu::jit::take_module() { Some(m) => { JIT_MOD = m; JIT_MOD.2.len() as u32 } None => 0 } }
}
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_lx_jit_ptr() -> *const u8 { unsafe { JIT_MOD.2.as_ptr() } }
#[no_mangle]
pub extern "C" fn plonk_lx_jit_id() -> u32 { unsafe { JIT_MOD.0 } }
#[no_mangle]
pub extern "C" fn plonk_lx_jit_count() -> u32 { unsafe { JIT_MOD.1 } }
#[no_mangle]
pub extern "C" fn plonk_lx_jit_installed(id: u32, first: u32) { plonk_cpu::jit::installed(id, first); }
/// Profiling of the slow path: on/off, and the report (as the message).
#[no_mangle]
pub extern "C" fn plonk_lx_jit_fpu_off(mask: u32) { plonk_cpu::jit::fpu_off(mask); }
#[no_mangle]
pub extern "C" fn plonk_lx_prof(on: u32) { plonk_cpu::jit::profile(on != 0); }
#[no_mangle]
pub extern "C" fn plonk_lx_prof_report() { unsafe { MSG = plonk_cpu::jit::profile_report(); } }
/// Functions installed so far.
#[no_mangle]
pub extern "C" fn plonk_lx_jit_stats() -> u32 { plonk_cpu::jit::stats().0 as u32 }

/// Millions of guest instructions executed so far.
#[no_mangle]
pub extern "C" fn plonk_lx_mips_total() -> u32 { (lx().retired() / 1_000_000) as u32 }

/// Move the guest's pending console output into the message buffer (read it with plonk_lx_msg / plonk_lx_msg_len).
#[no_mangle]
pub extern "C" fn plonk_lx_take_stdout() {
    let p = lx();
    unsafe { MSG = String::from_utf8_lossy(&p.stdout).into_owned(); }
    p.stdout.clear();
}

/// One line per live guest process into the message buffer (debugging).
#[no_mangle]
pub extern "C" fn plonk_lx_ps() { unsafe { MSG = lx().ps(); } }

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_lx_msg() -> *const u8 { unsafe { MSG.as_ptr() } }
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_lx_msg_len() -> u32 { unsafe { MSG.len() as u32 } }

// ---- OpenGL: the command stream (docs/GL_PROTOCOL.md) the page replays on WebGPU / WebGL2 ----

/// Pointer to the pending GL command words (valid until `plonk_lx_gl_clear`).
#[no_mangle]
pub extern "C" fn plonk_lx_gl_ptr() -> *const u32 { lx().gl.cmds.as_ptr() }
#[no_mangle]
pub extern "C" fn plonk_lx_gl_len() -> u32 { lx().gl.cmds.len() as u32 }
#[no_mangle]
pub extern "C" fn plonk_lx_gl_clear() { lx().gl.cmds.clear(); }
/// Frames presented so far.
#[no_mangle]
pub extern "C" fn plonk_lx_gl_presents() -> u32 { lx().gl.presents as u32 }
/// Pointer to 5 words: x, y, width, height of the GL window's client area on screen, and 1 when it is visible and on top.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_lx_gl_rect() -> *const u32 {
    static mut R: [u32; 5] = [0; 5];
    unsafe { R = lx().gl_screen_rect(); R.as_ptr() }
}

/// Change counter of the .lnk files in the prefix (poll it; re-read the listing when it changes).
#[no_mangle]
pub extern "C" fn plonk_lx_lnk_gen() -> u32 { lx().lnk_generation() }

/// The shortcuts of the Desktop / Start Menu folders as lines (see `LinuxProc::list_shortcuts`), as the message.
#[no_mangle]
pub extern "C" fn plonk_lx_shortcuts() { unsafe { MSG = lx().list_shortcuts(); } }

static mut FILEBUF: Vec<u8> = Vec::new();
/// Read a guest file (case-insensitive path) into a buffer; returns its length or -1. The bytes are at `plonk_lx_filebuf`.
///
/// # Safety
/// `p` must be valid for `len` bytes.
#[no_mangle]
#[allow(static_mut_refs)]
pub unsafe extern "C" fn plonk_lx_read_file(p: *const u8, len: u32) -> i32 {
    let path = String::from_utf8_lossy(core::slice::from_raw_parts(p, len as usize)).into_owned();
    match lx().read_ci(&path) { Some(b) => { FILEBUF = b; FILEBUF.len() as i32 } None => { FILEBUF = Vec::new(); -1 } }
}
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_lx_filebuf() -> *const u8 { unsafe { FILEBUF.as_ptr() } }

/// The topmost visible window at a screen position (0 = none).
#[no_mangle]
pub extern "C" fn plonk_lx_window_at(x: i32, y: i32) -> u32 { lx().window_at(x, y) }

/// Audio: take the PCM (s16le, interleaved) the guest's sound card played since the last call; returns its byte length.
#[no_mangle]
pub extern "C" fn plonk_lx_audio_take() -> u32 {
    let a = &mut lx().audio;
    unsafe { AUDIO = std::mem::take(&mut a.out); AUDIO.len() as u32 }
}
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn plonk_lx_audio_ptr() -> *const u8 { unsafe { AUDIO.as_ptr() } }
/// Sample rate of the PCM that `plonk_lx_audio_take` last returned.
#[no_mangle]
pub extern "C" fn plonk_lx_audio_rate() -> u32 { lx().audio.rate }
#[no_mangle]
pub extern "C" fn plonk_lx_audio_channels() -> u32 { lx().audio.ch }
static mut AUDIO: Vec<u8> = Vec::new();
