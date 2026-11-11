// Renderer-independent core of the Plonk GL backend (see docs/GL_PROTOCOL.md).
// Parses the command stream, tracks all OpenGL 1.x fixed-function state, matrix stacks, display lists, immediate-mode batching,
// light/material/fog/texture bookkeeping, and hands finished draws to a thin renderer (WebGL2 or WebGPU).
//
// Renderer interface (all calls synchronous):
//   create(w,h)                      (re)create the surface (color + depth)
//   resize(w,h)                      surface size changed (contents may be lost)
//   clear(mask, st)                  honours st.scissorTest/scissor, st.colorMask, st.depthMask, st.clearColor, st.clearDepth
//   draw(st, u, tex, verts, n, topo) st = live state (read synchronously!), u = Float32Array(U.COUNT*4) uniform block (copy it!),
//                                    tex = texture object or null, verts = Float32Array stride VSTRIDE (n vertices), topo 0 points/1 lines/2 triangles
//   texUpload(tex,x,y,w,h,data,full) full => (re)define level 0 with size tex.w x tex.h
//   texDelete(tex)
//   present(w,h,cb)                  cb(w,h,rgba) with rgba top row first (may be called later)

export const VSTRIDE = 15; // pos4 nrm3 col4 tc4 (floats)
// Uniform block layout, in vec4 units (shared by the GLSL and WGSL shaders).
export const U = {
  MV: 0, PROJ: 4, TEXM: 8, NRM: 12,       // mat4 x3, then normal matrix as 3 vec4 columns
  LMAMB: 16,                              // light model ambient
  FOG: 17,                                // (mode 0 off/1 linear/2 exp/3 exp2, density, start, end)
  FOGCOL: 18,
  F1: 19,                                 // (lighting, colorMaterial mode for front, twoSide, localViewer)
  F2: 20,                                 // (normalize, texture on, tex env 0 mod/1 rep/2 decal/3 add, alpha func 0..7 (7 = always))
  F3: 21,                                 // (alpha ref, 0, point size, colorMaterial mode for back)
  MATF: 22, MATB: 27,                     // material: amb, diff, spec, emission, (shininess,...)
  LIGHT: 32, LSTRIDE: 7,                  // per light: amb, diff, spec, pos(eye), spotdir(eye)+exp, (c,l,q,cutoff), (enabled,...)
  COUNT: 32 + 8 * 7,
};

const IDENT = () => new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
function mulInto(out, a, b) { // out = a * b (column-major), out may alias neither
  for (let c = 0; c < 4; c++) for (let r = 0; r < 4; r++) {
    out[c * 4 + r] = a[r] * b[c * 4] + a[4 + r] * b[c * 4 + 1] + a[8 + r] * b[c * 4 + 2] + a[12 + r] * b[c * 4 + 3];
  }
}
// normal matrix = inverse transpose of upper-left 3x3 of m; written as 3 vec4 columns into out[o..o+11]
function normalMatrix(m, out, o) {
  const c0 = [m[0], m[1], m[2]], c1 = [m[4], m[5], m[6]], c2 = [m[8], m[9], m[10]];
  const cr = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  const x12 = cr(c1, c2), x20 = cr(c2, c0), x01 = cr(c0, c1);
  const det = c0[0] * x12[0] + c0[1] * x12[1] + c0[2] * x12[2];
  let cols;
  if (Math.abs(det) < 1e-30 || !isFinite(det)) cols = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
  else cols = [x12, x20, x01].map((v) => v.map((q) => q / det));
  for (let i = 0; i < 3; i++) { out[o + i * 4] = cols[i][0]; out[o + i * 4 + 1] = cols[i][1]; out[o + i * 4 + 2] = cols[i][2]; out[o + i * 4 + 3] = 0; }
}

