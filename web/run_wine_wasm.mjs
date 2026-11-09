// Headless Node check of the Wine path in wasm: boot gui_stay.exe, verify pixels, click close.
import { readFileSync, readdirSync, existsSync, writeFileSync } from "node:fs";
import { deflateSync } from "node:zlib";
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
if (process.env.SNAPSHOT) { // dump the desktop as PNG for eyeballing
  const crc = (b) => { let c, t = []; for (let n = 0; n < 256; n++) { c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; t[n] = c >>> 0; } let r = 0xffffffff; for (const v of b) r = t[(r ^ v) & 255] ^ (r >>> 8); return (r ^ 0xffffffff) >>> 0; };
  const chunk = (type, data) => { const b = Buffer.concat([Buffer.from(type), data]); const o = Buffer.alloc(4); o.writeUInt32BE(data.length); const c = Buffer.alloc(4); c.writeUInt32BE(crc(b)); return Buffer.concat([o, b, c]); };
  const raw = Buffer.alloc((W * 4 + 1) * H); for (let y = 0; y < H; y++) { raw[y * (W * 4 + 1)] = 0; Buffer.from(fb.subarray(y * W * 4, (y + 1) * W * 4)).copy(raw, y * (W * 4 + 1) + 1); }
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(W, 0); ihdr.writeUInt32BE(H, 4); ihdr[8] = 8; ihdr[9] = 6;
  writeFileSync(process.env.SNAPSHOT, Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]));
}
x.plonk_wine_click(10 + 320 - 4 - 10, 10 + 4 + 8); // close box
st = x.plonk_wine_run();
ok &&= st === 1 && msg() === "7";
console.log(ok ? "wasm wine: PASS (closed, exit 7)" : `wasm wine: FAIL state=${st} msg=${msg()}`);
process.exit(ok ? 0 : 1);
