// WebGL2 renderer for the Plonk GL core: GLSL ES 3.00 fixed-function uber-shader, renders into its own FBO, synchronous readback.
import { U, VSTRIDE } from "./gl_core.mjs";

const VS = `#version 300 es
precision highp float;
precision highp int;
uniform vec4 u[${U.COUNT}];
layout(location=0) in vec4 aPos;
layout(location=1) in vec3 aNrm;
layout(location=2) in vec4 aCol;
layout(location=3) in vec4 aTc;
out vec4 vCol;
out vec4 vBCol;
out vec4 vTc;
out float vFz;
mat4 M(int i) { return mat4(u[i], u[i+1], u[i+2], u[i+3]); }
vec4 lit(vec3 P, vec3 N, vec4 vc, int mb, int cm) {
  vec4 ma = u[mb], md = u[mb+1], ms = u[mb+2], me = u[mb+3];
  float sh = u[mb+4].x;
  if (cm == 1) ma = vc; else if (cm == 2) md = vc; else if (cm == 3) { ma = vc; md = vc; } else if (cm == 4) ms = vc; else if (cm == 5) me = vc;
  vec3 c = me.rgb + ma.rgb * u[${U.LMAMB}].rgb;
  vec3 V = u[${U.F1}].w > 0.5 ? normalize(-P) : vec3(0.0, 0.0, 1.0);
  for (int i = 0; i < 8; i++) {
    int lb = ${U.LIGHT} + i * ${U.LSTRIDE};
    if (u[lb+6].x < 0.5) continue;
    vec4 lp = u[lb+3];
    vec3 L; float att = 1.0;
    if (lp.w == 0.0) { L = normalize(lp.xyz); }
    else {
      vec3 d = lp.xyz - P; float dist = max(length(d), 1e-6); L = d / dist;
      vec4 at = u[lb+5]; att = 1.0 / (at.x + at.y * dist + at.z * dist * dist);
      if (at.w < 180.0) {
        float sd = dot(-L, normalize(u[lb+4].xyz));
        if (sd < cos(radians(at.w))) att = 0.0; else att *= pow(max(sd, 0.0), u[lb+4].w);
      }
    }
    float ndl = max(dot(N, L), 0.0);
    vec3 col = u[lb].rgb * ma.rgb + ndl * u[lb+1].rgb * md.rgb;
    if (ndl > 0.0) {
      vec3 H = normalize(L + V);
      float nh = max(dot(N, H), 0.0);
      float s = sh > 0.0 ? pow(nh, sh) : 1.0;
      col += s * u[lb+2].rgb * ms.rgb;
    }
    c += att * col;
  }
  return vec4(clamp(c, 0.0, 1.0), md.a);
}
void main() {
  vec4 pe = M(${U.MV}) * aPos;
  gl_Position = M(${U.PROJ}) * pe;
  vFz = abs(pe.z);
  vTc = M(${U.TEXM}) * aTc;
  gl_PointSize = u[${U.F3}].z;
  if (u[${U.F1}].x > 0.5) {
    mat3 nm = mat3(u[${U.NRM}].xyz, u[${U.NRM + 1}].xyz, u[${U.NRM + 2}].xyz);
    vec3 N = nm * aNrm;
    float nl = length(N); if (nl > 0.0) N /= nl;
    vec3 P = pe.xyz / (pe.w == 0.0 ? 1.0 : pe.w);
    vCol = lit(P, N, aCol, ${U.MATF}, int(u[${U.F1}].y + 0.5));
    vBCol = u[${U.F1}].z > 0.5 ? lit(P, -N, aCol, ${U.MATB}, int(u[${U.F3}].w + 0.5)) : vCol;
  } else { vCol = aCol; vBCol = aCol; }
}`;

