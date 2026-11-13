// GPU desktop compositor: every top-level window is a texture (only the rectangle that changed is uploaded) and the screen
// is drawn as one quad per window, bottom first, on a single WebGL2 canvas. Frames from the 3D backend are drawn on top of
// the same canvas, so nothing needs a second canvas. createCompositor returns null when WebGL2 is not available.
const VS = `#version 300 es
uniform vec4 rect; uniform vec2 screen; out vec2 uv; out vec2 vpx;
void main() {
  vec2 p = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
  uv = p;
  vec2 px = rect.xy + p * rect.zw;
  vpx = px;
  gl_Position = vec4(px.x / screen.x * 2.0 - 1.0, 1.0 - px.y / screen.y * 2.0, 0.0, 1.0);
}`;
const FS = `#version 300 es
precision mediump float;
uniform sampler2D t; uniform float bgr;
uniform vec4 inner;      // the window itself (x, y, w, h); the quad is larger so the shadow has room
uniform vec3 chrome;     // x: 1 = rounded window with border and shadow, y: corner radius, z: shadow strength
uniform vec4 edge;       // border colour (rgb) and strength
uniform vec4 capr;       // title bar rectangle (screen), for chrome.x == 2: the texture is the title bar
in vec2 uv; in vec2 vpx; out vec4 o;
float sdf(vec2 p, float r) { vec2 q = abs(p - (inner.xy + inner.zw * 0.5)) - (inner.zw * 0.5 - r); return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r; }
void main() {
  if (chrome.x < 0.5) { vec4 c = texture(t, uv); o = vec4(bgr > 0.5 ? c.bgr : c.rgb, 1.0); return; }
  float r = chrome.y;
  float d = sdf(vpx, r);
  float inside = 1.0 - smoothstep(-0.5, 0.5, d);
  if (chrome.x > 1.5) { // the host-drawn title bar, clipped to the rounded window
    vec2 cp = (vpx - capr.xy) / capr.zw;
    float inCap = step(0.0, cp.x) * step(cp.x, 1.0) * step(0.0, cp.y) * step(cp.y, 1.0);
    vec3 cc = texture(t, clamp(cp, 0.0, 1.0)).rgb;
    float ring2 = (1.0 - smoothstep(0.0, 1.0, abs(d + 0.5))) * edge.a;
    cc = mix(cc, edge.rgb, ring2);
    float a = inside * inCap;
    o = vec4(cc * a, a);
    return;
  }
  vec4 c = texture(t, clamp((vpx - inner.xy) / inner.zw, 0.0, 1.0));
  vec3 rgb = bgr > 0.5 ? c.bgr : c.rgb;
  float ring = (1.0 - smoothstep(0.0, 1.0, abs(d + 0.5))) * edge.a; // 1px border just inside the edge
  rgb = mix(rgb, edge.rgb, ring);
  float ds = sdf(vpx - vec2(0.0, 6.0), r);                           // shadow: offset down, soft falloff
  float sa = chrome.z * exp(-max(ds, 0.0) * 0.16) * 0.55;
  o = vec4(rgb * inside, inside) + vec4(0.0, 0.0, 0.0, sa) * (1.0 - inside);
}`;

export function createCompositor(canvas) {
  const gl = canvas.getContext("webgl2", { alpha: false, antialias: false, depth: false, stencil: false, preserveDrawingBuffer: true });
  if (!gl) return null;
  const sh = (type, src) => { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s)); return s; };
  const prog = gl.createProgram();
  try {
    gl.attachShader(prog, sh(gl.VERTEX_SHADER, VS)); gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, FS)); gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(prog));
  } catch (e) { console.warn("compositor:", e); return null; }
  gl.useProgram(prog);
  const u = { capr: gl.getUniformLocation(prog, "capr"), inner: gl.getUniformLocation(prog, "inner"), chrome: gl.getUniformLocation(prog, "chrome"), edge: gl.getUniformLocation(prog, "edge"), rect: gl.getUniformLocation(prog, "rect"), screen: gl.getUniformLocation(prog, "screen"), bgr: gl.getUniformLocation(prog, "bgr") };
  gl.uniform1i(gl.getUniformLocation(prog, "t"), 0);
  gl.enable(gl.BLEND);
  gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA); // premultiplied output (shadows are translucent)
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
  const newTex = () => {
    const t = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return t;
  };
  const wins = new Map(); // hwnd -> { tex, w, h }
