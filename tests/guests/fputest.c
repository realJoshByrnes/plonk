// x87 differential test: the same program must print the same checksums with the interpreter and with the wasm JIT.
// gcc -m32 -O1 -static -o fputest.elf fputest.c  (run it hot: every block executes thousands of times)
#include <stdio.h>
#include <string.h>
#include <stdint.h>

static const double D[] = { 1e-3, -1e-3, 1.0, -1.0, 0.5, -0.5, 1.5, 2.5, -2.5, 3.0, 1e10, -1e10, 3.4e17, 1e-30, 4.5e15, 2147483647.0, 2147483648.0,
                            -2147483648.0, -2147483649.0, 32767.0, 32768.0, -32768.0, -32769.0, 9.3e18, -9.3e18, 123456.789, 0.1, 1.0/3, 7.0e-5 };
#define ND (sizeof(D) / sizeof(D[0]))
static uint64_t hh[24]; static int cur;
#ifdef DUMP
#define DUMPNL (cur == DUMP)
static void mix(uint64_t v) { if (cur == DUMP) printf("%llx ", (unsigned long long)v); hh[cur] = (hh[cur] ^ v) * 1099511628211ULL; }
#else
#define DUMPNL 0
static void mix(uint64_t v) { hh[cur] = (hh[cur] ^ v) * 1099511628211ULL; }
#endif
static void mixd(double d) { uint64_t b; memcpy(&b, &d, 8); mix(b); }

#define BLOCK(name, ...) \
static void name(double a, double b, int rc) { \
    unsigned short cw, sw; double r0, r1, r2; int32_t i32 = 0x55555555; int16_t i16 = 0x5555; int64_t i64 = 0x5555555555555555LL; float f32 = 0; unsigned flags; \
    unsigned short cw0 = 0x37F | (rc << 10); \
    __asm__ volatile("fldcw %0" :: "m"(cw0)); \
    __VA_ARGS__ \
    __asm__ volatile("fnstsw %0" : "=m"(sw)); __asm__ volatile("fnstcw %0" : "=m"(cw)); \
    mix(sw); mix(cw); mixd(r0); mixd(r1); mixd(r2); mix((uint32_t)i32); mix((uint16_t)i16); mix((uint64_t)i64); mix(*(uint32_t *)&f32); mix(flags); \
    __asm__ volatile("fninit"); \
}

