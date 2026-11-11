//! Display backend abstraction: every window is its own surface; a shell/compositor decides
//! where surfaces sit. `SoftBackend` is the software implementation (Canvas 2D presents its
//! composed framebuffer; a WebGL compositor can later upload surfaces as textures).

pub type SurfaceId = u32;

/// 8x8 glyph rows (bit 0 = leftmost pixel); non-ASCII maps to '?'.
pub fn glyph8(c: char) -> [u8; 8] {
    font8x8::legacy::BASIC_LEGACY[if (c as u32) < 128 { c as usize } else { b'?' as usize }]
}

pub trait Backend {
    fn create_surface(&mut self, w: u32, h: u32) -> SurfaceId;
    fn resize_surface(&mut self, id: SurfaceId, w: u32, h: u32);
    fn destroy_surface(&mut self, id: SurfaceId);
    /// Colors are 0xAARRGGBB.
    fn fill_rect(&mut self, id: SurfaceId, x: i32, y: i32, w: i32, h: i32, argb: u32);
    fn draw_text(&mut self, id: SurfaceId, x: i32, y: i32, text: &str, argb: u32);
    /// Anti-aliased text with the bundled UI font; (x, y) is the top-left of the line box.
    fn draw_text_px(&mut self, id: SurfaceId, x: i32, y: i32, text: &str, argb: u32, px: f32, bold: bool);
    fn blit(&mut self, id: SurfaceId, x: i32, y: i32, w: u32, h: u32, pixels: &[u32], stride: u32);
    /// Shell-side placement of a surface on the desktop.
    fn place(&mut self, id: SurfaceId, x: i32, y: i32, visible: bool);
    /// Glass styling: corner radius, backdrop blur radius and shadow size (px); 0 disables each.
    fn set_glass(&mut self, id: SurfaceId, radius: i32, blur: i32, shadow: i32);
    /// Move a surface to the top of the stacking order.
    fn raise(&mut self, id: SurfaceId);
    /// Flush to the screen (no-op for headless).
    fn present(&mut self);
}

pub struct Surface {
    /// Bumped on every mutation (compositor cache key).
    pub ver: u64,
    pub w: u32,
    pub h: u32,
    pub pix: Vec<u32>,
    pub x: i32,
    pub y: i32,
    pub visible: bool,
    pub radius: i32,
    pub blur: i32,
    pub shadow: i32,
}

#[derive(Default)]
pub struct SoftBackend {
    surfaces: Vec<Option<Surface>>,
    /// Stacking order, bottom to top.
    order: Vec<SurfaceId>,
    wall: std::cell::RefCell<Option<(u32, u32, Vec<u32>)>>,
    /// Bumped by every mutation; lets a shell skip composing/drawing unchanged frames.
    pub gen: u64,
    top_cache: std::cell::RefCell<Option<TopCache>>,
    /// Count of present() calls, handy in tests.
    pub presents: u32,
}

impl SoftBackend {
    pub fn new() -> Self { Self::default() }
    pub fn surface(&self, id: SurfaceId) -> Option<&Surface> { self.surfaces.get(id as usize - 1)?.as_ref() }
    pub fn pixel(&self, id: SurfaceId, x: u32, y: u32) -> Option<u32> {
        let s = self.surface(id)?;
        if x < s.w && y < s.h { Some(s.pix[(y * s.w + x) as usize]) } else { None }
    }
    /// Compose all visible surfaces (stacking order) over a flat background.
    pub fn compose(&self, w: u32, h: u32, bg: u32) -> Vec<u32> {
        let mut out = vec![bg; (w * h) as usize];
        self.compose_into(&mut out, w, h);
        out
    }

