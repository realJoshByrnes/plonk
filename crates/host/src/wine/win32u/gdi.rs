//! GDI bitmap engine: memory DCs, DDB/DIB bitmaps (pixels live in guest memory), BitBlt /
//! StretchBlt / AlphaBlend with raster ops, DIB transfer functions and GetObject.
use super::*;

/// A GDI bitmap. Pixels are in guest memory so DIB sections can be written by the app directly.
#[derive(Clone, Copy)]
pub(super) struct Bitmap {
    w: i32,
    h: i32,
    bpp: u32,
    stride: u32,
    top_down: bool,
    addr: u32,
    /// DIB sections keep their alpha channel; DDBs are always opaque.
    dib: bool,
}

impl Bitmap { pub(super) fn addr(&self) -> u32 { self.addr } }

fn align4(v: u32) -> u32 { (v + 3) & !3 }

fn rop_apply(rop8: u32, p: u32, s: u32, d: u32) -> u32 {
    let mut out = 0;
    for idx in 0..8u32 {
        if rop8 >> idx & 1 == 0 { continue; }
        let t = |bit: u32, v: u32| if idx & bit != 0 { v } else { !v };
        out |= t(4, p) & t(2, s) & t(1, d);
    }
    out & 0xFF_FFFF
}

fn argb_of_colorref(c: u32) -> u32 { 0xFF00_0000 | (c & 0xff) << 16 | (c & 0xff00) | (c >> 16 & 0xff) }

/// Decode a BITMAPINFO + bits into top-down ARGB (alpha kept for 32bpp).
struct Dib { w: i32, h: i32, pix: Vec<u32> }

impl WineProc {
    // ---- bitmap storage ----
    fn bmp_alloc(&mut self, w: i32, h: i32, bpp: u32, top_down: bool, dib: bool) -> Res<(u32, u32)> {
        let (w, h) = (w.max(1), h.max(1));
        let stride = align4((w as u32 * bpp + 7) / 8);
        let addr = self.vm_reserve(None, stride * h as u32).ok_or("bitmap: out of guest memory")?;
        let hbm = self.gdi_alloc(0x090000, Gdi::Bitmap);
        self.wm.bitmaps.insert(hbm, Bitmap { w, h, bpp, stride, top_down, addr, dib });
        Ok((hbm, addr))
    }

    fn bmp_row(&self, b: &Bitmap, y: i32) -> u32 { b.addr + b.stride * if b.top_down { y } else { b.h - 1 - y } as u32 }

    fn bmp_get(&self, hbm: u32, x: i32, y: i32) -> u32 {
        let Some(b) = self.wm.bitmaps.get(&hbm).copied() else { return 0 };
        if x < 0 || y < 0 || x >= b.w || y >= b.h { return 0; }
        let row = self.bmp_row(&b, y);
        let v = match b.bpp {
            32 => self.mem.r32(row + x as u32 * 4).unwrap_or(0),
            24 => { let o = row + x as u32 * 3; 0xFF00_0000 | (self.mem.r8(o + 2).unwrap_or(0) as u32) << 16 | (self.mem.r8(o + 1).unwrap_or(0) as u32) << 8 | self.mem.r8(o).unwrap_or(0) as u32 }
            16 => { let p = self.mem.r16(row + x as u32 * 2).unwrap_or(0) as u32; let e = |v: u32| (v << 3 | v >> 2) & 0xff; 0xFF00_0000 | e(p >> 10 & 31) << 16 | e(p >> 5 & 31) << 8 | e(p & 31) }
            1 => { let bit = self.mem.r8(row + x as u32 / 8).unwrap_or(0) >> (7 - x % 8) & 1; if bit != 0 { 0xFFFF_FFFF } else { 0xFF00_0000 } }
            _ => 0xFF00_0000,
        };
        if b.dib && b.bpp == 32 { v } else { v | 0xFF00_0000 }
    }

    pub(super) fn bmp_set(&mut self, hbm: u32, x: i32, y: i32, argb: u32) {
        let Some(b) = self.wm.bitmaps.get(&hbm).copied() else { return };
        if x < 0 || y < 0 || x >= b.w || y >= b.h { return; }
        let row = self.bmp_row(&b, y);
        match b.bpp {
            32 => { let _ = self.mem.w32(row + x as u32 * 4, argb); }
            24 => { let o = row + x as u32 * 3; let _ = self.mem.w8(o, argb as u8); let _ = self.mem.w8(o + 1, (argb >> 8) as u8); let _ = self.mem.w8(o + 2, (argb >> 16) as u8); }
            16 => { let v = ((argb >> 19 & 31) << 10 | (argb >> 11 & 31) << 5 | (argb >> 3 & 31)) as u16; let _ = self.mem.w16(row + x as u32 * 2, v); }
            1 => {
                let o = row + x as u32 / 8;
                let m = 0x80u8 >> (x % 8);
                let cur = self.mem.r8(o).unwrap_or(0);
                let on = (argb >> 16 & 0xff) + (argb >> 8 & 0xff) + (argb & 0xff) >= 384;
                let _ = self.mem.w8(o, if on { cur | m } else { cur & !m });
            }
            _ => {}
        }
    }

