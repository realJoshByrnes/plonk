/*
 * wineplonk.drv window, surface and input handling
 *
 * Modelled on wineandroid.drv / winewayland.drv.
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */

#if 0
#pragma makedep unix
#endif

#include "config.h"

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#define OEMRESOURCE

#include "plonkdrv.h"
#include "wine/server.h"
#include "wine/debug.h"

WINE_DEFAULT_DEBUG_CHANNEL(plonk);

/* private window data */
struct plonk_win_data
{
    HWND                   hwnd;          /* hwnd that this private data belongs to */
    BOOL                   toplevel;      /* parent is the desktop window */
    BOOL                   host_window;   /* the host has been told about this window */
    RECT                   window_rect;   /* window rectangle relative to parent */
    RECT                   client_rect;   /* client area relative to parent */
    struct window_surface *surface;
};

#define SWP_AGG_NOPOSCHANGE (SWP_NOSIZE | SWP_NOMOVE | SWP_NOCLIENTSIZE | SWP_NOCLIENTMOVE | SWP_NOZORDER)

static pthread_mutex_t win_data_mutex = PTHREAD_MUTEX_INITIALIZER;
static struct plonk_win_data *win_data_context[32768];

static inline int context_idx( HWND hwnd )
{
    return LOWORD( hwnd ) >> 1;
}

static inline unsigned int host_hwnd( HWND hwnd )
{
    return HandleToULong( hwnd );
}

/***********************************************************************
 *           win data helpers
 */
static struct plonk_win_data *alloc_win_data( HWND hwnd )
{
    struct plonk_win_data *data;

    if ((data = calloc( 1, sizeof(*data) )))
    {
        data->hwnd = hwnd;
        pthread_mutex_lock( &win_data_mutex );
        win_data_context[context_idx( hwnd )] = data;
    }
    return data;
}

/* must be called with the data held; releases the lock */
static void free_win_data( struct plonk_win_data *data )
{
    win_data_context[context_idx( data->hwnd )] = NULL;
    pthread_mutex_unlock( &win_data_mutex );
    free( data );
}

/* returns the data with the lock held; use release_win_data when done */
static struct plonk_win_data *get_win_data( HWND hwnd )
{
    struct plonk_win_data *data;

    if (!hwnd) return NULL;
    pthread_mutex_lock( &win_data_mutex );
    if ((data = win_data_context[context_idx( hwnd )]) && data->hwnd == hwnd) return data;
    pthread_mutex_unlock( &win_data_mutex );
    return NULL;
}

static void release_win_data( struct plonk_win_data *data )
{
    if (data) pthread_mutex_unlock( &win_data_mutex );
}

static struct plonk_win_data *create_win_data( HWND hwnd, const RECT *window_rect, const RECT *client_rect )
{
    struct plonk_win_data *data;
    HWND parent;

    if (!(parent = NtUserGetAncestor( hwnd, GA_PARENT ))) return NULL;  /* desktop or HWND_MESSAGE */

    if (!(data = alloc_win_data( hwnd ))) return NULL;

    data->toplevel = (parent == NtUserGetDesktopWindow());
    data->window_rect = *window_rect;
    data->client_rect = *client_rect;
    return data;
}


/***********************************************************************
 *           host window helpers
 */
static void send_host_title( HWND hwnd, const WCHAR *text )
{
    int len = 0;

    while (text[len]) len++;
    plonk_syscall3( PLONK_SYS_TITLE, host_hwnd( hwnd ), (long)text, len );
}

static void send_host_caption( HWND hwnd, UINT style, UINT ex_style )
{
    int a[8];
    TITLEBARINFO tb = { .cbSize = sizeof(tb) };

    memset( a, 0, sizeof(a) );
    a[0] = host_hwnd( hwnd );
    if ((style & WS_CAPTION) == WS_CAPTION && NtUserGetTitleBarInfo( hwnd, &tb ))
    {
        int cx = NtUserGetSystemMetrics( SM_CXSIZE );
        BOOL tool = (ex_style & WS_EX_TOOLWINDOW) != 0;
        a[1] = 1 | ((style & WS_MINIMIZEBOX) ? 2 : 0) | ((style & WS_MAXIMIZEBOX) ? 4 : 0) |
               ((style & WS_SYSMENU) && !(tb.rgstate[5] & STATE_SYSTEM_UNAVAILABLE) ? 8 : 0) |
               ((style & WS_MAXIMIZE) ? 16 : 0) | (tool ? 32 : 0);
        a[2] = tb.rcTitleBar.left - (tool ? 0 : cx); /* the icon area is part of the bar */
        a[3] = tb.rcTitleBar.top; a[4] = tb.rcTitleBar.right; a[5] = tb.rcTitleBar.bottom;
        a[6] = cx;
    }
    plonk_syscall1( PLONK_SYS_CAPTION, (long)a );
}

