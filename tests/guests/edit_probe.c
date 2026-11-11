#include <windows.h>
#include <stdio.h>
static void show(const char *tag, HWND e) { char b[64] = {0}; int n = GetWindowTextA(e, b, 64); printf("%s: len=%d '%s' (WM_GETTEXTLENGTH=%d)\n", tag, n, b, (int)SendMessageA(e, WM_GETTEXTLENGTH, 0, 0)); }
static LRESULT CALLBACK WndProc(HWND h, UINT m, WPARAM w, LPARAM l) { return DefWindowProcA(h, m, w, l); }
int main(void) {
    WNDCLASSA wc = {0}; wc.lpfnWndProc = WndProc; wc.hInstance = GetModuleHandleA(0); wc.lpszClassName = "P"; RegisterClassA(&wc);
    HWND p = CreateWindowExA(0, "P", "p", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 0, 0, 300, 200, 0, 0, wc.hInstance, 0);
    HWND e1 = CreateWindowExA(0, "EDIT", "hello", WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL, 5, 5, 200, 20, p, 0, wc.hInstance, 0);
    show("created", e1);
    SetWindowTextA(e1, "world!"); show("settext", e1);
    SendMessageA(e1, EM_SETSEL, 0, -1); SendMessageA(e1, EM_REPLACESEL, 0, (LPARAM)"replaced"); show("replacesel", e1);
    HWND e2 = CreateWindowExA(0, "EDIT", "pw", WS_CHILD | WS_VISIBLE | ES_PASSWORD, 5, 30, 200, 20, p, 0, wc.hInstance, 0);
    show("password", e2);
    HWND e3 = CreateWindowExA(0, "EDIT", "this is a longer text", WS_CHILD | WS_VISIBLE | ES_MULTILINE, 5, 55, 200, 60, p, 0, wc.hInstance, 0);
    show("multiline", e3);
    return 0;
}
