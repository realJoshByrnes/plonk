#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
int main(int argc, char **argv) {
    char buf[64];
    snprintf(buf, sizeof buf, "sqrt(2)=%.4f n=%d", sqrt(2.0), (int)strlen("abc"));
    printf("CRT hello: %s\n", buf);
    void *p = malloc(100000); memset(p, 1, 100000); free(p);
    return 3;
}
