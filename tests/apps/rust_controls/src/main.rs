//! Win32 controls showcase: raw FFI only (no crates), built for i686-pc-windows-msvc.
//! Classic user32 controls plus comctl32 (list view, tree view, tab, progress, trackbar, up-down, status bar,
//! date picker, IP address, hot key). Interacting with a control reports the event in the status bar.
#![windows_subsystem = "windows"]
#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use std::ptr::{null, null_mut};

type HWND = isize;
type HINSTANCE = isize;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type UINT = u32;

#[repr(C)] struct POINT { x: i32, y: i32 }
#[repr(C)] struct RECT { left: i32, top: i32, right: i32, bottom: i32 }
#[repr(C)] struct MSG { hwnd: HWND, message: UINT, wParam: WPARAM, lParam: LPARAM, time: u32, pt: POINT }
#[repr(C)] struct WNDCLASSW {
    style: UINT, lpfnWndProc: unsafe extern "system" fn(HWND, UINT, WPARAM, LPARAM) -> LRESULT, cbClsExtra: i32, cbWndExtra: i32,
    hInstance: HINSTANCE, hIcon: isize, hCursor: isize, hbrBackground: isize, lpszMenuName: *const u16, lpszClassName: *const u16,
}
#[repr(C)] struct INITCOMMONCONTROLSEX { dwSize: u32, dwICC: u32 }
#[repr(C)] struct NMHDR { hwndFrom: HWND, idFrom: usize, code: u32 }
#[repr(C)] struct LVCOLUMNW { mask: u32, fmt: i32, cx: i32, pszText: *mut u16, cchTextMax: i32, iSubItem: i32, iImage: i32, iOrder: i32 }
#[repr(C)] struct LVITEMW { mask: u32, iItem: i32, iSubItem: i32, state: u32, stateMask: u32, pszText: *mut u16, cchTextMax: i32, iImage: i32, lParam: isize }
#[repr(C)] struct TVITEMW { mask: u32, hItem: isize, state: u32, stateMask: u32, pszText: *mut u16, cchTextMax: i32, iImage: i32, iSelectedImage: i32, cChildren: i32, lParam: isize }
#[repr(C)] struct TVINSERTSTRUCTW { hParent: isize, hInsertAfter: isize, item: TVITEMW }
#[repr(C)] struct TCITEMW { mask: u32, dwState: u32, dwStateMask: u32, pszText: *mut u16, cchTextMax: i32, iImage: i32, lParam: isize }
#[repr(C)] struct NMLISTVIEW { hdr: NMHDR, iItem: i32, iSubItem: i32, uNewState: u32, uOldState: u32, uChanged: u32, ptAction: POINT, lParam: isize }

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(c: *const WNDCLASSW) -> u16;
    fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: isize, inst: HINSTANCE, param: *mut u8) -> HWND;
    fn DefWindowProcW(h: HWND, m: UINT, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetMessageW(m: *mut MSG, h: HWND, a: UINT, b: UINT) -> i32;
    fn TranslateMessage(m: *const MSG) -> i32;
    fn DispatchMessageW(m: *const MSG) -> LRESULT;
    fn PostQuitMessage(code: i32);
    fn SendMessageW(h: HWND, m: UINT, w: WPARAM, l: LPARAM) -> LRESULT;
    fn ShowWindow(h: HWND, cmd: i32) -> i32;
    fn UpdateWindow(h: HWND) -> i32;
    fn LoadCursorW(inst: HINSTANCE, name: usize) -> isize;
    fn GetClientRect(h: HWND, r: *mut RECT) -> i32;
    fn MoveWindow(h: HWND, x: i32, y: i32, w: i32, hh: i32, repaint: i32) -> i32;
    fn GetDlgItem(h: HWND, id: i32) -> HWND;
    fn GetWindowTextW(h: HWND, buf: *mut u16, n: i32) -> i32;
    fn SetWindowTextW(h: HWND, s: *const u16) -> i32;
    fn CheckRadioButton(h: HWND, first: i32, last: i32, check: i32) -> i32;
    fn MessageBoxW(h: HWND, text: *const u16, cap: *const u16, ty: u32) -> i32;
}
#[link(name = "gdi32")]
extern "system" { fn GetStockObject(i: i32) -> isize; }
#[link(name = "kernel32")]
extern "system" { fn GetModuleHandleW(name: *const u16) -> HINSTANCE; }
#[link(name = "comctl32")]
extern "system" { fn InitCommonControlsEx(c: *const INITCOMMONCONTROLSEX) -> i32; }

