# Plonk design

Status: direction chosen (Plonk Glass). Sections 1-2 say how we keep compatibility from collapsing; section 3 defines the glass look; 4 says which layer draws what; 5 is the staged roadmap; 6 lists open decisions. Preview: `web/design.html`.

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

## 3. Visual design: Plonk Glass

Decision (owner): go straight to a real visual style, with a premium glass feel. Light is the first-class scheme; dark is derived from the same recipe. Live preview: `web/design.html`. The glass compositor and chrome in section 3.2-3.5 are **already implemented** (software, Rust).

### 3.1 Concept
Frosted, translucent **chrome** floating over a vivid wallpaper; **opaque, quiet content** inside so text stays crisp. One accent colour. Depth comes from blur, soft shadow and a thin luminous edge, not from bevels.

Layers, back to front: wallpaper -> shadow -> glass frame/title bar (blurs what is behind it) -> opaque client area (controls) -> optional glass regions inside a client (sidebars/toolbars, section 3.7).

### 3.2 Glass recipe (all values are data in the theme file; light first)
| Property | Light | Dark |
|---|---|---|
| Backdrop blur | 24px (3 box passes, ~gaussian) | 24px |
| Backdrop adjust | saturation x1.25, +6/255 brightness | x1.15, -4/255 |
| Frame tint (active) | white 58% at top fading to 34% | #12161C 55% -> 38% |
| Frame tint (inactive) | white 40% -> 26% | #12161C 40% -> 28% |
| Side/bottom border tint | white 38% (active), 28% (inactive) | #12161C 40% / 30% |
| Specular edge | top 1px white 95%, left 55%, right 40%, bottom 28% | white 40% / 22% / 16% / 10% |
| Hairlines | 1px black 10-12% under the title bar and around the client | white 8% |
| Grain | +/-1.5 levels on glass pixels (kills banding) | same |
| Corner radius | 10px chrome | 10px |
| Shadow | 28px, offset y = 1/3 of size, max 32% black, only outside the window | 40% |
| Reduce-transparency mode | tints become 100% opaque, blur off | same |

Wallpaper (default): soft aurora gradient (blue -> violet -> pink with cyan and amber glows), chosen so glass has colour to pick up. Replaceable; glass adapts automatically.

### 3.3 Palette (system colours used by Wine's controls)
| Role | Light | Dark |
|---|---|---|
| Window / editable | #FFFFFF | #1F2329 |
| Window text | #1A1A1A | #E6E8EB |
| Face / dialog bg (BTNFACE) | #F1F3F6 | #2B3038 |
| Button text | #1A1A1A | #E6E8EB |
| Highlight 3D (BTNHILIGHT) | #FFFFFF | #3A4048 |
| 3D light | #E6EAF0 | #323840 |
| Shadow (BTNSHADOW) | #A7B0BC | #1A1D22 |
| Dark shadow (3DDKSHADOW) | #8E98A6 | #0F1114 |
| Frame (WINDOWFRAME) | #8A93A0 | #0F1114 |
| Accent / selection | #0A64C8 | #3B8EEA |
| Selection text | #FFFFFF | #FFFFFF |
| Caption text (active / inactive) | #1A1A1A / #6B7078 | #F2F4F7 / #8B9199 |
| Disabled text | #8A8F96 | #6B7078 |
| Menu | #F7F8FA | #262B32 |
| Tooltip bg / text | #FFFFE1 / #1A1A1A | #33383F / #E6E8EB |

### 3.4 Metrics (96 dpi; scale by dpi/96)
Frame ring 4px, caption 28px, caption buttons 36x22 with the close button a red glass pill (80% active, 35% inactive), menu bar 24, scrollbar 16, default control height 24, padding 6, chrome radius 10, control radius 4, focus ring 2px accent at 60% with 2px soft glow.

### 3.5 Window chrome (ours, implemented)
Drawn into the window surface with alpha; the compositor blurs behind every pixel whose alpha is below 255 and applies the recipe. Title: 11px-equivalent, etched (light 70% shadow one pixel down-right). Buttons: minimize, maximize, close with thin 1px glyphs. Hit regions: title band drags, close button posts WM_CLOSE; resize edges and min/max are next.

### 3.6 Controls: real visual style (Wine uxtheme + a Plonk theme)
Controls are drawn by Wine's user32/comctl32. With the classic path they are bevelled and opaque, which fights the glass look. The real style means Wine's **uxtheme** loads a **Plonk theme** (.msstyles: a resource DLL with an INI part list and 9-slice bitmaps with alpha) so every control state is a bitmap we design. Built-in classes then render through `DrawThemeBackground`, which needs the bitmap/DIB engine (AlphaBlend, StretchBlt, DIB sections) and comctl32 v6 theming (manifest + SubclassWindow behaviour).

