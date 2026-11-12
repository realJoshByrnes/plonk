//! Guest OpenGL 1.x: the generated opengl32.dll stub traps here (syscall table 2); every call is translated into the canonical
//! command stream described in docs/GL_PROTOCOL.md, which the page drains and replays on WebGPU/WebGL2.
use super::*;
use crate::glnames::GL;

type Res<T> = Result<T, String>;

#[derive(Default, Clone, Copy)]
struct Ptr { on: bool, size: u32, ty: u32, stride: u32, p: u32 }

#[derive(Default)]
pub struct Gl {
    pub cmds: Vec<u32>,
    pub frame: Vec<u32>,
    ctx_next: u32,
    cur: u32,
    hdc: u32,
    size: (u32, u32),
    created: bool,
    tex_next: u32,
    bound: u32,
    list_next: u32,
    align: u32,
    vp: [Ptr; 4], // vertex, normal, color, texcoord
    strs: u32,
    pub presents: u64,
    pub stubbed: Vec<String>,
}

type M = [f32; 16];
fn ident() -> M { let mut m = [0.0; 16]; m[0] = 1.0; m[5] = 1.0; m[10] = 1.0; m[15] = 1.0; m }
fn fl(v: u32) -> f32 { f32::from_bits(v) }
fn dbl(lo: u32, hi: u32) -> f64 { f64::from_bits(lo as u64 | (hi as u64) << 32) }

fn rotate(deg: f32, x: f32, y: f32, z: f32) -> M {
    let l = (x * x + y * y + z * z).sqrt();
    if l == 0.0 { return ident(); }
    let (x, y, z) = (x / l, y / l, z / l);
    let (s, c) = deg.to_radians().sin_cos();
    let t = 1.0 - c;
    let mut m = ident();
    m[0] = t * x * x + c; m[1] = t * x * y + s * z; m[2] = t * x * z - s * y;
    m[4] = t * x * y - s * z; m[5] = t * y * y + c; m[6] = t * y * z + s * x;
    m[8] = t * x * z + s * y; m[9] = t * y * z - s * x; m[10] = t * z * z + c;
    m
}

impl Gl {
    pub fn created_and_current(&self) -> bool { self.created && self.cur != 0 }
    fn e(&mut self, op: u32, p: &[u32]) {
        if self.cur == 0 { return; }
        self.cmds.push(op);
        self.cmds.push(p.len() as u32);
        self.cmds.extend_from_slice(p);
    }
    fn ef(&mut self, op: u32, p: &[f32]) { let v: Vec<u32> = p.iter().map(|f| f.to_bits()).collect(); self.e(op, &v); }
}

impl LinuxProc {
    fn gf(&self, p: u32, n: u32) -> Vec<f32> { (0..n).map(|i| fl(self.mem.r32(p + i * 4).unwrap_or(0))).collect() }
    fn gd(&self, p: u32, n: u32) -> Vec<f32> {
        (0..n).map(|i| dbl(self.mem.r32(p + i * 8).unwrap_or(0), self.mem.r32(p + i * 8 + 4).unwrap_or(0)) as f32).collect()
    }

    fn gl_size(&self, _hdc: u32) -> (u32, u32) { (self.gl_rect[4].max(1) as u32, self.gl_rect[5].max(1) as u32) }

