/* DIB orientation probe. Image: TOP row red, BOTTOM row green. Prints the pixel colours GDI produces at the top/bottom of a target bitmap
 * for StretchDIBits and SetDIBitsToDevice, with a bottom-up DIB (positive height) and a top-down DIB (negative height), plus a DIB section. */
#include <windows.h>
#include <stdio.h>

#define W 4
#define H 4
static unsigned int up[W * H];    /* bottom-up: memory row 0 = bottom row */
static unsigned int down[W * H];  /* top-down:  memory row 0 = top row    */

static const char *name(COLORREF c) { return c == RGB(255, 0, 0) ? "red" : c == RGB(0, 200, 0) ? "green" : c == RGB(0, 0, 0) ? "black" : "other"; }

static void report(const char *what, HDC dc) {
    printf("%-34s top=%-5s bottom=%-5s\n", what, name(GetPixel(dc, 1, 0)), name(GetPixel(dc, 1, H - 1)));
}

int main(void) {
    for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) {
        unsigned int c = y == 0 ? 0x00FF0000 : y == H - 1 ? 0x0000C800 : 0x00808080; /* image row y from the top */
        down[y * W + x] = c;
        up[(H - 1 - y) * W + x] = c;
    }
    HDC screen = GetDC(0), dc = CreateCompatibleDC(screen);
    HBITMAP bm = CreateCompatibleBitmap(screen, W, H);
    SelectObject(dc, bm);
    BITMAPINFO bu = { { sizeof(BITMAPINFOHEADER), W, H, 1, 32, BI_RGB } };
    BITMAPINFO bd = { { sizeof(BITMAPINFOHEADER), W, -H, 1, 32, BI_RGB } };

    PatBlt(dc, 0, 0, W, H, BLACKNESS);
    StretchDIBits(dc, 0, 0, W, H, 0, 0, W, H, up, &bu, DIB_RGB_COLORS, SRCCOPY);
    report("StretchDIBits bottom-up", dc);
    PatBlt(dc, 0, 0, W, H, BLACKNESS);
    StretchDIBits(dc, 0, 0, W, H, 0, 0, W, H, down, &bd, DIB_RGB_COLORS, SRCCOPY);
    report("StretchDIBits top-down", dc);
    PatBlt(dc, 0, 0, W, H, BLACKNESS);
    SetDIBitsToDevice(dc, 0, 0, W, H, 0, 0, 0, H, up, &bu, DIB_RGB_COLORS);
    report("SetDIBitsToDevice bottom-up", dc);
    PatBlt(dc, 0, 0, W, H, BLACKNESS);
    SetDIBitsToDevice(dc, 0, 0, W, H, 0, 0, 0, H, down, &bd, DIB_RGB_COLORS);
    report("SetDIBitsToDevice top-down", dc);
    /* partial source rectangle: only the bottom two image rows of the bottom-up DIB (ySrc counts from the bottom) */
    PatBlt(dc, 0, 0, W, H, BLACKNESS);
    StretchDIBits(dc, 0, 0, W, 2, 0, 0, W, 2, up, &bu, DIB_RGB_COLORS, SRCCOPY);
    printf("%-34s row0=%-5s row1=%-5s\n", "Stretch bottom-up src rows 0..1", name(GetPixel(dc, 1, 0)), name(GetPixel(dc, 1, 1)));

    /* DIB section: bits[] as written by the app (bottom-up) must read back with the top row red */
    void *bits = 0;
    HBITMAP ds = CreateDIBSection(screen, &bu, DIB_RGB_COLORS, &bits, 0, 0);
    memcpy(bits, up, sizeof up);
    HDC dc2 = CreateCompatibleDC(screen);
    SelectObject(dc2, ds);
    report("CreateDIBSection bottom-up", dc2);
    BitBlt(dc, 0, 0, W, H, dc2, 0, 0, SRCCOPY);
    report("BitBlt from DIB section", dc);
    return 0;
}
