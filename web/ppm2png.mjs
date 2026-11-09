// node web/ppm2png.mjs in.ppm out.png  (P6 only; for eyeballing desktop dumps)
import { readFileSync, writeFileSync } from "node:fs";
import { deflateSync } from "node:zlib";
const d = readFileSync(process.argv[2]);
const [, w, h] = /^P6\n(\d+) (\d+)\n255\n/.exec(d.subarray(0, 20).toString("latin1")).map(Number);
const off = d.indexOf("255\n") + 4;
const raw = Buffer.alloc((w * 3 + 1) * h);
for (let y = 0; y < h; y++) d.copy(raw, y * (w * 3 + 1) + 1, off + y * w * 3, off + (y + 1) * w * 3);
const crc = (b) => { let r = 0xffffffff; for (const v of b) { r ^= v; for (let k = 0; k < 8; k++) r = r & 1 ? 0xedb88320 ^ (r >>> 1) : r >>> 1; } return (r ^ 0xffffffff) >>> 0; };
const chunk = (t, data) => { const b = Buffer.concat([Buffer.from(t), data]); const l = Buffer.alloc(4); l.writeUInt32BE(data.length); const c = Buffer.alloc(4); c.writeUInt32BE(crc(b)); return Buffer.concat([l, b, c]); };
const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
writeFileSync(process.argv[3], Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]));