    /// Read one vertex-array element component list as f32 (ints normalised for colors/normals when `norm`).
    fn arr_elem(&self, p: Ptr, i: u32, n: u32, norm: bool) -> Vec<f32> {
        let tsz = match p.ty { 0x1400 | 0x1401 => 1, 0x1402 | 0x1403 => 2, 0x140A => 8, _ => 4 };
        let stride = if p.stride != 0 { p.stride } else { p.size * tsz };
        let base = p.p + i * stride;
        (0..n.min(p.size.max(1))).map(|c| {
            let a = base + c * tsz;
            match p.ty {
                0x1406 => fl(self.mem.r32(a).unwrap_or(0)),
                0x140A => dbl(self.mem.r32(a).unwrap_or(0), self.mem.r32(a + 4).unwrap_or(0)) as f32,
                0x1401 => { let v = self.mem.r8(a).unwrap_or(0) as f32; if norm { v / 255.0 } else { v } }
                0x1400 => { let v = self.mem.r8(a).unwrap_or(0) as i8 as f32; if norm { ((2.0 * v + 1.0) / 255.0).max(-1.0) } else { v } }
                0x1403 => { let v = self.mem.r16(a).unwrap_or(0) as f32; if norm { v / 65535.0 } else { v } }
                0x1402 => { let v = self.mem.r16(a).unwrap_or(0) as i16 as f32; if norm { ((2.0 * v + 1.0) / 65535.0).max(-1.0) } else { v } }
                0x1405 => { let v = self.mem.r32(a).unwrap_or(0) as f32; if norm { v / 4294967295.0 } else { v } }
                _ => { let v = self.mem.r32(a).unwrap_or(0) as i32 as f32; if norm { ((2.0 * v + 1.0) / 4294967295.0).max(-1.0) } else { v } }
            }
        }).collect()
    }

    fn gl_draw(&mut self, mode: u32, idx: &[u32]) {
        let vp = self.gl.vp;
        if !vp[0].on || idx.is_empty() { return; }
        let n = idx.len() as u32;
        let mut flags = 0;
        if vp[1].on { flags |= 1; }
        if vp[2].on { flags |= 2; }
        if vp[3].on { flags |= 4; }
        let mut d: Vec<u32> = vec![mode, n, flags];
        let mut push = |d: &mut Vec<u32>, v: &[f32], k: usize, fill: &[f32]| { for j in 0..k { d.push(v.get(j).copied().unwrap_or(fill[j]).to_bits()); } };
        for &i in idx { let v = self.arr_elem(vp[0], i, 4, false); push(&mut d, &v, 4, &[0.0, 0.0, 0.0, 1.0]); }
        if flags & 1 != 0 { for &i in idx { let v = self.arr_elem(vp[1], i, 3, true); push(&mut d, &v, 3, &[0.0; 3]); } }
        if flags & 2 != 0 { for &i in idx { let v = self.arr_elem(vp[2], i, 4, true); push(&mut d, &v, 4, &[0.0, 0.0, 0.0, 1.0]); } }
        if flags & 4 != 0 { for &i in idx { let v = self.arr_elem(vp[3], i, 2, false); push(&mut d, &v, 2, &[0.0; 2]); } }
        self.gl.e(33, &d);
    }

    fn gl_present(&mut self, hdc: u32) {
        let sz = self.gl_size(hdc);
        let g = &mut self.gl;
        if g.cur == 0 { return; }
        g.hdc = hdc;
        if sz != g.size { g.size = sz; g.e(2, &[sz.0, sz.1]); }
        g.e(49, &[sz.0, sz.1]);
        g.presents += 1;
        self.frame_break = true;
        self.resched = true; // hand the frame to the page now (one frame per slice) instead of batching a whole instruction budget
    }