    /// Compose over the default wallpaper (gives glass surfaces something to blur).
    /// Everything below the top window (plus its shadow and blurred backdrop) is cached, so a frame that only changed the top
    /// window's pixels costs one copy and one blit.
    pub fn compose_desktop(&self, w: u32, h: u32) -> Vec<u32> {
        let mut cache = self.wall.borrow_mut();
        if !matches!(&*cache, Some((cw, ch_, _)) if *cw == w && *ch_ == h) {
            *cache = Some((w, h, (0..h).flat_map(|y| (0..w).map(move |x| wallpaper(x, y, w, h))).collect()));
        }
        let wall_px = &cache.as_ref().unwrap().2;
        let vis: Vec<&Surface> = self.order.iter().filter_map(|id| self.surface(*id)).filter(|s| s.visible).collect();
        let Some((top, lower)) = vis.split_last() else { return wall_px.clone() };
        let mut key = vec![w as u64, h as u64, top.x as u32 as u64, top.y as u32 as u64, top.w as u64, top.h as u64, top.blur as u64, top.shadow as u64, top.radius as u64];
        for s in lower { key.extend([s.ver, s.x as u32 as u64, s.y as u32 as u64, s.w as u64, s.h as u64, s.blur as u64, s.shadow as u64, s.radius as u64]); }
        key.extend(self.order.iter().map(|i| *i as u64));
        let mut tc = self.top_cache.borrow_mut();
        if !matches!(&*tc, Some(c) if c.key == key) {
            let mut base = wall_px.clone();
            for s in lower { self.draw_surface(&mut base, w, h, s); }
            if top.shadow > 0 { draw_shadow(&mut base, w, h, top); }
            let blur = if top.blur > 0 { Some(blur_region(&base, w, h, top, (0, 0, top.w as i32, top.h as i32))) } else { None };
            *tc = Some(TopCache { key, base, blur });
        }
        let c = tc.as_ref().unwrap();
        let mut out = c.base.clone();
        self.blit_surface(&mut out, w, h, top, c.blur.as_ref());
        out
    }

    fn compose_into(&self, out: &mut [u32], w: u32, h: u32) {
        for s in self.order.iter().filter_map(|id| self.surface(*id)).filter(|s| s.visible) { self.draw_surface(out, w, h, s); }
    }

    fn draw_surface(&self, out: &mut [u32], w: u32, h: u32, s: &Surface) {
        if s.shadow > 0 { draw_shadow(out, w, h, s); }
        let blurred = if s.blur > 0 { Some(blur_region(out, w, h, s, (0, 0, s.w as i32, s.h as i32))) } else { None };
        self.blit_surface(out, w, h, s, blurred.as_ref());
    }

    fn blit_surface(&self, out: &mut [u32], w: u32, h: u32, s: &Surface, blurred: Option<&Blurred>) {
        for sy in 0..s.h as i32 {
            let dy = sy + s.y;
            if dy < 0 || dy >= h as i32 { continue; }
            for sx in 0..s.w as i32 {
                let dx = sx + s.x;
                if dx < 0 || dx >= w as i32 { continue; }
                let src = s.pix[(sy as u32 * s.w + sx as u32) as usize];
                let cov = if s.radius > 0 && in_corner(sx, sy, s.w as i32, s.h as i32, s.radius) { corner_cov(sx, sy, s.w as i32, s.h as i32, s.radius) } else { 1.0 };
                let a = (src >> 24) as f32 / 255.0;
                let di = (dy as u32 * w + dx as u32) as usize;
                if cov >= 1.0 && a >= 1.0 { out[di] = src | 0xFF00_0000; continue; }
                if cov <= 0.0 { continue; }
                let base = out[di];
                let back = match blurred { Some(b) => glass_adjust(b.at(sx, sy), dx, dy), None => base };
                let color = mix(back, src, a);
                out[di] = mix(base, color, cov);
            }
        }
    }

    fn get(&mut self, id: SurfaceId) -> Option<&mut Surface> { let s = self.surfaces.get_mut((id as usize).checked_sub(1)?)?.as_mut()?; s.ver += 1; Some(s) }
}

