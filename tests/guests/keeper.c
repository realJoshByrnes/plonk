// Invisible session keeper: Wine ends its session (explorer, services) when the last user process exits, so the web page
// keeps this one running. It touches the desktop once so explorer is started, then sleeps forever.
#include <windows.h>
int WINAPI WinMain(HINSTANCE h, HINSTANCE p, LPSTR cmd, int show) {
    GetDesktopWindow();
    Sleep(INFINITE);
    return 0;
}
