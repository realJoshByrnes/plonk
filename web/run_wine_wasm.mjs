// Headless Node check of the Wine path in wasm: boot gui_stay.exe, verify pixels, click close.
import { readFileSync, readdirSync, existsSync } from "node:fs";
const root = new URL("../", import.meta.url);
const wineOut = new URL("wine/out/", root);
if (!existsSync(new URL("user32.dll", wineOut))) { console.log("wasm wine: SKIP (wine/out not built)"); process.exit(0); }

const { instance } = await WebAssembly.instantiate(readFileSync(new URL("target/wasm32-unknown-unknown/release/plonk_host.wasm", root)), {});
const x = instance.exports;
const enc = new TextEncoder();
const put = (bytes) => { const p = x.plonk_alloc(bytes.length) >>> 0; new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes); return p; };
const addFile = (name, bytes) => { const n = enc.encode(name); x.plonk_wine_add_file(put(n), n.length, put(bytes), bytes.length); };
const msg = () => Buffer.from(new Uint8Array(x.memory.buffer, x.plonk_wine_msg_ptr() >>> 0, x.plonk_wine_msg_len())).toString();

for (const f of readdirSync(wineOut)) if (f.endsWith(".dll")) addFile(`c:\\windows\\system32\\${f}`, readFileSync(new URL(f, wineOut)));
for (const f of readdirSync(new URL("nls/", wineOut))) addFile(`nls\\${f}`, readFileSync(new URL(`nls/${f}`, wineOut)));
addFile("c:\\gui.exe", readFileSync(new URL("tests/guests/gui_stay.exe", root)));

const t0 = Date.now();
const path = enc.encode("c:\\gui.exe");
if (x.plonk_wine_boot(put(path), path.length) !== 0) { console.log("wasm wine: FAIL boot:", msg()); process.exit(1); }
let st = x.plonk_wine_run();
if (st !== 0) { console.log("wasm wine: FAIL run state", st, msg()); process.exit(1); }
const W = 1024, H = 768;
const px = (buf, X, Y) => { const o = (Y * W + X) * 4; return [buf[o], buf[o + 1], buf[o + 2]].join(","); };
const fbp = x.plonk_wine_compose(W, H) >>> 0;
const fb = new Uint8Array(x.memory.buffer, fbp, W * H * 4);
const red = px(fb, 10 + 4 + 50, 10 + 27 + 50);
console.log(`wasm wine: window painted, pixel=${red} (${Date.now() - t0} ms)`);
let ok = red === "200,30,30";
x.plonk_wine_click(10 + 320 - 4 - 10, 10 + 4 + 8); // close box
st = x.plonk_wine_run();
ok &&= st === 1 && msg() === "7";
console.log(ok ? "wasm wine: PASS (closed, exit 7)" : `wasm wine: FAIL state=${st} msg=${msg()}`);
process.exit(ok ? 0 : 1);