impl Backend for SoftBackend {
    fn create_surface(&mut self, w: u32, h: u32) -> SurfaceId {
        self.gen += 1;
        self.surfaces.push(Some(Surface { ver: 0, w, h, pix: vec![0xFF00_0000; (w * h) as usize], x: 0, y: 0, visible: false, radius: 0, blur: 0, shadow: 0 }));
        let id = self.surfaces.len() as SurfaceId;
        self.order.push(id);
        id
    }
    fn resize_surface(&mut self, id: SurfaceId, w: u32, h: u32) {
        self.gen += 1;
        if let Some(s) = self.get(id) {
            let mut pix = vec![0xFF00_0000; (w * h) as usize];
            for y in 0..h.min(s.h) { for x in 0..w.min(s.w) { pix[(y * w + x) as usize] = s.pix[(y * s.w + x) as usize]; } }
            s.w = w; s.h = h; s.pix = pix;
        }
    }
    fn destroy_surface(&mut self, id: SurfaceId) {
        self.gen += 1;
        if let Some(slot) = self.surfaces.get_mut((id as usize).wrapping_sub(1)) { *slot = None; }
        self.order.retain(|i| *i != id);
    }
    fn fill_rect(&mut self, id: SurfaceId, x: i32, y: i32, w: i32, h: i32, argb: u32) {
        self.gen += 1;
        let Some(s) = self.get(id) else { return };
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x + w).min(s.w as i32), (y + h).min(s.h as i32));
        for yy in y0..y1 { for xx in x0..x1 { s.pix[(yy as u32 * s.w + xx as u32) as usize] = argb; } }
    }
    fn draw_text_px(&mut self, id: SurfaceId, x: i32, y: i32, text: &str, argb: u32, px: f32, bold: bool) {
        self.gen += 1;
        let Some(s) = self.get(id) else { return };
        let ca = (argb >> 24) as f32 / 255.0;
        plonk_fonts::with(|f| {
            let base = y as f32 + f.vmetrics(px, bold).ascent;
            let mut pen = x as f32;
            for c in text.chars() {
                let g = f.glyph(c, px, bold);
                let (gx, gy) = (pen.round() as i32 + g.xmin, base.round() as i32 - g.ymin - g.height as i32);
                for row in 0..g.height as i32 {
                    for col in 0..g.width as i32 {
                        let cov = g.coverage[(row as usize) * g.width + col as usize] as f32 / 255.0 * ca;
                        let (dx, dy) = (gx + col, gy + row);
                        if cov <= 0.0 || dx < 0 || dy < 0 || dx >= s.w as i32 || dy >= s.h as i32 { continue; }
                        let i = (dy as u32 * s.w + dx as u32) as usize;
                        let old = s.pix[i];
                        let m = |sh: u32| ((old >> sh & 0xff) as f32 * (1.0 - cov) + (argb >> sh & 0xff) as f32 * cov).round() as u32;
                        let oa = (old >> 24) as f32;
                        let na = (oa + (255.0 - oa) * cov).round() as u32;
                        s.pix[i] = na << 24 | m(16) << 16 | m(8) << 8 | m(0);
                    }
                }
                pen += g.advance;
            }
        });
    }
    /// 8x8 bitmap font (font8x8, public domain), 8px advance. Non-ASCII renders as '?'.
    fn draw_text(&mut self, id: SurfaceId, x: i32, y: i32, text: &str, argb: u32) {
        self.gen += 1;
        let mut cx = x;
        for c in text.chars() {
            let g = font8x8::legacy::BASIC_LEGACY[if (c as u32) < 128 { c as usize } else { b'?' as usize }];
            for (row, bits) in g.iter().enumerate() {
                for col in 0..8 {
                    if bits >> col & 1 != 0 { self.fill_rect(id, cx + col, y + row as i32, 1, 1, argb); }
                }
            }
            cx += 8;
        }
    }
    fn blit(&mut self, id: SurfaceId, x: i32, y: i32, w: u32, h: u32, pixels: &[u32], stride: u32) {
        self.gen += 1;
        let Some(s) = self.get(id) else { return };
        for yy in 0..h as i32 {
            for xx in 0..w as i32 {
                let (dx, dy) = (x + xx, y + yy);
                if dx >= 0 && dy >= 0 && (dx as u32) < s.w && (dy as u32) < s.h {
                    s.pix[(dy as u32 * s.w + dx as u32) as usize] = pixels[(yy as u32 * stride + xx as u32) as usize];
                }
            }
        }
    }
    fn place(&mut self, id: SurfaceId, x: i32, y: i32, visible: bool) {
        self.gen += 1;
        if let Some(s) = self.get(id) { s.x = x; s.y = y; s.visible = visible; }
    }
    fn set_glass(&mut self, id: SurfaceId, radius: i32, blur: i32, shadow: i32) {
        self.gen += 1;
        if let Some(s) = self.get(id) { s.radius = radius; s.blur = blur; s.shadow = shadow; }
    }
    fn raise(&mut self, id: SurfaceId) {
        self.gen += 1;
        if self.order.contains(&id) { self.order.retain(|i| *i != id); self.order.push(id); }
    }
    fn present(&mut self) { self.presents += 1; }
}

fn ch(c: u32, sh: u32) -> f32 { ((c >> sh) & 0xff) as f32 }
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let m = |sh: u32| (ch(a, sh) * (1.0 - t) + ch(b, sh) * t).round().clamp(0.0, 255.0) as u32;
    0xFF00_0000 | m(16) << 16 | m(8) << 8 | m(0)
}

