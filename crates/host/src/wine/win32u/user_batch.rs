//! windows, classes, menus, accelerators, icons/cursors, input, carets, layered windows (win32u calls filled in systematically; see docs/WIN32U_COVERAGE.txt).
use super::*;

const WS_SYSMENU: u32 = 0x0008_0000;
const WS_DISABLED: u32 = 0x0800_0000;
const WS_MINIMIZE: u32 = 0x2000_0000;
const WS_HSCROLL: u32 = 0x0010_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const WM_COMMAND: u32 = 0x111;
const WM_INITMENU: u32 = 0x116;
const WM_INITMENUPOPUP: u32 = 0x117;
const WM_MOUSEHOVER: u32 = 0x2A1;
const WM_MOUSELEAVE: u32 = 0x2A3;
const MF_DIS: u32 = 3; // MF_GRAYED | MF_DISABLED

fn isect(a: Rect, b: Rect) -> Rect { [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])] }
fn is_empty(r: Rect) -> bool { r[2] <= r[0] || r[3] <= r[1] }

#[derive(Clone, Copy)]
struct Frame { w: i32, h: i32, color: u32, alpha: u32, mask: u32, hx: i32, hy: i32 }
#[derive(Default, Clone)]
struct IconData { is_icon: bool, frames: Vec<Frame> }

#[derive(Clone, Copy)]
struct Track { hwnd: u32, flags: u32, hover: u32, px: i32, py: i32, since: u64 }

#[derive(Clone, Copy, Default)]
struct Caret { hwnd: u32, w: i32, h: i32, x: i32, y: i32 }

/// Per-area state (add fields here; `Extra` owns one of these).
#[derive(Default)]
pub struct State {
    accels: BTreeMap<u32, Vec<(u8, u16, u16)>>,
    next_accel: u32,
    class_extra: BTreeMap<usize, Vec<u8>>,
    wnd_rgn: BTreeMap<u32, Rect>,
    layered: BTreeMap<u32, (u32, u32, u32)>, // hwnd -> colour key, alpha, LWA flags
    lock_hwnd: u32,
    caret: Caret,
    cursor: u32,
    icons: BTreeMap<u32, IconData>,
    sys_menus: BTreeMap<u32, u32>,
    menu_info: BTreeMap<u32, [u32; 5]>, // style, cyMax, hbrBack, helpId, menuData
    dwp: BTreeMap<u32, Vec<[u32; 8]>>,
    next_dwp: u32,
    track: Option<Track>,
    ipos: BTreeMap<u32, (u32, Rect, (i32, i32))>,
    sys_colors: BTreeMap<u32, u32>,
    pal: BTreeMap<u32, u32>,
}

impl WineProc {
    // ---- class / window words ----
    fn class_extra_get(&self, ci: usize, off: usize, n: usize) -> u32 {
        let Some(b) = self.wm.extra.user_b.class_extra.get(&ci) else { return 0 };
        if off + n > b.len() { return 0; }
        let mut v = 0u32;
        for i in (0..n).rev() { v = v << 8 | b[off + i] as u32; }
        v
    }
    fn class_extra_set(&mut self, ci: usize, off: usize, n: usize, v: u32) -> bool {
        let size = self.wm.classes[ci].raw[3] as usize;
        if off + n > size { return false; }
        let b = self.wm.extra.user_b.class_extra.entry(ci).or_insert_with(|| vec![0; size]);
        for i in 0..n { b[off + i] = (v >> (8 * i)) as u8; }
        true
    }

    pub(super) fn ub_class_long(&self, hwnd: u32, off: i32) -> u32 {
        let Some(w) = self.wm.windows.get(&hwnd) else { return 0 };
        let c = &self.wm.classes[w.class];
        match off {
            -26 => c.raw[1], -24 => c.wndproc, -20 => c.raw[3], -18 => c.raw[4], -16 => c.raw[5], -14 => c.raw[6],
            -12 => c.raw[7], -10 => c.bg, -8 => c.raw[9], -32 => c.atom, -34 => c.raw[11],
            o if o >= 0 => self.class_extra_get(w.class, o as usize, 4),
            _ => 0,
        }
    }

    fn ub_set_class_long(&mut self, hwnd: u32, off: i32, v: u32) -> u32 {
        let Some(ci) = self.wm.windows.get(&hwnd).map(|w| w.class) else { return 0 };
        let old = self.ub_class_long(hwnd, off);
        let c = &mut self.wm.classes[ci];
        match off {
            -26 => c.raw[1] = v, -24 => { c.wndproc = v; c.raw[2] = v; } -20 => c.raw[3] = v, -18 => c.raw[4] = v, -16 => c.raw[5] = v,
            -14 => c.raw[6] = v, -12 => c.raw[7] = v, -10 => { c.bg = v; c.raw[8] = v; } -8 => c.raw[9] = v, -34 => c.raw[11] = v,
            o if o >= 0 => { if !self.class_extra_set(ci, o as usize, 4, v) { return 0; } }
            _ => return 0,
        }
        old
    }

    pub(super) fn ub_window_word(&self, hwnd: u32, off: i32) -> u32 {
        if off >= 0 {
            let Some(w) = self.wm.windows.get(&hwnd) else { return 0 };
            let o = off as usize;
            return if o + 2 <= w.extra.len() { u16::from_le_bytes([w.extra[o], w.extra[o + 1]]) as u32 } else { 0 };
        }
        self.get_window_long(hwnd, off) & 0xffff
    }

    // ---- menus ----
    fn find_cmd(&self, top: u32, cmd: u32) -> Option<(u32, usize, Option<usize>)> {
        let m = self.wm.menus.get(&top)?;
        for (i, it) in m.items.iter().enumerate() {
            if it.sub == 0 && it.id == cmd && it.flags & 0x800 == 0 { return Some((top, i, None)); }
        }
        for (i, it) in m.items.iter().enumerate() {
            if it.sub != 0 {
                if let Some((h, k, _)) = self.find_cmd(it.sub, cmd) { return Some((h, k, Some(i))); }
            }
        }
        None
    }

