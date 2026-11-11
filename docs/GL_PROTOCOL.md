# Plonk 3D command stream (guest OpenGL -> browser GPU)

The host (Rust/wasm) translates every guest OpenGL 1.x call into a small set of **canonical commands** and appends them to a
byte buffer. The page drains that buffer and hands it to the JS backend (`web/gl_backend.mjs`), which replays it on WebGPU
(preferred) or WebGL2 (fallback). The backend implements OpenGL 1.x **fixed-function** semantics itself (matrix stacks, lighting,
fog, texturing, immediate mode, display lists) with shaders.

## Encoding
- A stream is a sequence of messages. Everything is little-endian 32-bit words (`Uint32Array`); floats are IEEE-754 bit patterns
  (`Float32Array` view over the same buffer), booleans are 0/1, GL enums are the raw GL values.
- Message = `[op, len, payload...]` where `len` is the number of payload words. Unknown ops must be skipped using `len`.
- "RGBA8 blob" = `w*h*4` bytes of tightly packed RGBA, top row first, padded with zero bytes to a multiple of 4 (so it is a whole
  number of words). `len` includes those words.

## Commands (op: payload)
| op | name | payload |
|---|---|---|
| 1 | CREATE | `width, height` (u32) - create (or recreate) the drawing surface, all state reset to GL defaults |
| 2 | RESIZE | `width, height` - surface size changed (keep resources) |
| 3 | CLEAR | `mask` (GL bits: color 0x4000, depth 0x100, stencil 0x400) |
| 4 | CLEAR_COLOR | `r,g,b,a` f32 |
| 5 | CLEAR_DEPTH | `d` f32 |
| 6 | ENABLE | `cap` |
| 7 | DISABLE | `cap` (caps: DEPTH_TEST 0xB71, CULL_FACE 0xB44, BLEND 0xBE2, LIGHTING 0xB50, LIGHT0..7 0x4000+i, TEXTURE_2D 0xDE1, FOG 0xB60, ALPHA_TEST 0xBC0, NORMALIZE 0xBA1, COLOR_MATERIAL 0xB57, SCISSOR_TEST 0xC11, POLYGON_OFFSET_FILL 0x8037; others ignored) |
| 8 | DEPTH_FUNC | `func` |
| 9 | DEPTH_MASK | `flag` |
| 10 | CULL_FACE | `mode` |
| 11 | FRONT_FACE | `mode` |
| 12 | BLEND_FUNC | `src, dst` |
| 13 | ALPHA_FUNC | `func, ref` (ref f32) |
| 14 | SHADE_MODEL | `mode` (GL_FLAT 0x1D00, GL_SMOOTH 0x1D01) |
| 15 | POLYGON_MODE | `face, mode` (GL_POINT 0x1B00, GL_LINE 0x1B01, GL_FILL 0x1B02) |
| 16 | LINE_WIDTH | `w` f32 |
| 17 | POINT_SIZE | `s` f32 |
| 18 | VIEWPORT | `x,y,w,h` (i32) |
| 19 | SCISSOR | `x,y,w,h` (i32) |
| 20 | COLOR_MASK | `r,g,b,a` |
| 21 | MATRIX_MODE | `mode` (GL_MODELVIEW 0x1700, GL_PROJECTION 0x1701, GL_TEXTURE 0x1702) |
| 22 | LOAD_IDENTITY | - |
| 23 | LOAD_MATRIX | 16 f32, column-major (GL order) |
| 24 | MULT_MATRIX | 16 f32, column-major; current = current * m |
| 25 | PUSH_MATRIX | - |
| 26 | POP_MATRIX | - |
| 27 | BEGIN | `mode` (GL_POINTS 0, LINES 1, LINE_LOOP 2, LINE_STRIP 3, TRIANGLES 4, TRIANGLE_STRIP 5, TRIANGLE_FAN 6, QUADS 7, QUAD_STRIP 8, POLYGON 9) |
| 28 | END | - |
| 29 | VERTEX | `x,y,z,w` f32 (uses the *current* color/normal/texcoord) |
| 30 | COLOR | `r,g,b,a` f32 (current color) |
| 31 | NORMAL | `x,y,z` f32 (current normal) |
| 32 | TEXCOORD | `s,t,r,q` f32 (current texcoord, unit 0) |
| 33 | DRAW | `mode, count, flags` then vertex data (see below). Equivalent to BEGIN/VERTEX*/END with per-vertex attributes; does not change the current attributes |
| 34 | LIGHT | `light(0..7), pname, 4 x f32` (pnames: AMBIENT 0x1200, DIFFUSE 0x1201, SPECULAR 0x1202, POSITION 0x1203, SPOT_DIRECTION 0x1204, SPOT_EXPONENT 0x1205, SPOT_CUTOFF 0x1206, CONSTANT_ATTENUATION 0x1207, LINEAR 0x1208, QUADRATIC 0x1209; scalar pnames use the first float). POSITION/SPOT_DIRECTION are transformed by the modelview matrix current *at the time of the command* |
| 35 | LIGHT_MODEL | `pname, 4 x f32` (LIGHT_MODEL_AMBIENT 0xB53, LIGHT_MODEL_TWO_SIDE 0xB52, LIGHT_MODEL_LOCAL_VIEWER 0xB51) |
| 36 | MATERIAL | `face, pname, 4 x f32` (AMBIENT 0x1200, DIFFUSE 0x1201, SPECULAR 0x1202, EMISSION 0x1600, SHININESS 0x1601 (first float), AMBIENT_AND_DIFFUSE 0x1602) |
| 37 | COLOR_MATERIAL | `face, mode` |
| 38 | FOG | `pname, 4 x f32` (FOG_MODE 0xB65 value is a GL enum as f32: LINEAR 0x2601, EXP 0x800, EXP2 0x801; FOG_DENSITY 0xB62, FOG_START 0xB63, FOG_END 0xB64, FOG_COLOR 0xB66) |
| 39 | TEX_IMAGE | `id, w, h` + RGBA8 blob - (re)define level 0 of texture `id` (host converted every GL format to RGBA8) |
| 40 | TEX_SUBIMAGE | `id, x, y, w, h` + RGBA8 blob |
| 41 | BIND_TEXTURE | `id` (0 = no texture bound; ids are chosen by the guest/host and arbitrary) |
| 42 | DELETE_TEXTURE | `id` |
| 43 | TEX_PARAM | `pname, value` (value is a GL enum/number as f32; MIN_FILTER 0x2801, MAG_FILTER 0x2800, WRAP_S 0x2802, WRAP_T 0x2803; NEAREST 0x2600, LINEAR 0x2601, *_MIPMAP_* 0x2700..0x2703 (a backend may approximate), REPEAT 0x2901, CLAMP 0x2900, CLAMP_TO_EDGE 0x812F) applies to the bound texture |
| 44 | TEX_ENV | `pname, value` (TEXTURE_ENV_MODE 0x2200: MODULATE 0x2100, REPLACE 0x1E01, DECAL 0x2101, ADD 0x104) |
| 45 | NEW_LIST | `id, mode` (mode GL_COMPILE 0x1300 / GL_COMPILE_AND_EXECUTE 0x1301) - start recording subsequent commands into list `id` |
| 46 | END_LIST | - |
| 47 | CALL_LIST | `id` - replay (lists may call other lists; limit nesting to 64) |
| 48 | DELETE_LISTS | `first, range` |
| 49 | PRESENT | `width, height` - the frame is finished: flush all work and deliver the framebuffer (see below). Start of the next frame keeps the state |
| 50 | PUSH_ATTRIB | `mask` - save the commonly used state (enables, depth/blend/cull, color, lighting enables, viewport, current color) |
| 51 | POP_ATTRIB | - |