/// Anti-aliased coverage of a rounded rectangle at pixel (x, y).
fn corner_cov(x: i32, y: i32, w: i32, h: i32, r: i32) -> f32 {
    let (px, py, r) = (x as f32 + 0.5, y as f32 + 0.5, r as f32);
    let (cx, cy) = (px.clamp(r, w as f32 - r), py.clamp(r, h as f32 - r));
    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - r;
    (0.5 - d).clamp(0.0, 1.0)
}

fn in_corner(x: i32, y: i32, w: i32, h: i32, r: i32) -> bool { (x < r || x >= w - r) && (y < r || y >= h - r) }

fn draw_shadow(out: &mut [u32], w: u32, h: u32, s: &Surface) {
    let (sh, off) = (s.shadow, s.shadow / 3);
    let r = s.radius.max(1) as f32;
    for dy in (s.y - sh).max(0)..(s.y + s.h as i32 + sh + off).min(h as i32) {
        let py0 = (dy - s.y - off) as f32 + 0.5;
        let inner = py0 >= r && py0 <= s.h as f32 - r; // rows where the window body covers [x+r, x+w-r]
        for dx in (s.x - sh).max(0)..(s.x + s.w as i32 + sh).min(w as i32) {
            if inner && dx as f32 >= s.x as f32 + r && (dx as f32) <= (s.x + s.w as i32) as f32 - r { continue; }
            let (px, py) = ((dx - s.x) as f32 + 0.5, (dy - s.y - off) as f32 + 0.5);
            let (cx, cy) = (px.clamp(r, s.w as f32 - r), py.clamp(r, s.h as f32 - r));
            let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - r;
            if d <= 0.0 { continue; }
            let a = 0.32 * (1.0 - d / sh as f32).clamp(0.0, 1.0).powi(2);
            let i = (dy as u32 * w + dx as u32) as usize;
            out[i] = mix(out[i], 0xFF00_0000, a);
        }
    }
}

/// Backdrop under a surface, box-blurred (3 passes ~ gaussian). Indexed like the surface pixels.
struct TopCache { key: Vec<u64>, base: Vec<u32>, blur: Option<Blurred> }

struct Blurred { px: Vec<[f32; 3]>, dw: usize, dh: usize, f: usize }
impl Blurred {
    /// Bilinear sample at region-local pixel (x, y).
    fn at(&self, x: i32, y: i32) -> u32 {
        let f = self.f as f32;
        let fy = ((y as f32 + 0.5) / f - 0.5).clamp(0.0, (self.dh - 1) as f32);
        let fx = ((x as f32 + 0.5) / f - 0.5).clamp(0.0, (self.dw - 1) as f32);
        let (x0, y0, tx, ty) = (fx as usize, fy as usize, fx.fract(), fy.fract());
        let (x1, y1) = ((x0 + 1).min(self.dw - 1), (y0 + 1).min(self.dh - 1));
        let mut c = [0f32; 3];
        for k in 0..3 {
            let top = self.px[y0 * self.dw + x0][k] * (1.0 - tx) + self.px[y0 * self.dw + x1][k] * tx;
            let bot = self.px[y1 * self.dw + x0][k] * (1.0 - tx) + self.px[y1 * self.dw + x1][k] * tx;
            c[k] = top * (1.0 - ty) + bot * ty;
        }
        0xFF00_0000 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
    }
}

fn blur_region(out: &[u32], w: u32, h: u32, s: &Surface, rect: (i32, i32, i32, i32)) -> Blurred {
    let (sw, sh) = ((rect.2 - rect.0) as usize, (rect.3 - rect.1) as usize);
    // blur a downsampled backdrop (f x f box average) and upsample bilinearly: same look, ~f^2 less work
    let f = if s.blur >= 12 { 4usize } else { 1 };
    let (dw, dh) = ((sw + f - 1) / f, (sh + f - 1) / f);
    let mut px: Vec<[f32; 3]> = Vec::with_capacity(dw * dh);
    for by in 0..dh {
        for bx in 0..dw {
            let mut acc = [0f32; 3];
            let mut n = 0f32;
            for oy in 0..f {
                for ox in 0..f {
                    let (sx, sy) = ((bx * f + ox).min(sw - 1) as i32 + rect.0, (by * f + oy).min(sh - 1) as i32 + rect.1);
                    let (x, y) = ((sx + s.x).clamp(0, w as i32 - 1), (sy + s.y).clamp(0, h as i32 - 1));
                    let c = out[(y as u32 * w + x as u32) as usize];
                    acc[0] += ch(c, 16); acc[1] += ch(c, 8); acc[2] += ch(c, 0);
                    n += 1.0;
                }
            }
            px.push([acc[0] / n, acc[1] / n, acc[2] / n]);
        }
    }
    let r = ((s.blur as usize / 3) / f).max(1);
    for _ in 0..3 {
        box_pass(&mut px, dw, dh, r, true);
        box_pass(&mut px, dw, dh, r, false);
    }
    Blurred { px, dw, dh, f }
}

