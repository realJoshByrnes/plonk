// Dev server: node web/serve.mjs [port]  -> http://localhost:8080/web/index.html
import { createServer } from "node:http";
import { readFileSync, readdirSync, existsSync, statSync } from "node:fs";
import { extname, join, normalize } from "node:path";
const root = new URL("../", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const types = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".wasm": "application/wasm", ".json": "application/json" };
createServer((req, res) => {
  const url = decodeURIComponent(req.url.split("?")[0]);
  if (url === "/manifest.json") {
    const dlls = readdirSync(join(root, "wine/out")).filter((f) => f.endsWith(".dll"));
    const nls = readdirSync(join(root, "wine/out/nls"));
    res.setHeader("content-type", "application/json");
    return res.end(JSON.stringify({ dlls, nls, apps: readdirSync(join(root, "tests/guests")).filter((f) => f.endsWith(".exe")) }));
  }
  const p = normalize(join(root, url));
  if (!p.startsWith(normalize(root)) || !existsSync(p) || statSync(p).isDirectory()) { res.statusCode = 404; return res.end("not found"); }
  res.setHeader("content-type", types[extname(p)] ?? "application/octet-stream");
  res.end(readFileSync(p));
}).listen(Number(process.argv[2] ?? 8080), () => console.log("plonk dev server on http://localhost:" + (process.argv[2] ?? 8080) + "/web/index.html"));
