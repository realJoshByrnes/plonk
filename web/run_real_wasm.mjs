// Headless check of the real-Wine path in wasm: node web/run_real_wasm.mjs [guest exe path] [seconds] [out.ppm]
// Loads wine/root.tar + wine/prefix.tar (+ tests/guests/*.exe as /home/user/*), starts `wine <exe>`, pumps it against the real clock.
import { readFileSync, writeFileSync, readdirSync } from "node:fs";
const root = new URL("../", import.meta.url);
const { instance } = await WebAssembly.instantiate(readFileSync(new URL("target/wasm32-unknown-unknown/release/plonk_host.wasm", root)), {});
const x = instance.exports;
const put = (bytes) => { const p = x.plonk_alloc(bytes.length); new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes); return p; };
const msg = () => new TextDecoder().decode(new Uint8Array(x.memory.buffer, x.plonk_lx_msg(), x.plonk_lx_msg_len()));
const t0 = performance.now();
for (const f of ["wine/root.tar", "wine/prefix.tar"]) { const d = readFileSync(new URL(f, root)); x.plonk_lx_add_tar(put(d), d.length); }
for (const f of readdirSync(new URL("tests/guests", root)).filter((f) => f.endsWith(".exe"))) {
  const n = new TextEncoder().encode("/home/user/" + f), d = readFileSync(new URL("tests/guests/" + f, root));
  x.plonk_lx_add_file(put(n), n.length, put(d), d.length);
}
console.log(`files loaded in ${Math.round(performance.now() - t0)} ms, wasm memory ${(x.memory.buffer.byteLength / 1048576) | 0} MB`);
const exe = process.argv[2] || "/home/user/gui_stay.exe", secs = +(process.argv[3] || 20);
const cmd = new TextEncoder().encode("/usr/local/bin/wine\n" + exe);
const cp = put(cmd);
if (x.plonk_lx_spawn(cp, cmd.length) < 0) throw new Error("spawn: " + msg());
let last = performance.now(), busy = 0, shown = false;
const start = last;
while (performance.now() - start < secs * 1000) {
  const now = performance.now();
  const r = x.plonk_lx_run(20_000_000, Math.round((now - last) * 1000));
  last = now;
  busy += performance.now() - now;
  x.plonk_lx_take_stdout(); const out = msg(); if (out) process.stdout.write(out);
  if (r < 0) { console.log("error:", msg()); break; }
  if (r === 1) { if (!shown && x.plonk_lx_frame() > 0) { shown = true; console.log(`first idle with a frame after ${((now - start) / 1000).toFixed(1)} s`); } await new Promise((res) => setTimeout(res, 4)); }
}
const mi = x.plonk_lx_mips_total();
console.log(`${mi} M instructions in ${(busy / 1000).toFixed(1)} s busy = ${(mi / (busy / 1000)).toFixed(0)} MIPS, wasm memory ${(x.memory.buffer.byteLength / 1048576) | 0} MB`);
x.plonk_lx_ps(); console.log(msg());
if (process.argv[4]) {
  const [w, h] = [1024, 768], p = x.plonk_lx_compose(), rgba = new Uint8Array(x.memory.buffer, p, w * h * 4), rgb = Buffer.alloc(w * h * 3);
  for (let i = 0; i < w * h; i++) { rgb[i * 3] = rgba[i * 4]; rgb[i * 3 + 1] = rgba[i * 4 + 1]; rgb[i * 3 + 2] = rgba[i * 4 + 2]; }
  writeFileSync(process.argv[4], Buffer.concat([Buffer.from(`P6\n${w} ${h}\n255\n`), rgb]));
}