    fn make_system_menu(&mut self) -> u32 {
        let h = self.wm.next_menu;
        self.wm.next_menu += 4;
        let mk = |id: u32, flags: u32, text: &str| menu::Item { id, flags, text: text.into(), sub: 0 };
        let items = vec![mk(0xF120, 1, "&Restore"), mk(0xF010, 0, "&Move"), mk(0xF000, 0, "&Size"), mk(0xF020, 0, "Mi&nimize"), mk(0xF030, 0, "Ma&ximize"), mk(0, 0x800, ""), mk(0xF060, 0, "&Close\tAlt+F4")];
        self.wm.menus.insert(h, Menu { items });
        h
    }

    // ---- accelerators ----
    fn accel_translate(&mut self, hwnd: u32, accel: u32, message: u32, wp: u32, lp: u32) -> Res<bool> {
        let Some(tbl) = self.wm.extra.user_b.accels.get(&accel).cloned() else { return Ok(false) };
        let k = |v: u32| self.wm.keys[v as usize];
        let mut mask = 0u32;
        if k(0x11) || k(0xA2) || k(0xA3) { mask |= 8; }
        if k(0x12) || k(0xA4) || k(0xA5) { mask |= 0x10; }
        if k(0x10) || k(0xA0) || k(0xA1) { mask |= 4; }
        let is_char = message == 0x102 || message == 0x106;
        for (virt, key, cmd) in tbl {
            let (virt, cmd) = (virt as u32, cmd as u32);
            if wp != key as u32 { continue; }
            let found = if is_char {
                virt & 1 == 0 && (mask & 0x10) == (virt & 0x10)
            } else if virt & 1 != 0 {
                mask == (virt & 0x1c)
            } else {
                lp & 0x0100_0000 == 0 && virt & 0x10 != 0 && lp & 0x2000_0000 != 0
            };
            if !found { continue; }
            // find the command in the system menu, then in the window menu, like Windows does
            let style = self.wm.windows.get(&hwnd).map_or(0, |w| w.style);
            let sys = self.wm.extra.user_b.sys_menus.get(&hwnd).copied().unwrap_or(0);
            let menu = if style & 0x4000_0000 != 0 { 0 } else { self.wm.windows.get(&hwnd).map_or(0, |w| w.menu) };
            let mut send: Option<(u32, u32, u32)> = Some((WM_COMMAND, 0x10000 | cmd, 0));
            for (top, is_sys) in [(sys, true), (menu, false)] {
                let Some((holder, idx, pos)) = self.find_cmd(top, cmd) else { continue };
                if self.wm.capture != 0 || style & WS_DISABLED != 0 { send = None; break; }
                self.send(hwnd, WM_INITMENU, top, 0)?;
                if let Some(p) = pos {
                    if let Some(sub) = self.wm.menus.get(&top).map(|m| m.items[p].sub) { self.send(hwnd, WM_INITMENUPOPUP, sub, p as u32 | (is_sys as u32) << 16)?; }
                }
                let dis = self.wm.menus.get(&holder).and_then(|m| m.items.get(idx)).map_or(false, |i| i.flags & MF_DIS != 0);
                send = if is_sys { if dis { None } else { Some((WM_SYSCOMMAND, cmd, 0x10000)) } } else if dis || style & WS_MINIMIZE != 0 { None } else { Some((WM_COMMAND, 0x10000 | cmd, 0)) };
                break;
            }
            if let Some((m, w, l)) = send { self.send(hwnd, m, w, l)?; }
            return Ok(true);
        }
        Ok(false)
    }

    // ---- mouse tracking ----
    fn point_in_window(&self, hwnd: u32, x: i32, y: i32) -> bool {
        let Some(w) = self.wm.windows.get(&hwnd) else { return false };
        if !w.visible { return false; }
        let top = if w.surface.is_some() { Some(hwnd) } else { self.window_at(x, y) };
        match top {
            Some(t) if w.surface.is_some() => t == self.window_at(x, y).unwrap_or(0) && self.hit_child(t, x, y) == hwnd,
            Some(t) => self.hit_child(t, x, y) == hwnd,
            None => false,
        }
    }

    fn track_poll(&mut self) {
        let Some(mut t) = self.wm.extra.user_b.track else { return };
        if !self.wm.windows.contains_key(&t.hwnd) { self.wm.extra.user_b.track = None; return; }
        let ((mx, my), now) = (self.wm.extra.mouse, self.wm.now_ms);
        if !self.point_in_window(t.hwnd, mx, my) {
            if t.flags & 2 != 0 { self.post_message(t.hwnd, WM_MOUSELEAVE, 0, 0); }
            self.wm.extra.user_b.track = None;
            return;
        }
        if (mx - t.px).abs() > 4 || (my - t.py).abs() > 4 { t.px = mx; t.py = my; t.since = now; }
        if t.flags & 1 != 0 && now.saturating_sub(t.since) >= t.hover as u64 {
            let (cx, cy) = self.client_origin_screen(t.hwnd);
            self.post_message(t.hwnd, WM_MOUSEHOVER, 0, ((my - cy) as u32) << 16 | ((mx - cx) as u32 & 0xffff));
            t.flags &= !1;
        }
        self.wm.extra.user_b.track = if t.flags & 3 == 0 { None } else { Some(t) };
    }

    // ---- icons ----
    fn copy_bitmap(&mut self, h: u32) -> Res<u32> {
        let Some(b) = self.wm.bitmaps.get(&h).copied() else { return Ok(0) };
        let (nh, addr) = self.bmp_alloc(b.w, b.h, b.bpp, b.top_down, b.dib)?;
        for o in (0..b.stride * b.h as u32).step_by(4) { let v = self.mem.r32(b.addr + o).unwrap_or(0); let _ = self.mem.w32(addr + o, v); }
        Ok(nh)
    }

