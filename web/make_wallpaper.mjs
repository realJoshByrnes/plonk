// Renders the Plonk 3D scene (web/scene3d.mjs) in headless Chrome (SwiftShader WebGL2).
//   node web/make_wallpaper.mjs <out_dir> [--size=960x600] [--t=-1,1.5,2] [--tile=48]   iteration stills (PNG)
//   node web/make_wallpaper.mjs <out_dir> --final   also writes web/wallpaper.jpg (2560x1600 settled) and web/poster.jpg (640x400 pre-splash)
import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const WEB = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const flag = (n) => args.find((a) => a.startsWith("--" + n + "="))?.split("=")[1];
const outDir = args.find((a) => !a.startsWith("--"));
const FINAL = args.includes("--final");
const [SW, SH] = (flag("size") ?? "960x600").split("x").map(Number);
const TILE = Number(flag("tile") ?? 48);
const TS = flag("t") ? flag("t").split(",").map(Number) : [-1, 1.5, 1.7, 2.0, 2.6, 4.5];
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

const job = (o) => `(async () => {
  const o = ${JSON.stringify(o)};
  const m = await import("/scene3d.mjs");
  const out = { stills: [], log: [] };
  const sync = (gl) => { const b = new Uint8Array(4); gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, b); };
  for (const j of o.jobs) {
    const sc = m.createScene({ width: j.w, height: j.h, tileRows: j.tile, grain: j.grain ?? 1 });
    if (!sc) return { error: "no webgl2" };
    const t0 = performance.now();
    const ok = await sc.ready;
    if (!ok) return { error: "shader link failed" };
    out.log.push("compile+link " + Math.round(performance.now() - t0) + " ms");
    const gl = sc.canvas.getContext("webgl2");
    for (const t of j.ts) {
      const a = performance.now(); sc.render(t); sync(gl); const ms = Math.round(performance.now() - a);
      out.stills.push({ name: j.names[j.ts.indexOf(t)], ms, data: j.jpeg ? sc.canvas.toDataURL("image/jpeg", j.jpeg) : sc.canvas.toDataURL("image/png"), w: j.w, h: j.h });
    }
    sc.dispose();
  }
  return out;
})()`;

async function runPage(flags, expr) {
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
    const r = await send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true });
    if (r.result?.exceptionDetails) return { error: r.result.exceptionDetails.exception?.description ?? r.result.exceptionDetails.text };
    return r.result.result.value;
  } finally { cleanup(); }
}

const BASE = ["--headless=new", "--ignore-gpu-blocklist", "--disable-gpu-sandbox", "--disable-gpu-watchdog", "--no-sandbox"];
const SETS = [[...BASE, "--use-angle=swiftshader", "--enable-unsafe-swiftshader"], [...BASE, "--use-gl=angle", "--use-angle=swiftshader-webgl"]];
const jobs = [];
const fmt = (t) => (t < 0 ? "pre" : "t" + t.toFixed(2));
if (FINAL) {
  jobs.push({ w: 2560, h: 1600, tile: 32, ts: [4.5], jpeg: 0.88, names: ["wallpaper.jpg"] });
  jobs.push({ w: 640, h: 400, tile: 0, ts: [-1], jpeg: 0.55, grain: 0, names: ["poster.jpg"] });
} else {
  jobs.push({ w: SW, h: SH, tile: TILE, ts: TS, names: TS.map((t) => `${fmt(t)}_${SW}x${SH}.png`) });
}
let res;
for (const f of SETS) { res = await runPage(f, job({ jobs })); if (res && !res.error) break; console.log("flags failed:", res?.error?.split("\n")[0]); }
server.close();
if (!res || res.error) { console.log("FAIL make_wallpaper:", res?.error); process.exit(1); }
for (const st of res.stills) {
  const dir = FINAL ? WEB : outDir; if (!dir) continue; mkdirSync(dir, { recursive: true });
  const buf = Buffer.from(st.data.split(",")[1], "base64"); writeFileSync(join(dir, st.name), buf);
  console.log(st.name, st.w + "x" + st.h, buf.length + " bytes", st.ms + " ms/frame");
}
console.log(res.log.join("; "));