Control look (light; dark swaps colours per 3.3):
| Control | Rest | Hover | Pressed | Disabled | Focus |
|---|---|---|---|---|---|
| Push button | white 78% fill, 1px border black 16%, top inner highlight white 70%, radius 4 | fill white 95%, border accent 55% | fill accent 14% over white, inner shadow 1px, text +1px | fill white 40%, text #8A8F96 | 2px accent ring 60% |
| Default / primary | accent gradient #2A7FE0 -> #0A64C8, white text, inner highlight | +6% lighter | -8% darker | 40% opacity | accent ring + white 1px gap |
| Check / radio | 14px, white fill, 1px #8A93A0, 4px radius (radio round) | border accent | fill accent 14% | border #C8CCD0 | accent ring |
| Edit | white, 1px #A7B0BC, radius 4, padding 6 | border #6E7681 | n/a | fill #F4F4F4 | border accent 2px |
| List / list view | white, row 22, selection accent with white text | row accent 8% | row accent 18% | text grey | dotted row |
| Combo | edit + 22px chevron button | as button | list opens as glass popup | as edit | as edit |
| Scrollbar | 10px thin overlay track (grows to 16 on hover), thumb black 22% | black 36% | black 50% | hidden | - |
| Tab | selected: white, 2px accent top line | face white 40% | - | - | accent ring |
| Progress | 6px track black 8%, fill accent gradient, 3px radius | - | - | - | - |
| Menu / popup | **glass popup** (blur 24, tint white 70%), 1px edge, row 26, selection accent 14% | - | - | grey | - |
| Tooltip | glass pill, tint white 80%, 150ms delay | - | - | - | - |

Dark values come from 3.3; hover/pressed overlays use white instead of black.

### 3.7 Glass inside client areas
Apps opt in as on Windows (DwmExtendFrameIntoClientArea, and painting with alpha-0 pixels via GDI/layered paths). We provide a `dwmapi` shim and treat alpha-0 or alpha-keyed regions in a client surface as glass; default is opaque. Toolbars/status bars may use it; text areas never do.

### 3.8 Typography
UI font: bundled open font (Inter or Noto Sans; OFL), 12px at 96 dpi, grayscale AA, hinting off, 400/600 weights; GDI names (MS Shell Dlg, Tahoma, Segoe UI, Arial) map to it, monospace names to a bundled mono. Text over glass is always dark-on-tinted with the etched shadow; minimum 4.5:1 against the worst case (white tint 34% over saturated wallpaper). The 8x8 bitmap font remains only as a boot/fallback face; real glyph metrics are required before premium finish.

### 3.9 Motion
Open 140ms (fade + scale from 97%, ease-out), close 90ms, minimize to taskbar: shrink toward icon 180ms, drag: no live resample. Respect reduced-motion. All effects live in the compositor.

### 3.10 DPI and accessibility
1x/1.5x/2x via devicePixelRatio; DPI-unaware apps are stretched, aware apps get real metrics; chrome and fonts re-rasterize. "Reduce transparency" and high-contrast switch tints to opaque and boost borders. Visible focus ring always; chrome keyboard-operable (Alt+Space, Alt+F4).

## 4. Which layer draws what

| Thing | Drawn by | We change it via |
|---|---|---|
| Wallpaper, shadows, blur, rounded corners, animations | Compositor (ours; software now, WebGL later) | theme + compositor |
| Window frame, title bar, caption buttons | Our win32u non-client code (alpha pixels) | theme + chrome renderer |
| Client controls (button, edit, static, list, ...) | Wine user32 via GDI/uxtheme into our NtGdi | Plonk theme (msstyles) + palette + fonts |
| comctl32 controls (toolbar, status bar, listview, tab, progress) | Wine comctl32 via uxtheme | same |
| Popup menus, tooltips | Wine user32 windows we mark as glass surfaces | surface style flags |
| Taskbar, launcher, notifications | Shell (ours) | shell code |

## 5. Roadmap (stages, each with an exit test)
| Stage | Work | Exit test | Status |
|---|---|---|---|
| G0 | Software glass compositor + glass chrome + wallpaper | PNG goldens, 15 ms compose for 2 glass windows at 1024x768 (wasm) | done |
| G1 | GPU compositor (WebGL blur/shadow); keeps software fallback | 60 fps with 8 glass windows | next |
| G2 | Bitmap/DIB engine: NtGdi DIB sections, BitBlt/StretchBlt/AlphaBlend, icons, cursors | Wine gdi32 tests for blits pass | |
| G3 | Font engine + real metrics (bundled fonts) | text layout tests match drawn text | |
| G4 | Plonk theme as .msstyles, built by a generator; Wine uxtheme loads it; comctl32 v6 | golden PNG per control state, light + dark | |
| G5 | Popup menus/tooltips as glass surfaces; client glass (dwmapi shim) | Notepad-class apps look native | |
| G6 | Resize, min/max, taskbar, shell | shell scenario tests | |

## 6. Decisions
Resolved: real visual style (not the palette trick); premium glass look; light first, dark derived.
Open (defaults in bold, change if you disagree):
1. **Theme mechanism: msstyles via Wine uxtheme** (alternatives: our own uxtheme-compatible provider replacing Wine's DLL, more work, more control).
2. **Fonts: Inter + Noto Sans Mono** (OFL).
3. **GPU path: WebGL for the compositor in G1**, software blur stays as fallback and for tests.
4. Reuse spike for Wine's dibdrv/region code (section 2.4.3): run it **alongside G2**, since G2 is where it would pay off.
5. Make Wine's conformance tests the headline compatibility metric (section 2.3), starting with user32 and gdi32.