### DRAW vertex data
After `mode, count, flags` the payload holds, in this order, tightly packed f32 arrays:
1. positions: `count x 4` (x,y,z,w)
2. if `flags & 1`: normals `count x 3`
3. if `flags & 2`: colors `count x 4`
4. if `flags & 4`: texcoords `count x 2`
Absent attributes use the current value (COLOR/NORMAL/TEXCOORD state). Indices were already applied by the host.

## Backend API (`web/gl_backend.mjs`)
```js
import { createGLBackend } from "./gl_backend.mjs";
const gl = await createGLBackend({ prefer: "webgpu" | "webgl2" });   // falls back automatically; gl.kind is "webgpu" or "webgl2"
gl.execute(uint32Array);          // replay a stream (may contain many messages, even partial frames); synchronous for webgl2
gl.onPresent = (w, h, rgba) => {}; // called for every PRESENT with a Uint8Array(w*h*4), top row first
```
`execute` must tolerate any message order that real OpenGL allows (state commands between frames, display lists, etc.). The
backend owns an offscreen canvas of the CREATE/RESIZE size and reads the frame back for `onPresent` (the Plonk compositor blits it
into the guest window). Depth buffer is required; antialiasing is optional.

## Fixed-function semantics to emulate
- Matrix stacks (modelview, projection, texture), column-major, GL rules (modelview = view*model, normal matrix = inverse transpose).
- Clip space matches OpenGL (z in [-1,1]); WebGPU backends must remap z to [0,1] in the shader.
- Immediate mode: batch vertices between BEGIN/END, convert QUADS/QUAD_STRIP/POLYGON/TRIANGLE_FAN/LINE_LOOP to triangles/lines.
- Lighting: up to 8 lights (directional or point, spot, attenuation), global ambient, material with COLOR_MATERIAL, two-sided optional,
  per-vertex (Gouraud) lighting is fine; GL_FLAT shading optional.
- Texturing: 2D, MODULATE/REPLACE/DECAL/ADD, wrap/filter parameters, (mip)maps via generateMipmap when a mipmap filter is requested.
- Fog (linear/exp/exp2), alpha test, blending, depth test/mask/func, culling, polygon mode (fill/line/point), scissor, viewport, color mask.
- Display lists record *commands* (not results) and replay them.