    pub(super) fn bmp_fill(&mut self, hbm: u32, r: Rect, argb: u32) {
        for y in r[1]..r[3] { for x in r[0]..r[2] { self.bmp_set(hbm, x, y, argb); } }
    }

    /// Font selected into a DC as (pixel size, bold); default is the UI font at 12px.
    fn dc_font(&self, hdc: u32) -> (f32, bool) {
        self.wm.dcs.get(&hdc).and_then(|d| self.wm.fonts.get(&d.font)).map_or((12.0, false), |f| (f.px, f.bold))
    }

    // ---- logical -> device mapping (DC_ATTR window/viewport origin and extents) ----
    fn dc_map(&self, hdc: u32) -> [i32; 8] {
        let at = self.r32(self.gdi_table + (hdc & 0xffff) * 24 + 16).unwrap_or(0);
        if at == 0 { return [0, 0, 1, 1, 0, 0, 1, 1]; }
        let g = |o: u32| self.r32(at + o).unwrap_or(0) as i32;
        // wnd_org, wnd_ext, vport_org, vport_ext
        let m = [g(100), g(104), g(108), g(112), g(116), g(120), g(124), g(128)];
        [m[0], m[1], if m[2] == 0 { 1 } else { m[2] }, if m[3] == 0 { 1 } else { m[3] }, m[4], m[5], if m[6] == 0 { 1 } else { m[6] }, if m[7] == 0 { 1 } else { m[7] }]
    }
    pub(super) fn lp(&self, hdc: u32, x: i32, y: i32) -> (i32, i32) {
        let m = self.dc_map(hdc);
        let sc = |v: i32, num: i32, den: i32| (v as i64 * num as i64 / den as i64) as i32;
        (sc(x - m[0], m[6], m[2]) + m[4], sc(y - m[1], m[7], m[3]) + m[5])
    }
    pub(super) fn dp(&self, hdc: u32, x: i32, y: i32) -> (i32, i32) {
        let m = self.dc_map(hdc);
        let sc = |v: i32, num: i32, den: i32| (v as i64 * num as i64 / den as i64) as i32;
        (sc(x - m[4], m[2], m[6]) + m[0], sc(y - m[5], m[3], m[7]) + m[1])
    }
    pub(super) fn lp_size(&self, hdc: u32, w: i32, h: i32) -> (i32, i32) {
        let m = self.dc_map(hdc);
        ((w as i64 * m[6] as i64 / m[2] as i64) as i32, (h as i64 * m[7] as i64 / m[3] as i64) as i32)
    }
    pub(super) fn lp_rect(&self, hdc: u32, r: Rect) -> Rect {
        let (a, b) = (self.lp(hdc, r[0], r[1]), self.lp(hdc, r[2], r[3]));
        [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)]
    }

    // ---- DC pixel access (client coordinates) ----
    fn dc_read_rect(&self, hdc: u32, x: i32, y: i32, w: i32, h: i32) -> Vec<u32> {
        let mut out = vec![0xFF00_0000u32; (w.max(0) * h.max(0)) as usize];
        let Some(dc) = self.wm.dcs.get(&hdc) else { return out };
        if dc.bitmap != 0 {
            for yy in 0..h { for xx in 0..w { out[(yy * w + xx) as usize] = self.bmp_get(dc.bitmap, x + xx, y + yy); } }
            return out;
        }
        let top = self.toplevel_of(dc.hwnd);
        let Some(sf) = self.wm.windows.get(&top).and_then(|t| t.surface).and_then(|s| self.wm.backend.surface(s)) else { return out };
        let (ox, oy) = self.client_origin_screen(dc.hwnd);
        let (tx, ty) = self.wm.windows.get(&top).map_or((0, 0), |t| (t.rect[0], t.rect[1]));
        for yy in 0..h {
            for xx in 0..w {
                let (sx, sy) = (ox - tx + x + xx, oy - ty + y + yy);
                if sx >= 0 && sy >= 0 && (sx as u32) < sf.w && (sy as u32) < sf.h { out[(yy * w + xx) as usize] = sf.pix[(sy as u32 * sf.w + sx as u32) as usize] | 0xFF00_0000; }
            }
        }
        out
    }

    fn dc_write_rect(&mut self, hdc: u32, x: i32, y: i32, w: i32, h: i32, px: &[u32]) {
        let Some(dc) = self.wm.dcs.get(&hdc) else { return };
        let (hwnd, bitmap, clip) = (dc.hwnd, dc.bitmap, dc.clip);
        if bitmap != 0 {
            let cl = clip.unwrap_or([i32::MIN / 2, i32::MIN / 2, i32::MAX / 2, i32::MAX / 2]);
            for yy in 0..h { for xx in 0..w {
                let (px_, py_) = (x + xx, y + yy);
                if px_ >= cl[0] && px_ < cl[2] && py_ >= cl[1] && py_ < cl[3] { self.bmp_set(bitmap, px_, py_, px[(yy * w + xx) as usize]); }
            } }
            return;
        }
        // window DC: clip to the client area and the DC clip, then one blit into the top-level surface
        let cr = self.wm.client_rect(hwnd);
        let mut r = [x, y, x + w, y + h];
        for c in [Some(cr), clip].into_iter().flatten() { r = [r[0].max(c[0]), r[1].max(c[1]), r[2].min(c[2]), r[3].min(c[3])]; }
        if r[2] <= r[0] || r[3] <= r[1] || !self.is_shown(hwnd) { return; }
        let top = self.toplevel_of(hwnd);
        let Some(s) = self.wm.windows.get(&top).and_then(|t| t.surface) else { return };
        let (ox, oy) = self.client_origin_screen(hwnd);
        let (tx, ty) = { let t = &self.wm.windows[&top]; (t.rect[0], t.rect[1]) };
        let (cw, ch) = (r[2] - r[0], r[3] - r[1]);
        let mut sub = Vec::with_capacity((cw * ch) as usize);
        for yy in r[1]..r[3] { for xx in r[0]..r[2] { sub.push(px[((yy - y) * w + (xx - x)) as usize] | 0xFF00_0000); } }
        self.wm.backend.blit(s, ox - tx + r[0], oy - ty + r[1], cw as u32, ch as u32, &sub, cw as u32);
    }

    fn dc_pattern(&self, hdc: u32) -> u32 {
        let brush = self.wm.dcs.get(&hdc).map_or(0, |d| d.brush);
        self.brush_argb(brush) & 0xFF_FFFF
    }

    fn resample(src: &[u32], sw: i32, sh: i32, dw: i32, dh: i32) -> Vec<u32> {
        if (sw, sh) == (dw, dh) { return src.to_vec(); }
        let mut out = Vec::with_capacity((dw * dh) as usize);
        for y in 0..dh { for x in 0..dw { out.push(src[((y * sh / dh) * sw + x * sw / dw) as usize]); } }
        out
    }

    /// Combine source/destination with the DC pattern using a ternary raster operation.
    fn blit_core(&mut self, dst: u32, x: i32, y: i32, w: i32, h: i32, src: Option<&[u32]>, rop: u32) {
        let rop8 = rop >> 16 & 0xff;
        let pat = self.dc_pattern(dst);
        let d = self.dc_read_rect(dst, x, y, w, h);
        let mut out = Vec::with_capacity(d.len());
        for (i, dp) in d.iter().enumerate() {
            let sp = src.map_or(0, |s| s[i]) & 0xFF_FFFF;
            out.push(0xFF00_0000 | rop_apply(rop8, pat, sp, dp & 0xFF_FFFF));
        }
        self.dc_write_rect(dst, x, y, w, h, &out);
    }

    fn rop_needs_src(rop: u32) -> bool { let r = rop >> 16 & 0xff; (0..8u32).any(|i| (r >> i & 1) != (r >> (i ^ 2) & 1)) }

    fn decode_dib(&self, bmi: u32, bits: u32, usage_rgb: bool) -> Res<Dib> {
        let (size, w, h) = (self.r32(bmi)?, self.r32(bmi + 4)? as i32, self.r32(bmi + 8)? as i32);
        let bpp = self.mem.r16(bmi + 14).map_err(|_| "dib")? as u32;
        let (clr_used, comp) = (self.r32(bmi + 32)?, self.r32(bmi + 16)?);
        let _ = usage_rgb;
        let top_down = h < 0;
        let (hh, ww) = (h.abs(), w.abs());
        let ncol = if bpp <= 8 { if clr_used != 0 { clr_used } else { 1 << bpp } } else { 0 };
        let pal: Vec<u32> = (0..ncol).map(|i| self.r32(bmi + size + i * 4).unwrap_or(0) | 0xFF00_0000).collect();
        let stride = align4((ww as u32 * bpp + 7) / 8);
        let mut pix = vec![0xFF00_0000u32; (ww * hh) as usize];
        let _ = comp;
        for y in 0..hh {
            let row = bits + stride * if top_down { y } else { hh - 1 - y } as u32;
            for x in 0..ww {
                let xu = x as u32;
                pix[(y * ww + x) as usize] = match bpp {
                    32 => self.mem.r32(row + xu * 4).unwrap_or(0),
                    24 => { let o = row + xu * 3; 0xFF00_0000 | (self.mem.r8(o + 2).unwrap_or(0) as u32) << 16 | (self.mem.r8(o + 1).unwrap_or(0) as u32) << 8 | self.mem.r8(o).unwrap_or(0) as u32 }
                    16 => { let p = self.mem.r16(row + xu * 2).unwrap_or(0) as u32; let e = |v: u32| (v << 3 | v >> 2) & 0xff; 0xFF00_0000 | e(p >> 10 & 31) << 16 | e(p >> 5 & 31) << 8 | e(p & 31) }
                    8 => pal.get(self.mem.r8(row + xu).unwrap_or(0) as usize).copied().unwrap_or(0xFF00_0000),
                    4 => { let v = self.mem.r8(row + xu / 2).unwrap_or(0); let i = if xu % 2 == 0 { v >> 4 } else { v & 15 }; pal.get(i as usize).copied().unwrap_or(0xFF00_0000) }
                    1 => { let v = self.mem.r8(row + xu / 8).unwrap_or(0) >> (7 - xu % 8) & 1; pal.get(v as usize).copied().unwrap_or(if v != 0 { 0xFFFF_FFFF } else { 0xFF00_0000 }) }
                    _ => 0xFF00_0000,
                };
            }
        }
        Ok(Dib { w: ww, h: hh, pix })
    }

    // ---- syscalls ----
    pub(super) fn gdi_call(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        Ok(Some(match name {
            "NtGdiCreateCompatibleDC" => {
                let hdc = self.new_dc(0, None);
                if self.wm.stock_bitmap == 0 {
                    let (h, _) = self.bmp_alloc(1, 1, 32, true, false)?;
                    self.wm.stock_bitmap = h | 0x80_0000;
                    if let Some(b) = self.wm.bitmaps.remove(&h) { self.wm.bitmaps.insert(self.wm.stock_bitmap, b); self.wm.gdi.remove(&h); self.wm.gdi.insert(self.wm.stock_bitmap, Gdi::Bitmap); }
                }
                let sb = self.wm.stock_bitmap;
                if let Some(dc) = self.wm.dcs.get_mut(&hdc) { dc.bitmap = sb; }
                hdc
            }
            "NtGdiCreateCompatibleBitmap" => self.bmp_alloc(a[1] as i32, a[2] as i32, 32, true, false)?.0,
            "NtGdiCreateBitmap" => {
                let (w, h, bpp, bits) = (a[0] as i32, a[1] as i32, a[3], a[4]);
                let bpp = if bpp == 1 { 1 } else { 32 };
                let (hbm, _) = self.bmp_alloc(w, h, bpp, true, false)?;
                if bits != 0 {
                    let src_stride = if bpp == 1 { ((w as u32 + 15) / 16) * 2 } else { w as u32 * 4 };
                    for y in 0..h { for x in 0..w {
                        let v = if bpp == 1 { let byte = self.mem.r8(bits + y as u32 * src_stride + x as u32 / 8).unwrap_or(0); if byte >> (7 - x % 8) & 1 != 0 { 0xFFFF_FFFF } else { 0xFF00_0000 } }
                                else { self.mem.r32(bits + y as u32 * src_stride + x as u32 * 4).unwrap_or(0) };
                        self.bmp_set(hbm, x, y, v);
                    } }
                }
                hbm
            }
            "NtGdiCreateDIBSection" => {
                let bmi = a[3];
                let (w, h) = (self.r32(bmi + 4)? as i32, self.r32(bmi + 8)? as i32);
                let bpp = self.mem.r16(bmi + 14).map_err(|_| "dibsection")? as u32;
                let bpp = if matches!(bpp, 16 | 24 | 32) { bpp } else { 32 }; // palette DIB sections are stored as 32bpp
                let (hbm, addr) = self.bmp_alloc(w.abs(), h.abs(), bpp, h < 0, true)?;
                if a[8] != 0 { self.w32(a[8], addr)?; }
                hbm
            }
            "NtGdiSelectBitmap" => {
                let Some(dc) = self.wm.dcs.get_mut(&a[0]) else { return Ok(Some(0)) };
                if !self.wm.bitmaps.contains_key(&a[1]) { return Ok(Some(0)); }
                std::mem::replace(&mut dc.bitmap, a[1])
            }
            "NtGdiBitBlt" => {
                let (dst, x, y, w, h, src, xs, ys, rop) = (a[0], a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32, a[5], a[6] as i32, a[7] as i32, a[8]);
                let ((x, y), (w, h)) = (self.lp(dst, x, y), self.lp_size(dst, w, h));
                let (xs, ys) = if src != 0 { self.lp(src, xs, ys) } else { (xs, ys) };
                if w <= 0 || h <= 0 { return Ok(Some(1)); }
                let s = if src != 0 && Self::rop_needs_src(rop) { Some(self.dc_read_rect(src, xs, ys, w, h)) } else { None };
                self.blit_core(dst, x, y, w, h, s.as_deref(), rop);
                1
            }
            "NtGdiStretchBlt" => {
                let (dst, x, y, wd, hd, src, xs, ys, ws, hs, rop) = (a[0], a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32, a[5], a[6] as i32, a[7] as i32, a[8] as i32, a[9] as i32, a[10]);
                let ((x, y), (wd, hd)) = (self.lp(dst, x, y), self.lp_size(dst, wd, hd));
                let ((xs, ys), (ws, hs)) = (self.lp(src, xs, ys), self.lp_size(src, ws, hs));
                if wd == 0 || hd == 0 { return Ok(Some(1)); }
                let (wda, hda) = (wd.abs(), hd.abs());
                let s = if src != 0 && Self::rop_needs_src(rop) && ws != 0 && hs != 0 {
                    let px = self.dc_read_rect(src, xs.min(xs + ws), ys.min(ys + hs), ws.abs(), hs.abs());
                    Some(Self::resample(&px, ws.abs(), hs.abs(), wda, hda))
                } else { None };
                self.blit_core(dst, x.min(x + wd), y.min(y + hd), wda, hda, s.as_deref(), rop);
                1
            }
            "NtGdiAlphaBlend" => {
                let (dst, x, y, wd, hd, src, xs, ys, ws, hs, bf) = (a[0], a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32, a[5], a[6] as i32, a[7] as i32, a[8] as i32, a[9] as i32, a[10]);
                let ((x, y), (wd, hd)) = (self.lp(dst, x, y), self.lp_size(dst, wd, hd));
                let ((xs, ys), (ws, hs)) = (self.lp(src, xs, ys), self.lp_size(src, ws, hs));
                if wd <= 0 || hd <= 0 || ws <= 0 || hs <= 0 { return Ok(Some(0)); }
                let (sca, per_pixel) = ((bf >> 16 & 0xff) as f32 / 255.0, bf >> 24 & 1 != 0);
                let spx = self.dc_read_rect(src, xs, ys, ws, hs);
                let spx = Self::resample(&spx, ws, hs, wd, hd);
                let dpx = self.dc_read_rect(dst, x, y, wd, hd);
                let ch = |c: u32, sh: u32| (c >> sh & 0xff) as f32;
                let out: Vec<u32> = spx.iter().zip(dpx.iter()).map(|(s, d)| {
                    let sa = if per_pixel { ch(*s, 24) / 255.0 * sca } else { sca };
                    let f = |sh: u32| {
                        let sc = if per_pixel { ch(*s, sh) * sca } else { ch(*s, sh) * sa };
                        (sc + ch(*d, sh) * (1.0 - sa)).clamp(0.0, 255.0) as u32
                    };
                    0xFF00_0000 | f(16) << 16 | f(8) << 8 | f(0)
                }).collect();
                self.dc_write_rect(dst, x, y, wd, hd, &out);
                1
            }
            "NtGdiSetDIBitsToDeviceInternal" => {
                // (hdc, x_dst, y_dst, cx, cy, x_src, y_src, startscan, lines, bits, bmi, ...)
                let (hdc, xd, yd, cx, cy, xs, ys, bits, bmi) = (a[0], a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32, a[5] as i32, a[6] as i32, a[9], a[10]);
                let d = self.decode_dib(bmi, bits, false)?;
                let mut out = vec![0xFF00_0000u32; (cx * cy).max(0) as usize];
                for y in 0..cy { for x in 0..cx {
                    let (sx, sy) = (xs + x, d.h - 1 - (ys + y)); // source y counts from the bottom of the DIB
                    if sx >= 0 && sy >= 0 && sx < d.w && sy < d.h { out[(y * cx + x) as usize] = d.pix[(sy * d.w + sx) as usize]; }
                } }
                self.dc_write_rect(hdc, xd, yd, cx, cy, &out);
                a[8] // scan lines copied
            }
            "NtGdiStretchDIBitsInternal" => {
                // (hdc, xd, yd, wd, hd, xs, ys, ws, hs, bits, bmi, coloruse, rop, ...)
                let (hdc, xd, yd, wd, hd, xs, ys, ws, hs, bits, bmi, rop) = (a[0], a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32, a[5] as i32, a[6] as i32, a[7] as i32, a[8] as i32, a[9], a[10], a[12]);
                let d = self.decode_dib(bmi, bits, false)?;
                let (wda, hda) = (wd.abs().max(1), hd.abs().max(1));
                let mut src = vec![0xFF00_0000u32; (ws.abs() * hs.abs()).max(0) as usize];
                for y in 0..hs.abs() { for x in 0..ws.abs() {
                    let (sx, sy) = (xs + x, d.h - 1 - (ys + y));
                    if sx >= 0 && sy >= 0 && sx < d.w && sy < d.h { src[(y * ws.abs() + x) as usize] = d.pix[(sy * d.w + sx) as usize]; }
                } }
                let px = Self::resample(&src, ws.abs().max(1), hs.abs().max(1), wda, hda);
                self.blit_core(hdc, xd.min(xd + wd), yd.min(yd + hd), wda, hda, Some(&px), rop);
                hs.unsigned_abs()
            }
            "NtGdiGetDIBitsInternal" => {
                // (hdc, hbitmap, startscan, lines, bits, info, coloruse, max_bits, max_info)
                let (hbm, bits, info) = (a[1], a[4], a[5]);
                let Some(b) = self.wm.bitmaps.get(&hbm).copied() else { return Ok(Some(0)) };
                if bits == 0 { // fill in the header only
                    for (o, v) in [(0u32, 40u32), (4, b.w as u32), (8, b.h as u32 | 0)] { self.w32(info + o, v)?; }
                    self.w16(info + 12, 1)?; self.w16(info + 14, 32)?; self.w32(info + 16, 0)?;
                    return Ok(Some(b.h as u32));
                }
                let want_bpp = self.mem.r16(info + 14).unwrap_or(32) as u32;
                let (h_req, top_down) = (self.r32(info + 8)? as i32, (self.r32(info + 8)? as i32) < 0);
                let h = h_req.abs().min(b.h);
                let stride = align4((b.w as u32 * want_bpp + 7) / 8);
                for y in 0..h {
                    let src_y = if top_down { y } else { b.h - 1 - y };
                    let row = bits + stride * y as u32;
                    for x in 0..b.w {
                        let c = self.bmp_get(hbm, x, src_y);
                        match want_bpp {
                            32 => { let _ = self.mem.w32(row + x as u32 * 4, c); }
                            24 => { let o = row + x as u32 * 3; let _ = self.mem.w8(o, c as u8); let _ = self.mem.w8(o + 1, (c >> 8) as u8); let _ = self.mem.w8(o + 2, (c >> 16) as u8); }
                            _ => {}
                        }
                    }
                }
                h as u32
            }
            "NtGdiExtGetObjectW" => {
                let (handle, count, buf) = (a[0], a[1], a[2]);
                if let Some(b) = self.wm.bitmaps.get(&handle).copied() {
                    if buf == 0 { return Ok(Some(if b.dib { 84 } else { 24 })); }
                    // BITMAP: type, width, height, widthBytes, planes, bitsPixel, bits
                    for (i, v) in [0u32, b.w as u32, b.h as u32, b.stride].iter().enumerate() { self.w32(buf + i as u32 * 4, *v)?; }
                    self.w16(buf + 16, 1)?; self.w16(buf + 18, b.bpp as u16)?;
                    self.w32(buf + 20, if b.dib { b.addr } else { 0 })?;
                    if b.dib && count >= 84 {
                        for (i, v) in [40u32, b.w as u32, if b.top_down { (-b.h) as u32 } else { b.h as u32 }].iter().enumerate() { self.w32(buf + 24 + i as u32 * 4, *v)?; }
                        self.w16(buf + 36, 1)?; self.w16(buf + 38, b.bpp as u16)?;
                        for o in (40..84).step_by(4) { self.w32(buf + o, 0)?; }
                        return Ok(Some(84));
                    }
                    return Ok(Some(24));
                }
                if let Some(f) = self.wm.fonts.get(&handle).cloned() {
                    if buf == 0 { return Ok(Some(92)); }
                    let raw = if f.raw.len() == 92 { f.raw } else { let mut r = vec![0u8; 92]; r[0..4].copy_from_slice(&(-(f.px as i32)).to_le_bytes()); r[16..20].copy_from_slice(&(if f.bold { 700u32 } else { 400 }).to_le_bytes()); r };
                    self.mem.write_bytes(buf, &raw[..(count as usize).min(92)]).map_err(|_| "getobject font")?;
                    return Ok(Some(count.min(92)));
                }
                if let Some(Gdi::Brush(c)) = self.wm.gdi.get(&handle).copied() { // LOGBRUSH
                    if buf == 0 { return Ok(Some(12)); }
                    self.w32(buf, 0)?; self.w32(buf + 4, argb_to_colorref(c))?; self.w32(buf + 8, 0)?;
                    return Ok(Some(12));
                }
                0
            }
            "NtGdiSetPixel" => {
                let c = argb_of_colorref(a[3]);
                let (x, y) = self.lp(a[0], a[1] as i32, a[2] as i32);
                self.dc_write_rect(a[0], x, y, 1, 1, &[c]);
                a[3]
            }
            "NtGdiGetPixel" => {
                let (x, y) = self.lp(a[0], a[1] as i32, a[2] as i32);
                let p = self.dc_read_rect(a[0], x, y, 1, 1)[0];
                argb_to_colorref(p)
            }
            "NtGdiHfontCreate" => {
                // (ENUMLOGFONTEXDVW*, size, type, flags, client data); LOGFONTW is the first 92 bytes
                let lf = a[0];
                let h = self.r32(lf)? as i32;
                let weight = self.r32(lf + 16)?;
                let px = if h < 0 { -h as f32 } else if h > 0 { h as f32 / 1.21 } else { 12.0 };
                let mut face = String::new();
                for i in 0..32 { let c = self.mem.r16(lf + 28 + i * 2).unwrap_or(0); if c == 0 { break; } face.push(char::from_u32(c as u32).unwrap_or('?')); }
                let raw = self.mem.read_bytes(lf, 92).map_err(|_| "logfont")?.to_vec();
                let hf = self.gdi_alloc(0x0a0000, Gdi::Font);
                self.wm.fonts.insert(hf, FontSpec { px, bold: weight >= 600, face, raw });
                hf
            }
            "NtGdiGetTextMetricsW" => {
                let (px, bold) = self.dc_font(a[0]);
                let m = plonk_fonts::with(|f| (f.vmetrics(px, bold), f.text_width("abcdefghijklmnopqrstuvwxyz", px, bold) / 26.0, f.advance('W', px, bold)));
                let (asc, desc) = (m.0.ascent.ceil() as u32, m.0.descent.ceil() as u32);
                let vals: [(u32, u32); 11] = [(0, asc + desc), (4, asc), (8, desc), (12, (asc + desc).saturating_sub(px.round() as u32)), (16, m.0.line_gap.round() as u32),
                    (20, m.1.round() as u32), (24, (m.2 * 1.15).round() as u32), (28, if bold { 700 } else { 400 }), (32, 0), (36, 96), (40, 96)];
                for (o, v) in vals { self.w32(a[1] + o, v)?; }
                self.w16(a[1] + 44, 32)?; self.w16(a[1] + 46, 255)?; self.w16(a[1] + 48, b'?' as u16)?; self.w16(a[1] + 50, 32)?;
                self.w32(a[1] + 52, 0)?; self.w32(a[1] + 56, 0x2 << 8)?; // TMPF_TRUETYPE | TMPF_VECTOR in PitchAndFamily
                1
            }
            "NtGdiGetTextExtentExW" => {
                // (hdc, str, count, max_extent, *fit, *dx, SIZE*, flags)
                let (px, bold) = self.dc_font(a[0]);
                let text = self.wstr(a[1], a[2])?;
                let (mut acc, mut fit) = (0.0f32, 0u32);
                let max = a[3] as i32;
                let (asc, desc) = plonk_fonts::with(|f| { let v = f.vmetrics(px, bold); (v.ascent.ceil(), v.descent.ceil()) });
                for (i, _) in text.chars().enumerate() {
                    let pre: String = text.chars().take(i + 1).collect();
                    acc = plonk_fonts::with(|f| f.text_width(&pre, px, bold));
                    if a[5] != 0 { self.w32(a[5] + i as u32 * 4, acc.round() as u32)?; }
                    if max < 0 || acc.round() as i32 <= max { fit = i as u32 + 1; }
                }
                if a[4] != 0 { self.w32(a[4], fit)?; }
                self.w32(a[6], acc.round() as u32)?;
                self.w32(a[6] + 4, (asc + desc) as u32)?;
                1
            }
            "NtGdiExtTextOutW" => {
                // (hdc, x, y, options, rect*, str, count, dx*, codepage)
                let (hdc, x0, y00, opts, rect_p, str_p, count, dx_p) = (a[0], a[1] as i32, a[2] as i32, a[3], a[4], a[5], a[6], a[7]);
                let ((mut x, y0), _) = (self.lp(hdc, x0, y00), ());
                let text = if count > 0 { self.wstr(str_p, count)? } else { String::new() };
                let (px, bold) = self.dc_font(hdc);
                let (asc, desc, widths): (i32, i32, Vec<f32>) = plonk_fonts::with(|f| {
                    let v = f.vmetrics(px, bold);
                    let ws = text.chars().map(|c| f.advance(c, px, bold)).collect();
                    (v.ascent.ceil() as i32, v.descent.ceil() as i32, ws)
                });
                let adv: Vec<f32> = (0..text.chars().count()).map(|i| if dx_p != 0 { self.r32(dx_p + i as u32 * 4).unwrap_or(0) as i32 as f32 } else { widths[i] }).collect();
                let total: f32 = adv.iter().sum();
                let at = self.r32(self.gdi_table + (hdc & 0xffff) * 24 + 16).unwrap_or(0);
                let (align, bkmode, bk, tc) = if at != 0 {
                    (self.mem.r16(at + 48).unwrap_or(0) as u32, self.mem.r16(at + 50).unwrap_or(2) as u32, self.r32(at + 12).unwrap_or(0xFF_FFFF), self.r32(at + 24).unwrap_or(0))
                } else { (0, 2, 0xFF_FFFF, 0) };
                if align & 6 == 6 { x -= (total / 2.0) as i32 } else if align & 2 != 0 { x -= total as i32 }
                let ytop = if align & 24 == 24 { y0 - asc } else if align & 8 != 0 { y0 - asc - desc } else { y0 };
                let h = asc + desc;
                let rect = if rect_p != 0 { Some(self.lp_rect(hdc, self.rect_at(rect_p)?)) } else { None };
                if opts & 2 != 0 { if let Some(r) = rect { self.dc_fill(hdc, r, argb_of_colorref(bk)); } }
                else if bkmode == 2 && !text.is_empty() { self.dc_fill(hdc, [x, ytop, x + total.round() as i32, ytop + h], argb_of_colorref(bk)); }
                // blend glyph coverage over the destination bounding box
                let bb = [x - 2, ytop - 2, x + total.ceil() as i32 + 3, ytop + h + 2];
                let (bw, bh) = (bb[2] - bb[0], bb[3] - bb[1]);
                let mut px_buf = self.dc_read_rect(hdc, bb[0], bb[1], bw, bh);
                let col = argb_of_colorref(tc);
                let clip = if opts & 4 != 0 { rect } else { None };
                let mut pen = x as f32;
                for (i, c) in text.chars().enumerate() {
                    let g = plonk_fonts::with(|f| f.glyph(c, px, bold));
                    let (gx, gy) = (pen.round() as i32 + g.xmin, ytop + asc - g.ymin - g.height as i32);
                    for row in 0..g.height as i32 {
                        for cc in 0..g.width as i32 {
                            let cov = g.coverage[row as usize * g.width + cc as usize] as f32 / 255.0;
                            let (dx, dy) = (gx + cc, gy + row);
                            if cov <= 0.0 || dx < bb[0] || dy < bb[1] || dx >= bb[2] || dy >= bb[3] { continue; }
                            if let Some(r) = clip { if dx < r[0] || dy < r[1] || dx >= r[2] || dy >= r[3] { continue; } }
                            let idx = ((dy - bb[1]) * bw + (dx - bb[0])) as usize;
                            let old = px_buf[idx];
                            let m = |sh: u32| ((old >> sh & 0xff) as f32 * (1.0 - cov) + (col >> sh & 0xff) as f32 * cov).round() as u32;
                            px_buf[idx] = 0xFF00_0000 | m(16) << 16 | m(8) << 8 | m(0);
                        }
                    }
                    pen += adv[i];
                }
                self.dc_write_rect(hdc, bb[0], bb[1], bw, bh, &px_buf);
                1
            }
            "NtGdiGetTextFaceW" => {
                let face = self.wm.dcs.get(&a[0]).and_then(|d| self.wm.fonts.get(&d.font)).map_or("Inter".to_string(), |f| f.face.clone());
                if a[2] == 0 { return Ok(Some(face.encode_utf16().count() as u32 + 1)); }
                let u: Vec<u16> = face.encode_utf16().take((a[1] as usize).saturating_sub(1)).collect();
                for (i, c) in u.iter().enumerate() { self.w16(a[2] + i as u32 * 2, *c)?; }
                self.w16(a[2] + u.len() as u32 * 2, 0)?;
                u.len() as u32 + 1
            }
            "NtGdiGetCharWidthW" => {
                // (hdc, first, last, chars*, flags, buffer*)
                let (px, bold) = self.dc_font(a[0]);
                let n = a[2].saturating_sub(a[1]) + 1;
                for i in 0..n {
                    let c = char::from_u32(a[1] + i).unwrap_or('?');
                    let w = plonk_fonts::with(|f| f.advance(c, px, bold)).round() as u32;
                    self.w32(a[5] + i * 4, w)?;
                }
                1
            }
            "NtGdiTransformPoints" => {
                // (hdc, in, out, count, mode): mode 0 = LPtoDP, 1 = DPtoLP
                for i in 0..a[3] {
                    let (x, y) = (self.r32(a[1] + i * 8)? as i32, self.r32(a[1] + i * 8 + 4)? as i32);
                    let (nx, ny) = if a[4] == 0 { self.lp(a[0], x, y) } else { self.dp(a[0], x, y) };
                    self.w32(a[2] + i * 8, nx as u32)?;
                    self.w32(a[2] + i * 8 + 4, ny as u32)?;
                }
                1
            }
            "NtGdiGetAppClipBox" => {
                let Some(dc) = self.wm.dcs.get(&a[0]) else { return Ok(Some(0)) };
                let (hwnd, clip) = (dc.hwnd, dc.clip);
                let r = clip.unwrap_or_else(|| self.wm.client_rect(hwnd));
                self.put_rect(a[1], r)?;
                2
            }
            "NtGdiGetDCObject" => {
                let Some(dc) = self.wm.dcs.get(&a[0]) else { return Ok(Some(0)) };
                match a[1] { 1 => dc.pen, 2 => dc.brush, 6 => dc.font, 7 => dc.bitmap, _ => 0 }
            }
            "NtGdiSaveDC" => 1,
            "NtGdiRestoreDC" => 1,
            _ => return Ok(None),
        }))
    }
}
