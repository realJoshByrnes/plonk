#include <windows.h>
#include <stdio.h>
#include <string.h>

/* Touches the API families real programs use. Results go to stdout; the window paints text with several fonts. */
static HINSTANCE inst;
static CRITICAL_SECTION cs;
static DWORD tls;

static LRESULT CALLBACK WndProc(HWND h, UINT m, WPARAM w, LPARAM l) {
    switch (m) {
    case WM_PAINT: {
        PAINTSTRUCT ps; RECT r; HDC dc = BeginPaint(h, &ps);
        GetClientRect(h, &r);
        HFONT f = CreateFontA(-16, 0, 0, 0, FW_BOLD, 0, 0, 0, DEFAULT_CHARSET, 0, 0, 0, 0, "Arial");
        HGDIOBJ old = SelectObject(dc, f);
        SetTextColor(dc, RGB(0, 0, 160)); SetBkMode(dc, TRANSPARENT);
        DrawTextA(dc, "Kitchen sink", -1, &r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        SelectObject(dc, old); DeleteObject(f);
        HPEN pen = CreatePen(PS_SOLID, 2, RGB(200, 0, 0)); old = SelectObject(dc, pen);
        MoveToEx(dc, 5, 5, 0); LineTo(dc, 100, 40); Rectangle(dc, 10, 50, 60, 90); Ellipse(dc, 70, 50, 120, 90);
        SelectObject(dc, old); DeleteObject(pen);
        EndPaint(h, &ps);
        return 0;
    }
    case WM_TIMER: KillTimer(h, 1); PostMessageA(h, WM_CLOSE, 0, 0); return 0;
    case WM_CLOSE: DestroyWindow(h); return 0;
    case WM_DESTROY: PostQuitMessage(0); return 0;
    }
    return DefWindowProcA(h, m, w, l);
}

int main(void) {
    inst = GetModuleHandleA(0);
    InitializeCriticalSection(&cs); EnterCriticalSection(&cs); LeaveCriticalSection(&cs);
    tls = TlsAlloc(); TlsSetValue(tls, (void *)42);
    printf("tls=%d\n", (int)(INT_PTR)TlsGetValue(tls));

    char path[MAX_PATH]; GetModuleFileNameA(0, path, MAX_PATH);
    printf("exe=%s cmd=%s\n", path, GetCommandLineA());
    char env[64]; DWORD n = GetEnvironmentVariableA("SystemRoot", env, sizeof env); printf("env=%s (%lu)\n", env, n);
    SYSTEMTIME st; GetLocalTime(&st); printf("year>=2000:%d\n", st.wYear >= 2000);
    HANDLE heap = GetProcessHeap(); void *p = HeapAlloc(heap, HEAP_ZERO_MEMORY, 1000); HeapFree(heap, 0, p);
    printf("sm=%d,%d dpi-ish=%d\n", GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN), GetSystemMetrics(SM_CYCAPTION));
    printf("color=%06lx\n", (unsigned long)GetSysColor(COLOR_BTNFACE));
    printf("ver=%lx\n", GetVersion() & 0xffff);
    printf("tick>0:%d\n", GetTickCount() >= 0);
    LARGE_INTEGER q; QueryPerformanceCounter(&q); QueryPerformanceFrequency(&q); printf("qpf>0:%d\n", q.QuadPart > 0);

    HANDLE f = CreateFileA("C:\\sink.txt", GENERIC_WRITE, 0, 0, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    DWORD wr = 0; if (f != INVALID_HANDLE_VALUE) { WriteFile(f, "hello file", 10, &wr, 0); CloseHandle(f); }
    f = CreateFileA("C:\\sink.txt", GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0);
    char rb[32] = {0}; DWORD rd = 0; if (f != INVALID_HANDLE_VALUE) { ReadFile(f, rb, 31, &rd, 0); printf("file=%s size=%lu\n", rb, GetFileSize(f, 0)); CloseHandle(f); } else printf("file: open failed\n");
    printf("attrs=%lx\n", GetFileAttributesA("C:\\sink.txt"));

    HKEY k; if (RegCreateKeyExA(HKEY_CURRENT_USER, "Software\\Plonk", 0, 0, 0, KEY_ALL_ACCESS, 0, &k, 0) == 0) {
        DWORD v = 7; RegSetValueExA(k, "n", 0, REG_DWORD, (BYTE *)&v, 4);
        DWORD got = 0, sz = 4, ty; RegQueryValueExA(k, "n", 0, &ty, (BYTE *)&got, &sz); printf("reg=%lu\n", got); RegCloseKey(k);
    }

    WNDCLASSA wc = {0}; wc.lpfnWndProc = WndProc; wc.hInstance = inst; wc.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    wc.hCursor = LoadCursorA(0, IDC_ARROW); wc.hIcon = LoadIconA(0, IDI_APPLICATION); wc.lpszClassName = "Sink";
    RegisterClassA(&wc);
    HWND h = CreateWindowExA(0, "Sink", "Kitchen sink", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 20, 20, 300, 200, 0, 0, inst, 0);
    ShowWindow(h, SW_SHOW); UpdateWindow(h);
    SetTimer(h, 1, 200, 0);
    MSG msg; while (GetMessageA(&msg, 0, 0, 0) > 0) { TranslateMessage(&msg); DispatchMessageA(&msg); }
    printf("done\n");
    return 0;
}