const newMat = (a, d, s, e, sh) => ({ amb: a, dif: d, spe: s, emi: e, shin: sh });
function defaultState(w, h) {
  const lights = [];
  for (let i = 0; i < 8; i++) lights.push({
    amb: [0, 0, 0, 1], dif: i === 0 ? [1, 1, 1, 1] : [0, 0, 0, 1], spe: i === 0 ? [1, 1, 1, 1] : [0, 0, 0, 1],
    pos: [0, 0, 1, 0], dir: [0, 0, -1], exp: 0, cut: 180, att: [1, 0, 0], en: false,
  });
  return {
    clearColor: [0, 0, 0, 0], clearDepth: 1,
    depthTest: false, depthFunc: 0x201, depthMask: true,
    cullFace: false, cullMode: 0x405, frontFace: 0x901,
    blend: false, blendSrc: 1, blendDst: 0,
    alphaTest: false, alphaFunc: 0x207, alphaRef: 0,
    flat: false, polyMode: 0x1b02, lineWidth: 1, pointSize: 1,
    viewport: [0, 0, w, h], scissorTest: false, scissor: [0, 0, w, h], colorMask: [true, true, true, true],
    lighting: false, normalize: false, colorMaterial: false, cmFace: 0x408, cmMode: 0x1602, polyOffset: false,
    tex2d: false, bound: 0, texEnv: 0x2100,
    fog: false, fogMode: 0x800, fogDensity: 1, fogStart: 0, fogEnd: 1, fogColor: [0, 0, 0, 0],
    lmAmbient: [0.2, 0.2, 0.2, 1], twoSide: false, localViewer: false,
    matF: newMat([0.2, 0.2, 0.2, 1], [0.8, 0.8, 0.8, 1], [0, 0, 0, 1], [0, 0, 0, 1], 0),
    matB: newMat([0.2, 0.2, 0.2, 1], [0.8, 0.8, 0.8, 1], [0, 0, 0, 1], [0, 0, 0, 1], 0),
    lights,
    col: [1, 1, 1, 1], nrm: [0, 0, 1], tc: [0, 0, 0, 1],
  };
}

const CM_CODE = { 0x1200: 1, 0x1201: 2, 0x1602: 3, 0x1202: 4, 0x1600: 5 };
const NO_RECORD = new Set([1, 2, 45, 46, 48, 49]); // CREATE RESIZE NEW_LIST END_LIST DELETE_LISTS PRESENT are never compiled into lists

