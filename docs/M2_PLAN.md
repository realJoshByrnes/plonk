# M2: Wine-on-wasm32 feasibility

Basis: Wine architecture knowledge (Wine 8/9 era). I did NOT clone or grep Wine source this session; items marked (verify) must be checked first in M3 step 0.

## Architecture facts that drive the decision
- Since Wine ~7, user-mode DLLs (ntdll, kernel32, kernelbase, win32u, user32, gdi32, ...) are **PE files** built with clang/mingw for i686-windows. Only a thin Unix half remains: `ntdll.so` (dlls/ntdll/unix, ~100k lines of POSIX: mmap, signals, threads, fd passing, futex) plus drivers. PE ntdll reaches it via syscalls and `__wine_unix_call`.
- win32u (window tree, message queues, focus, clipboard, ...) keeps its state in **wineserver**, a separate process reached over a unix socket + fd passing. Handles, files, registry, sections, threads and processes also live there.
- Software GDI (dibdrv) lives inside win32u (PE). A display driver is a function table (`user_driver_funcs`) that win32u calls: this is where our Canvas driver plugs in.

## Option A: compile Wine to wasm32 (the original ask)
Needs the old ELF/winelib-style build path of ntdll/kernel32/user32 retargeted to wasm32, because the modern build only emits PE.
- Blockers: Wine's Unix side assumes signals (SEH, SIGSEGV for stack/exception handling), threads, mmap/fd semantics, and the wineserver socket. None exist on wasm without large rewrites.
- Windows ABI details leak everywhere: stdcall, 2-byte wchar (`-fshort-wchar`), SEH (`__TRY`), FS:TEB access, inline asm, function pointers handed to guest code (WndProc). Each needs a shim.
- Guest function pointers vs wasm table indices need a translation layer in both directions.
- Verdict: **NO-GO for the smallest slice.** Months of porting before hello.exe runs, and it diverges from upstream.

## Option B (fallback, recommended): Wine's prebuilt PE DLLs run inside the interpreter
Interpret Wine's PE ntdll/kernel32/kernelbase/win32u/user32/gdi32 unchanged. Plonk implements, natively in Rust, everything below the PE line:
1. **Unix-side entry points**: `__wine_unix_call` dispatch and ntdll's unixlib functions used at startup/loader; syscall stubs (`NtCreateFile`, `NtAllocateVirtualMemory`, `NtMapViewOfSection`, `NtQuery*`, `NtWaitForSingleObject`, ...). Start with the set hit by a console hello, add on demand by logging unimplemented ones.
2. **A mini-wineserver in Rust**, in-process, replacing the socket: object/handle table, process/thread, events/mutexes, sections, a virtual FS, minimal registry. Skip what hello and a GUI exe never touch.
3. **Window/message server** (needed at M4): window tree, per-thread message queue, timers, input, WM_PAINT/WM_CLOSE. Because windows already live behind the server protocol, this is where the "separate surfaces for a future shell" design comes from naturally.
4. **Display driver** implementing `user_driver_funcs` against the display crate (software framebuffer to Canvas 2D).
5. **Interpreter growth**: FS segment (TEB/PEB), full integer ISA, string ops, shifts/mul/div, 16-bit ops, x87 (GDI/float), SSE2 only if the PE DLLs need it. Prefer building Wine's DLLs with `-mno-sse`-style flags where possible (verify).
6. **Guest memory**: allocate one fixed linear-memory region at startup; NT virtual-memory calls manage it (decision for M3).
- Cost: much smaller than A. Risks: interpreter speed (Wine startup runs millions of instructions; need a decode cache), exact NT semantics in the server, and Wine's build needing a Linux host (WSL Debian is present; need i686 mingw or clang with `-target i686-windows`, and a Wine source pin).

## Smallest viable slice (M3 target)
`hello.exe` linked normally (with CRT, import `kernel32!WriteFile`) -> Wine PE kernel32/kernelbase -> PE ntdll -> our syscalls. Steps: (0) verify claims above with a pinned Wine tree; (1) build PE DLLs in WSL, commit scripts not binaries; (2) TEB/PEB + loader init; (3) run `LdrInitializeThunk`-equivalent until main; (4) fill syscalls on demand; (5) console write via NtWriteFile/ConDrv path (verify; may need a stub).

## Decision
- **Option A: NO-GO.** **Option B: GO**, with the caveat that it is a "Wine as PE under interpretation" design, not "Wine compiled to wasm". Native-wasm Wine could still be revisited for hot DLLs (e.g. gdi32 dibdrv) later, behind the same import-thunk interface.
- Needs from you: OK to use WSL Debian for building Wine PE DLLs, and to install clang/lld there (not on Windows).