// ---- constants ----
const WS_CHILD: u32 = 0x4000_0000; const WS_VISIBLE: u32 = 0x1000_0000; const WS_BORDER: u32 = 0x0080_0000;
const WS_VSCROLL: u32 = 0x0020_0000; const WS_TABSTOP: u32 = 0x0001_0000; const WS_GROUP: u32 = 0x0002_0000;
const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000; const WS_CLIPCHILDREN: u32 = 0x0200_0000;
const WM_CREATE: u32 = 1; const WM_DESTROY: u32 = 2; const WM_SIZE: u32 = 5; const WM_COMMAND: u32 = 0x111; const WM_NOTIFY: u32 = 0x4E;
const WM_HSCROLL: u32 = 0x114; const WM_SETFONT: u32 = 0x30; const WM_USER: u32 = 0x400; const WM_CLOSE: u32 = 0x10;
const BS_PUSHBUTTON: u32 = 0; const BS_DEFPUSHBUTTON: u32 = 1; const BS_AUTOCHECKBOX: u32 = 3; const BS_AUTORADIOBUTTON: u32 = 9;
const BS_GROUPBOX: u32 = 7; const BS_AUTO3STATE: u32 = 6;
const BM_SETCHECK: u32 = 0xF1; const BM_GETCHECK: u32 = 0xF0;
const ES_AUTOHSCROLL: u32 = 0x80; const ES_MULTILINE: u32 = 4; const ES_PASSWORD: u32 = 0x20; const ES_WANTRETURN: u32 = 0x1000; const ES_AUTOVSCROLL: u32 = 0x40;
const SS_LEFT: u32 = 0; const SS_ETCHEDHORZ: u32 = 0x10; const SS_SUNKEN: u32 = 0x1000;
const CBS_DROPDOWNLIST: u32 = 3; const CBS_DROPDOWN: u32 = 2; const CB_ADDSTRING: u32 = 0x143; const CB_SETCURSEL: u32 = 0x14E;
const LBS_NOTIFY: u32 = 1; const LBS_EXTENDEDSEL: u32 = 0x800; const LB_ADDSTRING: u32 = 0x180;
const SBS_HORZ: u32 = 0;
const SB_SETPARTS: u32 = WM_USER + 4; const SB_SETTEXTW: u32 = WM_USER + 11;
const PBM_SETPOS: u32 = WM_USER + 2; const PBM_SETRANGE32: u32 = WM_USER + 6;
const TBM_SETRANGE: u32 = WM_USER + 6; const TBM_SETPOS: u32 = WM_USER + 5; const TBM_GETPOS: u32 = WM_USER; const TBS_AUTOTICKS: u32 = 1;
const UDM_SETRANGE32: u32 = WM_USER + 111; const UDM_SETPOS32: u32 = WM_USER + 113; const UDS_SETBUDDYINT: u32 = 2; const UDS_ALIGNRIGHT: u32 = 4; const UDS_ARROWKEYS: u32 = 0x20;
const LVS_REPORT: u32 = 1; const LVM_INSERTCOLUMNW: u32 = 0x1000 + 97; const LVM_INSERTITEMW: u32 = 0x1000 + 77; const LVM_SETITEMTEXTW: u32 = 0x1000 + 116;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1000 + 54; const LVS_EX_FULLROWSELECT: isize = 0x20; const LVS_EX_GRIDLINES: isize = 1;
const LVCF_TEXT: u32 = 4; const LVCF_WIDTH: u32 = 2; const LVIF_TEXT: u32 = 1; const LVN_ITEMCHANGED: u32 = (-101i32) as u32;
const TVM_INSERTITEMW: u32 = 0x1100 + 50; const TVS_HASLINES: u32 = 2; const TVS_HASBUTTONS: u32 = 1; const TVS_LINESATROOT: u32 = 4; const TVIF_TEXT: u32 = 1;
const TVN_SELCHANGEDW: u32 = (-451i32) as u32; const TVI_LAST: isize = -0xFFFE; const TVI_ROOT: isize = -0x10000;
const TCM_INSERTITEMW: u32 = 0x1300 + 62; const TCIF_TEXT: u32 = 1; const TCN_SELCHANGE: u32 = (-551i32) as u32;
const ICC_ALL: u32 = 0xFFFF;
const CW_USEDEFAULT: i32 = i32::MIN;
const COLOR_BTNFACE: isize = 15;
const DEFAULT_GUI_FONT: i32 = 17;
const IDC_ARROW: usize = 32512;

