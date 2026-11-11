//! Native replacement for Wine's win32u.so: window manager, message queue and GDI, backed by the
//! `display` crate. Called from the syscall dispatcher for NtUser*/NtGdi*.
use super::{Res, WineProc, STATUS_INVALID_HANDLE};
use crate::ntuser_codes as codes;
use plonk_cpu::{ESP, EAX};
use plonk_display::{Backend, SoftBackend, SurfaceId};
use std::collections::{BTreeMap, VecDeque};

mod gdi;
mod gdi_batch;
pub mod gl;
mod menu;
mod misc_batch;
mod more;
mod user_batch;
use gdi::Bitmap;
use menu::{Menu, OpenMenu, MENU_H};

#[derive(Clone)]
struct FontSpec { px: f32, bold: bool, face: String, raw: Vec<u8> }

const DESKTOP: u32 = 0x10010;
const WM_CREATE: u32 = 0x01;
const WM_DESTROY: u32 = 0x02;
const WM_PAINT: u32 = 0x0F;
const WM_CLOSE: u32 = 0x10;
const WM_QUIT: u32 = 0x12;
const WM_ERASEBKGND: u32 = 0x14;
const WM_SHOWWINDOW: u32 = 0x18;
const WM_NCCREATE: u32 = 0x81;
const WM_NCDESTROY: u32 = 0x82;
const WM_NCHITTEST: u32 = 0x84;
const WM_SYSCOMMAND: u32 = 0x112;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_CHILD: u32 = 0x4000_0000;
const CW_USEDEFAULT: u32 = 0x8000_0000;
const BORDER: i32 = 4;
const CAPTION: i32 = 28;
const CAPBTN_W: i32 = 36; // caption button width (min / max / close)

type Rect = [i32; 4]; // l, t, r, b

pub struct Class {
    atom: u32,
    name: String,
    wndproc: u32,
    bg: u32,
    raw: [u32; 12], // WNDCLASSEXW as registered
    ansi: bool,
}

pub struct Window {
    class: usize,
    style: u32,
    rect: Rect, // screen coordinates
    parent: u32,
    text: String,
    visible: bool,
    invalid: Option<Rect>, // client coordinates
    needs_erase: bool,
    surface: Option<SurfaceId>,
    ex_style: u32,
    menu: u32,
    id: u32,
    inst: u32,
    userdata: u32,
    proc_override: u32,
    extra: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Msg { hwnd: u32, msg: u32, wp: u32, lp: u32 }

#[derive(Clone, Copy)]
enum Gdi { Brush(u32), NullBrush, Pen(u32, bool), Region(Rect), Font, Bitmap, Other }

struct Dc { hwnd: u32, clip: Option<Rect>, brush: u32, pen: u32, pos: (i32, i32), font: u32, bitmap: u32 }

pub struct Wm {
    /// Recycled GDI handle-table indexes and DC_ATTR blocks (programs create and release thousands of DCs while painting).
    free_gdi: Vec<u32>,
    /// Arena for CREATESTRUCT + window-name copies during CreateWindowEx (base, bytes in use); nesting-safe (restored after each create).
    cs_arena: (u32, u32),
    dcattr_free: Vec<u32>,
    /// Screen size in pixels (set by the page from the available area).
    pub screen: (i32, i32),
    pub gl: gl::Gl,
    classes: Vec<Class>,
    pub windows: BTreeMap<u32, Window>,
    queue: VecDeque<Msg>,
    quit: Option<u32>,
    next_hwnd: u32,
    gdi: BTreeMap<u32, Gdi>,
    dcs: BTreeMap<u32, Dc>,
    next_gdi: u32,
    pub backend: SoftBackend,
    time: u32,
    /// Test/debug: when set, EndPaint stores a copy of the painted window surface here.
    zorder: Vec<u32>, // top-level windows, bottom to top
    active: u32,
    drag: Option<(u32, i32, i32)>, // hwnd, grab offset
    capture: u32,
    focus: u32,
    default_objs: [u32; 3],
    extra: more::Extra,
    menus: BTreeMap<u32, Menu>,
    next_menu: u32,
    open: Option<OpenMenu>,
    track_pending: bool,
    track_done: Option<u32>,
    timers: Vec<(u32, u32, u32, u64, u32)>, // hwnd, id, interval ms, next due, callback
    now_ms: u64,
    keys: [bool; 256],
    fonts: BTreeMap<u32, FontSpec>,
    bitmaps: BTreeMap<u32, Bitmap>,
    stock_bitmap: u32,
    client_procs_w: [u32; 14],
    pub keep_snapshot: bool,
    pub snapshot: Option<(u32, u32, Vec<u32>)>,
}

impl Wm {
    pub(crate) fn extra_scroll(&self, hwnd: u32, bar: u32) -> [i32; 5] { self.extra.scroll.get(&(hwnd, bar)).copied().unwrap_or([0, 100, 0, 0, 0]) }

    pub fn set_screen(&mut self, w: i32, h: i32) {
        self.screen = (w, h);
        if let Some(d) = self.windows.get_mut(&DESKTOP) { d.rect = [0, 0, w, h]; }
        self.backend.gen += 1;
    }

    pub fn new() -> Self {
        let mut w = Wm {
            free_gdi: Vec::new(), dcattr_free: Vec::new(), cs_arena: (0, 0),
            screen: (1024, 768),
            gl: Default::default(),
            classes: Vec::new(), windows: BTreeMap::new(), queue: VecDeque::new(), quit: None, next_hwnd: 0x10030,
            gdi: BTreeMap::new(), dcs: BTreeMap::new(), next_gdi: 1, backend: SoftBackend::new(), time: 1000, keep_snapshot: false, snapshot: None, zorder: Vec::new(), active: 0, drag: None, capture: 0, focus: 0, default_objs: [0; 3], extra: Default::default(), menus: BTreeMap::new(), next_menu: 0x30020, open: None, track_pending: false, track_done: None, timers: Vec::new(), now_ms: 0, keys: [false; 256], fonts: BTreeMap::new(), bitmaps: BTreeMap::new(), stock_bitmap: 0, client_procs_w: [0; 14],
        };
        w.classes.push(Class { atom: 0xC000, name: "#32769".into(), wndproc: 0, bg: 0, raw: [0; 12], ansi: false });
        w.windows.insert(DESKTOP, Window { class: 0, style: WS_VISIBLE, rect: [0, 0, 1024, 768], parent: 0, text: String::new(), visible: true, invalid: None, needs_erase: false, surface: None, ex_style: 0, menu: 0, id: 0, inst: 0, userdata: 0, proc_override: 0, extra: Vec::new() });
        w
    }
    fn new_gdi(&mut self, kind: u32, g: Gdi) -> u32 {
        let idx = match self.free_gdi.pop() { Some(i) => i, None => { let n = self.next_gdi; self.next_gdi += 1; n } };
        let h = kind | idx;
        self.gdi.insert(h, g);
        h
    }
    fn client_off(style: u32) -> (i32, i32) {
        if style & WS_CAPTION == WS_CAPTION { (BORDER, BORDER + CAPTION) } else { (0, 0) }
    }
    fn client_rect(&self, hwnd: u32) -> Rect {
        let Some(w) = self.windows.get(&hwnd) else { return [0; 4] };
        let (ox, oy) = if w.surface.is_some() { let (a, b) = Self::client_off(w.style); (a, b + if w.menu != 0 { MENU_H } else { 0 }) } else { (0, 0) };
        [0, 0, (w.rect[2] - w.rect[0] - 2 * ox).max(0), (w.rect[3] - w.rect[1] - oy - ox).max(0)]
    }
}

fn colorref(c: u32) -> u32 { 0xFF00_0000 | (c & 0xff) << 16 | (c & 0xff00) | (c >> 16 & 0xff) }
fn sys_color(i: u32) -> u32 {
    // Plonk Glass (light): see docs/DESIGN.md section 3.1
    const T: [u32; 25] = [0xF0F2F5, 0x6A8FD8, 0x0A64C8, 0xDADDE1, 0xF7F8FA, 0xFFFFFF, 0x8A93A0, 0x1A1A1A, 0x1A1A1A, 0x1A1A1A, 0xF0F2F5, 0xF0F2F5, 0x9AA3B0,
        0x0A64C8, 0xFFFFFF, 0xF1F3F6, 0xA7B0BC, 0x8A8F96, 0x1A1A1A, 0x5C6168, 0xFFFFFF, 0x8E98A6, 0xE6EAF0, 0x1A1A1A, 0xFFFFE1];
    0xFF00_0000 | T.get(i as usize).copied().unwrap_or(0xF1F3F6)
}
/// GetSystemMetrics values (96 dpi) matching the Plonk Glass metrics in docs/DESIGN.md.
fn system_metric(i: u32, screen: (i32, i32)) -> u32 {
    match i {
        0 | 16 | 61 | 78 | 59 => screen.0 as u32, 1 | 79 | 60 => screen.1 as u32, 17 | 62 => (screen.1 - 40).max(1) as u32,
        2 | 3 | 9 | 10 | 20 | 21 => 16, 4 => 28, 5 | 6 | 83 | 84 => 1, 7 | 8 => 3, 11 | 12 | 13 | 14 => 32, 15 => 24,
        19 | 75 | 81 | 80 => 1, 28 | 34 => 120, 29 | 35 => 40, 30 => 36, 31 => 22, 32 | 33 => 4, 36 | 37 | 68 | 69 => 4, 38 | 39 => 75,
        43 => 3, 45 | 46 => 2, 47 | 57 => 160, 48 | 58 => 24, 49 | 50 => 16, 51 => 20, 52 => 28, 53 => 20, 54 | 55 => 18, 56 => 8, 63 => 1, 71 | 72 => 13,
        76 | 77 => 0,
        _ => 0,
    }
}

fn argb_to_colorref(c: u32) -> u32 { (c & 0xff) << 16 | (c & 0xff00) | (c >> 16 & 0xff) }

impl WineProc {
    // ---- guest memory helpers ----
    fn rect_at(&self, p: u32) -> Res<Rect> {
        let mut r = [0i32; 4];
        for (i, v) in r.iter_mut().enumerate() { *v = self.r32(p + i as u32 * 4)? as i32; }
        Ok(r)
    }
    fn put_rect(&mut self, p: u32, r: Rect) -> Res<()> {
        for (i, v) in r.iter().enumerate() { self.w32(p + i as u32 * 4, *v as u32)?; }
        Ok(())
    }
    fn wstr(&self, p: u32, n: u32) -> Res<String> {
        let mut v = Vec::new();
        for i in 0..n { v.push(self.mem.r16(p + i * 2).map_err(|_| "wstr")?); }
        Ok(String::from_utf16_lossy(&v))
    }
    /// A class/window name argument: UNICODE_STRING*, where Length 0 + small Buffer means an atom.
    fn class_arg(&self, us: u32) -> Res<Result<u32, String>> {
        let (len, buf) = (self.mem.r16(us).map_err(|_| "us")? as u32, self.r32(us + 4)?);
        if buf < 0x10000 { return Ok(Ok(buf)); }
        Ok(Err(self.wstr(buf, len / 2)?))
    }