const PAD = 28; // room around a window for its shadow
  const quad = (x, y, w, h, bgr) => { gl.uniform3f(u.chrome, 0, 0, 0); gl.uniform4f(u.rect, x, y, w, h); gl.uniform1f(u.bgr, bgr ? 1 : 0); gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4); };
  const chromeQuad = (x, y, w, h, top, dark, cap) => {
    const small = w < 220 || h < 90; // menus, tooltips and the like: tighter corners, lighter shadow
    gl.uniform4f(u.inner, x, y, w, h);
    gl.uniform3f(u.chrome, 1, small ? 4 : 8, top ? 1.0 : 0.6);
    gl.uniform4f(u.edge, ...(dark ? [1, 1, 1, 0.14] : [0, 0, 0, 0.22]));
    gl.uniform4f(u.rect, x - PAD, y - PAD, w + 2 * PAD, h + 2 * PAD);
    gl.uniform1f(u.bgr, 1);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    if (cap) { // the title bar on top: same rounded clip, its own texture
      gl.bindTexture(gl.TEXTURE_2D, cap.tex);
      gl.uniform3f(u.chrome, 2, small ? 4 : 8, 0);
      gl.uniform4f(u.capr, cap.l, cap.t, cap.r - cap.l, cap.b - cap.t);
      gl.uniform1f(u.bgr, 0);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    }
  };
