/* Text measurement probe: DrawText(DT_CALCRECT), GetTextExtentPoint32 and GetTextMetrics on the dialog font. */
#include <windows.h>
#include <stdio.h>
int main(void) {
    NONCLIENTMETRICSW nc = { sizeof nc };
    BOOL ok = SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, sizeof nc, &nc, 0);
    printf("spi ok=%d msgfont height=%ld weight=%ld face=%ls\n", ok, nc.lfMessageFont.lfHeight, nc.lfMessageFont.lfWeight, nc.lfMessageFont.lfFaceName);
    HDC dc = GetDC(0);
    HFONT f = CreateFontIndirectW(&nc.lfMessageFont);
    HGDIOBJ old = SelectObject(dc, f);
    TEXTMETRICW tm; GetTextMetricsW(dc, &tm);
    printf("tm height=%ld ascent=%ld avgw=%ld\n", tm.tmHeight, tm.tmAscent, tm.tmAveCharWidth);
    SIZE sz; GetTextExtentPoint32W(dc, L"&Yes", 4, &sz);
    printf("extent &Yes = %ldx%ld\n", sz.cx, sz.cy);
    RECT r = { 0, 0, 0, 0 };
    int h = DrawTextW(dc, L"&Yes", -1, &r, DT_LEFT | DT_EXPANDTABS | DT_CALCRECT);
    printf("DrawText calc: ret=%d rect=%ld,%ld,%ld,%ld\n", h, r.left, r.top, r.right, r.bottom);
    RECT r2 = { 0, 0, 300, 0 };
    h = DrawTextW(dc, L"Direct X can not find your sound card.\nDo you wish to continue this application without sound?", -1, &r2, DT_LEFT | DT_WORDBREAK | DT_CALCRECT | DT_EXPANDTABS);
    printf("DrawText text: ret=%d rect=%ld,%ld,%ld,%ld\n", h, r2.left, r2.top, r2.right, r2.bottom);
    SelectObject(dc, old);
    return 0;
}