    /// Call a guest window procedure (stdcall, 4 args) and return its result.
    pub(crate) fn call_guest(&mut self, func: u32, args: &[u32]) -> Res<u32> {
        let saved = self.cpu.clone();
        let mut sp = (self.cpu.r[ESP].wrapping_sub(0x400)) & !15;
        for v in args.iter().rev() { sp -= 4; self.w32(sp, *v)?; }
        sp -= 4;
        self.w32(sp, super::CALL_RETURN_THUNK)?;
        self.cpu.r[ESP] = sp;
        self.cpu.eip = func;
        self.guest_returned = false;
        self.cb_depth += 1;
        #[cfg(not(target_arch = "wasm32"))]
        let wd_start = std::time::Instant::now();
        #[cfg(target_arch = "wasm32")]
        let wd_start = ();
        while !self.guest_returned {
            if let Err(e) = self.watchdog(&wd_start, "call_guest") { self.cb_depth -= 1; return Err(e); }
            if let Err(e) = self.pump() { self.cb_depth -= 1; return Err(e); }
            if self.exit.is_some() { self.cb_depth -= 1; return Err("process exited inside window procedure".into()); }
            if self.need_file.is_some() && !self.guest_returned {
                // a lazy file request raised inside a callback: unwind and let the shell supply it, then the callback resumes
                self.cb_depth -= 1;
                self.orphans.push(saved);
                return Err(super::YIELD.into());
            }
            if self.busy && !self.guest_returned {
                // the callback used up its whole slice (a long loop inside a window procedure, e.g. a game loop): suspend it exactly like
                // a modal loop so the page keeps painting and handling input; the original call completes when the procedure returns
                self.cb_depth -= 1;
                self.orphans.push(saved);
                return Err(super::YIELD.into());
            }
            if std::mem::take(&mut self.blocked) && !self.guest_returned {
                // a wait inside a callback: let guest time pass; if nothing can ever wake it, hand control to the shell
                self.no_progress += 1;
                if let super::threads::Stuck::Idle = self.on_stuck() { self.idle = true; self.parked = true; }
            }
            if self.idle && self.parked && !self.guest_returned {
                // the callback started a modal loop and now waits for input: unwind the Rust frames but keep the
                // guest mid-callback. When the procedure finally returns, the original syscall completes with its result.
                self.cb_depth -= 1;
                self.orphans.push(saved);
                return Err(super::YIELD.into());
            }
        }
        self.cb_depth -= 1;
        let r = self.cpu.r[EAX];
        self.guest_returned = false; // nested calls must not leak the flag to an outer call_guest
        self.cpu = saved;
        Ok(r)
    }

    fn send(&mut self, hwnd: u32, msg: u32, wp: u32, lp: u32) -> Res<u32> {
        let Some(w) = self.wm.windows.get(&hwnd) else { return Ok(0) };
        let proc_ = if w.proc_override != 0 { w.proc_override } else { self.wm.classes[w.class].wndproc };
        if proc_ == 0 { return Ok(0); }
        self.call_guest(proc_, &[hwnd, msg, wp, lp])
    }

    fn invalidate(&mut self, hwnd: u32, erase: bool) {
        let cr = self.wm.client_rect(hwnd);
        if let Some(w) = self.wm.windows.get_mut(&hwnd) {
            if w.visible { w.invalid = Some(cr); w.needs_erase |= erase; }
        }
        for c in self.children_of(hwnd) { self.invalidate(c, erase); }
    }

    /// Plonk Glass chrome: translucent frame + title bar over the blurred backdrop (the compositor
    /// blurs behind every pixel whose alpha < 255), opaque client area.
    fn paint_frame(&mut self, hwnd: u32) {
        let Some(w) = self.wm.windows.get(&hwnd) else { return };
        let (Some(s), true) = (w.surface, w.style & WS_CAPTION == WS_CAPTION) else { return };
        let (ww, wh) = (w.rect[2] - w.rect[0], w.rect[3] - w.rect[1]);
        let title = w.text.clone();
        let active = self.wm.active == hwnd;
        let b = &mut self.wm.backend;
        let a = |alpha: f32, rgb: u32| ((alpha * 255.0) as u32) << 24 | (rgb & 0xFF_FFFF);
        let top = BORDER + CAPTION;
        // opaque client background, then the glass ring on top of it
        b.fill_rect(s, 0, 0, ww, wh, 0xFFF1_F3F6);
        let (a0, a1) = if active { (0.58, 0.34) } else { (0.40, 0.26) };
        for y in 0..top {
            let t = y as f32 / top as f32;
            b.fill_rect(s, 0, y, ww, 1, a(a0 + (a1 - a0) * t, 0xFFFFFF));
        }
        let side = if active { 0.38 } else { 0.28 };
        b.fill_rect(s, 0, top, BORDER, wh - top, a(side, 0xFFFFFF));
        b.fill_rect(s, ww - BORDER, top, BORDER, wh - top, a(side, 0xFFFFFF));
        b.fill_rect(s, 0, wh - BORDER, ww, BORDER, a(side, 0xFFFFFF));
        // hairline under the title bar and around the client area
        b.fill_rect(s, 0, top - 1, ww, 1, a(0.10, 0x000000));
        for (x, y, w2, h2) in [(BORDER - 1, top, 1, wh - top - BORDER + 1), (ww - BORDER, top, 1, wh - top - BORDER + 1), (BORDER - 1, wh - BORDER, ww - 2 * BORDER + 2, 1)] {
            b.fill_rect(s, x, y, w2, h2, a(0.12, 0x000000));
        }
        // luminous edge: specular top, softer sides, faint bottom
        b.fill_rect(s, 0, 0, ww, 1, a(0.95, 0xFFFFFF));
        b.fill_rect(s, 0, 1, 1, wh - 2, a(0.55, 0xFFFFFF));
        b.fill_rect(s, ww - 1, 1, 1, wh - 2, a(0.40, 0xFFFFFF));
        b.fill_rect(s, 0, wh - 1, ww, 1, a(0.28, 0xFFFFFF));
        // title (etched: light shadow under dark text)
        let (tc, ty) = (if active { 0xFF1A_1A1A } else { 0xFF6B_7078 }, BORDER + (CAPTION - 15) / 2);
        b.draw_text_px(s, BORDER + 11, ty + 1, &title, a(0.7, 0xFFFFFF), 13.0, true);
        b.draw_text_px(s, BORDER + 10, ty, &title, tc, 13.0, true);
        // caption buttons: minimize, maximize, close (neutral glyphs, close gets a red glass pill)
        let bx = ww - BORDER - 3 * CAPBTN_W;
        let (by, bh) = (BORDER + 3, CAPTION - 6);
        b.fill_rect(s, bx + 2 * CAPBTN_W, by, CAPBTN_W, bh, a(if active { 0.80 } else { 0.35 }, 0xE5484D));
        let gc = if active { 0xFF2A_2E35 } else { 0xFF80_858D };
        let cy = by + bh / 2;
        b.fill_rect(s, bx + CAPBTN_W / 2 - 5, cy + 3, 10, 1, gc); // minimize
        let mx = bx + CAPBTN_W + CAPBTN_W / 2 - 5;
        for (x, y, w2, h2) in [(mx, cy - 5, 10, 1), (mx, cy + 4, 10, 1), (mx, cy - 5, 1, 10), (mx + 9, cy - 5, 1, 10)] { b.fill_rect(s, x, y, w2, h2, gc); } // maximize
        let xc = bx + 2 * CAPBTN_W + CAPBTN_W / 2;
        let xcol = if active { 0xFFFF_FFFF } else { 0xFF80_858D };
        for i in -4..=4 { b.fill_rect(s, xc + i, cy + i, 1, 1, xcol); b.fill_rect(s, xc + i, cy - i, 1, 1, xcol); } // close
        self.paint_menu_bar(hwnd);
    }

    fn show_window(&mut self, hwnd: u32, show: bool) -> Res<bool> {
        let Some(w) = self.wm.windows.get_mut(&hwnd) else { return Ok(false) };
        let was = w.visible;
        w.visible = show;
        if let Some(s) = w.surface { self.wm.backend.place(s, w.rect[0], w.rect[1], show); }
        if show && !was {
            if w_is_top(self, hwnd) { let old = std::mem::replace(&mut self.wm.active, hwnd); if old != hwnd { self.repaint_frame(old); } }
            self.paint_frame(hwnd);
            self.invalidate(hwnd, true);
        }
        if show != was { self.send(hwnd, WM_SHOWWINDOW, show as u32, 0)?; }
        Ok(was)
    }