fn box_pass(px: &mut [[f32; 3]], w: usize, h: usize, r: usize, horiz: bool) {
    let (n, m) = if horiz { (w, h) } else { (h, w) };
    let idx = |line: usize, i: usize| if horiz { line * w + i } else { i * w + line };
    let mut tmp = vec![[0f32; 3]; n];
    for line in 0..m {
        for c in 0..3 {
            let mut sum = 0.0;
            for i in 0..=r.min(n - 1) { sum += px[idx(line, i)][c]; }
            let mut cnt = r.min(n - 1) + 1;
            for i in 0..n {
                tmp[i][c] = sum / cnt as f32;
                if i + r + 1 < n { sum += px[idx(line, i + r + 1)][c]; cnt += 1; }
                if i >= r { sum -= px[idx(line, i - r)][c]; cnt -= 1; }
            }
        }
        for i in 0..n { px[idx(line, i)] = tmp[i]; }
    }
}

/// Frosted look: slight saturation boost and brightening, plus faint grain.
fn glass_adjust(c: u32, x: i32, y: i32) -> u32 {
    let (r, g, b) = (ch(c, 16), ch(c, 8), ch(c, 0));
    let lum = 0.299 * r + 0.587 * g + 0.114 * b;
    let grain = ((((x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663)) & 0xff) as f32 / 255.0 - 0.5) * 3.0;
    let f = |v: f32| (lum + (v - lum) * 1.25 + 6.0 + grain).clamp(0.0, 255.0) as u32;
    0xFF00_0000 | f(r) << 16 | f(g) << 8 | f(b)
}

/// Default procedural wallpaper: soft aurora gradient (deterministic).
pub fn wallpaper(x: u32, y: u32, w: u32, h: u32) -> u32 {
    let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
    let stops: [(f32, [f32; 3]); 3] = [(0.0, [96.0, 128.0, 220.0]), (0.55, [168.0, 128.0, 226.0]), (1.0, [240.0, 156.0, 190.0])];
    let t = (u * 0.55 + v * 0.45).clamp(0.0, 1.0);
    let (a, b) = if t < stops[1].0 { (stops[0], stops[1]) } else { (stops[1], stops[2]) };
    let k = ((t - a.0) / (b.0 - a.0)).clamp(0.0, 1.0);
    let mut c = [0f32; 3];
    for i in 0..3 { c[i] = a.1[i] * (1.0 - k) + b.1[i] * k; }
    for (cx, cy, rad, col) in [(0.22f32, 0.78f32, 0.35f32, [60.0f32, 210.0, 230.0]), (0.82, 0.22, 0.30, [255.0, 200.0, 120.0])] {
        let d = (((u - cx) * w as f32).powi(2) + ((v - cy) * h as f32).powi(2)).sqrt() / (rad * w as f32);
        let wgt = (1.0 - d).clamp(0.0, 1.0).powi(2) * 0.55;
        for i in 0..3 { c[i] = c[i] * (1.0 - wgt) + col[i] * wgt; }
    }
    0xFF00_0000 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raise_reorders_compose() {
        let mut b = SoftBackend::new();
        let (a, c) = (b.create_surface(4, 4), b.create_surface(4, 4));
        b.fill_rect(a, 0, 0, 4, 4, 0xFF11_1111);
        b.fill_rect(c, 0, 0, 4, 4, 0xFF22_2222);
        b.place(a, 0, 0, true);
        b.place(c, 0, 0, true);
        assert_eq!(b.compose(4, 4, 0)[0], 0xFF22_2222);
        b.raise(a);
        assert_eq!(b.compose(4, 4, 0)[0], 0xFF11_1111);
    }

    #[test]
    fn fill_and_compose() {
        let mut b = SoftBackend::new();
        let s = b.create_surface(10, 10);
        b.fill_rect(s, 2, 2, 4, 4, 0xFFFF_0000);
        b.place(s, 5, 5, true);
        assert_eq!(b.pixel(s, 3, 3), Some(0xFFFF_0000));
        let d = b.compose(20, 20, 0xFF00_00FF);
        assert_eq!(d[(8 * 20 + 8) as usize], 0xFFFF_0000);
        assert_eq!(d[0], 0xFF00_00FF);
    }
}
