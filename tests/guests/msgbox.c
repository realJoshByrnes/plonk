/* MessageBox layout probe: Yes/No box with a long two-line text and a question icon. */
#include <windows.h>
int main(void) {
    int r = MessageBoxA(0, "Direct X can not find your sound card.\nDo you wish to continue this application without sound?", "DX-Ball", MB_YESNO | MB_ICONQUESTION);
    return r == IDYES ? 10 : r == IDNO ? 11 : 12;
}
