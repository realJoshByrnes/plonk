/* Probes the window-manager / class / accelerator / icon / caret / input calls of user_batch.rs. */
#include <windows.h>
#include <stdio.h>

static int cmds, syscmds, leaves;
static WPARAM lastw;

static LRESULT CALLBACK wp(HWND h, UINT m, WPARAM w, LPARAM l) {
    if (m == WM_COMMAND) { cmds++; lastw = w; return 0; }
    if (m == WM_SYSCOMMAND) { syscmds++; lastw = w; return 0; }
    if (m == WM_MOUSELEAVE) { leaves++; return 0; }
    return DefWindowProcW(h, m, w, l);
}

static void pump(void) {
    MSG m;
    while (PeekMessageW(&m, NULL, 0, 0, PM_REMOVE)) DispatchMessageW(&m);
}

static MSG keymsg(HWND h, UINT msg, WPARAM vk) {
    MSG m = {0};
    m.hwnd = h; m.message = msg; m.wParam = vk; m.lParam = 1;
    return m;
}

int main(void) {
    WNDCLASSW wc = {0};
    wc.lpfnWndProc = wp; wc.hInstance = GetModuleHandleW(NULL); wc.lpszClassName = L"UBC";
    wc.cbClsExtra = 8; wc.cbWndExtra = 8;
    printf("reg=%d\n", RegisterClassW(&wc) != 0);
    HWND hw = CreateWindowExW(WS_EX_LAYERED, L"UBC", L"ub", WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_VSCROLL, 100, 100, 300, 200, NULL, NULL, wc.hInstance, NULL);
    printf("hw=%d\n", hw != NULL);

    /* class / window words */
    SetClassLongPtrW(hw, 0, 0x1234);
    SetClassLongW(hw, 4, 0x5678);
    printf("classlong %lx %lx\n", (long)GetClassLongPtrW(hw, 0), (long)GetClassLongW(hw, 4));
    DWORD oldstyle = SetClassLongW(hw, GCL_STYLE, CS_HREDRAW);
    printf("classstyle old=%lx new=%lx\n", (long)oldstyle, (long)GetClassLongW(hw, GCL_STYLE));
    SetWindowWord(hw, 2, 0xBEEF);
    printf("winword %x\n", GetWindowWord(hw, 2));
    printf("unreg busy=%d\n", UnregisterClassW(L"UBC", wc.hInstance));

    /* layered attributes */
    BOOL ok = SetLayeredWindowAttributes(hw, RGB(1, 2, 3), 128, LWA_ALPHA | LWA_COLORKEY);
    COLORREF key = 0; BYTE alpha = 0; DWORD fl = 0;
    BOOL ok2 = GetLayeredWindowAttributes(hw, &key, &alpha, &fl);
    printf("layered %d %d key=%lx alpha=%d flags=%lx\n", ok, ok2, (long)key, alpha, (long)fl);

    /* window region */
    HRGN r = CreateRectRgn(1, 2, 30, 40);
    printf("setrgn=%d\n", SetWindowRgn(hw, r, TRUE) != 0);
    HRGN r2 = CreateRectRgn(0, 0, 0, 0);
    int rt = GetWindowRgn(hw, r2);
    RECT bx; GetRgnBox(r2, &bx);
    printf("getrgn type=%d box=%ld,%ld,%ld,%ld\n", rt, bx.left, bx.top, bx.right, bx.bottom);

    /* caret */
    printf("caret create=%d\n", CreateCaret(hw, NULL, 2, 10));
    SetCaretPos(5, 6);
    POINT cp = {0, 0}; GetCaretPos(&cp);
    printf("caretpos %ld,%ld blink=%u\n", cp.x, cp.y, GetCaretBlinkTime());
    DestroyCaret();

    /* thread messages */
    printf("postthread=%d\n", PostThreadMessageW(GetCurrentThreadId(), WM_APP + 1, 7, 8));
    {
        MSG m; BOOL got = PeekMessageW(&m, NULL, WM_APP, WM_APP + 5, PM_REMOVE);
        printf("threadmsg got=%d msg=%x w=%lu l=%ld hwnd=%d\n", got, m.message, (unsigned long)m.wParam, (long)m.lParam, m.hwnd != NULL);
    }

    /* system menu */
    HMENU sm = GetSystemMenu(hw, FALSE);
    printf("sysmenu=%d count=%d close=%d bogus=%d\n", sm != NULL, GetMenuItemCount(sm), GetMenuState(sm, SC_CLOSE, MF_BYCOMMAND) != (UINT)-1, GetMenuState(sm, 9999, MF_BYCOMMAND) != (UINT)-1);
    printf("sysrevert=%d\n", GetSystemMenu(hw, TRUE) != NULL);

    /* menu info */
    HMENU mn = CreatePopupMenu();
    MENUINFO mi = {0}; mi.cbSize = sizeof mi; mi.fMask = MIM_STYLE | MIM_MENUDATA; mi.dwStyle = MNS_CHECKORBMP; mi.dwMenuData = 77;
    SetMenuInfo(mn, &mi);
    MENUINFO g = {0}; g.cbSize = sizeof g; g.fMask = MIM_STYLE | MIM_MENUDATA;
    GetMenuInfo(mn, &g);
    SetMenuContextHelpId(mn, 42);
    printf("menuinfo style=%lx data=%lu help=%lu\n", (long)g.dwStyle, (unsigned long)g.dwMenuData, (unsigned long)GetMenuContextHelpId(mn));

    /* accelerators */
    ACCEL ac[3] = { { FVIRTKEY | FCONTROL, 'S', 100 }, { FVIRTKEY, VK_F5, 101 }, { FVIRTKEY | FSHIFT | FCONTROL, 'Z', 102 } };
    HACCEL ha = CreateAcceleratorTableW(ac, 3);
    ACCEL back[3] = {0};
    int cnt = CopyAcceleratorTableW(ha, NULL, 0);
    int cpy = CopyAcceleratorTableW(ha, back, 3);
    printf("accel h=%d count=%d copy=%d first=%d/%d/%d\n", ha != NULL, cnt, cpy, back[0].fVirt, back[0].key, back[0].cmd);
    BYTE ks[256] = {0};
    MSG m1 = keymsg(hw, WM_KEYDOWN, 'S');
    int t1 = TranslateAcceleratorW(hw, ha, &m1);
    ks[VK_CONTROL] = 0x80; SetKeyboardState(ks);
    int t2 = TranslateAcceleratorW(hw, ha, &m1);
    printf("acc plainS=%d ctrlS=%d cmds=%d w=%lx\n", t1, t2, cmds, (long)lastw);
    ks[VK_CONTROL] = 0; SetKeyboardState(ks);
    MSG m2 = keymsg(hw, WM_KEYDOWN, VK_F5);
    int t3 = TranslateAcceleratorW(hw, ha, &m2);
    printf("acc F5=%d cmds=%d w=%lx\n", t3, cmds, (long)lastw);
    MSG m3 = keymsg(hw, WM_KEYDOWN, 'Z');
    ks[VK_CONTROL] = 0x80; SetKeyboardState(ks);
    int t4 = TranslateAcceleratorW(hw, ha, &m3);
    ks[VK_SHIFT] = 0x80; SetKeyboardState(ks);
    int t5 = TranslateAcceleratorW(hw, ha, &m3);
    printf("acc ctrlZ=%d ctrlshiftZ=%d cmds=%d w=%lx\n", t4, t5, cmds, (long)lastw);
    ks[VK_CONTROL] = 0; ks[VK_SHIFT] = 0; SetKeyboardState(ks);
    MSG m4 = keymsg(hw, WM_KEYUP, VK_F5);
    int ku = TranslateAcceleratorW(hw, ha, &m4);
    int ds = DestroyAcceleratorTable(ha);
    printf("acc keyup=%d destroy=%d\n", ku, ds);
    printf("acc afterdestroy=%d\n", TranslateAcceleratorW(hw, ha, &m2));
    /* a system-menu command resolves to WM_SYSCOMMAND */
    GetSystemMenu(hw, FALSE);
    ACCEL sa[1] = { { FVIRTKEY | FALT, VK_F4, SC_CLOSE } };
    HACCEL hs = CreateAcceleratorTableW(sa, 1);
    ks[VK_MENU] = 0x80; SetKeyboardState(ks);
    MSG m5 = keymsg(hw, WM_SYSKEYDOWN, VK_F4);
    int t6 = TranslateAcceleratorW(hw, hs, &m5);
    ks[VK_MENU] = 0; SetKeyboardState(ks);
    printf("acc sys=%d syscmds=%d w=%lx\n", t6, syscmds, (long)lastw);
    /* icons: 16x16 32-bit DIB with alpha, left half red opaque, right half alpha 0 */
    HDC sdc = GetDC(NULL);
    HDC mdc = CreateCompatibleDC(sdc);
    BITMAPINFO bi = {0};
    bi.bmiHeader.biSize = sizeof bi.bmiHeader; bi.bmiHeader.biWidth = 16; bi.bmiHeader.biHeight = -16; bi.bmiHeader.biPlanes = 1; bi.bmiHeader.biBitCount = 32;
    DWORD *pix = NULL;
    HBITMAP colbm = CreateDIBSection(sdc, &bi, DIB_RGB_COLORS, (void **)&pix, NULL, 0);
    for (int y = 0; y < 16; y++) for (int x = 0; x < 16; x++) pix[y * 16 + x] = x < 8 ? 0xFFFF0000 : 0x00000000;
    BYTE maskbits[32];
    for (int y = 0; y < 16; y++) { maskbits[y * 2] = 0x00; maskbits[y * 2 + 1] = 0x00; }
    HBITMAP maskbm = CreateBitmap(16, 16, 1, 1, maskbits);
    ICONINFO ii = { TRUE, 3, 4, maskbm, colbm };
    HICON ic = CreateIconIndirect(&ii);
    printf("icon=%d\n", ic != NULL);
    ICONINFO out = {0};
    BOOL gi = GetIconInfo(ic, &out);
    BITMAP bm = {0}; GetObject(out.hbmColor, sizeof bm, &bm);
    printf("iconinfo ok=%d icon=%d hot=%lu,%lu colorw=%ld distinct=%d\n", gi, out.fIcon, (unsigned long)out.xHotspot, (unsigned long)out.yHotspot, bm.bmWidth, out.hbmColor != colbm);
    DeleteObject(out.hbmColor); DeleteObject(out.hbmMask);
    HBITMAP dst = CreateCompatibleBitmap(sdc, 32, 32);
    HGDIOBJ od = SelectObject(mdc, dst);
    RECT all = {0, 0, 32, 32};
    FillRect(mdc, &all, (HBRUSH)GetStockObject(WHITE_BRUSH));
    printf("drawicon=%d\n", DrawIconEx(mdc, 4, 4, ic, 16, 16, 0, NULL, DI_NORMAL));
    printf("pix left=%lx right=%lx outside=%lx\n", (long)GetPixel(mdc, 6, 8), (long)GetPixel(mdc, 16, 8), (long)GetPixel(mdc, 1, 1));
    SelectObject(mdc, od);
    DestroyIcon(ic);

    /* ScrollDC on a memory bitmap */
    HBITMAP sb = CreateCompatibleBitmap(sdc, 16, 16);
    HGDIOBJ os = SelectObject(mdc, sb);
    for (int x = 0; x < 16; x++) SetPixel(mdc, x, 0, RGB(x * 10, 0, 0));
    RECT upd = {0};
    HRGN ur = CreateRectRgn(0, 0, 0, 0);
    printf("scrolldc=%d\n", ScrollDC(mdc, 4, 0, NULL, NULL, ur, &upd));
    printf("scroll px8=%lx upd=%ld,%ld,%ld,%ld\n", (long)GetPixel(mdc, 8, 0), upd.left, upd.top, upd.right, upd.bottom);
    SelectObject(mdc, os);

    /* deferred window positioning */
    HDWP dwp = BeginDeferWindowPos(2);
    dwp = DeferWindowPos(dwp, hw, NULL, 10, 20, 300, 200, SWP_NOZORDER);
    printf("endswp=%d\n", EndDeferWindowPos(dwp));
    RECT wr; GetWindowRect(hw, &wr);
    printf("deferred rect=%ld,%ld,%ld,%ld\n", wr.left, wr.top, wr.right, wr.bottom);

    /* misc window calls */
    HDC wdc = GetDC(hw);
    printf("wfromdc=%d\n", WindowFromDC(wdc) == hw);
    ReleaseDC(hw, wdc);
    printf("lock=%d unlock=%d\n", LockWindowUpdate(hw), LockWindowUpdate(NULL));
    HWND child = CreateWindowExW(0, L"UBC", L"c", WS_CHILD | WS_VISIBLE, 10, 10, 50, 50, hw, NULL, wc.hInstance, NULL);
    POINT pa = {20, 20}, pb = {150, 100};
    printf("realchild in=%d out=%d\n", RealChildWindowFromPoint(hw, pa) == child, RealChildWindowFromPoint(hw, pb) == hw);
    TITLEBARINFO tb = {0}; tb.cbSize = sizeof tb;
    int tbr = GetTitleBarInfo(hw, &tb);
    printf("titlebar=%d wide=%d\n", tbr, tb.rcTitleBar.right > tb.rcTitleBar.left);
    SCROLLBARINFO sbi = {0}; sbi.cbSize = sizeof sbi;
    int sbr = GetScrollBarInfo(hw, OBJID_VSCROLL, &sbi);
    printf("sbinfo=%d vis=%d\n", sbr, !(sbi.rgstate[0] & STATE_SYSTEM_INVISIBLE));
    int sa_ = ShowWindowAsync(hw, SW_HIDE);
    printf("showasync=%d vis=%d\n", sa_, IsWindowVisible(hw));
    ShowWindow(hw, SW_SHOW);

    /* colours and palettes */
    INT idx = COLOR_BTNFACE; COLORREF cv = RGB(1, 2, 3);
    SetSysColors(1, &idx, &cv);
    printf("syscolor %lx\n", (long)GetSysColor(COLOR_BTNFACE));
    HPALETTE pal = CreateHalftonePalette(mdc);
    HPALETTE o1 = SelectPalette(mdc, pal, FALSE);
    HPALETTE o2 = SelectPalette(mdc, o1, FALSE);
    printf("palette old=%d back=%d\n", o1 != NULL, o2 == pal);

    /* input injection */
    INPUT in[2] = {0};
    in[0].type = INPUT_KEYBOARD; in[0].ki.wVk = VK_CONTROL;
    in[1].type = INPUT_MOUSE; in[1].mi.dx = 32768; in[1].mi.dy = 16384; in[1].mi.dwFlags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE;
    printf("sendinput=%u\n", SendInput(2, in, sizeof(INPUT)));
    POINT cur; GetCursorPos(&cur);
    printf("cursor %d %d\n", cur.x == GetSystemMetrics(SM_CXSCREEN) / 2, cur.y == GetSystemMetrics(SM_CYSCREEN) / 4);
    printf("ctrl=%d\n", (GetKeyState(VK_CONTROL) & 0x8000) != 0);
    in[0].ki.dwFlags = KEYEVENTF_KEYUP;
    SendInput(1, in, sizeof(INPUT));
    printf("ctrlup=%d\n", (GetKeyState(VK_CONTROL) & 0x8000) != 0);

    /* mouse tracking: leave */
    SetCursorPos(200, 150);
    TRACKMOUSEEVENT te = { sizeof te, TME_LEAVE, hw, 0 };
    printf("track=%d\n", TrackMouseEvent(&te));
    TRACKMOUSEEVENT q = { sizeof q, TME_QUERY, hw, 0 };
    TrackMouseEvent(&q);
    printf("query flags=%lx\n", (long)q.dwFlags);
    pump();
    printf("leaves before=%d\n", leaves);
    SetCursorPos(900, 700);
    pump();
    printf("leaves after=%d\n", leaves);

    DestroyWindow(child);
    DestroyWindow(hw);
    printf("unreg=%d\n", UnregisterClassW(L"UBC", wc.hInstance));
    printf("done\n");
    return 0;
}
