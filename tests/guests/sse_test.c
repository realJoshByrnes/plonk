#include <stdio.h>
#include <string.h>
#include <math.h>
#include <emmintrin.h>

/* Built with -msse2 -mfpmath=sse: float/double math and SIMD integer code all go through SSE. */
static double dot(const double *a, const double *b, int n) { double s = 0; for (int i = 0; i < n; i++) s += a[i] * b[i]; return s; }

int main(void) {
    double a[4] = {1.5, 2.5, 3.5, 4.5}, b[4] = {2, 4, 6, 8};
    printf("dot=%.2f sqrt=%.4f\n", dot(a, b, 4), sqrt(2.0));
    float f = 1.0f; for (int i = 0; i < 10; i++) f = f * 1.5f + 0.25f;
    printf("f=%.3f int=%d\n", f, (int)(f * 3.7f));
    __m128i x = _mm_set_epi32(4, 3, 2, 1), y = _mm_set1_epi32(10);
    __m128i s = _mm_add_epi32(x, y);
    int out[4]; _mm_storeu_si128((__m128i *)out, s);
    printf("simd=%d,%d,%d,%d\n", out[0], out[1], out[2], out[3]);
    __m128d d = _mm_mul_pd(_mm_set_pd(3.0, 2.0), _mm_set1_pd(0.5));
    double dd[2]; _mm_storeu_pd(dd, d);
    printf("pd=%.1f,%.1f\n", dd[0], dd[1]);
    char buf[32]; memset(buf, 'x', 31); buf[31] = 0;
    printf("len=%d\n", (int)strlen(buf));
    return 5;
}
