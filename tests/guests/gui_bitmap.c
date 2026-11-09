#include <windows.h>
/* Double-buffered drawing: memory DC + compatible bitmap, BitBlt/StretchBlt, DIB section + AlphaBlend. */
static LRESULT CALLBACK WndProc(HWND h, UINT m, WPARAM w, LPARAM l) {
    if (m == WM_PAINT) {
        PAINTSTRUCT ps;
        HDC dc = BeginPaint(h, &ps);
        HDC mem = CreateCompatibleDC(dc);
        HBITMAP bmp = CreateCompatibleBitmap(dc, 100, 60);
        HGDIOBJ old = SelectObject(mem, bmp);
        HBRUSH blue = CreateSolidBrush(RGB(20, 60, 200));
        RECT all = {0, 0, 100, 60};
        FillRect(mem, &all, blue);
        TextOutA(mem, 4, 4, "offscreen", 9);
        BitBlt(dc, 10, 10, 100, 60, mem, 0, 0, SRCCOPY);
        StretchBlt(dc, 120, 10, 50, 30, mem, 0, 0, 100, 60, SRCCOPY);
        SelectObject(mem, old);
        DeleteObject(bmp);
        DeleteObject(blue);
        DeleteDC(mem);

        BITMAPINFO bi = {0};
        bi.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
        bi.bmiHeader.biWidth = 64;
        bi.bmiHeader.biHeight = -32;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;
        unsigned *bits = 0;
        HBITMAP dib = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, (void **)&bits, 0, 0);
        for (int i = 0; i < 64 * 32; i++) bits[i] = 0x80008000; /* premultiplied: a=128, g=128 */
        HDC src = CreateCompatibleDC(dc);
        SelectObject(src, dib);
        BLENDFUNCTION bf = {AC_SRC_OVER, 0, 255, AC_SRC_ALPHA};
        AlphaBlend(dc, 10, 90, 64, 32, src, 0, 0, 64, 32, bf);
        DeleteDC(src);
        DeleteObject(dib);
        EndPaint(h, &ps);
        PostMessageA(h, WM_CLOSE, 0, 0);
        return 0;
    }
    if (m == WM_CLOSE) { DestroyWindow(h); return 0; }
    if (m == WM_DESTROY) { PostQuitMessage(0); return 0; }
    return DefWindowProcA(h, m, w, l);
}
void __stdcall start(void) {
    WNDCLASSA wc = {0};
    wc.lpfnWndProc = WndProc;
    wc.hInstance = GetModuleHandleA(0);
    wc.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    wc.lpszClassName = "PlonkBmp";
    RegisterClassA(&wc);
    CreateWindowExA(0, "PlonkBmp", "Bitmaps", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 10, 10, 320, 240, 0, 0, wc.hInstance, 0);
    MSG msg;
    while (GetMessageA(&msg, 0, 0, 0) > 0) { TranslateMessage(&msg); DispatchMessageA(&msg); }
    ExitProcess(0);
}
