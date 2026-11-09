//! Display backend abstraction: every window is its own surface; a shell/compositor decides
//! where surfaces sit. `SoftBackend` is the software implementation (Canvas 2D presents its
//! composed framebuffer; a WebGL compositor can later upload surfaces as textures).

pub type SurfaceId = u32;

pub trait Backend {
    fn create_surface(&mut self, w: u32, h: u32) -> SurfaceId;
    fn resize_surface(&mut self, id: SurfaceId, w: u32, h: u32);
    fn destroy_surface(&mut self, id: SurfaceId);
    /// Colors are 0xAARRGGBB.
    fn fill_rect(&mut self, id: SurfaceId, x: i32, y: i32, w: i32, h: i32, argb: u32);
    fn draw_text(&mut self, id: SurfaceId, x: i32, y: i32, text: &str, argb: u32);
    fn blit(&mut self, id: SurfaceId, x: i32, y: i32, w: u32, h: u32, pixels: &[u32], stride: u32);
    /// Shell-side placement of a surface on the desktop.
    fn place(&mut self, id: SurfaceId, x: i32, y: i32, visible: bool);
    /// Flush to the screen (no-op for headless).
    fn present(&mut self);
}

pub struct Surface {
    pub w: u32,
    pub h: u32,
    pub pix: Vec<u32>,
    pub x: i32,
    pub y: i32,
    pub visible: bool,
}

#[derive(Default)]
pub struct SoftBackend {
    surfaces: Vec<Option<Surface>>,
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
    /// Compose all visible surfaces (in creation order = z-order) onto a w*h desktop.
    pub fn compose(&self, w: u32, h: u32, bg: u32) -> Vec<u32> {
        let mut out = vec![bg; (w * h) as usize];
        for s in self.surfaces.iter().flatten().filter(|s| s.visible) {
            for sy in 0..s.h as i32 {
                let dy = sy + s.y;
                if dy < 0 || dy >= h as i32 { continue; }
                for sx in 0..s.w as i32 {
                    let dx = sx + s.x;
                    if dx >= 0 && dx < w as i32 { out[(dy as u32 * w + dx as u32) as usize] = s.pix[(sy as u32 * s.w + sx as u32) as usize]; }
                }
            }
        }
        out
    }
    fn get(&mut self, id: SurfaceId) -> Option<&mut Surface> { self.surfaces.get_mut((id as usize).checked_sub(1)?)?.as_mut() }
}

impl Backend for SoftBackend {
    fn create_surface(&mut self, w: u32, h: u32) -> SurfaceId {
        self.surfaces.push(Some(Surface { w, h, pix: vec![0xFF00_0000; (w * h) as usize], x: 0, y: 0, visible: false }));
        self.surfaces.len() as SurfaceId
    }
    fn resize_surface(&mut self, id: SurfaceId, w: u32, h: u32) {
        if let Some(s) = self.get(id) {
            let mut pix = vec![0xFF00_0000; (w * h) as usize];
            for y in 0..h.min(s.h) { for x in 0..w.min(s.w) { pix[(y * w + x) as usize] = s.pix[(y * s.w + x) as usize]; } }
            s.w = w; s.h = h; s.pix = pix;
        }
    }
    fn destroy_surface(&mut self, id: SurfaceId) {
        if let Some(slot) = self.surfaces.get_mut((id as usize).wrapping_sub(1)) { *slot = None; }
    }
    fn fill_rect(&mut self, id: SurfaceId, x: i32, y: i32, w: i32, h: i32, argb: u32) {
        let Some(s) = self.get(id) else { return };
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x + w).min(s.w as i32), (y + h).min(s.h as i32));
        for yy in y0..y1 { for xx in x0..x1 { s.pix[(yy as u32 * s.w + xx as u32) as usize] = argb; } }
    }
    /// 8x8 bitmap font (font8x8, public domain), 8px advance. Non-ASCII renders as '?'.
    fn draw_text(&mut self, id: SurfaceId, x: i32, y: i32, text: &str, argb: u32) {
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
        if let Some(s) = self.get(id) { s.x = x; s.y = y; s.visible = visible; }
    }
    fn present(&mut self) { self.presents += 1; }
}

#[cfg(test)]
mod tests {
    use super::*;
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
