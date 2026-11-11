// WebGPU renderer for the Plonk GL core (best effort): WGSL fixed-function uber-shader, pipeline cache keyed by render state,
// z remapped from GL clip space [-1,1] to [0,1]. Frame readback is asynchronous (copyTextureToBuffer + mapAsync), so
// onPresent is called later than execute() returns. Limitations: no mipmaps (min filter approximated), points are 1px,
// line width is 1.
import { U, VSTRIDE } from "./gl_core.mjs";

const WGSL = `
@group(0) @binding(0) var<uniform> u : array<vec4<f32>, ${U.COUNT}>;
@group(0) @binding(1) var tex : texture_2d<f32>;
@group(0) @binding(2) var smp : sampler;

struct VIn { @location(0) pos: vec4<f32>, @location(1) nrm: vec3<f32>, @location(2) col: vec4<f32>, @location(3) tc: vec4<f32> };
struct VOut {
  @builtin(position) p: vec4<f32>,
  @location(0) col: vec4<f32>, @location(1) bcol: vec4<f32>, @location(2) tc: vec4<f32>, @location(3) fz: f32,
};

fn M(i: u32) -> mat4x4<f32> { return mat4x4<f32>(u[i], u[i + 1u], u[i + 2u], u[i + 3u]); }

fn lit(P: vec3<f32>, N: vec3<f32>, vc: vec4<f32>, mb: u32, cm: i32) -> vec4<f32> {
  var ma = u[mb]; var md = u[mb + 1u]; var ms = u[mb + 2u]; var me = u[mb + 3u];
  let sh = u[mb + 4u].x;
  if (cm == 1) { ma = vc; } else if (cm == 2) { md = vc; } else if (cm == 3) { ma = vc; md = vc; } else if (cm == 4) { ms = vc; } else if (cm == 5) { me = vc; }
  var c = me.rgb + ma.rgb * u[${U.LMAMB}].rgb;
  var V = vec3<f32>(0.0, 0.0, 1.0);
  if (u[${U.F1}].w > 0.5) { V = normalize(-P); }
  for (var i = 0u; i < 8u; i = i + 1u) {
    let lb = ${U.LIGHT}u + i * ${U.LSTRIDE}u;
    if (u[lb + 6u].x < 0.5) { continue; }
    let lp = u[lb + 3u];
    var L: vec3<f32>;
    var att = 1.0;
    if (lp.w == 0.0) { L = normalize(lp.xyz); }
    else {
      let d = lp.xyz - P; let dist = max(length(d), 1e-6); L = d / dist;
      let at = u[lb + 5u]; att = 1.0 / (at.x + at.y * dist + at.z * dist * dist);
      if (at.w < 180.0) {
        let sd = dot(-L, normalize(u[lb + 4u].xyz));
        if (sd < cos(radians(at.w))) { att = 0.0; } else { att = att * pow(max(sd, 0.0), u[lb + 4u].w); }
      }
    }
    let ndl = max(dot(N, L), 0.0);
    var col = u[lb].rgb * ma.rgb + ndl * u[lb + 1u].rgb * md.rgb;
    if (ndl > 0.0) {
      let H = normalize(L + V);
      let nh = max(dot(N, H), 0.0);
      var s = 1.0;
      if (sh > 0.0) { s = pow(nh, sh); }
      col = col + s * u[lb + 2u].rgb * ms.rgb;
    }
    c = c + att * col;
  }
  return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), md.a);
}

@vertex fn vs(v: VIn) -> VOut {
  var o: VOut;
  let pe = M(${U.MV}u) * v.pos;
  var cp = M(${U.PROJ}u) * pe;
  cp.z = (cp.z + cp.w) * 0.5; // GL clip z [-w,w] -> WebGPU [0,w]
  o.p = cp;
  o.fz = abs(pe.z);
  o.tc = M(${U.TEXM}u) * v.tc;
  if (u[${U.F1}].x > 0.5) {
    let nm = mat3x3<f32>(u[${U.NRM}].xyz, u[${U.NRM + 1}].xyz, u[${U.NRM + 2}].xyz);
    var N = nm * v.nrm;
    let nl = length(N);
    if (nl > 0.0) { N = N / nl; }
    var w = pe.w; if (w == 0.0) { w = 1.0; }
    let P = pe.xyz / w;
    o.col = lit(P, N, v.col, ${U.MATF}u, i32(u[${U.F1}].y + 0.5));
    o.bcol = o.col;
    if (u[${U.F1}].z > 0.5) { o.bcol = lit(P, -N, v.col, ${U.MATB}u, i32(u[${U.F3}].w + 0.5)); }
  } else { o.col = v.col; o.bcol = v.col; }
  return o;
}

@fragment fn fs(i: VOut, @builtin(front_facing) ff: bool) -> @location(0) vec4<f32> {
  var c = i.col;
  if (u[${U.F1}].z > 0.5 && !ff) { c = i.bcol; }
  var q = i.tc.w; if (q == 0.0) { q = 1.0; }
  let t = textureSample(tex, smp, i.tc.xy / q);
  if (u[${U.F2}].y > 0.5) {
    let m = i32(u[${U.F2}].z + 0.5);
    if (m == 1) { c = t; }
    else if (m == 2) { c = vec4<f32>(mix(c.rgb, t.rgb, t.a), c.a); }
    else if (m == 3) { c = vec4<f32>(c.rgb + t.rgb, c.a * t.a); }
    else { c = c * t; }
  }
  let fm = i32(u[${U.FOG}].x + 0.5);
  if (fm > 0) {
    var f = 0.0;
    if (fm == 1) { f = (u[${U.FOG}].w - i.fz) / (u[${U.FOG}].w - u[${U.FOG}].z); }
    else if (fm == 2) { f = exp(-u[${U.FOG}].y * i.fz); }
    else { let x = u[${U.FOG}].y * i.fz; f = exp(-x * x); }
    f = clamp(f, 0.0, 1.0);
    c = vec4<f32>(mix(u[${U.FOGCOL}].rgb, c.rgb, f), c.a);
  }
  let af = i32(u[${U.F2}].w + 0.5); let rf = u[${U.F3}].x;
  if (af == 0) { discard; }
  else if (af == 1) { if (!(c.a < rf)) { discard; } }
  else if (af == 2) { if (!(abs(c.a - rf) < 1.0 / 512.0)) { discard; } }
  else if (af == 3) { if (!(c.a <= rf)) { discard; } }
  else if (af == 4) { if (!(c.a > rf)) { discard; } }
  else if (af == 5) { if (!(abs(c.a - rf) >= 1.0 / 512.0)) { discard; } }
  else if (af == 6) { if (!(c.a >= rf)) { discard; } }
  return c;
}
`;

