#include <windows.h>
#include <stdio.h>
int main(void) {
    HANDLE f = CreateFileA("C:\\sink.txt", GENERIC_WRITE, 0, 0, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    printf("create=%p err=%lu\n", f, GetLastError());
    char cwd[MAX_PATH]; DWORD n = GetCurrentDirectoryA(MAX_PATH, cwd);
    printf("cwd=%s (%lu)\n", cwd, n);
    char full[MAX_PATH]; n = GetFullPathNameA("sink.txt", MAX_PATH, full, 0);
    printf("full=%s (%lu)\n", full, n);
    printf("attrs=%lx err=%lu\n", GetFileAttributesA("C:\\windows"), GetLastError());
    return 0;
}
