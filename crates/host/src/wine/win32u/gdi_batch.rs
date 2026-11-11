//! GDI drawing: paths, regions, bitmaps/DIBs, brushes, blits, world transforms, palettes, fonts resources, printing stubs (win32u calls filled in systematically; see docs/WIN32U_COVERAGE.txt).
//! Dispatch note: this handler runs before the others so that, while a path is open on a DC, the drawing calls (MoveTo/LineTo/Rectangle/...) are recorded into the path instead of being drawn.
use super::gdi::{rop_apply, Bitmap};
use super::*;
use std::collections::BTreeMap;

const PT_MOVETO: u8 = 6;
const PT_LINETO: u8 = 2;
const PT_BEZIERTO: u8 = 4;
const PT_CLOSE: u8 = 1;
const GDI_ERROR: u32 = 0xFFFF_FFFF;

type Pt = (i32, i32);

/// A GDI path in device coordinates (beziers kept as 3-point groups until flattened).
#[derive(Default, Clone)]
pub struct Path { open: bool, pts: Vec<Pt>, ty: Vec<u8>, new_stroke: bool }

impl Path {
    fn start(&mut self, from: Pt) {
        let closed_last = self.ty.last().is_some_and(|t| t & PT_CLOSE != 0);
        if self.new_stroke || self.pts.is_empty() || closed_last { self.pts.push(from); self.ty.push(PT_MOVETO); self.new_stroke = false; }
    }
    fn line(&mut self, from: Pt, to: Pt) { self.start(from); self.pts.push(to); self.ty.push(PT_LINETO); }
    fn bezier(&mut self, from: Pt, c: [Pt; 3]) { self.start(from); for p in c { self.pts.push(p); self.ty.push(PT_BEZIERTO); } }
    fn close(&mut self) { if let Some(t) = self.ty.last_mut() { *t |= PT_CLOSE; } self.new_stroke = true; }
    /// Flattened figures: (points, closed).
    fn figures(&self) -> Vec<(Vec<(f64, f64)>, bool)> {
        let mut out: Vec<(Vec<(f64, f64)>, bool)> = Vec::new();
        let mut i = 0;
        while i < self.pts.len() {
            let (p, t) = (self.pts[i], self.ty[i]);
            let f = |q: Pt| (q.0 as f64, q.1 as f64);
            match t & !PT_CLOSE {
                PT_MOVETO => out.push((vec![f(p)], false)),
                PT_LINETO => { if let Some(c) = out.last_mut() { c.0.push(f(p)); } }
                _ => {
                    if i + 2 < self.pts.len() {
                        if let Some(c) = out.last_mut() {
                            let p0 = *c.0.last().unwrap();
                            let (p1, p2, p3) = (f(p), f(self.pts[i + 1]), f(self.pts[i + 2]));
                            for s in 1..=16 {
                                let u = s as f64 / 16.0; let v = 1.0 - u;
                                let b = |a: f64, b: f64, c_: f64, d: f64| v * v * v * a + 3.0 * v * v * u * b + 3.0 * v * u * u * c_ + u * u * u * d;
                                c.0.push((b(p0.0, p1.0, p2.0, p3.0), b(p0.1, p1.1, p2.1, p3.1)));
                            }
                        }
                        i += 2;
                    }
                }
            }
            if self.ty[i.min(self.ty.len() - 1)] & PT_CLOSE != 0 { if let Some(c) = out.last_mut() { c.1 = true; } }
            i += 1;
        }
        out
    }
    fn bbox(&self) -> Rect {
        let mut r = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
        for (f, _) in self.figures() { for (x, y) in f { r = [r[0].min(x.floor() as i32), r[1].min(y.floor() as i32), r[2].max(x.ceil() as i32), r[3].max(y.ceil() as i32)]; } }
        if r[0] == i32::MAX { [0; 4] } else { r }
    }
}

/// Per-area state (add fields here; `Extra` owns one of these).
#[derive(Default)]
pub struct State {
    paths: BTreeMap<u32, Path>,
    xf: BTreeMap<u32, [f32; 6]>,
    bounds: BTreeMap<u32, (Rect, bool)>,
    bm_dim: BTreeMap<u32, (i32, i32)>,
    pals: BTreeMap<u32, Vec<u32>>,
    gamma: BTreeMap<u32, Vec<u8>>,
    cadj: BTreeMap<u32, Vec<u8>>,
    syspal: u32,
    fake_handle: u32,
}

fn combine(x1: &[f32; 6], x2: &[f32; 6]) -> [f32; 6] {
    [x1[0] * x2[0] + x1[1] * x2[2], x1[0] * x2[1] + x1[1] * x2[3], x1[2] * x2[0] + x1[3] * x2[2], x1[2] * x2[1] + x1[3] * x2[3],
     x1[4] * x2[0] + x1[5] * x2[2] + x2[4], x1[4] * x2[1] + x1[5] * x2[3] + x2[5]]
}
const IDENT: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
fn rect_empty(r: Rect) -> bool { r[2] <= r[0] || r[3] <= r[1] }

impl WineProc {
    fn gb_attr(&self, hdc: u32) -> u32 { self.r32(self.gdi_table + (hdc & 0xffff) * 24 + 16).unwrap_or(0) }
    fn gb_pos(&self, hdc: u32) -> Pt { self.wm.dcs.get(&hdc).map_or((0, 0), |d| d.pos) }
    fn gb_set_pos(&mut self, hdc: u32, p: Pt) { if let Some(d) = self.wm.dcs.get_mut(&hdc) { d.pos = p; } }
    fn gb_path_open(&self, hdc: u32) -> bool { self.wm.extra.gdi_b.paths.get(&hdc).is_some_and(|p| p.open) }
    fn gb_with_path(&mut self, hdc: u32, f: impl FnOnce(&mut Path)) {
        if let Some(p) = self.wm.extra.gdi_b.paths.get_mut(&hdc) { f(p); }
    }
    fn gb_read_f(&self, p: u32) -> f32 { f32::from_bits(self.r32(p).unwrap_or(0)) }

    /// Polyline start -> pts, into the open path or onto the DC with the current pen.
    fn gb_lines(&mut self, hdc: u32, start: Pt, pts: &[Pt]) {
        if self.gb_path_open(hdc) {
            let mut cur = start;
            self.gb_with_path(hdc, |p| for q in pts { p.line(cur, *q); cur = *q; });
        } else {
            let pen = self.wm.dcs.get(&hdc).map_or(0, |d| d.pen);
            let mut cur = start;
            for q in pts { self.dc_line(hdc, cur, *q, pen); cur = *q; }
        }
    }
    fn gb_bezier(&mut self, hdc: u32, start: Pt, c: [Pt; 3]) {
        if self.gb_path_open(hdc) { self.gb_with_path(hdc, |p| p.bezier(start, c)); return; }
        let mut v = Vec::new();
        for s in 1..=16 {
            let (u, w) = (s as f64 / 16.0, 1.0 - s as f64 / 16.0);
            let b = |a: i32, b: i32, c_: i32, d: i32| (w * w * w * a as f64 + 3.0 * w * w * u * b as f64 + 3.0 * w * u * u * c_ as f64 + u * u * u * d as f64).round() as i32;
            v.push((b(start.0, c[0].0, c[1].0, c[2].0), b(start.1, c[0].1, c[1].1, c[2].1)));
        }
        self.gb_lines(hdc, start, &v);
    }