static void send_host_window( HWND hwnd, const RECT *window_rect, const RECT *client_rect,
                              UINT style, UINT ex_style, BOOL first )
{
    struct plonk_window info;

    memset( &info, 0, sizeof(info) );
    info.hwnd         = host_hwnd( hwnd );
    info.owner        = host_hwnd( NtUserGetWindowRelative( hwnd, GW_OWNER ) );
    info.x            = window_rect->left;
    info.y            = window_rect->top;
    info.w            = window_rect->right - window_rect->left;
    info.h            = window_rect->bottom - window_rect->top;
    info.cx           = client_rect->left;
    info.cy           = client_rect->top;
    info.cw           = client_rect->right - client_rect->left;
    info.ch           = client_rect->bottom - client_rect->top;
    info.style        = style;
    info.ex_style     = ex_style;
    info.visible      = (style & WS_VISIBLE) != 0;
    /* always report the real current position in the Z-order: the sibling
     * directly above us, or 0 if we are at the top */
    info.insert_after = host_hwnd( NtUserGetWindowRelative( hwnd, GW_HWNDPREV ) );

    TRACE( "hwnd %p owner %x window %s client %s style %08x/%08x visible %u after %x\n",
           hwnd, info.owner, wine_dbgstr_rect( window_rect ), wine_dbgstr_rect( client_rect ),
           style, ex_style, info.visible, info.insert_after );

    if (plonk_syscall1( PLONK_SYS_WINDOW, (long)&info ) < 0)
        ERR( "PLONK_SYS_WINDOW failed for %p, errno %d\n", hwnd, errno );
    send_host_caption( hwnd, style, ex_style );

    if (first)
    {
        /* the title was set before the host knew about the window */
        WCHAR text[256];

        text[0] = 0;
        NtUserInternalGetWindowText( hwnd, text, ARRAY_SIZE( text ) );
        if (text[0]) send_host_title( hwnd, text );
    }
}


/***********************************************************************
 *           Window surface support
 */
struct plonk_window_surface
{
    struct window_surface header;
    HWND                  hwnd;
    RECT                  bounds;
    void                 *bits;
    pthread_mutex_t       mutex;
    BITMAPINFO            info;
};

static struct plonk_window_surface *get_plonk_surface( struct window_surface *surface )
{
    return (struct plonk_window_surface *)surface;
}

static inline void reset_bounds( RECT *bounds )
{
    bounds->left = bounds->top = INT_MAX;
    bounds->right = bounds->bottom = INT_MIN;
}

static void plonk_surface_lock( struct window_surface *window_surface )
{
    struct plonk_window_surface *surface = get_plonk_surface( window_surface );

    pthread_mutex_lock( &surface->mutex );
}

static void plonk_surface_unlock( struct window_surface *window_surface )
{
    struct plonk_window_surface *surface = get_plonk_surface( window_surface );

    pthread_mutex_unlock( &surface->mutex );
}

static void *plonk_surface_get_bitmap_info( struct window_surface *window_surface, BITMAPINFO *info )
{
    struct plonk_window_surface *surface = get_plonk_surface( window_surface );

    memcpy( info, &surface->info, sizeof(surface->info.bmiHeader) );
    return surface->bits;
}

static RECT *plonk_surface_get_bounds( struct window_surface *window_surface )
{
    struct plonk_window_surface *surface = get_plonk_surface( window_surface );

    return &surface->bounds;
}

static void plonk_surface_set_region( struct window_surface *window_surface, HRGN region )
{
    /* TODO: shaped windows are not supported by the host ABI */
}

