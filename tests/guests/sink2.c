#include <windows.h>
#include <commctrl.h>
#include <stdio.h>

/* Dialogs, MessageBox, common controls, scroll bars, clipboard. The main window's timer drives each step. */
static HWND main_wnd;
static int step = 0;

static INT_PTR CALLBACK DlgProc(HWND h, UINT m, WPARAM w, LPARAM l) {
    if (m == WM_INITDIALOG) { SetTimer(h, 9, 100, 0); return TRUE; }
    if (m == WM_TIMER) { KillTimer(h, 9); EndDialog(h, 77); return TRUE; }
    return FALSE;
}

static LRESULT CALLBACK WndProc(HWND h, UINT m, WPARAM w, LPARAM l) {
    switch (m) {
    case WM_CREATE: {
        INITCOMMONCONTROLSEX ic = { sizeof ic, ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES | ICC_TAB_CLASSES };
        InitCommonControlsEx(&ic);
        HWND lv = CreateWindowExA(0, WC_LISTVIEWA, "", WS_CHILD | WS_VISIBLE | LVS_REPORT, 10, 10, 260, 100, h, (HMENU)5, GetModuleHandleA(0), 0);
        LVCOLUMNA col = { LVCF_TEXT | LVCF_WIDTH, 0, 120, "Name" }; SendMessageA(lv, LVM_INSERTCOLUMNA, 0, (LPARAM)&col);
        LVITEMA it = { LVIF_TEXT, 0, 0, 0, 0, "first item" }; SendMessageA(lv, LVM_INSERTITEMA, 0, (LPARAM)&it);
        CreateWindowExA(0, STATUSCLASSNAMEA, "ready", WS_CHILD | WS_VISIBLE, 0, 0, 0, 0, h, (HMENU)6, GetModuleHandleA(0), 0);
        CreateWindowExA(0, "SCROLLBAR", "", WS_CHILD | WS_VISIBLE | SBS_VERT, 280, 10, 16, 100, h, (HMENU)7, GetModuleHandleA(0), 0);
        CreateWindowExA(0, "LISTBOX", "", WS_CHILD | WS_VISIBLE | WS_BORDER, 10, 120, 120, 60, h, (HMENU)8, GetModuleHandleA(0), 0);
        SendDlgItemMessageA(h, 8, LB_ADDSTRING, 0, (LPARAM)"alpha");
        SendDlgItemMessageA(h, 8, LB_ADDSTRING, 0, (LPARAM)"beta");
        CreateWindowExA(0, "COMBOBOX", "", WS_CHILD | WS_VISIBLE | CBS_DROPDOWNLIST, 140, 120, 120, 100, h, (HMENU)9, GetModuleHandleA(0), 0);
        SetTimer(h, 1, 150, 0);
        return 0;
    }
    case WM_TIMER:
        KillTimer(h, 1);
        if (OpenClipboard(h)) {
            EmptyClipboard();
            HGLOBAL g = GlobalAlloc(GMEM_MOVEABLE, 6); strcpy((char *)GlobalLock(g), "clip!"); GlobalUnlock(g);
            SetClipboardData(CF_TEXT, g); CloseClipboard();
        }
        printf("clip=%d\n", IsClipboardFormatAvailable(CF_TEXT));
        /* modal dialog from an in-memory template */
        struct { DLGTEMPLATE t; WORD menu, cls, title; } tpl = { { DS_MODALFRAME | WS_POPUP | WS_CAPTION, 0, 0, 0, 0, 120, 60 }, 0, 0, 0 };
        INT_PTR r = DialogBoxIndirectParamA(GetModuleHandleA(0), &tpl.t, h, DlgProc, 0);
        printf("dialog=%d\n", (int)r);
        PostMessageA(h, WM_CLOSE, 0, 0);
        return 0;
    case WM_CLOSE: DestroyWindow(h); return 0;
    case WM_DESTROY: PostQuitMessage(0); return 0;
    }
    return DefWindowProcA(h, m, w, l);
}

int main(void) {
    WNDCLASSA wc = {0}; wc.lpfnWndProc = WndProc; wc.hInstance = GetModuleHandleA(0); wc.hbrBackground = (HBRUSH)(COLOR_BTNFACE + 1);
    wc.hCursor = LoadCursorA(0, IDC_ARROW); wc.lpszClassName = "Sink2";
    RegisterClassA(&wc);
    main_wnd = CreateWindowExA(0, "Sink2", "Sink 2", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 20, 20, 320, 260, 0, 0, wc.hInstance, 0);
    MSG msg; while (GetMessageA(&msg, 0, 0, 0) > 0) { TranslateMessage(&msg); DispatchMessageA(&msg); }
    printf("done2\n");
    return 0;
}
