typedef void *H;
__declspec(dllimport) __attribute__((stdcall)) H GetStdHandle(unsigned n);
__declspec(dllimport) __attribute__((stdcall)) int WriteFile(H h, const void *b, unsigned n, unsigned *w, void *o);
__declspec(dllimport) __attribute__((stdcall)) void ExitProcess(unsigned c);
void __attribute__((stdcall)) start(void) {
    static const char msg[] = "Hello from Plonk\r\n";
    unsigned w;
    H out = GetStdHandle(-11);
    WriteFile(out, msg, sizeof msg - 1, &w, 0);
    ExitProcess(w == sizeof msg - 1 ? 0 : 1);
}
