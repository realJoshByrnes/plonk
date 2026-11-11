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
    lx().files.insert(n, Vec::from_raw_parts(data, dlen as usize, dlen as usize));
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
    let env = ["HOME=/home/user", "WINEPREFIX=/home/user/.wine", "PATH=/usr/local/bin:/usr/bin:/bin", "WINELOADERNOEXEC=1", "USER=user",
        "WINEDLLOVERRIDES=mscoree,mshtml=d", "WINEDEBUG=-all"];
    match lx().spawn_path(&argv, &env) { Ok(pid) => pid as i32, Err(e) => { MSG = e; -1 } }
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

/// Changes whenever a window moved or new pixels arrived.
#[no_mangle]
pub extern "C" fn plonk_lx_frame() -> u32 { lx().frame as u32 }

#[no_mangle]
pub extern "C" fn plonk_lx_set_screen(w: u32, h: u32) {
    let p = lx();
    if p.screen != (w, h) { p.screen = (w, h); p.push_event(8, 0, w as i32, h as i32, 0, 0); }
}

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
