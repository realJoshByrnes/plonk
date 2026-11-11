// Renders the Plonk wine scene (SVG) in headless Chrome: settled wallpapers at several sizes + animation stills.
// Usage: node web/make_wallpaper.mjs <out_dir>   -> wall_1024x768.png, wall_1920x1080.png, wall_800x1200.png, intro_t<t>.png
import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const WEB = dirname(fileURLToPath(import.meta.url));
const outDir = process.argv[2];
const CHROME = ["C:/Program Files/Google/Chrome/Application/chrome.exe", "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"].find(existsSync);
if (!CHROME) { console.log("SKIP make_wallpaper: no Chrome/Edge found"); process.exit(0); }
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const server = createServer((req, res) => {
  const p = new URL(req.url, "http://x").pathname;
  if (p === "/") { res.writeHead(200, { "content-type": "text/html" }); res.end("<!doctype html><body style='margin:0'></body>"); return; }
  if (p.endsWith(".mjs") && /^\/[\w.]+$/.test(p) && existsSync(join(WEB, p))) { res.writeHead(200, { "content-type": "text/javascript", "cache-control": "no-store" }); res.end(readFileSync(join(WEB, p))); return; }
  res.writeHead(404); res.end();
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const PORT = server.address().port;

const PAGE = `(async () => {
  const m = await import("/intro.mjs");
  const out = { wall: {}, stills: {}, ts: [0.5, 1.6, 2.0, 2.4, 3.2, m.DURATION] };
  const toURL = (c) => c.toDataURL("image/png");
  const bmpToCanvas = (b) => { const c = document.createElement("canvas"); c.width = b.width; c.height = b.height; c.getContext("2d").drawImage(b, 0, 0); return c; };
  for (const [w, h] of [[1024, 768], [1920, 1080], [800, 1200]]) out.wall[w + "x" + h] = toURL(bmpToCanvas(await m.rasterize(w, h)));
  // stage like the page: relative container, canvas with margin 0 12px
  const stage = document.createElement("div"); stage.style.cssText = "position:relative;width:1048px;height:768px;background:#000";
  const cv = document.createElement("canvas"); cv.width = 1024; cv.height = 768; cv.style.cssText = "display:block;margin:0 12px;width:1024px;height:768px";
  stage.appendChild(cv); document.body.appendChild(stage);
  const tl = m.createTimeline(stage);
  const get = (c) => c.getContext("2d").getImageData(0, 0, 1024, 768).data;
  const datas = {};
  for (const t of out.ts) { tl.seek(t); const c = await m.snapshot(tl.svg, 1024, 768); out.stills[t] = toURL(c); datas[t] = get(c); }
  const ref = get(await (async () => bmpToCanvas(await m.rasterize(1024, 768)))());
  const d = datas[m.DURATION]; let sum = 0, mx = 0;
  for (let i = 0; i < ref.length; i += 4) for (let k = 0; k < 3; k++) { const e = Math.abs(ref[i + k] - d[i + k]); sum += e; if (e > mx) mx = e; }
  let bad = 0; for (let i = 0; i < ref.length; i += 4) if (Math.abs(ref[i] - d[i]) + Math.abs(ref[i + 1] - d[i + 1]) + Math.abs(ref[i + 2] - d[i + 2]) > 30) bad++;
  out.meanDiff = sum / (ref.length / 4 * 3); out.maxDiff = mx; out.badFrac = bad / (ref.length / 4);
  const svg = tl.svg.getBoundingClientRect(); out.svgRect = [svg.left, svg.top, svg.width, svg.height]; out.nAnim = tl.anims.length; out.DURATION = m.DURATION;
  out.scene0Len = m.scene0SVG().length;
  return out;
})()`;

async function runPage(flags) {
  const udd = mkdtempSync(join(tmpdir(), "plonk-wp-"));
  const chrome = spawn(CHROME, [...flags, "--remote-debugging-port=0", `--user-data-dir=${udd}`, "--no-first-run", "--no-default-browser-check",
    "--disable-extensions", "about:blank"], { stdio: "ignore" });
  let ws; const pending = new Map(); let nextId = 1;
  const cleanup = () => {
    try { ws?.close(); } catch {}
    try { spawnSync("taskkill", ["/pid", String(chrome.pid), "/T", "/F"], { stdio: "ignore" }); } catch {}
    try { chrome.kill(); } catch {}
    setTimeout(() => { try { rmSync(udd, { recursive: true, force: true }); } catch {} }, 1500).unref();
  };
  try {
    let port = 0;
    for (let i = 0; i < 100 && !port; i++) { await sleep(150); try { port = parseInt(readFileSync(join(udd, "DevToolsActivePort"), "utf8").split("\n")[0], 10); } catch {} }
    if (!port) return { error: "chrome did not start" };
    let page;
    for (let i = 0; i < 50 && !page; i++) { try { page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((t) => t.type === "page"); } catch {} if (!page) await sleep(150); }
    if (!page) return { error: "no page target" };
    ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error("ws error")); });
    ws.onmessage = (m) => { const d = JSON.parse(m.data); if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); } };
    const send = (method, params = {}) => new Promise((res) => { const id = nextId++; pending.set(id, res); ws.send(JSON.stringify({ id, method, params })); });
    await send("Page.enable");
    await send("Page.navigate", { url: `http://127.0.0.1:${PORT}/` });
    await sleep(800);
    const r = await send("Runtime.evaluate", { expression: PAGE, returnByValue: true, awaitPromise: true });
    if (r.result?.exceptionDetails) return { error: r.result.exceptionDetails.exception?.description ?? r.result.exceptionDetails.text };
    return r.result.result.value;
  } finally { cleanup(); }
}


const BASE = ["--headless=new", "--ignore-gpu-blocklist", "--disable-gpu-sandbox"];
const SETS = [[...BASE, "--use-angle=swiftshader", "--enable-unsafe-swiftshader"], [...BASE, "--disable-gpu"]];
let res;
for (const f of SETS) { res = await runPage(f); if (res && !res.error) break; console.log("flags failed:", res?.error?.split("\n")[0]); }
server.close();
if (!res || res.error) { console.log("FAIL make_wallpaper:", res?.error); process.exit(1); }

const png = (d) => Buffer.from(d.split(",")[1], "base64");
if (outDir) {
  mkdirSync(outDir, { recursive: true });
  for (const [k, v] of Object.entries(res.wall)) writeFileSync(join(outDir, `wall_${k}.png`), png(v));
  for (const [t, v] of Object.entries(res.stills)) writeFileSync(join(outDir, `intro_t${Number(t).toFixed(2)}.png`), png(v));
}
let fail = 0;
const chk = (ok, name, detail = "") => { if (!ok) fail++; console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? " -- " + detail : ""}`); };
chk(res.meanDiff < 0.5 && res.badFrac < 0.005, "t=DURATION animation still matches rasterize()", `mean ${res.meanDiff.toFixed(3)} max ${res.maxDiff} bad ${(res.badFrac * 100).toFixed(3)}%`);
chk(Math.abs(res.svgRect[0] - 12) < 1 && Math.abs(res.svgRect[2] - 1024) < 1, "svg overlay covers canvas area", res.svgRect.join(","));
console.log(`animations: ${res.nAnim}, scene0 svg: ${res.scene0Len} bytes, DURATION ${res.DURATION}s`);
process.exit(fail ? 1 : 0);