const FS = `#version 300 es
precision highp float;
precision highp int;
uniform vec4 u[${U.COUNT}];
uniform sampler2D uTex;
in vec4 vCol; in vec4 vBCol; in vec4 vTc; in float vFz;
out vec4 oCol;
void main() {
  vec4 c = (u[${U.F1}].z > 0.5 && !gl_FrontFacing) ? vBCol : vCol;
  float q = vTc.w == 0.0 ? 1.0 : vTc.w;
  vec4 t = texture(uTex, vTc.xy / q);
  if (u[${U.F2}].y > 0.5) {
    int m = int(u[${U.F2}].z + 0.5);
    if (m == 1) c = t;
    else if (m == 2) c = vec4(mix(c.rgb, t.rgb, t.a), c.a);
    else if (m == 3) c = vec4(c.rgb + t.rgb, c.a * t.a);
    else c = c * t;
  }
  int fm = int(u[${U.FOG}].x + 0.5);
  if (fm > 0) {
    float f;
    if (fm == 1) f = (u[${U.FOG}].w - vFz) / (u[${U.FOG}].w - u[${U.FOG}].z);
    else if (fm == 2) f = exp(-u[${U.FOG}].y * vFz);
    else { float x = u[${U.FOG}].y * vFz; f = exp(-x * x); }
    f = clamp(f, 0.0, 1.0);
    c.rgb = mix(u[${U.FOGCOL}].rgb, c.rgb, f);
  }
  int af = int(u[${U.F2}].w + 0.5); float rf = u[${U.F3}].x;
  if (af == 0) discard;
  else if (af == 1) { if (!(c.a < rf)) discard; }
  else if (af == 2) { if (!(abs(c.a - rf) < 1.0/512.0)) discard; }
  else if (af == 3) { if (!(c.a <= rf)) discard; }
  else if (af == 4) { if (!(c.a > rf)) discard; }
  else if (af == 5) { if (!(abs(c.a - rf) >= 1.0/512.0)) discard; }
  else if (af == 6) { if (!(c.a >= rf)) discard; }
  oCol = c;
}`;

