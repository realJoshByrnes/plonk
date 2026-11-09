#include <windows.h>
/* Menu bar with a popup: File > Say hi (id 11), Quit (id 12). Exit code = id of the last command chosen. */
static int last_cmd = 0;
static LRESULT CALLBACK WndProc(HWND h, UINT m, WPARAM w, LPARAM l) {
    switch (m) {
    case WM_COMMAND: last_cmd = LOWORD(w); if (LOWORD(w) == 12) DestroyWindow(h); return 0;
    case WM_DESTROY: PostQuitMessage(last_cmd); return 0;
    }
    return DefWindowProcA(h, m, w, l);
}
void __stdcall start(void) {
    WNDCLASSA wc = {0};
    wc.lpfnWndProc = WndProc;
    wc.hInstance = GetModuleHandleA(0);
    wc.hbrBackground = (HBRUSH)(COLOR_BTNFACE + 1);
    wc.lpszClassName = "PlonkMenu";
    RegisterClassA(&wc);
    HMENU bar = CreateMenu(), file = CreatePopupMenu();
    AppendMenuA(file, MF_STRING, 11, "Say hi");
    AppendMenuA(file, MF_SEPARATOR, 0, 0);
    AppendMenuA(file, MF_STRING, 12, "Quit");
    AppendMenuA(bar, MF_POPUP, (UINT_PTR)file, "File");
    CreateWindowExA(0, "PlonkMenu", "Menu demo", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 40, 40, 300, 160, 0, bar, wc.hInstance, 0);
    MSG msg;
    while (GetMessageA(&msg, 0, 0, 0) > 0) { TranslateMessage(&msg); DispatchMessageA(&msg); }
    ExitProcess((UINT)msg.wParam);
}
