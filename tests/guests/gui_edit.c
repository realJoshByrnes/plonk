#include <windows.h>
/* A window with an EDIT control; on close the exit code is the text length typed into it. */
static HWND edit;
static LRESULT CALLBACK WndProc(HWND h, UINT m, WPARAM w, LPARAM l) {
    switch (m) {
    case WM_CREATE:
        edit = CreateWindowExA(0, "EDIT", "", WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL, 20, 20, 200, 22, h, (HMENU)1, GetModuleHandleA(0), 0);
        return 0;
    case WM_CLOSE: {
        char buf[64];
        int n = GetWindowTextA(edit, buf, sizeof buf);
        DestroyWindow(h);
        PostQuitMessage(n);
        return 0;
    }
    case WM_DESTROY: return 0;
    }
    return DefWindowProcA(h, m, w, l);
}
void __stdcall start(void) {
    WNDCLASSA wc = {0};
    wc.lpfnWndProc = WndProc;
    wc.hInstance = GetModuleHandleA(0);
    wc.hbrBackground = (HBRUSH)(COLOR_BTNFACE + 1);
    wc.lpszClassName = "PlonkEdit";
    RegisterClassA(&wc);
    CreateWindowExA(0, "PlonkEdit", "Edit demo", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 40, 40, 300, 140, 0, 0, wc.hInstance, 0);
    MSG msg;
    while (GetMessageA(&msg, 0, 0, 0) > 0) { TranslateMessage(&msg); DispatchMessageA(&msg); }
    ExitProcess((UINT)msg.wParam);
}
