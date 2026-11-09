//! Native replacement for Wine's win32u.so: window manager, message queue and GDI, backed by the
//! `display` crate. Called from the syscall dispatcher for NtUser*/NtGdi*.
use super::{Res, WineProc, STATUS_INVALID_HANDLE};
use crate::ntuser_codes as codes;
use plonk_cpu::{ESP, EAX};
use plonk_display::{Backend, SoftBackend, SurfaceId};
use std::collections::{BTreeMap, VecDeque};

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
const CAPTION: i32 = 23;

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
    id: u32,
    inst: u32,
    userdata: u32,
    proc_override: u32,
    extra: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Msg { hwnd: u32, msg: u32, wp: u32, lp: u32 }

#[derive(Clone, Copy)]
enum Gdi { Brush(u32), NullBrush, Pen(u32, bool), Region(Rect), Font, Other }

struct Dc { hwnd: u32, clip: Option<Rect>, brush: u32, pen: u32, pos: (i32, i32), font: u32 }

pub struct Wm {
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
    client_procs_w: [u32; 14],
    pub keep_snapshot: bool,
    pub snapshot: Option<(u32, u32, Vec<u32>)>,
}

impl Wm {
    pub fn new() -> Self {
        let mut w = Wm {
            classes: Vec::new(), windows: BTreeMap::new(), queue: VecDeque::new(), quit: None, next_hwnd: 0x10030,
            gdi: BTreeMap::new(), dcs: BTreeMap::new(), next_gdi: 1, backend: SoftBackend::new(), time: 1000, keep_snapshot: false, snapshot: None, zorder: Vec::new(), active: 0, drag: None, capture: 0, focus: 0, client_procs_w: [0; 14],
        };
        w.classes.push(Class { atom: 0xC000, name: "#32769".into(), wndproc: 0, bg: 0, raw: [0; 12], ansi: false });
        w.windows.insert(DESKTOP, Window { class: 0, style: WS_VISIBLE, rect: [0, 0, 1024, 768], parent: 0, text: String::new(), visible: true, invalid: None, needs_erase: false, surface: None, ex_style: 0, id: 0, inst: 0, userdata: 0, proc_override: 0, extra: Vec::new() });
        w
    }
    fn new_gdi(&mut self, kind: u32, g: Gdi) -> u32 {
        let h = kind | self.next_gdi;
        self.next_gdi += 1;
        self.gdi.insert(h, g);
        h
    }
    fn client_off(style: u32) -> (i32, i32) {
        if style & WS_CAPTION == WS_CAPTION { (BORDER, BORDER + CAPTION) } else { (0, 0) }
    }
    fn client_rect(&self, hwnd: u32) -> Rect {
        let Some(w) = self.windows.get(&hwnd) else { return [0; 4] };
        let (ox, oy) = if w.surface.is_some() { Self::client_off(w.style) } else { (0, 0) };
        [0, 0, (w.rect[2] - w.rect[0] - 2 * ox).max(0), (w.rect[3] - w.rect[1] - oy - ox).max(0)]
    }
}

