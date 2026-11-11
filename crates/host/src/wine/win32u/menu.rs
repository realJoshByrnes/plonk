//! Menus: menu objects, window menu bars, glass popup menus (one level), TrackPopupMenu.
use super::*;

pub(super) const MENU_H: i32 = 24;
const ROW_H: i32 = 26;
const SEP_H: i32 = 9;
const MF_GRAYED: u32 = 1;
const MF_DISABLED: u32 = 2;
const MF_POPUP: u32 = 0x10;
const MF_SEPARATOR: u32 = 0x800;
const MF_BYPOSITION: u32 = 0x400;

#[derive(Clone)]
pub(super) struct Item {
    id: u32,
    flags: u32,
    text: String,
    sub: u32,
}

#[derive(Clone, Default)]
pub(super) struct Menu {
    items: Vec<Item>,
}

pub(super) struct OpenMenu {
    owner: u32,
    menu: u32,
    surface: SurfaceId,
    rect: Rect, // screen
    hover: Option<usize>,
}

impl Item {
    fn clone_basic(&self) -> Item { Item { id: self.id, flags: self.flags, text: self.text.clone(), sub: self.sub } }

    fn enabled(&self) -> bool { self.flags & (MF_GRAYED | MF_DISABLED | MF_SEPARATOR) == 0 }
}

fn text_w(s: &str) -> i32 { plonk_fonts::with(|f| f.text_width(s, 12.0, false)).ceil() as i32 }

impl WineProc {
    fn menu_find(&self, menu: u32, id: u32, flags: u32) -> Option<usize> {
        let m = self.wm.menus.get(&menu)?;
        if flags & MF_BYPOSITION != 0 { return if (id as usize) < m.items.len() { Some(id as usize) } else { None }; }
        m.items.iter().position(|i| i.id == id)
    }

    pub(super) fn menu_count(&self, menu: u32) -> u32 { self.wm.menus.get(&menu).map_or(u32::MAX, |m| m.items.len() as u32) }

    /// Menu bar item rectangles in window-local coordinates.
    fn bar_items(&self, menu: u32) -> Vec<(i32, i32)> {
        let Some(m) = self.wm.menus.get(&menu) else { return Vec::new() };
        let mut x = BORDER + 4;
        m.items.iter().map(|i| { let w = text_w(&i.text) + 20; let r = (x, x + w); x += w; r }).collect()
    }

    pub(super) fn paint_menu_bar(&mut self, hwnd: u32) {
        let Some(w) = self.wm.windows.get(&hwnd) else { return };
        let (Some(s), true) = (w.surface, w.menu != 0) else { return };
        let (ww, menu) = (w.rect[2] - w.rect[0], w.menu);
        let y0 = BORDER + CAPTION;
        let items: Vec<(String, bool)> = self.wm.menus.get(&menu).map(|m| m.items.iter().map(|i| (i.text.clone(), i.flags & MF_SEPARATOR != 0)).collect()).unwrap_or_default();
        let spans = self.bar_items(menu);
        let open_idx = self.wm.open.as_ref().filter(|o| o.owner == hwnd).and_then(|o| self.wm.menus.get(&menu).and_then(|m| m.items.iter().position(|i| i.sub == o.menu)));
        let b = &mut self.wm.backend;
        b.fill_rect(s, BORDER, y0, ww - 2 * BORDER, MENU_H, 0xFFF7_F8FA);
        for (i, ((text, _), (x0, x1))) in items.iter().zip(spans.iter()).enumerate() {
            if open_idx == Some(i) { b.fill_rect(s, *x0, y0 + 2, x1 - x0, MENU_H - 4, 0xFFD6_E6F8); }
            b.draw_text_px(s, x0 + 10, y0 + 5, text, 0xFF1A_1A1A, 12.0, false);
        }
        b.fill_rect(s, BORDER, y0 + MENU_H - 1, ww - 2 * BORDER, 1, 0x1F00_0000);
    }

    fn popup_layout(&self, menu: u32) -> (i32, Vec<(i32, i32)>) {
        let Some(m) = self.wm.menus.get(&menu) else { return (0, Vec::new()) };
        let w = m.items.iter().map(|i| text_w(&i.text)).max().unwrap_or(40) + 56;
        let mut y = 4;
        let rows = m.items.iter().map(|i| { let h = if i.flags & MF_SEPARATOR != 0 { SEP_H } else { ROW_H }; let r = (y, y + h); y += h; r }).collect();
        (w.max(120), rows)
    }

