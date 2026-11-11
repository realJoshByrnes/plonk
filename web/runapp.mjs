// Run any 32-bit exe in the real page (headless Chrome via DevTools) and screenshot it. Needs `node web/serve.mjs 8080`.
// node web/runapp.mjs <exe path> <out.png> [seconds=8] ["x,y;x,y;..." clicks in screen coordinates, one per second]
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const CHROME = ["C:/Program Files/Google/Chrome/Application/chrome.exe", "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"].find(existsSync);
const [exe, out, secs = "8", clicks = ""] = process.argv.slice(2);
if (!CHROME || !exe || !out) { console.log("usage: node web/runapp.mjs <exe> <out.png> [seconds] [clicks]"); process.exit(1); }
const PORT = 9334, sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const chrome = spawn(CHROME, ["--headless=new", "--ignore-gpu-blocklist", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", `--remote-debugging-port=${PORT}`,
  `--user-data-dir=${mkdtempSync(join(tmpdir(), "plonk-run-"))}`, "--window-size=1100,1000", "--hide-scrollbars", "about:blank"], { stdio: "ignore" });
let ws, nextId = 1; const pending = new Map(); const errors = [];
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
  if (d.method === "Runtime.consoleAPICalled" && ["error", "info"].includes(d.params.type)) {
    const t = d.params.args.map((a) => a.value ?? a.description).join(" ");
    if (d.params.type === "error" || t.startsWith("[timing]")) errors.push(t);
  }
};
const send = (method, params = {}) => new Promise((res) => { const id = nextId++; pending.set(id, res); ws.send(JSON.stringify({ id, method, params })); });
const evalJs = async (expr) => (await send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true })).result?.result?.value;
await send("Runtime.enable");
await send("Page.navigate", { url: "http://localhost:8080/web/index.html" });
for (let i = 0; i < 100 && !/Pick an app/.test(await evalJs("document.getElementById('log').textContent")); i++) await sleep(200);
const doc = (await send("DOM.getDocument")).result.root.nodeId;
const node = (await send("DOM.querySelector", { nodeId: doc, selector: "#file" })).result.nodeId;
await send("DOM.setFileInputFiles", { files: [resolve(exe)], nodeId: node });
await sleep(300);
await evalJs("document.getElementById('run').click()");
const origin = await (async () => { await sleep(1500); return evalJs("(() => { const r = document.getElementById('screen').getBoundingClientRect(); return { x: r.left, y: r.top }; })()"); })();
const list = clicks.split(";").map((s) => (s ? s.split(",").map(Number) : null)); // empty slot = no click that second
for (let s = 0; s < Number(secs); s++) {
  if (list[s]) {
    const [x, y] = list[s];
    for (const type of ["mousePressed", "mouseReleased"]) await send("Input.dispatchMouseEvent", { type, x: origin.x + x, y: origin.y + y, button: "left", clickCount: 1 });
  }
  await sleep(1000);
}
const { result } = await send("Page.captureScreenshot", { format: "png" });
writeFileSync(out, Buffer.from(result.data, "base64"));
console.log("status:", await evalJs("document.getElementById('log').textContent"));
if (process.env.OPENS) {
  const tr = await evalJs("window.plonk ? window.plonk.trace() : ''");
  const opens = [...new Set(String(tr).split("\n").filter((l) => /\(open |unimplemented|Err/.test(l)))];
  console.log("trace opens (last 25):\n" + opens.slice(-25).join("\n"));
}
if (errors.length) console.log("console:", errors.slice(-5).join(" | ").slice(0, 400));
chrome.kill();
process.exit(0);
