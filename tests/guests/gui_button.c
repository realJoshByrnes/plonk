#include <windows.h>
/* A parent window with a standard BUTTON child; each click bumps a counter shown by the parent. */
static int clicks = 0;
static void itoa_(int v, char *s) { char t[12]; int n = 0, i = 0; do { t[n++] = '0' + v % 10; v /= 10; } while (v); while (n) s[i++] = t[--n]; s[i] = 0; }

static LRESULT CALLBACK WndProc(HWND h, UINT m, WPARAM w, LPARAM l) {
    switch (m) {
    case WM_CREATE:
        CreateWindowExA(0, "BUTTON", "Click me", WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON, 20, 20, 110, 28, h, (HMENU)1,
                        GetModuleHandleA(0), 0);
        return 0;
    case WM_COMMAND:
        if (LOWORD(w) == 1) { clicks++; InvalidateRect(h, 0, TRUE); }
        return 0;
    case WM_PAINT: {
        PAINTSTRUCT ps;
        char buf[32] = "clicks: ";
        HDC dc = BeginPaint(h, &ps);
        SetBkMode(dc, TRANSPARENT);
        itoa_(clicks, buf + 8);
        TextOutA(dc, 20, 70, buf, lstrlenA(buf));
        EndPaint(h, &ps);
        return 0;
    }
    case WM_DESTROY: PostQuitMessage(clicks); return 0;
    }
    return DefWindowProcA(h, m, w, l);
}
void __stdcall start(void) {
    WNDCLASSA wc = {0};
    wc.lpfnWndProc = WndProc;
    wc.hInstance = GetModuleHandleA(0);
    wc.hbrBackground = (HBRUSH)(COLOR_BTNFACE + 1);
    wc.lpszClassName = "PlonkBtn";
    RegisterClassA(&wc);
    CreateWindowExA(0, "PlonkBtn", "Button demo", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 40, 40, 300, 160, 0, 0, wc.hInstance, 0);
    MSG msg;
    while (GetMessageA(&msg, 0, 0, 0) > 0) { TranslateMessage(&msg); DispatchMessageA(&msg); }
    ExitProcess((UINT)msg.wParam);
}