    fn paint_popup(&mut self) {
        let Some(o) = self.wm.open.as_ref() else { return };
        let (s, menu, hover) = (o.surface, o.menu, o.hover);
        let Some(m) = self.wm.menus.get(&menu).cloned() else { return };
        let (w, rows) = self.popup_layout(menu);
        let h = rows.last().map_or(8, |r| r.1) + 4;
        let b = &mut self.wm.backend;
        b.fill_rect(s, 0, 0, w, h, 0xB8FF_FFFF);
        b.fill_rect(s, 0, 0, w, 1, 0xF2FF_FFFF);
        for (i, (item, (y0, y1))) in m.items.iter().zip(rows.iter()).enumerate() {
            if item.flags & MF_SEPARATOR != 0 { b.fill_rect(s, 10, y0 + SEP_H / 2, w - 20, 1, 0x2800_0000); continue; }
            if hover == Some(i) && item.enabled() { b.fill_rect(s, 4, *y0, w - 8, y1 - y0, 0xE00A_64C8); }
            let col = if item.enabled() { if hover == Some(i) { 0xFFFF_FFFF } else { 0xFF1A_1A1A } } else { 0xFF8A_8F96 };
            b.draw_text_px(s, 28, y0 + 6, &item.text, col, 12.0, false);
            if item.sub != 0 { b.draw_text_px(s, w - 18, y0 + 6, ">", col, 12.0, false); }
        }
    }

    pub(super) fn open_popup(&mut self, owner: u32, menu: u32, x: i32, y: i32) -> Res<()> {
        self.close_popup();
        if !self.wm.menus.contains_key(&menu) { return Ok(()); }
        let _ = self.send(owner, 0x117, menu, 0); // WM_INITMENUPOPUP
        let (w, rows) = self.popup_layout(menu);
        let h = rows.last().map_or(8, |r| r.1) + 4;
        let sid = self.wm.backend.create_surface(w as u32, h as u32);
        self.wm.backend.set_glass(sid, 8, 24, 20);
        self.wm.backend.place(sid, x, y, true);
        self.wm.backend.raise(sid);
        self.wm.open = Some(OpenMenu { owner, menu, surface: sid, rect: [x, y, x + w, y + h], hover: None });
        self.paint_popup();
        if owner != 0 { self.paint_frame(owner); self.paint_menu_bar(owner); }
        Ok(())
    }

    pub(super) fn close_popup(&mut self) {
        if let Some(o) = self.wm.open.take() {
            self.wm.backend.destroy_surface(o.surface);
            if o.owner != 0 { self.paint_frame(o.owner); self.paint_menu_bar(o.owner); self.invalidate(o.owner, true); }
        }
    }

    /// Shell mouse handling while a popup is open. Returns true if the event was consumed.
    pub(super) fn menu_mouse(&mut self, kind: u32, x: i32, y: i32) -> bool {
        let Some(o) = self.wm.open.as_ref() else { return false };
        let (rect, menu, owner) = (o.rect, o.menu, o.owner);
        let inside = x >= rect[0] && x < rect[2] && y >= rect[1] && y < rect[3];
        let (_, rows) = self.popup_layout(menu);
        let idx = if inside { rows.iter().position(|r| y - rect[1] >= r.0 && y - rect[1] < r.1) } else { None };
        match kind {
            1 => {
                if let Some(o) = self.wm.open.as_mut() { if o.hover != idx { o.hover = idx; self.paint_popup(); } }
                true
            }
            0 => {
                if !inside { self.close_popup(); self.wm.track_done = Some(0); return false; }
                if let Some(i) = idx {
                    let item = self.wm.menus[&menu].items[i].clone();
                    if item.enabled() && item.sub == 0 {
                        self.close_popup();
                        self.post_message(owner, 0x111, item.id & 0xffff, 0); // WM_COMMAND
                        self.wm.track_done = Some(item.id);
                    }
                }
                true
            }
            _ => true,
        }
    }

