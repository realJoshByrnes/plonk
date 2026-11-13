/*
 * wineplonk.drv unix-side private definitions
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */

#ifndef __WINE_PLONKDRV_H
#define __WINE_PLONKDRV_H

#ifndef __WINE_CONFIG_H
# error You must include config.h to use this header
#endif

#include <stdarg.h>
#include <unistd.h>
#include <sys/syscall.h>

#include "windef.h"
#include "winbase.h"
#include "wingdi.h"
#include "winuser.h"
#include "ntgdi.h"
#include "ntuser.h"
#include "wine/gdi_driver.h"
#include "wine/list.h"

#include "unixlib.h"
#include "plonk_abi.h"

/* driver-private window messages, handled in pWindowMessage */
enum plonk_window_messages
{
    WM_PLONK_SCREEN_CHANGED = WM_WINE_FIRST_DRIVER_MSG,
};

/* host calls */
static inline long plonk_syscall1( long nr, long a )
{
    return syscall( nr, a );
}

static inline long plonk_syscall2( long nr, long a, long b )
{
    return syscall( nr, a, b );
}

static inline long plonk_syscall3( long nr, long a, long b, long c )
{
    return syscall( nr, a, b, c );
}

static inline long plonk_syscall5( long nr, long a, long b, long c, long d, long e )
{
    return syscall( nr, a, b, c, d, e );
}

/* display.c */
extern void plonk_get_screen_size( int *width, int *height );
extern void plonk_init_display_devices( BOOL force );
extern void plonk_resize_desktop(void);

/* user driver entry points */
extern LONG PLONK_ChangeDisplaySettings( LPDEVMODEW displays, LPCWSTR primary_name, HWND hwnd, DWORD flags, LPVOID lparam );
extern BOOL PLONK_UpdateDisplayDevices( const struct gdi_device_manager *device_manager,
                                        BOOL force, void *param );
extern BOOL PLONK_CreateDesktop( const WCHAR *name, UINT width, UINT height );
extern BOOL PLONK_CreateWindow( HWND hwnd );
extern void PLONK_DestroyWindow( HWND hwnd );
extern BOOL PLONK_ProcessEvents( DWORD mask );
extern void PLONK_SetCursor( HWND hwnd, HCURSOR handle );
extern void PLONK_SetParent( HWND hwnd, HWND parent, HWND old_parent );
extern void PLONK_SetWindowStyle( HWND hwnd, INT offset, STYLESTRUCT *style );
extern void PLONK_SetWindowText( HWND hwnd, LPCWSTR text );
extern LRESULT PLONK_WindowMessage( HWND hwnd, UINT msg, WPARAM wp, LPARAM lp );
extern BOOL PLONK_WindowPosChanging( HWND hwnd, HWND insert_after, UINT swp_flags,
                                     const RECT *window_rect, const RECT *client_rect,
                                     RECT *visible_rect, struct window_surface **surface );
extern void PLONK_WindowPosChanged( HWND hwnd, HWND insert_after, UINT swp_flags,
                                    const RECT *window_rect, const RECT *client_rect,
                                    const RECT *visible_rect, const RECT *valid_rects,
                                    struct window_surface *surface );
extern void PLONK_ThreadDetach(void);

#endif /* __WINE_PLONKDRV_H */
