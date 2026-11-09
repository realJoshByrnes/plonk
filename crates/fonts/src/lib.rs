//! Bundled UI fonts (Inter, SIL OFL; see assets/LICENSE-Inter.txt) and glyph rasterization.
//! All GDI text metrics and text drawing come from here so layout matches what is drawn.
use fontdue::{Font, FontSettings};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const REGULAR: &[u8] = include_bytes!("../assets/Inter-Regular.otf");
const SEMIBOLD: &[u8] = include_bytes!("../assets/Inter-SemiBold.otf");

/// A rasterized glyph: `coverage` is width*height, top row first.
pub struct Glyph {
    pub xmin: i32,
    /// Offset of the bitmap's bottom edge above the baseline (negative = below).
    pub ymin: i32,
    pub width: usize,
    pub height: usize,
    pub advance: f32,
    pub coverage: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct VMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
}

pub struct Fonts {
    regular: Font,
    bold: Font,
    cache: RefCell<HashMap<(char, u32, bool), Rc<Glyph>>>,
}

impl Fonts {
    fn new() -> Self {
        let s = FontSettings::default();
        Fonts {
            regular: Font::from_bytes(REGULAR, s).expect("Inter Regular"),
            bold: Font::from_bytes(SEMIBOLD, FontSettings::default()).expect("Inter SemiBold"),
            cache: RefCell::new(HashMap::new()),
        }
    }
    fn face(&self, bold: bool) -> &Font { if bold { &self.bold } else { &self.regular } }

    pub fn glyph(&self, c: char, px: f32, bold: bool) -> Rc<Glyph> {
        let key = (c, (px * 4.0) as u32, bold);
        if let Some(g) = self.cache.borrow().get(&key) { return g.clone(); }
        let (m, coverage) = self.face(bold).rasterize(c, px);
        let g = Rc::new(Glyph { xmin: m.xmin, ymin: m.ymin, width: m.width, height: m.height, advance: m.advance_width, coverage });
        self.cache.borrow_mut().insert(key, g.clone());
        g
    }

    pub fn vmetrics(&self, px: f32, bold: bool) -> VMetrics {
        let m = self.face(bold).horizontal_line_metrics(px);
        match m {
            Some(l) => VMetrics { ascent: l.ascent, descent: -l.descent, line_gap: l.line_gap },
            None => VMetrics { ascent: px * 0.93, descent: px * 0.24, line_gap: 0.0 },
        }
    }

    pub fn advance(&self, c: char, px: f32, bold: bool) -> f32 { self.face(bold).metrics(c, px).advance_width }

    /// Width of a string including pair kerning.
    pub fn text_width(&self, s: &str, px: f32, bold: bool) -> f32 {
        let f = self.face(bold);
        let mut w = 0.0;
        let mut prev: Option<char> = None;
        for c in s.chars() {
            if let Some(p) = prev { w += f.horizontal_kern(p, c, px).unwrap_or(0.0); }
            w += f.metrics(c, px).advance_width;
            prev = Some(c);
        }
        w
    }
}

thread_local! { static FONTS: Fonts = Fonts::new(); }

/// Run `f` with the process-wide (per-thread) font set.
pub fn with<R>(f: impl FnOnce(&Fonts) -> R) -> R { FONTS.with(f) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metrics_and_raster() {
        with(|f| {
            let (v, w) = (f.vmetrics(12.0, false), f.text_width("Hello", 12.0, false));
            assert!(v.ascent > 8.0 && v.ascent < 14.0 && v.descent > 1.0, "{v:?}");
            assert!(w > 20.0 && w < 40.0, "width {w}");
            assert!(f.text_width("Hello", 12.0, true) > w, "semibold is wider");
            let g = f.glyph('A', 12.0, false);
            assert!(g.coverage.iter().any(|c| *c > 128) && g.width > 4);
        });
    }
}
