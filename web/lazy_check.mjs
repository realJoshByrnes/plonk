// Node check of lazy file supply: boot with only the core set, satisfy run()==3 requests from wine/out.
import { readFileSync, readdirSync, existsSync } from "node:fs";
const root = new URL("../", import.meta.url);
const app = process.argv[2] ?? "gui_stay.exe";
const CORE = ["apisetschema", "ntdll", "kernelbase", "kernel32", "win32u", "user32", "gdi32", "msvcrt", "ucrtbase", "advapi32", "sechost", "rpcrt4", "imm32"];
const { instance } = await WebAssembly.instantiate(readFileSync(new URL("target/wasm32-unknown-unknown/release/plonk_host.wasm", root)), {});
const x = instance.exports, enc = new TextEncoder();
const put = (b) => { const p = x.plonk_alloc(b.length) >>> 0; new Uint8Array(x.memory.buffer, p, b.length).set(b); return p; };
const add = (n, b) => { const e = enc.encode(n); x.plonk_wine_add_file(put(e), e.length, put(b), b.length); };
const msg = () => Buffer.from(new Uint8Array(x.memory.buffer, x.plonk_wine_msg_ptr() >>> 0, x.plonk_wine_msg_len())).toString();
for (const c of CORE) add(`c:\\windows\\system32\\${c}.dll`, readFileSync(new URL(`wine/out/${c}.dll`, root)));
for (const n of ["locale.nls", "l_intl.nls"]) add(`nls\\${n}`, readFileSync(new URL(`wine/out/nls/${n}`, root)));
add("c:\\app.exe", readFileSync(new URL(`tests/guests/${app}`, root)));
const remote = [...readdirSync(new URL("wine/out/nls/", root)), ...readdirSync(new URL("wine/out/", root)).filter((f) => f.endsWith(".dll") && !CORE.includes(f.replace(".dll", "")))];
for (const f of remote) { const e = enc.encode(f); x.plonk_wine_add_remote(put(e), e.length); }
const p = enc.encode("c:\\app.exe");
console.log("boot", x.plonk_wine_boot(put(p), p.length), msg());
const got = [];
for (let i = 0; i < 60; i++) {
  const st = x.plonk_wine_run();
  if (st !== 3) { console.log("state", st, "after lazily fetching:", got.join(", ")); break; }
  const key = msg(), base = key.split("\\").pop();
  got.push(base);
  const f = existsSync(new URL(`wine/out/${base}`, root)) ? `wine/out/${base}` : `wine/out/nls/${base}`;
  add(key, readFileSync(new URL(f, root)));
}
