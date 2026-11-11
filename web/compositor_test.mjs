// Headless-Chrome test of web/compositor_gpu.mjs. Usage: node web/compositor_test.mjs
import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const WEB = dirname(fileURLToPath(import.meta.url));
const CHROME = ["C:/Program Files/Google/Chrome/Application/chrome.exe", "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"].find(existsSync);
if (!CHROME) { console.log("SKIP compositor_test: no Chrome/Edge found"); process.exit(0); }
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const PAGE = `<!doctype html><meta charset=utf-8><body><script type=module>
import { createGpuCompositor } from "./compositor_gpu.mjs";
window.T = { done: false };
try {
const W = 1024, H = 768;
const c = createGpuCompositor(W, H);
if (!c) throw new Error("createGpuCompositor returned null");
const mk = (w, h, f) => { const a = new Uint32Array(w * h); for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) a[y * w + x] = f(x, y) >>> 0; return a; };
const A = mk(400, 300, (x, y) => y < 30 ? 0x80FFFFFF : 0xFF336699);
const B = mk(300, 250, () => 0xFFCC4444);
const layers = [
  { id: 1, x: 100, y: 100, w: 400, h: 300, radius: 12, blur: 24, shadow: 30, ver: 1, pix: A },
  { id: 2, x: 350, y: 250, w: 300, h: 250, radius: 12, blur: 0, shadow: 30, ver: 1, pix: B } ];
c.render(layers);
const cv = document.createElement("canvas"); cv.width = W; cv.height = H;
const ctx = cv.getContext("2d", { willReadFrequently: true });
ctx.drawImage(c.canvas, 0, 0);
const img = ctx.getImageData(0, 0, W, H).data;
const px = (x, y) => [img[(y * W + x) * 4], img[(y * W + x) * 4 + 1], img[(y * W + x) * 4 + 2], img[(y * W + x) * 4 + 3]];
function wall(x, y) {
  const w = W, h = H, u = x / w, v = y / h;
  const stops = [[0, [96, 128, 220]], [0.55, [168, 128, 226]], [1, [240, 156, 190]]];
  const t = Math.min(1, Math.max(0, u * 0.55 + v * 0.45));
  const [a, b] = t < stops[1][0] ? [stops[0], stops[1]] : [stops[1], stops[2]];
  const k = Math.min(1, Math.max(0, (t - a[0]) / (b[0] - a[0])));
  const c = [0, 1, 2].map((i) => a[1][i] * (1 - k) + b[1][i] * k);
  for (const [cx, cy, rad, col] of [[0.22, 0.78, 0.35, [60, 210, 230]], [0.82, 0.22, 0.30, [255, 200, 120]]]) {
    const d = Math.hypot((u - cx) * w, (v - cy) * h) / (rad * w);
    const g = Math.pow(Math.min(1, Math.max(0, 1 - d)), 2) * 0.55;
    for (let i = 0; i < 3; i++) c[i] = c[i] * (1 - g) + col[i] * g;
  }
  return c.map(Math.floor);
}
const res = [];
const chk = (name, ok, detail) => res.push({ name, ok: !!ok, detail });
const near = (a, e, tol) => e.every((v, i) => Math.abs(a[i] - v) <= tol);
chk("canvas size", c.canvas.width === W && c.canvas.height === H);
for (const [x, y] of [[2, 2], [1020, 760], [900, 100]]) { const g = px(x, y), e = wall(x, y); chk("wallpaper (" + x + "," + y + ") ~ CPU formula", near(g, e, 3) && g[3] === 255, "got " + g + " want " + e); }
{ const g = px(98, 250), e = wall(98, 250); chk("shadow darkens just outside window edge", g[0] < e[0] - 4 && g[1] < e[1] - 4, "got " + g + " wallpaper " + e); }
{ const g = px(150, 200); chk("opaque interior == source", near(g, [0x33, 0x66, 0x99, 255], 1), "got " + g); }
{ const g = px(300, 125), raw = [255, 255, 255];
  chk("translucent strip differs from raw source", Math.abs(g[0] - 255) > 10 || Math.abs(g[1] - 255) > 10, "got " + g);
  // expected: mix(glass(blur(wall)), white, 0.5); blurred wallpaper ~ wallpaper there (smooth), so compare loosely
  const w = wall(300, 125), lum = 0.299 * w[0] + 0.587 * w[1] + 0.114 * w[2];
  const back = w.map((v) => Math.min(255, lum + (v - lum) * 1.25 + 6));
  const e = back.map((v) => (v + 255) / 2);
  chk("strip ~ 0.5*glass(blurred backdrop) + 0.5*white", near(g, e, 8), "got " + g + " want ~" + e.map(Math.round)); }
{ const g = px(100, 100), e = wall(100, 100);
  // pixel (100,100) is layer-local (0,0): coverage 0 -> only the shadow (a ~ 0.11) over the wallpaper
  const r = 12, off = 10, pxx = 0.5, pyy = -off + 0.5, cx = r, cy = r;
  const d = Math.hypot(pxx - cx, pyy - cy) - r, a = 0.32 * Math.pow(Math.max(0, 1 - d / 30), 2);
  const exp = e.map((v) => v * (1 - a));
  chk("rounded corner shows wallpaper (+shadow)", near(g, exp, 3), "got " + g + " want ~" + exp.map(Math.round)); }
{ const g = px(500, 300); chk("second window opaque interior", near(g, [0xCC, 0x44, 0x44, 255], 1), "got " + g); }
// texture caching: unchanged ver must not re-upload (functional check: mutate pix without bumping ver -> no change)
A[40 * 400 + 50] = 0xFFFF0000; c.render(layers); ctx.drawImage(c.canvas, 0, 0);
{ const g = ctx.getImageData(150, 140, 1, 1).data; chk("no re-upload when ver unchanged", near([g[0], g[1], g[2]], [0x33, 0x66, 0x99], 1), "got " + [g[0], g[1], g[2]]); }
layers[0].ver = 2; c.render(layers); ctx.drawImage(c.canvas, 0, 0);
{ const g = ctx.getImageData(150, 140, 1, 1).data; chk("re-upload when ver bumped", near([g[0], g[1], g[2]], [255, 0, 0], 1), "got " + [g[0], g[1], g[2]]); }
// timing
for (let i = 0; i < 3; i++) { layers[0].ver++; c.render(layers); }
const N = 30; ctx.drawImage(c.canvas, 0, 0); ctx.getImageData(0, 0, 1, 1);
const t0 = performance.now();
for (let i = 0; i < N; i++) { layers[0].ver++; c.render(layers); ctx.drawImage(c.canvas, 0, 0); ctx.getImageData(0, 0, 1, 1); }
const ms = (performance.now() - t0) / N;
window.T = { done: true, res, ms };
} catch (e) { window.T = { done: true, error: String(e && e.stack || e) }; }
</script>`;