static void plonk_surface_flush( struct window_surface *window_surface )
{
    struct plonk_window_surface *surface = get_plonk_surface( window_surface );
    int width = surface->header.rect.right - surface->header.rect.left;
    int height = surface->header.rect.bottom - surface->header.rect.top;
    int dirty[4];

    window_surface->funcs->lock( window_surface );

    dirty[0] = max( surface->bounds.left, 0 );
    dirty[1] = max( surface->bounds.top, 0 );
    dirty[2] = min( surface->bounds.right, width );
    dirty[3] = min( surface->bounds.bottom, height );

    if (dirty[0] < dirty[2] && dirty[1] < dirty[3])
    {
        TRACE( "flushing %p hwnd %p %dx%d dirty (%d,%d)-(%d,%d)\n", surface, surface->hwnd,
               width, height, dirty[0], dirty[1], dirty[2], dirty[3] );

        if (plonk_syscall5( PLONK_SYS_FLUSH, host_hwnd( surface->hwnd ), (long)surface->bits,
                            width, height, (long)dirty ) < 0)
            WARN( "PLONK_SYS_FLUSH failed for %p, errno %d\n", surface->hwnd, errno );
    }
    reset_bounds( &surface->bounds );

    window_surface->funcs->unlock( window_surface );
}

static void plonk_surface_destroy( struct window_surface *window_surface )
{
    struct plonk_window_surface *surface = get_plonk_surface( window_surface );

    TRACE( "freeing %p bits %p\n", surface, surface->bits );

    pthread_mutex_destroy( &surface->mutex );
    free( surface->bits );
    free( surface );
}

static const struct window_surface_funcs plonk_surface_funcs =
{
    plonk_surface_lock,
    plonk_surface_unlock,
    plonk_surface_get_bitmap_info,
    plonk_surface_get_bounds,
    plonk_surface_set_region,
    plonk_surface_flush,
    plonk_surface_destroy
};

static struct window_surface *create_surface( HWND hwnd, const RECT *rect )
{
    struct plonk_window_surface *surface;
    int width = rect->right - rect->left, height = rect->bottom - rect->top;
    pthread_mutexattr_t attr;

    if (!(surface = calloc( 1, sizeof(*surface) ))) return NULL;

    surface->info.bmiHeader.biSize        = sizeof(surface->info.bmiHeader);
    surface->info.bmiHeader.biWidth       = width;
    surface->info.bmiHeader.biHeight      = -height; /* top-down */
    surface->info.bmiHeader.biPlanes      = 1;
    surface->info.bmiHeader.biBitCount    = 32;
    surface->info.bmiHeader.biCompression = BI_RGB;
    surface->info.bmiHeader.biSizeImage   = width * height * 4;

    pthread_mutexattr_init( &attr );
    pthread_mutexattr_settype( &attr, PTHREAD_MUTEX_RECURSIVE );
    pthread_mutex_init( &surface->mutex, &attr );
    pthread_mutexattr_destroy( &attr );

    surface->header.funcs = &plonk_surface_funcs;
    surface->header.rect  = *rect;
    surface->header.ref   = 1;
    surface->hwnd         = hwnd;
    reset_bounds( &surface->bounds );

    if (!(surface->bits = calloc( 1, surface->info.bmiHeader.biSizeImage )))
    {
        plonk_surface_destroy( &surface->header );
        return NULL;
    }

    TRACE( "created %p hwnd %p %s bits %p-%p\n", surface, hwnd, wine_dbgstr_rect( rect ),
           surface->bits, (char *)surface->bits + surface->info.bmiHeader.biSizeImage );

    return &surface->header;
}


/***********************************************************************
 *           Input events
 */

struct plonk_queued_event
{
    struct list        entry;
    struct plonk_event event;
};

struct plonk_thread_data
{
    int         fd;          /* events fd, or -1 if it could not be opened */
    struct list queue;       /* events read from the fd but not yet processed */
    BOOL        processing;  /* inside process_events, to avoid nested processing */
    HWND        last_hwnd;   /* last window reported under the pointer */
};

static inline struct plonk_thread_data *plonk_thread_data(void)
{
    return (struct plonk_thread_data *)(UINT_PTR)NtUserGetThreadInfo()->driver_data;
}

/***********************************************************************
 *           plonk_thread_init
 *
 * Open the events fd for the current thread and register it with the
 * server, so that the thread's message queue gets woken up for host events.
 */
static struct plonk_thread_data *plonk_thread_init(void)
{
    struct plonk_thread_data *data = plonk_thread_data();
    HANDLE handle;
    int ret;

    if (data) return data;