fn colorref(c: u32) -> u32 { 0xFF00_0000 | (c & 0xff) << 16 | (c & 0xff00) | (c >> 16 & 0xff) }
fn sys_color(i: u32) -> u32 {
    const T: [u32; 25] = [0xD4D0C8, 0x3A6EA5, 0x0A246A, 0x808080, 0xD4D0C8, 0xFFFFFF, 0x000000, 0x000000, 0x000000, 0xFFFFFF, 0xD4D0C8, 0xD4D0C8, 0x808080,
        0x0A246A, 0xFFFFFF, 0xD4D0C8, 0x808080, 0x808080, 0x000000, 0xD4D0C8, 0xFFFFFF, 0x404040, 0xD4D0C8, 0x000000, 0xFFFFE1];
    0xFF00_0000 | T.get(i as usize).copied().unwrap_or(0xD4D0C8)
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
        while !self.guest_returned {
            self.pump()?;
            if self.exit.is_some() { return Err("process exited inside window procedure".into()); }
        }
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

    fn paint_frame(&mut self, hwnd: u32) {
        let Some(w) = self.wm.windows.get(&hwnd) else { return };
        let (Some(s), true) = (w.surface, w.style & WS_CAPTION == WS_CAPTION) else { return };
        let (ww, wh) = (w.rect[2] - w.rect[0], w.rect[3] - w.rect[1]);
        let title = w.text.clone();
        let b = &mut self.wm.backend;
        b.fill_rect(s, 0, 0, ww, wh, 0xFFD4_D0C8);
        let cap = if self.wm.active == hwnd { 0xFF0A_246A } else { 0xFF80_8080 };
        let b = &mut self.wm.backend;
        b.fill_rect(s, BORDER, BORDER, ww - 2 * BORDER, CAPTION - 4, cap);
        b.draw_text(s, BORDER + 4, BORDER + 6, &title, 0xFFFF_FFFF);
        b.fill_rect(s, ww - BORDER - 20, BORDER + 2, 18, CAPTION - 8, 0xFFC0_3030); // close box
        b.draw_text(s, ww - BORDER - 15, BORDER + 6, "x", 0xFFFF_FFFF);
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
            (w.rect[0] + ox, w.rect[1] + oy)
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
            WM_NCHITTEST => 1,
            WM_NCCREATE => 1,
            0x0C => { // WM_SETTEXT (wide)
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
        Ok(Some(match name {
            "NtUserInitializeClientPfnArrays" => {
                let mut procs = [0u32; 14];
                for (i, p) in procs.iter_mut().enumerate() { *p = self.r32(a[1] + i as u32 * 4)?; }
                self.wm.client_procs_w = procs;
                // win32u registers the built-in control classes with user32's window procedures
                for (name, idx) in [("Button", 0), ("ComboBox", 1), ("#32770", 3), ("Edit", 4), ("ListBox", 5), ("MDIClient", 6), ("ScrollBar", 7), ("Static", 8), ("IME", 9)] {
                    let atom = 0xC001 + self.wm.classes.len() as u32;
                    self.wm.classes.push(Class { atom, name: name.into(), wndproc: procs[idx], bg: 0, raw: [48, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 0], ansi: false });
                }
                0
            }
            "NtUserSetCapture" => std::mem::replace(&mut self.wm.capture, a[0]),
            "NtUserSetFocus" => std::mem::replace(&mut self.wm.focus, a[0]),
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
                let title = if a[3] != 0 { match self.class_arg(a[3])? { Err(s) => s, Ok(_) => String::new() } } else { String::new() };
                let dim = |v: u32, d: i32| if v == CW_USEDEFAULT { d } else { v as i32 };
                let (x, y, cx, cy) = (dim(a[5], 100), dim(a[6], 100), dim(a[7], 640), dim(a[8], 480));
                let hwnd = self.wm.next_hwnd;
                self.wm.next_hwnd += 4;
                let parent = if a[9] == 0 { DESKTOP } else { a[9] };
                let toplevel = style & WS_CHILD == 0;
                let surface = if toplevel { Some(self.wm.backend.create_surface(cx.max(1) as u32, cy.max(1) as u32)) } else { None };
                if toplevel { self.wm.zorder.push(hwnd); }
                self.wm.windows.insert(hwnd, Window { class: ci, style: style & !WS_VISIBLE, rect: [x, y, x + cx, y + cy], parent: if toplevel { DESKTOP } else { parent }, text: title.clone(), visible: false, invalid: None, needs_erase: false, surface, ex_style, id: a[10], inst: a[11], userdata: 0, proc_override: 0, extra: vec![0; self.wm.classes[ci].raw[4] as usize] });
                // CREATESTRUCTW on the guest stack for WM_NCCREATE / WM_CREATE
                let cs = (self.cpu.r[ESP].wrapping_sub(0x800)) & !15;
                let (name_p, _) = self.put_wstr_at(cs + 0x40, &title);
                for (i, v) in [a[12], a[11], a[10], a[9], cy as u32, cx as u32, y as u32, x as u32, style, name_p, 0, a[0]].iter().enumerate() {
                    self.w32(cs + i as u32 * 4, *v)?;
                }
                if self.send(hwnd, WM_NCCREATE, 0, cs)? == 0 || self.send(hwnd, WM_CREATE, 0, cs)? == u32::MAX {
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
            "NtUserShowWindow" => self.show_window(a[0], a[1] != 0)? as u32,
            "NtUserDestroyWindow" => self.destroy_window(a[0])? as u32,
            "NtUserGetMessage" => {
                let m = match self.next_message(true) {
                    Some(m) => m,
                    None => { self.idle = true; self.took_over = true; return Ok(Some(0)); } // park at the syscall; resumed when input arrives
                };
                self.put_msg(a[0], m)?;
                (m.msg != WM_QUIT) as u32
            }
            "NtUserPeekMessage" => match self.next_message(a[4] & 1 != 0) {
                Some(m) => { self.put_msg(a[0], m)?; 1 }
                None => 0,
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
                    0x029e => self.def_window_proc(hwnd, msg, wp, lp)?,
                    0x3002 | 0x0303 | 0x0304 => 0, // spy hooks
                    0x02b1 | 0x3001 => { // SendMessage / GetDispatchParams: user32 dispatches via win_proc_params
                        let Some(w) = self.wm.windows.get(&hwnd) else { return Ok(Some(0)) };
                        let (func, class_ansi) = (self.wm.classes[w.class].wndproc, self.wm.classes[w.class].ansi);
                        if a[4] != 0 {
                            let v = [func, hwnd, msg, wp, lp, a[6], class_ansi as u32, 0, 0x8000_0010, func, func];
                            for (i, x) in v.iter().enumerate() { self.w32(a[4] + i as u32 * 4, *x)?; }
                            if ty == 0x3001 { 1 } else { 0 }
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
            "NtUserReleaseDC" => { self.wm.dcs.remove(&a[1]); 1 }
            "NtUserFillRect" => {
                let (hdc, r, brush) = (a[0], self.rect_at(a[1])?, a[2]);
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
                let ok = self.wm.gdi.remove(&a[0]).is_some();
                if ok { let _ = self.mem.w8(self.gdi_table + (a[0] & 0xffff) * 24 + 14, 0); }
                ok as u32
            }
            "NtGdiGetStockObject" => match a[0] {
                0..=4 => self.gdi_alloc(0x100000 | 0x80_0000, Gdi::Brush([0xFFFF_FFFF, 0xFFC0_C0C0, 0xFF80_8080, 0xFF40_4040, 0xFF00_0000][a[0] as usize])),
                5 => self.gdi_alloc(0x100000 | 0x80_0000, Gdi::NullBrush),
                6 => self.gdi_alloc(0x300000 | 0x80_0000, Gdi::Pen(0xFFFF_FFFF, false)),
                7 => self.gdi_alloc(0x300000 | 0x80_0000, Gdi::Pen(0xFF00_0000, false)),
                8 => self.gdi_alloc(0x300000 | 0x80_0000, Gdi::Pen(0, true)),
                _ => self.gdi_alloc(0x0a0000 | 0x80_0000, Gdi::Font),
            },
            "NtUserInternalGetWindowText" => {
                let t = self.wm.windows.get(&a[0]).map(|w| w.text.clone()).unwrap_or_default();
                let u: Vec<u16> = t.encode_utf16().take((a[2] as usize).saturating_sub(1)).collect();
                for (i, c) in u.iter().enumerate() { self.w16(a[1] + i as u32 * 2, *c)?; }
                if a[2] > 0 { self.w16(a[1] + u.len() as u32 * 2, 0)?; }
                u.len() as u32
            }
            "NtGdiGetTextMetricsW" => {
                let tm: [(u32, u32); 11] = [(0, 8), (4, 7), (8, 1), (12, 0), (16, 0), (20, 8), (24, 8), (28, 400), (32, 0), (36, 96), (40, 96)];
                for (o, v) in tm { self.w32(a[1] + o, v)?; }
                self.w16(a[1] + 44, 32)?; self.w16(a[1] + 46, 255)?; self.w16(a[1] + 48, b'?' as u16)?; self.w16(a[1] + 50, 32)?;
                self.w32(a[1] + 52, 0)?; self.w32(a[1] + 56, 0)?;
                1
            }
            "NtGdiGetTextExtentExW" => {
                // (hdc, str, count, max_extent, *fit, *dx, SIZE*, flags): fixed 8x8 cells
                let n = a[2];
                if a[4] != 0 { self.w32(a[4], n)?; }
                if a[5] != 0 { for i in 0..n { self.w32(a[5] + i * 4, (i + 1) * 8)?; } }
                self.w32(a[6], n * 8)?;
                self.w32(a[6] + 4, 8)?;
                1
            }
            "NtGdiGetAndSetDCDword" => { if a[3] != 0 { self.w32(a[3], 0)?; } 1 }
            "NtGdiComputeXformCoefficients" => 1,
            "NtGdiExtSelectClipRgn" => 2,
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
            "NtGdiGetRandomRgn" => { // visible/clip region of the DC, in screen coordinates
                let Some(dc) = self.wm.dcs.get(&a[0]) else { return Ok(Some(0)) };
                let (hwnd, clip) = (dc.hwnd, dc.clip);
                let (ox, oy) = self.client_origin_screen(hwnd);
                let c = clip.unwrap_or_else(|| self.wm.client_rect(hwnd));
                self.wm.gdi.insert(a[1], Gdi::Region([ox + c[0], oy + c[1], ox + c[2], oy + c[3]]));
                1
            }
            "NtGdiIntersectClipRect" => {
                let r = [a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32];
                if let Some(dc) = self.wm.dcs.get_mut(&a[0]) {
                    dc.clip = Some(match dc.clip { Some(c) => [c[0].max(r[0]), c[1].max(r[1]), c[2].min(r[2]), c[3].min(r[3])], None => r });
                }
                2
            }
            "NtGdiTransformPoints" => { // identity mapping: copy points
                let (src, dst, n) = (a[1], a[2], a[3]);
                for i in 0..n * 2 { let v = self.r32(src + i * 4)?; self.w32(dst + i * 4, v)?; }
                1
            }
            "NtGdiMoveTo" => {
                if let Some(dc) = self.wm.dcs.get_mut(&a[0]) { let old = dc.pos; dc.pos = (a[1] as i32, a[2] as i32); if a[3] != 0 { self.w32(a[3], old.0 as u32)?; self.w32(a[3] + 4, old.1 as u32)?; } }
                1
            }
            "NtGdiLineTo" => {
                let Some(dc) = self.wm.dcs.get(&a[0]) else { return Ok(Some(0)) };
                let (from, pen) = (dc.pos, dc.pen);
                let to = (a[1] as i32, a[2] as i32);
                self.dc_line(a[0], from, to, pen);
                if let Some(dc) = self.wm.dcs.get_mut(&a[0]) { dc.pos = to; }
                1
            }
            "NtGdiRectangle" => {
                let (hdc, r) = (a[0], [a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32]);
                let Some(dc) = self.wm.dcs.get(&hdc) else { return Ok(Some(0)) };
                let (brush, pen) = (dc.brush, dc.pen);
                if !matches!(self.wm.gdi.get(&brush), Some(Gdi::NullBrush)) { let c = self.brush_argb(brush); self.dc_fill(hdc, [r[0] + 1, r[1] + 1, r[2] - 1, r[3] - 1], c); }
                for (f, t) in [((r[0], r[1]), (r[2] - 1, r[1])), ((r[2] - 1, r[1]), (r[2] - 1, r[3] - 1)), ((r[2] - 1, r[3] - 1), (r[0], r[3] - 1)), ((r[0], r[3] - 1), (r[0], r[1]))] { self.dc_line(hdc, f, t, pen); }
                1
            }
            "NtGdiPatBlt" => {
                let (hdc, x, y, w, h, rop) = (a[0], a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32, a[5]);
                let Some(dc) = self.wm.dcs.get(&hdc) else { return Ok(Some(0)) };
                let c = match rop { 0x42 => 0xFF00_0000, 0xFF0062 => 0xFFFF_FFFF, _ => self.brush_argb(dc.brush) };
                self.dc_fill(hdc, [x, y, x + w, y + h], c);
                1
            }
            "NtGdiExtTextOutW" => {
                let (hdc, x, y, str_p, count) = (a[0], a[1] as i32, a[2] as i32, a[5], a[6]);
                let text = self.wstr(str_p, count)?;
                self.dc_text(hdc, x, y, &text);
                1
            }
            "NtGdiGetDeviceCaps" => match a[1] { 8 => 1024, 10 => 768, 12 => 32, 14 => 1, 88 | 90 => 96, 24 => 0xFFFF_FFFF, _ => 0 },
            "NtUserCallNoParam" => match codes::CALL_NO_PARAM.get(a[0] as usize).copied() {
                Some("GetDesktopWindow") => DESKTOP,
                Some("ReleaseCapture") => { self.wm.capture = 0; 1 }
                n => return Err(format!("NtUserCallNoParam {n:?}")),
            },
            "NtUserCallOneParam" => match codes::CALL_ONE_PARAM.get(a[1] as usize).copied() {
                Some("GetSysColor") => argb_to_colorref(sys_color(a[0])),
                Some("GetSysColorBrush") => self.gdi_alloc(0x100000 | 0x80_0000, Gdi::Brush(sys_color(a[0]))),
                Some("GetSysColorPen") => self.gdi_alloc(0x300000 | 0x80_0000, Gdi::Pen(sys_color(a[0]), false)),
                n => return Err(format!("NtUserCallOneParam {n:?} arg={:#x}", a[0])),
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
                n => return Err(format!("NtUserCallHwndParam {n:?} hwnd={:#x} param={:#x}", a[0], a[1])),
            },
            "NtUserCallHwnd" => match codes::CALL_HWND.get(a[1] as usize).copied() {
                Some("IsWindow") => self.wm.windows.contains_key(&a[0]) as u32,
                Some("NtUserIsCurrentProcessWindow" | "NtUserIsCurrentThreadWindow") => if self.wm.windows.contains_key(&a[0]) { a[0] } else { 0 },
                Some("IsWindowVisible") => self.wm.windows.get(&a[0]).map_or(0, |w| w.visible as u32),
                Some("GetWindowDpiAwarenessContext") => 0x8000_0010,
                Some("GetDpiForWindow") => 96,
                Some("GetDialogInfo" | "GetWindowContextHelpId" | "GetWindowInputContext" | "GetDefaultImeWindow" | "GetMDIClientInfo") => 0,
                Some("IsWindowEnabled" | "IsWindowUnicode") => 1,
                Some("GetParent") => self.wm.windows.get(&a[0]).map_or(0, |w| if w.parent == DESKTOP { 0 } else { w.parent }),
                n => return Err(format!("NtUserCallHwnd {n:?}")),
            },
            _ => return Ok(None),
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

    /// Desktop mouse events from the shell. kind: 0 = down, 1 = move, 2 = up.
    pub fn mouse(&mut self, kind: u32, x: i32, y: i32) {
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
                let in_caption = style & WS_CAPTION == WS_CAPTION && ly >= BORDER && ly < BORDER + CAPTION - 4;
                if in_caption && lx >= r[2] - r[0] - BORDER - 20 {
                    self.post_message(h, WM_CLOSE, 0, 0);
                } else if in_caption {
                    self.wm.drag = Some((h, lx, ly));
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
        let px = self.wm.backend.compose(w, h, 0xFF3A_6EA5);
        let mut out = Vec::with_capacity(px.len() * 4);
        for p in px { out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, 0xFF]); }
        out
    }

    pub(crate) fn post_quit(&mut self, code: u32) { self.wm.quit = Some(code); }

    /// Allocate a GDI handle and publish it in the shared handle table that gdi32 (PE) reads.
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
        let h = self.gdi_alloc(0x010000, Gdi::Other);
        // DC_ATTR lives in guest memory: gdi32 (PE) reads/writes it directly via the handle table.
        if self.dcattr_next == 0 { self.dcattr_next = self.vm_reserve(None, 0x40000).unwrap_or(0); }
        let at = self.dcattr_next;
        self.dcattr_next += 192;
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
        w(self, 132, 1024); w(self, 136, 768); w(self, 140, 320); w(self, 144, 240);
        w(self, 148, 1252);
        let _ = self.mem.w32(self.gdi_table + (h & 0xffff) * 24 + 16, at);
        self.wm.dcs.insert(h, Dc { hwnd, clip, brush: 0, pen: 0, pos: (0, 0), font: 0 });
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
        let hwnd = dc.hwnd;
        self.fill_client(hwnd, r, argb);
    }

    fn dc_text(&mut self, hdc: u32, x: i32, y: i32, text: &str) {
        let Some(dc) = self.wm.dcs.get(&hdc) else { return };
        let hwnd = dc.hwnd;
        let color = colorref(self.r32(self.gdi_table + (hdc & 0xffff) * 24 + 16).and_then(|at| self.r32(at + 24)).unwrap_or(0));
        let top = self.toplevel_of(hwnd);
        let Some(s) = self.wm.windows.get(&top).and_then(|w| w.surface) else { return };
        if !self.is_shown(hwnd) { return; }
        let (ox, oy) = self.client_origin_screen(hwnd);
        let (tx, ty) = { let t = &self.wm.windows[&top]; (t.rect[0], t.rect[1]) };
        self.wm.backend.draw_text(s, ox - tx + x, oy - ty + y, text, color);
    }
}

fn w_is_top(p: &WineProc, hwnd: u32) -> bool { p.wm.windows.get(&hwnd).is_some_and(|w| w.surface.is_some()) }
