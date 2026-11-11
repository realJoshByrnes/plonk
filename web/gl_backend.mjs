// Plonk GL backend entry point. See docs/GL_PROTOCOL.md.
//   const gl = await createGLBackend({ prefer: "webgpu" | "webgl2" });
//   gl.kind                       "webgpu" or "webgl2"
//   gl.execute(Uint32Array)       replay a command stream
//   gl.onPresent = (w,h,rgba)=>{} called for every PRESENT, rgba = Uint8Array(w*h*4), top row first.
//                                 Synchronous (inside execute) for webgl2; ASYNCHRONOUS (after a mapAsync) for webgpu, in order.
// WebGPU is tried first when prefer === "webgpu" (default); any failure (no navigator.gpu, no adapter/device, shader or pipeline
// validation error, module load failure) silently falls back to WebGL2.
import { createCore } from "./gl_core.mjs";

export async function createGLBackend({ prefer = "webgpu" } = {}) {
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
  const api = { kind: renderer.kind, onPresent: null, execute: null };
  const core = createCore(renderer, (w, h, rgba) => { if (api.onPresent) api.onPresent(w, h, rgba); });
  api.execute = (words) => core.execute(words instanceof Uint32Array ? words : new Uint32Array(words));
  return api;
}
