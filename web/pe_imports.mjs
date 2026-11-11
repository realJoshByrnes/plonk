// Minimal PE32 import scanner shared by the dev server and the page: returns lowercase DLL names that a
// PE file imports (normal + delay-load). Used to prefetch dependencies before the guest starts, so lazy
// file requests never have to happen in the middle of a window procedure.
export function peImports(buf) {
  const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  if (buf.length < 0x100 || dv.getUint16(0, true) !== 0x5a4d) return [];
  const pe = dv.getUint32(0x3c, true);
  if (dv.getUint32(pe, true) !== 0x4550 || dv.getUint16(pe + 24, true) !== 0x10b) return [];
  const nsec = dv.getUint16(pe + 6, true), optsz = dv.getUint16(pe + 20, true), opt = pe + 24;
  const imageBase = dv.getUint32(opt + 28, true);
  const secs = [];
  for (let i = 0; i < nsec; i++) {
    const s = opt + optsz + i * 40;
    secs.push({ va: dv.getUint32(s + 12, true), vsz: dv.getUint32(s + 8, true), raw: dv.getUint32(s + 20, true), rsz: dv.getUint32(s + 16, true) });
  }
  const off = (rva) => { for (const s of secs) if (rva >= s.va && rva < s.va + Math.max(s.vsz, s.rsz)) return rva - s.va + s.raw; return -1; };
  const cstr = (o) => { let e = o; while (e < buf.length && buf[e] !== 0) e++; return new TextDecoder().decode(buf.subarray(o, e)).toLowerCase(); };
  const names = new Set();
  const dir = (i) => ({ rva: dv.getUint32(opt + 96 + i * 8, true), size: dv.getUint32(opt + 96 + i * 8 + 4, true) });
  const imp = dir(1);
  if (imp.rva) {
    for (let o = off(imp.rva); o > 0 && o + 20 <= buf.length; o += 20) {
      const name = dv.getUint32(o + 12, true);
      if (!name) break;
      const n = off(name); if (n > 0) names.add(cstr(n));
    }
  }
  const del = dir(13);
  if (del.rva) {
    for (let o = off(del.rva); o > 0 && o + 32 <= buf.length; o += 32) {
      const attrs = dv.getUint32(o, true), name = dv.getUint32(o + 4, true);
      if (!name) break;
      const rva = attrs & 1 ? name : name - imageBase;
      const n = off(rva); if (n > 0) names.add(cstr(n));
    }
  }
  return [...names];
}
