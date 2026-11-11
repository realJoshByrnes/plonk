#include <windows.h>
#include <stdio.h>
int main(void) {
    DWORD t0 = GetTickCount();
    Sleep(50);
    printf("slept %lu\n", (unsigned long)(GetTickCount() - t0));
    HANDLE ev = CreateEventA(0, TRUE, FALSE, 0);
    printf("wait=%lu\n", (unsigned long)WaitForSingleObject(ev, 10));
    return 0;
}