    if (!(data = calloc( 1, sizeof(*data) )))
    {
        ERR( "could not create thread data\n" );
        return NULL;
    }
    list_init( &data->queue );
    NtUserGetThreadInfo()->driver_data = (UINT_PTR)data;

    if ((data->fd = open( PLONK_EVENTS_PATH, O_RDONLY | O_NONBLOCK | O_CLOEXEC )) == -1)
    {
        ERR( "could not open %s, errno %d\n", PLONK_EVENTS_PATH, errno );
        return data;
    }
    if (wine_server_fd_to_handle( data->fd, GENERIC_READ | SYNCHRONIZE, 0, &handle ))
    {
        ERR( "can't allocate handle for events fd\n" );
        close( data->fd );
        data->fd = -1;
        return data;
    }
    SERVER_START_REQ( set_queue_fd )
    {
        req->handle = wine_server_obj_handle( handle );
        ret = wine_server_call( req );
    }
    SERVER_END_REQ;
    NtClose( handle );
    if (ret)
    {
        ERR( "can't store handle for events fd, status %x\n", ret );
        close( data->fd );
        data->fd = -1;
        return data;
    }

    TRACE( "thread %04x events fd %d\n", (int)GetCurrentThreadId(), data->fd );
    return data;
}

/***********************************************************************
 *           PLONK_ThreadDetach
 */
void PLONK_ThreadDetach(void)
{
    struct plonk_thread_data *data = plonk_thread_data();
    struct plonk_queued_event *event, *next;

    if (!data) return;

    LIST_FOR_EACH_ENTRY_SAFE( event, next, &data->queue, struct plonk_queued_event, entry )
    {
        list_remove( &event->entry );
        free( event );
    }
    if (data->fd != -1) close( data->fd );
    free( data );
    NtUserGetThreadInfo()->driver_data = 0;
}

/* read everything that is pending on the fd into the thread's queue */
static void pull_events( struct plonk_thread_data *data )
{
    struct plonk_event buffer[32];
    struct plonk_queued_event *queued;
    int i, res;

    for (;;)
    {
        res = read( data->fd, buffer, sizeof(buffer) );
        if (res == -1 && errno == EINTR) continue;
        if (res < (int)sizeof(buffer[0])) break;

        for (i = 0; i < res / (int)sizeof(buffer[0]); i++)
        {
            if (!(queued = malloc( sizeof(*queued) ))) return;
            queued->event = buffer[i];
            list_add_tail( &data->queue, &queued->entry );
        }
        if (res < (int)sizeof(buffer)) break;
    }
}

static void handle_mouse_event( struct plonk_thread_data *data, const struct plonk_event *event )
{
    HWND hwnd = ULongToHandle( event->hwnd );
    INPUT input;

    memset( &input, 0, sizeof(input) );
    input.type           = INPUT_MOUSE;
    input.mi.dx          = event->x;   /* absolute screen pixels, as for the other drivers */
    input.mi.dy          = event->y;
    input.mi.mouseData   = 0;
    input.mi.time        = 0;          /* let the server stamp it, host ticks have a different base */
    input.mi.dwExtraInfo = 0;
    input.mi.dwFlags     = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE;

    switch (event->type)
    {
    case PLONK_EVENT_BUTTON_DOWN:
        switch (event->data)
        {
        case 0: input.mi.dwFlags |= MOUSEEVENTF_LEFTDOWN; break;
        case 1: input.mi.dwFlags |= MOUSEEVENTF_RIGHTDOWN; break;
        case 2: input.mi.dwFlags |= MOUSEEVENTF_MIDDLEDOWN; break;
        default: FIXME( "unknown button %u\n", event->data ); return;
        }
        break;
    case PLONK_EVENT_BUTTON_UP:
        switch (event->data)
        {
        case 0: input.mi.dwFlags |= MOUSEEVENTF_LEFTUP; break;
        case 1: input.mi.dwFlags |= MOUSEEVENTF_RIGHTUP; break;
        case 2: input.mi.dwFlags |= MOUSEEVENTF_MIDDLEUP; break;
        default: FIXME( "unknown button %u\n", event->data ); return;
        }
        break;
    case PLONK_EVENT_WHEEL:
        input.mi.dwFlags |= MOUSEEVENTF_WHEEL;
        input.mi.mouseData = event->data;
        break;
    }

    TRACE( "type %u pos %d,%d hwnd %p flags %x data %d\n", event->type, event->x, event->y,
           hwnd, (int)input.mi.dwFlags, (int)input.mi.mouseData );

    /* The host knows the real Z-order; make sure the server agrees that this
     * window is the one under the pointer. Only done when the window changes
     * or on a click, to limit the number of server calls. */
    if (hwnd && (hwnd != data->last_hwnd || event->type == PLONK_EVENT_BUTTON_DOWN))
    {
        SERVER_START_REQ( update_window_zorder )
        {
            req->window      = wine_server_user_handle( hwnd );
            req->rect.left   = event->x;
            req->rect.top    = event->y;
            req->rect.right  = event->x + 1;
            req->rect.bottom = event->y + 1;
            wine_server_call( req );
        }
        SERVER_END_REQ;
    }
    data->last_hwnd = hwnd;

    /* the server routes it to the capture window, or to the child of hwnd
     * (or of whatever is at that position if hwnd is 0) under the pointer */
    __wine_send_input( hwnd, &input, NULL );
}

