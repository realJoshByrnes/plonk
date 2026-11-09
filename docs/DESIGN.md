# Plonk design

Status: proposal for review. Sections 1-2 say how we keep compatibility from collapsing; sections 3-5 define how windows and controls look and which layer draws what; section 6 lists decisions needed. A live visual preview of section 3 is in `web/design.html` (served by `node web/serve.mjs`).

## 1. Principles

1. **Wine does the Windows semantics, we do the platform.** Wine's PE user32/gdi32/comctl32/etc. run unmodified. We own everything below them: syscalls, windows, input, drawing, fonts, the shell.
2. **Compatibility is measured, not guessed.** Every claim ("Notepad works") is backed by a test that runs in CI.
3. **Look is data, not code.** Colors, metrics and fonts live in one theme file; changing the look must not touch syscall handlers.
4. **Windows are surfaces.** Each top-level window is its own surface so the shell can move, stack, animate and composite them (already true).
5. **Fail loudly in dev, softly for users.** Unimplemented calls abort in tests and are listed (not fatal) in the browser.

## 2. Compatibility strategy

### 2.1 What "all calls" really means
Wine's `win32u` is not run by us, so its ~800 NtUser/NtGdi calls (plus the wineserver requests user32 issues directly) are ours to provide. We have 58 of 796. Most apps touch 100-150; the rest is a long tail concentrated in a few expensive families (fonts, bitmaps/DIBs, regions/paths, menus, keyboard/IME, clipboard/OLE drag-drop, hooks, multi-monitor/display settings).

### 2.2 Tiers (what we promise at each level)
| Tier | Target | Needs beyond today |
|---|---|---|
| T0 | Console apps | done (kernel32/ntdll path) |
| T1 | Simple GUI: windows, buttons, static text, custom drawing | done (needs fonts, bitmaps for polish) |
| T2 | Typical desktop apps: Notepad, Calculator, Paint-class | keyboard + Edit, menus, timers, real fonts + metrics, DIB/bitmap blits, clipboard, common dialogs (comdlg32), comctl32 (toolbar, status bar, listview, tabs) |
| T3 | Shell-integrated apps | shell32/ole32/oleaut32, registry fidelity, file system view, COM |
| T4 | Graphics/audio/network heavy | d3d/ddraw -> WebGL, winmm/dsound -> WebAudio, ws2_32 -> fetch/WebSocket |

Each tier closes only when its exit tests pass.

### 2.3 How we measure
- **Wine's own conformance tests** (`dlls/user32/tests`, `dlls/gdi32/tests`, `dlls/comctl32/tests`, ...) are PE programs built from the same pinned tree. Running them gives pass/fail counts per DLL: a real, large, independent compatibility meter. First action: enable tests in `wine/build.sh`, run user32_test with a result parser, publish counts in STATUS.md.
- **Real app list**, kept in `tests/apps/` with expected pixel/exit assertions (Notepad-class apps built against Wine, then freeware).
- **Syscall coverage report**: calls handled / total, plus a ranked list of calls hit-but-unimplemented from real runs (the browser banner already collects this).

### 2.4 How we implement the long tail cheaply
1. **Generate, then specialize.** Generated typed stubs for all 796 calls from Wine's headers (names, argument counts, sizes) so a missing call is a typed TODO with correct stack cleanup, not a crash.
2. **Group by family** and build shared engines once: a font engine, a bitmap/DIB engine, a region engine, a menu engine. Hundreds of calls collapse onto four engines.
3. **Borrow Wine's code where it is pure computation.** Wine's `dibdrv` (software GDI raster), region math and font mapping are portable C. Spike (time-boxed): compile selected win32u source files to wasm32 as a library called from our handlers. Go/no-go after the spike; this is the "reuse more of Wine" option without taking the whole Unix layer and wineserver.
4. Keep demand-driven order within each family (real apps decide).

## 3. Visual design

Name: **Plonk Flat** (default), with a **Plonk Dark** variant. Goals: crisp, flat, high-contrast, legible at 1x and 2x; looks deliberate, not like emulation.

### 3.1 Palette (system colors, COLOR_* indices)
Light / Dark. Wine's controls draw from these, so changing them restyles every control.

