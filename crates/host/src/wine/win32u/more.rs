//! Breadth batch: window geometry/queries, update regions, scrolling, clipboard, hooks, SPI, cursor,
//! keyboard helpers, regions, ellipses/polygons, font/palette stubs. Everything here is single-threaded.
use super::*;

fn rect_isect(a: Rect, b: Rect) -> Rect { [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])] }
fn rect_union(a: Rect, b: Rect) -> Rect { [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])] }
fn empty(r: Rect) -> bool { r[2] <= r[0] || r[3] <= r[1] }
fn region_kind(r: Rect) -> u32 { if empty(r) { 1 } else { 2 } }

#[derive(Default)]
pub struct Extra {
    pub props: BTreeMap<(u32, String), u32>,
    pub scroll: BTreeMap<(u32, u32), [i32; 5]>, // (hwnd, bar) -> min,max,page,pos,track
    pub clip_text: Option<Vec<u8>>,
    pub clip_fmt: u32,
    pub cursor_show: i32,
    pub mouse: (i32, i32),
    pub hook_next: u32,
    pub brush_cache: BTreeMap<u32, u32>,
}

impl WineProc {
    fn rgn_rect(&self, h: u32) -> Option<Rect> { match self.wm.gdi.get(&h) { Some(Gdi::Region(r)) => Some(*r), _ => None } }

    fn wide_key(&self, p: u32) -> Res<String> {
        if p < 0x10000 { return Ok(format!("#{p}")); }
        let mut v = Vec::new(); while v.len() < 256 { let c = self.mem.r16(p + v.len() as u32 * 2).map_err(|_| "prop name")?; if c == 0 { break; } v.push(c); }
        Ok(String::from_utf16_lossy(&v).to_ascii_lowercase())
    }

    pub(super) fn move_window(&mut self, hwnd: u32, x: i32, y: i32, cx: i32, cy: i32, size: bool, mv: bool) -> Res<()> {
        let Some(w) = self.wm.windows.get_mut(&hwnd) else { return Ok(()) };
        let (ow, oh) = (w.rect[2] - w.rect[0], w.rect[3] - w.rect[1]);
        let (nx, ny) = if mv { (x, y) } else { (w.rect[0], w.rect[1]) };
        let (nw, nh) = if size { (cx.max(1), cy.max(1)) } else { (ow, oh) };
        w.rect = [nx, ny, nx + nw, ny + nh];
        let (surface, visible, top) = (w.surface, w.visible, w.surface.is_some());
        if let Some(s) = surface {
            if (nw, nh) != (ow, oh) { self.wm.backend.resize_surface(s, nw as u32, nh as u32); }
            self.wm.backend.place(s, nx, ny, visible);
        }
        if (nw, nh) != (ow, oh) {
            let cr = self.wm.client_rect(hwnd);
            self.send(hwnd, 0x05, 0, ((cr[3] as u32) << 16) | (cr[2] as u32 & 0xffff))?; // WM_SIZE
        }
        if mv { let (px, py) = self.client_origin_screen(hwnd); self.send(hwnd, 0x03, 0, ((py as u32) << 16) | (px as u32 & 0xffff))?; } // WM_MOVE
        if top { self.paint_frame(hwnd); }
        self.invalidate(hwnd, true);
        Ok(())
    }

    fn fill_logfont(&mut self, at: u32, h: i32, bold: bool) -> Res<()> {
        for o in (0..92).step_by(4) { self.w32(at + o, 0)?; }
        self.w32(at, h as u32)?;
        self.w32(at + 16, if bold { 700 } else { 400 })?;
        self.mem.w8(at + 23, 1).map_err(|_| "lf")?; // DEFAULT_CHARSET
        for (i, c) in "Segoe UI".encode_utf16().enumerate() { self.w16(at + 28 + i as u32 * 2, c)?; }
        Ok(())
    }

    fn ellipse(&mut self, hdc: u32, r: Rect) {
        let Some(dc) = self.wm.dcs.get(&hdc) else { return };
        let (brush, pen) = (dc.brush, dc.pen);
        let fill = if matches!(self.wm.gdi.get(&brush), Some(Gdi::NullBrush)) { None } else { Some(self.brush_argb(brush)) };
        let line = self.pen_argb(pen);
        let (cx, cy) = ((r[0] + r[2]) as f32 / 2.0, (r[1] + r[3]) as f32 / 2.0);
        let (rx, ry) = ((r[2] - r[0]) as f32 / 2.0, (r[3] - r[1]) as f32 / 2.0);
        if rx <= 0.0 || ry <= 0.0 { return; }
        for y in r[1]..r[3] {
            let dy = (y as f32 + 0.5 - cy) / ry;
            if dy.abs() > 1.0 { continue; }
            let half = rx * (1.0 - dy * dy).sqrt();
            let (x0, x1) = ((cx - half).round() as i32, (cx + half).round() as i32);
            if let Some(c) = fill { self.dc_fill(hdc, [x0, y, x1, y + 1], c); }
            if let Some(c) = line {
                let edge = y == r[1] || y == r[3] - 1;
                if edge { self.dc_fill(hdc, [x0, y, x1, y + 1], c); } else { self.dc_fill(hdc, [x0, y, x0 + 1, y + 1], c); self.dc_fill(hdc, [x1 - 1, y, x1, y + 1], c); }
            }
        }
    }