let dark = !window.matchMedia || window.matchMedia("(prefers-color-scheme: dark)").matches;
  // ---- title bars: drawn on a 2D canvas per window and used as a texture ----
  const caps = new Map(); // hwnd -> { cv, ctx, tex, key }
  let lastCaps = [];      // captioned windows of the last frame, top first
  let hoverKey = "";
  let wall = null; // { tex, w, h }
  const FONT = '"Segoe UI Variable Text", "Segoe UI", Selawik, system-ui, sans-serif';
  const btnLayout = (c) => { // button rectangles (x offsets from the bar's left edge), right to left
    const w = c.r - c.l, out = [];
    if (!(c.flags & 8)) return out;
    out.push({ id: "close", x: w - c.bw });
    if (c.flags & 6) { out.push({ id: "max", x: w - 2 * c.bw }); out.push({ id: "min", x: w - 3 * c.bw }); }
    return out;
  };
  const hoverOf = (x, y) => {
    for (const c of lastCaps) {
      if (x >= c.l && x < c.r && y >= c.t && y < c.b) {
        const b = btnLayout(c).find((q) => x >= c.l + q.x && x < c.l + q.x + c.bw);
        return c.hwnd + ":" + (b ? b.id : "bar");
      }
      if (x >= c.wx && x < c.wx + c.ww && y >= c.wy && y < c.wy + c.wh) return ""; // a window on top hides the ones below
    }
    return "";
  };
  const drawCaption = (e, c, title, active, hover) => {
    const w = c.r - c.l, h = c.b - c.t;
    if (!e.cv) { e.cv = document.createElement("canvas"); e.ctx = e.cv.getContext("2d"); e.tex = newTex(); }
    if (e.cv.width !== w || e.cv.height !== h) { e.cv.width = w; e.cv.height = h; }
    const g = e.ctx, k = h / 32;
    g.fillStyle = dark ? (active ? "#202020" : "#2b2b2b") : (active ? "#f3f3f3" : "#ffffff");
    g.fillRect(0, 0, w, h);
    const fg = dark ? (active ? "#ffffff" : "#9d9d9d") : (active ? "#1b1b1b" : "#808080");
    const btns = btnLayout(c);
    g.fillStyle = fg; g.font = `${Math.max(12, Math.round(h * 0.46))}px ${FONT}`; g.textBaseline = "middle";
    const left = Math.round(12 * k), avail = (btns.length ? btns[btns.length - 1].x : w) - left - 8;
    let t = title;
    while (t.length > 1 && g.measureText(t).width > avail) t = t.slice(0, -2) + "…";
    g.fillText(t, left, Math.round(h / 2) + 1);
    for (const b of btns) {
      const hot = hover === b.id, red = hot && b.id === "close";
      if (hot) { g.fillStyle = red ? "#c42b1c" : dark ? "rgba(255,255,255,0.10)" : "rgba(0,0,0,0.06)"; g.fillRect(b.x, 0, c.bw, h); }
      const cx = b.x + c.bw / 2, cy = h / 2, s = 5 * k;
      g.strokeStyle = red ? "#ffffff" : fg; g.lineWidth = Math.max(1, k); g.beginPath();
      if (b.id === "min") { g.moveTo(cx - s, cy + 0.5); g.lineTo(cx + s, cy + 0.5); }
      else if (b.id === "max") {
        if (c.flags & 16) { g.rect(cx - s, cy - s + 2 * k, s * 2 - 2 * k, s * 2 - 2 * k); g.moveTo(cx - s + 2 * k, cy - s + 2 * k); g.lineTo(cx - s + 2 * k, cy - s); g.lineTo(cx + s, cy - s); g.lineTo(cx + s, cy + s - 2 * k); g.lineTo(cx + s - 2 * k, cy + s - 2 * k); }
        else g.rect(cx - s, cy - s, s * 2, s * 2);
      } else { g.moveTo(cx - s, cy - s); g.lineTo(cx + s, cy + s); g.moveTo(cx + s, cy - s); g.lineTo(cx - s, cy + s); }
      g.stroke();
    }
    gl.bindTexture(gl.TEXTURE_2D, e.tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, e.cv);
  };
  let ext = null; // texture of the externally rendered (3D) frame
  return {
    kind: "webgl2",
    /** Draw the desktop. `buf`: the wasm memory; `scene`: Uint32Array/Int32Array of the kernel's scene (see plonk_lx_scene);
     *  `over`: null or { source (ImageBitmap | canvas | { w, h, rgba }), x, y, w, h } drawn on top. */
    /** Use an image (e.g. an SVG) as the desktop wallpaper, scaled to cover the screen. */
    setWallpaper(url) {
      const img = new Image();
      img.onload = () => {
        const t = newTex();
        gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, img);
        wall = { tex: t, w: img.naturalWidth || 1920, h: img.naturalHeight || 1080 };
        this.onchange?.();
      };
      img.src = url;
    },
    /** Dark or light chrome. */
    setDark(d) { dark = !!d; },
    /** The pointer moved (screen coordinates; null = left): true when a title bar highlight changed, so a redraw is needed. */
    setPointer(x, y) { const k = x == null ? "" : hoverOf(x, y); const ch = k !== hoverKey; hoverKey = k; return ch; },
    /** Draw the desktop. `titleOf(hwnd)` gives a window's title. */
    frame(buf, scene, W, H, over, titleOf) {
      gl.viewport(0, 0, W, H);
      gl.uniform2f(u.screen, W, H);
      gl.clearColor(58 / 255, 110 / 255, 165 / 255, 1); gl.clear(gl.COLOR_BUFFER_BIT);
      const s32 = new Int32Array(scene.buffer, scene.byteOffset, scene.length);
      if (window.__dbg) window.__dbgLast = Array.from(s32);
      const seen = new Set();
      gl.activeTexture(gl.TEXTURE0);
      if (wall) { // cover the screen, centred
        const k = Math.max(W / wall.w, H / wall.h), ww = wall.w * k, wh = wall.h * k;
        gl.bindTexture(gl.TEXTURE_2D, wall.tex);
        quad((W - ww) / 2, (H - wh) / 2, ww, wh, false);
      }
      let activeIdx = -1;
      for (let i = 0, n = scene[0]; i < n; i++) if (s32[1 + i * 17 + 10] & 1) activeIdx = i; // the topmost window with a title bar
      const tops = [];
      for (let i = 0, n = scene[0]; i < n; i++) {
        const o = 1 + i * 17, hwnd = scene[o], x = s32[o + 1], y = s32[o + 2], w = scene[o + 3], h = scene[o + 4], ptr = scene[o + 5];
        let [l, t, r, b] = [s32[o + 6], s32[o + 7], s32[o + 8], s32[o + 9]];
        seen.add(hwnd);
        let e = wins.get(hwnd);
        if (!e || e.w !== w || e.h !== h) {
          if (e) gl.deleteTexture(e.tex);
          e = { tex: newTex(), w, h };
          wins.set(hwnd, e);
          gl.bindTexture(gl.TEXTURE_2D, e.tex);
          gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
          [l, t, r, b] = [0, 0, w, h];
        } else gl.bindTexture(gl.TEXTURE_2D, e.tex);
        if (r > l && b > t) { // the surface is BGRX; the shader swaps the channels
          gl.pixelStorei(gl.UNPACK_ROW_LENGTH, w); gl.pixelStorei(gl.UNPACK_SKIP_PIXELS, l); gl.pixelStorei(gl.UNPACK_SKIP_ROWS, t);
          gl.texSubImage2D(gl.TEXTURE_2D, 0, l, t, r - l, b - t, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(buf, ptr, w * h * 4));
          gl.pixelStorei(gl.UNPACK_ROW_LENGTH, 0); gl.pixelStorei(gl.UNPACK_SKIP_PIXELS, 0); gl.pixelStorei(gl.UNPACK_SKIP_ROWS, 0);
        }
let capDraw = null;
        const flags = s32[o + 10];
        if (flags & 1) {
          const c = { hwnd, flags, l: s32[o + 11], t: s32[o + 12], r: s32[o + 13], b: s32[o + 14], bw: s32[o + 15] || 46, wx: x, wy: y, ww: w, wh: h };
          if (c.r > c.l && c.b > c.t) {
            tops.push(c);
            let ce = caps.get(hwnd);
            if (!ce) { ce = {}; caps.set(hwnd, ce); }
            const hv = hoverKey.startsWith(hwnd + ":") ? hoverKey.slice(String(hwnd).length + 1) : "";
            const act = i === activeIdx, key = [scene[o + 16], c.r - c.l, c.b - c.t, flags, act, dark, hv, c.bw].join("|");
            if (ce.key !== key) { ce.key = key; drawCaption(ce, c, titleOf ? titleOf(hwnd) : "", act, hv); }
            capDraw = { tex: ce.tex, l: c.l, t: c.t, r: c.r, b: c.b };
          }
        }
        gl.bindTexture(gl.TEXTURE_2D, e.tex);
        chromeQuad(x, y, w, h, i === activeIdx || (activeIdx < 0 && i === scene[0] - 1), dark, capDraw);
      }
      lastCaps = tops.reverse();
      for (const [h, ce] of caps) if (!seen.has(h)) { if (ce.tex) gl.deleteTexture(ce.tex); caps.delete(h); }
      for (const [h, e] of wins) if (!seen.has(h)) { gl.deleteTexture(e.tex); wins.delete(h); }
      if (over) this.over(over, W, H, false);
    },
    /** Draw only the 3D frame (the canvas keeps what is on it, so a new 3D frame does not need the desktop drawn again). */
    over(over, W, H, setup = true) {
      {
        if (setup) { gl.viewport(0, 0, W, H); gl.uniform2f(u.screen, W, H); gl.activeTexture(gl.TEXTURE0); }
        if (!ext) ext = newTex();
        gl.bindTexture(gl.TEXTURE_2D, ext);
        const s = over.source;
        if (s.rgba) gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, s.w, s.h, 0, gl.RGBA, gl.UNSIGNED_BYTE, s.rgba);
        else gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, s);
        quad(over.x, over.y, over.w, over.h, false);
      }
    },
  };
}
