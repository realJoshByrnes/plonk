// Real-browser test of the Plonk GL backend (web/gl_backend.mjs) via Chrome DevTools Protocol.
// Usage: node web/gl_test.mjs [screenshot.png]
// Starts a tiny static server for web/, launches headless Chrome (trying several GPU flag sets until WebGL2 works), runs
// web/gl_test.html, asserts concrete pixels, prints one PASS/FAIL line per check and a final ALL PASS (exit code 0/1).
// Runs the suite on WebGL2 (required) and, if headless Chrome exposes WebGPU, on WebGPU as well (otherwise reports SKIP).
import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const WEB = dirname(fileURLToPath(import.meta.url));
const shotPath = process.argv[2];
const CHROME = ["C:/Program Files/Google/Chrome/Application/chrome.exe", "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"].find(existsSync);
if (!CHROME) { console.log("SKIP gl_test: no Chrome/Edge found"); process.exit(0); }
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ------------------------------------------------------------------ static server
const MIME = { ".html": "text/html", ".mjs": "text/javascript", ".js": "text/javascript", ".css": "text/css" };
const server = createServer((req, res) => {
  const p = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname)).replace(/^[\\/]+/, "");
  const f = join(WEB, p);
  if (p.includes("..") || !existsSync(f)) { res.writeHead(404); res.end("not found"); return; }
  res.writeHead(200, { "content-type": MIME[extname(f)] ?? "application/octet-stream", "cache-control": "no-store" });
  res.end(readFileSync(f));
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const PORT = server.address().port;

// ------------------------------------------------------------------ chrome driver
async function runPage(flags, prefer, shot) {
  const udd = mkdtempSync(join(tmpdir(), "plonk-gl-"));
  const chrome = spawn(CHROME, [...flags, "--remote-debugging-port=0", `--user-data-dir=${udd}`, "--no-first-run", "--no-default-browser-check",
    "--disable-extensions", "--window-size=1000,1100", "--hide-scrollbars", "about:blank"], { stdio: "ignore" });
  let ws; const pending = new Map(); let nextId = 1; const logs = [];
  const cleanup = () => {
    try { ws?.close(); } catch {}
    try { spawnSync("taskkill", ["/pid", String(chrome.pid), "/T", "/F"], { stdio: "ignore" }); } catch {}
    try { chrome.kill(); } catch {}
    setTimeout(() => { try { rmSync(udd, { recursive: true, force: true }); } catch {} }, 1500).unref();
  };
  try {
    let port = 0;
    for (let i = 0; i < 100 && !port; i++) {
      await sleep(150);
      try { port = parseInt(readFileSync(join(udd, "DevToolsActivePort"), "utf8").split("\n")[0], 10); } catch {}
    }
    if (!port) return { error: "chrome did not start" };
    let page;
    for (let i = 0; i < 50 && !page; i++) {
      try { page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((t) => t.type === "page"); } catch {}
      if (!page) await sleep(150);
    }
    if (!page) return { error: "no page target" };
    ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error("ws error")); });
    ws.onmessage = (m) => {
      const d = JSON.parse(m.data);
      if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); }
      if (d.method === "Runtime.exceptionThrown") logs.push(d.params.exceptionDetails.exception?.description ?? d.params.exceptionDetails.text);
      if (d.method === "Runtime.consoleAPICalled") logs.push(d.params.args.map((a) => a.value ?? a.description).join(" "));
    };
    const send = (method, params = {}) => new Promise((res) => { const id = nextId++; pending.set(id, res); ws.send(JSON.stringify({ id, method, params })); });
    const ev = async (expr) => (await send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true })).result?.result?.value;
    await send("Runtime.enable"); await send("Page.enable");
    await send("Page.navigate", { url: `http://127.0.0.1:${PORT}/gl_test.html?prefer=${prefer}` });
    let done = false;
    for (let i = 0; i < 400 && !done; i++) { await sleep(150); done = await ev("!!(window.T && window.T.done)"); }
    if (!done) return { error: "page timeout", logs };
    const error = await ev("T.error"), kind = await ev("T.kind");
    if (error) return { error, kind, logs };
    const frames = JSON.parse(await ev("JSON.stringify(T.dump())"));
    for (const f of Object.values(frames)) f.data = Buffer.from(f.b64, "base64");
    const meta = await ev("T.meta");
    if (shot) { const r = await send("Page.captureScreenshot", { format: "png" }); writeFileSync(shot, Buffer.from(r.result.data, "base64")); }
    return { kind, frames, meta, logs, direct: await ev("T.direct") };
  } finally { cleanup(); }
}