| Role | Light | Dark |
|---|---|---|
| Desktop (COLOR_BACKGROUND) | #3A6EA5 | #1B2433 |
| Window / editable (WINDOW) | #FFFFFF | #1F2329 |
| Window text (WINDOWTEXT) | #1A1A1A | #E6E8EB |
| Button face / dialog bg (BTNFACE, 3DFACE) | #F0F0F0 | #2B3038 |
| Button text (BTNTEXT) | #1A1A1A | #E6E8EB |
| Button highlight (BTNHILIGHT) | #FFFFFF | #3A4048 |
| 3D light (3DLIGHT) | #E3E3E3 | #323840 |
| Button shadow (BTNSHADOW) | #A0A0A0 | #1A1D22 |
| 3D dark shadow (3DDKSHADOW) | #696969 | #0F1114 |
| Window frame (WINDOWFRAME) | #646464 | #0F1114 |
| Accent / selection (HIGHLIGHT) | #0A64C8 | #3B8EEA |
| Selection text (HIGHLIGHTTEXT) | #FFFFFF | #FFFFFF |
| Active caption (ACTIVECAPTION) | #0A64C8 | #2C3A52 |
| Inactive caption (INACTIVECAPTION) | #DADDE1 | #23272E |
| Caption text (CAPTIONTEXT) | #FFFFFF | #FFFFFF |
| Inactive caption text | #5C6168 | #8B9199 |
| Disabled text (GRAYTEXT) | #8A8F96 | #6B7078 |
| Menu / menu text | #F7F7F7 / #1A1A1A | #262B32 / #E6E8EB |
| Tooltip bg / text (INFOBK/INFOTEXT) | #FFFFE1 / #1A1A1A | #33383F / #E6E8EB |

### 3.2 Metrics (GetSystemMetrics, at 96 dpi; scale by dpi/96)
Border 1px (frame 4px resizable, 1px fixed), caption height 28, caption buttons 28x22 with 1px gap, menu bar height 24, scrollbar width 16 (thumb min 24), small icon 16, icon 32, default control height 24 (buttons/edits), padding 6, corner radius 0 for chrome (square), 2px for focus-visible only.

### 3.3 Typography
- UI font: bundled open font (candidates: Inter, Noto Sans; both OFL), 12px at 96 dpi (9pt), hinting off, grayscale antialiasing. GDI names ("MS Shell Dlg", "Tahoma", "Segoe UI", "Arial", "Microsoft Sans Serif") map to it; "Courier New"/"Consolas" map to a bundled mono (Noto Sans Mono / JetBrains Mono).
- Fixed 8x8 bitmap font stays as the last-resort fallback and for the boot console.
- Text metrics must come from the real font so layout code in apps (GetTextExtent, DrawText) matches what is drawn.

### 3.4 Window chrome (drawn by us in the non-client handler)
- Title bar: 28px, flat accent fill when active, neutral when inactive, 12px title text left aligned with 8px padding, icon 16px optional.
- Caption buttons right-aligned: minimize, maximize/restore, close. Glyphs are 10px vector strokes, 1px; hover = 12% overlay, close hover = #E5484D with white glyph, pressed = 20% overlay.
- Border: 1px #646464-ish line (WINDOWFRAME) plus 3px invisible resize hit area; subtle 1px lighter inner edge in dark mode.
- Drop shadow: 0 4px 16px rgba(0,0,0,0.35) drawn by the compositor (surfaces carry shadow metadata, not pixels).
- Active window drawn above inactive; inactive window title/border de-emphasized per palette.

### 3.5 Controls (drawn by Wine's user32/comctl32 from the palette and metrics)
Classic Win32 controls are bevelled. Tier 1 achieves a flat look by choosing the palette so the bevel collapses into a 1px border (BTNHILIGHT = 3DLIGHT = face-adjacent, BTNSHADOW = border). Tier 2 (section 6, decision 2) replaces this with a real visual style.

