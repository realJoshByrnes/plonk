// Headless Node runner: loads the wasm build and runs the in-wasm selftest.
import { readFileSync } from "node:fs";
const path = new URL("../target/wasm32-unknown-unknown/release/plonk_host.wasm", import.meta.url);
const { instance } = await WebAssembly.instantiate(readFileSync(path), {});
const got = instance.exports.plonk_selftest();
console.log(got === 5 ? "wasm selftest: PASS" : `wasm selftest: FAIL (eax=${got})`);
process.exit(got === 5 ? 0 : 1);
