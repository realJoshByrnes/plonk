/*
 * Plonk host ABI, shared between wineplonk.drv and the Plonk kernel.
 *
 * The host kernel implements extra syscalls reachable with libc
 * syscall(nr, ...) (int 0x80). All structures are plain little-endian
 * 32-bit ints.
 */

#ifndef __PLONK_ABI_H
#define __PLONK_ABI_H

#define PLONK_SYS_SCREEN  (0x504C0000 + 0) /* (int wh[2]) -> 0; writes screen width, height */
#define PLONK_SYS_WINDOW  (0x504C0000 + 1) /* (const struct plonk_window *) -> 0; create or update a top-level window */
#define PLONK_SYS_DESTROY (0x504C0000 + 2) /* (unsigned hwnd) */
#define PLONK_SYS_FLUSH   (0x504C0000 + 3) /* (unsigned hwnd, const void *bits, int width, int height, const int dirty[4] l,t,r,b);
                                              bits = whole surface, 32bpp BGRX, top-down, stride width*4 */
#define PLONK_SYS_TITLE   (0x504C0000 + 4) /* (unsigned hwnd, const unsigned short *utf16, int len) */
#define PLONK_SYS_CAPTION (0x504C0000 + 7) /* (int a[8]: hwnd, flags, caption left, top, right, bottom (screen), button width): the host draws the title bar.
                                              flags: 1 has a caption, 2 minimize, 4 maximize, 8 close (enabled), 16 maximized, 32 tool window */
#define PLONK_SYS_CURSOR  (0x504C0000 + 5) /* (unsigned hwnd, int kind) */

/* cursor kinds for PLONK_SYS_CURSOR */
#define PLONK_CURSOR_HIDDEN   (-1)
#define PLONK_CURSOR_ARROW    0
#define PLONK_CURSOR_IBEAM    1
#define PLONK_CURSOR_WAIT     2
#define PLONK_CURSOR_CROSS    3
#define PLONK_CURSOR_SIZENWSE 4
#define PLONK_CURSOR_SIZENESW 5
#define PLONK_CURSOR_SIZEWE   6
#define PLONK_CURSOR_SIZENS   7
#define PLONK_CURSOR_SIZEALL  8
#define PLONK_CURSOR_NO       9
#define PLONK_CURSOR_HAND     10

struct plonk_window
{
    unsigned hwnd, owner;
    int x, y, w, h;          /* window rect, screen coords */
    int cx, cy, cw, ch;      /* client rect, screen coords */
    unsigned style, ex_style;
    unsigned visible;        /* 0/1 */
    unsigned insert_after;   /* hwnd it was placed behind, 0 = top (HWND_TOP) */
};

/* Input: open("/dev/plonk/events", O_RDONLY | O_NONBLOCK). The fd is readable
 * (poll POLLIN) when events are queued; read() returns a whole number of these
 * records (never partial) or -1/EAGAIN. */
#define PLONK_EVENTS_PATH "/dev/plonk/events"

#define PLONK_EVENT_MOUSE_MOVE    1
#define PLONK_EVENT_BUTTON_DOWN   2
#define PLONK_EVENT_BUTTON_UP     3
#define PLONK_EVENT_WHEEL         4
#define PLONK_EVENT_KEY_DOWN      5
#define PLONK_EVENT_KEY_UP        6
#define PLONK_EVENT_CLOSE         7
#define PLONK_EVENT_SCREEN_RESIZE 8

struct plonk_event
{
    unsigned type;   /* PLONK_EVENT_* */
    unsigned hwnd;   /* top-level window under the pointer / with focus as known to the host, or 0 */
    int x, y;        /* absolute screen coords (mouse); new width, height for type 8 */
    unsigned data;   /* button: 0 left 1 right 2 middle; wheel: delta (signed, 120 per notch); key: Windows virtual-key code */
    unsigned flags;  /* key: hardware scan code */
    unsigned time;   /* ms tick */
    unsigned pad;
};

#endif /* __PLONK_ABI_H */
