# Plonk status
**Milestone:** M4 done; Plonk Glass visual direction chosen (docs/DESIGN.md); stage G0 (software glass compositor + glass chrome) done.

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
- Text: bundled Inter (fontdue) behind GDI font calls: HfontCreate, text metrics, extents, ExtTextOut with alignment/opaque bg/clipping, GetObject(font); DC window/viewport mapping honoured. One family (+SemiBold for bold); no mono, no hinting, no font enumeration; only solid brushes, PatBlt/FillRect.
- Shell: drag by title bar, click-to-raise, active/inactive caption, close box (no resize/min/max, no taskbar). Child windows + built-in classes (real Wine BUTTON works: clicks -> WM_COMMAND), window longs, capture/focus, pens/lines/Rectangle, fixed-metrics text, GDI bitmap engine (memory DCs, DDB/DIB sections in guest memory, BitBlt/StretchBlt/AlphaBlend with ROPs, DIB transfer, GetObject). Keyboard input + real Wine EDIT control (typing, backspace, focus messages), timers (shell tick), caret stubs. Menus: menu bars + one-level glass popups + TrackPopupMenu (no submenus, keyboard nav, check marks, accelerators). Clipboard, multiple threads: not implemented.
- Registry/VFS minimal; most wineserver requests unimplemented; unimplemented syscalls abort the run (LENIENT=1 in run_wine stubs NtUser/NtGdi).
- SSE not implemented (build guests with -mno-sse); x87 uses f64 precision. Guest memory capped at 0x7ffe1000 (wasm32 Vec limit).
- CRT-linked mingw exes work (crt_hello). Real-app risks: SSE/SSE2 code (not implemented; CPUID hides it), missing NtUser/NtGdi calls (listed in the page banner), single thread, no sound/network.

## Design
See docs/DESIGN.md (compatibility strategy, Plonk Flat visual spec, layer ownership, open decisions); preview at /web/design.html.

## G4 theme spike (findings)
- Built uxtheme, comctl32, shlwapi, version and Wine's stock `light.msstyles` (7.5MB resource DLL) in WSL; all load in the interpreter.
- `tests/guests/gui_button_v6.exe` (comctl32 v6 manifest + InitCommonControls) runs to idle after adding atoms, GetSystemMetrics table, cursor/icon stubs.
- Theming still inactive: uxtheme is never loaded because comctl32 resolves to the v5 build. Needs side-by-side (winsxs) resolution of Microsoft.Windows.Common-Controls 6.0 via ntdll activation contexts (VFS layout + manifest), then registry ThemeManager seeding (ThemeActive, DllName=...\light.msstyles, ColorName, SizeName), then DrawThemeBackground paths (AlphaBlend/StretchBlt exist).
- After that: replace light.msstyles bitmaps with the Plonk Glass set (generator tool) - see DESIGN section 3.6.

## Next (priority order, hand-off)
1. G4 Plonk control theme: author .msstyles (windres + generator), have Wine uxtheme load it; needs comctl32/uxtheme DLLs in wine/build.sh and DrawThemeBackground-related NtGdi calls.
2. (keyboard, Edit, timers, basic menus done) Submenus, menu keyboard nav, check marks, clipboard, caret rendering.
3. Compatibility meter: enable Wine tests in wine/build.sh, run user32/gdi32 test exes, publish pass counts.
4. GPU (WebGL) compositor for blur; resize/min/max; taskbar/shell.
Workflow tips: `LENIENT=1 N=40 ./target/release/examples/run_wine.exe <exe>` shows last syscalls + unimplemented list; `examples/shot.exe <exe> out.ppm "d,x,y;u,x,y"` renders the desktop (convert with web/ppm2png.mjs).