// ---- helpers ----
fn w(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }

unsafe fn mk(h_parent: HWND, inst: HINSTANCE, class: &str, text: &str, style: u32, x: i32, y: i32, wd: i32, ht: i32, id: i32) -> HWND {
    let c = w(class); let t = w(text);
    let h = CreateWindowExW(0, c.as_ptr(), t.as_ptr(), WS_CHILD | WS_VISIBLE | style, x, y, wd, ht, h_parent, id as isize, inst, null_mut());
    SendMessageW(h, WM_SETFONT, GetStockObject(DEFAULT_GUI_FONT) as usize, 1);
    h
}

unsafe fn set_status(hwnd: HWND, text: &str) {
    let sb = GetDlgItem(hwnd, ID_STATUS);
    let t = w(text);
    SendMessageW(sb, SB_SETTEXTW, 0, t.as_ptr() as isize);
}

const ID_STATUS: i32 = 900;
const ID_PUSH: i32 = 101; const ID_DEFPUSH: i32 = 102; const ID_CHECK: i32 = 103; const ID_TRI: i32 = 104;
const ID_R1: i32 = 111; const ID_R2: i32 = 112; const ID_R3: i32 = 113;
const ID_EDIT: i32 = 120; const ID_PASS: i32 = 121; const ID_MULTI: i32 = 122;
const ID_COMBO1: i32 = 130; const ID_COMBO2: i32 = 131; const ID_LIST: i32 = 132;
const ID_LV: i32 = 140; const ID_TV: i32 = 141; const ID_TAB: i32 = 142;
const ID_TRACK: i32 = 150; const ID_PROG: i32 = 151; const ID_SPIN: i32 = 152; const ID_SPINEDIT: i32 = 153; const ID_HSCROLL: i32 = 154;
const ID_DATE: i32 = 160; const ID_IP: i32 = 161; const ID_HOTKEY: i32 = 162;
const ID_ABOUT: i32 = 170;

