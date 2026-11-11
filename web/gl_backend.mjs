// Plonk GL backend entry point. See docs/GL_PROTOCOL.md.
//   const gl = await createGLBackend({ prefer: "webgpu" | "webgl2" });
//   gl.kind                       "webgpu" or "webgl2"
//   gl.execute(Uint32Array)       replay a command stream
//   gl.onPresent = (w,h,rgba)=>{} called for every PRESENT, rgba = Uint8Array(w*h*4), top row first.
//                                 Synchronous (inside execute) for webgl2; ASYNCHRONOUS (after a mapAsync) for webgpu, in order.
// Direct (zero-readback) mode: createGLBackend({direct:true}) or set the mutable `gl.direct = true`. When direct is true and
//   gl.onFrame is set, PRESENT does not read pixels; it calls gl.onFrame(w, h, imageBitmap) synchronously (both backends), the
//   bitmap being upright (top row first) and equal in content to the onPresent readback. If the renderer cannot produce a
//   bitmap, or direct is false / onFrame unset, the normal readback + onPresent path is used.
// WebGPU is tried first when prefer === "webgpu" (default); any failure (no navigator.gpu, no adapter/device, shader or pipeline
// validation error, module load failure) silently falls back to WebGL2.
import { createCore } from "./gl_core.mjs";

export async function createGLBackend({ prefer = "webgpu", direct = false } = {}) {
  let renderer = null;
  if (prefer === "webgpu") {
    try {
      const m = await import("./gl_webgpu.mjs");
      renderer = await m.createWebGPURenderer();
    } catch (e) { renderer = null; console.info("GL backend: WebGPU unavailable, using WebGL2 (" + (e?.message ?? e) + ")"); }
  }
  if (!renderer) {
    const m = await import("./gl_webgl2.mjs");
    renderer = m.createWebGL2Renderer();
  }
  const api = { kind: renderer.kind, direct: !!direct, onPresent: null, onFrame: null, execute: null };
  const core = createCore(renderer, (w, h, rgba) => { if (api.onPresent) api.onPresent(w, h, rgba); },
    { on: () => api.direct && !!api.onFrame, cb: (w, h, bmp) => api.onFrame(w, h, bmp) });
  api.execute = (words) => core.execute(words instanceof Uint32Array ? words : new Uint32Array(words));
  return api;
}