// ------------------------------------------------------------------ checks
let failures = 0;
function suite(label, { frames, meta, direct }) {
  const out = (ok, name, detail = "") => { if (!ok) failures++; console.log(`${ok ? "PASS" : "FAIL"} [${label}] ${name}${!ok && detail ? " -- " + detail : ""}`); };
  const px = (n, x, y) => { const f = frames[n]; const o = (y * f.w + x) * 4; return [f.data[o], f.data[o + 1], f.data[o + 2], f.data[o + 3]]; };
  const near = (a, e, tol = 3) => e.every((v, i) => Math.abs(a[i] - v) <= tol);
  const fmt = (a) => `(${a.join(",")})`;
  const at = (name, desc, n, x, y, exp, tol = 3) => { const a = px(n, x, y); out(near(a, exp, tol), name, `${desc} at (${x},${y}) got ${fmt(a)} want ${fmt(exp)}`); };
  const diff = (a, b) => { let c = 0; const A = frames[a].data, B = frames[b].data; for (let i = 0; i < A.length; i++) if (A[i] !== B[i]) c++; return c; };
  const nonBg = (n, bg) => { let c = 0; const D = frames[n].data; for (let i = 0; i < D.length; i += 4) if (Math.abs(D[i] - bg[0]) + Math.abs(D[i + 1] - bg[1]) + Math.abs(D[i + 2] - bg[2]) > 12) c++; return c; };

  out(frames.clear.w === 160 && frames.clear.h === 120 && frames.clear.data.length === 160 * 120 * 4, "frame size 160x120 RGBA");
  at("clear color", "centre", "clear", 80, 60, [51, 102, 153, 255], 2);
  at("clear color corners", "bottom-right", "clear", 159, 119, [51, 102, 153, 255], 2);

  at("solid red quad", "", "solid", 30, 90, [255, 0, 0, 255], 1);
  at("depth test: near blue wins", "overlap", "solid", 140, 90, [0, 0, 255, 255], 1);
  at("depth test: far yellow visible elsewhere", "", "solid", 155, 90, [255, 255, 0, 255], 1);
  at("cull face: CW quad culled", "", "solid", 30, 40, [0, 0, 0, 255], 1);
  at("cull face: CCW quad drawn", "", "solid", 80, 40, [255, 0, 255, 255], 1);

  const texel = { tl: [20, 15], tr: [60, 15], bl: [20, 45], br: [60, 45] };
  const texCol = { tl: [0, 0, 255], tr: [255, 255, 255], bl: [255, 0, 0], br: [255, 255, 0] }; // after TEX_SUBIMAGE
  const cells = { replace: [0, 0, (c) => c], modulate: [80, 0, (c) => c.map((v) => Math.round(v * 0.5))],
    add: [0, 60, (c) => [c[0], c[1], Math.min(255, c[2] + 128)]], decal: [80, 60, (c) => c] };
  for (const [env, [ox, oy, fn]] of Object.entries(cells)) {
    for (const k of Object.keys(texel)) at(`texture ${env} ${k} texel`, "", "tex", ox + texel[k][0], oy + texel[k][1], [...fn(texCol[k]), 255], 3);
  }

  const lit = meta.lit;
  out(lit.length >= 2, "lit cube: >=2 faces visible", `${lit.length}`);
  for (const f of lit) at(`lit cube face n=(${f.n}) = c*(0.2+n.l)`, "", "lit", f.x, f.y, [...f.exp.map(Math.round), 255], 4);
  at("lit cube background", "", "lit", 2, 2, [26, 26, 26, 255], 1);
  const pp = px("point", 80, 60);
  out(near(pp, [meta.point, meta.point, meta.point, 255], 4), "point light + attenuation + material", `got ${fmt(pp)} want ~${Math.round(meta.point)}`);

  const fn = px("fog", ...meta.fog.near), ff = px("fog", ...meta.fog.far);
  out(near(fn, [227, 0, 28, 255], 4), "linear fog, near quad", `got ${fmt(fn)} want (227,0,28)`);
  out(near(ff, [57, 0, 198, 255], 4), "linear fog, far quad", `got ${fmt(ff)} want (57,0,198)`);
  out(ff[2] > fn[2] + 100 && ff[0] < fn[0] - 100, "fog colour increases with depth");

  // (destination alpha also goes through glBlendFunc, as in real GL: 0.5*0.5 + 1*0.5 = 0.75)
  at("blend: src-alpha over red", "", "blend", 65, 55, [128, 128, 0, 191], 3);
  at("blend: src-alpha over black", "", "blend", 95, 25, [0, 128, 0, 191], 3);
  at("blend: opaque red untouched", "", "blend", 30, 90, [255, 0, 0, 255], 1);
  at("alpha test GREATER 0.5 rejects a=0.3", "", "blend", 135, 95, [0, 0, 0, 255], 1);
  at("alpha test GREATER 0.5 accepts a=0.8", "", "blend", 135, 45, [255, 255, 255, 204], 1);

  at("scissor: clear inside rect", "", "scissor", 40, 80, [255, 255, 0, 255], 1);
  at("scissor: outside rect untouched", "", "scissor", 10, 100, [26, 26, 26, 255], 1);
  at("viewport: draw inside viewport", "", "scissor", 120, 90, [0, 0, 255, 255], 1);
  at("viewport: nothing outside (top)", "", "scissor", 120, 30, [26, 26, 26, 255], 1);
  at("viewport: nothing outside (left)", "", "scissor", 70, 100, [26, 26, 26, 255], 1);

  const sm = px("shade_smooth", 80, 73), fl = px("shade_flat", 80, 73);
  out(near(sm, [85, 85, 85, 255], 10), "smooth shading interpolates", `got ${fmt(sm)}`);
  out(near(fl, [0, 0, 255, 255], 1), "flat shading uses provoking (last) vertex", `got ${fmt(fl)}`);

  const edge = []; for (let x = 16; x <= 24; x++) edge.push(px("wire", x, 60)[0]);
  out(px("wire", 80, 60)[0] === 0 && Math.max(...edge) > 128, "polygon mode LINE: edge drawn, interior empty", `centre ${fmt(px("wire", 80, 60))} edge max ${Math.max(...edge)}`);

  at("TRIANGLE_FAN", "", "prims", 40, 75, [255, 0, 0, 255], 1);
  at("QUAD_STRIP", "", "prims", 100, 75, [0, 255, 0, 255], 1);
  at("POLYGON", "", "prims", 45, 25, [0, 0, 255, 255], 1);
  at("TRIANGLE_STRIP", "", "prims", 110, 25, [255, 255, 0, 255], 1);

  out(nonBg("dl_direct", [0, 0, 0]) > 1000, "display list reference frame has content");
  out(diff("dl_direct", "dl_list") === 0, "display list replay == direct drawing", `${diff("dl_direct", "dl_list")} bytes differ`);
  out(diff("dl_direct", "dl_nested") === 0, "nested display list replay == direct", `${diff("dl_direct", "dl_nested")} bytes differ`);
  out(diff("dl_direct", "dl_ce") === 0, "GL_COMPILE_AND_EXECUTE == direct", `${diff("dl_direct", "dl_ce")} bytes differ`);
  out(nonBg("dl_compile", [0, 0, 0]) === 0, "GL_COMPILE does not execute");
  out(nonBg("imm_strip", [0, 0, 0]) > 5000, "immediate triangle strip has content");
  out(diff("imm_strip", "draw_strip") === 0, "DRAW == BEGIN/VERTEX/END", `${diff("imm_strip", "draw_strip")} bytes differ`);
  out(diff("imm_strip", "split_strip") === 0, "stream split message-by-message == whole", `${diff("imm_strip", "split_strip")} bytes differ`);

  const bg = [13, 18, 31];
  out(nonBg("demo3", bg) > 4000, "demo scene (textured/lit/fogged cube, DRAW ground) has content", `${nonBg("demo3", bg)} px`);
  out(diff("demo0", "demo3") > 2000, "demo frames differ (rotation)");
  const o = px("demo3", 35, 95);
  out(o[0] > 100, "demo translucent overlay present", `got ${fmt(o)}`);
  const ds = Object.entries(direct?.scenes ?? {});
  out(direct && direct.kind === label && !direct.fellBack && ds.length === Object.keys(frames).length && ds.every(([, s]) => !s.missing && s.bw === 160 && s.bh === 120),
    "direct mode: onFrame called for every PRESENT with 160x120 ImageBitmap, no readback", JSON.stringify(direct && { k: direct.kind, c: direct.count, fb: direct.fellBack }));
  const badScenes = ds.filter(([, s]) => s.bad > 0 || s.missing);
  out(badScenes.length === 0, "direct bitmap == readback frame (all scenes, tol 3)", badScenes.map(([n, s]) => `${n}:bad=${s.bad},max=${s.max}`).join(" "));
  out(direct?.scenes?.demo3 && direct.scenes.demo3.max <= 3, "direct bitmap upright, matches demo frame", JSON.stringify(direct?.scenes?.demo3));
  const cols = new Set(); { const D = frames.demo3.data; for (let i = 0; i < D.length; i += 4) cols.add((D[i] >> 3) << 10 | (D[i + 1] >> 3) << 5 | D[i + 2] >> 3); }
  out(cols.size > 40, "demo scene has many distinct colours (texture + lighting + fog)", `${cols.size}`);
}