    fn gl_tex(&self, a: &[u32], sub: bool) -> Option<(Vec<u32>, u32)> {
        // glTexImage2D(target, level, internalformat, w, h, border, format, type, pixels)
        // glTexSubImage2D(target, level, xoff, yoff, w, h, format, type, pixels)
        let (w, h, fmt, ty, p) = if sub { (a[4], a[5], a[6], a[7], a[8]) } else { (a[3], a[4], a[6], a[7], a[8]) };
        if a[1] != 0 || w == 0 || h == 0 || w > 4096 || h > 4096 { return None; }
        let ncomp = match fmt { 0x1908 | 0x80E1 => 4, 0x1907 | 0x80E0 => 3, 0x190A => 2, _ => 1 };
        let bpp = if ty == 0x1401 || ty == 0x1400 { ncomp } else { return None };
        let al = self.gl.align.max(1);
        let stride = (w * bpp + al - 1) / al * al;
        let mut out = vec![0u8; (w * h * 4) as usize];
        if p != 0 {
            for y in 0..h { for x in 0..w {
                let s = p + y * stride + x * bpp;
                let g = |i: u32| self.mem.r8(s + i).unwrap_or(0);
                let px: [u8; 4] = match fmt {
                    0x1908 => [g(0), g(1), g(2), g(3)],
                    0x80E1 => [g(2), g(1), g(0), g(3)],
                    0x1907 => [g(0), g(1), g(2), 255],
                    0x80E0 => [g(2), g(1), g(0), 255],
                    0x190A => [g(0), g(0), g(0), g(1)],
                    0x1906 => [255, 255, 255, g(0)],
                    _ => [g(0), g(0), g(0), 255],
                };
                let o = ((y * w + x) * 4) as usize;
                out[o..o + 4].copy_from_slice(&px);
            } }
        }
        let words: Vec<u32> = out.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        Some((words, w * 65536 + h))
    }

    /// The gdi32 side of OpenGL (pixel formats, SwapBuffers).
    pub(crate) fn gl_nt(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        Ok(Some(match name {
            "NtGdiDescribePixelFormat" => {
                // (hdc, format, size, ppfd) -> number of formats; the one format is double-buffered RGBA8 + depth24
                if a[3] != 0 && a[2] >= 40 && a[1] == 1 {
                    let mut b = [0u8; 40];
                    b[0..2].copy_from_slice(&40u16.to_le_bytes());
                    b[2..4].copy_from_slice(&1u16.to_le_bytes());
                    b[4..8].copy_from_slice(&(0x4u32 | 0x20 | 0x1).to_le_bytes()); // DRAW_TO_WINDOW|SUPPORT_OPENGL|DOUBLEBUFFER
                    // iPixelType RGBA, cColorBits 32, R/G/B/A bits+shifts, cDepthBits 24
                    b[8] = 0; b[9] = 32; b[10] = 8; b[11] = 16; b[12] = 8; b[13] = 8; b[14] = 8; b[15] = 0; b[16] = 8; b[17] = 24;
                    b[23] = 24;
                    for (i, v) in b.iter().enumerate() { self.mem.w8(a[3] + i as u32, *v).map_err(|_| "pfd")?; }
                }
                1
            }
            "NtGdiSetPixelFormat" | "NtGdiGetPixelFormat" => 1,
            "NtGdiSwapBuffers" => { self.gl_present(a[0]); 1 }
            _ => return Ok(None),
        }))
    }

