/* OpenGL demo: spinning lit, textured cube + a stats bar (FPS, frame time, frames, GL strings). */
#include <windows.h>
#include <GL/gl.h>
#include <stdio.h>

#define GLW 640
#define GLH 420
static HWND hmain, hgl, hstat;
static HDC hdc;
static HGLRC rc;
static float angle;
static DWORD t_last, t_fps;
static int frames, frames_total;
static float fps, ms;
static GLuint tex;

static void init_gl(void) {
    PIXELFORMATDESCRIPTOR pfd = { sizeof pfd, 1, PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER, PFD_TYPE_RGBA, 32, 0,0,0,0,0,0,0,0,0,0,0,0,0, 24, 0, 0, PFD_MAIN_PLANE, 0, 0, 0, 0 };
    hdc = GetDC(hgl);
    SetPixelFormat(hdc, ChoosePixelFormat(hdc, &pfd), &pfd);
    rc = wglCreateContext(hdc);
    wglMakeCurrent(hdc, rc);

    unsigned char img[64 * 64 * 4];
    for (int y = 0; y < 64; y++) for (int x = 0; x < 64; x++) {
        int c = ((x / 8) ^ (y / 8)) & 1;
        unsigned char *p = img + (y * 64 + x) * 4;
        p[0] = c ? 240 : 60; p[1] = c ? 200 : 90; p[2] = c ? 80 : 200; p[3] = 255;
    }
    glGenTextures(1, &tex);
    glBindTexture(GL_TEXTURE_2D, tex);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 64, 64, 0, GL_RGBA, GL_UNSIGNED_BYTE, img);

    glEnable(GL_DEPTH_TEST);
    glEnable(GL_CULL_FACE);
    glEnable(GL_TEXTURE_2D);
    glEnable(GL_LIGHTING);
    glEnable(GL_LIGHT0);
    glEnable(GL_NORMALIZE);
    glClearColor(0.07f, 0.09f, 0.16f, 1.0f);
    GLfloat pos[4] = { 2.0f, 3.0f, 4.0f, 1.0f }, amb[4] = { 0.25f, 0.25f, 0.3f, 1 }, dif[4] = { 1, 1, 0.95f, 1 };
    glLightfv(GL_LIGHT0, GL_POSITION, pos);
    glLightfv(GL_LIGHT0, GL_AMBIENT, amb);
    glLightfv(GL_LIGHT0, GL_DIFFUSE, dif);
    glViewport(0, 0, GLW, GLH);
    glMatrixMode(GL_PROJECTION);
    glLoadIdentity();
    glFrustum(-0.8 * 0.6, 0.8 * 0.6, -0.6 * 0.6, 0.6 * 0.6, 1.0, 20.0);
    glMatrixMode(GL_MODELVIEW);
}

static void quad(float nx, float ny, float nz, const float v[4][3]) {
    static const float uv[4][2] = { {0,0}, {1,0}, {1,1}, {0,1} };
    glNormal3f(nx, ny, nz);
    for (int i = 0; i < 4; i++) { glTexCoord2f(uv[i][0], uv[i][1]); glVertex3fv(v[i]); }
}

static void draw(void) {
    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    glLoadIdentity();
    glTranslatef(0, 0, -4.5f);
    glRotatef(angle, 1, 1, 0);
    glRotatef(angle * 0.6f, 0, 1, 0);
    glColor3f(1, 1, 1);
    glBegin(GL_QUADS);
    static const float f[6][4][3] = {
        {{-1,-1, 1},{ 1,-1, 1},{ 1, 1, 1},{-1, 1, 1}}, {{ 1,-1,-1},{-1,-1,-1},{-1, 1,-1},{ 1, 1,-1}},
        {{-1,-1,-1},{-1,-1, 1},{-1, 1, 1},{-1, 1,-1}}, {{ 1,-1, 1},{ 1,-1,-1},{ 1, 1,-1},{ 1, 1, 1}},
        {{-1, 1, 1},{ 1, 1, 1},{ 1, 1,-1},{-1, 1,-1}}, {{-1,-1,-1},{ 1,-1,-1},{ 1,-1, 1},{-1,-1, 1}} };
    static const float n[6][3] = { {0,0,1}, {0,0,-1}, {-1,0,0}, {1,0,0}, {0,1,0}, {0,-1,0} };
    for (int i = 0; i < 6; i++) quad(n[i][0], n[i][1], n[i][2], f[i]);
    glEnd();
    SwapBuffers(hdc);
}

static void tick(void) {
    DWORD now = GetTickCount();
    ms = (float)(now - t_last);
    t_last = now;
    angle += 1.2f;
    draw();
    frames++; frames_total++;
    if (now - t_fps >= 1000) {
        fps = frames * 1000.0f / (now - t_fps);
        frames = 0; t_fps = now;
        char buf[200];
        const char *r = (const char *)glGetString(GL_RENDERER);
        sprintf(buf, "FPS: %.1f   frame: %.1f ms   frames: %d   %s", fps, ms, frames_total, r ? r : "?");
        SetWindowTextA(hstat, buf);
        sprintf(buf, "GL cube - %.0f FPS", fps);
        SetWindowTextA(hmain, buf);
    }
}

static LRESULT CALLBACK proc(HWND h, UINT m, WPARAM w, LPARAM l) {
    switch (m) {
    case WM_TIMER: tick(); return 0;
    case WM_DESTROY: KillTimer(h, 1); PostQuitMessage(0); return 0;
    }
    return DefWindowProcA(h, m, w, l);
}

int WINAPI WinMain(HINSTANCE hi, HINSTANCE hp, LPSTR cmd, int show) {
    WNDCLASSA wc = { 0 };
    wc.lpfnWndProc = proc; wc.hInstance = hi; wc.hCursor = LoadCursor(0, IDC_ARROW);
    wc.hbrBackground = (HBRUSH)(COLOR_BTNFACE + 1); wc.lpszClassName = "GLDemo";
    RegisterClassA(&wc);
    wc.lpszClassName = "GLView"; wc.style = CS_OWNDC; wc.hbrBackground = 0;
    RegisterClassA(&wc);
    RECT r = { 0, 0, GLW, GLH + 28 };
    AdjustWindowRect(&r, WS_OVERLAPPEDWINDOW, FALSE);
    hmain = CreateWindowA("GLDemo", "GL cube", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 40, 30, r.right - r.left, r.bottom - r.top, 0, 0, hi, 0);
    hgl = CreateWindowA("GLView", "", WS_CHILD | WS_VISIBLE, 0, 0, GLW, GLH, hmain, 0, hi, 0);
    hstat = CreateWindowA("STATIC", "starting...", WS_CHILD | WS_VISIBLE, 6, GLH + 5, GLW - 12, 20, hmain, 0, hi, 0);
    init_gl();
    t_last = t_fps = GetTickCount();
    SetTimer(hmain, 1, 10, 0);
    MSG msg;
    while (GetMessageA(&msg, 0, 0, 0) > 0) { TranslateMessage(&msg); DispatchMessageA(&msg); }
    wglMakeCurrent(0, 0);
    wglDeleteContext(rc);
    return 0;
}
