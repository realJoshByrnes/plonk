// Default desktop wallpaper: a glass of red wine in golden-hour light, drawn from one vector scene (web/wine_scene.svg.mjs).
// Resolution independent: rasterize(w, h) renders the SVG at any size/aspect ratio (landscape or portrait); the glass stays in view
// (preserveAspectRatio "xMaxYMid slice"). Needs only a 2D canvas, no WebGL.
import { sceneSVG } from "./wine_scene.svg.mjs";

function svgToCanvas(svgText, w, h) {
  return new Promise((res, rej) => {
    const url = URL.createObjectURL(new Blob([svgText], { type: "image/svg+xml" }));
    const img = new Image();
    img.onload = () => {
      const c = document.createElement("canvas"); c.width = w; c.height = h;
      c.getContext("2d").drawImage(img, 0, 0, w, h);
      URL.revokeObjectURL(url); res(c);
    };
    img.onerror = () => { URL.revokeObjectURL(url); rej(new Error("svg raster failed")); };
    img.src = url;
  });
}

export async function rasterize(w, h) {
  const c = await svgToCanvas(sceneSVG({ w, h }), w, h);
  return typeof createImageBitmap === "function" ? createImageBitmap(c) : c;
}
