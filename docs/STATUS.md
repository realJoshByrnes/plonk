# Plonk status
**Milestone:** M4 done (headless-verified): a GUI exe creates a window, paints, handles WM_CLOSE through Wine 9.0's real PE user32/gdi32; our native win32u draws to per-window surfaces.

## Works
- Crates: mem, cpu (i386 integer + x87, FS base), pe, display (Backend trait, SoftBackend, compose), host (WineProc + Process), testkit.
- WineProc (crates/host/src/wine.rs): Wine's PE ntdll/kernelbase/kernel32/user32/gdi32/win32u/advapi32/... run in the interpreter; syscalls via TEB dispatcher thunk.
- Native win32u (crates/host/src/wine/win32u.rs): classes, windows (one display surface per top-level window), message queue + synthesized WM_PAINT,
  DefWindowProc, BeginPaint/EndPaint, GDI handle table + DC_ATTR in guest memory (gdi32 reads them directly), FillRect/PatBlt/placeholder text.
  Guest window procs are called via call_guest (re-entrant); wineserver requests: post_quit_message only.
- Tests: `bash scripts/test.sh` = 61 snippets, Wine hello + GUI native tests (skip if wine/out missing), wasm selftest/hello/GUI in Node.
- Browser: `node web/serve.mjs` then open /web/index.html (wasm, canvas 2D, click the red X). Verified headless in Node; the DOM page itself is not browser-tested.
- Build Wine DLLs: wine/build.sh in WSL, copy into wine/out (+ nls/ from wine-src/nls). Generators: wine/gen_*.sh.

## Gaps
- Text is a fixed 8x8 bitmap font (font8x8; no GDI font engine/metrics); only solid brushes, PatBlt/FillRect; no pens/lines/bitmaps/regions/clipping beyond the paint rect.
- Shell: drag by title bar, click-to-raise, active/inactive caption, close box (no resize/min/max, no taskbar). Child windows + built-in classes (real Wine BUTTON works: clicks -> WM_COMMAND), window longs, capture/focus, pens/lines/Rectangle, fixed-metrics text. Keyboard, menus, timers, edit controls (untested), multiple threads: not implemented.
- Registry/VFS minimal; most wineserver requests unimplemented; unimplemented syscalls abort the run (LENIENT=1 in run_wine stubs NtUser/NtGdi).
- SSE not implemented (build guests with -mno-sse); x87 uses f64 precision. Guest memory capped at 0x7ffe1000 (wasm32 Vec limit).
- CRT-linked (msvcrt) apps untested.

## Design
See docs/DESIGN.md (compatibility strategy, Plonk Flat visual spec, layer ownership, open decisions); preview at /web/design.html.

## Next
Pens/lines/bitmaps, child windows/controls, keyboard, timers, a shell (move/stack windows), WebGL compositor.