unsafe fn build(hwnd: HWND, inst: HINSTANCE) {
    // column 1: buttons + static + edits
    mk(hwnd, inst, "BUTTON", "Buttons", BS_GROUPBOX, 10, 6, 220, 150, 0);
    mk(hwnd, inst, "BUTTON", "Push button", BS_PUSHBUTTON | WS_TABSTOP, 20, 24, 95, 24, ID_PUSH);
    mk(hwnd, inst, "BUTTON", "Default", BS_DEFPUSHBUTTON | WS_TABSTOP, 122, 24, 95, 24, ID_DEFPUSH);
    mk(hwnd, inst, "BUTTON", "Check box", BS_AUTOCHECKBOX | WS_TABSTOP, 20, 56, 90, 20, ID_CHECK);
    mk(hwnd, inst, "BUTTON", "Three-state", BS_AUTO3STATE | WS_TABSTOP, 122, 56, 100, 20, ID_TRI);
    mk(hwnd, inst, "BUTTON", "Radio A", BS_AUTORADIOBUTTON | WS_GROUP | WS_TABSTOP, 20, 84, 64, 20, ID_R1);
    mk(hwnd, inst, "BUTTON", "Radio B", BS_AUTORADIOBUTTON, 88, 84, 64, 20, ID_R2);
    mk(hwnd, inst, "BUTTON", "Radio C", BS_AUTORADIOBUTTON, 156, 84, 64, 20, ID_R3);
    CheckRadioButton(hwnd, ID_R1, ID_R3, ID_R1);
    mk(hwnd, inst, "BUTTON", "About...", BS_PUSHBUTTON | WS_TABSTOP, 20, 116, 95, 26, ID_ABOUT);

    mk(hwnd, inst, "STATIC", "Static text label", SS_LEFT, 12, 164, 150, 16, 0);
    mk(hwnd, inst, "STATIC", "", SS_ETCHEDHORZ, 12, 184, 218, 2, 0);
    mk(hwnd, inst, "EDIT", "Single-line edit", WS_BORDER | ES_AUTOHSCROLL | WS_TABSTOP, 12, 192, 218, 22, ID_EDIT);
    mk(hwnd, inst, "EDIT", "secret", WS_BORDER | ES_AUTOHSCROLL | ES_PASSWORD | WS_TABSTOP, 12, 220, 218, 22, ID_PASS);
    mk(hwnd, inst, "EDIT", "Multi-line edit.\r\nType here...", WS_BORDER | ES_MULTILINE | ES_WANTRETURN | ES_AUTOVSCROLL | WS_VSCROLL | WS_TABSTOP, 12, 248, 218, 70, ID_MULTI);

    // column 2: combo boxes, list box, sliders
    mk(hwnd, inst, "STATIC", "Combo boxes", SS_LEFT, 244, 8, 150, 16, 0);
    let c1 = mk(hwnd, inst, "COMBOBOX", "", CBS_DROPDOWNLIST | WS_VSCROLL | WS_TABSTOP, 244, 26, 170, 200, ID_COMBO1);
    let c2 = mk(hwnd, inst, "COMBOBOX", "", CBS_DROPDOWN | WS_VSCROLL | WS_TABSTOP, 244, 54, 170, 200, ID_COMBO2);
    for (i, s) in ["Alpha", "Beta", "Gamma", "Delta"].iter().enumerate() {
        let t = w(s);
        SendMessageW(c1, CB_ADDSTRING, 0, t.as_ptr() as isize);
        if i < 3 { SendMessageW(c2, CB_ADDSTRING, 0, t.as_ptr() as isize); }
    }
    SendMessageW(c1, CB_SETCURSEL, 1, 0);
    SendMessageW(c2, CB_SETCURSEL, 0, 0);
    mk(hwnd, inst, "STATIC", "List box (multi-select)", SS_LEFT, 244, 86, 170, 16, 0);
    let lb = mk(hwnd, inst, "LISTBOX", "", WS_BORDER | WS_VSCROLL | LBS_NOTIFY | LBS_EXTENDEDSEL | WS_TABSTOP, 244, 104, 170, 84, ID_LIST);
    for s in ["Red", "Green", "Blue", "Cyan", "Magenta", "Yellow", "Black"] { let t = w(s); SendMessageW(lb, LB_ADDSTRING, 0, t.as_ptr() as isize); }

    mk(hwnd, inst, "STATIC", "Trackbar drives the progress bar", SS_LEFT, 244, 198, 200, 16, 0);
    let tb = mk(hwnd, inst, "msctls_trackbar32", "", TBS_AUTOTICKS | WS_TABSTOP, 244, 216, 170, 30, ID_TRACK);
    SendMessageW(tb, TBM_SETRANGE, 1, (100 << 16) as isize);
    SendMessageW(tb, TBM_SETPOS, 1, 35);
    let pb = mk(hwnd, inst, "msctls_progress32", "", WS_BORDER, 244, 252, 170, 18, ID_PROG);
    SendMessageW(pb, PBM_SETRANGE32, 0, 100);
    SendMessageW(pb, PBM_SETPOS, 35, 0);
    mk(hwnd, inst, "STATIC", "Up-down:", SS_LEFT, 244, 282, 56, 16, 0);
    let ed = mk(hwnd, inst, "EDIT", "5", WS_BORDER | ES_AUTOHSCROLL, 304, 278, 60, 22, ID_SPINEDIT);
    let ud = mk(hwnd, inst, "msctls_updown32", "", UDS_SETBUDDYINT | UDS_ALIGNRIGHT | UDS_ARROWKEYS, 0, 0, 0, 0, ID_SPIN);
    let _ = ed;
    SendMessageW(ud, UDM_SETRANGE32, 0, 20);
    SendMessageW(ud, UDM_SETPOS32, 0, 5);
    mk(hwnd, inst, "SCROLLBAR", "", SBS_HORZ, 244, 306, 170, 16, ID_HSCROLL);

    // column 3: list view, tree view, tab, pickers
    let lv = mk(hwnd, inst, "SysListView32", "", WS_BORDER | LVS_REPORT | WS_TABSTOP, 428, 8, 322, 110, ID_LV);
    SendMessageW(lv, LVM_SETEXTENDEDLISTVIEWSTYLE, 0, LVS_EX_FULLROWSELECT | LVS_EX_GRIDLINES);
    for (i, (name, cx)) in [("Name", 110), ("Type", 90), ("Size", 70)].iter().enumerate() {
        let mut t = w(name);
        let col = LVCOLUMNW { mask: LVCF_TEXT | LVCF_WIDTH, fmt: 0, cx: *cx, pszText: t.as_mut_ptr(), cchTextMax: 0, iSubItem: i as i32, iImage: 0, iOrder: 0 };
        SendMessageW(lv, LVM_INSERTCOLUMNW, i, &col as *const _ as isize);
    }
    for (row, cells) in [["readme.txt", "Text", "2 KB"], ["plonk.exe", "Application", "1.4 MB"], ["notes.md", "Markdown", "11 KB"], ["icon.png", "Image", "48 KB"]].iter().enumerate() {
        let mut t = w(cells[0]);
        let item = LVITEMW { mask: LVIF_TEXT, iItem: row as i32, iSubItem: 0, state: 0, stateMask: 0, pszText: t.as_mut_ptr(), cchTextMax: 0, iImage: 0, lParam: 0 };
        SendMessageW(lv, LVM_INSERTITEMW, 0, &item as *const _ as isize);
        for (sub, text) in cells.iter().enumerate().skip(1) {
            let mut t = w(text);
            let it = LVITEMW { mask: 0, iItem: row as i32, iSubItem: sub as i32, state: 0, stateMask: 0, pszText: t.as_mut_ptr(), cchTextMax: 0, iImage: 0, lParam: 0 };
            SendMessageW(lv, LVM_SETITEMTEXTW, row, &it as *const _ as isize);
        }
    }

    let tv = mk(hwnd, inst, "SysTreeView32", "", WS_BORDER | TVS_HASLINES | TVS_HASBUTTONS | TVS_LINESATROOT | WS_TABSTOP, 428, 126, 160, 130, ID_TV);
    let ins = |parent: isize, name: &str| -> isize {
        let mut t = w(name);
        let s = TVINSERTSTRUCTW { hParent: parent, hInsertAfter: TVI_LAST, item: TVITEMW { mask: TVIF_TEXT, hItem: 0, state: 0, stateMask: 0, pszText: t.as_mut_ptr(), cchTextMax: 0, iImage: 0, iSelectedImage: 0, cChildren: 0, lParam: 0 } };
        SendMessageW(tv, TVM_INSERTITEMW, 0, &s as *const _ as isize)
    };
    let root = ins(TVI_ROOT, "Computer");
    let c = ins(root, "Local Disk (C:)");
    ins(c, "Windows"); ins(c, "Users"); ins(c, "Program Files");
    ins(root, "Network");

    let tab = mk(hwnd, inst, "SysTabControl32", "", WS_TABSTOP, 596, 126, 154, 130, ID_TAB);
    for (i, s) in ["General", "Advanced", "About"].iter().enumerate() {
        let mut t = w(s);
        let item = TCITEMW { mask: TCIF_TEXT, dwState: 0, dwStateMask: 0, pszText: t.as_mut_ptr(), cchTextMax: 0, iImage: 0, lParam: 0 };
        SendMessageW(tab, TCM_INSERTITEMW, i, &item as *const _ as isize);
    }
    mk(hwnd, inst, "STATIC", "Date picker", SS_LEFT, 428, 266, 120, 16, 0);
    mk(hwnd, inst, "SysDateTimePick32", "", WS_TABSTOP, 428, 284, 150, 24, ID_DATE);
    mk(hwnd, inst, "STATIC", "IP address", SS_LEFT, 596, 266, 120, 16, 0);
    mk(hwnd, inst, "SysIPAddress32", "", WS_TABSTOP, 596, 284, 154, 24, ID_IP);
    mk(hwnd, inst, "STATIC", "Hot key", SS_LEFT, 428, 314, 120, 16, 0);
    mk(hwnd, inst, "msctls_hotkey32", "", WS_BORDER | WS_TABSTOP, 428, 332, 150, 22, ID_HOTKEY);

    let sb = mk(hwnd, inst, "msctls_statusbar32", "Ready - interact with any control", 0, 0, 0, 0, 0, ID_STATUS);
    let parts = [560i32, -1];
    SendMessageW(sb, SB_SETPARTS, 2, parts.as_ptr() as isize);
    let t = w("Plonk controls demo");
    SendMessageW(sb, SB_SETTEXTW, 1, t.as_ptr() as isize);
}

