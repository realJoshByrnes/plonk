// Boot intro: a cherry drops into a glass of red wine ("plonk"). The settled end state is the desktop wallpaper.
// Animation = WAAPI keyframes (transform/opacity only, precomputed from the physics) so it keeps running on the
// compositor while the main thread is busy. Everything is created and started within one task.
import { DURATION, TI, scene0SVG, sceneSVG, startAnimations, animatedIds } from "./wine_scene.svg.mjs";
export { DURATION, scene0SVG };

function svgToCanvas(svgText, w, h) {
  return new Promise((res, rej) => {
    const url = URL.createObjectURL(new Blob([svgText], { type: "image/svg+xml" }));
    const img = new Image();
    img.onload = () => {
      const c = document.createElement("canvas"); c.width = w; c.height = h;
      c.getContext("2d").drawImage(img, 0, 0, w, h);
      URL.revokeObjectURL(url); res(c);
    };
    img.onerror = () => { URL.revokeObjectURL(url); rej(new Error("svg raster failed")); };
    img.src = url;
  });
}

// Settled scene at w x h (2D canvas only, no WebGL).
export async function rasterize(w, h) {
  const c = await svgToCanvas(sceneSVG({ w, h }), w, h);
  return typeof createImageBitmap === "function" ? createImageBitmap(c) : c;
}

// Rasterize a live (possibly paused/seeked) mounted svg: bakes the animated transform/opacity into a clone first.
export async function snapshot(svg, w, h) {
  const clone = svg.cloneNode(true);
  clone.setAttribute("width", w); clone.setAttribute("height", h);
  clone.removeAttribute("style");
  for (const id of animatedIds()) {
    const o = svg.querySelector("#" + id), c = clone.querySelector("#" + id);
    if (!o || !c) continue;
    const cs = getComputedStyle(o);
    c.style.transformOrigin = "0 0";
    c.style.transform = cs.transform === "none" ? "" : cs.transform;
    c.style.opacity = cs.opacity;
  }
  return svgToCanvas(new XMLSerializer().serializeToString(clone), w, h);
}

// Mount the scene into stage (reusing an inlined scene-0 svg#plonkScene if the page already has one).
export function mount(stage) {
  let svg = stage.querySelector("svg#plonkScene");
  let created = false;
  if (!svg) {
    const cv = stage.querySelector("canvas");
    const left = cv ? cv.offsetLeft : 0, top = cv ? cv.offsetTop : 0, w = cv ? cv.clientWidth : stage.clientWidth, h = cv ? cv.clientHeight : stage.clientHeight;
    const holder = document.createElement("div");
    holder.innerHTML = scene0SVG({ id: "plonkScene", style: `position:absolute;left:${left}px;top:${top}px;width:${w}px;height:${h}px;display:block;pointer-events:none` });
    svg = holder.firstChild; stage.appendChild(svg); created = true;
  }
  return { svg, created };
}

// Seekable timeline for tests/stills: animations paused, seek(t seconds).
export function createTimeline(stage) {
  const { svg, created } = mount(stage);
  const anims = startAnimations(svg, { paused: true });
  const seek = (t) => { for (const a of anims) { a.pause(); a.currentTime = t * 1000; } };
  return { svg, created, anims, seek };
}

// ---- sound: scheduled entirely on the AudioContext clock ----
function plonk(ctx, at) {
  const out = ctx.createGain(); out.gain.value = 0.9; out.connect(ctx.destination);
  const o = ctx.createOscillator(), g = ctx.createGain();
  o.type = "sine"; o.frequency.setValueAtTime(420, at); o.frequency.exponentialRampToValueAtTime(110, at + 0.14);
  g.gain.setValueAtTime(0.0001, at); g.gain.exponentialRampToValueAtTime(0.8, at + 0.008); g.gain.exponentialRampToValueAtTime(0.0005, at + 0.28);
  o.connect(g).connect(out); o.start(at); o.stop(at + 0.32);
  const len = Math.floor(ctx.sampleRate * 0.18), buf = ctx.createBuffer(1, len, ctx.sampleRate), d = buf.getChannelData(0);
  for (let i = 0; i < len; i++) d[i] = (Math.random() * 2 - 1) * Math.pow(1 - i / len, 2);
  const n = ctx.createBufferSource(); n.buffer = buf;
  const bp = ctx.createBiquadFilter(); bp.type = "bandpass"; bp.frequency.value = 2400; bp.Q.value = 0.9;
  const ng = ctx.createGain(); ng.gain.setValueAtTime(0.0001, at); ng.gain.exponentialRampToValueAtTime(0.5, at + 0.004); ng.gain.exponentialRampToValueAtTime(0.0005, at + 0.16);
  n.connect(bp).connect(ng).connect(out); n.start(at);
  const pings = 3 + ((Math.random() * 4) | 0);
  for (let i = 0; i < pings; i++) {
    const t = at + 0.1 + Math.random() * 0.7, p = ctx.createOscillator(), pg = ctx.createGain();
    p.type = "sine"; const fq = 1800 + Math.random() * 2600;
    p.frequency.setValueAtTime(fq, t); p.frequency.exponentialRampToValueAtTime(fq * 1.5, t + 0.05);
    pg.gain.setValueAtTime(0.0001, t); pg.gain.exponentialRampToValueAtTime(0.07 + Math.random() * 0.05, t + 0.004); pg.gain.exponentialRampToValueAtTime(0.0005, t + 0.07);
    p.connect(pg).connect(out); p.start(t); p.stop(t + 0.09);
  }
}

export async function playIntro({ stage, w, h, mute } = {}) {
  let ctx = null;
  try {
    if (!mute) {
      const AC = globalThis.AudioContext || globalThis.webkitAudioContext;
      if (AC) { ctx = new AC(); ctx.resume?.().catch(() => {}); plonk(ctx, ctx.currentTime + TI + 0.02); }
    }
  } catch { ctx = null; }
  const { svg, created, anims } = (() => { const m = mount(stage); return { ...m, anims: startAnimations(m.svg) }; })();
  try { await Promise.all(anims.map((a) => a.finished.catch(() => {}))); } catch {}
  if (created) svg.remove(); else svg.remove();
  if (ctx) setTimeout(() => { try { ctx.close(); } catch {} }, 1500);
}