    pub(crate) fn gl_call(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        let a = a.to_vec();
        let g = |i: usize| a.get(i).copied().unwrap_or(0);
        let r: u32 = match name {
            "wglCreateContext" => { self.gl.ctx_next += 1; 0x10000 + self.gl.ctx_next }
            "wglDeleteContext" => { if self.gl.cur == g(0) { self.gl.cur = 0; } 1 }
            "wglMakeCurrent" => {
                if g(1) == 0 { self.gl.cur = 0; return Ok(Some(1)); }
                let sz = self.gl_size(g(0));
                let first = !self.gl.created;
                self.gl.cur = g(1);
                self.gl.hdc = g(0);
                if first { self.gl.created = true; self.gl.size = sz; self.gl.e(1, &[sz.0, sz.1]); }
                1
            }
            "wglGetCurrentContext" => self.gl.cur,
            "wglGetCurrentDC" => if self.gl.cur != 0 { self.gl.hdc } else { 0 },
            "wglShareLists" => 1,
            "wglSwapBuffers" => { self.gl_present(g(0)); 1 }
            "wglSwapLayerBuffers" => { self.gl_present(g(0)); 1 }
            "wglUseFontBitmapsA" | "wglUseFontBitmapsW" | "wglUseFontOutlinesA" | "wglUseFontOutlinesW" | "wglDescribeLayerPlane" => 0,
            "glGetError" => 0,
            "glGetIntegerv" | "glGetFloatv" | "glGetDoublev" | "glGetBooleanv" => {
                let vals: Vec<i32> = match g(0) {
                    0xD33 => vec![4096], 0xBA2 => { let (w, h) = self.gl.size; vec![0, 0, w as i32, h as i32] }
                    0xD3A => vec![4096, 4096], 0xD31 => vec![8], 0xD3B => vec![32], 0xD38 => vec![32], 0xD39 => vec![4], 0xD36 => vec![2],
                    0xD50 | 0xD52 | 0xD53 | 0xD54 | 0xD55 | 0xD56 => vec![8], 0xD57 | 0xD58 => vec![24], 0xD5B | 0xD5C => vec![0],
                    0xCF5 => vec![self.gl.align as i32],
                    _ => vec![0],
                };
                for (i, v) in vals.iter().enumerate() {
                    match name {
                        "glGetIntegerv" | "glGetBooleanv" if name == "glGetIntegerv" => { self.w32(g(1) + i as u32 * 4, *v as u32)?; }
                        "glGetFloatv" => { self.w32(g(1) + i as u32 * 4, (*v as f32).to_bits())?; }
                        "glGetDoublev" => { let b = (*v as f64).to_bits(); self.w32(g(1) + i as u32 * 8, b as u32)?; self.w32(g(1) + i as u32 * 8 + 4, (b >> 32) as u32)?; }
                        _ => { self.mem.w8(g(1) + i as u32, *v as u8).map_err(|_| "gl")?; }
                    }
                }
                0
            }
            "glIsEnabled" | "glIsList" => 0,
            "glIsTexture" => (g(0) != 0 && g(0) < self.gl.tex_next) as u32,
            "glGenLists" => { let n = g(0); if self.gl.list_next == 0 { self.gl.list_next = 1; } let f = self.gl.list_next; self.gl.list_next += n; f }
            "glGenTextures" => {
                for i in 0..g(0) { self.gl.tex_next = self.gl.tex_next.max(1); let id = self.gl.tex_next; self.gl.tex_next += 1; self.w32(g(1) + i * 4, id)?; }
                0
            }
            "glDeleteTextures" => { for i in 0..g(0) { let id = self.r32(g(1) + i * 4)?; self.gl.e(42, &[id]); } 0 }
            "glBindTexture" => { self.gl.bound = g(1); self.gl.e(41, &[g(1)]); 0 }
            "glDeleteLists" => { self.gl.e(48, &[g(0), g(1)]); 0 }
            "glNewList" => { self.gl.e(45, &[g(0), g(1)]); 0 }
            "glEndList" => { self.gl.e(46, &[]); 0 }
            "glCallList" => { self.gl.e(47, &[g(0)]); 0 }
            "glClear" => { self.gl.e(3, &[g(0)]); 0 }
            "glClearColor" => { self.gl.e(4, &a[0..4]); 0 }
            "glClearDepth" => { self.gl.ef(5, &[dbl(g(0), g(1)) as f32]); 0 }
            "glEnable" | "glDisable" => {
                let on = name == "glEnable";
                match g(0) {
                    c @ (0x8074..=0x8078) => {
                        let slot = match c { 0x8074 => 0, 0x8075 => 1, 0x8076 => 2, 0x8078 => 3, _ => 9 };
                        if slot < 4 { self.gl.vp[slot].on = on; }
                    }
                    c => self.gl.e(if on { 6 } else { 7 }, &[c]),
                }
                0
            }
            "glEnableClientState" | "glDisableClientState" => {
                let slot = match g(0) { 0x8074 => 0, 0x8075 => 1, 0x8076 => 2, 0x8078 => 3, _ => 9 };
                if slot < 4 { self.gl.vp[slot].on = name == "glEnableClientState"; }
                0
            }
            "glDepthFunc" => { self.gl.e(8, &[g(0)]); 0 }
            "glDepthMask" => { self.gl.e(9, &[g(0) & 0xff]); 0 }
            "glCullFace" => { self.gl.e(10, &[g(0)]); 0 }
            "glFrontFace" => { self.gl.e(11, &[g(0)]); 0 }
            "glBlendFunc" => { self.gl.e(12, &[g(0), g(1)]); 0 }
            "glAlphaFunc" => { self.gl.e(13, &[g(0), g(1)]); 0 }
            "glShadeModel" => { self.gl.e(14, &[g(0)]); 0 }
            "glPolygonMode" => { self.gl.e(15, &[g(0), g(1)]); 0 }
            "glLineWidth" => { self.gl.e(16, &[g(0)]); 0 }
            "glPointSize" => { self.gl.e(17, &[g(0)]); 0 }
            "glViewport" => { self.gl.e(18, &a[0..4]); 0 }
            "glScissor" => { self.gl.e(19, &a[0..4]); 0 }
            "glColorMask" => { let v: Vec<u32> = a[0..4].iter().map(|x| x & 0xff).collect(); self.gl.e(20, &v); 0 }
            "glMatrixMode" => { self.gl.e(21, &[g(0)]); 0 }
            "glLoadIdentity" => { self.gl.e(22, &[]); 0 }
            "glPushMatrix" => { self.gl.e(25, &[]); 0 }
            "glPopMatrix" => { self.gl.e(26, &[]); 0 }
            "glPushAttrib" => { self.gl.e(50, &[g(0)]); 0 }
            "glPopAttrib" => { self.gl.e(51, &[]); 0 }
            "glLoadMatrixf" => { let m = self.gf(g(0), 16); self.gl.ef(23, &m); 0 }
            "glLoadMatrixd" => { let m = self.gd(g(0), 16); self.gl.ef(23, &m); 0 }
            "glMultMatrixf" => { let m = self.gf(g(0), 16); self.gl.ef(24, &m); 0 }
            "glMultMatrixd" => { let m = self.gd(g(0), 16); self.gl.ef(24, &m); 0 }
            "glTranslatef" | "glTranslated" | "glScalef" | "glScaled" => {
                let v: Vec<f32> = if name.ends_with('d') { (0..3).map(|i| dbl(g(i * 2), g(i * 2 + 1)) as f32).collect() } else { (0..3).map(|i| fl(g(i))).collect() };
                let mut m = ident();
                if name.starts_with("glTrans") { m[12] = v[0]; m[13] = v[1]; m[14] = v[2]; } else { m[0] = v[0]; m[5] = v[1]; m[10] = v[2]; }
                self.gl.ef(24, &m); 0
            }
            "glRotatef" => { let m = rotate(fl(g(0)), fl(g(1)), fl(g(2)), fl(g(3))); self.gl.ef(24, &m); 0 }
            "glRotated" => { let v: Vec<f32> = (0..4).map(|i| dbl(g(i * 2), g(i * 2 + 1)) as f32).collect(); let m = rotate(v[0], v[1], v[2], v[3]); self.gl.ef(24, &m); 0 }
            "glOrtho" | "glFrustum" => {
                let v: Vec<f32> = (0..6).map(|i| dbl(g(i * 2), g(i * 2 + 1)) as f32).collect();
                let (l, r, b, t, n, f) = (v[0], v[1], v[2], v[3], v[4], v[5]);
                let mut m = [0.0f32; 16];
                if name == "glOrtho" {
                    m[0] = 2.0 / (r - l); m[5] = 2.0 / (t - b); m[10] = -2.0 / (f - n);
                    m[12] = -(r + l) / (r - l); m[13] = -(t + b) / (t - b); m[14] = -(f + n) / (f - n); m[15] = 1.0;
                } else {
                    m[0] = 2.0 * n / (r - l); m[5] = 2.0 * n / (t - b); m[8] = (r + l) / (r - l); m[9] = (t + b) / (t - b);
                    m[10] = -(f + n) / (f - n); m[11] = -1.0; m[14] = -2.0 * f * n / (f - n);
                }
                self.gl.ef(24, &m); 0
            }
            "glBegin" => { self.gl.e(27, &[g(0)]); 0 }
            "glEnd" => { self.gl.e(28, &[]); 0 }
            "glLightf" | "glLighti" | "glLightfv" | "glLightiv" => {
                let pn = g(1);
                let vals = if name.ends_with('v') {
                    let n = match pn { 0x1203 | 0x1200..=0x1202 => 4, 0x1204 => 3, _ => 1 };
                    self.gf(g(2), n)
                } else if name == "glLighti" { vec![g(2) as i32 as f32] } else { vec![fl(g(2))] };
                let mut p = [0.0f32; 4];
                for (i, v) in vals.iter().enumerate() { p[i] = *v; }
                let mut m = vec![g(0).wrapping_sub(0x4000), pn];
                m.extend(p.iter().map(|f| f.to_bits()));
                self.gl.e(34, &m); 0
            }
            "glLightModelf" | "glLightModeli" | "glLightModelfv" | "glLightModeliv" => {
                let v = if name.ends_with('v') { self.gf(g(1), if g(0) == 0xB53 { 4 } else { 1 }) } else if name.ends_with('i') { vec![g(1) as i32 as f32] } else { vec![fl(g(1))] };
                let mut p = [0.0f32; 4];
                for (i, x) in v.iter().enumerate() { p[i] = *x; }
                let mut m = vec![g(0)]; m.extend(p.iter().map(|f| f.to_bits()));
                self.gl.e(35, &m); 0
            }
            "glMaterialf" | "glMateriali" | "glMaterialfv" => {
                let v = if name.ends_with('v') { self.gf(g(2), if g(1) == 0x1601 { 1 } else { 4 }) } else if name.ends_with('i') { vec![g(2) as i32 as f32] } else { vec![fl(g(2))] };
                let mut p = [0.0f32; 4];
                for (i, x) in v.iter().enumerate() { p[i] = *x; }
                let mut m = vec![g(0), g(1)]; m.extend(p.iter().map(|f| f.to_bits()));
                self.gl.e(36, &m); 0
            }
            "glColorMaterial" => { self.gl.e(37, &[g(0), g(1)]); 0 }
            "glFogf" | "glFogi" | "glFogfv" | "glFogiv" => {
                let pn = g(0);
                let v = if name.ends_with('v') {
                    if pn == 0xB66 { self.gf(g(1), 4) } else if name == "glFogiv" { vec![self.r32(g(1))? as i32 as f32] } else { self.gf(g(1), 1) }
                } else if name == "glFogi" { vec![g(1) as i32 as f32] } else { vec![fl(g(1))] };
                let mut p = [0.0f32; 4];
                for (i, x) in v.iter().enumerate() { p[i] = *x; }
                let mut m = vec![pn]; m.extend(p.iter().map(|f| f.to_bits()));
                self.gl.e(38, &m); 0
            }
            "glTexParameterf" | "glTexParameteri" | "glTexParameterfv" | "glTexParameteriv" | "glTexEnvf" | "glTexEnvi" | "glTexEnvfv" | "glTexEnviv" => {
                let env = name.starts_with("glTexEnv");
                let v = if name.ends_with('v') { if name.ends_with("fv") { self.gf(g(2), 1)[0] } else { self.r32(g(2))? as i32 as f32 } }
                        else if name.ends_with('i') { g(2) as i32 as f32 } else { fl(g(2)) };
                if !(env && g(1) != 0x2200) { self.gl.e(if env { 44 } else { 43 }, &[g(1), v.to_bits()]); }
                0
            }
            "glPixelStorei" => { if g(0) == 0xCF5 { self.gl.align = g(1); } 0 }
            "glTexImage2D" | "glTexSubImage2D" => {
                let sub = name == "glTexSubImage2D";
                if let Some((words, wh)) = self.gl_tex(&a, sub) {
                    let (w, h) = (wh >> 16, wh & 0xffff);
                    let id = self.gl.bound;
                    let mut m: Vec<u32> = if sub { vec![id, g(2), g(3), w, h] } else { vec![id, w, h] };
                    m.extend(words);
                    self.gl.e(if sub { 40 } else { 39 }, &m);
                }
                0
            }
            "glVertexPointer" | "glNormalPointer" | "glColorPointer" | "glTexCoordPointer" => {
                let (slot, p) = match name {
                    "glVertexPointer" => (0, Ptr { on: self.gl.vp[0].on, size: g(0), ty: g(1), stride: g(2), p: g(3) }),
                    "glNormalPointer" => (1, Ptr { on: self.gl.vp[1].on, size: 3, ty: g(0), stride: g(1), p: g(2) }),
                    "glColorPointer" => (2, Ptr { on: self.gl.vp[2].on, size: g(0), ty: g(1), stride: g(2), p: g(3) }),
                    _ => (3, Ptr { on: self.gl.vp[3].on, size: g(0), ty: g(1), stride: g(2), p: g(3) }),
                };
                self.gl.vp[slot] = p; 0
            }
            "glDrawArrays" => { let idx: Vec<u32> = (g(1)..g(1) + g(2)).collect(); self.gl_draw(g(0), &idx); 0 }
            "glDrawElements" => {
                let sz = match g(2) { 0x1401 => 1, 0x1403 => 2, _ => 4 };
                let idx: Vec<u32> = (0..g(1)).map(|i| {
                    let p = g(3) + i * sz;
                    match sz { 1 => self.mem.r8(p).unwrap_or(0) as u32, 2 => self.mem.r16(p).unwrap_or(0) as u32, _ => self.mem.r32(p).unwrap_or(0) }
                }).collect();
                self.gl_draw(g(0), &idx); 0
            }
            "glArrayElement" => {
                let vp = self.gl.vp;
                let i = g(0);
                if vp[2].on { let v = self.arr_elem(vp[2], i, 4, true); let mut c = [0.0, 0.0, 0.0, 1.0]; for (k, x) in v.iter().enumerate() { c[k] = *x; } self.gl.ef(30, &c); }
                if vp[1].on { let v = self.arr_elem(vp[1], i, 3, true); let mut c = [0.0; 3]; for (k, x) in v.iter().enumerate() { c[k] = *x; } self.gl.ef(31, &c); }
                if vp[3].on { let v = self.arr_elem(vp[3], i, 2, false); let mut c = [0.0, 0.0, 0.0, 1.0]; for (k, x) in v.iter().enumerate() { c[k] = *x; } self.gl.ef(32, &c); }
                if vp[0].on { let v = self.arr_elem(vp[0], i, 4, false); let mut c = [0.0, 0.0, 0.0, 1.0]; for (k, x) in v.iter().enumerate() { c[k] = *x; } self.gl.ef(29, &c); }
                0
            }
            n if n.starts_with("glColor") || n.starts_with("glNormal") || n.starts_with("glVertex") || n.starts_with("glTexCoord") => { self.gl_attr(n, &a)?; 0 }
            "glFinish" | "glFlush" | "glHint" | "glDrawBuffer" | "glReadBuffer" | "glDepthRange" | "glClearStencil" | "glStencilFunc" | "glStencilMask"
            | "glStencilOp" | "glPolygonOffset" | "glListBase" | "glCopyTexImage2D" => 0,
            other => { if !self.gl.stubbed.iter().any(|s| s == other) { self.gl.stubbed.push(other.to_string()); } 0 }
        };
        Ok(Some(r))
    }

