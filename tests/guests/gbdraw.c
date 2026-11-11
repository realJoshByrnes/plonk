/* NtGdi batch: paths, flood fill, regions, blits, gradients, DIB bitmaps/brushes. Prints key=value lines. */
#include <windows.h>
#include <stdio.h>

#define N 32
static HDC dc;
static void clear(void) { PatBlt(dc, 0, 0, N, N, WHITENESS); SelectClipRgn(dc, NULL); }
static void px(const char *k, int x, int y) { printf("%s=%06lx\n", k, (unsigned long)GetPixel(dc, x, y)); }

int main(void) {
    HDC screen = GetDC(0);
    dc = CreateCompatibleDC(screen);
    SelectObject(dc, CreateCompatibleBitmap(screen, N, N));
    HBRUSH red = CreateSolidBrush(RGB(255, 0, 0)), green = CreateSolidBrush(RGB(0, 255, 0));
    HPEN blue = CreatePen(PS_SOLID, 1, RGB(0, 0, 255)), rpen = CreatePen(PS_SOLID, 1, RGB(255, 0, 0));

    /* FillPath of a rectangle */
    clear();
    SelectObject(dc, red);
    BeginPath(dc); Rectangle(dc, 2, 2, 8, 8); printf("endpath=%d\n", EndPath(dc));
    printf("fillpath=%d\n", FillPath(dc));
    px("fill_in", 4, 4); px("fill_out", 9, 9);
    printf("fillpath_again=%d\n", FillPath(dc));

    /* StrokePath of a closed triangle */
    clear();
    SelectObject(dc, blue);
    BeginPath(dc); MoveToEx(dc, 10, 10, NULL); LineTo(dc, 20, 10); LineTo(dc, 20, 20); CloseFigure(dc); EndPath(dc);
    printf("strokepath=%d\n", StrokePath(dc));
    px("stroke_on", 15, 10); px("stroke_off", 15, 5);

    /* GetPath */
    BeginPath(dc); MoveToEx(dc, 1, 1, NULL); LineTo(dc, 5, 1); LineTo(dc, 5, 5); EndPath(dc);
    POINT pts[8]; BYTE ty[8];
    int n = GetPath(dc, NULL, NULL, 0);
    int m = GetPath(dc, pts, ty, 8);
    printf("getpath n=%d m=%d t=%d,%d,%d p1=%ld,%ld\n", n, m, ty[0], ty[1], ty[2], pts[1].x, pts[1].y);
    AbortPath(dc);
    printf("getpath_after_abort=%d\n", GetPath(dc, NULL, NULL, 0));

    /* PathToRegion + SelectClipPath */
    BeginPath(dc); Rectangle(dc, 2, 3, 8, 9); EndPath(dc);
    HRGN pr = PathToRegion(dc); RECT rc; GetRgnBox(pr, &rc);
    printf("pathrgn=%ld,%ld,%ld,%ld\n", rc.left, rc.top, rc.right, rc.bottom);
    clear();
    BeginPath(dc); Rectangle(dc, 0, 0, 16, 16); EndPath(dc);
    printf("clippath=%d\n", SelectClipPath(dc, RGN_COPY));
    SelectObject(dc, red); PatBlt(dc, 0, 0, N, N, PATCOPY);
    px("clip_in", 5, 5); px("clip_out", 20, 20);

    /* PolyDraw */
    clear(); SelectObject(dc, rpen);
    POINT pd[2] = { { 2, 20 }, { 12, 20 } }; BYTE pt[2] = { PT_MOVETO, PT_LINETO };
    printf("polydraw=%d\n", PolyDraw(dc, pd, pt, 2));
    px("polydraw_px", 7, 20);

    /* ExtFloodFill */
    clear();
    SelectObject(dc, GetStockObject(NULL_BRUSH)); SelectObject(dc, GetStockObject(BLACK_PEN));
    Rectangle(dc, 2, 2, 12, 12);
    SelectObject(dc, green);
    printf("flood=%d\n", ExtFloodFill(dc, 5, 5, RGB(0, 0, 0), FLOODFILLBORDER));
    px("flood_in", 5, 5); px("flood_out", 0, 0);

    /* FrameRgn / InvertRgn */
    clear();
    HRGN r = CreateRectRgn(4, 4, 12, 12);
    FrameRgn(dc, r, red, 1, 1);
    px("frame_edge", 4, 4); px("frame_mid", 8, 8);
    InvertRgn(dc, r);
    px("invert_mid", 8, 8);
    {
        HRGN e = CreateEllipticRgn(0, 0, 10, 10);
        printf("ellrgn=%d\n", e != NULL);
        HRGN q = CreateRectRgn(1, 2, 5, 7);
        DWORD sz = GetRegionData(q, 0, NULL);
        RGNDATA *rd = (RGNDATA *)malloc(sz);
        DWORD got = GetRegionData(q, sz, rd);
        printf("rgndata size=%lu got=%lu count=%lu rect=%ld,%ld,%ld,%ld\n", sz, got, rd->rdh.nCount,
               ((RECT *)rd->Buffer)->left, ((RECT *)rd->Buffer)->top, ((RECT *)rd->Buffer)->right, ((RECT *)rd->Buffer)->bottom);
    }

    /* GradientFill */
    clear();
    TRIVERTEX v[2] = { { 0, 0, 0, 0, 0, 0 }, { N, 8, 0xff00, 0xff00, 0xff00, 0 } };
    GRADIENT_RECT gr = { 0, 1 };
    printf("gradient=%d\n", GradientFill(dc, v, 2, &gr, 1, GRADIENT_FILL_RECT_H));
    printf("grad_left=%lu grad_right=%lu\n", (unsigned long)(GetPixel(dc, 1, 4) & 0xff), (unsigned long)(GetPixel(dc, 30, 4) & 0xff));

    /* MaskBlt / TransparentBlt / PlgBlt */
    HDC src = CreateCompatibleDC(screen);
    SelectObject(src, CreateCompatibleBitmap(screen, 8, 8));
    SelectObject(src, red); PatBlt(src, 0, 0, 8, 8, PATCOPY);
    BYTE mbits[16] = { 0 }; for (int i = 0; i < 8; i++) mbits[i * 2] = 0xF0;
    HBITMAP mask = CreateBitmap(8, 8, 1, 1, mbits);
    clear();
    printf("maskblt=%d\n", MaskBlt(dc, 0, 0, 8, 8, src, 0, 0, mask, 0, 0, MAKEROP4(SRCCOPY, 0x00AA0029)));
    px("mask_set", 1, 1); px("mask_clear", 6, 1);
    SelectObject(src, green); PatBlt(src, 0, 0, 4, 8, PATCOPY);
    SelectObject(src, CreateSolidBrush(RGB(255, 0, 255))); PatBlt(src, 4, 0, 4, 8, PATCOPY);
    clear();
    printf("transparent=%d\n", TransparentBlt(dc, 0, 0, 8, 8, src, 0, 0, 8, 8, RGB(255, 0, 255)));
    px("tb_opaque", 1, 1); px("tb_key", 6, 1);
    clear();
    POINT par[3] = { { 0, 0 }, { 8, 0 }, { 0, 8 } };
    printf("plgblt=%d\n", PlgBlt(dc, par, src, 0, 0, 8, 8, NULL, 0, 0));
    px("plg_left", 1, 1); px("plg_right", 6, 1);

    /* CreateDIBitmap with init data (bottom-up: first row is the bottom) */
    {
        BITMAPINFO bi = { { sizeof(BITMAPINFOHEADER), 2, 2, 1, 32, BI_RGB } };
        unsigned int bits[4] = { 0x000000FF, 0x000000FF, 0x00FF0000, 0x00FF0000 }; /* bottom blue, top red */
        HBITMAP hb = CreateDIBitmap(screen, &bi.bmiHeader, CBM_INIT, bits, &bi, DIB_RGB_COLORS);
        HDC d2 = CreateCompatibleDC(screen); SelectObject(d2, hb);
        printf("dibitmap top=%06lx bottom=%06lx\n", (unsigned long)GetPixel(d2, 0, 0), (unsigned long)GetPixel(d2, 0, 1));
    }
    /* DIB pattern brush (solid green stands in for the pattern) */
    {
        struct { BITMAPINFOHEADER h; unsigned int px[4]; } pk = { { sizeof(BITMAPINFOHEADER), 2, 2, 1, 32, BI_RGB }, { 0x0000FF00, 0x0000FF00, 0x0000FF00, 0x0000FF00 } };
        HBRUSH pb = CreateDIBPatternBrushPt(&pk, DIB_RGB_COLORS);
        clear(); RECT fr = { 0, 0, 8, 8 }; FillRect(dc, &fr, pb);
        printf("dibbrush=%d ", pb != NULL); px("dibbrush_px", 3, 3);
        HBRUSH hb = CreateHatchBrush(HS_CROSS, RGB(0, 0, 255));
        printf("hatch=%d\n", hb != NULL);
    }
    printf("nearest=%06lx\n", (unsigned long)GetNearestColor(dc, RGB(1, 2, 3)));
    printf("done\n");
    return 0;
}