const server = createServer((req, res) => {
  const p = new URL(req.url, "http://x").pathname;
  if (p === "/t.html") { res.writeHead(200, { "content-type": "text/html" }); res.end(PAGE); return; }
  if (p === "/compositor_gpu.mjs") { res.writeHead(200, { "content-type": "text/javascript", "cache-control": "no-store" }); res.end(readFileSync(join(WEB, "compositor_gpu.mjs"))); return; }
  res.writeHead(404); res.end();
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const PORT = server.address().port;

async function runPage(flags) {
  const udd = mkdtempSync(join(tmpdir(), "plonk-comp-"));
  const chrome = spawn(CHROME, [...flags, "--remote-debugging-port=0", `--user-data-dir=${udd}`, "--no-first-run", "--no-default-browser-check", "--disable-extensions", "about:blank"], { stdio: "ignore" });
  let ws; const logs = []; const pending = new Map(); let nid = 1;
  try {
    let port = 0;
    for (let i = 0; i < 100 && !port; i++) { await sleep(150); try { port = parseInt(readFileSync(join(udd, "DevToolsActivePort"), "utf8").split("\n")[0], 10); } catch {} }
    if (!port) return { error: "chrome did not start" };
    let page;
    for (let i = 0; i < 50 && !page; i++) { try { page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((t) => t.type === "page"); } catch {} if (!page) await sleep(150); }
    if (!page) return { error: "no page target" };
    ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error("ws error")); });
    ws.onmessage = (m) => { const d = JSON.parse(m.data); if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); }
      if (d.method === "Runtime.consoleAPICalled") logs.push(d.params.args.map((a) => a.value ?? a.description).join(" ")); };
    const send = (method, params = {}) => new Promise((res) => { const id = nid++; pending.set(id, res); ws.send(JSON.stringify({ id, method, params })); });
    const ev = async (expr) => (await send("Runtime.evaluate", { expression: expr, returnByValue: true })).result?.result?.value;
    await send("Runtime.enable"); await send("Page.enable");
    await send("Page.navigate", { url: `http://127.0.0.1:${PORT}/t.html` });
    let done = false;
    for (let i = 0; i < 200 && !done; i++) { await sleep(200); done = await ev("!!(window.T && window.T.done)"); }
    if (!done) return { error: "page timeout", logs };
    return { T: JSON.parse(await ev("JSON.stringify(window.T)")), logs };
  } finally {
    try { ws?.close(); } catch {}
    try { spawnSync("taskkill", ["/pid", String(chrome.pid), "/T", "/F"], { stdio: "ignore" }); } catch {}
    setTimeout(() => { try { rmSync(udd, { recursive: true, force: true }); } catch {} }, 1500).unref();
  }
}

const BASE = ["--headless=new", "--ignore-gpu-blocklist", "--disable-gpu-sandbox"];
const SETS = [[...BASE, "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--enable-webgl"], [...BASE, "--use-gl=angle", "--use-angle=swiftshader-webgl", "--enable-unsafe-swiftshader"], [...BASE, "--enable-unsafe-swiftshader"], ["--headless=new"]];
let out, lastErr = "";
for (const flags of SETS) { const r = await runPage(flags); if (r.T && !r.T.error) { out = r; console.log("info flags: " + flags.join(" ")); break; } lastErr = r.error ?? r.T?.error ?? "?"; }
server.close();
if (!out) { console.log("FAIL could not run: " + lastErr); process.exit(1); }
let fails = 0;
for (const r of out.T.res) { if (!r.ok) fails++; console.log(`${r.ok ? "PASS" : "FAIL"} ${r.name}${r.ok ? "" : " -- " + r.detail}`); }
console.log(`timing: ${out.T.ms.toFixed(1)} ms/frame (1024x768, 2 layers, one layer re-uploaded per frame, incl. 2D drawImage + readback) [SwiftShader software GL]`);
console.log(fails === 0 ? "ALL PASS" : `FAILED (${fails})`);
process.exit(fails === 0 ? 0 : 1);