    fn gb_fill_figs(&mut self, hdc: u32, figs: &[(Vec<(f64, f64)>, bool)], winding: bool, color: u32) {
        let (mut y0, mut y1) = (i32::MAX, i32::MIN);
        for (f, _) in figs { for p in f { y0 = y0.min(p.1.floor() as i32); y1 = y1.max(p.1.ceil() as i32); } }
        for y in y0..y1 {
            let yc = y as f64 + 0.5;
            let mut xs: Vec<(f64, i32)> = Vec::new();
            for (f, _) in figs {
                let n = f.len();
                if n < 2 { continue; }
                for i in 0..n {
                    let (p, q) = (f[i], f[(i + 1) % n]);
                    if (p.1 <= yc) != (q.1 <= yc) { xs.push((p.0 + (yc - p.1) * (q.0 - p.0) / (q.1 - p.1), if q.1 > p.1 { 1 } else { -1 })); }
                }
            }
            xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let mut spans = Vec::new();
            if winding {
                let mut w = 0;
                for i in 0..xs.len().saturating_sub(1) { w += xs[i].1; if w != 0 { spans.push((xs[i].0, xs[i + 1].0)); } }
            } else {
                for c in xs.chunks(2) { if c.len() == 2 { spans.push((c[0].0, c[1].0)); } }
            }
            for (a, b) in spans {
                let (x0, x1) = ((a - 0.5).ceil() as i32, (b - 0.5).ceil() as i32);
                if x1 > x0 { self.dc_fill(hdc, [x0, y, x1, y + 1], color); }
            }
        }
    }

    fn gb_fill_color(&self, hdc: u32) -> Option<u32> {
        let b = self.wm.dcs.get(&hdc)?.brush;
        if matches!(self.wm.gdi.get(&b), Some(Gdi::NullBrush)) { None } else { Some(self.brush_argb(b)) }
    }

    /// Take a closed path (FillPath & co); None when there is no closed path.
    fn gb_take_closed(&mut self, hdc: u32) -> Option<Path> {
        if self.wm.extra.gdi_b.paths.get(&hdc).is_some_and(|p| !p.open) { self.wm.extra.gdi_b.paths.remove(&hdc) } else { None }
    }

    fn gb_arc_points(&self, hdc: u32, a: &[u32]) -> (Vec<Pt>, Pt) {
        // a: type, hdc, l, t, r, b, xs, ys, xe, ye
        let r = self.lp_rect(hdc, [a[2] as i32, a[3] as i32, a[4] as i32, a[5] as i32]);
        let (s, e) = (self.lp(hdc, a[6] as i32, a[7] as i32), self.lp(hdc, a[8] as i32, a[9] as i32));
        let (cx, cy) = ((r[0] + r[2]) as f64 / 2.0, (r[1] + r[3]) as f64 / 2.0);
        let (rx, ry) = (((r[2] - r[0]) as f64 / 2.0).max(0.5), ((r[3] - r[1]) as f64 / 2.0).max(0.5));
        let ang = |p: Pt| (-(p.1 as f64 - cy) / ry).atan2((p.0 as f64 - cx) / rx);
        let (a0, a1) = (ang(s), ang(e));
        let at = self.gb_attr(hdc);
        let cw = at != 0 && self.r32(at + 40).unwrap_or(1) == 2;
        let tau = std::f64::consts::TAU;
        let mut sweep = a1 - a0;
        if s == e { sweep = if cw { -tau } else { tau }; }
        else if !cw && sweep <= 0.0 { sweep += tau } else if cw && sweep >= 0.0 { sweep -= tau }
        let n = ((sweep.abs() / (tau / 72.0)).ceil() as usize).max(4);
        let pts = (0..=n).map(|i| { let t = a0 + sweep * i as f64 / n as f64; ((cx + rx * t.cos()).round() as i32, (cy - ry * t.sin()).round() as i32) }).collect();
        (pts, (cx.round() as i32, cy.round() as i32))
    }

    /// While a path is open on the DC, drawing calls are recorded instead of drawn.
    fn gb_record(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        let hdc = if name == "NtGdiArcInternal" { a[1] } else { a[0] };
        let pos = self.gb_pos(hdc);
        match name {
            "NtGdiMoveTo" => {
                let np = self.lp(hdc, a[1] as i32, a[2] as i32);
                self.gb_set_pos(hdc, np);
                if a[3] != 0 { self.w32(a[3], pos.0 as u32)?; self.w32(a[3] + 4, pos.1 as u32)?; }
                self.gb_with_path(hdc, |p| p.new_stroke = true);
            }
            "NtGdiLineTo" => { let to = self.lp(hdc, a[1] as i32, a[2] as i32); self.gb_lines(hdc, pos, &[to]); self.gb_set_pos(hdc, to); }
            "NtGdiRectangle" | "NtGdiRoundRect" => {
                let r = self.lp_rect(hdc, [a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32]);
                self.gb_with_path(hdc, |p| { p.new_stroke = true; p.line((r[2], r[1]), (r[0], r[1])); p.pts.push((r[0], r[3])); p.ty.push(PT_LINETO); p.pts.push((r[2], r[3])); p.ty.push(PT_LINETO); p.close(); });
            }
            "NtGdiEllipse" => {
                let r = self.lp_rect(hdc, [a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32]);
                let (cx, cy, rx, ry) = ((r[0] + r[2]) as f64 / 2.0, (r[1] + r[3]) as f64 / 2.0, (r[2] - r[0]) as f64 / 2.0, (r[3] - r[1]) as f64 / 2.0);
                let pts: Vec<Pt> = (0..72).map(|i| { let t = i as f64 * std::f64::consts::TAU / 72.0; ((cx + rx * t.cos()).round() as i32, (cy - ry * t.sin()).round() as i32) }).collect();
                self.gb_with_path(hdc, |p| { p.new_stroke = true; for w in pts.windows(2) { p.line(w[0], w[1]); } p.line(pts[71], pts[0]); p.close(); });
            }
            "NtGdiArcInternal" => {
                let (pts, c) = self.gb_arc_points(hdc, a);
                let ty = a[0];
                self.gb_with_path(hdc, |p| {
                    p.new_stroke = ty != 1;
                    if ty == 3 { p.new_stroke = true; p.line(c, pts[0]); } else if ty == 1 { p.line(pos, pts[0]); }
                    p.new_stroke = ty != 1 && ty != 3;
                    for w in pts.windows(2) { p.line(w[0], w[1]); }
                    if ty >= 2 { p.close(); }
                });
                if ty == 1 { self.gb_set_pos(hdc, *pts.last().unwrap()); }
            }
            "NtGdiPolyPolyDraw" => {
                let (pts, counts, n, func) = (a[1], a[2], a[3], a[4]);
                let mut off = 0u32;
                for i in 0..n {
                    let cnt = if func >= 3 { n } else { self.r32(counts + i * 4)? };
                    let mut v = Vec::new();
                    for k in 0..cnt { let (x, y) = (self.r32(pts + (off + k) * 8)? as i32, self.r32(pts + (off + k) * 8 + 4)? as i32); v.push(self.lp(hdc, x, y)); }
                    match func {
                        1 if !v.is_empty() => self.gb_with_path(hdc, |p| { p.new_stroke = true; for w in v.windows(2) { p.line(w[0], w[1]); } if v.len() == 1 { p.start(v[0]); } p.close(); }),
                        2 if !v.is_empty() => self.gb_with_path(hdc, |p| { p.new_stroke = true; for w in v.windows(2) { p.line(w[0], w[1]); } }),
                        3 | 4 => {
                            let mut cur = if func == 3 && !v.is_empty() { v.remove(0) } else { pos };
                            for c in v.chunks_exact(3) { self.gb_bezier(hdc, cur, [c[0], c[1], c[2]]); cur = c[2]; }
                            if func == 4 { self.gb_set_pos(hdc, cur); }
                        }
                        5 => { self.gb_lines(hdc, pos, &v); if let Some(l) = v.last() { self.gb_set_pos(hdc, *l); } }
                        _ => {}
                    }
                    off += cnt;
                    if func >= 3 { break; }
                }
            }
            _ => return Ok(None),
        }
        Ok(Some(1))
    }

    fn gb_dc_bounds(&self, hdc: u32) -> Rect {
        let Some(dc) = self.wm.dcs.get(&hdc) else { return [0; 4] };
        if dc.bitmap != 0 { if let Some(b) = self.wm.bitmaps.get(&dc.bitmap) { return [0, 0, b.w, b.h]; } }
        self.wm.client_rect(dc.hwnd)
    }

    fn gb_bitmap_layout(b: &Bitmap) -> u32 { if b.bpp == 1 { ((b.w as u32 + 15) / 16) * 2 } else if b.dib { b.stride } else { b.w as u32 * 4 } }

