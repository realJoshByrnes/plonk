/* Interpreter benchmark: integer ALU loop, memory-heavy sieve, and float loop. Prints a checksum so nothing is optimised away. */
#include <windows.h>
#include <stdio.h>
static unsigned char sieve[4 * 1024 * 1024];
static volatile unsigned sink;
int main(void) {
    unsigned a = 1, b = 2, c = 3;
    for (unsigned i = 0; i < 40000000; i++) { a = a * 1664525u + 1013904223u; b ^= a >> 7; c += b + (a << 3); }
    sink = a ^ b ^ c;
    unsigned cnt = 0;
    for (int r = 0; r < 3; r++) {
        for (unsigned i = 0; i < sizeof sieve; i++) sieve[i] = 1;
        for (unsigned i = 2; i * i < sizeof sieve; i++) if (sieve[i]) for (unsigned j = i * i; j < sizeof sieve; j += i) sieve[j] = 0;
        for (unsigned i = 2; i < sizeof sieve; i++) cnt += sieve[i];
    }
    double x = 0.0;
    for (int i = 1; i < 6000000; i++) x += 1.0 / ((double)i * i) + (i & 1 ? 0.5 : -0.5) * 1e-9;
    printf("bench %u %u %.9f\n", sink, cnt, x);
    return 0;
}
