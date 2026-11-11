#include <windows.h>
#include <stdio.h>
#include <string.h>

/* Kernel-side services: drives, volume info, tokens, timers, handles, directory enumeration, file ops, modules. */
int main(void) {
    printf("drives=%lx\n", (unsigned long)GetLogicalDrives());
    char vol[64] = {0}, fs[16] = {0}; DWORD serial = 0, maxc = 0, fl = 0;
    printf("vol=%d fs=%s\n", GetVolumeInformationA("C:\\", vol, 64, &serial, &maxc, &fl, fs, 16), fs);
    ULARGE_INTEGER fb, tb, tf; printf("disk=%d\n", GetDiskFreeSpaceExA("C:\\", &fb, &tb, &tf));
    printf("type=%u\n", GetDriveTypeA("C:\\"));
    HANDLE tok = 0; printf("token=%d\n", OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &tok));
    DWORD el = 0, rl = 0; if (tok) { GetTokenInformation(tok, TokenElevation, &el, 4, &rl); CloseHandle(tok); }
    HANDLE t = CreateWaitableTimerA(0, FALSE, 0); LARGE_INTEGER due; due.QuadPart = -100000; SetWaitableTimer(t, &due, 0, 0, 0, 0);
    printf("timer=%lu\n", (unsigned long)WaitForSingleObject(t, 2000));
    HANDLE d; printf("dup=%d\n", DuplicateHandle(GetCurrentProcess(), GetCurrentProcess(), GetCurrentProcess(), &d, 0, FALSE, DUPLICATE_SAME_ACCESS));
    printf("mkdir=%d\n", CreateDirectoryA("C:\\data", 0));
    HANDLE f = CreateFileA("C:\\data\\a.txt", GENERIC_WRITE, 0, 0, CREATE_NEW, 0, 0); WriteFile(f, "x", 1, &fl, 0); CloseHandle(f);
    printf("copy=%d\n", CopyFileA("C:\\data\\a.txt", "C:\\data\\b.txt", FALSE));
    printf("move=%d\n", MoveFileA("C:\\data\\b.txt", "C:\\data\\c.txt"));
    WIN32_FIND_DATAA fd; HANDLE h = FindFirstFileA("C:\\data\\*", &fd); int n = 0;
    if (h != INVALID_HANDLE_VALUE) { do { if (strcmp(fd.cFileName, ".") && strcmp(fd.cFileName, "..")) n++; } while (FindNextFileA(h, &fd)); FindClose(h); }
    printf("entries=%d\n", n);
    printf("delete=%d\n", DeleteFileA("C:\\data\\a.txt"));
    char tmp[MAX_PATH]; GetTempPathA(MAX_PATH, tmp); printf("temp=%s\n", tmp);
    HMODULE m = LoadLibraryA("user32.dll"); printf("lib=%d proc=%d\n", m != 0, GetProcAddress(m, "MessageBoxA") != 0);
    char mp[MAX_PATH]; GetModuleFileNameA(m, mp, MAX_PATH); printf("modpath=%s\n", mp);
    SYSTEM_INFO si; GetSystemInfo(&si); printf("cpus=%lu page=%lu\n", (unsigned long)si.dwNumberOfProcessors, (unsigned long)si.dwPageSize);
    MEMORYSTATUS ms; GlobalMemoryStatus(&ms); printf("mem>0:%d\n", ms.dwTotalPhys > 0);
    OSVERSIONINFOA vi = { sizeof vi }; printf("osver=%d\n", GetVersionExA(&vi));
    char cn[64]; DWORD cl = 64; printf("computer=%d\n", GetComputerNameA(cn, &cl));
    printf("lang=%x\n", GetUserDefaultLangID());
    printf("cp=%u\n", GetACP());
    WCHAR wb[16]; printf("mb2wc=%d\n", MultiByteToWideChar(CP_ACP, 0, "hello", -1, wb, 16));
    printf("cmp=%d\n", CompareStringA(LOCALE_USER_DEFAULT, NORM_IGNORECASE, "abc", -1, "ABC", -1));
    char dt[32]; printf("date=%d\n", GetDateFormatA(LOCALE_USER_DEFAULT, 0, 0, "yyyy", dt, 32) > 0);
    return 0;
}
