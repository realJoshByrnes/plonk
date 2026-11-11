// Interpreter speed in wasm (Node): node web/bench_wasm.mjs [guest.exe]  -> guest MIPS (retired instructions / wall time)
import { readFileSync, readdirSync } from "node:fs";
const root = new URL("../", import.meta.url), wineOut = new URL("wine/out/", root);
const { instance } = await WebAssembly.instantiate(readFileSync(new URL("target/wasm32-unknown-unknown/release/plonk_host.wasm", root)), {});
const x = instance.exports, enc = new TextEncoder();
const put = (b) => { const p = x.plonk_alloc(b.length) >>> 0; new Uint8Array(x.memory.buffer, p, b.length).set(b); return p; };
const add = (n, b) => { const e = enc.encode(n); x.plonk_wine_add_file(put(e), e.length, put(b), b.length); };
const msg = () => Buffer.from(new Uint8Array(x.memory.buffer, x.plonk_wine_msg_ptr() >>> 0, x.plonk_wine_msg_len())).toString();
for (const f of readdirSync(wineOut)) if (f.endsWith(".dll")) add(`c:\\windows\\system32\\${f}`, readFileSync(new URL(f, wineOut)));
for (const f of readdirSync(new URL("nls/", wineOut))) add(`nls\\${f}`, readFileSync(new URL(`nls/${f}`, wineOut)));
add("c:\\bench.exe", readFileSync(process.argv[2] ?? new URL("tests/guests/bench.exe", root)));
const p = enc.encode("c:\\bench.exe");
if (x.plonk_wine_boot(put(p), p.length) !== 0) throw new Error("boot failed: " + msg());
const t0 = performance.now(), i0 = x.plonk_wine_retired();
let st, runs = 0;
while ((st = x.plonk_wine_run()) === 2) runs++;
const dt = (performance.now() - t0) / 1000, n = x.plonk_wine_retired() - i0;
const out = Buffer.from(new Uint8Array(x.memory.buffer, x.plonk_wine_stdout_ptr() >>> 0, x.plonk_wine_stdout_len())).toString().trim();
console.log(`state=${st} output="${out}"\n${(n / 1e6).toFixed(0)} M instructions in ${dt.toFixed(2)} s = ${(n / 1e6 / dt).toFixed(0)} MIPS`);