    fn destroy_window(&mut self, hwnd: u32) -> Res<bool> {
        if !self.wm.windows.contains_key(&hwnd) || hwnd == DESKTOP { return Ok(false); }
        self.show_window(hwnd, false)?;
        self.send(hwnd, WM_DESTROY, 0, 0)?;
        self.send(hwnd, WM_NCDESTROY, 0, 0)?;
        if let Some(w) = self.wm.windows.remove(&hwnd) { if let Some(s) = w.surface { self.wm.backend.destroy_surface(s); } }
        self.wm.queue.retain(|m| m.hwnd != hwnd);
        self.wm.zorder.retain(|h| *h != hwnd);
        if self.wm.active == hwnd { self.wm.active = self.wm.zorder.last().copied().unwrap_or(0); self.repaint_frame(self.wm.active); }
        Ok(true)
    }

    /// Next message for the (single) thread; paint messages are synthesized from invalid regions.
    fn next_message(&mut self, remove: bool) -> Option<Msg> {
        let m = if let Some(m) = self.wm.queue.front().copied() {
            if remove { self.wm.queue.pop_front(); }
            Some(m)
        } else if let Some(code) = self.wm.quit {
            if remove { self.wm.quit = None; }
            Some(Msg { hwnd: 0, msg: WM_QUIT, wp: code, lp: 0 })
        } else {
            self.wm.windows.iter().find(|(h, w)| **h != DESKTOP && w.visible && w.invalid.is_some() && self.is_shown(**h)).map(|(h, _)| Msg { hwnd: *h, msg: WM_PAINT, wp: 0, lp: 0 })
        };
        m
    }

    fn put_msg(&mut self, p: u32, m: Msg) -> Res<()> {
        self.wm.time += 16;
        for (i, v) in [m.hwnd, m.msg, m.wp, m.lp, self.wm.time, 0, 0].iter().enumerate() { self.w32(p + i as u32 * 4, *v)?; }
        Ok(())
    }

    fn toplevel_of(&self, mut hwnd: u32) -> u32 {
        while let Some(w) = self.wm.windows.get(&hwnd) { if w.surface.is_some() || w.parent == DESKTOP || w.parent == 0 { return hwnd; } hwnd = w.parent; }
        hwnd
    }

    /// Client-area origin of `hwnd` in desktop (screen) coordinates.
    fn client_origin_screen(&self, hwnd: u32) -> (i32, i32) {
        let Some(w) = self.wm.windows.get(&hwnd) else { return (0, 0) };
        if w.surface.is_some() {
            let (ox, oy) = Wm::client_off(w.style);
            (w.rect[0] + ox, w.rect[1] + oy + if w.menu != 0 { MENU_H } else { 0 })
        } else {
            let (px, py) = self.client_origin_screen(w.parent);
            (px + w.rect[0], py + w.rect[1])
        }
    }

    fn get_window_long(&self, hwnd: u32, idx: i32) -> u32 {
        let Some(w) = self.wm.windows.get(&hwnd) else { return 0 };
        match idx {
            -16 => w.style | if w.visible { WS_VISIBLE } else { 0 },
            -20 => w.ex_style,
            -4 => if w.proc_override != 0 { w.proc_override } else { self.wm.classes[w.class].wndproc },
            -6 => w.inst,
            -12 => w.id,
            -21 => w.userdata,
            -8 => 0,
            i if i >= 0 && (i as usize) + 4 <= w.extra.len() => u32::from_le_bytes(w.extra[i as usize..i as usize + 4].try_into().unwrap()),
            _ => 0,
        }
    }

    fn set_window_long(&mut self, hwnd: u32, idx: i32, v: u32) -> u32 {
        let old = self.get_window_long(hwnd, idx);
        let Some(w) = self.wm.windows.get_mut(&hwnd) else { return 0 };
        match idx {
            -16 => w.style = v & !WS_VISIBLE,
            -20 => w.ex_style = v,
            -4 => w.proc_override = v,
            -12 => w.id = v,
            -21 => w.userdata = v,
            i if i >= 0 && (i as usize) + 4 <= w.extra.len() => w.extra[i as usize..i as usize + 4].copy_from_slice(&v.to_le_bytes()),
            _ => {}
        }
        old
    }

    fn is_shown(&self, mut hwnd: u32) -> bool {
        while hwnd != DESKTOP && hwnd != 0 {
            let Some(w) = self.wm.windows.get(&hwnd) else { return false };
            if !w.visible { return false; }
            hwnd = w.parent;
        }
        true
    }

    fn children_of(&self, hwnd: u32) -> Vec<u32> {
        self.wm.windows.iter().filter(|(h, w)| **h != DESKTOP && hwnd != 0 && w.parent == hwnd && w.surface.is_none()).map(|(h, _)| *h).collect()
    }

    /// Deepest visible window under a desktop point (starting from a top-level window).
    fn hit_child(&self, top: u32, x: i32, y: i32) -> u32 {
        for c in self.children_of(top).into_iter().rev() {
            if !self.wm.windows[&c].visible { continue; }
            let (cx, cy) = self.client_origin_screen(c);
            let cr = self.wm.client_rect(c);
            if x >= cx && x < cx + cr[2] && y >= cy && y < cy + cr[3] { return self.hit_child(c, x, y); }
        }
        top
    }

    fn fill_client(&mut self, hwnd: u32, r: Rect, argb: u32) {
        let top = self.toplevel_of(hwnd);
        let Some(s) = self.wm.windows.get(&top).and_then(|w| w.surface) else { return };
        if !self.is_shown(hwnd) { return; }
        let cr = self.wm.client_rect(hwnd);
        let r = [r[0].max(0), r[1].max(0), r[2].min(cr[2]), r[3].min(cr[3])];
        let (ox, oy) = self.client_origin_screen(hwnd);
        let (tx, ty) = { let t = &self.wm.windows[&top]; (t.rect[0], t.rect[1]) };
        self.wm.backend.fill_rect(s, ox - tx + r[0], oy - ty + r[1], r[2] - r[0], r[3] - r[1], argb);
    }

    fn brush_argb(&self, brush: u32) -> u32 {
        match self.wm.gdi.get(&brush) {
            Some(Gdi::Brush(c)) => *c,
            _ if brush != 0 && brush < 0x100 => sys_color(brush - 1),
            _ => 0xFFFF_FFFF,
        }
    }

    /// Built-in scroll bar control: paint track + thumb from the stored scroll info; no tracking yet.
    fn scrollbar_proc(&mut self, hwnd: u32, msg: u32, wp: u32, lp: u32) -> Res<u32> {
        match msg {
            WM_PAINT | WM_ERASEBKGND => {
                let cr = self.wm.client_rect(hwnd);
                let vert = self.wm.windows.get(&hwnd).is_some_and(|w| w.style & 1 != 0);
                let e = self.wm.extra_scroll(hwnd, 2);
                self.fill_client(hwnd, cr, 0xFFE9_ECF0);
                let (len, span) = if vert { (cr[3], (e[1] - e[0] + 1).max(1)) } else { (cr[2], (e[1] - e[0] + 1).max(1)) };
                let (page, pos) = (e[2].max(1), e[3] - e[0]);
                let th = (len * page / span).clamp(16, len.max(16)); let off = ((len - th).max(0) as i64 * pos as i64 / (span - page).max(1) as i64) as i32;
                let r = if vert { [2, off, cr[2] - 2, off + th] } else { [off, 2, off + th, cr[3] - 2] };
                self.fill_client(hwnd, r, 0xFFB5_BCC6);
                if msg == WM_PAINT { if let Some(w) = self.wm.windows.get_mut(&hwnd) { w.invalid = None; } }
                Ok(1)
            }
            _ => self.def_window_proc(hwnd, msg, wp, lp),
        }
    }

    fn def_window_proc(&mut self, hwnd: u32, msg: u32, wp: u32, lp: u32) -> Res<u32> {
        Ok(match msg {
            WM_CLOSE => { self.destroy_window(hwnd)?; 0 }
            WM_SYSCOMMAND if wp & 0xFFF0 == 0xF060 => { self.send(hwnd, WM_CLOSE, 0, 0)?; 0 }
            WM_PAINT => { if let Some(w) = self.wm.windows.get_mut(&hwnd) { w.invalid = None; } 0 }
            WM_ERASEBKGND => {
                let bg = self.wm.windows.get(&hwnd).map(|w| self.wm.classes[w.class].bg).unwrap_or(0);
                let c = self.brush_argb(bg);
                let cr = self.wm.client_rect(hwnd);
                self.fill_client(hwnd, cr, c);
                1
            }
            0x132..=0x138 => {
                // WM_CTLCOLOR*: set the DC's text / background colours and return the matching system brush
                let editlike = matches!(msg, 0x133 | 0x134);
                let (bk_idx, bg) = if editlike { (5u32, sys_color(5)) } else { (15, sys_color(15)) };
                let at = self.r32(self.gdi_table + (wp & 0xffff) * 24 + 16).unwrap_or(0);
                if at != 0 {
                    let tc = argb_to_colorref(sys_color(8));
                    let _ = self.mem.w32(at + 24, tc);
                    let _ = self.mem.w32(at + 12, argb_to_colorref(bg));
                    let _ = self.mem.w16(at + 50, 2);
                }
                let h = match self.wm.extra.brush_cache.get(&bk_idx) { Some(h) => *h, None => { let h = self.gdi_alloc(0x100000 | 0x80_0000, Gdi::Brush(bg)); self.wm.extra.brush_cache.insert(bk_idx, h); h } };
                h
            }
            WM_NCHITTEST => 1,
            WM_NCCREATE => 1,
            0x0E => self.wm.windows.get(&hwnd).map_or(0, |w| w.text.encode_utf16().count() as u32), // WM_GETTEXTLENGTH
            0x0D => { // WM_GETTEXT (wide): wp = buffer size in characters
                let text: Vec<u16> = self.wm.windows.get(&hwnd).map(|w| w.text.encode_utf16().collect()).unwrap_or_default();
                if wp == 0 || lp == 0 { return Ok(0); }
                let n = text.len().min(wp as usize - 1);
                for (i, c) in text.iter().take(n).enumerate() { self.w16(lp + i as u32 * 2, *c)?; }
                self.w16(lp + n as u32 * 2, 0)?;
                n as u32
            }
            0x0C => { // WM_SETTEXT (wide)
                #[cfg(not(target_arch = "wasm32"))]
                if std::env::var("SETTEXT_LOG").is_ok() { let b: Vec<u8> = (0..24).map(|i| self.mem.r8(lp + i).unwrap_or(0)).collect(); eprintln!("settext(wide path) hwnd={hwnd:#x} bytes={b:02x?}"); }
                let mut n = 0;
                while self.mem.r16(lp + n * 2).unwrap_or(0) != 0 && n < 512 { n += 1; }
                let t = self.wstr(lp, n)?;
                if let Some(w) = self.wm.windows.get_mut(&hwnd) { w.text = t; }
                1
            }
            _ => 0,
        })
    }

