// JIT differential check: run a static guest ELF from tests/guests in the wasm kernel with the interpreter and with the wasm JIT
// and compare what it prints. node web/jit_diff.mjs fputest.elf   (build first: cargo build --release -p plonk-host --target wasm32-unknown-unknown)
// FPUOFF=<mask> switches classes of x87 translation off (bit (opcode-0xD8)*2 + register form; 16+reg: memory forms with that /reg).
import { readFileSync } from "node:fs";
const root = new URL("../", import.meta.url);
const wasm = readFileSync(new URL("target/wasm32-unknown-unknown/release/plonk_host.wasm", root));
const name = process.argv[2] || "fputest.elf";
const file = readFileSync(new URL("tests/guests/" + name, root));
async function run(jit) {
  const { instance } = await WebAssembly.instantiate(wasm, {});
  const x = instance.exports, table = x.__indirect_function_table;
  const put = (b) => { const p = x.plonk_alloc(b.length); new Uint8Array(x.memory.buffer, p, b.length).set(b); return p; };
  const msg = () => new TextDecoder().decode(new Uint8Array(x.memory.buffer, x.plonk_lx_msg(), x.plonk_lx_msg_len()));
  const n = new TextEncoder().encode("/home/user/" + name);
  x.plonk_lx_add_file(put(n), n.length, put(file), file.length);
  if (jit) { x.plonk_lx_jit_enable(1); if (process.env.FPUOFF) x.plonk_lx_jit_fpu_off(Number(process.env.FPUOFF)); }
  x.plonk_lx_spawn(put(n), n.length);
  let out = "", t0 = performance.now();
  for (let k = 0; k < 4000; k++) {
    const r = x.plonk_lx_run(2_000_000, 20000);
    if (r < 0) throw new Error(msg());
    if (jit) {
      const c = x.plonk_lx_jit_take();
      if (c) {
        const bytes = new Uint8Array(x.memory.buffer, x.plonk_lx_jit_ptr(), c).slice(), id = x.plonk_lx_jit_id(), count = x.plonk_lx_jit_count();
        const inst = new WebAssembly.Instance(new WebAssembly.Module(bytes), { e: { m: x.memory } });
        const first = table.length; table.grow(count);
        for (let i = 0; i < count; i++) table.set(first + i, inst.exports[i]);
        x.plonk_lx_jit_installed(id, first);
      }
    }
    x.plonk_lx_take_stdout(); out += msg();
    if (!x.plonk_lx_alive(1000)) break;
  }
  return { out, ms: Math.round(performance.now() - t0), fns: jit ? x.plonk_lx_jit_stats() : 0 };
}
const a = await run(false), b = await run(true);
console.log(`interpreter ${a.ms} ms, JIT ${b.ms} ms (${b.fns} functions)`);
if (a.out === b.out) console.log("identical output:\n" + a.out);
else { console.log("MISMATCH\n--- interpreter\n" + a.out + "\n--- jit\n" + b.out); process.exit(1); }