BLOCK(t_arith_mem, {
    __asm__ volatile("fldl %3; fldl %4; fadd %%st(1), %%st; fmull %4; fsubl %3; fdivrl %4; fstpl %0; fstpl %1; fldl %3; fsubrl %4; fdivl %3; fstpl %2; fninit" : "=m"(r0), "=m"(r1), "=m"(r2) : "m"(a), "m"(b), "m"(a));
    flags = 0;
})
BLOCK(t_arith_reg, {
    __asm__ volatile("fldl %3; fldl %4; fld %%st(1); fadd %%st(1), %%st; fxch %%st(2); fsub %%st(2), %%st; fmul %%st(1), %%st; fdiv %%st(1), %%st;"
                     "fsubr %%st(2), %%st; fdivr %%st(1), %%st; faddp %%st, %%st(1); fsubrp %%st, %%st(1); fmulp %%st, %%st(1); fstpl %0; fldl %3; fldl %4; fdivp %%st, %%st(1); fstpl %1; fldl %3; fchs; fabs; fchs; fstpl %2" : "=m"(r0), "=m"(r1), "=m"(r2) : "m"(a), "m"(b), "m"(a));
    flags = 0;
})
BLOCK(t_int, {
    __asm__ volatile("fldl %3; fistpl %0; fldl %3; fistpll %2; fldl %3; fistps %1; fldl %4; fistl %0; fistp %5; fldl %3; fnstsw %%ax" : "=m"(i32), "=m"(i16), "=m"(i64), "+m"(a) : "m"(b), "m"(i32) : "eax");
    r0 = r1 = r2 = 0; flags = 0;
})
BLOCK(t_ild, {
    int32_t x = (int32_t)a; int16_t y = (int16_t)b; int64_t z = (int64_t)(a * 1e6);
    __asm__ volatile("fildl %3; fild %4; faddp; filds %4; fmulp; fildll %5; faddp; fstpl %0; fldz; fld1; fstpl %1; fstpl %2; fistpl %6" : "=m"(r0), "=m"(r1), "=m"(r2) : "m"(x), "m"(y), "m"(z), "m"(i32));
    flags = 0;
})
BLOCK(t_flt, {
    float fa = (float)a, fb = (float)b;
    __asm__ volatile("flds %3; fadds %4; fmuls %4; fsts %0; fstps %5; flds %3; fsubrs %4; fdivs %4; fstpl %1; flds %3; fsqrt; fstpl %2" : "=m"(f32), "=m"(r1), "=m"(r2) : "m"(fa), "m"(fb), "m"(i32));
    r0 = f32; flags = 0;
})
BLOCK(t_cmp, {
    unsigned short s1, s2, s3, s4;
    __asm__ volatile("fldl %5; fldl %6; fcom %%st(1); fnstsw %0; fcomp %%st(1); fnstsw %1; fcompp; fnstsw %2; fldl %5; fldl %6; fucompp; fnstsw %3" : "=m"(s1), "=m"(s2), "=m"(s3), "=m"(s4) : "m"(i32), "m"(a), "m"(b) : "ax");
    __asm__ volatile("fldl %0; ftst; fnstsw %%ax; fstp %%st(0); fldl %0; fcoml %1; fnstsw %%ax; fcompl %1; fstp %%st(0)" :: "m"(a), "m"(b) : "eax");
    r0 = s1 | s2 << 16; r1 = s3 | s4 << 16; r2 = 0; flags = 0;
})
BLOCK(t_cmpi, {
    unsigned f1, f2, f3;
    __asm__ volatile("fldl %3; fldl %4; fcomi %%st(1), %%st; pushf; pop %0; fucomi %%st(1), %%st; pushf; pop %1; fcomip %%st(1), %%st; pushf; pop %2; fstp %%st(0); fldl %3; fldl %4; fucomip %%st(1), %%st; fstp %%st(0)"
                     : "=r"(f1), "=r"(f2), "=r"(f3) : "m"(a), "m"(b));
    r0 = f1 & 0x8D5; r1 = f2 & 0x8D5; r2 = f3 & 0x8D5; flags = f1 ^ f3;
})
BLOCK(t_cmov, {
    unsigned x = 1;
    __asm__ volatile("fldl %3; fldl %4; fcomi %%st(1), %%st; fcmovb %%st(1), %%st; fcmove %%st(1), %%st; fcmovbe %%st(1), %%st; fcmovu %%st(1), %%st; fcmovnb %%st(1), %%st;"
                     "fcmovne %%st(1), %%st; fcmovnbe %%st(1), %%st; fcmovnu %%st(1), %%st; fstpl %0; fstp %%st(0); fldl %4; fstpl %1; fld1; fldz; fldpi; fstpl %2; fstp %%st(0); fstp %%st(0)" : "=m"(r0), "=m"(r1), "=m"(r2) : "m"(a), "m"(b), "m"(x));
    flags = 0;
})
BLOCK(t_misc, {
    __asm__ volatile("fldl %3; frndint; fstpl %0; fldl %4; frndint; fsqrt; fstpl %1; fldl %3; fld1; fldl2e; fldl2t; fldlg2; fldln2; fstp %%st(0); fstp %%st(0); fstp %%st(0); fstp %%st(0); fxch; fstpl %2; fstp %%st(0)" : "=m"(r0), "=m"(r1), "=m"(r2) : "m"(a), "m"(b));
    flags = 0;
})