const DEPTH_FN = { 0x200: "never", 0x201: "less", 0x202: "equal", 0x203: "less-equal", 0x204: "greater", 0x205: "not-equal", 0x206: "greater-equal", 0x207: "always" };
const BLEND_F = { 0: "zero", 1: "one", 0x300: "src", 0x301: "one-minus-src", 0x302: "src-alpha", 0x303: "one-minus-src-alpha", 0x304: "dst-alpha",
  0x305: "one-minus-dst-alpha", 0x306: "dst", 0x307: "one-minus-dst", 0x308: "src-alpha-saturated" };
const TOPO = ["point-list", "line-list", "triangle-list"];
const UB_SLOT = Math.ceil((U.COUNT * 16) / 256) * 256;
const UB_SLOTS = 2048, VB_SIZE = 8 << 20;

export async function createWebGPURenderer() {
  if (typeof navigator === "undefined" || !navigator.gpu) throw new Error("WebGPU not available");
  const adapter = await navigator.gpu.requestAdapter();
  if (!adapter) throw new Error("no WebGPU adapter");
  const device = await adapter.requestDevice();
  let lost = false;
  device.lost.then((i) => { lost = true; console.warn("WebGPU device lost:", i.message); });

  device.pushErrorScope("validation");
  const module = device.createShaderModule({ code: WGSL });
  const bgl = device.createBindGroupLayout({ entries: [
    { binding: 0, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, buffer: { type: "uniform", hasDynamicOffset: true, minBindingSize: U.COUNT * 16 } },
    { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "float" } },
    { binding: 2, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
  ] });
  const layout = device.createPipelineLayout({ bindGroupLayouts: [bgl] });
  const ub = device.createBuffer({ size: UB_SLOT * UB_SLOTS, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
  const vb = device.createBuffer({ size: VB_SIZE, usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST });

  const pipes = new Map();
  function getPipe(topo, dt, dfn, dmask, blend, bs, bd, cmask, cull, ff) {
    const key = [topo, dt ? dfn : "x", dt && dmask ? 1 : 0, blend ? bs + "," + bd : "-", cmask, cull, ff].join("|");
    let p = pipes.get(key);
    if (p) return p;
    const bf = (f) => BLEND_F[f] ?? "one";
    const target = { format: "rgba8unorm", writeMask: cmask };
    if (blend) { const c = { operation: "add", srcFactor: bf(bs), dstFactor: bf(bd) }; target.blend = { color: c, alpha: { ...c } }; }
    p = device.createRenderPipeline({
      layout,
      vertex: { module, entryPoint: "vs", buffers: [{ arrayStride: VSTRIDE * 4, attributes: [
        { shaderLocation: 0, offset: 0, format: "float32x4" }, { shaderLocation: 1, offset: 16, format: "float32x3" },
        { shaderLocation: 2, offset: 28, format: "float32x4" }, { shaderLocation: 3, offset: 44, format: "float32x4" }] }] },
      fragment: { module, entryPoint: "fs", targets: [target] },
      primitive: { topology: TOPO[topo], frontFace: ff, cullMode: cull },
      depthStencil: { format: "depth24plus", depthCompare: dt ? DEPTH_FN[dfn] ?? "less" : "always", depthWriteEnabled: !!(dt && dmask) },
    });
    pipes.set(key, p);
    return p;
  }
  // default pipeline + validation: any shader/pipeline error here makes init fail so the caller falls back to WebGL2
  getPipe(2, true, 0x201, true, true, 0x302, 0x303, 15, "back", "ccw");
  const err = await device.popErrorScope();
  if (err) throw new Error("WebGPU validation: " + err.message);
  device.addEventListener("uncapturederror", (e) => console.warn("WebGPU error:", e.error?.message));

  const samplers = new Map();
  const getSampler = (t) => {
    const key = [t.min, t.mag, t.wrapS, t.wrapT].join();
    let s = samplers.get(key);
    if (!s) {
      const addr = (v) => (v === 0x2901 ? "repeat" : v === 0x8370 ? "mirror-repeat" : "clamp-to-edge");
      s = { key, s: device.createSampler({ magFilter: t.mag === 0x2600 ? "nearest" : "linear", minFilter: t.min === 0x2600 || t.min === 0x2700 || t.min === 0x2702 ? "nearest" : "linear",
        addressModeU: addr(t.wrapS), addressModeV: addr(t.wrapT) }) };
      samplers.set(key, s);
    }
    return s;
  };
  const mkTex = (w, h) => {
    const texture = device.createTexture({ size: [w, h], format: "rgba8unorm", usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST });
    return { texture, view: texture.createView(), w, h, bgs: new Map() };
  };
  const white = mkTex(1, 1);
  device.queue.writeTexture({ texture: white.texture }, new Uint8Array([255, 255, 255, 255]), { bytesPerRow: 4 }, [1, 1]);
  const bindGroupFor = (g, samp) => {
    let bg = g.bgs.get(samp.key);
    if (!bg) {
      bg = device.createBindGroup({ layout: bgl, entries: [{ binding: 0, resource: { buffer: ub, size: U.COUNT * 16 } },
        { binding: 1, resource: g.view }, { binding: 2, resource: samp.s }] });
      g.bgs.set(samp.key, bg);
    }
    return bg;
  };
  const whiteSampler = { key: "w", s: device.createSampler({}) };

  let W = 1, H = 1, color = null, depth = null, colorView = null, depthView = null;
  let enc = null, pass = null, pc = null, pd = null, ubSlot = 0, vbOff = 0; let temps = [];

  function endPass() { if (pass) { pass.end(); pass = null; } }
  function flush() {
    endPass();
    if (enc) { device.queue.submit([enc.finish()]); enc = null; }
    ubSlot = 0; vbOff = 0;
    for (const b of temps) b.destroy();
    temps = [];
  }
  function ensurePass() {
    if (pass) return;
    if (!enc) enc = device.createCommandEncoder();
    pass = enc.beginRenderPass({
      colorAttachments: [{ view: colorView, loadOp: pc ? "clear" : "load", clearValue: pc ?? { r: 0, g: 0, b: 0, a: 0 }, storeOp: "store" }],
      depthStencilAttachment: { view: depthView, depthLoadOp: pd != null ? "clear" : "load", depthClearValue: pd ?? 1, depthStoreOp: "store" },
    });
    pc = null; pd = null;
  }
  function makeTargets(w, h) {
    flush();
    color?.destroy(); depth?.destroy();
    W = Math.max(1, w); H = Math.max(1, h);
    color = device.createTexture({ size: [W, H], format: "rgba8unorm", usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC });
    depth = device.createTexture({ size: [W, H], format: "depth24plus", usage: GPUTextureUsage.RENDER_ATTACHMENT });
    colorView = color.createView(); depthView = depth.createView();
    pc = { r: 0, g: 0, b: 0, a: 0 }; pd = 1;
  }

  const clampRect = (x, y, w, h) => { // GL (bottom-left origin) rect -> WebGPU (top-left), clamped inside the surface
    let l = Math.max(0, x), t = Math.max(0, H - (y + h)), rr = Math.min(W, x + w), b = Math.min(H, H - y);
    return [l, t, Math.max(0, rr - l), Math.max(0, b - t)];
  };

  function drawRaw(st, u, tex, verts, n, topo) {
    if (lost || n <= 0) return;
    const cull = st.cullFace && topo === 2 ? (st.cullMode === 0x404 ? "front" : st.cullMode === 0x405 ? "back" : null) : "none";
    if (cull === null) return; // GL_FRONT_AND_BACK culls every polygon
    const vp = clampRect(st.viewport[0], st.viewport[1], st.viewport[2], st.viewport[3]);
    if (vp[2] <= 0 || vp[3] <= 0) return;
    const sc = st.scissorTest ? clampRect(st.scissor[0], st.scissor[1], st.scissor[2], st.scissor[3]) : [0, 0, W, H];
    if (sc[2] <= 0 || sc[3] <= 0) return;
    const bytes = n * VSTRIDE * 4;
    if (ubSlot >= UB_SLOTS || (bytes <= VB_SIZE && vbOff + bytes > VB_SIZE)) flush();
    ensurePass();
    const cm = (st.colorMask[0] ? 1 : 0) | (st.colorMask[1] ? 2 : 0) | (st.colorMask[2] ? 4 : 0) | (st.colorMask[3] ? 8 : 0);
    const pipe = getPipe(topo, st.depthTest, st.depthFunc, st.depthMask, st.blend, st.blendSrc, st.blendDst, cm, cull, st.frontFace === 0x900 ? "cw" : "ccw");
    const ubOff = ubSlot * UB_SLOT; ubSlot++;
    device.queue.writeBuffer(ub, ubOff, u, 0, U.COUNT * 4);
    let vbuf = vb, voff = vbOff;
    if (bytes > VB_SIZE) { vbuf = device.createBuffer({ size: bytes, usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST }); temps.push(vbuf); voff = 0; }
    else vbOff += bytes;
    device.queue.writeBuffer(vbuf, voff, verts, 0, n * VSTRIDE);
    const g = tex?.gpu ?? white, samp = tex?.gpu ? getSampler(tex) : whiteSampler;
    pass.setPipeline(pipe);
    pass.setBindGroup(0, bindGroupFor(g, samp), [ubOff]);
    pass.setVertexBuffer(0, vbuf, voff, bytes);
    pass.setViewport(vp[0], vp[1], vp[2], vp[3], 0, 1);
    pass.setScissorRect(sc[0], sc[1], sc[2], sc[3]);
    pass.draw(n);
  }

  const idm = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
  const quadU = new Float32Array(U.COUNT * 4);
  for (const b of [U.MV, U.PROJ, U.TEXM]) quadU.set(idm, b * 4);
  for (let i = 0; i < 3; i++) quadU[(U.NRM + i) * 4 + i] = 1;
  quadU[U.F2 * 4 + 3] = 7;

  return {
    kind: "webgpu",
    create(w, h) { makeTargets(w, h); },
    resize(w, h) { makeTargets(w, h); },
    clear(mask, st) {
      const doC = !!(mask & 0x4000), doD = !!(mask & 0x100);
      if (!doC && !doD) return;
      const sc = st.scissorTest ? st.scissor : null;
      const full = !sc || (sc[0] <= 0 && sc[1] <= 0 && sc[0] + sc[2] >= W && sc[1] + sc[3] >= H);
      const cFull = doC && full && st.colorMask.every(Boolean), dFull = doD && full && st.depthMask;
      if (cFull || dFull) {
        endPass();
        if (cFull) { const c = st.clearColor; pc = { r: c[0], g: c[1], b: c[2], a: c[3] }; }
        if (dFull) pd = Math.min(1, Math.max(0, st.clearDepth));
      }
      const cPart = doC && !cFull, dPart = doD && !dFull && st.depthMask;
      if (!cPart && !dPart) return;
      // partial clear (scissor / masks): draw a full-surface quad with the clear colour/depth
      const c = st.clearColor, z = Math.min(1, Math.max(0, st.clearDepth)) * 2 - 1;
      const v = new Float32Array(6 * VSTRIDE);
      [[-1, -1], [1, -1], [1, 1], [-1, -1], [1, 1], [-1, 1]].forEach(([x, y], i) => { v.set([x, y, z, 1, 0, 0, 1, c[0], c[1], c[2], c[3], 0, 0, 0, 1], i * VSTRIDE); });
      drawRaw({ depthTest: true, depthFunc: 0x207, depthMask: dPart, blend: false, blendSrc: 1, blendDst: 0, cullFace: false, frontFace: 0x901,
        colorMask: cPart ? st.colorMask : [false, false, false, false], scissorTest: st.scissorTest, scissor: st.scissor, viewport: [0, 0, W, H] }, quadU, null, v, 6, 2);
    },
    draw: drawRaw,
    texUpload(tex, x, y, w, h, data, full) {
      if (enc) flush(); // writeTexture happens on the queue timeline: make sure earlier draws were submitted first
      if (full) {
        if (!tex.gpu || tex.gpu.w !== w || tex.gpu.h !== h) { tex.gpu?.texture.destroy(); tex.gpu = mkTex(w, h); }
      }
      if (!tex.gpu) return;
      device.queue.writeTexture({ texture: tex.gpu.texture, origin: [x, y] }, data, { bytesPerRow: w * 4, rowsPerImage: h }, [w, h]);
    },
    texDelete(tex) { if (tex.gpu) { flush(); tex.gpu.texture.destroy(); tex.gpu = null; } },
    present(_w, _h, cb) {
      if (lost) return;
      if (pc || pd != null) ensurePass();
      endPass();
      if (!enc) enc = device.createCommandEncoder();
      const bpr = Math.ceil((W * 4) / 256) * 256, w = W, h = H;
      const buf = device.createBuffer({ size: bpr * h, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
      enc.copyTextureToBuffer({ texture: color }, { buffer: buf, bytesPerRow: bpr, rowsPerImage: h }, [w, h]);
      flush();
      buf.mapAsync(GPUMapMode.READ).then(() => {
        const src = new Uint8Array(buf.getMappedRange()), out = new Uint8Array(w * h * 4);
        for (let y = 0; y < h; y++) out.set(src.subarray(y * bpr, y * bpr + w * 4), y * w * 4);
        buf.unmap(); buf.destroy();
        cb(w, h, out);
      }).catch((e) => console.warn("WebGPU readback failed:", e));
    },
  };
}
