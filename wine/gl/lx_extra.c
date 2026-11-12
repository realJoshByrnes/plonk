/* Hand-written part of Plonk's opengl32 for the real-Wine path (appended to the generated stubs by wine/gen_gl.py).
   The host has one double-buffered RGBA8 + depth24 pixel format and does not know about device contexts, so the entries
   that take an HDC tell it which window (and where on screen) the context draws to. */

#define PLONK_GL_BASE 0x504C1000u
#define PLONK_SYS_GLRECT (0x504C0000u + 6) /* (int a[6]: top-level hwnd, hwnd, client x, y on screen, client width, height) */

static void tell_rect(HDC hdc)
{
    HWND w = WindowFromDC(hdc);
    RECT r = {0, 0, 0, 0};
    POINT p = {0, 0};
    int a[6];
    if (!w) return;
    GetClientRect(w, &r);
    ClientToScreen(w, &p);
    a[0] = (int)(INT_PTR)GetAncestor(w, GA_ROOT); a[1] = (int)(INT_PTR)w;
    a[2] = p.x; a[3] = p.y; a[4] = r.right; a[5] = r.bottom;
    plonk_host(PLONK_SYS_GLRECT, a);
}

BOOL WINAPI wglMakeCurrent(HDC hdc, HGLRC rc)
{
    void *a[2] = { hdc, rc };
    if (rc) tell_rect(hdc);
    return plonk_host(PLONK_GL_BASE + IDX_wglMakeCurrent, a);
}

BOOL WINAPI wglSwapBuffers(HDC hdc)
{
    void *a[1] = { hdc };
    tell_rect(hdc);
    return plonk_host(PLONK_GL_BASE + IDX_wglSwapBuffers, a);
}

const unsigned char * WINAPI glGetString(unsigned name)
{
    switch (name) {
    case 0x1F00: return (const unsigned char *)"Plonk";                  /* GL_VENDOR */
    case 0x1F01: return (const unsigned char *)"Plonk GL (browser GPU)"; /* GL_RENDERER */
    case 0x1F02: return (const unsigned char *)"1.5 Plonk";              /* GL_VERSION */
    case 0x1F03: return (const unsigned char *)"GL_EXT_bgra";            /* GL_EXTENSIONS */
    }
    return 0;
}

/* gdi32's ChoosePixelFormat / DescribePixelFormat / SetPixelFormat / GetPixelFormat / SwapBuffers forward to these. */
int WINAPI wglDescribePixelFormat(HDC hdc, int format, UINT size, PIXELFORMATDESCRIPTOR *pfd)
{
    if (pfd && size >= sizeof(*pfd) && format == 1) {
        unsigned i;
        for (i = 0; i < sizeof(*pfd); i++) ((char *)pfd)[i] = 0;
        pfd->nSize = sizeof(*pfd); pfd->nVersion = 1;
        pfd->dwFlags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER;
        pfd->iPixelType = PFD_TYPE_RGBA; pfd->cColorBits = 32;
        pfd->cRedBits = 8; pfd->cRedShift = 16; pfd->cGreenBits = 8; pfd->cGreenShift = 8; pfd->cBlueBits = 8; pfd->cBlueShift = 0;
        pfd->cAlphaBits = 8; pfd->cAlphaShift = 24; pfd->cDepthBits = 24;
    }
    return 1;
}
int WINAPI wglChoosePixelFormat(HDC hdc, const PIXELFORMATDESCRIPTOR *pfd) { return 1; }
BOOL WINAPI wglSetPixelFormat(HDC hdc, int format, const PIXELFORMATDESCRIPTOR *pfd) { return format == 1; }
int WINAPI wglGetPixelFormat(HDC hdc) { return 1; }

/* Extension / core>=1.2 lookups: succeed only for functions this DLL actually exports. */
PROC WINAPI wglGetProcAddress(LPCSTR name)
{
    static HMODULE self;
    if (!self) GetModuleHandleExA(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, (LPCSTR)&wglGetProcAddress, &self);
    return self ? (PROC)GetProcAddress(self, name) : 0;
}

BOOL WINAPI DllMain(HINSTANCE h, DWORD why, LPVOID r) { return TRUE; }
