// List a PE's exported names/ordinals: node web/pe_exports.mjs <file>
import { readFileSync } from "node:fs";
const b = readFileSync(process.argv[2]);
const pe = b.readUInt32LE(0x3c), nsec = b.readUInt16LE(pe + 6), optSize = b.readUInt16LE(pe + 20), opt = pe + 24;
const expRva = b.readUInt32LE(opt + 96), expSize = b.readUInt32LE(opt + 100);
const secs = [];
for (let i = 0; i < nsec; i++) { const s = opt + optSize + i * 40; secs.push({ va: b.readUInt32LE(s + 12), vs: b.readUInt32LE(s + 8), raw: b.readUInt32LE(s + 20) }); }
const off = (rva) => { for (const s of secs) if (rva >= s.va && rva < s.va + Math.max(s.vs, 1)) return rva - s.va + s.raw; throw new Error("rva " + rva.toString(16)); };
if (!expRva) { console.log("no exports"); process.exit(0); }
const e = off(expRva), base = b.readUInt32LE(e + 16), nfun = b.readUInt32LE(e + 20), nnam = b.readUInt32LE(e + 24);
const fun = off(b.readUInt32LE(e + 28)), nam = off(b.readUInt32LE(e + 32)), ord = off(b.readUInt32LE(e + 36));
const names = new Map();
for (let i = 0; i < nnam; i++) { const s = off(b.readUInt32LE(nam + i * 4)); let t = ""; for (let k = s; b[k]; k++) t += String.fromCharCode(b[k]); names.set(b.readUInt16LE(ord + i * 2), t); }
console.log(`base ${base}, ${nfun} functions, ${nnam} named`);
for (let i = 0; i < nfun; i++) {
  const rva = b.readUInt32LE(fun + i * 4);
  if (!rva) continue;
  const fwd = rva >= expRva && rva < expRva + expSize ? " -> fwd" : "";
  console.log(`  #${base + i} ${names.get(i) ?? "(ordinal only)"} rva ${rva.toString(16)}${fwd}`);
}