BLOCK(u_istpl, { __asm__ volatile("fldl %1; fistpl %0" : "=m"(i32) : "m"(a)); r0 = r1 = r2 = 0; flags = 0; })
BLOCK(u_istpll, { __asm__ volatile("fldl %1; fistpll %0" : "=m"(i64) : "m"(a)); r0 = r1 = r2 = 0; flags = 0; })
BLOCK(u_istps, { __asm__ volatile("fldl %1; fistps %0" : "=m"(i16) : "m"(a)); r0 = r1 = r2 = 0; flags = 0; })
BLOCK(u_istl, { __asm__ volatile("fldl %1; fistl %0; fstp %%st(0)" : "=m"(i32) : "m"(a)); r0 = r1 = r2 = 0; flags = 0; })
BLOCK(u_fisttp, { __asm__ volatile("fldl %1; fisttpl %0" : "=m"(i32) : "m"(a)); r0 = r1 = r2 = 0; flags = 0; })
BLOCK(u_ildl, { int32_t x = (int32_t)a; __asm__ volatile("fildl %1; fstpl %0" : "=m"(r0) : "m"(x)); r1 = r2 = 0; flags = 0; })
BLOCK(u_ilds, { int16_t x = (int16_t)a; __asm__ volatile("filds %1; fstpl %0" : "=m"(r0) : "m"(x)); r1 = r2 = 0; flags = 0; })
BLOCK(u_ildll, { int64_t x = (int64_t)(a * 1e6); __asm__ volatile("fildll %1; fstpl %0" : "=m"(r0) : "m"(x)); r1 = r2 = 0; flags = 0; })
BLOCK(u_rnd, { __asm__ volatile("fldl %1; frndint; fstpl %0" : "=m"(r0) : "m"(a)); r1 = r2 = 0; flags = 0; })
BLOCK(u_sqrt, { __asm__ volatile("fldl %1; fabs; fsqrt; fstpl %0" : "=m"(r0) : "m"(a)); r1 = r2 = 0; flags = 0; })
BLOCK(u_const, { __asm__ volatile("fld1; fldl2e; fldl2t; fldlg2; fldln2; fldpi; fldz; fstpl %0; fstpl %1; fstpl %2; fstp %%st(0); fstp %%st(0); fstp %%st(0); fstp %%st(0)" : "=m"(r0), "=m"(r1), "=m"(r2)); flags = 0; })
BLOCK(u_xch, { __asm__ volatile("fldl %2; fldl %3; fxch %%st(1); fstpl %0; fstpl %1" : "=m"(r0), "=m"(r1) : "m"(a), "m"(b)); r2 = 0; flags = 0; })

int main(void) {
    for (int it = 0; it < 4000; it++) {
        for (int rc = 0; rc < 4; rc++) {
            double a = D[it % ND], b = D[(it * 7 + 3) % ND];
            if (it % 11 == 0) b = a;
            int odd = 0;
            if (it % 13 == 0) { a = __builtin_nan(""); odd = 1; }
            if (it % 17 == 0) { b = __builtin_inf(); odd = 1; }
            if (!odd) { cur = 0; t_arith_mem(a, b, rc); cur = 1; t_arith_reg(a, b, rc); cur = 2; t_int(a, b, rc); cur = 3; t_ild(a, b, rc); cur = 4; t_flt(a, b, rc); } cur = 5; t_cmp(a, b, rc); cur = 6; t_cmpi(a, b, rc); cur = 7; t_cmov(a, b, rc); if (!odd) { cur = 8; t_misc(a, b, rc); cur = 9; u_istpl(a, b, rc); cur = 10; u_istpll(a, b, rc); cur = 11; u_istps(a, b, rc); cur = 12; u_istl(a, b, rc); cur = 13; u_fisttp(a, b, rc); cur = 14; u_ildl(a, b, rc); cur = 15; u_ilds(a, b, rc); cur = 16; u_ildll(a, b, rc); cur = 17; u_rnd(a, b, rc); cur = 18; u_sqrt(a, b, rc); cur = 19; u_const(a, b, rc); cur = 20; u_xch(a, b, rc); }
        }
    }
    for (int k = 0; k < 21; k++) printf("fputest %d checksum %016llx\n", k, (unsigned long long)hh[k]);
    return 0;
}
