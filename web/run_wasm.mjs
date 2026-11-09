// Headless Node runner: loads the wasm build, runs the selftest and hello.exe.
import { readFileSync } from "node:fs";
const rd = (p) => readFileSync(new URL(p, import.meta.url));
const { instance } = await WebAssembly.instantiate(rd("../target/wasm32-unknown-unknown/release/plonk_host.wasm"), {});
const x = instance.exports;
let ok = true;
const st = x.plonk_selftest();
console.log(st === 5 ? "wasm selftest: PASS" : `wasm selftest: FAIL (eax=${st})`);
ok &&= st === 5;

const exe = rd("../tests/guests/hello.exe");
const p = x.plonk_alloc(exe.length);
new Uint8Array(x.memory.buffer, p, exe.length).set(exe);
const code = x.plonk_run_pe(p, exe.length);
const str = (ptr, len) => Buffer.from(new Uint8Array(x.memory.buffer, ptr, len)).toString();
const out = str(x.plonk_stdout_ptr(), x.plonk_stdout_len());
const good = code === 0 && out === "Hello from Plonk\r\n";
console.log(good ? "wasm hello.exe: PASS" : `wasm hello.exe: FAIL code=${code} out=${JSON.stringify(out)} err=${str(x.plonk_error_ptr(), x.plonk_error_len())}`);
process.exit(ok && good ? 0 : 1);
