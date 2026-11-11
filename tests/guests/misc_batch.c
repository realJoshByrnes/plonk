/* Probes clipboard / keyboard layout / display / DPI / window station / raw input / IME / hotkey calls. */
#include <windows.h>
#include <stdio.h>

static int mons;
static BOOL CALLBACK mon_cb(HMONITOR m, HDC dc, LPRECT r, LPARAM lp) {
    mons++;
    printf("monitor rect=%ld,%ld,%ld,%ld\n", r->left, r->top, r->right, r->bottom);
    return TRUE;
}

int main(void) {
    int sw = GetSystemMetrics(SM_CXSCREEN), sh = GetSystemMetrics(SM_CYSCREEN);
    printf("screen=%dx%d\n", sw, sh);

    /* clipboard */
    printf("clip count0=%d viewer=%d owner=%d\n", CountClipboardFormats(), GetClipboardViewer() != NULL, GetClipboardOwner() != NULL);
    int la = AddClipboardFormatListener(GetDesktopWindow());
    int lr = RemoveClipboardFormatListener(GetDesktopWindow());
    int lr2 = RemoveClipboardFormatListener(GetDesktopWindow());
    printf("listener add=%d remove=%d remove2=%d\n", la, lr, lr2);

    /* keyboard layout */
    char kl[KL_NAMELENGTH]; HKL list[4];
    GetKeyboardLayoutNameA(kl);
    printf("kl=%s n=%d\n", kl, GetKeyboardLayoutList(4, list));

    /* display */
    EnumDisplayMonitors(NULL, NULL, mon_cb, 0);
    printf("monitors=%d\n", mons);
    DISPLAY_DEVICEA dd; dd.cb = sizeof dd;
    int e1 = EnumDisplayDevicesA(NULL, 1, &dd, 0);
    int e0 = EnumDisplayDevicesA(NULL, 0, &dd, 0);
    printf("adapter0=%d adapter1=%d\n", e0, e1);
    EnumDisplayDevicesA(NULL, 0, &dd, 0);
    printf("name=%s primary=%d\n", dd.DeviceName, (dd.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE) != 0);
    DEVMODEA dm; memset(&dm, 0, sizeof dm); dm.dmSize = sizeof dm;
    BOOL ok = EnumDisplaySettingsA(NULL, ENUM_CURRENT_SETTINGS, &dm);
    printf("settings ok=%d %lux%lu bpp=%lu hz=%lu\n", ok, dm.dmPelsWidth, dm.dmPelsHeight, dm.dmBitsPerPel, dm.dmDisplayFrequency);
    printf("change=%ld\n", ChangeDisplaySettingsA(NULL, 0));

    /* DPI */
    UINT dx = 0, dy = 0;
    HMODULE sh32 = LoadLibraryA("shcore.dll");
    printf("dpi ctx=%d\n", SetProcessDPIAware());
    (void)sh32; (void)dx; (void)dy;

    /* window station / desktop */
    HWINSTA ws = GetProcessWindowStation();
    HDESK dk = GetThreadDesktop(GetCurrentThreadId());
    printf("winsta=%d desktop=%d\n", ws != NULL, dk != NULL);

    /* raw input */
    UINT n = 99;
    UINT r = GetRawInputDeviceList(NULL, &n, sizeof(RAWINPUTDEVICELIST));
    printf("rawlist ret=%u n=%u\n", r, n);
    RAWINPUTDEVICE rid; rid.usUsagePage = 1; rid.usUsage = 2; rid.dwFlags = 0; rid.hwndTarget = NULL;
    printf("rawreg=%d\n", RegisterRawInputDevices(&rid, 1, sizeof rid));
    n = 0; GetRegisteredRawInputDevices(NULL, &n, sizeof rid);
    printf("rawregistered n=%u\n", n);

    /* hotkeys */
    int h1 = RegisterHotKey(NULL, 1, MOD_CONTROL, 'A');
    int h2 = RegisterHotKey(NULL, 1, MOD_ALT, 'B');
    int h3 = UnregisterHotKey(NULL, 1);
    int h4 = UnregisterHotKey(NULL, 1);
    printf("hotkey reg=%d dup=%d unreg=%d unreg2=%d\n", h1, h2, h3, h4);
    printf("done\n");
    return 0;
}