    fn polygon(&mut self, hdc: u32, pts: &[(i32, i32)]) {
        let Some(dc) = self.wm.dcs.get(&hdc) else { return };
        let (brush, pen) = (dc.brush, dc.pen);
        if !matches!(self.wm.gdi.get(&brush), Some(Gdi::NullBrush)) && pts.len() >= 3 {
            let c = self.brush_argb(brush);
            let (miny, maxy) = (pts.iter().map(|p| p.1).min().unwrap(), pts.iter().map(|p| p.1).max().unwrap());
            for y in miny..maxy {
                let mut xs: Vec<i32> = Vec::new();
                for i in 0..pts.len() {
                    let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                    if (a.1 <= y && b.1 > y) || (b.1 <= y && a.1 > y) { xs.push(a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1)); }
                }
                xs.sort();
                for pair in xs.chunks(2) { if pair.len() == 2 { self.dc_fill(hdc, [pair[0], y, pair[1], y + 1], c); } }
            }
        }
        for i in 0..pts.len() { let (a, b) = (pts[i], pts[(i + 1) % pts.len()]); self.dc_line(hdc, a, b, pen); }
    }

    /// wineserver requests that user32/comctl32 (PE side) issue directly. Reply fields overlap the
    /// request, so every request field is read before anything is written. Returns false if not ours.
    pub(crate) fn server_window_request(&mut self, name: &str, req: u32) -> Res<bool> {
        match name {
            "get_window_children" => {
                let parent = self.r32(req + 16)?;
                let rdata = self.r32(req + 68)?;
                let cap = self.r32(req + 8)? / 4;
                let list: Vec<u32> = if parent == 0 || parent == DESKTOP { self.wm.zorder.iter().rev().copied().collect() }
                    else { self.wm.windows.iter().rev().filter(|(_, w)| w.parent == parent && w.surface.is_none()).map(|(h, _)| *h).collect() };
                let n = list.len().min(cap as usize);
                for (i, h) in list.iter().take(n).enumerate() { self.w32(rdata + i as u32 * 4, *h)?; }
                self.w32(req + 8, n as u32)?;
                self.w32(req + 4, n as u32 * 4)?;
            }
            "get_window_info" => {
                let h = self.r32(req + 12)?;
                let atom = self.wm.windows.get(&h).map_or(0, |w| self.wm.classes[w.class].atom);
                for (o, v) in [(8u32, h), (12, h), (16, 0x20), (20, 0x24), (24, atom), (28, 1)] { self.w32(req + o, v)?; }
                self.w32(req + 4, 0)?;
            }
            "is_window_hung" | "get_window_properties" | "enum_winstation" | "enum_desktop" | "send_message" => { for o in [8u32, 12] { self.w32(req + o, 0)?; } self.w32(req + 4, 0)?; }
            "get_last_input_time" => { let t = self.wm_now_ms() as u32; self.w32(req + 8, t)?; self.w32(req + 4, 0)?; }
            _ => return Ok(false),
        }
        self.w32(req, 0)?; // error = STATUS_SUCCESS
        Ok(true)
    }

    pub(super) fn more_call(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        Ok(Some(match name {
            // ---- windows: geometry, hierarchy, queries ----
            "NtUserMoveWindow" => { self.move_window(a[0], a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32, true, true)?; 1 }
            "NtUserSetWindowPos" => {
                let f = a[6];
                self.move_window(a[0], a[2] as i32, a[3] as i32, a[4] as i32, a[5] as i32, f & 1 == 0, f & 2 == 0)?;
                if f & 4 == 0 && (a[1] == 0 || a[1] == u32::MAX) {
                    if let Some(s) = self.wm.windows.get(&a[0]).and_then(|w| w.surface) { self.wm.backend.raise(s); self.wm.zorder.retain(|z| *z != a[0]); self.wm.zorder.push(a[0]); }
                }
                if f & 0x40 != 0 { self.show_window(a[0], true)?; }
                if f & 0x80 != 0 { self.show_window(a[0], false)?; }
                1
            }
            "NtUserRedrawWindow" => {
                let f = a[3];
                if f & 1 != 0 { self.invalidate(a[0], f & 4 != 0); }
                if f & 8 != 0 { if let Some(w) = self.wm.windows.get_mut(&a[0]) { w.invalid = None; } }
                if f & 0x100 != 0 && self.wm.windows.get(&a[0]).is_some_and(|w| w.invalid.is_some()) { self.send(a[0], WM_PAINT, 0, 0)?; }
                1
            }
            "NtUserGetUpdateRect" => {
                let inv = self.wm.windows.get(&a[0]).and_then(|w| w.invalid);
                if a[1] != 0 { self.put_rect(a[1], inv.unwrap_or([0; 4]))?; }
                inv.is_some() as u32
            }
            "NtUserGetUpdateRgn" => {
                let inv = self.wm.windows.get(&a[0]).and_then(|w| w.invalid);
                self.wm.gdi.insert(a[1], Gdi::Region(inv.unwrap_or([0; 4])));
                if inv.is_some() { 2 } else { 1 }
            }
            "NtUserValidateRect" | "NtUserValidateRgn" => { if let Some(w) = self.wm.windows.get_mut(&a[0]) { w.invalid = None; } 1 }
            "NtUserUpdateWindow" => { if self.wm.windows.get(&a[0]).is_some_and(|w| w.invalid.is_some()) { self.send(a[0], WM_PAINT, 0, 0)?; } 1 }
            "NtUserSetActiveWindow" => {
                let old = std::mem::replace(&mut self.wm.active, a[0]);
                if old != a[0] { self.repaint_frame(old); self.repaint_frame(a[0]); }
                old
            }
            "NtUserGetAncestor" => {
                let parent = self.wm.windows.get(&a[0]).map_or(0, |w| w.parent);
                match a[1] { 1 => if parent == DESKTOP { 0 } else { parent }, _ => self.toplevel_of(a[0]) }
            }
            "NtUserWindowFromPoint" | "NtUserChildWindowFromPointEx" => {
                let (x, y) = if name == "NtUserWindowFromPoint" { (a[0] as i32, a[1] as i32) } else { (a[1] as i32, a[2] as i32) };
                self.window_at(x, y).map_or(DESKTOP, |h| self.hit_child(h, x, y))
            }
            "NtUserGetClassName" => {
                let Some(w) = self.wm.windows.get(&a[0]) else { return Ok(Some(0)) };
                let cn = self.wm.classes[w.class].name.clone();
                let (max, buf) = (self.mem.r16(a[2] + 2).unwrap_or(0) as u32 / 2, self.r32(a[2] + 4)?);
                let u: Vec<u16> = cn.encode_utf16().take(max.saturating_sub(1) as usize).collect();
                for (i, c) in u.iter().enumerate() { self.w16(buf + i as u32 * 2, *c)?; }
                self.w16(buf + u.len() as u32 * 2, 0)?;
                self.mem.w16(a[2], u.len() as u16 * 2).ok();
                u.len() as u32
            }
            "NtUserFindWindowEx" => {
                let cls = if a[2] != 0 { match self.class_arg(a[2])? { Err(s) => Some(s.to_ascii_lowercase()), Ok(_) => None } } else { None };
                let title = if a[3] != 0 { match self.class_arg(a[3])? { Err(s) => Some(s.to_ascii_lowercase()), Ok(_) => None } } else { None };
                let hit = self.wm.windows.iter().find(|(h, w)| **h != DESKTOP && w.parent == if a[0] == 0 { DESKTOP } else { a[0] }
                    && cls.as_ref().is_none_or(|c| self.wm.classes[w.class].name.eq_ignore_ascii_case(c))
                    && title.as_ref().is_none_or(|t| w.text.eq_ignore_ascii_case(t))).map(|(h, _)| *h);
                hit.unwrap_or(0)
            }
            "NtUserBuildHwndList" => {
                let list: Vec<u32> = self.wm.zorder.iter().rev().copied().collect();
                let n = list.len().min(a[5] as usize);
                for (i, h) in list.iter().take(n).enumerate() { self.w32(a[6] + i as u32 * 4, *h)?; }
                self.w32(a[7], n as u32)?;
                0
            }
            "NtUserSetParent" => { let old = self.wm.windows.get(&a[0]).map_or(0, |w| w.parent); if let Some(w) = self.wm.windows.get_mut(&a[0]) { w.parent = if a[1] == 0 { DESKTOP } else { a[1] }; } old }
            "NtUserGetWindowPlacement" => {
                let Some(w) = self.wm.windows.get(&a[0]) else { return Ok(Some(0)) };
                let r = w.rect;
                for (i, v) in [44u32, 0, 1, 0, 0, u32::MAX, u32::MAX, r[0] as u32, r[1] as u32, r[2] as u32, r[3] as u32].iter().enumerate() { self.w32(a[1] + i as u32 * 4, *v)?; }
                1
            }
            "NtUserSetWindowPlacement" => { let r = self.rect_at(a[1] + 28)?; self.move_window(a[0], r[0], r[1], r[2] - r[0], r[3] - r[1], true, true)?; 1 }
            "NtUserSetProp" => { let k = self.wide_key(a[1])?; self.wm.extra.props.insert((a[0], k), a[2]); 1 }
            "NtUserGetProp" => { let k = self.wide_key(a[1])?; self.wm.extra.props.get(&(a[0], k)).copied().unwrap_or(0) }
            "NtUserRemoveProp" => { let k = self.wide_key(a[1])?; self.wm.extra.props.remove(&(a[0], k)).unwrap_or(0) }
            "NtUserGetWindowDC" => self.new_dc(a[0], None),
            "NtUserScrollWindowEx" => { self.invalidate(a[0], true); 2 }
            "NtUserDrawCaptionTemp" | "NtUserEnableScrollBar" | "NtUserShowScrollBar" | "NtUserClipCursor" | "NtUserUnhookWindowsHookEx" | "NtUserSetCaretBlinkTime" => 1,
            "NtUserGetObjectInformation" | "NtUserCallNextHookEx" => 0,
            "NtUserSetWindowsHookEx" => { self.wm.extra.hook_next += 4; 0x50000 + self.wm.extra.hook_next }
            "NtUserSetScrollInfo" => {
                let (mask, min, max, page, pos) = (self.r32(a[2] + 4)?, self.r32(a[2] + 8)? as i32, self.r32(a[2] + 12)? as i32, self.r32(a[2] + 16)?, self.r32(a[2] + 20)? as i32);
                let e = self.wm.extra.scroll.entry((a[0], a[1])).or_insert([0, 100, 0, 0, 0]);
                if mask & 1 != 0 { e[0] = min; e[1] = max; }
                if mask & 2 != 0 { e[2] = page as i32; }
                if mask & 4 != 0 { e[3] = pos.clamp(e[0], (e[1] - e[2].max(1) + 1).max(e[0])); }
                e[3] as u32
            }
            // ---- cursor / input helpers ----
            "NtUserShowCursor" => { self.wm.extra.cursor_show += if a[0] != 0 { 1 } else { -1 }; self.wm.extra.cursor_show as u32 }
            "NtUserSetCursorPos" => { self.wm.extra.mouse = (a[0] as i32, a[1] as i32); 1 }
            "NtUserGetCursorInfo" => {
                let (x, y) = self.wm.extra.mouse;
                for (i, v) in [20u32, 1, 0x30001, x as u32, y as u32].iter().enumerate() { self.w32(a[0] + i as u32 * 4, *v)?; }
                1
            }
            "NtUserGetKeyNameText" => { if a[2] > 0 { self.w16(a[1], 0)?; } 0 }
            "NtUserToUnicodeEx" => 0,
            "NtUserMapVirtualKeyEx" => match a[1] { 2 => if (0x30..=0x5A).contains(&a[0]) || a[0] == 0x20 { a[0] } else { 0 }, _ => 0 },
            "NtUserVkKeyScanEx" => {
                let c = a[0] & 0xffff;
                match c { 0x61..=0x7A => c - 32, 0x41..=0x5A => (c | 0x100) & !0, 0x30..=0x39 | 0x20 => c, 0x0D => 13, _ => 0xFFFF }
            }
            "NtUserGetQueueStatus" => if self.wm.queue.is_empty() { 0 } else { (8 << 16) | 8 },
            "NtUserWaitMessage" => {
                if self.next_message(false).is_none() { self.idle = true; self.parked = true; self.took_over = true; return Ok(Some(0)); }
                1
            }
            "NtUserMsgWaitForMultipleObjectsEx" => {
                // (count, handles*, timeout, mask, flags): a queued message wins, then signaled handles, else yield
                if !self.wm.queue.is_empty() || self.wm.windows.values().any(|w| w.visible && w.invalid.is_some()) { return Ok(Some(a[0])); }
                for i in 0..a[0] { let h = self.r32(a[1] + i * 4)?; if matches!(self.handles.get(&h), Some(crate::wine::Obj::Sync { signaled: true, .. })) { return Ok(Some(i)); } }
                self.idle = true;
                0x102
            }
            "NtUserOpenClipboard" | "NtUserCloseClipboard" => 1,
            "NtUserEmptyClipboard" => { self.wm.extra.clip_text = None; 1 }
            "NtUserIsClipboardFormatAvailable" => (self.wm.extra.clip_text.is_some() && self.wm.extra.clip_fmt == a[0]) as u32,
            "NtUserSetClipboardData" => {
                // (format, handle, set_clipboard_params*{data, size, cache_only, seqno})
                let (data, size) = (self.r32(a[2])?, self.r32(a[2] + 4)?);
                if data != 0 { self.wm.extra.clip_text = Some(self.mem.read_bytes(data, size).map_err(|_| "clip")?.to_vec()); self.wm.extra.clip_fmt = a[0]; }
                1
            }
            "NtUserGetClipboardData" => {
                let (data, size) = (self.r32(a[1])?, self.r32(a[1] + 4)?);
                match (&self.wm.extra.clip_text, self.wm.extra.clip_fmt == a[0]) {
                    (Some(t), true) => {
                        let t = t.clone();
                        if data != 0 && size >= t.len() as u32 { self.mem.write_bytes(data, &t).map_err(|_| "clip")?; }
                        self.w32(a[1] + 8, t.len() as u32)?;
                        1
                    }
                    _ => 0,
                }
            }
            // ---- NtUserCall* families ----
            "NtUserCallNoParam" if matches!(codes::CALL_NO_PARAM.get(a[0] as usize).copied(), Some("GetDialogBaseUnits" | "GetInputState" | "GetProcessDefaultLayout" | "GetProgmanWindow" | "GetShellWindow" | "GetTaskmanWindow" | "DestroyCaret")) => {
                match codes::CALL_NO_PARAM[a[0] as usize] { "GetDialogBaseUnits" => 0x000F_0007, "DestroyCaret" => 1, _ => 0 }
            }
            "NtGdiRectVisible" | "NtGdiPtVisible" => 1,
            "NtUserCallOneParam" if matches!(codes::CALL_ONE_PARAM.get(a[1] as usize).copied(), Some("GetCursorPos" | "GetClipCursor" | "GetPrimaryMonitorRect" | "GetVirtualScreenRect" | "MessageBeep" | "RealizePalette" | "ReplyMessage" | "SetKeyboardAutoRepeat" | "EnumClipboardFormats" | "EnableDC" | "EnableThunkLock" | "BeginDeferWindowPos" | "SetProcessDefaultLayout" | "IsWindowRectFullScreen" | "GetIconParam")) => {
                match codes::CALL_ONE_PARAM[a[1] as usize] {
                    "GetCursorPos" => { let (x, y) = self.wm.extra.mouse; self.w32(a[0], x as u32)?; self.w32(a[0] + 4, y as u32)?; 1 }
                    "GetClipCursor" | "GetPrimaryMonitorRect" | "GetVirtualScreenRect" => { self.put_rect(a[0], [0, 0, 1024, 768])?; 1 }
                    "IsWindowRectFullScreen" => 0,
                    _ => 1,
                }
            }
            "NtUserCallTwoParam" if matches!(codes::CALL_TWO_PARAM.get(a[2] as usize).copied(), Some("NtUserAllocWinProc" | "GetMonitorInfo" | "GetSystemMetricsForDpi" | "MonitorFromRect" | "UnhookWindowsHook" | "SetIconParam" | "GetDialogProc")) => {
                match codes::CALL_TWO_PARAM[a[2] as usize] {
                    "GetMonitorInfo" => { // MONITORINFO: cbSize, rcMonitor, rcWork, dwFlags
                        let sz = self.r32(a[1])?;
                        self.put_rect(a[1] + 4, [0, 0, 1024, 768])?; self.put_rect(a[1] + 20, [0, 0, 1024, 768])?; self.w32(a[1] + 36, 1)?;
                        let _ = sz; 1
                    }
                    "GetSystemMetricsForDpi" => system_metric(a[0]),
                    "MonitorFromRect" => 0x20001,
                    "NtUserAllocWinProc" | "GetDialogProc" => a[0], // window procedure "handle" = the function pointer itself
                    _ => 1,
                }
            }
            "NtUserCallHwnd" if matches!(codes::CALL_HWND.get(a[1] as usize).copied(), Some("GetWindowTextLength" | "SetForegroundWindow" | "GetDialogInfo" | "GetMDIClientInfo" | "NtUserGetFullWindowHandle" | "NtUserIsCurrentProcessWindow" | "NtUserIsCurrentThreadWindow")) => {
                match codes::CALL_HWND[a[1] as usize] {
                    "NtUserGetFullWindowHandle" | "NtUserIsCurrentProcessWindow" | "NtUserIsCurrentThreadWindow" => if self.wm.windows.contains_key(&a[0]) { a[0] } else { 0 },
                    "GetDialogInfo" => self.wm.extra.props.get(&(a[0], "#dlginfo".into())).copied().unwrap_or(0),
                    "GetMDIClientInfo" => self.wm.extra.props.get(&(a[0], "#mdiinfo".into())).copied().unwrap_or(0),
                    "GetWindowTextLength" => self.wm.windows.get(&a[0]).map_or(0, |w| w.text.encode_utf16().count() as u32),
                    _ => { let old = std::mem::replace(&mut self.wm.active, a[0]); if old != a[0] { self.repaint_frame(old); self.repaint_frame(a[0]); } 1 }
                }
            }
            "NtUserCallHwndParam" if matches!(codes::CALL_HWND_PARAM.get(a[2] as usize).copied(), Some("SetDialogInfo" | "SetMDIClientInfo" | "ClientToScreen" | "ScreenToClient" | "MapWindowPoints" | "GetScrollInfo" | "EnableWindow" | "GetClassLongA" | "GetClassLongW" | "GetClassLongPtrA" | "GetClassLongPtrW" | "IsChild" | "GetWindowThread" | "GetWindowWord" | "GetWindowRelative" | "GetWindowInfo" | "MonitorFromWindow" | "SetWindowContextHelpId" | "ShowOwnedPopups" | "GetChildRect" | "KillSystemTimer" | "NtUserSetWindowStyle")) => {
                match codes::CALL_HWND_PARAM[a[2] as usize] {
                    "ClientToScreen" | "ScreenToClient" => {
                        let (ox, oy) = self.client_origin_screen(a[0]);
                        let sign = if codes::CALL_HWND_PARAM[a[2] as usize] == "ClientToScreen" { 1 } else { -1 };
                        let (x, y) = (self.r32(a[1])? as i32, self.r32(a[1] + 4)? as i32);
                        self.w32(a[1], (x + sign * ox) as u32)?; self.w32(a[1] + 4, (y + sign * oy) as u32)?;
                        1
                    }
                    "MapWindowPoints" => { // param -> struct { hwnd_to, POINT* pts, UINT count }
                        let (to, pts, n) = (self.r32(a[1])?, self.r32(a[1] + 4)?, self.r32(a[1] + 8)?);
                        let (fx, fy) = if a[0] == 0 { (0, 0) } else { self.client_origin_screen(a[0]) };
                        let (tx, ty) = if to == 0 { (0, 0) } else { self.client_origin_screen(to) };
                        for i in 0..n { let (x, y) = (self.r32(pts + i * 8)? as i32, self.r32(pts + i * 8 + 4)? as i32); self.w32(pts + i * 8, (x + fx - tx) as u32)?; self.w32(pts + i * 8 + 4, (y + fy - ty) as u32)?; }
                        (((fy - ty) as u32) << 16) | ((fx - tx) as u32 & 0xffff)
                    }
                    "GetScrollInfo" => { // param -> { INT bar; SCROLLINFO *info }
                        let (bar, info) = (self.r32(a[1])?, self.r32(a[1] + 4)?);
                        let e = self.wm.extra.scroll.get(&(a[0], bar)).copied().unwrap_or([0, 100, 0, 0, 0]);
                        let mask = self.r32(info + 4)?;
                        if mask & 1 != 0 { self.w32(info + 8, e[0] as u32)?; self.w32(info + 12, e[1] as u32)?; }
                        if mask & 2 != 0 { self.w32(info + 16, e[2] as u32)?; }
                        if mask & 4 != 0 { self.w32(info + 20, e[3] as u32)?; }
                        if mask & 8 != 0 { self.w32(info + 24, e[4] as u32)?; }
                        1
                    }
                    "SetDialogInfo" => { self.wm.extra.props.insert((a[0], "#dlginfo".into()), a[1]); 1 }
                    "SetMDIClientInfo" => { self.wm.extra.props.insert((a[0], "#mdiinfo".into()), a[1]); 1 }
                    "EnableWindow" => 0,
                    "IsChild" => self.wm.windows.get(&a[1]).is_some_and(|w| w.parent == a[0]) as u32,
                    "GetWindowThread" => 0x24,
                    "GetWindowRelative" => match a[1] { 0 => self.wm.zorder.last().copied().unwrap_or(0), 5 | 2 => 0, _ => 0 },
                    "MonitorFromWindow" => 0x20001,
                    "GetClassLongA" | "GetClassLongW" | "GetClassLongPtrA" | "GetClassLongPtrW" | "GetWindowWord" => 0,
                    "GetWindowInfo" => {
                        let Some(w) = self.wm.windows.get(&a[0]) else { return Ok(Some(0)) };
                        let (r, st, ex) = (w.rect, w.style, w.ex_style);
                        let cr = self.wm.client_rect(a[0]);
                        let (ox, oy) = self.client_origin_screen(a[0]);
                        self.w32(a[1], 60)?; self.put_rect(a[1] + 4, r)?; self.put_rect(a[1] + 20, [ox, oy, ox + cr[2], oy + cr[3]])?;
                        self.w32(a[1] + 36, st)?; self.w32(a[1] + 40, ex)?; self.w32(a[1] + 44, (self.wm.active == a[0]) as u32)?;
                        1
                    }
                    _ => 1,
                }
            }
            // ---- regions ----
            "NtGdiSetRectRgn" => { self.wm.gdi.insert(a[0], Gdi::Region([a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32])); 1 }
            "NtGdiCreateRoundRectRgn" => self.gdi_alloc(0x040000, Gdi::Region([a[0] as i32, a[1] as i32, a[2] as i32, a[3] as i32])),
            "NtGdiExtCreateRegion" => {
                let n = self.r32(a[2] + 8)?;
                let mut b: Option<Rect> = None;
                for i in 0..n { let r = self.rect_at(a[2] + 32 + i * 16)?; b = Some(b.map_or(r, |x| rect_union(x, r))); }
                self.gdi_alloc(0x040000, Gdi::Region(b.unwrap_or([0; 4])))
            }
            "NtGdiCombineRgn" => {
                let (d, s1, s2, mode) = (a[0], self.rgn_rect(a[1]).unwrap_or([0; 4]), self.rgn_rect(a[2]).unwrap_or([0; 4]), a[3]);
                let r = match mode { 1 => rect_isect(s1, s2), 2 | 3 => if empty(s1) { s2 } else if empty(s2) { s1 } else { rect_union(s1, s2) }, 4 => s1, _ => s1 };
                self.wm.gdi.insert(d, Gdi::Region(r));
                region_kind(r)
            }
            "NtGdiPtInRegion" => self.rgn_rect(a[0]).is_some_and(|r| (a[1] as i32) >= r[0] && (a[1] as i32) < r[2] && (a[2] as i32) >= r[1] && (a[2] as i32) < r[3]) as u32,
            "NtGdiRectInRegion" => { let q = self.rect_at(a[1])?; self.rgn_rect(a[0]).is_some_and(|r| !empty(rect_isect(r, q))) as u32 }
            "NtGdiOffsetRgn" => { if let Some(r) = self.rgn_rect(a[0]) { let n = [r[0] + a[1] as i32, r[1] + a[2] as i32, r[2] + a[1] as i32, r[3] + a[2] as i32]; self.wm.gdi.insert(a[0], Gdi::Region(n)); region_kind(n) } else { 0 } }
            "NtGdiGetRgnBox" => { let r = self.rgn_rect(a[0]).unwrap_or([0; 4]); self.put_rect(a[1], r)?; region_kind(r) }
            "NtGdiEqualRgn" => (self.rgn_rect(a[0]) == self.rgn_rect(a[1])) as u32,
            "NtGdiFillRgn" => { if let Some(r) = self.rgn_rect(a[1]) { let c = self.brush_argb(a[2]); self.dc_fill(a[0], r, c); } 1 }
            "NtGdiOffsetClipRgn" | "NtGdiExcludeClipRect" => 2,
            "NtGdiExtSelectClipRgn" => {
                let (hdc, hrgn, mode) = (a[0], a[1], a[2]);
                let r = if hrgn == 0 { None } else { self.rgn_rect(hrgn) };
                if let Some(dc) = self.wm.dcs.get_mut(&hdc) {
                    dc.clip = match (mode, r, dc.clip) { (5, r, _) => r, (1, Some(r), Some(c)) => Some(rect_isect(r, c)), (1, Some(r), None) => Some(r), (_, _, c) => c };
                }
                r.map_or(2, region_kind)
            }
            // ---- shapes ----
            "NtGdiEllipse" => { let r = self.lp_rect(a[0], [a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32]); self.ellipse(a[0], r); 1 }
            "NtGdiRoundRect" => { let r = self.lp_rect(a[0], [a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32]); self.ellipse(a[0], r); 1 }
            "NtGdiArcInternal" => { let r = self.lp_rect(a[1], [a[2] as i32, a[3] as i32, a[4] as i32, a[5] as i32]); self.ellipse(a[1], r); 1 }
            "NtGdiPolyPolyDraw" => {
                // (hdc, points*, counts*, count, function): 1 PolyPolygon, 2 PolyPolyline, 3 PolyBezier, 4 PolyBezierTo, 5 PolylineTo
                let (hdc, pts, counts, n, func) = (a[0], a[1], a[2], a[3], a[4]);
                let mut off = 0u32;
                let pen = self.wm.dcs.get(&hdc).map_or(0, |d| d.pen);
                for i in 0..n {
                    let cnt = if func >= 3 { n } else { self.r32(counts + i * 4)? };
                    let mut v = Vec::new();
                    for k in 0..cnt { let (x, y) = (self.r32(pts + (off + k) * 8)? as i32, self.r32(pts + (off + k) * 8 + 4)? as i32); v.push(self.lp(hdc, x, y)); }
                    match func {
                        1 => self.polygon(hdc, &v),
                        5 => { let start = self.wm.dcs.get(&hdc).map_or((0, 0), |d| d.pos); let mut p = start; for q in &v { self.dc_line(hdc, p, *q, pen); p = *q; } if let Some(dc) = self.wm.dcs.get_mut(&hdc) { dc.pos = p; } }
                        _ => for w in v.windows(2) { self.dc_line(hdc, w[0], w[1], pen); },
                    }
                    off += cnt;
                    if func >= 3 { break; }
                }
                1
            }
            // ---- fonts / palettes ----
            "NtGdiGetGlyphIndicesW" => { for i in 0..a[2] { let c = self.mem.r16(a[1] + i * 2).unwrap_or(0); self.w16(a[3] + i * 2, c)?; } a[2] }
            "NtGdiGetCharABCWidthsW" => {
                let (px, bold) = self.dc_font(a[0]);
                for i in 0..a[2].saturating_sub(a[1]) + 1 {
                    let w = plonk_fonts::with(|f| f.advance(char::from_u32(a[1] + i).unwrap_or('?'), px, bold)).round() as u32;
                    self.w32(a[5] + i * 12, 0)?; self.w32(a[5] + i * 12 + 4, w)?; self.w32(a[5] + i * 12 + 8, 0)?;
                }
                1
            }
            "NtGdiGetTextCharsetInfo" => { if a[1] != 0 { for o in (0..24).step_by(4) { self.w32(a[1] + o, 0)?; } } 1 }
            "NtGdiEnumFonts" => { if a[6] != 0 { self.w32(a[6], 0)?; } 1 }
            "NtGdiGetCharWidthInfo" => { for o in (0..12).step_by(4) { self.w32(a[1] + o, 0)?; } 1 }
            "NtGdiGetOutlineTextMetricsInternalW" | "NtGdiGetKerningPairs" | "NtGdiGetFontUnicodeRanges" | "NtGdiGetCharSet" => 0,
            "NtGdiGetFontData" => u32::MAX, // GDI_ERROR: no raw font tables
            "NtGdiCreatePaletteInternal" | "NtGdiCreateHalftonePalette" => self.gdi_alloc(0x080000, Gdi::Other),
            "NtGdiSelectPalette" | "NtGdiRealizePalette" | "NtGdiGetNearestPaletteIndex" | "NtGdiSetLayout" => 0,
            "NtGdiGetSystemPaletteUse" => 1,
            "NtGdiSetBrushOrg" => { if a[3] != 0 { self.w32(a[3], 0)?; self.w32(a[3] + 4, 0)?; } 1 }
            // ---- SystemParametersInfo ----
            "NtUserSystemParametersInfo" => {
                let (action, val, ptr) = (a[0], a[1], a[2]);
                match action {
                    0x29 => { // SPI_GETNONCLIENTMETRICS (NONCLIENTMETRICSW)
                        let sz = self.r32(ptr)?;
                        for o in (4..sz).step_by(4) { self.w32(ptr + o, 0)?; }
                        for (o, v) in [(4u32, 1u32), (8, 16), (12, 16), (16, 28), (20, 28)] { self.w32(ptr + o, v)?; }
                        self.fill_logfont(ptr + 24, -12, true)?;
                        for (o, v) in [(116u32, 28u32), (120, 24)] { self.w32(ptr + o, v)?; }
                        self.fill_logfont(ptr + 124, -11, false)?;
                        for (o, v) in [(216u32, 24u32), (220, 24)] { self.w32(ptr + o, v)?; }
                        for o in [224u32, 316, 408] { self.fill_logfont(ptr + o, -12, false)?; }
                        1
                    }
                    0x1F => { self.fill_logfont(ptr, -12, false)?; 1 } // SPI_GETICONTITLELOGFONT
                    0x30 => { self.put_rect(ptr, [0, 0, 1024, 768])?; 1 } // SPI_GETWORKAREA
                    0x68 => { self.w32(ptr, 3)?; 1 } // wheel scroll lines
                    0x26 | 0x4A | 0x1012 | 0x1042 => { if ptr != 0 { self.w32(ptr, 1)?; } 1 } // drag full windows, font smoothing, ...
                    0x4B | 0x48 => { if ptr != 0 { self.w32(ptr, 0)?; } 1 }
                    0x31 => 1,
                    _ => { let _ = val; 1 }
                }
            }
            _ => return Ok(None),
        }))
    }
}
