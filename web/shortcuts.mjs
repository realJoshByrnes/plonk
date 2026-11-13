// Shortcut (.lnk) support for the real-Wine page: parse the kernel's listing, pull an icon out of a PE file, command-line splitting.

export function parseList(text) {
  return text.split("\n").filter(Boolean).map((l) => {
    const [kind, name, target, args, cwd, icon, wtarget, group, wlnk] = l.split("\t");
    return { kind, name, target, args, cwd, icon, wtarget, group, wlnk };
  });
}

// `a "b c" d` -> ["a", "b c", "d"]
export function splitArgs(s) {
  const out = [];
  const re = /"([^"]*)"|(\S+)/g;
  for (let m; (m = re.exec(s || ""));) out.push(m[1] ?? m[2]);
  return out;
}

const GENERIC = "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect x="3" y="5" width="26" height="22" rx="3" fill="#5b6b86"/><rect x="3" y="5" width="26" height="6" rx="3" fill="#8ea2c4"/><rect x="7" y="15" width="8" height="8" rx="1" fill="#dbe4f5"/></svg>');
export const genericIcon = GENERIC;

// The best icon of a PE file's first RT_GROUP_ICON (or of a .ico file) as an .ico blob URL; null when there is none.
export function iconFromBytes(b) {
  try {
    const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
    if (dv.getUint16(0, true) === 0 && dv.getUint16(2, true) === 1) return URL.createObjectURL(new Blob([b], { type: "image/x-icon" }));
    if (dv.getUint16(0, true) !== 0x5a4d) return null;
    const pe = dv.getUint32(0x3c, true);
    if (dv.getUint32(pe, true) !== 0x4550) return null;
    const nsec = dv.getUint16(pe + 6, true), opt = pe + 24, magic = dv.getUint16(opt, true);
    const dd = opt + (magic === 0x20b ? 112 : 96); // data directories
    const rsrcRva = dv.getUint32(dd + 16, true);
    if (!rsrcRva) return null;
    const secs = [];
    for (let i = 0; i < nsec; i++) { const s = opt + dv.getUint16(pe + 20, true) + i * 40; secs.push([dv.getUint32(s + 12, true), dv.getUint32(s + 8, true), dv.getUint32(s + 20, true)]); }
    const off = (rva) => { for (const [va, vs, raw] of secs) if (rva >= va && rva < va + Math.max(vs, 1)) return raw + rva - va; return -1; };
    const base = off(rsrcRva);
    if (base < 0) return null;
    const entries = (o) => { const n = dv.getUint16(o + 12, true) + dv.getUint16(o + 14, true), r = []; for (let i = 0; i < n; i++) r.push([dv.getUint32(o + 16 + i * 8, true), dv.getUint32(o + 20 + i * 8, true)]); return r; };
    const sub = (e) => base + (e & 0x7fffffff);
    const find = (o, id) => entries(o).find(([n]) => n === id)?.[1];
    // a leaf: descend through name and language levels to the first data entry
    const leaf = (e) => { let o = e; for (let i = 0; i < 3 && (o & 0x80000000); i++) { const d = sub(o); o = entries(d)[0][1]; } const d = base + o; return [dv.getUint32(d, true), dv.getUint32(d + 4, true)]; };
    const root = base, grp = find(root, 14), ico = find(root, 3);
    if (grp === undefined || ico === undefined) return null;
    const gdir = sub(grp), g0 = entries(gdir)[0][1];
    const [grva, gsz] = leaf(g0);
    const go = off(grva);
    const count = dv.getUint16(go + 4, true);
    let best = null;
    for (let i = 0; i < count; i++) {
      const e = go + 6 + i * 14, w = dv.getUint8(e) || 256, bits = dv.getUint16(e + 6, true);
      const score = (w >= 32 && w <= 48 ? 1000 : w > 48 ? 500 - w : w) * 100 + bits;
      if (!best || score > best.score) best = { score, e, id: dv.getUint16(e + 12, true) };
    }
    if (!best) return null;
    const idir = sub(ico), ie = entries(idir).find(([n]) => n === best.id);
    if (!ie) return null;
    const [irva, isz] = leaf(ie[1]);
    const io = off(irva);
    const out = new Uint8Array(22 + isz), od = new DataView(out.buffer);
    od.setUint16(2, 1, true); od.setUint16(4, 1, true);
    for (let i = 0; i < 12; i++) out[6 + i] = dv.getUint8(best.e + i);
    od.setUint32(14, isz, true); od.setUint32(18, 22, true);
    out.set(new Uint8Array(b.buffer, b.byteOffset + io, isz), 22);
    return URL.createObjectURL(new Blob([out], { type: "image/x-icon" }));
  } catch { return null; }
}

// A minimal .lnk (LinkInfo with a local base path, working dir, arguments) for tests.
export function makeLnk(target, args = "", cwd = "") {
  const te = new TextEncoder(), t = te.encode(target + "\0"), u = (s) => { const a = new Uint8Array(2 + s.length * 2), v = new DataView(a.buffer); v.setUint16(0, s.length, true); for (let i = 0; i < s.length; i++) v.setUint16(2 + i * 2, s.charCodeAt(i), true); return a; };
  const liSize = 0x1c + t.length + 1, li = new Uint8Array(liSize), lv = new DataView(li.buffer);
  lv.setUint32(0, liSize, true); lv.setUint32(4, 0x1c, true); lv.setUint32(8, 1, true); lv.setUint32(16, 0x1c, true); lv.setUint32(24, 0x1c + t.length, true);
  li.set(t, 0x1c);
  const parts = [new Uint8Array(76), li, u(cwd), u(args)], hv = new DataView(parts[0].buffer);
  hv.setUint32(0, 0x4c, true); hv.setUint32(0x14, 2 | 0x10 | 0x20 | 0x80, true);
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0) + 4);
  let o = 0;
  for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
}
