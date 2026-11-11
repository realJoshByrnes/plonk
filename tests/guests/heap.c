#include <windows.h>
#include <stdio.h>
int main(void) {
    HLOCAL h = LocalAlloc(LMEM_MOVEABLE | LMEM_ZEROINIT, 16);
    printf("alloc=%d size=%lu\n", h != 0, (unsigned long)LocalSize(h));
    HLOCAL n = LocalReAlloc(h, 64, LMEM_MOVEABLE | LMEM_ZEROINIT);
    printf("realloc=%d size=%lu err=%lu\n", n != 0, n ? (unsigned long)LocalSize(n) : 0, (unsigned long)GetLastError());
    void *p = HeapAlloc(GetProcessHeap(), 0, 16); void *q = HeapReAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, p, 256);
    printf("heaprealloc=%d\n", q != 0);
    return 0;
}