    /// Ok(None) = not handled here (falls through to the next handler / the lenient stub).
    pub(super) fn gdi_batch_call(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        if !self.wm.extra.gdi_b.paths.is_empty()
            && matches!(name, "NtGdiMoveTo" | "NtGdiLineTo" | "NtGdiRectangle" | "NtGdiRoundRect" | "NtGdiEllipse" | "NtGdiArcInternal" | "NtGdiPolyPolyDraw")
            && self.gb_path_open(if name == "NtGdiArcInternal" { a[1] } else { a[0] })
        {
            return self.gb_record(name, a);
        }
        if matches!(name, "NtGdiMoveTo" | "NtGdiLineTo") {
            // gdi32 reads the current position from DC_ATTR.cur_pos (logical); the drawing itself stays with the next handler
            let at = self.gb_attr(a[0]);
            if at != 0 { let _ = self.w32(at + 28, a[1]); let _ = self.w32(at + 32, a[2]); }
            return Ok(None);
        }
        Ok(Some(match name {
            // ---------------- paths ----------------
            "NtGdiBeginPath" => { if !self.wm.dcs.contains_key(&a[0]) { return Ok(Some(0)); } self.wm.extra.gdi_b.paths.insert(a[0], Path { open: true, ..Default::default() }); 1 }
            "NtGdiEndPath" => match self.wm.extra.gdi_b.paths.get_mut(&a[0]) { Some(p) if p.open => { p.open = false; 1 } _ => 0 },
            "NtGdiAbortPath" => { self.wm.extra.gdi_b.paths.remove(&a[0]); 1 }
            "NtGdiCloseFigure" => match self.wm.extra.gdi_b.paths.get_mut(&a[0]) { Some(p) if p.open => { p.close(); 1 } _ => 0 },
            "NtGdiFillPath" | "NtGdiStrokePath" | "NtGdiStrokeAndFillPath" => {
                let Some(p) = self.gb_take_closed(a[0]) else { return Ok(Some(0)) };
                let figs = p.figures();
                if name != "NtGdiStrokePath" {
                    let at = self.gb_attr(a[0]);
                    let winding = at != 0 && self.mem.r16(at + 52).unwrap_or(1) == 2;
                    if let Some(c) = self.gb_fill_color(a[0]) { self.gb_fill_figs(a[0], &figs, winding, c); }
                }
                if name != "NtGdiFillPath" {
                    let pen = self.wm.dcs.get(&a[0]).map_or(0, |d| d.pen);
                    for (f, closed) in &figs {
                        let r = |q: &(f64, f64)| (q.0.round() as i32, q.1.round() as i32);
                        for w in f.windows(2) { self.dc_line(a[0], r(&w[0]), r(&w[1]), pen); }
                        if *closed && f.len() > 1 { self.dc_line(a[0], r(f.last().unwrap()), r(&f[0]), pen); }
                    }
                }
                1
            }
            "NtGdiWidenPath" => self.wm.extra.gdi_b.paths.get(&a[0]).is_some_and(|p| !p.open) as u32,
            "NtGdiFlattenPath" => {
                let Some(p) = self.wm.extra.gdi_b.paths.get_mut(&a[0]) else { return Ok(Some(0)) };
                if p.open { return Ok(Some(0)); }
                let figs = p.figures();
                p.pts.clear(); p.ty.clear();
                for (f, closed) in figs {
                    for (i, q) in f.iter().enumerate() { p.pts.push((q.0.round() as i32, q.1.round() as i32)); p.ty.push(if i == 0 { PT_MOVETO } else { PT_LINETO }); }
                    if closed { p.close(); }
                }
                1
            }
            "NtGdiGetPath" => {
                let (hdc, pts, types, size) = (a[0], a[1], a[2], a[3] as i32);
                let Some(p) = self.wm.extra.gdi_b.paths.get(&hdc).filter(|p| !p.open).cloned() else { return Ok(Some(GDI_ERROR)) };
                let n = p.pts.len() as i32;
                if size == 0 { return Ok(Some(n as u32)); }
                if size < n { return Ok(Some(GDI_ERROR)); }
                for (i, q) in p.pts.iter().enumerate() {
                    let (x, y) = self.dp(hdc, q.0, q.1);
                    self.w32(pts + i as u32 * 8, x as u32)?; self.w32(pts + i as u32 * 8 + 4, y as u32)?;
                    self.mem.w8(types + i as u32, p.ty[i]).map_err(|_| "getpath")?;
                }
                n as u32
            }
            "NtGdiPathToRegion" => {
                let Some(p) = self.gb_take_closed(a[0]) else { return Ok(Some(0)) };
                self.gdi_alloc(0x040000, Gdi::Region(p.bbox()))
            }
            "NtGdiSelectClipPath" => {
                let Some(p) = self.gb_take_closed(a[0]) else { return Ok(Some(0)) };
                let b = p.bbox();
                if let Some(dc) = self.wm.dcs.get_mut(&a[0]) {
                    dc.clip = Some(match (a[1], dc.clip) { (1, Some(c)) => [c[0].max(b[0]), c[1].max(b[1]), c[2].min(b[2]), c[3].min(b[3])], _ => b });
                }
                1
            }
            "NtGdiPolyDraw" => {
                let (hdc, pts, types, n) = (a[0], a[1], a[2], a[3]);
                let mut pos = self.gb_pos(hdc);
                let mut start = pos;
                let mut i = 0;
                let mut new_fig = true;
                while i < n {
                    let t = self.mem.r8(types + i).map_err(|_| "polydraw")?;
                    let rd = |s: &Self, k: u32| -> Res<Pt> { let (x, y) = (s.r32(pts + k * 8)? as i32, s.r32(pts + k * 8 + 4)? as i32); Ok(s.lp(hdc, x, y)) };
                    match t & !PT_CLOSE {
                        PT_MOVETO => { pos = rd(self, i)?; start = pos; new_fig = true; self.gb_with_path(hdc, |p| p.new_stroke = true); }
                        PT_LINETO => { let q = rd(self, i)?; self.gb_lines(hdc, pos, &[q]); pos = q; }
                        PT_BEZIERTO if i + 2 < n => { let c = [rd(self, i)?, rd(self, i + 1)?, rd(self, i + 2)?]; self.gb_bezier(hdc, pos, c); pos = c[2]; i += 2; }
                        _ => return Ok(Some(0)),
                    }
                    if t & PT_CLOSE != 0 && !new_fig {
                        if self.gb_path_open(hdc) { self.gb_with_path(hdc, |p| p.close()); } else { self.gb_lines(hdc, pos, &[start]); }
                        pos = start;
                    }
                    if t & !PT_CLOSE != PT_MOVETO { new_fig = false; }
                    i += 1;
                }
                self.gb_set_pos(hdc, pos);
                1
            }
            "NtGdiAngleArc" => {
                let hdc = a[0];
                let (c, r) = (self.lp(hdc, a[1] as i32, a[2] as i32), a[3] as f64);
                let (s, sw) = (f32::from_bits(a[4]) as f64, f32::from_bits(a[5]) as f64);
                let n = (sw.abs().ceil() as usize).clamp(2, 720);
                let pt = |deg: f64| { let t = deg.to_radians(); ((c.0 as f64 + r * t.cos()).round() as i32, (c.1 as f64 - r * t.sin()).round() as i32) };
                let v: Vec<Pt> = (0..=n).map(|i| pt(s + sw * i as f64 / n as f64)).collect();
                let pos = self.gb_pos(hdc);
                self.gb_lines(hdc, pos, &v);
                self.gb_set_pos(hdc, *v.last().unwrap());
                1
            }

            // ---------------- world transform / mapping ----------------
            "NtGdiModifyWorldTransform" => {
                let (hdc, xp, mode) = (a[0], a[1], a[2]);
                if xp == 0 && mode != 1 { return Ok(Some(0)); }
                if !self.wm.dcs.contains_key(&hdc) { return Ok(Some(0)); }
                let mut x = [0f32; 6];
                if xp != 0 { for (i, v) in x.iter_mut().enumerate() { *v = self.gb_read_f(xp + i as u32 * 4); } }
                let cur = self.wm.extra.gdi_b.xf.get(&hdc).copied().unwrap_or(IDENT);
                let at = self.gb_attr(hdc);
                let new = match mode {
                    1 => IDENT,
                    2 => combine(&x, &cur),
                    3 => combine(&cur, &x),
                    4 => { let adv = at != 0 && self.r32(at + 36).unwrap_or(1) == 2; if !adv || x[0] * x[3] == x[1] * x[2] { return Ok(Some(0)); } x }
                    _ => return Ok(Some(0)),
                };
                self.wm.extra.gdi_b.xf.insert(hdc, new);
                1
            }
            "NtGdiGetTransform" => {
                let (hdc, which, out) = (a[0], a[1], a[2]);
                if !self.wm.dcs.contains_key(&hdc) { return Ok(Some(0)); }
                let at = self.gb_attr(hdc);
                let g = |o: u32| if at != 0 { self.r32(at + o).unwrap_or(0) as i32 } else { 0 };
                let (sx, sy) = (if g(108) != 0 { g(124) as f32 / g(108) as f32 } else { 1.0 }, if g(112) != 0 { g(128) as f32 / g(112) as f32 } else { 1.0 });
                let w2v = [sx, 0.0, 0.0, sy, g(116) as f32 - g(100) as f32 * sx, g(120) as f32 - g(104) as f32 * sy];
                let w2w = self.wm.extra.gdi_b.xf.get(&hdc).copied().unwrap_or(IDENT);
                let both = combine(&w2w, &w2v);
                let x = match which {
                    0x203 => w2w,
                    0x304 => w2v,
                    0x204 => both,
                    0x402 => {
                        let det = both[0] * both[3] - both[1] * both[2];
                        if det == 0.0 { return Ok(Some(0)); }
                        [both[3] / det, -both[1] / det, -both[2] / det, both[0] / det, (both[2] * both[5] - both[3] * both[4]) / det, (both[1] * both[4] - both[0] * both[5]) / det]
                    }
                    _ => return Ok(Some(0)),
                };
                for (i, v) in x.iter().enumerate() { self.w32(out + i as u32 * 4, v.to_bits())?; }
                1
            }
            "NtGdiScaleViewportExtEx" | "NtGdiScaleWindowExtEx" => {
                let (hdc, xn, xd, yn, yd, size) = (a[0], a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32, a[5]);
                if !self.wm.dcs.contains_key(&hdc) { return Ok(Some(0)); }
                let at = self.gb_attr(hdc);
                if at == 0 { return Ok(Some(0)); }
                let o = if name == "NtGdiScaleWindowExtEx" { 108 } else { 124 };
                let (cx, cy) = (self.r32(at + o)? as i32, self.r32(at + o + 4)? as i32);
                if size != 0 { self.w32(size, cx as u32)?; self.w32(size + 4, cy as u32)?; }
                let mode = self.r32(at + 60)?;
                if mode == 7 || mode == 8 {
                    if xn == 0 || xd == 0 || yn == 0 || yd == 0 { return Ok(Some(0)); }
                    let (nx, ny) = (((cx as i64 * xn as i64) / xd as i64) as i32, ((cy as i64 * yn as i64) / yd as i64) as i32);
                    self.w32(at + o, if nx == 0 { 1 } else { nx } as u32)?; self.w32(at + o + 4, if ny == 0 { 1 } else { ny } as u32)?;
                }
                1
            }
            "NtGdiSetVirtualResolution" => {
                let (hdc, v) = (a[0], &a[1..5]);
                if v.iter().any(|x| *x == 0) && v.iter().any(|x| *x != 0) { return Ok(Some(0)); }
                if !self.wm.dcs.contains_key(&hdc) { return Ok(Some(0)); }
                let at = self.gb_attr(hdc);
                if at != 0 { for (i, x) in v.iter().enumerate() { self.w32(at + 132 + i as u32 * 4, *x)?; } }
                1
            }
            "NtGdiGetDCDword" => {
                let hdc = a[0];
                if !self.wm.dcs.contains_key(&hdc) { return Ok(Some(0)); }
                let at = self.gb_attr(hdc);
                let (w32o, w16o): (Option<u32>, Option<u32>) = match a[1] { 0 => (Some(40), None), 1 => (Some(12), None), 2 => (None, Some(50)), 3 => (Some(16), None), 4 => (Some(20), None), 5 => (Some(36), None), 6 => (Some(44), None), 7 => (None, Some(52)), 8 => (None, Some(54)), 9 => (Some(24), None), _ => (None, None) };
                let v = if a[1] == 10 { (self.wm.dcs[&hdc].hwnd == 0 || self.wm.dcs[&hdc].bitmap != 0) as u32 }
                    else if let Some(o) = w32o { self.r32(at + o)? }
                    else if let Some(o) = w16o { self.mem.r16(at + o).map_err(|_| "dcdword")? as u32 }
                    else { return Ok(Some(0)) };
                self.w32(a[2], v)?;
                1
            }
            "NtGdiGetDCPoint" => {
                let hdc = a[0];
                let Some(dc) = self.wm.dcs.get(&hdc) else { return Ok(Some(0)) };
                let (hwnd, pos) = (dc.hwnd, dc.pos);
                let (x, y) = match a[1] {
                    0 => { let at = self.gb_attr(hdc); (self.r32(at + 92)?, self.r32(at + 96)?) }
                    1 => { let p = self.dp(hdc, pos.0, pos.1); (p.0 as u32, p.1 as u32) }
                    2 => { let o = if self.wm.dcs[&hdc].bitmap != 0 { (0, 0) } else { self.client_origin_screen(hwnd) }; (o.0 as u32, o.1 as u32) }
                    _ => return Ok(Some(0)),
                };
                self.w32(a[2], x)?; self.w32(a[2] + 4, y)?;
                1
            }
            "NtGdiGetBoundsRect" => {
                let (hdc, rp, flags) = (a[0], a[1], a[2]);
                if !self.wm.dcs.contains_key(&hdc) { return Ok(Some(0)); }
                let (b, _) = self.wm.extra.gdi_b.bounds.get(&hdc).copied().unwrap_or(([i32::MAX, i32::MAX, i32::MIN, i32::MIN], false));
                let mut ret = 0;
                if rp != 0 {
                    if rect_empty(b) { self.put_rect(rp, [0; 4])?; ret = 1; }
                    else {
                        let (p0, p1) = (self.dp(hdc, b[0].max(0), b[1].max(0)), self.dp(hdc, b[2], b[3]));
                        self.put_rect(rp, [p0.0, p0.1, p1.0, p1.1])?; ret = 2;
                    }
                }
                if flags & 1 != 0 { if let Some(e) = self.wm.extra.gdi_b.bounds.get_mut(&hdc) { e.0 = [i32::MAX, i32::MAX, i32::MIN, i32::MIN]; } }
                ret
            }
            "NtGdiSetBoundsRect" => {
                let (hdc, rp, flags) = (a[0], a[1], a[2]);
                if flags & 4 != 0 && flags & 8 != 0 { return Ok(Some(0)); }
                if !self.wm.dcs.contains_key(&hdc) { return Ok(Some(0)); }
                let (mut b, mut en) = self.wm.extra.gdi_b.bounds.get(&hdc).copied().unwrap_or(([i32::MAX, i32::MAX, i32::MIN, i32::MIN], false));
                let ret = (if en { 4 } else { 8 }) | if rect_empty(b) { 0 } else { 2 };
                if flags & 1 != 0 { b = [i32::MAX, i32::MAX, i32::MIN, i32::MIN]; }
                if flags & 2 != 0 && rp != 0 {
                    let r = self.rect_at(rp)?;
                    let (p0, p1) = (self.lp(hdc, r[0], r[1]), self.lp(hdc, r[2], r[3]));
                    let rc = [p0.0.min(p1.0), p0.1.min(p1.1), p0.0.max(p1.0), p0.1.max(p1.1)];
                    if !rect_empty(rc) { b = if rect_empty(b) { rc } else { [b[0].min(rc[0]), b[1].min(rc[1]), b[2].max(rc[2]), b[3].max(rc[3])] }; }
                }
                if flags & 4 != 0 { en = true; }
                if flags & 8 != 0 { en = false; }
                self.wm.extra.gdi_b.bounds.insert(hdc, (b, en));
                ret
            }

            "NtGdiGetAndSetDCDword" => {
                // (hdc, method, value, prev*): DC_ATTR fields live in guest memory
                let (hdc, method, val) = (a[0], a[1], a[2]);
                if !self.wm.dcs.contains_key(&hdc) { return Ok(Some(0)); }
                let at = self.gb_attr(hdc);
                if at == 0 { return Ok(Some(0)); }
                let (off, wide): (u32, bool) = match method { 100 => (12, true), 101 => (50, false), 102 => (24, true), 103 => (16, true), 104 => (20, true), 105 => (36, true), 106 => (54, false), 107 => (48, false), 8 => (60, true), _ => return Ok(Some(0)) };
                let prev = if wide { self.r32(at + off)? } else { self.mem.r16(at + off).map_err(|_| "dcattr")? as u32 };
                if method == 8 {
                    let g = |s: &Self, o: u32| s.r32(at + o).unwrap_or(0) as i32;
                    let (res, size) = ((g(self, 132), g(self, 136)), (g(self, 140), g(self, 144)));
                    let md = |n: i64, d: i64, v: i32| (n * v as i64 / d) as i32;
                    let ext = match val { 1 => Some((1, 1, 1, 1)), 2 | 7 => Some((size.0 * 10, size.1 * 10, res.0, -res.1)), 3 => Some((size.0 * 100, size.1 * 100, res.0, -res.1)),
                        4 => Some((md(1000, 254, size.0), md(1000, 254, size.1), res.0, -res.1)), 5 => Some((md(10000, 254, size.0), md(10000, 254, size.1), res.0, -res.1)),
                        6 => Some((md(14400, 254, size.0), md(14400, 254, size.1), res.0, -res.1)), 8 => None, _ => return Ok(Some(0)) };
                    if !((val == 7 || val == 8) && prev == val) {
                        if let Some((wx, wy, vx, vy)) = ext { for (o, v) in [(108, wx), (112, wy), (124, vx), (128, vy)] { self.w32(at + o, v as u32)?; } }
                        self.w32(at + 60, val)?;
                    }
                } else if method == 105 {
                    if val != 1 && val != 2 { return Ok(Some(0)); }
                    if val == 1 { self.wm.extra.gdi_b.xf.remove(&hdc); }
                    self.w32(at + 36, val)?;
                } else if wide { self.w32(at + off, val)?; } else { self.w16(at + off, val as u16)?; }
                if a[3] != 0 { self.w32(a[3], prev)?; }
                1
            }
            // ---------------- bitmaps / brushes / colors ----------------
            "NtGdiCreateDIBitmapInternal" => {
                // (hdc, width, height, init, bits, bmi, coloruse, max_info, max_bits, flags, xform)
                let (w, h, init, bits, bmi) = (a[1] as i32, a[2] as i32, a[3], a[4], a[5]);
                if w < 0 || h < 0 { return Ok(Some(0)); }
                let (hbm, _) = self.bmp_alloc(w, h, 32, true, false)?;
                if init & 4 != 0 && bits != 0 && bmi != 0 {
                    let d = self.decode_dib(bmi, bits, false)?;
                    for y in 0..h.min(d.h) { for x in 0..w.min(d.w) { self.bmp_set(hbm, x, y, d.pix[(y * d.w + x) as usize]); } }
                }
                hbm
            }
            "NtGdiCreateDIBBrush" => {
                // (data, coloruse, size, is_8x8, pen, client): packed DIB; a solid brush with the average color stands in for the pattern
                let data = a[0];
                let hdr = self.r32(data)?;
                let bpp = self.mem.r16(data + 14).map_err(|_| "dibbrush")? as u32;
                let (used, comp) = (self.r32(data + 32)?, self.r32(data + 16)?);
                let ncol = if bpp <= 8 { if used != 0 { used } else { 1 << bpp } } else { used };
                let bits = data + hdr + if comp == 3 && hdr == 40 { 12 } else { 0 } + ncol * if a[1] == 1 { 2 } else { 4 };
                let col = match if hdr >= 40 { self.decode_dib(data, bits, false).ok() } else { None } {
                    Some(d) if !d.pix.is_empty() => {
                        let n = d.pix.len() as u64;
                        let s = |sh: u32| (d.pix.iter().map(|p| (p >> sh & 0xff) as u64).sum::<u64>() / n) as u32;
                        0xFF00_0000 | s(16) << 16 | s(8) << 8 | s(0)
                    }
                    _ => 0xFFC0_C0C0,
                };
                if a[4] != 0 { self.gdi_alloc(0x500000, Gdi::Pen(col, false)) } else { self.gdi_alloc(0x100000, Gdi::Brush(col)) }
            }
            "NtGdiCreateHatchBrushInternal" => {
                // hatch lines cover a fraction of the cell; approximate with the color lightened toward white
                let c = colorref(a[1]);
                let l = |sh: u32| { let v = c >> sh & 0xff; v + (255 - v) / 4 };
                let col = 0xFF00_0000 | l(16) << 16 | l(8) << 8 | l(0);
                if a[2] != 0 { self.gdi_alloc(0x500000, Gdi::Pen(col, false)) } else { self.gdi_alloc(0x100000, Gdi::Brush(col)) }
            }
            "NtGdiGetNearestColor" => a[1] & 0xFF_FFFF,
            "NtGdiGetBitmapBits" | "NtGdiSetBitmapBits" => {
                let (hbm, count, bits) = (a[0], a[1] as i32, a[2]);
                let Some(b) = self.wm.bitmaps.get(&hbm).copied() else { return Ok(Some(0)) };
                let rb = Self::gb_bitmap_layout(&b);
                let total = rb * b.h as u32;
                if name == "NtGdiGetBitmapBits" {
                    if bits == 0 { return Ok(Some(total)); }
                    let n = (count.max(0) as u32).min(total);
                    let mut buf = vec![0u8; total as usize];
                    if b.dib {
                        for i in 0..total.min(b.stride * b.h as u32) { buf[i as usize] = self.mem.r8(b.addr + i).unwrap_or(0); }
                    } else {
                        for y in 0..b.h { for x in 0..b.w {
                            let v = self.bmp_get(hbm, x, y);
                            if b.bpp == 1 { if v & 0xFF_FFFF != 0 { buf[(y as u32 * rb + x as u32 / 8) as usize] |= 0x80 >> (x % 8); } }
                            else { let o = (y as u32 * rb + x as u32 * 4) as usize; buf[o..o + 4].copy_from_slice(&(v & 0xFF_FFFF).to_le_bytes()); }
                        } }
                    }
                    self.mem.write_bytes(bits, &buf[..n as usize]).map_err(|_| "getbitmapbits")?;
                    n
                } else {
                    if bits == 0 { return Ok(Some(0)); }
                    let n = (count.max(0) as u32).min(total);
                    let mut buf = vec![0u8; n as usize];
                    for i in 0..n { buf[i as usize] = self.mem.r8(bits + i).map_err(|_| "setbitmapbits")?; }
                    if b.dib {
                        let m = n.min(b.stride * b.h as u32);
                        for i in 0..m { let _ = self.mem.w8(b.addr + i, buf[i as usize]); }
                    } else {
                        for y in 0..b.h { for x in 0..b.w {
                            if b.bpp == 1 {
                                let o = (y as u32 * rb + x as u32 / 8) as usize;
                                if o < buf.len() { self.bmp_set(hbm, x, y, if buf[o] >> (7 - x % 8) & 1 != 0 { 0xFFFF_FFFF } else { 0xFF00_0000 }); }
                            } else {
                                let o = (y as u32 * rb + x as u32 * 4) as usize;
                                if o + 4 <= buf.len() { self.bmp_set(hbm, x, y, u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]) | 0xFF00_0000); }
                            }
                        } }
                    }
                    n
                }
            }
            "NtGdiGetBitmapDimension" => {
                if !self.wm.bitmaps.contains_key(&a[0]) { return Ok(Some(0)); }
                let d = self.wm.extra.gdi_b.bm_dim.get(&a[0]).copied().unwrap_or((0, 0));
                self.w32(a[1], d.0 as u32)?; self.w32(a[1] + 4, d.1 as u32)?;
                1
            }
            "NtGdiSetBitmapDimension" => {
                if !self.wm.bitmaps.contains_key(&a[0]) { return Ok(Some(0)); }
                let old = self.wm.extra.gdi_b.bm_dim.insert(a[0], (a[1] as i32, a[2] as i32)).unwrap_or((0, 0));
                if a[3] != 0 { self.w32(a[3], old.0 as u32)?; self.w32(a[3] + 4, old.1 as u32)?; }
                1
            }
            "NtGdiMaskBlt" => {
                // (hdc, x, y, w, h, src, xs, ys, mask, xm, ym, rop4, bk)
                let (hdc, src, mask, rop) = (a[0], a[5], a[8], a[11]);
                if mask == 0 { return self.gdi_call("NtGdiBitBlt", &[hdc, a[1], a[2], a[3], a[4], src, a[6], a[7], rop & 0x00FF_FFFF]); }
                if !self.wm.bitmaps.contains_key(&mask) { return Ok(Some(0)); }
                let ((x, y), (w, h)) = (self.lp(hdc, a[1] as i32, a[2] as i32), self.lp_size(hdc, a[3] as i32, a[4] as i32));
                if w <= 0 || h <= 0 { return Ok(Some(1)); }
                let (xs, ys) = if src != 0 { self.lp(src, a[6] as i32, a[7] as i32) } else { (0, 0) };
                let dpx = self.dc_read_rect(hdc, x, y, w, h);
                let spx = if src != 0 { self.dc_read_rect(src, xs, ys, w, h) } else { vec![0; dpx.len()] };
                let pat = self.wm.dcs.get(&hdc).map_or(0, |d| self.brush_argb(d.brush)) & 0xFF_FFFF;
                let (fore, back) = (rop >> 16 & 0xff, rop >> 24 & 0xff);
                let mut out = dpx.clone();
                for j in 0..h { for i in 0..w {
                    let k = (j * w + i) as usize;
                    let m = self.bmp_get(mask, a[9] as i32 + i, a[10] as i32 + j) & 0xFF_FFFF != 0;
                    out[k] = 0xFF00_0000 | rop_apply(if m { fore } else { back }, pat, spx[k] & 0xFF_FFFF, dpx[k] & 0xFF_FFFF);
                } }
                self.dc_write_rect(hdc, x, y, w, h, &out);
                1
            }
            "NtGdiPlgBlt" => {
                // (hdc, pts*, src, xs, ys, w, h, mask, xm, ym, bk)
                let (hdc, pp, src, w, h, mask) = (a[0], a[1], a[2], a[5] as i32, a[6] as i32, a[7]);
                if w <= 0 || h <= 0 { return Ok(Some(0)); }
                let mut p = [(0f64, 0f64); 3];
                for (i, q) in p.iter_mut().enumerate() { let (x, y) = (self.r32(pp + i as u32 * 8)? as i32, self.r32(pp + i as u32 * 8 + 4)? as i32); let d = self.lp(hdc, x, y); *q = (d.0 as f64, d.1 as f64); }
                let (e1, e2) = ((p[1].0 - p[0].0, p[1].1 - p[0].1), (p[2].0 - p[0].0, p[2].1 - p[0].1));
                let det = e1.0 * e2.1 - e1.1 * e2.0;
                if det == 0.0 { return Ok(Some(0)); }
                let c4 = (p[1].0 + p[2].0 - p[0].0, p[1].1 + p[2].1 - p[0].1);
                let xs_ = [p[0].0, p[1].0, p[2].0, c4.0]; let ys_ = [p[0].1, p[1].1, p[2].1, c4.1];
                let fmin = |v: &[f64]| v.iter().cloned().fold(f64::MAX, f64::min); let fmax = |v: &[f64]| v.iter().cloned().fold(f64::MIN, f64::max);
                let (bx, by, bw, bh) = (fmin(&xs_).floor() as i32, fmin(&ys_).floor() as i32, (fmax(&xs_).ceil() - fmin(&xs_).floor()) as i32, (fmax(&ys_).ceil() - fmin(&ys_).floor()) as i32);
                if bw <= 0 || bh <= 0 || bw > 8192 || bh > 8192 { return Ok(Some(0)); }
                let (sx0, sy0) = self.lp(src, a[3] as i32, a[4] as i32);
                let spx = self.dc_read_rect(src, sx0, sy0, w, h);
                let mut dpx = self.dc_read_rect(hdc, bx, by, bw, bh);
                for j in 0..bh { for i in 0..bw {
                    let (rx, ry) = (bx as f64 + i as f64 + 0.5 - p[0].0, by as f64 + j as f64 + 0.5 - p[0].1);
                    let (u, v) = ((rx * e2.1 - ry * e2.0) / det, (e1.0 * ry - e1.1 * rx) / det);
                    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) { continue; }
                    let (sx, sy) = ((u * w as f64) as i32, (v * h as f64) as i32);
                    if mask != 0 && self.bmp_get(mask, a[8] as i32 + sx, a[9] as i32 + sy) & 0xFF_FFFF == 0 { continue; }
                    dpx[(j * bw + i) as usize] = spx[(sy * w + sx) as usize] | 0xFF00_0000;
                } }
                self.dc_write_rect(hdc, bx, by, bw, bh, &dpx);
                1
            }
            "NtGdiTransparentBlt" => {
                let (dst, src) = (a[0], a[5]);
                let ((x, y), (wd, hd)) = (self.lp(dst, a[1] as i32, a[2] as i32), self.lp_size(dst, a[3] as i32, a[4] as i32));
                let ((xs, ys), (ws, hs)) = (self.lp(src, a[6] as i32, a[7] as i32), self.lp_size(src, a[8] as i32, a[9] as i32));
                if wd <= 0 || hd <= 0 || ws <= 0 || hs <= 0 { return Ok(Some(0)); }
                let spx = self.dc_read_rect(src, xs, ys, ws, hs);
                let spx = Self::resample(&spx, ws, hs, wd, hd);
                let mut d = self.dc_read_rect(dst, x, y, wd, hd);
                let key = colorref(a[10]) & 0xFF_FFFF;
                for (dp, sp) in d.iter_mut().zip(spx.iter()) { if sp & 0xFF_FFFF != key { *dp = *sp | 0xFF00_0000; } }
                self.dc_write_rect(dst, x, y, wd, hd, &d);
                1
            }
            "NtGdiGradientFill" => {
                let (hdc, vp, nv, gp, ng, mode) = (a[0], a[1], a[2], a[3], a[4], a[5]);
                let vert = |s: &Self, i: u32| -> Res<(Pt, [f64; 3])> {
                    if i >= nv { return Err("gradient: vertex index".into()); }
                    let o = vp + i * 16;
                    let p = s.lp(hdc, s.r32(o)? as i32, s.r32(o + 4)? as i32);
                    let c = |k: u32| (s.mem.r16(o + 8 + k * 2).unwrap_or(0) >> 8) as f64;
                    Ok((p, [c(0), c(1), c(2)]))
                };
                let px = |c: [f64; 3]| 0xFF00_0000 | (c[0].round().clamp(0.0, 255.0) as u32) << 16 | (c[1].round().clamp(0.0, 255.0) as u32) << 8 | c[2].round().clamp(0.0, 255.0) as u32;
                match mode {
                    0 | 1 => for g in 0..ng {
                        let (v0, v1) = (vert(self, self.r32(gp + g * 8)?)?, vert(self, self.r32(gp + g * 8 + 4)?)?);
                        let r = [v0.0 .0.min(v1.0 .0), v0.0 .1.min(v1.0 .1), v0.0 .0.max(v1.0 .0), v0.0 .1.max(v1.0 .1)];
                        let (c0, c1, rev) = if mode == 0 { (v0.1, v1.1, v0.0 .0 > v1.0 .0) } else { (v0.1, v1.1, v0.0 .1 > v1.0 .1) };
                        let (lo, hi) = if rev { (c1, c0) } else { (c0, c1) };
                        let n = if mode == 0 { r[2] - r[0] } else { r[3] - r[1] };
                        for i in 0..n {
                            let t = (i as f64 + 0.5) / n as f64;
                            let c = px([lo[0] + (hi[0] - lo[0]) * t, lo[1] + (hi[1] - lo[1]) * t, lo[2] + (hi[2] - lo[2]) * t]);
                            if mode == 0 { self.dc_fill(hdc, [r[0] + i, r[1], r[0] + i + 1, r[3]], c); } else { self.dc_fill(hdc, [r[0], r[1] + i, r[2], r[1] + i + 1], c); }
                        }
                    },
                    2 => for g in 0..ng {
                        let t = [vert(self, self.r32(gp + g * 12)?)?, vert(self, self.r32(gp + g * 12 + 4)?)?, vert(self, self.r32(gp + g * 12 + 8)?)?];
                        let xs: Vec<i32> = t.iter().map(|v| v.0 .0).collect(); let ys: Vec<i32> = t.iter().map(|v| v.0 .1).collect();
                        let (bx, by) = (*xs.iter().min().unwrap(), *ys.iter().min().unwrap());
                        let (bw, bh) = (*xs.iter().max().unwrap() - bx + 1, *ys.iter().max().unwrap() - by + 1);
                        if bw <= 0 || bh <= 0 || bw > 8192 || bh > 8192 { continue; }
                        let mut buf = self.dc_read_rect(hdc, bx, by, bw, bh);
                        let (p0, p1, p2) = ((t[0].0 .0 as f64, t[0].0 .1 as f64), (t[1].0 .0 as f64, t[1].0 .1 as f64), (t[2].0 .0 as f64, t[2].0 .1 as f64));
                        let det = (p1.1 - p2.1) * (p0.0 - p2.0) + (p2.0 - p1.0) * (p0.1 - p2.1);
                        if det == 0.0 { continue; }
                        for j in 0..bh { for i in 0..bw {
                            let (x, y) = (bx as f64 + i as f64 + 0.5, by as f64 + j as f64 + 0.5);
                            let l0 = ((p1.1 - p2.1) * (x - p2.0) + (p2.0 - p1.0) * (y - p2.1)) / det;
                            let l1 = ((p2.1 - p0.1) * (x - p2.0) + (p0.0 - p2.0) * (y - p2.1)) / det;
                            let l2 = 1.0 - l0 - l1;
                            if l0 < -1e-9 || l1 < -1e-9 || l2 < -1e-9 { continue; }
                            let c = |k: usize| t[0].1[k] * l0 + t[1].1[k] * l1 + t[2].1[k] * l2;
                            buf[(j * bw + i) as usize] = px([c(0), c(1), c(2)]);
                        } }
                        self.dc_write_rect(hdc, bx, by, bw, bh, &buf);
                    },
                    _ => return Ok(Some(0)),
                }
                1
            }
            "NtGdiExtFloodFill" => {
                let (hdc, color, ty) = (a[0], colorref(a[3]) & 0xFF_FFFF, a[4]);
                let bnd = self.gb_dc_bounds(hdc);
                let (w, h) = (bnd[2] - bnd[0], bnd[3] - bnd[1]);
                let (sx, sy) = self.lp(hdc, a[1] as i32, a[2] as i32);
                if w <= 0 || h <= 0 || sx < 0 || sy < 0 || sx >= w || sy >= h { return Ok(Some(0)); }
                let Some(fill) = self.gb_fill_color(hdc) else { return Ok(Some(0)) };
                let mut px = self.dc_read_rect(hdc, 0, 0, w, h);
                let ok = |p: u32| if ty == 1 { p & 0xFF_FFFF == color } else { p & 0xFF_FFFF != color };
                if !ok(px[(sy * w + sx) as usize]) { return Ok(Some(0)); }
                let mut seen = vec![false; px.len()];
                let mut stack = vec![(sx, sy)];
                let mut bb = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
                while let Some((x, y)) = stack.pop() {
                    if x < 0 || y < 0 || x >= w || y >= h { continue; }
                    let i = (y * w + x) as usize;
                    if seen[i] || !ok(px[i]) { continue; }
                    seen[i] = true;
                    bb = [bb[0].min(x), bb[1].min(y), bb[2].max(x + 1), bb[3].max(y + 1)];
                    stack.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
                }
                for (i, s) in seen.iter().enumerate() { if *s { px[i] = fill; } }
                let (bw, bh) = (bb[2] - bb[0], bb[3] - bb[1]);
                let mut sub = Vec::with_capacity((bw * bh) as usize);
                for y in bb[1]..bb[3] { for x in bb[0]..bb[2] { sub.push(px[(y * w + x) as usize]); } }
                self.dc_write_rect(hdc, bb[0], bb[1], bw, bh, &sub);
                1
            }

            // ---------------- regions ----------------
            "NtGdiCreateEllipticRgn" => self.gdi_alloc(0x040000, Gdi::Region([a[0] as i32, a[1] as i32, a[2] as i32, a[3] as i32])),
            "NtGdiFrameRgn" => {
                let Some(Gdi::Region(r)) = self.wm.gdi.get(&a[1]).copied() else { return Ok(Some(0)) };
                let c = self.brush_argb(a[2]);
                let (w, h) = (a[3] as i32, a[4] as i32);
                for q in [[r[0], r[1], r[2], r[1] + h], [r[0], r[3] - h, r[2], r[3]], [r[0], r[1], r[0] + w, r[3]], [r[2] - w, r[1], r[2], r[3]]] { self.dc_fill(a[0], q, c); }
                1
            }
            "NtGdiInvertRgn" => {
                let Some(Gdi::Region(r)) = self.wm.gdi.get(&a[1]).copied() else { return Ok(Some(0)) };
                if rect_empty(r) { return Ok(Some(1)); }
                let (w, h) = (r[2] - r[0], r[3] - r[1]);
                let px: Vec<u32> = self.dc_read_rect(a[0], r[0], r[1], w, h).iter().map(|p| 0xFF00_0000 | !p & 0xFF_FFFF).collect();
                self.dc_write_rect(a[0], r[0], r[1], w, h, &px);
                1
            }
            "NtGdiGetRegionData" => {
                let Some(Gdi::Region(r)) = self.wm.gdi.get(&a[0]).copied() else { return Ok(Some(0)) };
                let n = if rect_empty(r) { 0 } else { 1 };
                let size = 32 + n * 16;
                if a[2] == 0 { return Ok(Some(size)); }
                if a[1] < size { return Ok(Some(0)); }
                let d = a[2];
                for (i, v) in [32u32, 1, n, n * 16].iter().enumerate() { self.w32(d + i as u32 * 4, *v)?; }
                self.put_rect(d + 16, if n == 0 { [0; 4] } else { r })?;
                if n == 1 { self.put_rect(d + 32, r)?; }
                size
            }

            // ---------------- palettes ----------------
            "NtGdiCreatePaletteInternal" => {
                let n = a[1];
                let mut v = Vec::with_capacity(n as usize);
                for i in 0..n { v.push(self.r32(a[0] + 4 + i * 4).unwrap_or(0)); }
                let h = self.gdi_alloc(0x080000, Gdi::Other);
                self.wm.extra.gdi_b.pals.insert(h, v);
                h
            }
            "NtGdiGetNearestPaletteIndex" => {
                let Some(p) = self.wm.extra.gdi_b.pals.get(&a[0]) else { return Ok(Some(GDI_ERROR)) };
                let c = a[1];
                let d = |e: u32| { let f = |sh: u32| ((e >> sh & 0xff) as i32 - (c >> sh & 0xff) as i32).pow(2); f(0) + f(8) + f(16) };
                p.iter().enumerate().min_by_key(|(_, e)| d(**e)).map_or(GDI_ERROR, |(i, _)| i as u32)
            }
            "NtGdiDoPalette" => {
                let (h, start, count, ep, func) = (a[0], (a[1] & 0xffff) as usize, (a[2] & 0xffff) as usize, a[3], a[4]);
                match func {
                    0 | 1 | 2 => {
                        let Some(mut pal) = self.wm.extra.gdi_b.pals.remove(&h) else { return Ok(Some(0)) };
                        let ret = if func == 2 && ep == 0 { pal.len() } else if start >= pal.len() { 0 } else {
                            let n = count.min(pal.len() - start);
                            for i in 0..n {
                                if func == 2 { let _ = self.w32(ep + i as u32 * 4, pal[start + i]); } else { pal[start + i] = self.r32(ep + i as u32 * 4).unwrap_or(0); }
                            }
                            n
                        };
                        self.wm.extra.gdi_b.pals.insert(h, pal);
                        ret as u32
                    }
                    _ => 0,
                }
            }
            "NtGdiResizePalette" => match self.wm.extra.gdi_b.pals.get_mut(&a[0]) { Some(p) => { p.resize(a[1] as usize, 0); 1 } None => 0 },
            "NtGdiUpdateColors" | "NtGdiUnrealizeObject" | "NtGdiSetMagicColors" => 1,
            "NtGdiSetSystemPaletteUse" => { let s = &mut self.wm.extra.gdi_b; if s.syspal == 0 { s.syspal = 1; } std::mem::replace(&mut s.syspal, a[1]) }

            // ---------------- text / fonts ----------------
            "NtGdiSetTextJustification" => self.wm.dcs.contains_key(&a[0]) as u32,
            "NtGdiGetRasterizerCaps" => {
                if a[0] == 0 { return Ok(Some(0)); }
                let bytes = [6u8, 0, 3, 0, 0x09, 0x04];
                for i in 0..(a[1] as usize).min(6) { self.mem.w8(a[0] + i as u32, bytes[i]).map_err(|_| "rastcaps")?; }
                1
            }
            "NtGdiAddFontResourceW" | "NtGdiRemoveFontResourceW" => 1,
            "NtGdiAddFontMemResourceEx" => {
                if a[4] != 0 { self.w32(a[4], 1)?; }
                let s = &mut self.wm.extra.gdi_b; s.fake_handle += 1; 0x7000_0000 + s.fake_handle
            }
            "NtGdiRemoveFontMemResourceEx" => 1,
            "NtGdiFontIsLinked" | "NtGdiGetFontFileData" | "NtGdiGetFontFileInfo" | "NtGdiIcmBrushInfo" => 0,
            "NtGdiGetRealizationInfo" => {
                let sz = self.r32(a[1])?;
                if sz != 16 && sz != 24 { return Ok(Some(0)); }
                for (i, v) in [3u32, 1, 1, 1].iter().enumerate() { self.w32(a[1] + 4 + i as u32 * 4, *v)?; }
                if sz == 24 { self.w32(a[1] + 20, 0)?; }
                1
            }
            "NtGdiGetGlyphOutline" => {
                // (hdc, ch, format, metrics*, size, buffer*, mat2*, ignore_rotation)
                let (hdc, ch, fmt) = (a[0], a[1], a[2] & 0x7f);
                let c = char::from_u32(ch).unwrap_or('?');
                let (px, bold) = self.dc_font(hdc);
                let (g, adv) = plonk_fonts::with(|f| (f.glyph(c, px, bold), f.advance(c, px, bold)));
                let (gw, gh) = (g.width as u32, g.height as u32);
                if a[3] != 0 {
                    let m = a[3];
                    self.w32(m, gw.max(1))?; self.w32(m + 4, gh.max(1))?;
                    self.w32(m + 8, g.xmin as u32)?; self.w32(m + 12, (g.ymin + g.height as i32) as u32)?;
                    self.w16(m + 16, adv.round() as i16 as u16)?; self.w16(m + 18, 0)?;
                }
                let (rb, levels) = match fmt { 0 => return Ok(Some(1)), 1 => (((gw + 31) / 32) * 4, 2u32), 4 => ((gw + 3) & !3, 5), 5 => ((gw + 3) & !3, 17), 6 => ((gw + 3) & !3, 65), _ => return Ok(Some(GDI_ERROR)) };
                let size = rb * gh;
                if a[4] == 0 || a[5] == 0 { return Ok(Some(size)); }
                if a[4] < size { return Ok(Some(GDI_ERROR)); }
                for i in 0..size { let _ = self.mem.w8(a[5] + i, 0); }
                for row in 0..gh { for col in 0..gw {
                    let cov = g.coverage[(row * gw + col) as usize] as u32;
                    if levels == 2 { if cov >= 128 { let o = a[5] + row * rb + col / 8; let cur = self.mem.r8(o).unwrap_or(0); let _ = self.mem.w8(o, cur | 0x80 >> (col % 8)); } }
                    else { let _ = self.mem.w8(a[5] + row * rb + col, (cov * (levels - 1) / 255) as u8); }
                } }
                size
            }

            // ---------------- device / misc ----------------
            "NtGdiFlush" => 1,
            "NtGdiGetDeviceGammaRamp" => {
                if !self.wm.dcs.contains_key(&a[0]) { return Ok(Some(0)); }
                let stored = self.wm.extra.gdi_b.gamma.get(&a[0]).cloned();
                for i in 0..768u32 {
                    let v = match &stored { Some(s) => u16::from_le_bytes([s[i as usize * 2], s[i as usize * 2 + 1]]), None => { let k = i % 256; (k << 8 | k) as u16 } };
                    self.w16(a[1] + i * 2, v)?;
                }
                1
            }
            "NtGdiSetDeviceGammaRamp" => {
                if !self.wm.dcs.contains_key(&a[0]) { return Ok(Some(0)); }
                let mut s = vec![0u8; 1536];
                for i in 0..1536u32 { s[i as usize] = self.mem.r8(a[1] + i).map_err(|_| "gamma")?; }
                self.wm.extra.gdi_b.gamma.insert(a[0], s);
                1
            }
            "NtGdiGetColorAdjustment" => {
                if !self.wm.dcs.contains_key(&a[0]) { return Ok(Some(0)); }
                let d = self.wm.extra.gdi_b.cadj.get(&a[0]).cloned().unwrap_or_else(|| {
                    // size 24, flags 0, illuminant 0, gammas 10000, black 0, white 10000, contrast/brightness/colorfulness/tint 0
                    let w: [u16; 12] = [24, 0, 0, 10000, 10000, 10000, 0, 10000, 0, 0, 0, 0];
                    w.iter().flat_map(|v| v.to_le_bytes()).collect()
                });
                self.mem.write_bytes(a[1], &d).map_err(|_| "cadj")?;
                1
            }
            "NtGdiSetColorAdjustment" => {
                if !self.wm.dcs.contains_key(&a[0]) { return Ok(Some(0)); }
                let mut s = vec![0u8; 24];
                for i in 0..24u32 { s[i as usize] = self.mem.r8(a[1] + i).map_err(|_| "cadj")?; }
                self.wm.extra.gdi_b.cadj.insert(a[0], s);
                1
            }
            "NtGdiSetMetaRgn" => 2,
            "NtGdiCreateClientObj" => self.gdi_alloc(a[0], Gdi::Other),
            "NtGdiDeleteClientObj" => return self.win32u_call("NtGdiDeleteObjectApp", &[a[0]]),
            "NtGdiCreateMetafileDC" => return self.gdi_call("NtGdiCreateCompatibleDC", &[a[0]]),
            "NtGdiDrawStream" | "NtGdiGetSpoolMessage" | "NtGdiInitSpool" | "NtGdiResetDC" | "NtGdiExtEscape" => 0,
            // no printer: the print job calls fail like on a machine without one (SP_ERROR)
            "NtGdiStartDoc" | "NtGdiEndDoc" | "NtGdiStartPage" | "NtGdiEndPage" | "NtGdiAbortDoc" => GDI_ERROR,
            _ => return Ok(None),
        }))
    }
}