static void handle_key_event( const struct plonk_event *event )
{
    INPUT input;

    memset( &input, 0, sizeof(input) );
    input.type           = INPUT_KEYBOARD;
    input.ki.wVk         = event->data & 0xffff;
    input.ki.wScan       = event->flags & 0xff;
    input.ki.time        = 0;
    input.ki.dwExtraInfo = 0;
    input.ki.dwFlags     = 0;
    if (event->type == PLONK_EVENT_KEY_UP) input.ki.dwFlags |= KEYEVENTF_KEYUP;
    /* extended keys: scan code with bit 8 set, or an 0xe0 prefix byte */
    if ((event->flags & 0x100) || (event->flags & 0xff00) == 0xe000)
        input.ki.dwFlags |= KEYEVENTF_EXTENDEDKEY;

    TRACE( "%s hwnd %x vkey %x scan %x flags %x\n", event->type == PLONK_EVENT_KEY_UP ? "KEYUP" : "KEYDOWN",
           event->hwnd, input.ki.wVk, input.ki.wScan, (int)input.ki.dwFlags );

    /* keyboard input always goes to the Wine foreground/focus window */
    __wine_send_input( 0, &input, NULL );
}

static void handle_close_event( const struct plonk_event *event )
{
    HWND hwnd = ULongToHandle( event->hwnd );

    TRACE( "close request for %p\n", hwnd );

    if (!hwnd) return;
    if (NtUserGetWindowLongW( hwnd, GWL_STYLE ) & WS_DISABLED) return;
    NtUserPostMessage( hwnd, WM_SYSCOMMAND, SC_CLOSE, 0 );
}

static void handle_activate_event( const struct plonk_event *event )
{
    HWND hwnd = ULongToHandle( event->hwnd );

    if (!hwnd) return;
    if (event->data == 2) { NtUserPostMessage( hwnd, WM_SYSCOMMAND, SC_MINIMIZE, 0 ); return; }
    if (NtUserGetWindowLongW( hwnd, GWL_STYLE ) & WS_MINIMIZE) NtUserPostMessage( hwnd, WM_SYSCOMMAND, SC_RESTORE, 0 );
    NtUserSetForegroundWindow( hwnd );
}

