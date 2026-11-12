// Open the real-Wine page in headless Chrome, start a program, wait, screenshot. Needs `node web/serve.mjs 8080`.
// node web/runreal.mjs <out.png> [seconds=60] [program and args = notepad] ["x,y;x,y" clicks in screen coordinates, 2 s apart, after the wait]
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const CHROME = ["C:/Program Files/Google/Chrome/Application/chrome.exe", "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"].find(existsSync);
const [out, secs = "60", run = "notepad", clicks = ""] = process.argv.slice(2);
if (!CHROME || !out) { console.log("usage: node web/runreal.mjs <out.png> [seconds] [program] [clicks]"); process.exit(1); }
const PORT = 9335, sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const chrome = spawn(CHROME, ["--headless=new", "--ignore-gpu-blocklist", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", `--remote-debugging-port=${PORT}`, `--user-data-dir=${mkdtempSync(join(tmpdir(), "plonk-real-"))}`,
  "--window-size=1100,1000", "--hide-scrollbars", "about:blank"], { stdio: "ignore" });
let ws, nextId = 1;
const pending = new Map(), errors = [];
for (let i = 0; i < 50 && !ws; i++) {
  try {
    const page = (await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json()).find((t) => t.type === "page");
    if (page) { ws = new WebSocket(page.webSocketDebuggerUrl); await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; }); }
  } catch { await sleep(200); }
}
ws.onmessage = (m) => {
  const d = JSON.parse(m.data);
  if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); }
  if (d.method === "Runtime.exceptionThrown") errors.push(d.params.exceptionDetails.exception?.description ?? d.params.exceptionDetails.text);
};
const send = (method, params = {}) => new Promise((res) => {
  const id = nextId++;
  pending.set(id, res);
  ws.send(JSON.stringify({ id, method, params }));
  setTimeout(() => { if (pending.delete(id)) res({ error: "timeout" }); }, 30000);
});
const evalJs = async (expression) => (await send("Runtime.evaluate", { expression, returnByValue: true })).result?.result?.value;
try {
  await send("Runtime.enable");
  await send("Page.enable");
  await send("Page.navigate", { url: `http://localhost:8080/web/real.html?run=${encodeURIComponent(run)}${process.env.QUERY ? "&" + process.env.QUERY : ""}` });
  const t0 = Date.now();
  let shownAt = 0;
  while (Date.now() - t0 < +secs * 1000) {
    await sleep(2000);
    const st = await evalJs(`document.getElementById("stxt").textContent`);
    // a window is up once the canvas is no longer one flat colour
    const varied = await evalJs(`(() => { const c = document.getElementById("screen"), d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data; for (let i = 4; i < d.length; i += 4096) if (d[i] !== d[0] || d[i + 1] !== d[1]) return true; return false; })()`);
    if (varied && !shownAt) { shownAt = Date.now() - t0; console.log(`first window pixels after ${(shownAt / 1000).toFixed(0)} s (${st})`); }
    if (shownAt && Date.now() - t0 > shownAt + +(process.env.AFTER || 6) * 1000) break;
  }
  const rect = await evalJs(`(() => { const r = document.getElementById("screen").getBoundingClientRect(); return [r.left, r.top]; })()`);
  for (const c of clicks.split(";").filter(Boolean)) {
    const [cx, cy] = c.split(",").map(Number), x = rect[0] + cx, y = rect[1] + cy;
    await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
    await send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
    await send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
    await sleep(2000);
  }
  console.log("gl:", await evalJs(`JSON.stringify({ov: document.getElementById("glov").style.cssText, w: document.getElementById("glov").width, nz: window.__glNonZero})`));
  console.log("kernel:", await evalJs(`(() => { const { x, msg } = window.__lx; x.plonk_lx_ps(); return "presents " + x.plonk_lx_gl_presents() + " queued " + x.plonk_lx_gl_len() + String.fromCharCode(10) + msg().split(String.fromCharCode(10)).filter((l) => !l.includes("exited")).map((l) => l.slice(0, 160)).join(String.fromCharCode(10)); })()`));
  console.log("status:", await evalJs(`document.getElementById("stxt").textContent`));
  console.log("log:", ((await evalJs(`document.getElementById("log").textContent`)) ?? "").slice(0, 700));
  const shot = await send("Page.captureScreenshot", { format: "png" });
  if (shot.result?.data) { writeFileSync(out, Buffer.from(shot.result.data, "base64")); console.log("wrote", out); } else console.log("screenshot failed", shot.error);
  if (errors.length) console.log("page errors:", errors.join("\n"));
} finally { chrome.kill(); process.exit(0); }
