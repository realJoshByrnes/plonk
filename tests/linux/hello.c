#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(int argc, char **argv) {
    char *p = malloc(100000);
    memset(p, 'x', 100000);
    printf("hello from linux %s %d %c %.2f\n", argv[0], argc, p[99999], 3.14159);
    return 42;
}
