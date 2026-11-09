#include <windows.h>
/* Two top-level windows from one process: drag them by the title bar, click to raise. */
static int open_windows = 0;

static LRESULT paint(HWND h, COLORREF c, const char *label) {
    PAINTSTRUCT ps;
    HDC dc = BeginPaint(h, &ps);
    HBRUSH b = CreateSolidBrush(c);
    RECT r = {10, 10, 150, 60};
    FillRect(dc, &r, b);
    DeleteObject(b);
    TextOutA(dc, 12, 80, label, lstrlenA(label));
    EndPaint(h, &ps);
    return 0;
}
static LRESULT CALLBACK ProcA(HWND h, UINT m, WPARAM w, LPARAM l) {
    if (m == WM_PAINT) return paint(h, RGB(40, 120, 220), "Window A");
    if (m == WM_CLOSE) { DestroyWindow(h); return 0; }
    if (m == WM_DESTROY) { if (--open_windows == 0) PostQuitMessage(0); return 0; }
    return DefWindowProcA(h, m, w, l);
}
static LRESULT CALLBACK ProcB(HWND h, UINT m, WPARAM w, LPARAM l) {
    if (m == WM_PAINT) return paint(h, RGB(60, 180, 90), "Window B");
    if (m == WM_CLOSE) { DestroyWindow(h); return 0; }
    if (m == WM_DESTROY) { if (--open_windows == 0) PostQuitMessage(0); return 0; }
    return DefWindowProcA(h, m, w, l);
}
void __stdcall start(void) {
    HINSTANCE inst = GetModuleHandleA(0);
    WNDCLASSA a = {0}, b = {0};
    a.lpfnWndProc = ProcA; a.hInstance = inst; a.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1); a.lpszClassName = "PlonkA";
    b.lpfnWndProc = ProcB; b.hInstance = inst; b.hbrBackground = (HBRUSH)(COLOR_BTNFACE + 1); b.lpszClassName = "PlonkB";
    RegisterClassA(&a);
    RegisterClassA(&b);
    CreateWindowExA(0, "PlonkA", "Window A", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 40, 40, 240, 160, 0, 0, inst, 0);
    open_windows++;
    CreateWindowExA(0, "PlonkB", "Window B", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 140, 100, 240, 160, 0, 0, inst, 0);
    open_windows++;
    MSG msg;
    while (GetMessageA(&msg, 0, 0, 0) > 0) { TranslateMessage(&msg); DispatchMessageA(&msg); }
    ExitProcess(0);
}