static BOOL process_events( struct plonk_thread_data *data, DWORD mask )
{
    struct plonk_queued_event *queued, *next;
    unsigned int count = 0;

    pull_events( data );

    if (data->processing) return FALSE;  /* don't process nested events */
    data->processing = TRUE;

    LIST_FOR_EACH_ENTRY_SAFE( queued, next, &data->queue, struct plonk_queued_event, entry )
    {
        const struct plonk_event *event = &queued->event;

        switch (event->type)
        {
        case PLONK_EVENT_MOUSE_MOVE:
            if (mask & QS_MOUSEMOVE) break;
            continue;
        case PLONK_EVENT_BUTTON_DOWN:
        case PLONK_EVENT_BUTTON_UP:
        case PLONK_EVENT_WHEEL:
            if (mask & QS_MOUSEBUTTON) break;
            continue;
        case PLONK_EVENT_KEY_DOWN:
        case PLONK_EVENT_KEY_UP:
            if (mask & QS_KEY) break;
            continue;
        default:
            if (mask & (QS_POSTMESSAGE | QS_SENDMESSAGE)) break;
            continue;
        }

        list_remove( &queued->entry );

        switch (event->type)
        {
        case PLONK_EVENT_MOUSE_MOVE:
        case PLONK_EVENT_BUTTON_DOWN:
        case PLONK_EVENT_BUTTON_UP:
        case PLONK_EVENT_WHEEL:
            handle_mouse_event( data, event );
            break;
        case PLONK_EVENT_KEY_DOWN:
        case PLONK_EVENT_KEY_UP:
            handle_key_event( event );
            break;
        case PLONK_EVENT_CLOSE:
            handle_close_event( event );
            break;
        case PLONK_EVENT_ACTIVATE:
            handle_activate_event( event );
            break;
        case PLONK_EVENT_SCREEN_RESIZE:
            TRACE( "screen resized to %dx%d\n", event->x, event->y );
            /* the display devices and the desktop window are updated from the
             * thread owning the desktop window (in explorer) */
            NtUserPostMessage( NtUserGetDesktopWindow(), WM_PLONK_SCREEN_CHANGED, 0, 0 );
            break;
        default:
            FIXME( "got unknown event %u\n", event->type );
            break;
        }
        free( queued );
        count++;
    }

    data->processing = FALSE;
    return count != 0;
}

/***********************************************************************
 *           PLONK_ProcessEvents
 */
BOOL PLONK_ProcessEvents( DWORD mask )
{
    struct plonk_thread_data *data = plonk_thread_data();

    if (!data || data->fd == -1) return FALSE;
    return process_events( data, mask );
}


/***********************************************************************
 *           PLONK_WindowMessage
 */
LRESULT PLONK_WindowMessage( HWND hwnd, UINT msg, WPARAM wp, LPARAM lp )
{
    switch (msg)
    {
    case WM_PLONK_SCREEN_CHANGED:
        plonk_init_display_devices( TRUE );
        if (hwnd == NtUserGetDesktopWindow()) plonk_resize_desktop();
        return 0;
    default:
        FIXME( "got window msg %x hwnd %p wp %lx lp %lx\n", msg, hwnd, (long)wp, (long)lp );
        return 0;
    }
}


/***********************************************************************
 *           PLONK_CreateWindow
 */
BOOL PLONK_CreateWindow( HWND hwnd )
{
    HWND desktop = NtUserGetDesktopWindow();

    TRACE( "%p\n", hwnd );

    /* threads owning the desktop or top-level windows listen for host events */
    if (hwnd == desktop || NtUserGetAncestor( hwnd, GA_PARENT ) == desktop)
        plonk_thread_init();

    return TRUE;
}


/***********************************************************************
 *           PLONK_DestroyWindow
 */
void PLONK_DestroyWindow( HWND hwnd )
{
    struct plonk_win_data *data;
    struct window_surface *surface;
    BOOL host_window;

    if (!(data = get_win_data( hwnd ))) return;

    TRACE( "%p\n", hwnd );

    surface = data->surface;
    host_window = data->host_window;
    free_win_data( data );

    if (surface) window_surface_release( surface );
    if (host_window) plonk_syscall1( PLONK_SYS_DESTROY, host_hwnd( hwnd ) );
}


/***********************************************************************
 *           PLONK_WindowPosChanging
 */
BOOL PLONK_WindowPosChanging( HWND hwnd, HWND insert_after, UINT swp_flags,
                              const RECT *window_rect, const RECT *client_rect,
                              RECT *visible_rect, struct window_surface **surface )
{
    struct plonk_win_data *data = get_win_data( hwnd );
    RECT surface_rect;

    TRACE( "win %p window %s client %s style %08x flags %08x\n",
           hwnd, wine_dbgstr_rect( window_rect ), wine_dbgstr_rect( client_rect ),
           (int)NtUserGetWindowLongW( hwnd, GWL_STYLE ), swp_flags );

    if (!data && !(data = create_win_data( hwnd, window_rect, client_rect ))) return TRUE;

    *visible_rect = *window_rect;

    /* create the window surface if necessary */

    if (!data->toplevel) goto done;
    if (swp_flags & SWP_HIDEWINDOW) goto done;

    /* the surface covers the whole window rectangle */
    SetRect( &surface_rect, 0, 0, window_rect->right - window_rect->left,
             window_rect->bottom - window_rect->top );
    if (surface_rect.right <= 0 || surface_rect.bottom <= 0) goto done;

    if (data->surface && EqualRect( &data->surface->rect, &surface_rect ))
    {
        /* existing surface is good enough */
        window_surface_add_ref( data->surface );
        if (*surface) window_surface_release( *surface );
        *surface = data->surface;
        goto done;
    }
    if (!(swp_flags & SWP_SHOWWINDOW) && !(NtUserGetWindowLongW( hwnd, GWL_STYLE ) & WS_VISIBLE))
        goto done;

    if (*surface) window_surface_release( *surface );
    *surface = create_surface( data->hwnd, &surface_rect );

done:
    release_win_data( data );
    return TRUE;
}


