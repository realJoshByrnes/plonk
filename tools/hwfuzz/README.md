# hwfuzz: hardware differential fuzzer for crates/cpu

Runs ~1800 instruction templates x 40-100 random input states (143k cases) on the real CPU
(32-bit mingw exe, WoW64) and through `plonk_cpu::Cpu`, then compares all GPRs, defined EFLAGS
(undefined flags masked per the SDM), esp delta and a 512-byte scratch buffer.

Files: `gen.py` (templates + deterministic cases), `harness.c` (native runner), `build.sh`,
`run.ps1` (driver), `crates/cpu/examples/hwfuzz.rs` (interpreter side). Output goes to `tools/hwfuzz/out/`.

## One command each (from the repo root, PowerShell)

    powershell -ExecutionPolicy Bypass -File tools/hwfuzz/run.ps1 gen          # templates.s + cases.bin (seed 12345; optional: python tools/hwfuzz/gen.py tools/hwfuzz/out SEED)
    powershell -ExecutionPolicy Bypass -File tools/hwfuzz/run.ps1 hw           # build harness.exe in WSL Debian, run on hardware -> hw.bin, code.bin
    powershell -ExecutionPolicy Bypass -File tools/hwfuzz/run.ps1 cmp --all    # run the interpreter and compare
    powershell -ExecutionPolicy Bypass -File tools/hwfuzz/run.ps1 all --all    # all three

`cmp` options: `--slow` (use `Cpu::run_slow`, the reference loop, instead of `run`), `--all` (every failing
template, default top 40), `--filter SUBSTR` (only templates whose name contains it).
Exit status is 1 when anything mismatches. `gen`/`hw` need only be redone when gen.py changes;
`cmp` alone re-tests a new core in a few seconds.

If harness.exe prints "cannot map fixed region at 200000" just rerun `hw` (ASLR collision with 0x200000).

Layout shared by both sides: scratch 0x210000 (esi=+128..143, edi=+320..335, ebx=+224..239 for memory
templates), CRC32 table 0x220000, code 0x230000, interpreter esp 0x3F0000.

Adding a template: `T(name, asm, mask, mem=..., init=..., mfn=...)` in gen.py; asm is GAS intel syntax or
`.byte` lines (`;` separates lines); `mfn(state) -> (flag_mask, skipped_reg_bitmask)` for case-dependent
undefined results.