    // ---- syscall entry ----
    /// Ok(None): not handled here.
    pub(crate) fn win32u_call(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        if let Some(v) = self.gdi_batch_call(name, a)? { return Ok(Some(v)); }
        if let Some(v) = self.more_call(name, a)? { return Ok(Some(v)); }
        if let Some(v) = self.user_batch_call(name, a)? { return Ok(Some(v)); }
        if let Some(v) = self.misc_batch_call(name, a)? { return Ok(Some(v)); }
        if let Some(v) = self.gl_nt(name, a)? { return Ok(Some(v)); }
        Ok(Some(match name {
            "NtUserInitializeClientPfnArrays" => {
                let mut procs = [0u32; 14];
                for (i, p) in procs.iter_mut().enumerate() { *p = self.r32(a[1] + i as u32 * 4)?; }
                self.wm.client_procs_w = procs;
                // win32u registers the built-in control classes with user32's window procedures
                for (name, idx) in [("Button", 0), ("ComboBox", 1), ("#32770", 3), ("Edit", 4), ("ListBox", 5), ("MDIClient", 6), ("ScrollBar", 7), ("Static", 8), ("IME", 9), ("ComboLBox", 5), ("Message", 13), ("#32768", 12), ("SysShadow", 10)] {
                    let atom = if name == "#32770" { 0x8002 } else { 0xC001 + self.wm.classes.len() as u32 };
                    self.wm.classes.push(Class { atom, name: name.into(), wndproc: procs[idx], bg: 0, raw: [48, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 0], ansi: false });
                }
                0
            }
            "NtUserSetCapture" => std::mem::replace(&mut self.wm.capture, a[0]),
            "NtUserQueryInputContext" => 0,
            "NtUserSetCursorIconData" | "NtUserSetCursor" | "NtUserDestroyCursor" => 1,
            "NtUserFindExistingCursorIcon" => 0,
            "NtGdiOpenDCW" => self.new_dc(DESKTOP, None),
            "NtGdiCreatePatternBrushInternal" => self.gdi_alloc(0x100000, Gdi::Brush(0xFFC0_C0C0)),
            "NtUserSetTimer" => {
                // (hwnd, id, elapse, callback, tolerance)
                let id = if a[1] == 0 { 0x1000 + self.wm.timers.len() as u32 } else { a[1] };
                self.wm.timers.retain(|t| !(t.0 == a[0] && t.1 == id));
                let ms = a[2].max(10);
                self.wm.timers.push((a[0], id, ms, self.wm.now_ms + ms as u64, a[3]));
                id
            }
            "NtUserKillTimer" => { let n = self.wm.timers.len(); self.wm.timers.retain(|t| !(t.0 == a[0] && t.1 == a[1])); (n != self.wm.timers.len()) as u32 }
            "NtUserGetKeyboardLayout" => 0x0409_0409,
            "NtUserSetFocus" => {
                let old = std::mem::replace(&mut self.wm.focus, a[0]);
                if old != a[0] {
                    if old != 0 { self.send(old, 0x08, a[0], 0)?; }
                    if a[0] != 0 { self.send(a[0], 0x07, old, 0)?; }
                }
                old
            }
            "NtUserGetKeyState" | "NtUserGetAsyncKeyState" => if self.wm.keys[(a[0] & 0xff) as usize] { 0x8000 } else { 0 },
            "NtUserGetKeyboardState" => { for i in 0..256u32 { self.mem.w8(a[0] + i, if self.wm.keys[i as usize] { 0x80 } else { 0 }).map_err(|_| "kbstate")?; } 1 }
            "NtUserCreateCaret" | "NtUserDestroyCaret" | "NtUserHideCaret" | "NtUserShowCaret" | "NtUserSetCaretPos" => 1,
            "NtUserGetFocus" => self.wm.focus,
            "NtUserGetForegroundWindow" => self.wm.active,
            "NtUserGetSystemDpiForProcess" => 96,
            "NtUserGetProcessDpiAwarenessContext" | "NtUserGetThreadDpiAwarenessContext" => 0x6010,
            "NtUserGetClassInfoEx" => {
                let cname = self.class_arg(a[1])?;
                let Some(c) = self.wm.classes.iter().find(|c| match &cname { Ok(at) => c.atom == *at, Err(n) => c.name.eq_ignore_ascii_case(n) }) else { return Ok(Some(0)) };
                let (raw, atom) = (c.raw, c.atom);
                for (i, v) in raw.iter().enumerate() { self.w32(a[2] + i as u32 * 4, *v)?; }
                self.w32(a[2], 48)?;
                atom
            }
            "NtUserRegisterClassExWOW" => {
                let wc = a[0];
                let name = match self.class_arg(a[1])? { Ok(atom) => format!("#{atom}"), Err(s) => s };
                let atom = 0xC001 + self.wm.classes.len() as u32;
                let (wndproc, bg) = (self.r32(wc + 8)?, self.r32(wc + 32)?);
                let mut raw = [0u32; 12];
                for (i, v) in raw.iter_mut().enumerate() { *v = self.r32(wc + i as u32 * 4)?; }
                self.wm.classes.push(Class { atom, name, wndproc, bg, raw, ansi: a[5] & 1 != 0 });
                atom
            }
            "NtUserCreateWindowEx" => {
                let (ex_style, style) = (a[0], a[4]);
                let cname = self.class_arg(a[1])?;
                let ci = self.wm.classes.iter().position(|c| match &cname { Ok(at) => c.atom == *at, Err(n) => c.name.eq_ignore_ascii_case(n) });
                let Some(ci) = ci else { self.log(format!("  (CreateWindowEx: unknown class {cname:?})")); return Ok(Some(0)); };
                // the window name is always a wide UNICODE_STRING (the ansi flag only says the window itself is an ANSI window)
                let title = if a[3] != 0 { match self.class_arg(a[3])? { Err(s) => s, Ok(_) => String::new() } } else { String::new() };
                let dim = |v: u32, d: i32| if v == CW_USEDEFAULT { d } else { v as i32 };
                // Windows: with x == CW_USEDEFAULT the y argument is ignored (it is a show command), likewise cy when cx == CW_USEDEFAULT
                let (x, y) = if a[5] == CW_USEDEFAULT { (100, 100) } else { (dim(a[5], 100), dim(a[6], 100)) };
                let (cx, cy) = if a[7] == CW_USEDEFAULT { (640, 480) } else { (dim(a[7], 640), dim(a[8], 480)) };
                #[cfg(not(target_arch = "wasm32"))]
                if std::env::var("CREATE_LOG").is_ok() { eprintln!("create {cname:?} title={title:?} codes={:?} ansi_arg={:#x} style={style:#x} parent={:#x} id={:#x}", title.chars().map(|c| c as u32).collect::<Vec<_>>(), a.get(16).copied().unwrap_or(99), a[9], a[10]); }
                let hwnd = self.wm.next_hwnd;
                self.wm.next_hwnd += 4;
                let parent = if a[9] == 0 { DESKTOP } else { a[9] };
                let toplevel = style & WS_CHILD == 0;
                let surface = if toplevel { Some(self.wm.backend.create_surface(cx.max(1) as u32, cy.max(1) as u32)) } else { None };
                if let (Some(sid), true) = (surface, style & WS_CAPTION == WS_CAPTION) { self.wm.backend.set_glass(sid, 10, 24, 28); }
                if toplevel { self.wm.zorder.push(hwnd); }
                self.wm.windows.insert(hwnd, Window { class: ci, style: style & !WS_VISIBLE, rect: [x, y, x + cx, y + cy], parent: if toplevel { DESKTOP } else { parent }, text: title.clone(), visible: false, invalid: None, needs_erase: false, surface, ex_style, menu: if toplevel && self.wm.menus.contains_key(&a[10]) { a[10] } else { 0 }, id: a[10], inst: a[11], userdata: 0, proc_override: 0, extra: vec![0; self.wm.classes[ci].raw[4] as usize] });
                // CREATESTRUCTW on the guest stack for WM_NCCREATE / WM_CREATE
                if self.wm.cs_arena.0 == 0 { self.wm.cs_arena.0 = self.vm_reserve(None, 0x40000).unwrap_or(0); }
                let used0 = self.wm.cs_arena.1;
                let need = 0x40 + ((title.encode_utf16().count() as u32 + 1) * 2 + 15 & !15);
                // the window name can be long (a dialog paragraph): it must never be written onto the guest stack below esp
                let (cs, title_w) = if self.wm.cs_arena.0 != 0 && used0 + need <= 0x40000 {
                    self.wm.cs_arena.1 = used0 + need;
                    (self.wm.cs_arena.0 + used0, title.clone())
                } else { ((self.cpu.r[ESP].wrapping_sub(0x200)) & !15, title.chars().take(180).collect::<String>()) };
                let (name_p, _) = self.put_wstr_at(cs + 0x40, &title_w);
                for (i, v) in [a[12], a[11], a[10], a[9], cy as u32, cx as u32, y as u32, x as u32, style, name_p, 0, a[0]].iter().enumerate() {
                    self.w32(cs + i as u32 * 4, *v)?;
                }
                let created = self.send(hwnd, WM_NCCREATE, 0, cs)? != 0 && self.send(hwnd, WM_CREATE, 0, cs)? != u32::MAX;
                if self.wm.cs_arena.1 >= used0 { self.wm.cs_arena.1 = used0; }
                if !created {
                    self.destroy_window(hwnd)?;
                    return Ok(Some(0));
                }
                if style & WS_VISIBLE != 0 { self.show_window(hwnd, true)?; }
                hwnd
            }
            "NtUserSetWindowLong" | "NtUserSetWindowLongPtr" => self.set_window_long(a[0], a[1] as i32, a[2]),
            "NtUserInvalidateRect" => {
                let hs: Vec<u32> = if a[0] == 0 { self.wm.zorder.clone() } else { vec![a[0]] };
                for h in hs { self.invalidate(h, a[2] != 0); }
                1
            }
            "NtUserGetGUIThreadInfo" => {
                // GUITHREADINFO: cbSize, flags, active, focus, capture, menuOwner, moveSize, caret, rcCaret
                let (active, focus, capture) = (self.wm.active, self.wm.focus, self.wm.capture);
                for (i, v) in [48, 0, active, focus, capture, 0, 0, 0, 0, 0, 0, 0].iter().enumerate() { self.w32(a[1] + i as u32 * 4, *v)?; }
                1
            }
            "NtUserShowWindow" => self.show_window(a[0], a[1] != 0)? as u32,
            "NtUserDestroyWindow" => self.destroy_window(a[0])? as u32,
            "NtUserGetMessage" => {
                let m = match self.next_message(true) {
                    Some(m) => m,
                    None => {
                        // park at the syscall; resumed when input arrives. With several threads, just block so the others run.
                        if self.threads_len() > 1 { self.blocked = true; } else { self.idle = true; self.parked = true; }
                        self.took_over = true;
                        return Ok(Some(0));
                    }
                };
                self.put_msg(a[0], m)?;
                (m.msg != WM_QUIT) as u32
            }
            "NtUserPeekMessage" => match self.next_message(a[4] & 1 != 0) {
                Some(m) => { self.put_msg(a[0], m)?; 1 }
                None => { self.idle = true; 0 } // polling loop: hand control back to the shell
            },
            "NtUserTranslateMessage" => 0,
            "NtUserDispatchMessage" => {
                let m = Msg { hwnd: self.r32(a[0])?, msg: self.r32(a[0] + 4)?, wp: self.r32(a[0] + 8)?, lp: self.r32(a[0] + 12)? };
                self.send(m.hwnd, m.msg, m.wp, m.lp)?
            }
            "NtUserPostMessage" => {
                self.wm.queue.push_back(Msg { hwnd: a[0], msg: a[1], wp: a[2], lp: a[3] });
                1
            }
            "NtUserMessageCall" => {
                let (hwnd, msg, wp, lp, ty) = (a[0], a[1], a[2], a[3], a[5]);
                match ty {
                    0x029e if a[6] != 0 && msg == 0x0E => self.wm.windows.get(&hwnd).map_or(0, |w| w.text.chars().count() as u32), // DefWindowProcA(WM_GETTEXTLENGTH)
                    0x029e if a[6] != 0 && msg == 0x0D && lp != 0 && wp != 0 => { // DefWindowProcA(WM_GETTEXT): narrow to the ANSI buffer
                        let bytes: Vec<u8> = self.wm.windows.get(&hwnd).map(|w| w.text.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect()).unwrap_or_default();
                        let n = bytes.len().min(wp as usize - 1);
                        for (i, b) in bytes.iter().take(n).enumerate() { self.mem.w8(lp + i as u32, *b).map_err(|_| "gettext")?; }
                        self.mem.w8(lp + n as u32, 0).map_err(|_| "gettext")?;
                        n as u32
                    }
                    0x029e if msg == 0x0C && a[6] != 0 && lp != 0 => { // DefWindowProcA(WM_SETTEXT): the string is ANSI
                        let mut bytes = Vec::new();
                        while let Ok(b) = self.mem.r8(lp + bytes.len() as u32) { if b == 0 || bytes.len() >= 512 { break; } bytes.push(b); }
                        if let Some(w) = self.wm.windows.get_mut(&hwnd) { w.text = bytes.iter().map(|&b| b as char).collect(); }
                        1
                    }
                    0x029e => self.def_window_proc(hwnd, msg, wp, lp)?,
                    0x3002 | 0x0303 | 0x0304 | 0x0305 | 0x0306 | 0x0300 => 0, // spy hooks, IME, tray, clipboard window
                    0x029c | 0x029d => self.def_window_proc(hwnd, msg, wp, lp)?,
                    0x029a => self.scrollbar_proc(hwnd, msg, wp, lp)?,
                    0x02b3 => { let r = if a[6] != 0 { self.send_ansi(hwnd, msg, wp, lp)? } else { self.send(hwnd, msg, wp, lp)? }; if a[4] != 0 { self.w32(a[4] + 8, r)?; } 1 }
                    0x02b7 | 0x02b8 => { self.send(hwnd, msg, wp, lp)?; 1 }
                    0x02b1 | 0x3001 | 0x02ab => { // SendMessage / GetDispatchParams: user32 dispatches via win_proc_params
                        #[cfg(not(target_arch = "wasm32"))]
                        if msg == 0x2B && std::env::var("DRAWITEM_LOG").is_ok() { // WM_DRAWITEM: DRAWITEMSTRUCT
                            let g = |i: u32| self.mem.r32(lp + i * 4).unwrap_or(0);
                            eprintln!("WM_DRAWITEM to {hwnd:#x}: ctl={} item={} action={:#x} state={:#x} hdc={:#x} rect=[{},{},{},{}]", g(1), g(2), g(3), g(4), g(6), g(8) as i32, g(9) as i32, g(10) as i32, g(11) as i32);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        if msg == 0x2C && std::env::var("DRAWITEM_LOG").is_ok() { eprintln!("WM_MEASUREITEM to {hwnd:#x}: ctl={} item={}", self.mem.r32(lp + 4).unwrap_or(0), self.mem.r32(lp + 8).unwrap_or(0)); }
                        let Some(w) = self.wm.windows.get(&hwnd) else { return Ok(Some(0)) };
                        let (mut func, class_ansi) = (if w.proc_override != 0 { w.proc_override } else { self.wm.classes[w.class].wndproc }, self.wm.classes[w.class].ansi);
                        if ty == 0x02ab && a[4] != 0 { let given = self.r32(a[4])?; if given != 0 { func = given; } } // CallWindowProc: the caller names the procedure
                        if a[4] != 0 {
                            let v = [func, hwnd, msg, wp, lp, a[6], class_ansi as u32, 0, 0x8000_0010, func, func];
                            for (i, x) in v.iter().enumerate() { self.w32(a[4] + i as u32 * 4, *x)?; }
                            if ty == 0x3001 || ty == 0x02ab { 1 } else { 0 }
                        } else { self.send(hwnd, msg, wp, lp)? }
                    }
                    _ => return Err(format!("NtUserMessageCall type {:#x}", ty)),
                }
            }
            "NtUserBeginPaint" => {
                let (hwnd, ps) = (a[0], a[1]);
                let Some(w) = self.wm.windows.get(&hwnd) else { return Ok(Some(0)) };
                let inv = w.invalid.unwrap_or([0, 0, 0, 0]);
                let erase = w.needs_erase;
                #[cfg(not(target_arch = "wasm32"))]
                if std::env::var("PAINT_LOG").is_ok() { eprintln!("BeginPaint {hwnd:#x} invalid={:?} client={:?}", w.invalid, self.wm.client_rect(hwnd)); }
                let hdc = self.new_dc(hwnd, Some(inv));
                let mut f_erase = 0;
                if erase {
                    f_erase = (self.send(hwnd, WM_ERASEBKGND, hdc, 0)? == 0) as u32;
                    if let Some(w) = self.wm.windows.get_mut(&hwnd) { w.needs_erase = false; }
                }
                if let Some(w) = self.wm.windows.get_mut(&hwnd) { w.invalid = None; }
                self.w32(ps, hdc)?;
                self.w32(ps + 4, f_erase)?;
                self.put_rect(ps + 8, inv)?;
                for o in (24..64).step_by(4) { self.w32(ps + o, 0)?; }
                hdc
            }
            "NtUserEndPaint" => {
                self.wm.backend.present();
                if self.wm.keep_snapshot {
                    if let Some(sf) = self.wm.windows.get(&a[0]).and_then(|w| w.surface).and_then(|s| self.wm.backend.surface(s)) {
                        self.wm.snapshot = Some((sf.w, sf.h, sf.pix.clone()));
                    }
                }
                1
            }
            "NtUserGetDC" => self.new_dc(a[0], None),
            "NtUserGetDCEx" => self.new_dc(a[0], None),
            "NtUserReleaseDC" => {
                let h = a[1];
                if self.wm.dcs.contains_key(&h) {
                    self.free_dc(h);
                    if self.wm.gdi.remove(&h).is_some() && (h & 0xffff) >= 64 { self.wm.free_gdi.push(h & 0xffff); let _ = self.mem.w8(self.gdi_table + (h & 0xffff) * 24 + 14, 0); }
                }
                1
            }
            "NtUserFillRect" => {
                let (hdc, r, brush) = (a[0], self.lp_rect(a[0], self.rect_at(a[1])?), a[2]);
                let c = self.brush_argb(brush);
                self.dc_fill(hdc, r, c);
                1
            }
            "NtGdiCreateSolidBrush" => self.gdi_alloc(0x100000, Gdi::Brush(colorref(a[0]))),
            "NtGdiSelectBrush" => {
                let Some(dc) = self.wm.dcs.get_mut(&a[0]) else { return Ok(Some(STATUS_INVALID_HANDLE)) };
                std::mem::replace(&mut dc.brush, a[1])
            }
            "NtGdiDeleteObjectApp" => {
                if a[0] & 0x80_0000 != 0 { return Ok(Some(1)); } // stock objects cannot be deleted (apps try anyway)
                let ok = self.wm.gdi.remove(&a[0]).is_some();
                self.free_dc(a[0]);
                if ok && a[0] & 0x80_0000 == 0 && (a[0] & 0xffff) >= 64 { self.wm.free_gdi.push(a[0] & 0xffff); }
                if let Some(b) = self.wm.bitmaps.remove(&a[0]) { if a[0] & 0x80_0000 == 0 { self.vm_release(b.addr()); } }
                if ok { let _ = self.mem.w8(self.gdi_table + (a[0] & 0xffff) * 24 + 14, 0); }
                ok as u32
            }
            "NtGdiGetStockObject" => {
                // the stock objects live at fixed slots of the shared handle table (init_stock_objects): return those, always the same handles
                let idx = 32 + a[0];
                if a[0] > 23 { 0 } else {
                    let e = self.gdi_table + idx * 24;
                    if self.mem.r8(e + 14).unwrap_or(0) == 0 { 0 } else { idx | ((self.mem.r16(e + 12).unwrap_or(0) as u32) << 16) }
                }
            }
            "NtUserInternalGetWindowText" => {
                let t = self.wm.windows.get(&a[0]).map(|w| w.text.clone()).unwrap_or_default();
                let u: Vec<u16> = t.encode_utf16().take((a[2] as usize).saturating_sub(1)).collect();
                for (i, c) in u.iter().enumerate() { self.w16(a[1] + i as u32 * 2, *c)?; }
                if a[2] > 0 { self.w16(a[1] + u.len() as u32 * 2, 0)?; }
                u.len() as u32
            }
            "NtGdiGetAndSetDCDword" => { if a[3] != 0 { self.w32(a[3], 0)?; } 1 }
            "NtGdiComputeXformCoefficients" => 1,
            "NtGdiExtSelectClipRgn" => 2,
            "NtGdiExtCreatePen" => self.gdi_alloc(0x500000, Gdi::Pen(colorref(a[3]), a[0] & 0xf == 5)),
            "NtGdiCreatePen" => self.gdi_alloc(0x300000, Gdi::Pen(colorref(a[2]), a[0] == 5)),
            "NtGdiSelectPen" => {
                let Some(dc) = self.wm.dcs.get_mut(&a[0]) else { return Ok(Some(0)) };
                std::mem::replace(&mut dc.pen, a[1])
            }
            "NtGdiSelectFont" => {
                let Some(dc) = self.wm.dcs.get_mut(&a[0]) else { return Ok(Some(0)) };
                std::mem::replace(&mut dc.font, a[1])
            }
            "NtGdiCreateRectRgn" => self.gdi_alloc(0x040000, Gdi::Region([a[0] as i32, a[1] as i32, a[2] as i32, a[3] as i32])),
            "NtGdiGetRandomRgn" => {
                // code 1 = the DC's clip region (DC-relative coordinates, 0 if there is none - callers restore it with SelectClipRgn(NULL));
                // code 4 = the system/visible region in screen coordinates
                let Some(dc) = self.wm.dcs.get(&a[0]) else { return Ok(Some(0)) };
                let (hwnd, clip) = (dc.hwnd, dc.clip);
                match a[2] & 0xff {
                    1 => match clip { Some(c) => { self.wm.gdi.insert(a[1], Gdi::Region(c)); 1 } None => 0 },
                    4 => {
                        let (ox, oy) = self.client_origin_screen(hwnd);
                        let c = clip.unwrap_or_else(|| self.wm.client_rect(hwnd));
                        self.wm.gdi.insert(a[1], Gdi::Region([ox + c[0], oy + c[1], ox + c[2], oy + c[3]]));
                        1
                    }
                    _ => 0,
                }
            }
            "NtGdiIntersectClipRect" => {
                let r = [a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32];
                if let Some(dc) = self.wm.dcs.get_mut(&a[0]) {
                    dc.clip = Some(match dc.clip { Some(c) => [c[0].max(r[0]), c[1].max(r[1]), c[2].min(r[2]), c[3].min(r[3])], None => r });
                }
                2
            }
            "NtGdiMoveTo" => {
                let np = self.lp(a[0], a[1] as i32, a[2] as i32);
                if let Some(dc) = self.wm.dcs.get_mut(&a[0]) { let old = dc.pos; dc.pos = np; if a[3] != 0 { self.w32(a[3], old.0 as u32)?; self.w32(a[3] + 4, old.1 as u32)?; } }
                1
            }
            "NtGdiLineTo" => {
                let Some(dc) = self.wm.dcs.get(&a[0]) else { return Ok(Some(0)) };
                let (from, pen) = (dc.pos, dc.pen);
                let to = self.lp(a[0], a[1] as i32, a[2] as i32);
                self.dc_line(a[0], from, to, pen);
                if let Some(dc) = self.wm.dcs.get_mut(&a[0]) { dc.pos = to; }
                1
            }
            "NtGdiRectangle" => {
                let (hdc, r) = (a[0], self.lp_rect(a[0], [a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32]));
                let Some(dc) = self.wm.dcs.get(&hdc) else { return Ok(Some(0)) };
                let (brush, pen) = (dc.brush, dc.pen);
                if !matches!(self.wm.gdi.get(&brush), Some(Gdi::NullBrush)) { let c = self.brush_argb(brush); self.dc_fill(hdc, [r[0] + 1, r[1] + 1, r[2] - 1, r[3] - 1], c); }
                for (f, t) in [((r[0], r[1]), (r[2] - 1, r[1])), ((r[2] - 1, r[1]), (r[2] - 1, r[3] - 1)), ((r[2] - 1, r[3] - 1), (r[0], r[3] - 1)), ((r[0], r[3] - 1), (r[0], r[1]))] { self.dc_line(hdc, f, t, pen); }
                1
            }
            "NtGdiPatBlt" => {
                let (hdc, rop) = (a[0], a[5]);
                let ((x, y), (w, h)) = (self.lp(hdc, a[1] as i32, a[2] as i32), self.lp_size(hdc, a[3] as i32, a[4] as i32));
                let Some(dc) = self.wm.dcs.get(&hdc) else { return Ok(Some(0)) };
                let c = match rop { 0x42 => 0xFF00_0000, 0xFF0062 => 0xFFFF_FFFF, _ => self.brush_argb(dc.brush) };
                self.dc_fill(hdc, [x, y, x + w, y + h], c);
                1
            }
            "NtGdiGetDeviceCaps" => match a[1] { 8 => self.wm.screen.0 as u32, 10 => self.wm.screen.1 as u32, 12 => 32, 14 => 1, 88 | 90 => 96, 24 => 0xFFFF_FFFF, _ => 0 },
            "NtUserCallNoParam" => match codes::CALL_NO_PARAM.get(a[0] as usize).copied() {
                Some("GetDesktopWindow") => DESKTOP,
                Some("ReleaseCapture") => { self.wm.capture = 0; 1 }
                n => { self.note(format!("NtUserCallNoParam {n:?}")); return Ok(Some(0)); },
            },
            "NtUserCallOneParam" => match codes::CALL_ONE_PARAM.get(a[1] as usize).copied() {
                Some("GetSystemMetrics") => system_metric(a[0], self.wm.screen),
                Some("CreateCursorIcon") => { let h = self.wm.next_menu; self.wm.next_menu += 4; h }
                Some("CreateMenu") => { let h = self.wm.next_menu; self.wm.next_menu += 4; self.wm.menus.insert(h, Menu::default()); h }
                Some("GetMenuItemCount") => self.menu_count(a[0]),
                Some("GetSysColor") => argb_to_colorref(sys_color(a[0])),
                Some("GetSysColorBrush") => self.gdi_alloc(0x100000 | 0x80_0000, Gdi::Brush(sys_color(a[0]))),
                Some("GetSysColorPen") => self.gdi_alloc(0x300000 | 0x80_0000, Gdi::Pen(sys_color(a[0]), false)),
                n => { self.note(format!("NtUserCallOneParam {n:?} arg={:#x}", a[0])); return Ok(Some(0)); },
            },
            "NtUserCallTwoParam" => match codes::CALL_TWO_PARAM.get(a[2] as usize).copied() {
                Some("SetCaretPos") => 1,
                Some("GetMenuInfo") => { if self.wm.menus.contains_key(&a[0]) { for o in (4..28).step_by(4) { self.w32(a[1] + o, 0)?; } 1 } else { 0 } }
                Some("SetMenuInfo") => 1,
                n => { self.note(format!("NtUserCallTwoParam {n:?} a={:#x} b={:#x}", a[0], a[1])); return Ok(Some(0)); },
            },
            "NtUserCallHwndParam" => match codes::CALL_HWND_PARAM.get(a[2] as usize).copied() {
                Some("GetClientRect") => { let r = self.wm.client_rect(a[0]); self.put_rect(a[1], r)?; 1 }
                Some("GetWindowRect") => {
                    let Some(w) = self.wm.windows.get(&a[0]) else { return Ok(Some(0)) };
                    let r = if w.surface.is_some() { w.rect } else {
                        let (x, y) = self.client_origin_screen(a[0]);
                        [x, y, x + w.rect[2] - w.rect[0], y + w.rect[3] - w.rect[1]]
                    };
                    self.put_rect(a[1], r)?;
                    1
                }
                Some(n) if n.starts_with("GetWindowLong") => self.get_window_long(a[0], a[1] as i32),
                n => { self.note(format!("NtUserCallHwndParam {n:?} hwnd={:#x} param={:#x}", a[0], a[1])); return Ok(Some(0)); },
            },
            "NtUserCallHwnd" => match codes::CALL_HWND.get(a[1] as usize).copied() {
                Some("IsWindow") => self.wm.windows.contains_key(&a[0]) as u32,
                Some("NtUserIsCurrentProcessWindow" | "NtUserIsCurrentThreadWindow") => if self.wm.windows.contains_key(&a[0]) { a[0] } else { 0 },
                Some("IsWindowVisible") => self.wm.windows.get(&a[0]).map_or(0, |w| w.visible as u32),
                Some("GetWindowDpiAwarenessContext") => 0x8000_0010,
                Some("GetDpiForWindow") => 96,
                Some("DrawMenuBar") => { self.paint_frame(a[0]); self.invalidate(a[0], true); 1 }
                Some("GetWindowSysSubMenu") => 0,
                Some("GetDialogInfo" | "GetWindowContextHelpId" | "GetWindowInputContext" | "GetDefaultImeWindow" | "GetMDIClientInfo") => 0,
                Some("IsWindowEnabled" | "IsWindowUnicode") => 1,
                Some("GetParent") => self.wm.windows.get(&a[0]).map_or(0, |w| if w.parent == DESKTOP { 0 } else { w.parent }),
                n => { self.note(format!("NtUserCallHwnd {n:?}")); return Ok(Some(0)); },
            },
            _ => match self.menu_call(name, a)? { Some(v) => v, None => return self.gdi_call(name, a) },
        }))
    }

    /// Queue a message for a window (input injection from the shell).
    pub fn post_message(&mut self, hwnd: u32, msg: u32, wp: u32, lp: u32) { self.wm.queue.push_back(Msg { hwnd, msg, wp, lp }); }

    fn repaint_frame(&mut self, hwnd: u32) {
        self.paint_frame(hwnd);
        // the frame repaint wiped the client area: ask the app to repaint it
        self.invalidate(hwnd, true);
    }

    fn window_at(&self, x: i32, y: i32) -> Option<u32> {
        self.wm.zorder.iter().rev().copied().find(|h| {
            self.wm.windows.get(h).is_some_and(|w| w.visible && x >= w.rect[0] && x < w.rect[2] && y >= w.rect[1] && y < w.rect[3])
        })
    }

    /// Advance the shell clock; due timers post WM_TIMER to their windows.
    pub fn tick(&mut self, dt_ms: u32) {
        self.wm.now_ms += dt_ms as u64;
        self.update_kuser();
        self.fire_ktimers();
        let now = self.wm.now_ms;
        let mut due = Vec::new();
        for t in self.wm.timers.iter_mut() {
            if t.3 <= now { due.push((t.0, t.1, t.4)); t.3 = now + t.2 as u64; }
        }
        for (h, id, cb) in due { self.post_message(h, 0x113, id, cb); }
    }

    /// Keyboard input from the shell. kind: 0 = down, 1 = up. `ch` is the translated character (0 if none).
    pub fn key(&mut self, kind: u32, vk: u32, ch: u32) {
        let target = if self.wm.focus != 0 && self.wm.windows.contains_key(&self.wm.focus) { self.wm.focus } else { self.wm.active };
        if target == 0 { return; }
        self.wm.keys[(vk & 0xff) as usize] = kind == 0;
        if kind == 0 {
            self.post_message(target, 0x100, vk, 1);
            if ch != 0 { self.post_message(target, 0x102, ch, 1); }
        } else {
            self.post_message(target, 0x101, vk, 0xC000_0001);
        }
    }

    /// Desktop mouse events from the shell. kind: 0 = down, 1 = move, 2 = up.
    pub fn mouse(&mut self, kind: u32, x: i32, y: i32) {
        self.wm.extra.mouse = (x, y);
        if self.menu_mouse(kind, x, y) { return; }
        match kind {
            0 => {
                let Some(h) = self.window_at(x, y) else { return };
                let (r, style) = { let w = &self.wm.windows[&h]; (w.rect, w.style) };
                if self.wm.active != h {
                    let old = std::mem::replace(&mut self.wm.active, h);
                    self.wm.zorder.retain(|z| *z != h);
                    self.wm.zorder.push(h);
                    if let Some(s) = self.wm.windows[&h].surface { self.wm.backend.raise(s); }
                    self.repaint_frame(old);
                    self.repaint_frame(h);
                }
                let (lx, ly) = (x - r[0], y - r[1]);
                let in_caption = style & WS_CAPTION == WS_CAPTION && ly >= BORDER && ly < BORDER + CAPTION;
                if in_caption && lx >= r[2] - r[0] - BORDER - CAPBTN_W {
                    self.post_message(h, WM_CLOSE, 0, 0);
                } else if in_caption {
                    self.wm.drag = Some((h, lx, ly));
                } else if self.menu_bar_click(h, lx, ly) {
                } else {
                    let t = self.hit_child(h, x, y);
                    let (cx, cy) = self.client_origin_screen(t);
                    self.post_message(t, 0x201, 1, ((y - cy) as u32) << 16 | ((x - cx) as u32 & 0xffff));
                }
            }
            1 if self.wm.drag.is_none() => {
                let t = self.wm.capture;
                if t != 0 && self.wm.windows.contains_key(&t) {
                    let (cx, cy) = self.client_origin_screen(t);
                    self.post_message(t, 0x200, 1, ((y - cy) as u32) << 16 | ((x - cx) as u32 & 0xffff));
                }
            }
            1 => {
                if let Some((h, gx, gy)) = self.wm.drag {
                    if let Some(w) = self.wm.windows.get_mut(&h) {
                        let (ww, wh) = (w.rect[2] - w.rect[0], w.rect[3] - w.rect[1]);
                        w.rect = [x - gx, y - gy, x - gx + ww, y - gy + wh];
                        if let Some(s) = w.surface { self.wm.backend.place(s, w.rect[0], w.rect[1], w.visible); }
                    }
                }
            }
            _ => {
                self.wm.drag = None;
                let t = if self.wm.capture != 0 { self.wm.capture } else { self.window_at(x, y).map_or(0, |h| self.hit_child(h, x, y)) };
                if t != 0 {
                    let (cx, cy) = self.client_origin_screen(t);
                    self.post_message(t, 0x202, 0, ((y - cy) as u32) << 16 | ((x - cx) as u32 & 0xffff));
                }
            }
        }
    }

    /// Mouse click at a desktop position (down + up).
    pub fn click(&mut self, x: i32, y: i32) { self.mouse(0, x, y); self.mouse(2, x, y); }

    /// Composite all visible windows onto the desktop as RGBA bytes (for ImageData).
    pub fn compose_rgba(&self, w: u32, h: u32) -> Vec<u8> {
        let px = self.wm.backend.compose_desktop(w, h);
        let mut out = vec![0xFFu8; px.len() * 4];
        for (o, p) in out.chunks_exact_mut(4).zip(px) { o[0] = (p >> 16) as u8; o[1] = (p >> 8) as u8; o[2] = p as u8; }
        out
    }

    /// Debug: all top-level windows (handle:class:visible:rect).
    pub fn debug_windows(&self) -> String {
        self.wm.windows.iter().filter(|(h, w)| **h != DESKTOP && w.surface.is_some()).map(|(h, w)| format!("{:#x}:{}:{}:{:?}", h, self.wm.classes[w.class].name, w.visible, w.rect)).collect::<Vec<_>>().join(" ")
    }

    /// Debug: nesting depth of host-side guest callbacks.
    pub fn cb_depth_dbg(&self) -> u32 { self.cb_depth }

    /// Debug: send a message to a window as if the program had (returns the result).
    pub fn debug_send(&mut self, hwnd: u32, msg: u32, wp: u32, lp: u32) -> u32 { self.send(hwnd, msg, wp, lp).unwrap_or(0xDEAD_BEEF) }

    /// Debug: every window (children included) with its class, style and rect.
    pub fn debug_all_windows(&self) -> String {
        self.wm.windows.iter().filter(|(h, _)| **h != DESKTOP).map(|(h, w)| format!("{:#x}:{}:{:#x}:{}:p{:#x}:{:?}", h, self.wm.classes[w.class].name, w.style, w.visible, w.parent, w.rect)).collect::<Vec<_>>().join("
")
    }

    /// Debug: windows that still have an invalid region (class, style, visible).
    pub fn debug_invalid(&self) -> String {
        self.wm.windows.iter().filter(|(_, w)| w.invalid.is_some()).map(|(h, w)| format!("{:#x}:{}:{:#x}:{}", h, self.wm.classes[w.class].name, w.style, w.visible)).collect::<Vec<_>>().join(" ")
    }

    pub(crate) fn threads_len(&self) -> usize { self.threads.len() }

    pub(crate) fn next_wm_timer(&self) -> Option<u64> { self.wm.timers.iter().map(|t| t.3).min() }

    /// gdi32's GetStockObject reads the shared handle table directly, so win32u must pre-populate the stock
    /// objects at fixed slots (FIRST_GDI_HANDLE = 32, in Wine's init_stock_objects order).
    pub(crate) fn init_stock_objects(&mut self) -> Res<()> {
        self.wm.next_gdi = 64;
        let brush = |c: u32| (0x10u32, Gdi::Brush(c));
        for i in 0..24u32 {
            let (ext, obj): (u32, Gdi) = match i {
                0 => { let (e, g) = brush(0xFFFF_FFFF); (e, g) }
                1 => { let (e, g) = brush(0xFFC0_C0C0); (e, g) }
                2 => { let (e, g) = brush(0xFF80_8080); (e, g) }
                3 => { let (e, g) = brush(0xFF40_4040); (e, g) }
                4 => { let (e, g) = brush(0xFF00_0000); (e, g) }
                5 => (0x10, Gdi::NullBrush),
                6 => (0x30, Gdi::Pen(0xFFFF_FFFF, false)),
                7 => (0x30, Gdi::Pen(0xFF00_0000, false)),
                8 => (0x30, Gdi::Pen(0, true)),
                10..=14 | 16 | 17 | 21..=23 => (0x0a, Gdi::Font),
                20 => (0x05, Gdi::Bitmap), // DEFAULT_BITMAP: what CreateCompatibleBitmap(dc, 0, 0) and a fresh memory DC select
                15 => (0x08, Gdi::Other),
                18 => { let (e, g) = brush(0xFFFF_FFFF); (e, g) }
                19 => (0x30, Gdi::Pen(0xFF00_0000, false)),
                _ => continue,
            };
            let idx = 32 + i;
            let unique = ext | 0x80; // ExtType | StockFlag
            let e = self.gdi_table + idx * 24;
            self.w32(e, idx)?;          // Object: non-zero cookie
            self.w16(e + 8, 0x20)?;     // Owner.ProcessId
            self.w16(e + 12, unique as u16)?;
            self.mem.w8(e + 14, (ext & 0x1f) as u8).map_err(|_| "stock")?;
            let h = idx | (unique << 16);
            if matches!(obj, Gdi::Font) { self.wm.fonts.insert(h, FontSpec { px: 12.0, bold: false, face: "Inter".into(), raw: Vec::new() }); }
            self.wm.gdi.insert(h, obj);
        }
        // the stock bitmap needs pixel storage too (1x1), keyed by its stock handle
        let hb = 52 | (0x85u32 << 16);
        let (tmp, _) = self.bmp_alloc(1, 1, 32, true, false)?;
        if let Some(b) = self.wm.bitmaps.remove(&tmp) { self.wm.bitmaps.insert(hb, b); }
        self.wm.gdi.remove(&tmp);
        self.wm.stock_bitmap = hb;
        let h = |i: u32, ext: u32| (32 + i) | ((ext | 0x80) << 16);
        self.wm.default_objs = [h(0, 0x10), h(7, 0x30), h(13, 0x0a)];
        Ok(())
    }

    pub(crate) fn wm_now_ms(&self) -> u64 { self.wm.now_ms }

    pub(crate) fn post_quit(&mut self, code: u32) { self.wm.quit = Some(code); }

    /// Allocate a GDI handle and publish it in the shared handle table that gdi32 (PE) reads.
    /// Forget a DC: its DC_ATTR block goes back to the pool (the handle index is recycled by the caller when the object is deleted).
    fn free_dc(&mut self, h: u32) {
        if self.wm.dcs.remove(&h).is_none() { return; }
        let slot = self.gdi_table + (h & 0xffff) * 24;
        let at = self.r32(slot + 16).unwrap_or(0);
        if at != 0 { self.wm.dcattr_free.push(at); let _ = self.mem.w32(slot + 16, 0); }
    }

    fn gdi_alloc(&mut self, kind: u32, g: Gdi) -> u32 {
        let h = self.wm.new_gdi(kind, g);
        let e = self.gdi_table + (h & 0xffff) * 24;
        let ext = (kind >> 16) & 0xff; // ExtType (7 bits) | StockFlag (bit 7)
        let _ = self.mem.w32(e, h & 0xffff); // Object (non-zero cookie)
        let _ = self.mem.w16(e + 8, 0x20); // Owner.ProcessId
        let _ = self.mem.w16(e + 12, ext as u16); // Unique
        let _ = self.mem.w8(e + 14, (ext & 0x1f) as u8); // Type
        h
    }

    fn put_wstr_at(&mut self, at: u32, s: &str) -> (u32, u32) {
        let u: Vec<u16> = s.encode_utf16().collect();
        for (i, c) in u.iter().enumerate() { let _ = self.mem.w16(at + i as u32 * 2, *c); }
        let _ = self.mem.w16(at + u.len() as u32 * 2, 0);
        (at, u.len() as u32)
    }

    fn new_dc(&mut self, hwnd: u32, clip: Option<Rect>) -> u32 {
        // every DC starts with the default stock objects selected (WHITE_BRUSH, BLACK_PEN, SYSTEM_FONT) like on Windows
        let [db, dp, df] = self.wm.default_objs;
        let h = self.gdi_alloc(0x010000, Gdi::Other);
        // DC_ATTR lives in guest memory: gdi32 (PE) reads/writes it directly via the handle table.
        if self.dcattr_next == 0 { self.dcattr_next = self.vm_reserve(None, 0x40000).unwrap_or(0); }
        let at = match self.wm.dcattr_free.pop() { Some(a) => a, None => { let a = self.dcattr_next; self.dcattr_next += 192; a } };
        for o in (0..192).step_by(4) { let _ = self.mem.w32(at + o, 0); }
        let cr = self.wm.client_rect(hwnd);
        let w = |p: &mut Self, off: u32, v: u32| { let _ = p.mem.w32(at + off, v); };
        w(self, 0, h);
        w(self, 12, 0xFF_FFFF); w(self, 16, 0xFF_FFFF); w(self, 20, 0); w(self, 24, 0);
        w(self, 36, 1); w(self, 40, 1);
        let _ = self.mem.w16(at + 50, 2); let _ = self.mem.w16(at + 52, 1);
        let _ = self.mem.w16(at + 54, 13); let _ = self.mem.w16(at + 58, 1);
        w(self, 60, 1);
        for (i, v) in [0, 0, cr[2], cr[3]].iter().enumerate() { w(self, 72 + i as u32 * 4, *v as u32); }
        w(self, 88, 10.0f32.to_bits());
        w(self, 108, 1); w(self, 112, 1); w(self, 124, 1); w(self, 128, 1);
        let (sw, sh) = self.wm.screen; w(self, 132, sw as u32); w(self, 136, sh as u32); w(self, 140, 320); w(self, 144, 240);
        w(self, 148, 1252);
        let _ = self.mem.w32(self.gdi_table + (h & 0xffff) * 24 + 16, at);
        self.wm.dcs.insert(h, Dc { hwnd, clip, brush: db, pen: dp, pos: (0, 0), font: df, bitmap: 0 });
        h
    }

    fn pen_argb(&self, pen: u32) -> Option<u32> {
        match self.wm.gdi.get(&pen) { Some(Gdi::Pen(c, false)) => Some(*c), Some(Gdi::Pen(_, true)) => None, _ => Some(0xFF00_0000) }
    }

    /// 1px Bresenham line in the DC's client coordinates with the pen color.
    fn dc_line(&mut self, hdc: u32, from: (i32, i32), to: (i32, i32), pen: u32) {
        let Some(c) = self.pen_argb(pen) else { return };
        let (mut x, mut y) = from;
        let (dx, dy) = ((to.0 - x).abs(), -(to.1 - y).abs());
        let (sx, sy) = (if x < to.0 { 1 } else { -1 }, if y < to.1 { 1 } else { -1 });
        let mut err = dx + dy;
        loop {
            self.dc_fill(hdc, [x, y, x + 1, y + 1], c);
            if (x, y) == to { break; }
            let e2 = 2 * err;
            if e2 >= dy { err += dy; x += sx; }
            if e2 <= dx { err += dx; y += sy; }
        }
    }

    fn dc_fill(&mut self, hdc: u32, mut r: Rect, argb: u32) {
        let Some(dc) = self.wm.dcs.get(&hdc) else { return };
        if let Some(c) = dc.clip { r = [r[0].max(c[0]), r[1].max(c[1]), r[2].min(c[2]), r[3].min(c[3])]; }
        let (hwnd, bitmap) = (dc.hwnd, dc.bitmap);
        if bitmap != 0 { self.bmp_fill(bitmap, r, argb); return; }
        self.fill_client(hwnd, r, argb);
    }

    fn dc_text(&mut self, hdc: u32, x: i32, y: i32, text: &str) {
        let Some(dc) = self.wm.dcs.get(&hdc) else { return };
        let (hwnd, bitmap) = (dc.hwnd, dc.bitmap);
        let color = colorref(self.r32(self.gdi_table + (hdc & 0xffff) * 24 + 16).and_then(|at| self.r32(at + 24)).unwrap_or(0));
        if bitmap != 0 {
            for (i, c) in text.chars().enumerate() {
                for (row, bits) in plonk_display::glyph8(c).iter().enumerate() {
                    for col in 0..8 { if bits >> col & 1 != 0 { self.bmp_set(bitmap, x + i as i32 * 8 + col, y + row as i32, color); } }
                }
            }
            return;
        }
        let top = self.toplevel_of(hwnd);
        let Some(s) = self.wm.windows.get(&top).and_then(|w| w.surface) else { return };
        if !self.is_shown(hwnd) { return; }
        let (ox, oy) = self.client_origin_screen(hwnd);
        let (tx, ty) = { let t = &self.wm.windows[&top]; (t.rect[0], t.rect[1]) };
        self.wm.backend.draw_text(s, ox - tx + x, oy - ty + y, text, color);
    }
}

fn w_is_top(p: &WineProc, hwnd: u32) -> bool { p.wm.windows.get(&hwnd).is_some_and(|w| w.surface.is_some()) }