    fn draw_icon(&mut self, hdc: u32, x: i32, y: i32, f: Frame, cw: i32, ch: i32, flags: u32) {
        if cw <= 0 || ch <= 0 || f.w <= 0 || f.h <= 0 { return; }
        let mut px = self.dc_read_rect(hdc, x, y, cw, ch);
        let (draw_mask, draw_img) = (flags & 1 != 0, flags & 2 != 0);
        for dy in 0..ch {
            for dx in 0..cw {
                let (sx, sy) = (dx * f.w / cw, dy * f.h / ch);
                let i = (dy * cw + dx) as usize;
                let mut d = px[i] & 0xFF_FFFF;
                if f.alpha != 0 && draw_img {
                    let p = self.bmp_get(f.alpha, sx, sy);
                    let a = p >> 24;
                    if a == 0 { continue; }
                    let mut out = 0u32;
                    for sh in [0u32, 8, 16] {
                        let c = ((p >> sh & 0xff) + (d >> sh & 0xff) * (255 - a) / 255).min(255);
                        out |= c << sh;
                    }
                    px[i] = 0xFF00_0000 | out;
                    continue;
                }
                let (and_m, xor_c) = if f.color != 0 {
                    let m = self.bmp_get(f.mask, sx, sy) & 0xFF_FFFF != 0;
                    (if m { 0xFF_FFFF } else { 0 }, self.bmp_get(f.color, sx, sy) & 0xFF_FFFF)
                } else {
                    (if self.bmp_get(f.mask, sx, sy) & 0xFF_FFFF != 0 { 0xFF_FFFF } else { 0 }, self.bmp_get(f.mask, sx, sy + f.h) & 0xFF_FFFF)
                };
                if draw_mask { d &= and_m; }
                if draw_img { d = if draw_mask { d ^ xor_c } else if and_m == 0 { xor_c } else { d }; }
                px[i] = 0xFF00_0000 | d;
            }
        }
        self.dc_write_rect(hdc, x, y, cw, ch, &px);
    }

    fn icon_frame(&self, h: u32, step: u32) -> Option<Frame> {
        let ic = self.wm.extra.user_b.icons.get(&h)?;
        ic.frames.get(step as usize).or(ic.frames.first()).copied()
    }

    fn default_palette(&self) -> u32 {
        let e = self.gdi_table + (32 + 15) * 24;
        if self.mem.r8(e + 14).unwrap_or(0) == 0 { 0 } else { (32 + 15) | ((self.mem.r16(e + 12).unwrap_or(0) as u32) << 16) }
    }

