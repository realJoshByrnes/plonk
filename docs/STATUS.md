# Plonk status
**Milestone:** M3 done for the nostdlib console case: hello.exe runs through Wine 9.0's real PE ntdll/kernelbase/kernel32. Next: M4.

## Works
- Crates: mem, cpu, pe (map/relocs/imports/exports), host (Process = M1 stubs; WineProc = Wine path), testkit.
- cpu: near-full i386 integer ISA (16/32-bit, shifts, mul/div, string ops, cmov, bt*, FS base). 61 snippet tests. No x87/SSE yet.
- WineProc (crates/host/src/wine.rs): 2GB flat guest mem, KUSER_SHARED_DATA/PEB/TEB/params set up like ntdll.so,
  enters LdrInitializeThunk; syscalls trap via TEB->WOW32Reserved thunk, names from generated sysnames.rs (Wine 9.0 tables).
  Implemented so far (~35): VM alloc/free/protect/query, VFS file open/read/query, sections+image mapping, in-memory registry,
  NLS files, system/process/token info, time, NtWriteFile (stdout), NtContinue, NtTerminateProcess.
- Run: `cargo run -q -p plonk-host --release --example run_wine -- tests/guests/hello.exe` (prints last 12 syscalls + result).
- Tests: `bash scripts/test.sh`; the Wine test skips if wine/out is not built (`wine/build.sh` in WSL, then copy DLLs + nls/ into wine/out).

## Gaps
- No unix-call (`__wine_unix_call`) handlers yet; no threads/events/waits; no exceptions (SEH dispatch); no CRT exe tested (needs msvcrt).
- Registry/VFS/NLS are minimal; wineserver-protocol features (windows, messages) not started.
- Wine path not yet in the wasm build (uses std HashMap/Vec; needs a way to load DLL bytes in the browser).
- x87/SSE missing (GDI/float code will need it).

## Next (M4)
user32/gdi32: win32u syscalls (802-entry table), window/message server, display driver to Canvas 2D, then a GUI exe.
