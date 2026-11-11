// Time the page's wasm flow in Node: node web/perf.mjs [app.exe]
import { readFileSync, readdirSync } from "node:fs";
const root = new URL("../", import.meta.url);
const app = process.argv[2] ?? "gui_stay.exe";
const t = (n, f) => { const a = performance.now(); const r = f(); console.log(n.padEnd(22), (performance.now() - a).toFixed(1), "ms"); return r; };
const { instance } = await WebAssembly.instantiate(readFileSync(new URL("target/wasm32-unknown-unknown/release/plonk_host.wasm", root)), {});
const x = instance.exports, enc = new TextEncoder();
const put = (b) => { const p = x.plonk_alloc(b.length) >>> 0; new Uint8Array(x.memory.buffer, p, b.length).set(b); return p; };
const add = (n, b) => { const e = enc.encode(n); x.plonk_wine_add_file(put(e), e.length, put(b), b.length); };
const out = new URL("wine/out/", root);
t("add files", () => {
  for (const f of readdirSync(out)) if (f.endsWith(".dll")) add(`c:\\windows\\system32\\${f}`, readFileSync(new URL(f, out)));
  for (const f of readdirSync(new URL("nls/", out))) add(`nls\\${f}`, readFileSync(new URL(`nls/${f}`, out)));
  add("c:\\app.exe", readFileSync(new URL(`tests/guests/${app}`, root)));
});
const p = enc.encode("c:\\app.exe");
t("boot", () => x.plonk_wine_boot(put(p), p.length));
t("first run", () => x.plonk_wine_run());
t("compose", () => x.plonk_wine_compose(1024, 768));
t("tick+run x10", () => { for (let i = 0; i < 10; i++) { x.plonk_wine_tick(50); x.plonk_wine_run(); } });
t("mouse down+run", () => { x.plonk_wine_mouse(0, 60, 50); x.plonk_wine_run(); });
t("mouse move+run x10", () => { for (let i = 0; i < 10; i++) { x.plonk_wine_mouse(1, 70 + i * 5, 60); x.plonk_wine_run(); x.plonk_wine_compose(1024, 768); } });
console.log("wasm memory MB:", (x.memory.buffer.byteLength / 1048576).toFixed(0));