// ------------------------------------------------------------------ main
const BASE = ["--headless=new", "--ignore-gpu-blocklist", "--disable-gpu-sandbox"];
const WEBGL_FLAGSETS = [
  [...BASE, "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--enable-webgl"],
  [...BASE, "--use-gl=angle", "--use-angle=swiftshader-webgl", "--enable-unsafe-swiftshader"],
  [...BASE, "--enable-unsafe-swiftshader"],
  ["--headless=new"],
];
const WEBGPU_FLAGSETS = [
  [...BASE, "--enable-unsafe-webgpu", "--enable-features=Vulkan", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
  [...BASE, "--enable-unsafe-webgpu", "--use-webgpu-adapter=swiftshader", "--enable-features=Vulkan", "--use-vulkan=swiftshader", "--enable-unsafe-swiftshader"],
  [...BASE, "--enable-unsafe-webgpu", "--enable-features=Vulkan,WebGPU"],
];

async function attempt(sets, prefer, want, shot) {
  const why = [];
  for (const flags of sets) {
    const r = await runPage(flags, prefer, shot);
    if (!r.error && r.kind === want) return { r, flags };
    why.push(`${flags.filter((f) => f.startsWith("--use") || f.startsWith("--enable-unsafe")).join(" ")} => ${r.error ? r.error.split("\n")[0] : "kind=" + r.kind}`);
  }
  return { why };
}

const w2 = await attempt(WEBGL_FLAGSETS, "webgl2", "webgl2", shotPath);
if (!w2.r) {
  console.log("FAIL [webgl2] could not get a working WebGL2 backend in headless Chrome");
  for (const l of w2.why) console.log("  tried: " + l);
  failures++;
} else {
  console.log(`info [webgl2] backend ok with flags: ${w2.flags.join(" ")}`);
  suite("webgl2", w2.r);
}

const wg = await attempt(WEBGPU_FLAGSETS, "webgpu", "webgpu", shotPath ? shotPath.replace(/(\.png)?$/i, "-webgpu.png") : undefined);
if (!wg.r) {
  console.log("SKIP [webgpu] not available in this headless Chrome (backend fell back to WebGL2 as designed)");
  for (const l of wg.why) console.log("  tried: " + l);
} else {
  console.log(`info [webgpu] backend ok with flags: ${wg.flags.join(" ")}`);
  suite("webgpu", wg.r);
}

server.close();
console.log(failures === 0 ? "ALL PASS" : `FAILED (${failures} failing checks)`);
process.exit(failures === 0 ? 0 : 1);