    /// Ok(None) = not handled here (falls through to the next handler / the lenient stub).
    pub(super) fn user_batch_call(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        if !name.starts_with("NtUser") { return Ok(None); }
        Ok(Some(match name {
            // ---- polled by every message loop: deliver pending mouse-tracking notifications, then fall through ----
            "NtUserGetMessage" | "NtUserPeekMessage" => { self.track_poll(); return Ok(None); }

            // ---- classes ----
            "NtUserSetClassLong" | "NtUserSetClassLongPtr" => self.ub_set_class_long(a[0], a[1] as i32, a[2]),
            "NtUserSetClassWord" => {
                let Some(ci) = self.wm.windows.get(&a[0]).map(|w| w.class) else { return Ok(Some(0)) };
                if (a[1] as i32) < 0 { return Ok(Some(0)); }
                let old = self.class_extra_get(ci, a[1] as usize, 2);
                if !self.class_extra_set(ci, a[1] as usize, 2, a[2] & 0xffff) { return Ok(Some(0)); }
                old
            }
            "NtUserUnregisterClass" => {
                let cname = self.class_arg(a[0])?;
                let Some(ci) = self.wm.classes.iter().position(|c| c.raw[2] != 0 && c.atom != 0 && match &cname { Ok(at) => c.atom == *at, Err(n) => c.name.eq_ignore_ascii_case(n) }) else { return Ok(Some(0)) };
                if self.wm.windows.values().any(|w| w.class == ci) { return Ok(Some(0)); }
                // keep the slot (window class indexes stay valid) but make it unfindable
                let c = &mut self.wm.classes[ci];
                c.atom = 0;
                c.name = format!("\u{1}unregistered{ci}");
                1
            }
            "NtUserCallHwndParam" if matches!(codes::CALL_HWND_PARAM.get(a[2] as usize).copied(), Some("GetClassWord")) => {
                let Some(ci) = self.wm.windows.get(&a[0]).map(|w| w.class) else { return Ok(Some(0)) };
                if (a[1] as i32) == -32 { self.wm.classes[ci].atom & 0xffff } else if (a[1] as i32) >= 0 { self.class_extra_get(ci, a[1] as usize, 2) } else { 0 }
            }

            // ---- window words, regions, layered windows, misc window calls ----
            "NtUserSetWindowWord" => {
                let (hwnd, off) = (a[0], a[1] as i32);
                if !self.wm.windows.contains_key(&hwnd) { return Ok(Some(0)); }
                let old = self.ub_window_word(hwnd, off);
                if off >= 0 {
                    let w = self.wm.windows.get_mut(&hwnd).unwrap();
                    let o = off as usize;
                    if o + 2 > w.extra.len() { return Ok(Some(0)); }
                    w.extra[o..o + 2].copy_from_slice(&(a[2] as u16).to_le_bytes());
                } else {
                    let full = self.get_window_long(hwnd, off);
                    self.set_window_long(hwnd, off, (full & 0xffff_0000) | (a[2] & 0xffff));
                }
                old
            }
            "NtUserSetWindowRgn" => {
                if !self.wm.windows.contains_key(&a[0]) { return Ok(Some(0)); }
                if a[1] == 0 { self.wm.extra.user_b.wnd_rgn.remove(&a[0]); } else {
                    let Some(Gdi::Region(r)) = self.wm.gdi.get(&a[1]).copied() else { return Ok(Some(0)) };
                    self.wm.extra.user_b.wnd_rgn.insert(a[0], r);
                }
                if a[2] != 0 { self.invalidate(a[0], true); }
                1
            }
            "NtUserGetWindowRgnEx" => {
                let Some(r) = self.wm.extra.user_b.wnd_rgn.get(&a[0]).copied() else { return Ok(Some(0)) };
                self.wm.gdi.insert(a[1], Gdi::Region(r));
                if is_empty(r) { 1 } else { 2 }
            }
            "NtUserSetLayeredWindowAttributes" => {
                let Some(w) = self.wm.windows.get(&a[0]) else { return Ok(Some(0)) };
                if w.ex_style & WS_EX_LAYERED == 0 { return Ok(Some(0)); }
                self.wm.extra.user_b.layered.insert(a[0], (a[1], a[2] & 0xff, a[3]));
                1
            }
            "NtUserGetLayeredWindowAttributes" => {
                let Some(w) = self.wm.windows.get(&a[0]) else { return Ok(Some(0)) };
                if w.ex_style & WS_EX_LAYERED == 0 { return Ok(Some(0)); }
                let (key, alpha, fl) = self.wm.extra.user_b.layered.get(&a[0]).copied().unwrap_or((0, 0, 0));
                if a[1] != 0 { self.w32(a[1], key)?; }
                if a[2] != 0 { self.mem.w8(a[2], alpha as u8).map_err(|_| "layered")?; }
                if a[3] != 0 { self.w32(a[3], fl)?; }
                1
            }
            "NtUserUpdateLayeredWindow" => {
                // (hwnd, hdc_dst, pts_dst*, size*, hdc_src, pts_src*, key, blend*, flags, dirty*): position/size are applied; pixels are not composited
                let Some(w) = self.wm.windows.get(&a[0]) else { return Ok(Some(0)) };
                if w.ex_style & WS_EX_LAYERED == 0 { return Ok(Some(0)); }
                let r = w.rect;
                let (x, y) = if a[2] != 0 { (self.r32(a[2])? as i32, self.r32(a[2] + 4)? as i32) } else { (r[0], r[1]) };
                let (cx, cy) = if a[3] != 0 { (self.r32(a[3])? as i32, self.r32(a[3] + 4)? as i32) } else { (r[2] - r[0], r[3] - r[1]) };
                self.move_window(a[0], x, y, cx, cy, a[3] != 0, a[2] != 0)?;
                self.wm.extra.user_b.layered.insert(a[0], (a[6], if a[7] != 0 { self.mem.r8(a[7] + 2).unwrap_or(255) as u32 } else { 255 }, a[8]));
                1
            }
            "NtUserShowWindowAsync" => {
                if !self.wm.windows.contains_key(&a[0]) { return Ok(Some(0)); }
                self.show_window(a[0], a[1] != 0)?;
                1
            }
            "NtUserWindowFromDC" => self.wm.dcs.get(&a[0]).map_or(0, |d| if d.bitmap != 0 { 0 } else { d.hwnd }),
            "NtUserLockWindowUpdate" => {
                let st = &mut self.wm.extra.user_b;
                if a[0] == 0 { st.lock_hwnd = 0; 1 } else if st.lock_hwnd != 0 && st.lock_hwnd != a[0] { 0 } else { st.lock_hwnd = a[0]; 1 }
            }
            "NtUserFlashWindowEx" => {
                let hwnd = self.r32(a[0] + 4)?;
                (self.wm.active == hwnd) as u32
            }
            "NtUserRealChildWindowFromPoint" => {
                let (parent, x, y) = (a[0], a[1] as i32, a[2] as i32);
                let cr = self.wm.client_rect(parent);
                if !self.wm.windows.contains_key(&parent) || x < cr[0] || y < cr[1] || x >= cr[2] || y >= cr[3] { return Ok(Some(0)); }
                let mut hit = parent;
                for c in self.children_of(parent).into_iter().rev() {
                    let r = self.wm.windows[&c].rect;
                    if x >= r[0] && x < r[2] && y >= r[1] && y < r[3] { hit = c; break; }
                }
                hit
            }
            "NtUserSetInternalWindowPos" => {
                // (hwnd, cmd, rect*, pt*)
                let rect = if a[2] != 0 { self.rect_at(a[2])? } else { self.wm.windows.get(&a[0]).map_or([0; 4], |w| w.rect) };
                let pt = if a[3] != 0 { (self.r32(a[3])? as i32, self.r32(a[3] + 4)? as i32) } else { (-1, -1) };
                self.wm.extra.user_b.ipos.insert(a[0], (a[1], rect, pt));
                1
            }
            "NtUserGetInternalWindowPos" => {
                let Some(w) = self.wm.windows.get(&a[0]) else { return Ok(Some(0)) };
                let cur = w.rect;
                let (cmd, rect, pt) = self.wm.extra.user_b.ipos.get(&a[0]).copied().unwrap_or((1, cur, (-1, -1)));
                if a[1] != 0 { self.put_rect(a[1], rect)?; }
                if a[2] != 0 { self.w32(a[2], pt.0 as u32)?; self.w32(a[2] + 4, pt.1 as u32)?; }
                cmd
            }
            "NtUserDeferWindowPosAndBand" => {
                // (hdwp, hwnd, after, x, y, cx, cy, flags, unk1, unk2)
                let Some(list) = self.wm.extra.user_b.dwp.get_mut(&a[0]) else { return Ok(Some(0)) };
                let e = [a[1], a[2], a[3], a[4], a[5], a[6], a[7], 0];
                if let Some(old) = list.iter_mut().find(|x| x[0] == a[1]) { *old = e; } else { list.push(e); }
                a[0]
            }
            "NtUserCallOneParam" if matches!(codes::CALL_ONE_PARAM.get(a[1] as usize).copied(), Some("BeginDeferWindowPos")) => {
                let st = &mut self.wm.extra.user_b;
                st.next_dwp += 1;
                let h = 0x7A00_0000 + st.next_dwp * 4;
                st.dwp.insert(h, Vec::new());
                h
            }
            "NtUserEndDeferWindowPosEx" => {
                let Some(list) = self.wm.extra.user_b.dwp.remove(&a[0]) else { return Ok(Some(0)) };
                for e in list {
                    self.more_call("NtUserSetWindowPos", &[e[0], e[1], e[2], e[3], e[4], e[5], e[6]])?;
                }
                1
            }
            "NtUserGetScrollBarInfo" => {
                // (hwnd, objid, SCROLLBARINFO*)
                let Some(w) = self.wm.windows.get(&a[0]) else { return Ok(Some(0)) };
                if self.r32(a[2])? != 60 { return Ok(Some(0)); }
                let (style, wr) = (w.style, w.rect);
                let (bar, present) = match a[1] as i32 { -6 => (0, style & WS_HSCROLL != 0), -5 => (1, style & WS_VSCROLL != 0), -2 | -4 => (2, true), _ => return Ok(Some(0)) };
                let cr = self.wm.client_rect(a[0]);
                let (ox, oy) = self.client_origin_screen(a[0]);
                let vert = bar == 1 || (bar == 2 && style & 1 != 0);
                let rc = match bar { 0 => [ox, oy + cr[3], ox + cr[2], oy + cr[3] + 16], 1 => [ox + cr[2], oy, ox + cr[2] + 16, oy + cr[3]], _ => wr };
                let e = self.wm.extra_scroll(a[0], bar);
                let len = ((if vert { rc[3] - rc[1] } else { rc[2] - rc[0] }) - 32).max(0);
                let span = (e[1] - e[0] + 1).max(1);
                let page = e[2].max(1);
                let th = (len * page / span).clamp(8.min(len), len);
                let off = if span - page > 0 { ((len - th) as i64 * (e[3] - e[0]) as i64 / (span - page) as i64) as i32 } else { 0 };
                self.put_rect(a[2] + 4, rc)?;
                for (i, v) in [16u32, (16 + off) as u32, (16 + off + th) as u32, 0].iter().enumerate() { self.w32(a[2] + 20 + i as u32 * 4, *v)?; }
                for i in 0..6u32 { self.w32(a[2] + 36 + i * 4, if i == 0 && !present { 0x8000 } else { 0 })?; }
                1
            }
            "NtUserGetTitleBarInfo" => {
                let Some(w) = self.wm.windows.get(&a[0]) else { return Ok(Some(0)) };
                if self.r32(a[1])? != 44 { return Ok(Some(0)); }
                let (style, ex, r) = (w.style, w.ex_style, w.rect);
                let cap = style & WS_CAPTION == WS_CAPTION;
                self.put_rect(a[1] + 4, if cap { [r[0] + BORDER, r[1] + BORDER, r[2] - BORDER, r[1] + BORDER + CAPTION] } else { [0; 4] })?;
                let st = [if cap { 0 } else { 0x8000 }, 0x8000, if style & 0x20000 != 0 { 0 } else { 1 }, if style & 0x10000 != 0 { 0 } else { 1 }, if ex & 0x400 != 0 { 0 } else { 0x8000 }, if style & WS_SYSMENU != 0 { 0 } else { 1 }];
                for (i, v) in st.iter().enumerate() { self.w32(a[1] + 20 + i as u32 * 4, *v)?; }
                1
            }
            "NtUserScrollDC" => {
                // (hdc, dx, dy, scroll*, clip*, update_rgn, update_rect*)
                let (hdc, dx, dy) = (a[0], a[1] as i32, a[2] as i32);
                let Some((hwnd, bmp)) = self.wm.dcs.get(&hdc).map(|d| (d.hwnd, d.bitmap)) else { return Ok(Some(0)) };
                let full = if bmp != 0 { self.wm.bitmaps.get(&bmp).map_or([0; 4], |b| [0, 0, b.w, b.h]) } else { self.wm.client_rect(hwnd) };
                let sr = if a[3] != 0 { self.rect_at(a[3])? } else { full };
                let cl = if a[4] != 0 { isect(self.rect_at(a[4])?, full) } else { full };
                let area = isect(sr, cl);
                let dst = isect([area[0] + dx, area[1] + dy, area[2] + dx, area[3] + dy], area);
                if !is_empty(dst) {
                    let px = self.dc_read_rect(hdc, dst[0] - dx, dst[1] - dy, dst[2] - dst[0], dst[3] - dst[1]);
                    self.dc_write_rect(hdc, dst[0], dst[1], dst[2] - dst[0], dst[3] - dst[1], &px);
                }
                let upd = if is_empty(dst) { area } else if dx == 0 && dy == 0 { [0; 4] } else if dx != 0 && dy != 0 { area } else if dy > 0 { [area[0], area[1], area[2], area[1] + dy] } else if dy < 0 { [area[0], area[3] + dy, area[2], area[3]] } else if dx > 0 { [area[0], area[1], area[0] + dx, area[3]] } else { [area[2] + dx, area[1], area[2], area[3]] };
                if a[5] != 0 { self.wm.gdi.insert(a[5], Gdi::Region(upd)); }
                if a[6] != 0 { self.put_rect(a[6], upd)?; }
                1
            }
            "NtUserExcludeUpdateRgn" => if self.wm.windows.get(&a[1]).is_some_and(|w| w.invalid.is_some()) { 2 } else { 1 },

            // ---- caret ----
            "NtUserCreateCaret" => { self.wm.extra.user_b.caret = Caret { hwnd: a[0], w: a[2] as i32, h: a[3] as i32, x: 0, y: 0 }; 1 }
            "NtUserDestroyCaret" => { self.wm.extra.user_b.caret = Caret::default(); 1 }
            "NtUserCallTwoParam" if matches!(codes::CALL_TWO_PARAM.get(a[2] as usize).copied(), Some("SetCaretPos")) => {
                let c = &mut self.wm.extra.user_b.caret;
                if c.hwnd == 0 { return Ok(Some(0)); }
                c.x = a[0] as i32; c.y = a[1] as i32;
                1
            }
            "NtUserGetCaretPos" => {
                let c = self.wm.extra.user_b.caret;
                if c.hwnd == 0 { return Ok(Some(0)); }
                self.w32(a[0], c.x as u32)?; self.w32(a[0] + 4, c.y as u32)?;
                1
            }
            "NtUserGetCaretBlinkTime" => 530,

            // ---- cursors and icons ----
            "NtUserCallOneParam" if matches!(codes::CALL_ONE_PARAM.get(a[1] as usize).copied(), Some("CreateCursorIcon")) => {
                let h = self.wm.next_menu;
                self.wm.next_menu += 4;
                self.wm.extra.user_b.icons.insert(h, IconData { is_icon: a[0] != 0, frames: Vec::new() });
                h
            }
            "NtUserSetCursorIconData" => {
                // (handle, module us*, resname us*, cursoricon_desc*): flags, num_steps, num_frames, delay, frames*, ...
                let desc = a[3];
                if desc == 0 { return Ok(Some(0)); }
                let (nf, fp) = (self.r32(desc + 8)?, self.r32(desc + 16)?);
                let mut frames = Vec::new();
                let animated = self.r32(desc + 4)? != 0; // static icons carry exactly frames[0] (num_frames may be 0)
                for i in 0..(if animated { nf.min(256) } else { 1 }) {
                    let p = fp + i * 28;
                    frames.push(Frame { w: self.r32(p)? as i32, h: self.r32(p + 4)? as i32, color: self.r32(p + 8)?, alpha: self.r32(p + 12)?, mask: self.r32(p + 16)?, hx: self.r32(p + 20)? as i32, hy: self.r32(p + 24)? as i32 });
                }
                self.wm.extra.user_b.icons.entry(a[0]).or_default().frames = frames;
                1
            }
            "NtUserDestroyCursor" => { self.wm.extra.user_b.icons.remove(&a[0]); 1 }
            "NtUserSetCursor" => std::mem::replace(&mut self.wm.extra.user_b.cursor, a[0]),
            "NtUserGetCursor" => self.wm.extra.user_b.cursor,
            "NtUserGetCursorFrameInfo" => {
                if !self.wm.extra.user_b.icons.contains_key(&a[0]) { return Ok(Some(0)); }
                if a[2] != 0 { self.w32(a[2], 0)?; }
                if a[3] != 0 { self.w32(a[3], 1)?; }
                a[0]
            }
            "NtUserGetIconSize" => {
                let Some(f) = self.icon_frame(a[0], a[1]) else { return Ok(Some(0)) };
                if a[2] != 0 { self.w32(a[2], f.w as u32)?; }
                if a[3] != 0 { self.w32(a[3], f.h as u32)?; }
                1
            }
            "NtUserGetIconInfo" => {
                // (icon, ICONINFO*, module us*, resname us*, bpp*, unk)
                let Some(f) = self.icon_frame(a[0], 0) else { return Ok(Some(0)) };
                let is_icon = self.wm.extra.user_b.icons[&a[0]].is_icon;
                let (mask, color) = (self.copy_bitmap(f.mask)?, self.copy_bitmap(f.color)?);
                let bpp = self.wm.bitmaps.get(&f.color).map_or(1, |b| b.bpp);
                for (i, v) in [is_icon as u32, f.hx as u32, f.hy as u32, mask, color].iter().enumerate() { self.w32(a[1] + i as u32 * 4, *v)?; }
                if a[2] != 0 { self.mem.w16(a[2], 0).map_err(|_| "iconinfo")?; }
                if a[3] != 0 { self.mem.w16(a[3], 0).map_err(|_| "iconinfo")?; self.w32(a[3] + 4, 0)?; }
                if a[4] != 0 { self.w32(a[4], bpp)?; }
                1
            }
            "NtUserInternalGetWindowIcon" => {
                if !self.wm.windows.contains_key(&a[0]) { return Ok(Some(0)); }
                let r = self.send(a[0], 0x7F, a[1], 0)?;
                if r != 0 { return Ok(Some(r)); }
                let c = &self.wm.classes[self.wm.windows[&a[0]].class];
                if a[1] == 0 && c.raw[11] != 0 { c.raw[11] } else { c.raw[6] }
            }
            "NtUserDrawIconEx" => {
                // (hdc, x, y, icon, cx, cy, istep, hbr, flags)
                let Some(f) = self.icon_frame(a[3], a[6]) else { return Ok(Some(0)) };
                let flags = if a[8] & 3 == 0 { a[8] | 3 } else { a[8] };
                let (cw, ch) = if a[4] == 0 || a[5] == 0 || a[8] & 8 != 0 { (f.w, f.h) } else { (a[4] as i32, a[5] as i32) };
                self.draw_icon(a[0], a[1] as i32, a[2] as i32, f, cw, ch, flags);
                1
            }

            // ---- system menu, menu info ----
            "NtUserGetSystemMenu" => {
                let Some(w) = self.wm.windows.get(&a[0]) else { return Ok(Some(0)) };
                let has = w.style & WS_SYSMENU != 0;
                if a[1] != 0 {
                    if let Some(old) = self.wm.extra.user_b.sys_menus.remove(&a[0]) { self.wm.menus.remove(&old); }
                    return Ok(Some(0));
                }
                if !has { return Ok(Some(0)); }
                if let Some(m) = self.wm.extra.user_b.sys_menus.get(&a[0]).copied() { return Ok(Some(m)); }
                let m = self.make_system_menu();
                self.wm.extra.user_b.sys_menus.insert(a[0], m);
                m
            }
            "NtUserSetSystemMenu" => {
                if !self.wm.windows.contains_key(&a[0]) || (a[1] != 0 && !self.wm.menus.contains_key(&a[1])) { return Ok(Some(0)); }
                if a[1] == 0 { self.wm.extra.user_b.sys_menus.remove(&a[0]); } else { self.wm.extra.user_b.sys_menus.insert(a[0], a[1]); }
                1
            }
            "NtUserEndMenu" => { if self.wm.open.is_some() { self.close_popup(); } 1 }
            "NtUserDrawMenuBarTemp" => {
                if self.wm.windows.get(&a[0]).is_some_and(|w| w.menu == a[3] && a[3] != 0) { self.paint_menu_bar(a[0]); }
                MENU_H as u32
            }
            "NtUserThunkedMenuInfo" => {
                // (menu, MENUINFO*): cbSize, fMask, dwStyle, cyMax, hbrBack, dwContextHelpID, dwMenuData
                if !self.wm.menus.contains_key(&a[0]) || a[1] == 0 { return Ok(Some(0)); }
                let mask = self.r32(a[1] + 4)?;
                let mut v = self.wm.extra.user_b.menu_info.get(&a[0]).copied().unwrap_or([0; 5]);
                for (bit, slot, off) in [(0x10u32, 0usize, 8u32), (1, 1, 12), (2, 2, 16), (4, 3, 20), (8, 4, 24)] {
                    if mask & bit != 0 { v[slot] = self.r32(a[1] + off)?; }
                }
                self.wm.extra.user_b.menu_info.insert(a[0], v);
                1
            }
            "NtUserCallTwoParam" if matches!(codes::CALL_TWO_PARAM.get(a[2] as usize).copied(), Some("GetMenuInfo")) => {
                if !self.wm.menus.contains_key(&a[0]) || a[1] == 0 { return Ok(Some(0)); }
                let mask = self.r32(a[1] + 4)?;
                let v = self.wm.extra.user_b.menu_info.get(&a[0]).copied().unwrap_or([0; 5]);
                for (bit, slot, off) in [(0x10u32, 0usize, 8u32), (1, 1, 12), (2, 2, 16), (4, 3, 20), (8, 4, 24)] {
                    if mask & bit != 0 { self.w32(a[1] + off, v[slot])?; }
                }
                1
            }
            "NtUserSetMenuContextHelpId" => {
                if !self.wm.menus.contains_key(&a[0]) { return Ok(Some(0)); }
                self.wm.extra.user_b.menu_info.entry(a[0]).or_insert([0; 5])[3] = a[1];
                1
            }

            // ---- accelerator tables ----
            "NtUserCreateAcceleratorTable" => {
                // (ACCEL*, count): ACCEL is { BYTE fVirt; WORD key; WORD cmd } packed to 2 => 6 bytes
                if a[0] == 0 || a[1] == 0 || a[1] > 0x1000 { return Ok(Some(0)); }
                let mut v = Vec::new();
                for i in 0..a[1] {
                    let p = a[0] + i * 6;
                    v.push((self.mem.r8(p).map_err(|_| "accel")?, self.mem.r16(p + 2).map_err(|_| "accel")?, self.mem.r16(p + 4).map_err(|_| "accel")?));
                }
                let st = &mut self.wm.extra.user_b;
                st.next_accel += 1;
                let h = 0x6B00_0000 + st.next_accel * 4;
                st.accels.insert(h, v);
                h
            }
            "NtUserCopyAcceleratorTable" => {
                let Some(t) = self.wm.extra.user_b.accels.get(&a[0]).cloned() else { return Ok(Some(0)) };
                if a[1] == 0 { return Ok(Some(t.len() as u32)); }
                let n = t.len().min(a[2] as usize);
                for (i, (f, k, c)) in t.iter().take(n).enumerate() {
                    let p = a[1] + i as u32 * 6;
                    self.mem.w8(p, *f).map_err(|_| "accel")?; self.mem.w8(p + 1, 0).map_err(|_| "accel")?;
                    self.mem.w16(p + 2, *k).map_err(|_| "accel")?; self.mem.w16(p + 4, *c).map_err(|_| "accel")?;
                }
                n as u32
            }
            "NtUserDestroyAcceleratorTable" => self.wm.extra.user_b.accels.remove(&a[0]).is_some() as u32,
            "NtUserTranslateAccelerator" => {
                // (hwnd, accel, MSG*)
                if a[0] == 0 || a[2] == 0 { return Ok(Some(0)); }
                let (message, wp, lp) = (self.r32(a[2] + 4)?, self.r32(a[2] + 8)?, self.r32(a[2] + 12)?);
                if !matches!(message, 0x100 | 0x104 | 0x102 | 0x106) { return Ok(Some(0)); }
                self.accel_translate(a[0], a[1], message, wp, lp)? as u32
            }

            // ---- input ----
            "NtUserTrackMouseEvent" => {
                let p = a[0];
                if self.r32(p)? != 16 { return Ok(Some(0)); }
                let (fl, hwnd, hover) = (self.r32(p + 4)?, self.r32(p + 8)?, self.r32(p + 12)?);
                if !self.wm.windows.contains_key(&hwnd) { return Ok(Some(0)); }
                let cur = self.wm.extra.user_b.track.filter(|t| t.hwnd == hwnd);
                if fl & 0x4000_0000 != 0 {
                    // TME_QUERY
                    let (f, h) = cur.map_or((0, 0), |t| (t.flags, t.hover));
                    self.w32(p + 4, f)?; self.w32(p + 12, h)?;
                    return Ok(Some(1));
                }
                if fl & 0x8000_0000 != 0 {
                    if let Some(mut t) = cur { t.flags &= !(fl & 3); self.wm.extra.user_b.track = if t.flags & 3 == 0 { None } else { Some(t) }; }
                    return Ok(Some(1));
                }
                let (mx, my) = self.wm.extra.mouse;
                let hover = if hover == u32::MAX || hover == 0 { 400 } else { hover };
                let mut t = cur.unwrap_or(Track { hwnd, flags: 0, hover, px: mx, py: my, since: self.wm.now_ms });
                if fl & 1 != 0 { t.hover = hover; t.since = self.wm.now_ms; }
                t.flags |= fl & 3;
                if t.flags & 3 != 0 { self.wm.extra.user_b.track = Some(t); }
                1
            }
            "NtUserDragDetect" => {
                let (m, x, y) = (self.wm.extra.mouse, a[1] as i32, a[2] as i32);
                ((m.0 - x).abs() > 4 || (m.1 - y).abs() > 4) as u32
            }
            "NtUserDragObject" => 0,
            "NtUserPostThreadMessage" => { self.post_message(0, a[1], a[2], a[3]); 1 }
            "NtUserSendInput" => {
                if a[2] != 28 || a[1] == 0 { return Ok(Some(0)); }
                let mut done = 0;
                for i in 0..a[0].min(512) {
                    let p = a[1] + i * 28;
                    match self.r32(p)? {
                        1 => {
                            let (vk, scan, fl) = (self.mem.r16(p + 4).map_err(|_| "input")? as u32, self.mem.r16(p + 6).map_err(|_| "input")? as u32, self.r32(p + 8)?);
                            let up = fl & 2 != 0;
                            if fl & 4 != 0 {
                                // KEYEVENTF_UNICODE: a character straight to the focus window
                                let t = if self.wm.focus != 0 { self.wm.focus } else { self.wm.active };
                                if !up && t != 0 { self.post_message(t, 0x102, scan, 1); }
                            } else {
                                let shift = self.wm.keys[0x10];
                                let ch = if up || self.wm.keys[0x11] || self.wm.keys[0x12] { 0 } else {
                                    match vk { 0x41..=0x5A => if shift { vk } else { vk + 32 }, 0x30..=0x39 | 0x20 => vk, _ => 0 }
                                };
                                self.key(up as u32, vk, ch);
                            }
                            done += 1;
                        }
                        0 => {
                            let (dx, dy, fl) = (self.r32(p + 4)? as i32, self.r32(p + 8)? as i32, self.r32(p + 16)?);
                            let (cur, scr) = (self.wm.extra.mouse, self.wm.screen);
                            let (x, y) = if fl & 0x8000 != 0 { ((dx as i64 * scr.0 as i64 / 65536) as i32, (dy as i64 * scr.1 as i64 / 65536) as i32) } else { (cur.0 + dx, cur.1 + dy) };
                            if fl & 1 != 0 || fl & 0x8000 != 0 { self.mouse(1, x, y); }
                            if fl & 2 != 0 { self.mouse(0, x, y); }
                            if fl & 4 != 0 { self.mouse(2, x, y); }
                            done += 1;
                        }
                        _ => {}
                    }
                }
                done
            }
            "NtUserSetKeyboardState" => {
                for i in 0..256u32 { self.wm.keys[i as usize] = self.mem.r8(a[0] + i).map_err(|_| "kbstate")? & 0x80 != 0; }
                1
            }
            "NtUserAttachThreadInput" => (a[0] != a[1]) as u32,
            "NtUserCallMsgFilter" => 0, // message-filter hooks are not recorded (SetWindowsHookEx is a stub), so nothing consumes the message
            "NtUserWaitForInputIdle" => 0, // the single desktop session is always idle when asked
            "NtUserSetSystemTimer" => {
                let id = if a[1] == 0 { 0x2000 + self.wm.timers.len() as u32 } else { a[1] };
                self.wm.timers.retain(|t| !(t.0 == a[0] && t.1 == id));
                let ms = a[2].max(10);
                self.wm.timers.push((a[0], id, ms, self.wm.now_ms + ms as u64, 0));
                id
            }

            // ---- colours, atoms, palettes ----
            "NtUserSetSysColors" => {
                for i in 0..a[0].min(64) {
                    let (idx, v) = (self.r32(a[1] + i * 4)?, self.r32(a[2] + i * 4)?);
                    self.wm.extra.user_b.sys_colors.insert(idx, v);
                }
                1
            }
            "NtUserCallOneParam" if matches!(codes::CALL_ONE_PARAM.get(a[1] as usize).copied(), Some("GetSysColor" | "GetSysColorBrush" | "GetSysColorPen")) && self.wm.extra.user_b.sys_colors.contains_key(&a[0]) => {
                let c = self.wm.extra.user_b.sys_colors[&a[0]];
                match codes::CALL_ONE_PARAM[a[1] as usize] {
                    "GetSysColor" => c,
                    "GetSysColorBrush" => self.gdi_alloc(0x100000 | 0x80_0000, Gdi::Brush(colorref(c))),
                    _ => self.gdi_alloc(0x300000 | 0x80_0000, Gdi::Pen(colorref(c), false)),
                }
            }
            "NtUserGetAtomName" => {
                // (atom, UNICODE_STRING*)
                let max = self.mem.r16(a[1] + 2).map_err(|_| "atom")? as usize;
                let buf = self.r32(a[1] + 4)?;
                let atom = a[0] & 0xffff;
                let nm = if atom < 0xC000 { format!("#{atom}") } else if let Some((n, _)) = self.atoms.iter().find(|(_, id)| *id == atom) { n.clone() } else if let Some(c) = self.wm.classes.iter().find(|c| c.atom == atom) { c.name.clone() } else { return Ok(Some(0)) };
                if max < 2 || buf == 0 { return Ok(Some(0)); }
                let u: Vec<u16> = nm.encode_utf16().take(max / 2 - 1).collect();
                for (i, c) in u.iter().enumerate() { self.w16(buf + i as u32 * 2, *c)?; }
                self.w16(buf + u.len() as u32 * 2, 0)?;
                u.len() as u32
            }
            "NtUserSelectPalette" => {
                let def = self.default_palette();
                let old = self.wm.extra.user_b.pal.insert(a[0], a[1]).unwrap_or(def);
                if old == 0 { def } else { old }
            }
            _ => return Ok(None),
        }))
    }
}
