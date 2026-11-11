// Renders the boot-intro final frame to web/wallpaper.png (1024x768) and checks the intro in headless Chrome (SwiftShader).
// Usage: node web/make_wallpaper.mjs [stills_dir]   (stills_dir gets intro_t0.00.png, t1.50, t3.00, t4.00)
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
const W = 1024, H = 768;

const server = createServer((req, res) => {
  const p = new URL(req.url, "http://x").pathname;
  if (p === "/") { res.writeHead(200, { "content-type": "text/html" }); res.end("<!doctype html><body></body>"); return; }
  if (p === "/intro.mjs") { res.writeHead(200, { "content-type": "text/javascript", "cache-control": "no-store" }); res.end(readFileSync(join(WEB, "intro.mjs"))); return; }
  res.writeHead(404); res.end();
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const PORT = server.address().port;

const PAGE = `(async () => {
  const m = await import("/intro.mjs");
  const it = m.createIntro(${W}, ${H});
  if (!it) return { error: "no webgl2" };
  const cx = document.createElement("canvas"); cx.width = ${W}; cx.height = ${H};
  const x2 = cx.getContext("2d", { willReadFrequently: true });
  const gl = it.canvas.getContext("webgl2");
  const grab = (t) => { it.draw(t); x2.drawImage(it.canvas, 0, 0); return x2.getImageData(0, 0, ${W}, ${H}).data; };
  const out = { png: {}, px: {}, ms: [] };
  const spots = {}; const b = m.ballState(${W}, ${H}, m.DURATION);
  const ts = [0, 1.5, 3, m.DURATION];
  const frames = {};
  for (const t of ts) { frames[t] = grab(t); out.png[t] = it.canvas.toDataURL("image/png"); }
  const at = (t, x, y) => { const o = (Math.round(y) * ${W} + Math.round(x)) * 4; return Array.from(frames[t].slice(o, o + 3)); };
  const b15 = m.ballState(${W}, ${H}, 1.5);
  out.px = { rest: { final: at(m.DURATION, b.x, b.y), empty: at(0, b.x, b.y), at3: at(3, b.x, b.y) },
             mid: { mid: at(1.5, b15.x, b15.y), empty: at(0, b15.x, b15.y) }, ball0: m.ballState(${W}, ${H}, 0), b, b15 };
  // fraction of pixels that differ from the t=0 frame
  const diff = (t) => { let c = 0; for (let i = 0; i < frames[0].length; i += 4) if (Math.abs(frames[t][i] - frames[0][i]) + Math.abs(frames[t][i+1] - frames[0][i+1]) + Math.abs(frames[t][i+2] - frames[0][i+2]) > 24) c++; return c / (${W} * ${H}); };
  out.px.diff = { 1.5: diff(1.5), 3: diff(3), 4: diff(m.DURATION) };
  // timing (draw + sync readback of 1 pixel)
  const px1 = new Uint8Array(4);
  for (let i = 0; i < 6; i++) { const t0 = performance.now(); it.draw(0.5 + i * 0.6); gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px1); out.ms.push(performance.now() - t0); }
  // determinism: final frame twice
  const a = grab(m.DURATION); const a2 = grab(m.DURATION + 10); let same = true; for (let i = 0; i < a.length; i++) if (a[i] !== a2[i]) { same = false; break; }
  out.deterministic = same;
  out.png.final = it.canvas.toDataURL("image/png");
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
const SETS = [[...BASE, "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--enable-webgl"],
  [...BASE, "--use-gl=angle", "--use-angle=swiftshader-webgl", "--enable-unsafe-swiftshader"], [...BASE, "--enable-unsafe-swiftshader"]];
let res;
for (const f of SETS) { res = await runPage(f); if (res && !res.error) break; console.log("flags failed:", res?.error?.split("\n")[0]); }
server.close();
if (!res || res.error) { console.log("FAIL make_wallpaper:", res?.error); process.exit(1); }

const png = (d) => Buffer.from(d.split(",")[1], "base64");
writeFileSync(join(WEB, "wallpaper.png"), png(res.png.final));
if (outDir) {
  mkdirSync(outDir, { recursive: true });
  for (const t of ["0", "1.5", "3", "4"]) writeFileSync(join(outDir, `intro_t${Number(t).toFixed(2)}.png`), png(res.png[t]));
}
let fail = 0;
const chk = (ok, name, detail = "") => { if (!ok) fail++; console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? " -- " + detail : ""}`); };
const dist = (a, b) => Math.abs(a[0] - b[0]) + Math.abs(a[1] - b[1]) + Math.abs(a[2] - b[2]);
const p = res.px;
chk(p.ball0.x + p.ball0.r < 0, "t=0: ball centre/edge off-screen left", `x=${p.ball0.x.toFixed(0)} r=${p.ball0.r.toFixed(0)}`);
chk(dist(p.rest.final, p.rest.empty) > 30, "final: ball differs from empty backdrop at rest centre", `${p.rest.final} vs ${p.rest.empty}`);
chk(Math.abs(p.b.x - 0.75 * W) < 1 && Math.abs(p.b.y - 0.71 * H) < 1, "final: ball centre near (0.75w, floor)", `(${p.b.x},${p.b.y})`);
chk(p.mid.mid && dist(p.mid.mid, p.mid.empty) > 30 && p.b15.x > 0 && p.b15.x < 0.75 * W, "t=1.5: ball partway", `x=${p.b15.x.toFixed(0)}`);
chk(dist(p.rest.at3, p.rest.final) < 400 && p.diff[3] > 0.02, "t=3: ball nearly settled", `diff vs empty ${(p.diff[3] * 100).toFixed(1)}%`);
chk(p.diff[4] > 0.05 && p.diff[4] < 0.25, "final frame ball coverage plausible", `${(p.diff[4] * 100).toFixed(1)}% pixels differ from empty`);
chk(res.deterministic, "draw(DURATION) deterministic / identical for t > DURATION");
console.log(`frame time on SwiftShader (draw+sync, ${W}x${H}): ${res.ms.map((m) => m.toFixed(0)).join(", ")} ms`);
console.log(`wrote ${join(WEB, "wallpaper.png")}`);
process.exit(fail ? 1 : 0);