/***********************************************************************
 *           PLONK_WindowPosChanged
 */
void PLONK_WindowPosChanged( HWND hwnd, HWND insert_after, UINT swp_flags,
                             const RECT *window_rect, const RECT *client_rect,
                             const RECT *visible_rect, const RECT *valid_rects,
                             struct window_surface *surface )
{
    struct plonk_win_data *data;
    UINT style = NtUserGetWindowLongW( hwnd, GWL_STYLE );
    UINT ex_style = NtUserGetWindowLongW( hwnd, GWL_EXSTYLE );
    BOOL toplevel, first;

    if (!(data = get_win_data( hwnd ))) return;

    data->window_rect = *window_rect;
    data->client_rect = *client_rect;

    if (surface) window_surface_add_ref( surface );
    if (data->surface) window_surface_release( data->surface );
    data->surface = surface;

    toplevel = data->toplevel;
    first = toplevel && !data->host_window;
    if (toplevel) data->host_window = TRUE;
    release_win_data( data );

    TRACE( "win %p window %s client %s style %08x after %p flags %08x\n", hwnd,
           wine_dbgstr_rect( window_rect ), wine_dbgstr_rect( client_rect ),
           style, insert_after, swp_flags );

    if (toplevel) send_host_window( hwnd, window_rect, client_rect, style, ex_style, first );

    /* A window that made itself active before it was shown (SetFocus on a child
     * in WM_CREATE, dialogs) drew its caption inactive, because it was not the
     * foreground window yet; win32u makes it the foreground window right after
     * this call but does not repaint the caption. Ask for that once it is done. */
    if (toplevel && (swp_flags & SWP_SHOWWINDOW) && !(swp_flags & SWP_NOACTIVATE) &&
        !(style & (WS_CHILD | WS_DISABLED)) && !(ex_style & WS_EX_NOACTIVATE))
    {
        GUITHREADINFO info = { .cbSize = sizeof(info) };
        if (NtUserGetGUIThreadInfo( GetCurrentThreadId(), &info ) && info.hwndActive == hwnd)
            NtUserPostMessage( hwnd, WM_NCACTIVATE, TRUE, 0 );
    }
}


/***********************************************************************
 *           PLONK_SetParent
 */
void PLONK_SetParent( HWND hwnd, HWND parent, HWND old_parent )
{
    struct plonk_win_data *data;
    BOOL destroy = FALSE;

    if (parent == old_parent) return;
    if (!(data = get_win_data( hwnd ))) return;

    TRACE( "win %p parent %p -> %p\n", hwnd, old_parent, parent );

    data->toplevel = (parent == NtUserGetDesktopWindow());
    if (!data->toplevel && data->host_window)
    {
        /* it became a child window, it no longer has a host window; a window
         * that becomes top-level gets one on the next WindowPosChanged */
        data->host_window = FALSE;
        destroy = TRUE;
    }
    release_win_data( data );

    if (destroy) plonk_syscall1( PLONK_SYS_DESTROY, host_hwnd( hwnd ) );
}


/***********************************************************************
 *           PLONK_SetWindowStyle
 */
void PLONK_SetWindowStyle( HWND hwnd, INT offset, STYLESTRUCT *style )
{
    struct plonk_win_data *data;
    RECT window_rect, client_rect;
    UINT new_style, new_ex_style;

    if (offset != GWL_STYLE && offset != GWL_EXSTYLE) return;
    if (style->styleOld == style->styleNew) return;
    if (!(data = get_win_data( hwnd ))) return;
    if (!data->host_window)
    {
        release_win_data( data );
        return;
    }
    window_rect = data->window_rect;
    client_rect = data->client_rect;
    release_win_data( data );

    if (offset == GWL_STYLE)
    {
        new_style = style->styleNew;
        new_ex_style = NtUserGetWindowLongW( hwnd, GWL_EXSTYLE );
    }
    else
    {
        new_style = NtUserGetWindowLongW( hwnd, GWL_STYLE );
        new_ex_style = style->styleNew;
    }
    send_host_window( hwnd, &window_rect, &client_rect, new_style, new_ex_style, FALSE );
}