export function createWebGL2Renderer() {
  const canvas = typeof OffscreenCanvas !== "undefined" ? new OffscreenCanvas(1, 1) : Object.assign(document.createElement("canvas"), { width: 1, height: 1 });
  const gl = canvas.getContext("webgl2", { antialias: false, depth: false, stencil: false, alpha: true, premultipliedAlpha: false, preserveDrawingBuffer: false });
  if (!gl) throw new Error("WebGL2 not available");

  const sh = (type, src) => {
    const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error("shader: " + gl.getShaderInfoLog(s));
    return s;
  };
  const prog = gl.createProgram();
  gl.attachShader(prog, sh(gl.VERTEX_SHADER, VS)); gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, FS));
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error("link: " + gl.getProgramInfoLog(prog));
  gl.useProgram(prog);
  const uLoc = gl.getUniformLocation(prog, "u"), texLoc = gl.getUniformLocation(prog, "uTex");
  gl.uniform1i(texLoc, 0);

  const vao = gl.createVertexArray(); gl.bindVertexArray(vao);
  const vbo = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
  const stride = VSTRIDE * 4;
  [[0, 4, 0], [1, 3, 16], [2, 4, 28], [3, 4, 44]].forEach(([loc, n, off]) => { gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, n, gl.FLOAT, false, stride, off); });

  const white = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, white);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([255, 255, 255, 255]));

  let fbo = null, rbC = null, rbD = null, W = 1, H = 1;
  function makeTargets(w, h) {
    W = Math.max(1, w); H = Math.max(1, h);
    canvas.width = W; canvas.height = H;
    if (fbo) { gl.deleteFramebuffer(fbo); gl.deleteRenderbuffer(rbC); gl.deleteRenderbuffer(rbD); }
    fbo = gl.createFramebuffer(); rbC = gl.createRenderbuffer(); rbD = gl.createRenderbuffer();
    gl.bindRenderbuffer(gl.RENDERBUFFER, rbC); gl.renderbufferStorage(gl.RENDERBUFFER, gl.RGBA8, W, H);
    gl.bindRenderbuffer(gl.RENDERBUFFER, rbD); gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT24, W, H);
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, rbC);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, rbD);
    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) throw new Error("framebuffer incomplete");
    gl.disable(gl.SCISSOR_TEST); gl.colorMask(true, true, true, true); gl.depthMask(true);
    gl.clearColor(0, 0, 0, 0); gl.clearDepth(1); gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
  }
  function applyScissor(st) {
    if (st.scissorTest) { gl.enable(gl.SCISSOR_TEST); gl.scissor(st.scissor[0], st.scissor[1], Math.max(0, st.scissor[2]), Math.max(0, st.scissor[3])); }
    else gl.disable(gl.SCISSOR_TEST);
  }
  const VALID_MIN = new Set([0x2600, 0x2601, 0x2700, 0x2701, 0x2702, 0x2703]);
  const wrapOf = (v) => (v === 0x2901 ? gl.REPEAT : v === 0x8370 ? gl.MIRRORED_REPEAT : gl.CLAMP_TO_EDGE);

  return {
    kind: "webgl2",
    create(w, h) { makeTargets(w, h); },
    resize(w, h) { makeTargets(w, h); },
    clear(mask, st) {
      applyScissor(st);
      gl.colorMask(...st.colorMask); gl.depthMask(st.depthMask);
      let bits = 0;
      if (mask & 0x4000) { gl.clearColor(...st.clearColor); bits |= gl.COLOR_BUFFER_BIT; }
      if (mask & 0x100) { gl.clearDepth(st.clearDepth); bits |= gl.DEPTH_BUFFER_BIT; }
      if (bits) gl.clear(bits);
    },
    draw(st, u, tex, verts, n, topo) {
      gl.viewport(st.viewport[0], st.viewport[1], Math.max(0, st.viewport[2]), Math.max(0, st.viewport[3]));
      applyScissor(st);
      if (st.depthTest) { gl.enable(gl.DEPTH_TEST); gl.depthFunc(st.depthFunc); } else gl.disable(gl.DEPTH_TEST);
      gl.depthMask(st.depthMask);
      if (st.cullFace && topo === 2) { gl.enable(gl.CULL_FACE); gl.cullFace(st.cullMode); } else gl.disable(gl.CULL_FACE);
      gl.frontFace(st.frontFace);
      if (st.blend) { gl.enable(gl.BLEND); gl.blendFunc(st.blendSrc, st.blendDst); } else gl.disable(gl.BLEND);
      gl.colorMask(...st.colorMask);
      gl.uniform4fv(uLoc, u);
      gl.activeTexture(gl.TEXTURE0);
      if (tex && tex.gpu) {
        gl.bindTexture(gl.TEXTURE_2D, tex.gpu.t);
        const mip = tex.min >= 0x2700 && tex.min <= 0x2703 && VALID_MIN.has(tex.min);
        if (tex.gpu.pv !== tex.pv) {
          tex.gpu.pv = tex.pv;
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, VALID_MIN.has(tex.min) ? tex.min : gl.LINEAR);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, tex.mag === 0x2600 ? gl.NEAREST : gl.LINEAR);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, wrapOf(tex.wrapS));
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, wrapOf(tex.wrapT));
        }
        if (mip && tex.mipDirty) { gl.generateMipmap(gl.TEXTURE_2D); tex.mipDirty = false; }
      } else gl.bindTexture(gl.TEXTURE_2D, white);
      gl.bufferData(gl.ARRAY_BUFFER, verts.subarray(0, n * VSTRIDE), gl.STREAM_DRAW);
      gl.drawArrays(topo === 0 ? gl.POINTS : topo === 1 ? gl.LINES : gl.TRIANGLES, 0, n);
    },
    texUpload(tex, x, y, w, h, data, full) {
      if (!tex.gpu) tex.gpu = { t: gl.createTexture(), pv: -1 };
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, tex.gpu.t);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
      if (full) gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, data);
      else gl.texSubImage2D(gl.TEXTURE_2D, 0, x, y, w, h, gl.RGBA, gl.UNSIGNED_BYTE, data);
      tex.gpu.pv = -1; // re-apply params (and mipmaps) lazily
    },
    texDelete(tex) { if (tex.gpu) { gl.deleteTexture(tex.gpu.t); tex.gpu = null; } },
    presentDirect(_w, _h, cb) {
      if (!canvas.transferToImageBitmap) return false;
      gl.disable(gl.SCISSOR_TEST); gl.colorMask(true, true, true, true);
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, fbo); gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, null);
      gl.blitFramebuffer(0, 0, W, H, 0, 0, W, H, gl.COLOR_BUFFER_BIT, gl.NEAREST); // canvas is bottom-up like the FBO: no flip
      const bmp = canvas.transferToImageBitmap();
      gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
      cb(W, H, bmp);
      return true;
    },
    present(_w, _h, cb) {
      gl.disable(gl.SCISSOR_TEST);
      const buf = new Uint8Array(W * H * 4);
      gl.readPixels(0, 0, W, H, gl.RGBA, gl.UNSIGNED_BYTE, buf);
      const out = new Uint8Array(W * H * 4), row = W * 4;
      for (let y = 0; y < H; y++) out.set(buf.subarray((H - 1 - y) * row, (H - y) * row), y * row);
      cb(W, H, out);
    },
  };
}