    /// glColor*/glNormal*/glVertex*/glTexCoord* families, parsed from the function name.
    fn gl_attr(&mut self, name: &str, a: &[u32]) -> Res<()> {
        let (kind, rest) = if let Some(r) = name.strip_prefix("glColor") { (0, r) } else if let Some(r) = name.strip_prefix("glNormal") { (1, r) }
            else if let Some(r) = name.strip_prefix("glVertex") { (2, r) } else { (3, &name[10..]) };
        let n = (rest.as_bytes()[0] - b'0') as usize;
        let vec_form = rest.ends_with('v');
        let ty = rest[1..].trim_end_matches('v');
        let norm = kind <= 1;
        let tsz = |t: &str| match t { "b" | "ub" => 1, "s" | "us" => 2, "d" => 8, _ => 4 };
        let conv = |t: &str, raw: u64| -> f32 {
            match t {
                "f" => f32::from_bits(raw as u32), "d" => f64::from_bits(raw) as f32,
                "b" => { let v = raw as u8 as i8 as f32; if norm { (2.0 * v + 1.0) / 255.0 } else { v } }
                "ub" => { let v = raw as u8 as f32; if norm { v / 255.0 } else { v } }
                "s" => { let v = raw as u16 as i16 as f32; if norm { (2.0 * v + 1.0) / 65535.0 } else { v } }
                "us" => { let v = raw as u16 as f32; if norm { v / 65535.0 } else { v } }
                _ => { let v = raw as u32 as i32 as f32; if norm { (2.0 * v + 1.0) / 4294967295.0 } else { v } }
            }
        };
        let mut v = [0.0f32; 4];
        for i in 0..n {
            let raw: u64 = if vec_form {
                let p = a[0] + (i as u32) * tsz(ty);
                match tsz(ty) { 1 => self.mem.r8(p).unwrap_or(0) as u64, 2 => self.mem.r16(p).unwrap_or(0) as u64, 4 => self.mem.r32(p).unwrap_or(0) as u64,
                    _ => self.mem.r32(p).unwrap_or(0) as u64 | (self.mem.r32(p + 4).unwrap_or(0) as u64) << 32 }
            } else if ty == "d" { a[i * 2] as u64 | (a[i * 2 + 1] as u64) << 32 } else { a[i] as u64 };
            v[i] = conv(ty, raw);
        }
        let out: [f32; 4] = match kind {
            0 => [v[0], v[1], v[2], if n == 4 { v[3] } else { 1.0 }],
            1 => [v[0], v[1], v[2], 0.0],
            2 => [v[0], v[1], if n >= 3 { v[2] } else { 0.0 }, if n == 4 { v[3] } else { 1.0 }],
            _ => [v[0], if n >= 2 { v[1] } else { 0.0 }, if n >= 3 { v[2] } else { 0.0 }, if n == 4 { v[3] } else { 1.0 }],
        };
        match kind {
            0 => self.gl.ef(30, &out), 1 => self.gl.ef(31, &out[..3]), 2 => self.gl.ef(29, &out), _ => self.gl.ef(32, &out),
        }
        Ok(())
    }

    pub(crate) fn gl_name(num: usize) -> Option<(&'static str, u32)> { GL.get(num).map(|&(n, b)| (n, b as u32)) }
}

impl LinuxProc {

    fn r32(&self, a: u32) -> Res<u32> { self.mem.r32(a).map_err(|_| "gl: bad pointer".to_string()) }
    fn w32(&mut self, a: u32, v: u32) -> Res<()> { self.mem.w32(a, v).map_err(|_| "gl: bad pointer".to_string()) }

    /// Where the page should show the GL frame: x, y, width, height, and 1 when the GL window is visible and on top.
    pub fn gl_screen_rect(&self) -> [u32; 5] {
        let [top, _, x, y, w, h] = self.gl_rect;
        let shown = self.gl.created_and_current() && self.windows.get(&(top as u32)).map_or(false, |win| win.visible) && self.zorder.iter().find(|h| self.windows.get(h).map_or(false, |win| win.visible)) == Some(&(top as u32));
        [x.max(0) as u32, y.max(0) as u32, w.max(0) as u32, h.max(0) as u32, shown as u32]
    }
}
