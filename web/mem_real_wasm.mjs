// Memory check of the real-Wine path in wasm: restore the snapshot, run a program to completion several times, print the
// wasm memory size after each run (it must level off).  node web/mem_real_wasm.mjs [rounds=6] [program and args = /home/user/hello.exe]
import { readFileSync, readdirSync } from "node:fs";
const root = new URL("../", import.meta.url);
const { instance } = await WebAssembly.instantiate(readFileSync(new URL("target/wasm32-unknown-unknown/release/plonk_host.wasm", root)), {});
const x = instance.exports;
const put = (bytes) => { const p = x.plonk_alloc(bytes.length); new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes); return p; };
const msg = () => new TextDecoder().decode(new Uint8Array(x.memory.buffer, x.plonk_lx_msg(), x.plonk_lx_msg_len()));
const mb = () => (x.memory.buffer.byteLength / 1048576) | 0;
for (const f of ["wine/root.tar", "wine/prefix.tar"]) { const d = readFileSync(new URL(f, root)); x.plonk_lx_add_tar(put(d), d.length); }
for (const f of readdirSync(new URL("tests/guests", root)).filter((f) => f.endsWith(".exe"))) {
  const n = new TextEncoder().encode("/home/user/" + f), d = readFileSync(new URL("tests/guests/" + f, root));
  x.plonk_lx_add_file(put(n), n.length, put(d), d.length);
}
const snap = readFileSync(new URL("wine/snapshot.bin", root));
if (x.plonk_lx_restore(put(snap), snap.length) < 0) throw new Error("restore: " + msg());
console.log(`restored: ${mb()} MB`);
const rounds = +(process.argv[2] || 6), args = process.argv.slice(3);
if (!args.length) args.push("/home/user/hello.exe");
for (let i = 1; i <= rounds; i++) {
  const cmd = new TextEncoder().encode(["/usr/local/bin/wine", ...args].join("\n"));
  const pid = x.plonk_lx_spawn(put(cmd), cmd.length);
  if (pid < 0) throw new Error("spawn: " + msg());
  let last = performance.now();
  const t0 = last;
  while (x.plonk_lx_alive(pid) && performance.now() - t0 < 60000) {
    const now = performance.now();
    const r = x.plonk_lx_run(20_000_000, Math.round((now - last) * 1000));
    last = now;
    if (r < 0) throw new Error(msg());
    if (r === 1) await new Promise((res) => setTimeout(res, 2));
  }
  x.plonk_lx_take_stdout();
  console.log(`run ${i}: ${mb()} MB after ${((performance.now() - t0) / 1000).toFixed(1)} s${x.plonk_lx_alive(pid) ? " (still running)" : ""}`);
}
