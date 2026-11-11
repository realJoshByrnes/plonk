/* Loads a DLL by path (argv[1] or the default below) and reports LoadLibrary/GetProcAddress results. Debugging aid for installers. */
#include <windows.h>
#include <stdio.h>
int main(int argc, char **argv) {
    const char *path = argc > 1 ? argv[1] : "c:\\windows\\~glc0000.tmp";
    SetLastError(0);
    HMODULE h = LoadLibraryA(path);
    printf("LoadLibrary(%s) = %p err=%lu\n", path, (void *)h, GetLastError());
    if (h) {
        const char *names[] = { "Install", "HandleFtp", "FileWrite" };
        for (int i = 0; i < 3; i++) printf("  GetProcAddress(%s) = %p\n", names[i], (void *)GetProcAddress(h, names[i]));
    }
    return 0;
}
