#include <windows.h>
static LRESULT CALLBACK WndProc(HWND h, UINT m, WPARAM w, LPARAM l) {
    switch (m) {
    case WM_PAINT: {
        PAINTSTRUCT ps;
        HDC dc = BeginPaint(h, &ps);
        HBRUSH b = CreateSolidBrush(RGB(200, 30, 30));
        RECT r = {20, 20, 180, 100};
        FillRect(dc, &r, b);
        DeleteObject(b);
        TextOutA(dc, 30, 120, "Hello GUI", 9);
        EndPaint(h, &ps);
        PostMessageA(h, WM_CLOSE, 0, 0);
        return 0;
    }
    case WM_CLOSE: DestroyWindow(h); return 0;
    case WM_DESTROY: PostQuitMessage(7); return 0;
    }
    return DefWindowProcA(h, m, w, l);
}
void __stdcall start(void) {
    WNDCLASSA wc = {0};
    wc.lpfnWndProc = WndProc;
    wc.hInstance = GetModuleHandleA(0);
    wc.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    wc.lpszClassName = "PlonkWnd";
    RegisterClassA(&wc);
    HWND h = CreateWindowExA(0, "PlonkWnd", "Plonk", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 10, 10, 320, 240, 0, 0, wc.hInstance, 0);
    MSG msg;
    while (GetMessageA(&msg, 0, 0, 0) > 0) { TranslateMessage(&msg); DispatchMessageA(&msg); }
    ExitProcess((UINT)msg.wParam);
}
