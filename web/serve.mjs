// Dev server: node web/serve.mjs [port]  -> http://localhost:8080/web/index.html
import { createServer } from "node:http";
import { createReadStream, readFileSync, writeFileSync, unlinkSync, readdirSync, existsSync, statSync } from "node:fs";
import { extname, join, normalize } from "node:path";
import { gzipSync } from "node:zlib";
import { createHash } from "node:crypto";
import { peImports } from "./pe_imports.mjs";
const root = new URL("../", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const types = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".wasm": "application/wasm", ".json": "application/json", ".exe": "application/octet-stream" };
// Always-needed set shipped in the first download; everything else is fetched on demand via /file/<name>.
const CORE = ["apisetschema.dll", "ntdll.dll", "kernelbase.dll", "kernel32.dll", "win32u.dll", "user32.dll", "gdi32.dll", "msvcrt.dll", "ucrtbase.dll", "advapi32.dll", "sechost.dll", "rpcrt4.dll", "imm32.dll"];
let pack = null;
function buildPack() {
  const out = readdirSync(join(root, "wine/out")).filter((f) => CORE.includes(f)).map((f) => [`c:\\windows\\system32\\${f}`, join(root, "wine/out", f)])
    .concat(["locale.nls", "l_intl.nls", "sortdefault.nls", "normnfc.nls", "normnfd.nls", "normnfkc.nls", "normnfkd.nls", "normidna.nls", "c_1252.nls", "c_437.nls", "c_20127.nls"].map((f) => [`nls\\${f}`, join(root, "wine/out/nls", f)]));
  const t0 = performance.now();
  // cache key from file names/sizes/mtimes only: an unchanged DLL set reuses the gzip stored on disk (no re-compress on server start)
  const key = out.map(([n, p]) => `${n}:${statSync(p).size}:${statSync(p).mtimeMs}`).join("|");
  const etag = `"${createHash("sha1").update(key).digest("hex").slice(0, 16)}"`;
  const cacheFile = join(root, "wine/out/.pack-" + etag.slice(1, -1) + ".gz");
  if (existsSync(cacheFile)) {
    const gz = readFileSync(cacheFile);
    console.log(`pack: ${out.length} files from disk cache, ${(gz.length / 1048576).toFixed(1)} MB gzip, ${Math.round(performance.now() - t0)} ms`);
    return { gz, etag };
  }
  const parts = [];
  for (const [name, path] of out) {
    const n = Buffer.from(name), d = readFileSync(path), h = Buffer.alloc(4);
    h.writeUInt32LE(n.length); parts.push(h);
    parts.push(n);
    const l = Buffer.alloc(4); l.writeUInt32LE(d.length); parts.push(l, d);
  }
  const raw = Buffer.concat(parts), gz = gzipSync(raw, { level: 9 });
  try { for (const f of readdirSync(join(root, "wine/out"))) if (f.startsWith(".pack-")) unlinkSync(join(root, "wine/out", f)); writeFileSync(cacheFile, gz); } catch {}
  console.log(`pack: ${out.length} files, ${(raw.length / 1048576).toFixed(1)} MB -> ${(gz.length / 1048576).toFixed(1)} MB gzip, built in ${Math.round(performance.now() - t0)} ms`);
  return { gz, etag };
}
pack = buildPack(); // ready before the first request
createServer((req, res) => {
  const url = decodeURIComponent(req.url.split("?")[0]);
  if (url === "/pack") {
    if (req.headers["if-none-match"] === pack.etag) { res.statusCode = 304; return res.end(); }
    res.writeHead(200, { "content-type": "application/octet-stream", "content-encoding": "gzip", etag: pack.etag, "cache-control": "no-cache", "content-length": pack.gz.length });
    return res.end(pack.gz);
  }
  if (url === "/deps.json") { // dll -> DLLs it imports (incl. delay-load), for prefetching
    const out = join(root, "wine/out"), deps = {};
    const all = readdirSync(out).filter((f) => f.endsWith(".dll"));
    for (const f of all) {
      const data = readFileSync(join(out, f));
      const d = new Set(peImports(new Uint8Array(data)));
      if (!CORE.includes(f)) { // run-time LoadLibrary targets: another DLL's name appearing as a string literal
        for (const o of all) if (o !== f && !CORE.includes(o) && (data.indexOf(o, 0, "latin1") >= 0 || data.indexOf(Buffer.from(o, "utf16le")) >= 0)) d.add(o);
      }
      deps[f] = [...d];
    }
    res.setHeader("content-type", "application/json");
    return res.end(JSON.stringify(deps));
  }
  if (url === "/manifest.json") {
    res.setHeader("content-type", "application/json");
    const out = join(root, "wine/out");
    return res.end(JSON.stringify({
      core: CORE,
      apps: readdirSync(join(root, "tests/guests")).filter((f) => f.endsWith(".exe")),
      remote: readdirSync(out).filter((f) => /\.(dll|msstyles)$/.test(f) && !CORE.includes(f)).concat(readdirSync(join(out, "nls"))),
    }));
  }
  if (url.startsWith("/file/")) {
    const n = url.slice(6).replace(/[\\/]/g, "");
    const p = [join(root, "wine/out", n), join(root, "wine/out/nls", n)].find((x) => existsSync(x));
    if (!p) { res.statusCode = 404; return res.end("no such file"); }
    res.writeHead(200, { "content-type": "application/octet-stream", "cache-control": "public, max-age=3600" });
    return createReadStream(p).pipe(res);
  }
  const p = normalize(join(root, url));
  if (!p.startsWith(normalize(root)) || !existsSync(p) || statSync(p).isDirectory()) { res.statusCode = 404; return res.end("not found"); }
  const st = statSync(p), etag = `"${st.size}-${st.mtimeMs}"`;
  if (req.headers["if-none-match"] === etag) { res.statusCode = 304; return res.end(); }
  res.writeHead(200, { "content-type": types[extname(p)] ?? "application/octet-stream", etag, "cache-control": "no-cache", "content-length": st.size });
  createReadStream(p).pipe(res);
}).listen(Number(process.argv[2] ?? 8080), () => console.log("plonk dev server on http://localhost:" + (process.argv[2] ?? 8080) + "/web/index.html"));
