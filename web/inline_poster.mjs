// Inlines web/poster.jpg into web/index.html between <!--poster--> and <!--/poster--> as <img id="poster">.
// If the markers are absent, prints the snippet instead.   node web/inline_poster.mjs
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const WEB = dirname(fileURLToPath(import.meta.url));
const jpg = join(WEB, "poster.jpg"), html = join(WEB, "index.html");
if (!existsSync(jpg)) { console.log("no web/poster.jpg (run make_wallpaper.mjs --final)"); process.exit(1); }
const b64 = readFileSync(jpg).toString("base64");
const img = `<img id="poster" alt="" src="data:image/jpeg;base64,${b64}" style="position:absolute;left:0;top:0;width:100%;height:100%;object-fit:cover;object-position:right center;pointer-events:none">`;
let src = readFileSync(html, "utf8");
const a = src.indexOf("<!--poster-->"), b = src.indexOf("<!--/poster-->");
if (a < 0 || b < a) { console.log("markers <!--poster--> ... <!--/poster--> not found in index.html; snippet (" + img.length + " chars):\n" + img.slice(0, 160) + "..."); process.exit(0); }
src = src.slice(0, a) + "<!--poster-->" + img + src.slice(b);
writeFileSync(html, src);
console.log("poster inlined:", img.length, "chars");