/***********************************************************************
 *           PLONK_SetWindowText
 */
void PLONK_SetWindowText( HWND hwnd, LPCWSTR text )
{
    struct plonk_win_data *data;
    BOOL host_window;

    if (!(data = get_win_data( hwnd ))) return;
    host_window = data->host_window;
    release_win_data( data );

    /* if the host doesn't know the window yet, the title is sent when it is created */
    if (host_window) send_host_title( hwnd, text );
}


/***********************************************************************
 *           Cursors
 */
static const struct
{
    WORD id;
    int  kind;
} system_cursors[] =
{
    { OCR_NORMAL,      PLONK_CURSOR_ARROW },
    { OCR_APPSTARTING, PLONK_CURSOR_ARROW },
    { OCR_HELP,        PLONK_CURSOR_ARROW },
    { OCR_UP,          PLONK_CURSOR_ARROW },
    { OCR_IBEAM,       PLONK_CURSOR_IBEAM },
    { OCR_WAIT,        PLONK_CURSOR_WAIT },
    { OCR_CROSS,       PLONK_CURSOR_CROSS },
    { OCR_SIZENWSE,    PLONK_CURSOR_SIZENWSE },
    { OCR_SIZENESW,    PLONK_CURSOR_SIZENESW },
    { OCR_SIZEWE,      PLONK_CURSOR_SIZEWE },
    { OCR_SIZENS,      PLONK_CURSOR_SIZENS },
    { OCR_SIZE,        PLONK_CURSOR_SIZEALL },
    { OCR_SIZEALL,     PLONK_CURSOR_SIZEALL },
    { OCR_NO,          PLONK_CURSOR_NO },
    { OCR_HAND,        PLONK_CURSOR_HAND },
};

static int get_cursor_kind( HCURSOR handle )
{
    static const WCHAR user32W[] = {'u','s','e','r','3','2','.','d','l','l',0};
    WCHAR module_name[MAX_PATH], res_name[MAX_PATH];
    UNICODE_STRING module, res;
    const WCHAR *name, *p;
    ICONINFO info;
    unsigned int i;
    WORD id;

    module.Buffer = module_name;
    module.MaximumLength = sizeof(module_name) - sizeof(WCHAR);
    res.Buffer = res_name;
    res.MaximumLength = sizeof(res_name) - sizeof(WCHAR);
    if (!NtUserGetIconInfo( handle, &info, &module, &res, NULL, 0 )) return PLONK_CURSOR_ARROW;
    if (info.hbmColor) NtGdiDeleteObjectApp( info.hbmColor );
    if (info.hbmMask) NtGdiDeleteObjectApp( info.hbmMask );

    if (res.Length) return PLONK_CURSOR_ARROW;  /* only integer resources are supported here */
    id = LOWORD( res.Buffer );
    module_name[module.Length / sizeof(WCHAR)] = 0;

    for (name = p = module_name; *p; p++) if (*p == '\\' || *p == '/') name = p + 1;
    for (i = 0; user32W[i]; i++)
    {
        WCHAR ch = name[i];
        if (ch >= 'A' && ch <= 'Z') ch += 'a' - 'A';
        if (ch != user32W[i]) return PLONK_CURSOR_ARROW;
    }
    if (name[i]) return PLONK_CURSOR_ARROW;

    for (i = 0; i < ARRAY_SIZE( system_cursors ); i++)
        if (system_cursors[i].id == id) return system_cursors[i].kind;

    /* TODO: custom cursor images are not supported by the host ABI */
    return PLONK_CURSOR_ARROW;
}

/***********************************************************************
 *           PLONK_SetCursor
 */
void PLONK_SetCursor( HWND hwnd, HCURSOR handle )
{
    int kind = handle ? get_cursor_kind( handle ) : PLONK_CURSOR_HIDDEN;

    TRACE( "hwnd %p cursor %p kind %d\n", hwnd, handle, kind );
    plonk_syscall2( PLONK_SYS_CURSOR, host_hwnd( hwnd ), kind );
}