export function createCore(r, presentCb) {
  let W = 0, H = 0, created = false;
  let st = defaultState(1, 1);
  let cur = [IDENT(), IDENT(), IDENT()], stacks = [[], [], []], mmode = 0;
  const textures = new Map(), lists = new Map();
  let rec = null, callDepth = 0; const attribs = [];
  let vb = new Float32Array(VSTRIDE * 1024), vn = 0, prim = -1;
  let ob = new Float32Array(VSTRIDE * 1024);
  const uni = new Float32Array(U.COUNT * 4);
  const tmp = new Float32Array(16);
  let uidGen = 1;

  function resetAll(w, h) {
    for (const t of textures.values()) r.texDelete(t);
    textures.clear(); lists.clear(); rec = null; attribs.length = 0;
    st = defaultState(w, h);
    cur = [IDENT(), IDENT(), IDENT()]; stacks = [[], [], []]; mmode = 0; vn = 0; prim = -1;
  }
  const getTex = (id) => {
    let t = textures.get(id);
    if (!t) { t = { id, uid: uidGen++, w: 0, h: 0, min: 0x2702, mag: 0x2601, wrapS: 0x2901, wrapT: 0x2901, pv: 0, mipDirty: true, gpu: null }; textures.set(id, t); }
    return t;
  };

  function fillUniforms() {
    const m = cur, u = uni;
    u.set(m[0], U.MV * 4); u.set(m[1], U.PROJ * 4); u.set(m[2], U.TEXM * 4);
    normalMatrix(m[0], u, U.NRM * 4);
    u.set(st.lmAmbient, U.LMAMB * 4);
    const fm = !st.fog ? 0 : st.fogMode === 0x2601 ? 1 : st.fogMode === 0x800 ? 2 : 3;
    u.set([fm, st.fogDensity, st.fogStart, st.fogEnd], U.FOG * 4);
    u.set(st.fogColor, U.FOGCOL * 4);
    const cmF = st.colorMaterial && st.cmFace !== 0x405 ? CM_CODE[st.cmMode] ?? 0 : 0;
    const cmB = st.colorMaterial && st.cmFace !== 0x404 ? CM_CODE[st.cmMode] ?? 0 : 0;
    u.set([st.lighting ? 1 : 0, cmF, st.twoSide ? 1 : 0, st.localViewer ? 1 : 0], U.F1 * 4);
    const tex = st.tex2d ? textures.get(st.bound) : null;
    const texOn = tex && tex.w > 0 && tex.h > 0;
    const env = st.texEnv === 0x1e01 ? 1 : st.texEnv === 0x2101 ? 2 : st.texEnv === 0x104 ? 3 : 0;
    u.set([st.normalize ? 1 : 0, texOn ? 1 : 0, env, st.alphaTest ? (st.alphaFunc - 0x200) & 7 : 7], U.F2 * 4);
    u.set([st.alphaRef, 0, st.pointSize, cmB], U.F3 * 4);
    const putMat = (mt, base) => {
      u.set(mt.amb, base * 4); u.set(mt.dif, (base + 1) * 4); u.set(mt.spe, (base + 2) * 4); u.set(mt.emi, (base + 3) * 4);
      u.set([mt.shin, 0, 0, 0], (base + 4) * 4);
    };
    putMat(st.matF, U.MATF); putMat(st.matB, U.MATB);
    for (let i = 0; i < 8; i++) {
      const L = st.lights[i], b = (U.LIGHT + i * U.LSTRIDE) * 4;
      u.set(L.amb, b); u.set(L.dif, b + 4); u.set(L.spe, b + 8); u.set(L.pos, b + 12);
      u.set([L.dir[0], L.dir[1], L.dir[2], L.exp], b + 16); u.set([L.att[0], L.att[1], L.att[2], L.cut], b + 20);
      u.set([L.en ? 1 : 0, 0, 0, 0], b + 24);
    }
    return texOn ? tex : null;
  }

  // ---- primitive assembly -------------------------------------------------------------------------------------------
  function emit(mode, v, n) {
    if (!created || n <= 0 || mode < 0 || mode > 9) return;
    const tri = [], prov = [], lin = [];
    const add = (a, b, c, p) => { tri.push(a, b, c); prov.push(p); };
    const ln = (a, b) => { lin.push(a, b); };
    let list, topo;
    switch (mode) {
      case 0: topo = 0; list = Array.from({ length: n }, (_, i) => i); break;
      case 1: for (let i = 0; i + 1 < n; i += 2) ln(i, i + 1); topo = 1; list = lin; break;
      case 2: for (let i = 0; i + 1 < n; i++) ln(i, i + 1); if (n > 2) ln(n - 1, 0); topo = 1; list = lin; break;
      case 3: for (let i = 0; i + 1 < n; i++) ln(i, i + 1); topo = 1; list = lin; break;
      case 4: for (let i = 0; i + 2 < n; i += 3) { add(i, i + 1, i + 2, i + 2); ln(i, i + 1); ln(i + 1, i + 2); ln(i + 2, i); } break;
      case 5: for (let i = 0; i + 2 < n; i++) { if (i & 1) add(i + 1, i, i + 2, i + 2); else add(i, i + 1, i + 2, i + 2); ln(i, i + 1); ln(i, i + 2); } if (n > 1) ln(n - 2, n - 1); break;
      case 6: for (let i = 1; i + 1 < n; i++) { add(0, i, i + 1, i + 1); ln(0, i); ln(i, i + 1); } if (n > 2) ln(0, n - 1); break;
      case 7: for (let q = 0; q + 3 < n; q += 4) { add(q, q + 1, q + 2, q + 3); add(q, q + 2, q + 3, q + 3); ln(q, q + 1); ln(q + 1, q + 2); ln(q + 2, q + 3); ln(q + 3, q); } break;
      case 8: for (let k = 0; k + 3 < n; k += 2) { add(k, k + 1, k + 3, k + 3); add(k, k + 3, k + 2, k + 3); ln(k, k + 1); ln(k + 1, k + 3); ln(k + 3, k + 2); ln(k + 2, k); } break;
      case 9: for (let i = 1; i + 1 < n; i++) add(0, i, i + 1, 0); for (let i = 0; i < n; i++) ln(i, (i + 1) % n); break;
    }
    if (mode >= 4) {
      if (st.polyMode === 0x1b02) { topo = 2; list = tri; }
      else if (st.polyMode === 0x1b01) { topo = 1; list = lin; }
      else { topo = 0; list = Array.from({ length: n }, (_, i) => i); }
    }
    const k = list.length;
    if (!k) return;
    if (k * VSTRIDE > ob.length) ob = new Float32Array(Math.max(k * VSTRIDE, ob.length * 2));
    for (let j = 0; j < k; j++) ob.set(v.subarray(list[j] * VSTRIDE, list[j] * VSTRIDE + VSTRIDE), j * VSTRIDE);
    if (st.flat && topo >= 1) { // provoking-vertex colour/normal replicated (identical result on every backend)
      const per = topo === 2 ? 3 : 2;
      for (let g = 0; g * per < k; g++) {
        const pv = topo === 2 && mode >= 4 && st.polyMode === 0x1b02 ? prov[g] : list[g * per + per - 1];
        const src = v.subarray(pv * VSTRIDE + 4, pv * VSTRIDE + 11);
        for (let e = 0; e < per; e++) ob.set(src, (g * per + e) * VSTRIDE + 4);
      }
    }
    const tex = fillUniforms();
    r.draw(st, uni, tex, ob, k, topo);
  }

  function pushV(x, y, z, w) {
    if ((vn + 1) * VSTRIDE > vb.length) { const nb = new Float32Array(vb.length * 2); nb.set(vb); vb = nb; }
    const o = vn * VSTRIDE;
    vb[o] = x; vb[o + 1] = y; vb[o + 2] = z; vb[o + 3] = w;
    vb[o + 4] = st.nrm[0]; vb[o + 5] = st.nrm[1]; vb[o + 6] = st.nrm[2];
    vb.set(st.col, o + 7); vb.set(st.tc, o + 11);
    vn++;
  }

  const setCap = (cap, v) => {
    switch (cap) {
      case 0xb71: st.depthTest = v; break; case 0xb44: st.cullFace = v; break; case 0xbe2: st.blend = v; break;
      case 0xb50: st.lighting = v; break; case 0xdE1: st.tex2d = v; break; case 0xb60: st.fog = v; break;
      case 0xbc0: st.alphaTest = v; break; case 0xba1: st.normalize = v; break; case 0xb57: st.colorMaterial = v; break;
      case 0xc11: st.scissorTest = v; break; case 0x8037: st.polyOffset = v; break;
      default: if (cap >= 0x4000 && cap < 0x4008) st.lights[cap - 0x4000].en = v;
    }
  };

  function matMul(m) { mulInto(tmp, cur[mmode], m); cur[mmode].set(tmp); }
  const mvTransformPoint = (x, y, z, w) => {
    const m = cur[0];
    return [m[0] * x + m[4] * y + m[8] * z + m[12] * w, m[1] * x + m[5] * y + m[9] * z + m[13] * w,
      m[2] * x + m[6] * y + m[10] * z + m[14] * w, m[3] * x + m[7] * y + m[11] * z + m[15] * w];
  };

  // ---- the interpreter ----------------------------------------------------------------------------------------------
  function run(words) {
    const f = new Float32Array(words.buffer, words.byteOffset, words.length);
    let i = 0;
    while (i + 2 <= words.length) {
      const op = words[i], len = words[i + 1], p = i + 2, next = p + len;
      if (next > words.length) break; // truncated message
      i = next;
      if (rec && !NO_RECORD.has(op)) {
        rec.parts.push(words.slice(p - 2, next));
        if (rec.mode === 0x1300) continue; // GL_COMPILE: record only
      }
      switch (op) {
        case 1: W = Math.max(1, words[p]); H = Math.max(1, words[p + 1]); resetAll(W, H); r.create(W, H); created = true; break;
        case 2: W = Math.max(1, words[p]); H = Math.max(1, words[p + 1]); r.resize(W, H); created = true; break;
        case 3: if (created) r.clear(words[p], st); break;
        case 4: st.clearColor = [f[p], f[p + 1], f[p + 2], f[p + 3]]; break;
        case 5: st.clearDepth = f[p]; break;
        case 6: setCap(words[p], true); break;
        case 7: setCap(words[p], false); break;
        case 8: st.depthFunc = words[p]; break;
        case 9: st.depthMask = !!words[p]; break;
        case 10: st.cullMode = words[p]; break;
        case 11: st.frontFace = words[p]; break;
        case 12: st.blendSrc = words[p]; st.blendDst = words[p + 1]; break;
        case 13: st.alphaFunc = words[p]; st.alphaRef = f[p + 1]; break;
        case 14: st.flat = words[p] === 0x1d00; break;
        case 15: st.polyMode = words[p + 1]; break;
        case 16: st.lineWidth = f[p]; break;
        case 17: st.pointSize = f[p]; break;
        case 18: st.viewport = [words[p] | 0, words[p + 1] | 0, words[p + 2] | 0, words[p + 3] | 0]; break;
        case 19: st.scissor = [words[p] | 0, words[p + 1] | 0, words[p + 2] | 0, words[p + 3] | 0]; break;
        case 20: st.colorMask = [!!words[p], !!words[p + 1], !!words[p + 2], !!words[p + 3]]; break;
        case 21: { const m = words[p] - 0x1700; if (m >= 0 && m < 3) mmode = m; break; }
        case 22: cur[mmode].set(IDENT()); break;
        case 23: cur[mmode].set(f.subarray(p, p + 16)); break;
        case 24: matMul(f.subarray(p, p + 16)); break;
        case 25: if (stacks[mmode].length < 128) stacks[mmode].push(cur[mmode].slice()); break;
        case 26: { const s = stacks[mmode].pop(); if (s) cur[mmode].set(s); break; }
        case 27: prim = words[p]; vn = 0; break;
        case 28: if (prim >= 0) { emit(prim, vb, vn); } prim = -1; vn = 0; break;
        case 29: if (prim >= 0) pushV(f[p], f[p + 1], f[p + 2], f[p + 3]); break;
        case 30: st.col = [f[p], f[p + 1], f[p + 2], f[p + 3]]; break;
        case 31: st.nrm = [f[p], f[p + 1], f[p + 2]]; break;
        case 32: st.tc = [f[p], f[p + 1], f[p + 2], f[p + 3]]; break;
        case 33: {
          const mode = words[p], n = words[p + 1], flags = words[p + 2];
          let o = p + 3; const pos = o; o += n * 4;
          const nrm = flags & 1 ? o : -1; if (flags & 1) o += n * 3;
          const col = flags & 2 ? o : -1; if (flags & 2) o += n * 4;
          const tc = flags & 4 ? o : -1; if (flags & 4) o += n * 2;
          if (o > next) break;
          const v = new Float32Array(n * VSTRIDE);
          for (let k = 0; k < n; k++) {
            const b = k * VSTRIDE;
            for (let c = 0; c < 4; c++) v[b + c] = f[pos + k * 4 + c];
            for (let c = 0; c < 3; c++) v[b + 4 + c] = nrm >= 0 ? f[nrm + k * 3 + c] : st.nrm[c];
            for (let c = 0; c < 4; c++) v[b + 7 + c] = col >= 0 ? f[col + k * 4 + c] : st.col[c];
            if (tc >= 0) { v[b + 11] = f[tc + k * 2]; v[b + 12] = f[tc + k * 2 + 1]; v[b + 13] = 0; v[b + 14] = 1; }
            else for (let c = 0; c < 4; c++) v[b + 11 + c] = st.tc[c];
          }
          emit(mode, v, n);
          break;
        }
        case 34: {
          const li = words[p]; if (li > 7) break;
          const L = st.lights[li], pn = words[p + 1], a = [f[p + 2], f[p + 3], f[p + 4], f[p + 5]];
          switch (pn) {
            case 0x1200: L.amb = a; break; case 0x1201: L.dif = a; break; case 0x1202: L.spe = a; break;
            case 0x1203: L.pos = mvTransformPoint(a[0], a[1], a[2], a[3]); break;
            case 0x1204: { const n3 = new Float32Array(12); normalMatrix(cur[0], n3, 0);
              L.dir = [n3[0] * a[0] + n3[4] * a[1] + n3[8] * a[2], n3[1] * a[0] + n3[5] * a[1] + n3[9] * a[2], n3[2] * a[0] + n3[6] * a[1] + n3[10] * a[2]]; break; }
            case 0x1205: L.exp = a[0]; break; case 0x1206: L.cut = a[0]; break;
            case 0x1207: L.att[0] = a[0]; break; case 0x1208: L.att[1] = a[0]; break; case 0x1209: L.att[2] = a[0]; break;
          }
          break;
        }
        case 35: {
          const pn = words[p];
          if (pn === 0xb53) st.lmAmbient = [f[p + 1], f[p + 2], f[p + 3], f[p + 4]];
          else if (pn === 0xb52) st.twoSide = f[p + 1] !== 0; else if (pn === 0xb51) st.localViewer = f[p + 1] !== 0;
          break;
        }
        case 36: {
          const face = words[p], pn = words[p + 1], a = [f[p + 2], f[p + 3], f[p + 4], f[p + 5]];
          for (const [mt, on] of [[st.matF, face !== 0x405], [st.matB, face !== 0x404]]) {
            if (!on) continue;
            switch (pn) {
              case 0x1200: mt.amb = a; break; case 0x1201: mt.dif = a; break; case 0x1202: mt.spe = a; break;
              case 0x1600: mt.emi = a; break; case 0x1601: mt.shin = a[0]; break;
              case 0x1602: mt.amb = a; mt.dif = a; break;
            }
          }
          break;
        }
        case 37: st.cmFace = words[p]; st.cmMode = words[p + 1]; break;
        case 38: {
          const pn = words[p];
          if (pn === 0xb65) st.fogMode = Math.round(f[p + 1]); else if (pn === 0xb62) st.fogDensity = f[p + 1];
          else if (pn === 0xb63) st.fogStart = f[p + 1]; else if (pn === 0xb64) st.fogEnd = f[p + 1];
          else if (pn === 0xb66) st.fogColor = [f[p + 1], f[p + 2], f[p + 3], f[p + 4]];
          break;
        }
        case 39: {
          const id = words[p], w = words[p + 1], h = words[p + 2];
          if (w * h * 4 > (next - (p + 3)) * 4) break;
          const t = getTex(id); t.w = w; t.h = h; t.mipDirty = true;
          if (created) r.texUpload(t, 0, 0, w, h, new Uint8Array(words.buffer, words.byteOffset + (p + 3) * 4, w * h * 4), true);
          break;
        }
        case 40: {
          const t = textures.get(words[p]), x = words[p + 1], y = words[p + 2], w = words[p + 3], h = words[p + 4];
          if (!t || !t.w || x + w > t.w || y + h > t.h || w * h * 4 > (next - (p + 5)) * 4) break;
          t.mipDirty = true;
          if (created) r.texUpload(t, x, y, w, h, new Uint8Array(words.buffer, words.byteOffset + (p + 5) * 4, w * h * 4), false);
          break;
        }
        case 41: st.bound = words[p]; if (st.bound) getTex(st.bound); break;
        case 42: { const t = textures.get(words[p]); if (t) { r.texDelete(t); textures.delete(words[p]); } if (st.bound === words[p]) st.bound = 0; break; }
        case 43: {
          const t = st.bound ? textures.get(st.bound) : null; if (!t) break;
          const pn = words[p], v = Math.round(f[p + 1]);
          if (pn === 0x2801) t.min = v; else if (pn === 0x2800) t.mag = v; else if (pn === 0x2802) t.wrapS = v; else if (pn === 0x2803) t.wrapT = v; else break;
          t.pv++;
          break;
        }
        case 44: if (words[p] === 0x2200) st.texEnv = Math.round(f[p + 1]); break;
        case 45: if (!rec) rec = { id: words[p], mode: words[p + 1], parts: [] }; break;
        case 46: if (rec) {
          let n = 0; for (const q of rec.parts) n += q.length;
          const all = new Uint32Array(n); let o = 0; for (const q of rec.parts) { all.set(q, o); o += q.length; }
          lists.set(rec.id, all); rec = null;
        } break;
        case 47: {
          const l = lists.get(words[p]);
          if (l && callDepth < 64) { const saved = rec; rec = null; callDepth++; try { run(l); } finally { callDepth--; rec = saved; } }
          break;
        }
        case 48: for (let k = 0; k < words[p + 1]; k++) lists.delete(words[p] + k); break;
        case 49: if (created) r.present(W, H, presentCb); break;
        case 50: attribs.push(structuredClone(st)); break;
        case 51: { const s = attribs.pop(); if (s) st = s; break; }
        default: break; // unknown op: skipped via len
      }
    }
  }
  return { execute: (words) => run(words) };
}
