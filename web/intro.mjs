// Boot intro driver: plays the GPU scene (scene3d.mjs) from requestAnimationFrame using performance.now (frame-rate independent)
// plus a WebAudio "plonk" scheduled on the AudioContext clock at T_IMPACT.
//   const c = playIntro({ canvas, scene, mute }); c.skip(); await c.done;
// `canvas` is the overlay canvas the caller positioned over the desktop; the scene's canvas is drawImage'd into it each frame
// (if it is the scene's own canvas nothing is copied). `scene` must be ready (await scene.ready).
import { DURATION, T_IMPACT } from "./scene3d.mjs";
export { DURATION, T_IMPACT };

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

export function playIntro({ canvas, scene, mute } = {}) {
  let ctx = null, raf = 0, finished = false, resolve;
  const done = new Promise((r) => { resolve = r; });
  const copy = () => {
    if (canvas && scene && canvas !== scene.canvas) {
      const c2 = canvas.getContext("2d");
      if (c2) c2.drawImage(scene.canvas, 0, 0, canvas.width, canvas.height);
    }
  };
  const frame = (t) => { try { scene.render(t); copy(); } catch (e) { console.info("intro frame failed", e); } };
  const finish = () => {
    if (finished) return; finished = true;
    cancelAnimationFrame(raf);
    frame(DURATION);
    if (ctx) { const c = ctx; setTimeout(() => { try { c.close(); } catch {} }, 1500); }
    resolve();
  };
  try {
    if (!mute) {
      const AC = globalThis.AudioContext || globalThis.webkitAudioContext;
      if (AC) { ctx = new AC(); ctx.resume?.().catch(() => {}); plonk(ctx, ctx.currentTime + T_IMPACT + 0.02); }
    }
  } catch { ctx = null; }
  const t0 = performance.now();
  const tick = () => {
    if (finished) return;
    const t = (performance.now() - t0) / 1000;
    if (t >= DURATION) return finish();
    frame(t);
    raf = requestAnimationFrame(tick);
  };
  tick();
  const skip = () => { if (ctx && !finished) { try { ctx.close(); } catch {} ctx = null; } finish(); };
  return { done, skip, then: (a, b) => done.then(a, b) };
}
