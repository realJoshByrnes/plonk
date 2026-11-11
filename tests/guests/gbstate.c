/* NtGdi batch: transforms, mapping, bounds, bitmap bits/dimension, palettes, fonts, printing, misc. Prints key=value lines. */
#include <windows.h>
#include <stdio.h>

int main(void) {
    HDC screen = GetDC(0);
    HDC dc = CreateCompatibleDC(screen);
    SelectObject(dc, CreateCompatibleBitmap(screen, 16, 16));

    /* world transform */
    printf("gm=%d\n", SetGraphicsMode(dc, GM_ADVANCED));
    XFORM x = { 2, 0, 0, 3, 5, 7 }, o;
    printf("setwt=%d\n", SetWorldTransform(dc, &x));
    GetWorldTransform(dc, &o);
    printf("wt=%g,%g,%g,%g,%g,%g\n", o.eM11, o.eM12, o.eM21, o.eM22, o.eDx, o.eDy);
    XFORM t = { 1, 0, 0, 1, 10, 0 };
    printf("rightmul=%d\n", ModifyWorldTransform(dc, &t, MWT_RIGHTMULTIPLY));
    GetWorldTransform(dc, &o);
    printf("wt2=%g,%g,%g,%g,%g,%g\n", o.eM11, o.eM12, o.eM21, o.eM22, o.eDx, o.eDy);
    printf("ident=%d\n", ModifyWorldTransform(dc, NULL, MWT_IDENTITY));
    GetWorldTransform(dc, &o);
    printf("wt3=%g,%g,%g,%g,%g,%g\n", o.eM11, o.eM12, o.eM21, o.eM22, o.eDx, o.eDy);

    /* mapping */
    SetMapMode(dc, MM_ANISOTROPIC);
    SetWindowExtEx(dc, 100, 100, NULL); SetViewportExtEx(dc, 200, 200, NULL);
    SIZE sz;
    printf("scalevp=%d ", ScaleViewportExtEx(dc, 1, 2, 1, 2, &sz));
    printf("old=%ld,%ld ", sz.cx, sz.cy);
    GetViewportExtEx(dc, &sz); printf("new=%ld,%ld\n", sz.cx, sz.cy);
    printf("scalewin=%d ", ScaleWindowExtEx(dc, 2, 1, 2, 1, &sz));
    GetWindowExtEx(dc, &sz); printf("win=%ld,%ld\n", sz.cx, sz.cy);
    SetMapMode(dc, MM_TEXT);

    /* DC queries */
    POINT pt;
    MoveToEx(dc, 7, 9, NULL); GetCurrentPositionEx(dc, &pt); printf("curpos=%ld,%ld\n", pt.x, pt.y);
    SetBkColor(dc, RGB(1, 2, 3)); printf("bk=%06lx\n", (unsigned long)GetBkColor(dc));
    printf("ismem=%d\n", GetDeviceCaps(dc, TECHNOLOGY) >= 0);

    /* bounds */
    RECT r = { 1, 2, 5, 6 }, rb;
    printf("setbounds1=%u\n", SetBoundsRect(dc, NULL, DCB_ENABLE));
    printf("setbounds2=%u\n", SetBoundsRect(dc, &r, DCB_ACCUMULATE));
    printf("getbounds=%u ", GetBoundsRect(dc, &rb, DCB_RESET));
    printf("rect=%ld,%ld,%ld,%ld\n", rb.left, rb.top, rb.right, rb.bottom);
    printf("getbounds_reset=%u\n", GetBoundsRect(dc, &rb, 0));

    /* bitmap bits and dimension */
    {
        unsigned int src[8] = { 0x00112233, 0x00445566, 0x00778899, 0x00AABBCC, 1, 2, 3, 4 }, dst[8] = { 0 };
        HBITMAP bm = CreateBitmap(4, 2, 1, 32, src);
        LONG n = GetBitmapBits(bm, sizeof(dst), dst);
        printf("getbits=%ld first=%06x last=%06x size=%ld\n", n, dst[0] & 0xffffff, dst[7] & 0xffffff, GetBitmapBits(bm, 0, NULL));
        unsigned int nw[8] = { 0x00FF0000, 0, 0, 0, 0, 0, 0, 0x0000FF00 };
        printf("setbits=%ld\n", SetBitmapBits(bm, sizeof(nw), nw));
        HDC d2 = CreateCompatibleDC(screen); SelectObject(d2, bm);
        printf("setbits_px=%06lx,%06lx\n", (unsigned long)GetPixel(d2, 0, 0), (unsigned long)GetPixel(d2, 3, 1));
        SIZE prev = { -1, -1 }, cur;
        int sd = SetBitmapDimensionEx(bm, 10, 20, &prev);
        printf("setdim=%d prev=%ld,%ld ", sd, prev.cx, prev.cy);
        printf("getdim=%d ", GetBitmapDimensionEx(bm, &cur)); printf("cur=%ld,%ld\n", cur.cx, cur.cy);
    }

    /* palettes */
    {
        struct { WORD v, n; PALETTEENTRY e[4]; } lp = { 0x300, 4, { { 0, 0, 0, 0 }, { 255, 0, 0, 0 }, { 0, 255, 0, 0 }, { 0, 0, 255, 0 } } };
        HPALETTE pal = CreatePalette((LOGPALETTE *)&lp);
        PALETTEENTRY out[4];
        printf("palget=%u ", GetPaletteEntries(pal, 0, 4, out)); printf("e1=%d,%d,%d\n", out[1].peRed, out[1].peGreen, out[1].peBlue);
        PALETTEENTRY ne = { 9, 8, 7, 0 };
        printf("palset=%u ", SetPaletteEntries(pal, 2, 1, &ne));
        GetPaletteEntries(pal, 2, 1, out); printf("e2=%d,%d,%d\n", out[0].peRed, out[0].peGreen, out[0].peBlue);
        printf("palsize=%u ", GetPaletteEntries(pal, 0, 0, NULL));
        printf("resize=%d ", ResizePalette(pal, 8));
        printf("palsize2=%u nearest=%u\n", GetPaletteEntries(pal, 0, 0, NULL), GetNearestPaletteIndex(pal, RGB(0, 250, 0)));
    }

    /* fonts / text */
    {
        RASTERIZER_STATUS rs = { 0 };
        int rr = GetRasterizerCaps(&rs, sizeof(rs));
        printf("raster=%d size=%d flags=%d\n", rr, rs.nSize, rs.wFlags);
        printf("addfont=%d\n", AddFontResourceW(L"nothere.ttf"));
        DWORD cnt = 0; HANDLE fh = AddFontMemResourceEx((void *)"abcd", 4, NULL, &cnt);
        printf("memfont=%d cnt=%lu remove=%d\n", fh != NULL, cnt, RemoveFontMemResourceEx(fh));
        printf("justify=%d\n", SetTextJustification(dc, 4, 2));
        GLYPHMETRICS gm; MAT2 m2 = { { 0, 1 }, { 0, 0 }, { 0, 0 }, { 0, 1 } };
        DWORD rm = GetGlyphOutline(dc, 'A', GGO_METRICS, &gm, 0, NULL, &m2);
        DWORD rb2 = GetGlyphOutline(dc, 'A', GGO_BITMAP, &gm, 0, NULL, &m2);
        printf("glyph metrics=%d bitmap=%d box=%d adv=%d\n", rm != GDI_ERROR, rb2 != GDI_ERROR && rb2 > 0, gm.gmBlackBoxX > 0, gm.gmCellIncX > 0);
    }

    /* device */
    {
        WORD ramp[3][256];
        int gr = GetDeviceGammaRamp(screen, ramp);
        printf("gamma=%d r0=%u r255=%u\n", gr, ramp[0][0], ramp[0][255]);
        COLORADJUSTMENT ca;
        int cr = GetColorAdjustment(dc, &ca);
        printf("cadj=%d size=%u\n", cr, ca.caSize);
        printf("flush=%d\n", GdiFlush());
        DOCINFOA di = { sizeof(di), "t", NULL, NULL, 0 };
        printf("startdoc_fails=%d\n", StartDocA(dc, &di) <= 0);
        printf("enhmf=%d\n", CreateEnhMetaFileW(NULL, NULL, NULL, NULL) != NULL);
    }
    printf("done\n");
    return 0;
}
