# Plonk status
Browser desktop that runs 32-bit Windows programs: Rust i386 interpreter (wasm) + Wine 9.0 PE DLLs interpreted + our native win32u/NtGdi/kernel layer.

## Works (all covered by `bash scripts/test.sh` + `node web/e2e.mjs` in real Chrome)
- CPU: i386 integer, x87 (f64 precision), SSE/SSE2, cmpxchg8b, exceptions (SEH via Wine's KiUserExceptionDispatcher), NULL-page guard.
- Programs: console (mingw + Rust i686-msvc/gnu hello), CRT apps, GUI apps: windows, child windows, dialogs (modal), menus, real Wine controls
  (button/edit/combo/list/listview/treeview/tab/progress/trackbar/updown/statusbar/date/ip/hotkey), clipboard, timers, keyboard, mouse.
- Threads: cooperative scheduler (CreateThread, events, CS, waits with timeouts, suspend/resume, TLS).
- Kernel: writable in-memory FS + registry + environment, tokens, timers, handles, volumes, API-set map, KUSER clock.
- Display: glass compositor (blur/shadow/rounded), glass window chrome, Inter font via fontdue, bitmap/DIB engine, shapes, regions.
- Browser: single gzip core pack + lazy DLL/NLS download, dependency prefetch (`/deps.json`), own-exe picker/drop, fresh process per Run,
  banner lists unimplemented NtUser/NtGdi calls (answered 0 so apps keep going).

## Known gaps
- Roughly half of the ~400 NtUser/NtGdi calls have no real handler (stubs return 0); menus lack submenus/keyboard nav; no drag-and-drop, hooks, printing.
- One shared message queue across threads; mutex ownership not tracked; no real processes (CreateProcess), no sockets/audio/D3D.
- Controls are classic GDI-drawn (not themed); Plonk Glass theme (uxtheme/msstyles) is only designed (docs/DESIGN.md), not wired: comctl32 v6 side-by-side works but theming is inactive.
- x87 is f64; no MMX registers; no 16-bit (NE) or 64-bit programs.
- Single font family (Inter + SemiBold); no font enumeration/mono.

## Workflow
- Build Wine DLLs: `wine/build.sh` in WSL Debian -> copy into wine/out (git-ignored; tests skip if absent).
- Debug: `examples/drive.exe <exe> [secs]` (LAZY=1 mimics browser, WATCH=1 reports spins), `examples/shot.exe <exe> out.ppm "d,x,y;u,x,y;k,vk,ch"`
  then `node web/ppm2png.mjs`; `web/perf.mjs`, `web/lazy_check.mjs`, `web/e2e.mjs [shotdir]` (needs `node web/serve.mjs 8080`).
- Gotcha: heredocs halve backslashes; write Rust/JS with `c:\` strings via the Write/Edit tools.

## Next
1. Run bigger real programs (open-source builds) and fix what breaks; add handlers for the remaining NtUser/NtGdi calls by demand.
2. Wire the Plonk Glass control theme (uxtheme + generated msstyles), then glass popups for menus/tooltips everywhere.
3. Per-thread message queues; menu submenus/keyboard navigation; GPU (WebGL) compositor.