unsafe fn control_name(id: i32) -> &'static str {
    match id {
        ID_PUSH => "Push button clicked", ID_DEFPUSH => "Default button clicked", ID_CHECK => "Check box toggled", ID_TRI => "Three-state toggled",
        ID_R1 => "Radio A selected", ID_R2 => "Radio B selected", ID_R3 => "Radio C selected", ID_EDIT => "Edit changed", ID_PASS => "Password edit changed",
        ID_MULTI => "Multi-line edit changed", ID_COMBO1 => "Combo box selection changed", ID_COMBO2 => "Editable combo changed", ID_LIST => "List box selection changed",
        ID_SPINEDIT => "Up-down value changed", _ => "",
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: UINT, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let inst = GetModuleHandleW(null());
            build(hwnd, inst);
            0
        }
        WM_SIZE => {
            let sb = GetDlgItem(hwnd, ID_STATUS);
            SendMessageW(sb, WM_SIZE, 0, 0);
            0
        }
        WM_COMMAND => {
            let (id, notify) = ((wp & 0xFFFF) as i32, ((wp >> 16) & 0xFFFF) as u32);
            if id == ID_ABOUT {
                let (t, c) = (w("Plonk controls demo\nWin32 controls running on the Plonk desktop."), w("About"));
                MessageBoxW(hwnd, t.as_ptr(), c.as_ptr(), 0x40);
                return 0;
            }
            // BN_CLICKED=0 for buttons; EN_CHANGE=0x300; CBN_SELCHANGE=1; LBN_SELCHANGE=1
            let name = control_name(id);
            if !name.is_empty() && (notify == 0 || notify == 0x300 || notify == 1) {
                if id == ID_CHECK || id == ID_TRI {
                    let state = SendMessageW(lp as HWND, BM_GETCHECK, 0, 0);
                    set_status(hwnd, &format!("{name} (state {state})"));
                } else if matches!(id, ID_EDIT | ID_PASS | ID_MULTI | ID_COMBO2) {
                    let mut buf = [0u16; 64];
                    let n = GetWindowTextW(lp as HWND, buf.as_mut_ptr(), 64) as usize;
                    set_status(hwnd, &format!("{name}: \"{}\"", String::from_utf16_lossy(&buf[..n.min(40)]).replace("\r\n", " ")));
                } else {
                    set_status(hwnd, name);
                }
            }
            0
        }
        WM_HSCROLL => {
            let src = lp as HWND;
            if src == GetDlgItem(hwnd, ID_TRACK) {
                let pos = SendMessageW(src, TBM_GETPOS, 0, 0);
                SendMessageW(GetDlgItem(hwnd, ID_PROG), PBM_SETPOS, pos as usize, 0);
                set_status(hwnd, &format!("Trackbar position {pos}% -> progress bar"));
            } else if src == GetDlgItem(hwnd, ID_HSCROLL) {
                set_status(hwnd, "Scroll bar moved");
            }
            0
        }
        WM_NOTIFY => {
            let nm = &*(lp as *const NMHDR);
            match (nm.idFrom as i32, nm.code) {
                (ID_LV, LVN_ITEMCHANGED) => {
                    let n = &*(lp as *const NMLISTVIEW);
                    if n.uNewState & 2 != 0 { set_status(hwnd, &format!("List view: row {} selected", n.iItem + 1)); }
                }
                (ID_TV, TVN_SELCHANGEDW) => set_status(hwnd, "Tree view: selection changed"),
                (ID_TAB, TCN_SELCHANGE) => {
                    let sel = SendMessageW(nm.hwndFrom, 0x1300 + 11, 0, 0);
                    set_status(hwnd, &format!("Tab {} selected", sel + 1));
                }
                _ => {}
            }
            0
        }
        WM_CLOSE | WM_DESTROY => {
            if msg == WM_CLOSE { return DefWindowProcW(hwnd, msg, wp, lp); }
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn main() {
    unsafe {
        let inst = GetModuleHandleW(null());
        InitCommonControlsEx(&INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_ALL });
        let class = w("PlonkControlsDemo");
        let wc = WNDCLASSW {
            style: 0, lpfnWndProc: wndproc, cbClsExtra: 0, cbWndExtra: 0, hInstance: inst, hIcon: 0,
            hCursor: LoadCursorW(0, IDC_ARROW), hbrBackground: COLOR_BTNFACE + 1, lpszMenuName: null(), lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let title = w("Win32 controls - Rust on Plonk");
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_VISIBLE, 40, 30, 780, 460, 0, 0, inst, null_mut());
        ShowWindow(hwnd, 5);
        UpdateWindow(hwnd);
        let mut msg = std::mem::zeroed::<MSG>();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
