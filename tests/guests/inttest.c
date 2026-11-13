// String ops, mul/div and cmpxchg/xadd differential test (see web/jit_diff.mjs): the checksums must not depend on the JIT.
// gcc -m32 -O1 -static -o inttest.elf inttest.c
#include <stdio.h>
#include <stdint.h>
#include <string.h>

static uint64_t hh[8]; static int cur;
static void mix(uint64_t v) { hh[cur] = (hh[cur] ^ v) * 1099511628211ULL; }
static uint8_t buf[4096 + 64], buf2[4096 + 64];
static const uint32_t V[] = { 0, 1, 2, 3, 7, 0xff, 0x100, 0x7fffffff, 0x80000000, 0x80000001, 0xffffffff, 0xfffffffe, 123456789, 0xdeadbeef, 65535, 65536 };
#define NV (sizeof(V) / sizeof(V[0]))

static inline uint32_t fl(void) { uint32_t f; __asm__ volatile("pushf; pop %0" : "=r"(f)); return f & 0x8d5; }

int main(void) {
    for (int it = 0; it < 3000; it++) {
        uint32_t a = V[it % NV], b = V[(it * 5 + 1) % NV];
        // movs/stos/lods with rep, all widths, different offsets (also crossing the page of the buffer)
        for (int k = 0; k < 3; k++) {
            for (int j = 0; j < 64; j++) { buf[j + (it & 31)] = (uint8_t)(a + j * 7 + k); }
            void *s = buf + (it & 31), *d = buf2 + (it & 15); unsigned n = 5 + (it % 37), c;
            cur = 0; c = n; __asm__ volatile("cld; rep movsb" : "+S"(s), "+D"(d), "+c"(c) :: "memory"); mix(c); mix((uintptr_t)d - (uintptr_t)buf2);
            s = buf + (it & 31); d = buf2 + 100; c = n; __asm__ volatile("rep movsl" : "+S"(s), "+D"(d), "+c"(c) :: "memory");
            s = buf + (it & 31); d = buf2 + 300; c = n; __asm__ volatile("rep movsw" : "+S"(s), "+D"(d), "+c"(c) :: "memory");
            d = buf2 + 600; c = n; uint32_t ax = a; __asm__ volatile("rep stosl" : "+D"(d), "+c"(c) : "a"(ax) : "memory");
            d = buf2 + 700; c = n; __asm__ volatile("rep stosb" : "+D"(d), "+c"(c) : "a"(ax) : "memory");
            d = buf2 + 800; c = n; __asm__ volatile("rep stosw" : "+D"(d), "+c"(c) : "a"(ax) : "memory");
            s = buf2 + 100; uint32_t r; __asm__ volatile("lodsl" : "=a"(r), "+S"(s)); mix(r);
            s = buf2 + 300; __asm__ volatile("lodsb" : "=a"(r), "+S"(s)); mix(r & 0xff);
            s = buf2 + 301; __asm__ volatile("lodsw" : "=a"(r), "+S"(s)); mix(r & 0xffff);
            s = buf + 3; d = buf2 + 900; __asm__ volatile("movsb; movsl; movsw" : "+S"(s), "+D"(d) :: "memory");
            d = buf2 + 1000; __asm__ volatile("stosb; stosl" : "+D"(d) : "a"(ax) : "memory");
            __asm__ volatile("std; cld");
            for (int j = 0; j < 1100; j += 8) mix(*(uint64_t *)(buf2 + j));
            // page crossing: copy across the 4 KB boundary of the buffers
            s = buf + 4090; d = buf2 + 4090; c = 6; __asm__ volatile("rep movsb" : "+S"(s), "+D"(d), "+c"(c) :: "memory");
            s = buf + 4094; d = buf2 + 4094; c = 2; __asm__ volatile("rep movsl" : "+S"(s), "+D"(d), "+c"(c) :: "memory");
        }
        // 32 bit mul/imul/div/idiv
        cur = 1; { uint32_t lo, hi, f; __asm__ volatile("mull %3; pushf; pop %2" : "=a"(lo), "=d"(hi), "=r"(f) : "r"(b), "0"(a)); mix(lo); mix(hi); mix(f & 0x801); }
        { uint32_t lo, hi, f; __asm__ volatile("imull %3; pushf; pop %2" : "=a"(lo), "=d"(hi), "=r"(f) : "r"(b), "0"(a)); mix(lo); mix(hi); mix(f & 0x801); }
        { uint32_t m = V[(it * 3) % NV]; uint32_t lo, hi; __asm__ volatile("mull (%3)" : "=a"(lo), "=d"(hi) : "0"(a), "r"(&m)); mix(lo); mix(hi); }
        if (b != 0) { uint32_t q, r; uint32_t hi = a >> 8; if (hi < b) { __asm__ volatile("divl %4" : "=a"(q), "=d"(r) : "0"(a << 24 | 5), "1"(hi), "r"(b)); mix(q); mix(r); } }
        if (b != 0 && b != 0xffffffff) { int32_t q, r; int32_t hi = (int32_t)a >> 31; int32_t bs = (int32_t)b; __asm__ volatile("idivl %4" : "=a"(q), "=d"(r) : "0"(a), "1"(hi), "r"(bs)); mix(q); mix(r); }
        if (b == 0xffffffff) { int32_t q, r; int32_t av = (int32_t)(a % 1000), hi = av >> 31; int32_t m1 = -1; __asm__ volatile("idivl %4" : "=a"(q), "=d"(r) : "0"(av), "1"(hi), "r"(m1)); mix(q); mix(r); }
        // cmpxchg / xadd
        cur = 2; {
            uint32_t mem = a, acc = b, nw = V[(it + 3) % NV], f;
            __asm__ volatile("lock cmpxchgl %3, %0; pushf; pop %2" : "+m"(mem), "+a"(acc), "=r"(f) : "r"(nw) : "memory"); mix(mem); mix(acc); mix(f & 0x8d5);
            mem = a; acc = a; __asm__ volatile("lock cmpxchgl %3, %0; pushf; pop %2" : "+m"(mem), "+a"(acc), "=r"(f) : "r"(nw) : "memory"); mix(mem); mix(acc); mix(f & 0x8d5);
            uint32_t x = a, y = b; __asm__ volatile("lock xaddl %1, %0; pushf; pop %2" : "+m"(x), "+r"(y), "=r"(f) :: "memory"); mix(x); mix(y); mix(f & 0x8d5);
            uint32_t p = a, q = b; __asm__ volatile("xaddl %0, %1; pushf; pop %2" : "+r"(p), "+r"(q), "=r"(f)); mix(p); mix(q); mix(f & 0x8d5);
            uint32_t e1 = a, e2 = b; __asm__ volatile("cmpxchgl %0, %1; pushf; pop %2" : "+r"(e1), "+r"(e2), "=r"(f) : "a"(b)); mix(e1); mix(e2); mix(f & 0x8d5);
        }
    }
    for (int k = 0; k < 3; k++) printf("inttest %d checksum %016llx\n", k, (unsigned long long)hh[k]);
    return 0;
}
