# Plonk status
**Milestone:** M0 done (workspace + toolchain skeleton). Next: M1.

## Works
- Crates: mem (guest memory, base+addr), cpu (i386 interpreter subset), testkit (snippet harness), host (wasm cdylib).
- cpu: ALU (8/32-bit), inc/dec, push/pop, mov/lea, modrm+SIB, jcc/jmp/call/ret, flags.
- `bash scripts/test.sh`: 20 snippet tests + wasm build + Node selftest, quiet output.

## Gaps
- No 16-bit ops/prefixes, shifts, mul/div, string ops, 0xFF group, FPU, segments.
- No PE loader, no thunks. Host guest region is a static 64KB array (placeholder).
- clang/wasm-ld not installed (needed for M2/M3).

## Next
M1: PE loader + thunk traps + kernel32 stubs to run mingw hello.exe.