| Control | Rest | Hover | Pressed | Disabled | Focus |
|---|---|---|---|---|---|
| Push button | face #F0F0F0, 1px border #A0A0A0, 24px high | border #0A64C8, face #F7FAFF | face #DCE9FA, border #0A64C8, text offset 1px | text GRAYTEXT, face #F4F4F4, border #D0D0D0 | 1px dotted inner focus rect, #1A1A1A |
| Default button | as rest, border #0A64C8 2px | as hover | as pressed | - | - |
| Checkbox / radio | 14px box (radio circle), 1px #8A8F96, white fill | border #0A64C8 | fill #DCE9FA | text GRAYTEXT, border #C8CCD0 | focus rect on label |
| Edit | white, 1px border #A0A0A0, 6px padding | border #6E7681 | n/a | face #F4F4F4, text GRAYTEXT | border #0A64C8, 2px |
| Static / group box | no chrome; group box 1px #C8CCD0 line, label on interruption | - | - | text GRAYTEXT | - |
| List box / list view | white, 1px border, row 20px, selection #0A64C8 with white text | row #EAF2FD | - | text GRAYTEXT | dotted row focus |
| Combo box | edit + 16px drop button with chevron, list as list box | as button | list open: border #0A64C8 | as edit | as edit |
| Scrollbar | 16px track #F0F0F0, thumb #C1C5CA (hover #A8ADB3, drag #8A8F96), arrows are chevrons | thumb darkens | - | thumb hidden | - |
| Tab control | selected tab white with accent top line 2px | tab face #F7FAFF | - | - | dotted |
| Progress | 8px track #E3E3E3, fill #0A64C8, no segments | - | - | - | - |
| Menu | #F7F7F7, 1px border, 24px rows, 28px icon gutter, accelerator column right-aligned, selected row accent | - | - | text GRAYTEXT | - |
| Tooltip | INFOBK, 1px border, 6px padding, 150ms delay | - | - | - | - |

Dark values come from the palette in 3.1; interactive accents use the dark accent #3B8EEA.

### 3.6 Motion and compositor effects
Window open: 120ms fade+scale from 98%; close: 80ms fade. Drag: no live resample (move surface only). Respect reduced-motion. All effects live in the compositor, never in app drawing.

### 3.7 DPI and scaling
Default 96 dpi. Support 1x/1.5x/2x via devicePixelRatio: apps are DPI-unaware by default (we scale the surface, bitmap-stretched, as Windows does); DPI-aware apps get real metrics. Fonts and chrome are re-rasterized at scale; control bitmaps/icons use 1x and 2x assets.

### 3.8 Accessibility
Minimum 4.5:1 contrast for text and 3:1 for control boundaries (palette above is built to it), visible focus indicator always, keyboard-operable chrome (Alt+Space menu, Alt+F4), high-contrast variant derived from the palette.

## 4. Which layer draws what

| Thing | Drawn by | We change it via |
|---|---|---|
| Window frame, title bar, caption buttons, system menu | Our win32u non-client handler | theme file + chrome renderer (ours) |
| Client-area controls (button, edit, static, list...) | Wine user32 (GDI calls into our NtGdi) | system colors/metrics, fonts |
| comctl32 controls (toolbar, status bar, listview, tab, progress) | Wine comctl32 (GDI, optionally uxtheme) | same + uxtheme theme |
| Scrollbars of windows | Wine user32 scroll code | colors/metrics |
| Desktop, taskbar, launcher, notifications, shadows, animations | Shell + compositor (ours) | shell code |
| Cursors, icons | Our assets via NtUserSetCursor / LoadImage resources | asset pack |

## 5. Engineering plan for the look

1. `theme` module: palette + metrics + font mapping as data (TOML/JSON, embedded), served to win32u for `GetSysColor`, `GetSystemMetrics`, `SystemParametersInfo`.
2. Font engine (fontdue or similar, Rust, wasm friendly) behind NtGdi font calls; real metrics for layout.
3. Non-client renderer for chrome, used by DefWindowProc handling.
4. Bitmap/DIB engine so GDI fills, blits, icons, alpha work (needed for flat icons and caption glyphs).
5. Compositor: shadows, z-order, focus rings of surfaces; later WebGL.
6. Visual regression: golden PNG per control state and theme (the Node snapshot tool already produces PNGs), compared with tolerance in CI.

## 6. Decisions needed
1. **Flat look scope.** Tier 1 palette trick now (cheap, still has some bevel artifacts) vs. going straight to a real visual style.
2. **Visual style mechanism.** (a) Keep classic GDI drawing + palette; (b) run Wine's uxtheme with a Plonk-authored msstyles/theme (truer flat look, more work, needs bitmap engine first); (c) intercept control painting in our layer (breaks fidelity, not recommended).
3. **Fonts.** Which bundled families (Inter vs Noto Sans; mono choice), and licence acceptance (OFL).
4. **Default scheme.** Light first, dark as a toggle, or follow `prefers-color-scheme`.
5. **Reuse-Wine spike.** Approve the time-boxed spike to compile Wine's dibdrv/region/font code to wasm as a library (section 2.4.3).
6. **Compatibility meter.** Approve enabling Wine's own tests in the build as the headline metric.