    /// Click on a window's menu bar: open the item's popup below it.
    pub(super) fn menu_bar_click(&mut self, hwnd: u32, lx: i32, ly: i32) -> bool {
        let Some(w) = self.wm.windows.get(&hwnd) else { return false };
        let (menu, wx, wy) = (w.menu, w.rect[0], w.rect[1]);
        let y0 = BORDER + CAPTION;
        if menu == 0 || ly < y0 || ly >= y0 + MENU_H { return false; }
        let spans = self.bar_items(menu);
        if let Some(i) = spans.iter().position(|(a, b)| lx >= *a && lx < *b) {
            let sub = self.wm.menus.get(&menu).and_then(|m| m.items.get(i)).map_or(0, |it| it.sub);
            if sub != 0 { let _ = self.open_popup(hwnd, sub, wx + spans[i].0, wy + y0 + MENU_H); }
        }
        true
    }

    pub(super) fn menu_call(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        Ok(Some(match name {
            "NtUserThunkedMenuItemInfo" => {
                // (menu, pos, flags, method, MENUITEMINFOW*, UNICODE_STRING*)
                let (menu, pos, flags, method, info, ustr) = (a[0], a[1], a[2], a[3], a[4], a[5]);
                if !self.wm.menus.contains_key(&menu) { return Ok(Some(0)); }
                match method {
                    1 => { // InsertMenuItem
                        let mask = if info != 0 { self.r32(info + 4)? } else { 0 };
                        let (ftype, id, sub) = if info != 0 { (self.r32(info + 8)?, self.r32(info + 16)?, if mask & 4 != 0 { self.r32(info + 20)? } else { 0 }) } else { (0, 0, 0) };
                        let text = if ustr != 0 { self.get_ustr(ustr)? } else if info != 0 && mask & 0x40 != 0 && ftype & MF_SEPARATOR == 0 {
                            let p = self.r32(info + 36)?;
                            let mut n = 0; while p != 0 && self.mem.r16(p + n * 2).unwrap_or(0) != 0 && n < 256 { n += 1; }
                            if p != 0 { self.wstr(p, n)? } else { String::new() }
                        } else { String::new() };
                        let state = if info != 0 { self.r32(info + 12)? } else { 0 };
                        let it = Item { id, flags: (ftype & MF_SEPARATOR) | (if sub != 0 { MF_POPUP } else { 0 }) | (state & (MF_GRAYED | MF_DISABLED)), text, sub };
                        let m = self.wm.menus.get_mut(&menu).unwrap();
                        let at = if pos == u32::MAX { m.items.len() } else if flags & MF_BYPOSITION != 0 { (pos as usize).min(m.items.len()) } else { m.items.iter().position(|i| i.id == pos).unwrap_or(m.items.len()) };
                        m.items.insert(at, it);
                        for w in self.wm.windows.keys().copied().collect::<Vec<_>>() { if self.wm.windows[&w].menu == menu { self.paint_frame(w); self.paint_menu_bar(w); } }
                        1
                    }
                    0 => { // SetMenuItemInfo: only state / text changes we understand
                        let Some(i) = self.menu_find(menu, pos, flags) else { return Ok(Some(0)) };
                        let mask = self.r32(info + 4)?;
                        if mask & 1 != 0 { let st = self.r32(info + 12)?; let it = &mut self.wm.menus.get_mut(&menu).unwrap().items[i]; it.flags = (it.flags & !(MF_GRAYED | MF_DISABLED)) | (st & (MF_GRAYED | MF_DISABLED)); }
                        1
                    }
                    2 => 1, // CheckMenuRadioItem: the radio mark is not drawn yet
                    3 => u32::MAX, // GetMenuDefaultItem: no default item
                    4 | 7 | 8 | 5 | 6 => { // GetMenuItemID / GetMenuState / GetSubMenu / GetMenuItemInfoA|W
                        let Some(i) = self.menu_find(menu, pos, flags) else { return Ok(Some(if method == 8 { 0 } else { u32::MAX })) };
                        let it = self.wm.menus[&menu].items[i].clone_basic();
                        match method {
                            4 => if it.sub != 0 || it.flags & MF_SEPARATOR != 0 { u32::MAX } else { it.id },
                            8 => it.sub,
                            7 => if it.sub != 0 { (it.flags & (MF_GRAYED | MF_DISABLED)) | MF_POPUP | ((self.wm.menus.get(&it.sub).map_or(0, |m| m.items.len()) as u32) << 8) } else { it.flags & (MF_GRAYED | MF_DISABLED | MF_SEPARATOR) },
                            _ => { // MENUITEMINFO: cbSize, fMask, fType, fState, wID, hSubMenu, hbmpChecked, hbmpUnchecked, dwItemData, dwTypeData, cch, hbmpItem
                                if info == 0 { return Ok(Some(0)); }
                                let mask = self.r32(info + 4)?;
                                if mask & 0x100 != 0 || mask & 0x10 != 0 { self.w32(info + 8, it.flags & MF_SEPARATOR)?; }
                                if mask & 1 != 0 { self.w32(info + 12, it.flags & (MF_GRAYED | MF_DISABLED))?; }
                                if mask & 2 != 0 { self.w32(info + 16, it.id)?; }
                                if mask & 4 != 0 { self.w32(info + 20, it.sub)?; }
                                if mask & 0x40 != 0 || mask & 0x10 != 0 {
                                    let (buf, cch) = (self.r32(info + 36)?, self.r32(info + 40)?);
                                    let ansi = method == 5;
                                    if buf != 0 && cch > 0 {
                                        let chars: Vec<char> = it.text.chars().take(cch as usize - 1).collect();
                                        for (k, c) in chars.iter().enumerate() {
                                            if ansi { self.mem.w8(buf + k as u32, if (*c as u32) < 256 { *c as u8 } else { b'?' }).map_err(|_| "menu text")?; } else { self.w16(buf + k as u32 * 2, *c as u16)?; }
                                        }
                                        if ansi { self.mem.w8(buf + chars.len() as u32, 0).map_err(|_| "menu text")?; } else { self.w16(buf + chars.len() as u32 * 2, 0)?; }
                                        self.w32(info + 40, chars.len() as u32)?;
                                    } else { self.w32(info + 40, it.text.chars().count() as u32)?; }
                                }
                                1
                            }
                        }
                    }
                    _ => return Err(format!("NtUserThunkedMenuItemInfo method {method}")),
                }
            }
            "NtUserSetMenu" => {
                if let Some(w) = self.wm.windows.get_mut(&a[0]) { w.menu = a[1]; }
                self.paint_frame(a[0]); self.paint_menu_bar(a[0]); self.invalidate(a[0], true);
                1
            }
            "NtUserDestroyMenu" => self.wm.menus.remove(&a[0]).is_some() as u32,
            "NtUserEnableMenuItem" => {
                let Some(i) = self.menu_find(a[0], a[1], a[2]) else { return Ok(Some(u32::MAX)) };
                let it = &mut self.wm.menus.get_mut(&a[0]).unwrap().items[i];
                let old = it.flags & (MF_GRAYED | MF_DISABLED);
                it.flags = (it.flags & !(MF_GRAYED | MF_DISABLED)) | (a[2] & (MF_GRAYED | MF_DISABLED));
                old
            }
            "NtUserDeleteMenu" | "NtUserRemoveMenu" => match self.menu_find(a[0], a[1], a[2]) { Some(i) => { self.wm.menus.get_mut(&a[0]).unwrap().items.remove(i); 1 } None => 0 },
            "NtUserCheckMenuItem" => 0,
            "NtUserSetMenuDefaultItem" | "NtUserHiliteMenuItem" => 1,
            "NtUserGetMenuBarInfo" | "NtUserGetMenuItemRect" | "NtUserMenuItemFromPoint" => 0,
            "NtUserTrackPopupMenuEx" => {
                // (menu, flags, x, y, hwnd, params): park the guest until the popup closes
                if !self.wm.track_pending {
                    self.wm.track_pending = true;
                    self.wm.track_done = None;
                    self.open_popup(a[4], a[0], a[2] as i32, a[3] as i32)?;
                }
                if self.wm.open.is_some() { self.idle = true; self.took_over = true; return Ok(Some(0)); }
                self.wm.track_pending = false;
                let chosen = self.wm.track_done.take().unwrap_or(0);
                if a[1] & 0x100 != 0 { chosen } else { (chosen != 0) as u32 }
            }
            _ => return Ok(None),
        }))
    }
}
