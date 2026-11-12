// Where does the real-Wine path spend guest instructions? node web/perf_real_wasm.mjs [program=notepad]
// Restores wine/snapshot.bin, starts the program, then measures: start-up, an idle second, mouse moves, a click, key presses.
import { readFileSync } from "node:fs";
const root = new URL("../", import.meta.url);
const { instance } = await WebAssembly.instantiate(readFileSync(new URL("target/wasm32-unknown-unknown/release/plonk_host.wasm", root)), {});
const x = instance.exports;
const put = (bytes) => { const p = x.plonk_alloc(bytes.length); new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes); return p; };
const msg = () => new TextDecoder().decode(new Uint8Array(x.memory.buffer, x.plonk_lx_msg(), x.plonk_lx_msg_len()));
for (const f of ["wine/root.tar", "wine/prefix.tar"]) { const d = readFileSync(new URL(f, root)); x.plonk_lx_add_tar(put(d), d.length); }
const snap = readFileSync(new URL("wine/snapshot.bin", root));
if (x.plonk_lx_restore(put(snap), snap.length) !== 0) throw new Error("restore: " + msg());
const args = (process.argv[2] || "notepad").split(" ");
for (const a of args) if (a.startsWith("/home/user/")) { const n = new TextEncoder().encode(a), d = readFileSync(new URL("tests/guests/" + a.slice(11), root)); x.plonk_lx_add_file(put(n), n.length, put(d), d.length); }
const cmd = new TextEncoder().encode(["/usr/local/bin/wine", ...args].join("\n"));
x.plonk_lx_spawn(put(cmd), cmd.length);
// run until everything waits; guest time follows wall time. Returns [M instructions, ms busy].
let last = performance.now();
function settle(maxMs, minMs = 0) {
  const m0 = x.plonk_lx_mips_total(), t0 = performance.now();
  let busy = 0, idleRounds = 0;
  while (performance.now() - t0 < maxMs) {
    const now = performance.now();
    const r = x.plonk_lx_run(5_000_000, Math.round((now - last) * 1000));
    last = now;
    if (r < 0) throw new Error(msg());
    if (r === 1) { if (++idleRounds > 3 && performance.now() - t0 >= minMs) break; const until = performance.now() + 4; while (performance.now() < until); } else { idleRounds = 0; busy += performance.now() - now; }
  }
  x.plonk_lx_take_stdout(); const out = msg(); if (out) process.stdout.write(out);
  return [x.plonk_lx_mips_total() - m0, Math.round(busy)];
}
const show = (what, [mi, ms]) => console.log(`${what.padEnd(28)} ${String(mi).padStart(6)} M instructions, ${String(ms).padStart(6)} ms busy`);
show("start-up to idle", settle(120000, 1500));
show("idle for 3 s", settle(3000, 3000));
for (let i = 0; i < 10; i++) x.plonk_lx_mouse(1, 200 + i * 10, 200 + i * 5, 0);
show("10 mouse moves (batched)", settle(20000));
let tot = [0, 0];
for (let i = 0; i < 10; i++) { x.plonk_lx_mouse(1, 300 + i * 10, 300, 0); const r = settle(20000); tot = [tot[0] + r[0], tot[1] + r[1]]; }
show("10 mouse moves (one by one)", tot);
x.plonk_lx_mouse(0, 300, 300, 0); x.plonk_lx_mouse(2, 300, 300, 0);
show("click in the client area", settle(20000));
for (const vk of [72, 73]) { x.plonk_lx_key(1, vk, 0); x.plonk_lx_key(0, vk, 0); }
show("two key presses", settle(20000));
x.plonk_lx_ps(); console.log(msg().split("\n").filter((l) => !l.includes("exited")).map((l) => l.slice(0, 110)).join("\n"));
