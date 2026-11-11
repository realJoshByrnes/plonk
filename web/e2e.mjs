// Real-browser end-to-end check (Chrome via DevTools protocol). Needs `node web/serve.mjs 8080` running
// and Chrome installed. Usage: node web/e2e.mjs [shot-dir]
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const CHROME = ["C:/Program Files/Google/Chrome/Application/chrome.exe", "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"].find(existsSync);
if (!CHROME) { console.log("e2e: SKIP (no Chrome/Edge found)"); process.exit(0); }
const shotDir = process.argv[2];
const PORT = 9333, URL_ = "http://localhost:8080/web/index.html?nointro";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const chrome = spawn(CHROME, ["--headless=new", "--ignore-gpu-blocklist", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", `--remote-debugging-port=${PORT}`, `--user-data-dir=${mkdtempSync(join(tmpdir(), "plonk-e2e-"))}`,
  "--window-size=1100,1000", "--hide-scrollbars", "about:blank"], { stdio: "ignore" });
let ws, nextId = 1; const pending = new Map(); const consoleErrors = [];
async function connect() {
  for (let i = 0; i < 50; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      const page = list.find((t) => t.type === "page");
      if (page) { ws = new WebSocket(page.webSocketDebuggerUrl); await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; }); break; }
    } catch { await sleep(200); }
  }
  ws.onmessage = (m) => {
    const d = JSON.parse(m.data);
    if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); }
    if (d.method === "Runtime.exceptionThrown") consoleErrors.push(d.params.exceptionDetails.exception?.description ?? d.params.exceptionDetails.text);
    if (d.method === "Runtime.consoleAPICalled" && d.params.type === "error") consoleErrors.push(d.params.args.map((a) => a.value ?? a.description).join(" "));
  };
}
const send = (method, params = {}) => new Promise((res) => { const id = nextId++; pending.set(id, res); ws.send(JSON.stringify({ id, method, params })); });
const evalJs = async (expr) => (await send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true })).result?.result?.value;
const status = () => evalJs("document.getElementById('log').textContent");
async function waitStatus(re, ms = 60000) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) { const s = await status(); if (re.test(s)) return s; await sleep(150); }
  return await status();
}
let origin;
async function click(x, y) {
  const px = origin.x + x, py = origin.y + y;
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x: px, y: py, button: "left", clickCount: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: px, y: py, button: "left", clickCount: 1 });
  await sleep(300);
}
async function type(key, vk, text) {
  await send("Input.dispatchKeyEvent", { type: "keyDown", key, windowsVirtualKeyCode: vk, text });
  await send("Input.dispatchKeyEvent", { type: "keyUp", key, windowsVirtualKeyCode: vk });
  await sleep(300);
}
async function open(app) {
  await send("Page.navigate", { url: URL_ });
  await waitStatus(/Pick an app/);
  await evalJs(`(() => { const s = document.getElementById('app'); s.value = ${JSON.stringify(app)}; document.getElementById('run').click(); })()`);
  await sleep(500);
  await waitStatus(/running|exited|error/);
  const r = await evalJs("(() => { const r = document.getElementById('screen').getBoundingClientRect(); return { x: r.left, y: r.top }; })()");
  origin = r;
  await sleep(1500); // let the first paint settle
}
async function shot(name) {
  if (!shotDir) return;
  const { result } = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(shotDir, `${name}.png`), Buffer.from(result.data, "base64"));
}

const results = [];
const check = async (name, ok, detail) => { results.push(ok); console.log(`e2e ${name}: ${ok ? "PASS" : "FAIL " + detail}`); if (!ok) console.log((await evalJs("window.plonk ? window.plonk.trace() : ''") ?? "").split("\n").slice(-25).join("\n")); };

try {
  await connect();
  await send("Runtime.enable"); await send("Page.enable");

  await open("gui_stay.exe"); // warm-up
  await open("gui_button.exe");
  for (let i = 0; i < 2; i++) { await click(100, 100); await sleep(500); }                // button at desktop (64..174, 92..120)
  await shot("button");
  await click(326, 52);                                             // close box
  let s = await waitStatus(/exited|error/);
  await check("button clicks -> exit code 2", /exited with code 2/.test(s), s);

  await open("gui_edit.exe");
  await click(100, 100);                                            // focus the edit control
  for (const [k, vk, t] of [["h", 72, "h"], ["i", 73, "i"], ["!", 49, "!"]]) await type(k, vk, t);
  await type("Backspace", 8, undefined);
  await shot("edit");
  await click(326, 52);
  s = await waitStatus(/exited|error/);
  await check("edit typing -> exit code 2", /exited with code 2/.test(s), s);

  await open("gui_menu.exe");
  await click(60, 80);                                              // File
  await shot("menu");
  await click(80, 148);                                             // Quit
  s = await waitStatus(/exited|error/);
  await check("menu File>Quit -> exit code 12", /exited with code 12/.test(s), s);

  await open("rust_controls.exe");
  s = await waitStatus(/running|exited|error/, 60000);
  await sleep(1500);
  s = await status();
  await check("rust controls app runs (lazy comctl32/vcruntime)", /running/.test(s), s);
  await shot("rust");

  await open("gl_cube.exe");
  await sleep(6000);
  const gl = await evalJs("({ kind: window.__glKind || null, perf: window.__perf })");
  console.log("PERF", JSON.stringify(gl), await evalJs("window.__glDirect")); await check("3D cube renders via GPU backend", !!gl.kind, JSON.stringify(gl));
  await shot("gl");
  await open("gui_two.exe");
  await click(60, 50);
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x: origin.x + 60, y: origin.y + 50, button: "left", clickCount: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseMoved", x: origin.x + 300, y: origin.y + 250, button: "left" });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: origin.x + 300, y: origin.y + 250, button: "left", clickCount: 1 });
  await sleep(300);
  await shot("two");
  s = await status();
  await check("two windows drag keeps running", /running/.test(s), s);

  await check("no page errors", consoleErrors.length === 0, consoleErrors.join(" | "));
} catch (e) {
  console.log("e2e: ERROR", e);
  results.push(false);
} finally {
  try { ws?.close(); } catch {}
  chrome.kill();
}
console.log(results.every(Boolean) ? "e2e: ALL PASS" : "e2e: FAILURES");
process.exit(results.every(Boolean) ? 0 : 1);
