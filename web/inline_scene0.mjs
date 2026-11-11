// Inlines scene 0 (the pre-splash frame of the boot intro) into web/index.html so it paints on first byte: node web/inline_scene0.mjs
import { readFileSync, writeFileSync } from "node:fs";
import { scene0SVG } from "./wine_scene.svg.mjs";
const f = new URL("./index.html", import.meta.url);
let s = readFileSync(f, "utf8");
const crlf = s.includes("\r\n");
s = s.replace(/\r\n/g, "\n");
const svg = scene0SVG({ id: "plonkScene", style: "position:absolute;left:12px;top:0;width:min(1024px,calc(100% - 24px));aspect-ratio:4/3;height:auto;display:block;pointer-events:none" });
const out = s.replace(/<!--scene0-->[\s\S]*?<!--\/scene0-->/, "<!--scene0-->" + svg + "<!--/scene0-->");
if (out === s && !s.includes(svg)) throw new Error("markers not found");
writeFileSync(f, crlf ? out.replace(/\n/g, "\r\n") : out);
console.log("inlined scene 0:", (svg.length / 1024).toFixed(1), "KB");
