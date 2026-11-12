// GPU desktop compositor: every top-level window is a texture (only the rectangle that changed is uploaded) and the screen
// is drawn as one quad per window, bottom first, on a single WebGL2 canvas. Frames from the 3D backend are drawn on top of
// the same canvas, so nothing needs a second canvas. createCompositor returns null when WebGL2 is not available.
const VS = `#version 300 es
uniform vec4 rect; uniform vec2 screen; out vec2 uv;
void main() {
  vec2 p = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
  uv = p;
  vec2 px = rect.xy + p * rect.zw;
  gl_Position = vec4(px.x / screen.x * 2.0 - 1.0, 1.0 - px.y / screen.y * 2.0, 0.0, 1.0);
}`;
const FS = `#version 300 es
precision mediump float;
uniform sampler2D t; uniform float bgr; in vec2 uv; out vec4 o;
void main() { vec4 c = texture(t, uv); o = vec4(bgr > 0.5 ? c.bgr : c.rgb, 1.0); }`;

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
  const u = { rect: gl.getUniformLocation(prog, "rect"), screen: gl.getUniformLocation(prog, "screen"), bgr: gl.getUniformLocation(prog, "bgr") };
  gl.uniform1i(gl.getUniformLocation(prog, "t"), 0);
  gl.disable(gl.BLEND);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
  const newTex = () => {
    const t = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return t;
  };
  const wins = new Map(); // hwnd -> { tex, w, h }
  const quad = (x, y, w, h, bgr) => { gl.uniform4f(u.rect, x, y, w, h); gl.uniform1f(u.bgr, bgr ? 1 : 0); gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4); };
  let ext = null; // texture of the externally rendered (3D) frame
  return {
    kind: "webgl2",
    /** Draw the desktop. `buf`: the wasm memory; `scene`: Uint32Array/Int32Array of the kernel's scene (see plonk_lx_scene);
     *  `over`: null or { source (ImageBitmap | canvas | { w, h, rgba }), x, y, w, h } drawn on top. */
    frame(buf, scene, W, H, over) {
      gl.viewport(0, 0, W, H);
      gl.uniform2f(u.screen, W, H);
      gl.clearColor(58 / 255, 110 / 255, 165 / 255, 1); gl.clear(gl.COLOR_BUFFER_BIT);
      const s32 = new Int32Array(scene.buffer, scene.byteOffset, scene.length);
      const seen = new Set();
      gl.activeTexture(gl.TEXTURE0);
      for (let i = 0, n = scene[0]; i < n; i++) {
        const o = 1 + i * 10, hwnd = scene[o], x = s32[o + 1], y = s32[o + 2], w = scene[o + 3], h = scene[o + 4], ptr = scene[o + 5];
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
        quad(x, y, w, h, true);
      }
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
