# Plonk status
**Milestone:** M2 plan written (docs/M2_PLAN.md), awaiting go-ahead for M3.

## Works
- Crates: mem, cpu, pe (sections/relocs/imports), host (Process, thunk dispatch, wasm exports), testkit.
- hello.exe (mingw i686, -nostdlib) runs natively and in wasm under Node: GetStdHandle, WriteFile, ExitProcess stubs.
- Thunks: IAT -> magic addresses 0xF000_0000+; cpu returns Exit::Thunk; host does stdcall return. run() is reentrant (state is in Cpu).
- cpu adds: FF group, setcc, movzx/movsx. 27 snippet tests.
- `bash scripts/test.sh` = snippets + wasm build + Node (selftest + hello.exe).

## Gaps
- Guest memory is an owned 8MB Vec, not yet a fixed linear-memory region shared with Wine.
- No CRT-based exe tested; no 16-bit ops, shifts, mul/div, string ops, FPU, segments/TEB.
- clang/wasm-ld not installed (needed for M2/M3).

## Next
M2: time-boxed Wine-on-wasm32 feasibility; written plan with go/no-go + fallback.
