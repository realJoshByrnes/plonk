# CPU core mismatches vs real hardware (tools/hwfuzz)

Seed 12345, 1811 templates / 143820 cases: 50 failing templates, ALL in the groups below.
Everything else matched hardware bit-exactly, including every add/or/adc/sbb/and/sub/xor/cmp form
(8/16/32, high byte regs, group-1, acc-imm, memory), inc/dec/neg/not/test, mul/imul (1/2/3 op),
div/idiv (8/16/32), shl/shr/sar/rol/ror/rcl/rcr (1, imm, cl, counts 0..255, 8/16/32), bsf/bsr, bswap,
xchg/cmpxchg/xadd, movzx/movsx, cwde/cdq, setcc/cmovcc (all 16), jcc/loop/jecxz, lea (32-bit modes),
lahf/sahf, pushfd/popfd, all string ops with rep/repe/repne and DF=0/1, xlat, leave, call/ret,
and the CRC32 (4 variants, table driven), adler32 (div 65521) and inflate bit-reader / window-copy snippets.

Line numbers refer to crates/cpu/src/lib.rs as of this run (it is being rewritten, treat as hints).

## 1. 16-bit bt/bts/btr/btc use 32-bit semantics (silent wrong result)  ~lines 590-602
Templates `bt16/bts16/btr16/btc16 r,r | r,imm | [esi],r`. `66 0F AB C8` (bts ax,cx) with cx=0xffff:
hardware bit = cx & 15 = 15, result 0x801a; interpreter used off&31 and the full ecx, set bit 31.
Cause: `rd/wr(.., 32)`, `off & 31`, `>>5 * 4` ignore the operand size `vw`; the register offset is read from
`self.r[reg]` (32 bit). Patch:
```diff
-let (kind, off) = if op2 == 0xBA {(reg, self.f8(m)? as u32)} else {(.., self.r[reg])};
+let (kind, off) = if op2 == 0xBA {(reg, self.f8(m)? as u32)} else {(.., self.getr(reg, vw))};
+let wb = vw;   // 16 or 32
-Op::M(a) if op2 != 0xBA => (Op::M(a + ((off as i32) >> 5) as u32 * 4), off & 31),
+Op::M(a) if op2 != 0xBA => { let o = if wb == 16 { off as u16 as i16 as i32 } else { off as i32 };
+                             (Op::M(a.wrapping_add(((o >> if wb == 16 {4} else {5}) as u32) * (wb/8))), off & (wb-1)) }
-_ => (rm, off & 31),
+_ => (rm, off & (wb-1)),
-let v = self.rd(m, rm, 32)?;  ... self.wr(m, rm, 32, nv)?;
+let v = self.rd(m, rm, wb)?;  ... self.wr(m, rm, wb, nv)?;
```
Impact: only 16-bit bt; unlikely in the Wise failure.

## 2. shld/shrd: OF not computed (count==1), and count>16 for 16-bit  ~lines 603-618
`shld eax,edx,1`: hardware OF = (sign changed) = msb(result) ^ msb(original dest); interpreter leaves OF stale.
Same for shrd (OF = msb(result) ^ msb(dest)). Also ZF/SF/PF are fine, AF undefined (ignored).
Patch after `self.szp(...)`:
```diff
+if c == 1 { let sgn = 1u64 << (vw - 1); self.setf(OF, (r ^ d) & sgn != 0); }
```
Impact: only code that reads OF right after a 1-bit double shift (jo/seto/pushf); practically none.

## 3. Unimplemented opcodes (loud: Exit::Unimplemented, not silent)
- `D6` salc; `27/2F/37/3F` daa/das/aaa/aas; `D4/D5` aam/aad (imm8) (hardware accepts them in 32-bit mode)
- `C8` enter (leave is OK)
- `67` address-size prefix (`jcxz`, `loop`, 16-bit-addressed lea/mem operands)
(`xlat` was missing in the first run; another change added it, it now passes.)

## 4. 16-bit push/pop and `66 68` are executed as 32-bit  lines 254-263, 381-382, 394
`66 50+r`/`66 58+r` (push ax / pop bx), `66 68 imm16`, `66 6A`, `66 FF /6` move esp by 4 instead of 2 and
write/read 4 bytes. Hardware: esp -= 2, pop bx only replaces bx. Patch: in push/pop opcodes check `self.ow == 16`
and use `m.w16/m.r16` with +-2 and `set16` for the register. Impact: 16-bit code only.

## Wise CRC failure: not explained
No integer instruction semantic difference was found in anything inflate/CRC32 compiled code uses:
the table-driven CRC32 variants (movzx/xor dl,al/shr 8/xor [table+edx*4], lodsb form, slice-by-4 with
ah/al/shr 16, bitwise shr/jnc/xor 0xEDB88320), adler32 with div 65521, the shl/or/shr bit-buffer getbits
with `shl eax,cl`, the shr bl,1/rcr bl,1/adc eax,eax bit reader, `rep movsb/movsd` window copies with
overlap and DF=1, xlat, and all conditional jumps. All 100% hardware-exact (flags too).
Not covered by this fuzzer (so the next suspects): x87/SSE, unaligned or page/region-boundary memory access
in crates/mem (r32/w32 crossing guard/region ends), the `rep` count hack at lib.rs ~473
(`ecx*size > 0x1000_0000` treated specially), instructions that fault mid-way and restart (rep with a fault),
self-modifying code / the new block cache invalidation, `lock`/`rep`-prefixed non-string instructions,
segment overrides (fs:), and host side (ReadFile/SetFilePointer/GetFileSize/memcpy thunks, file offsets >2GB or
MapViewOfFile). The failure is therefore more likely an API/host-semantics bug (e.g. data the CRC is computed
over differs because of a short read/seek) than an ALU bug; compare the CRC input buffer bytes against a
native Wine run, or run the installer's CRC routine in the interpreter against the same bytes natively.
