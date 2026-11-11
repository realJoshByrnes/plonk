/*
 * wineplonk.drv display devices
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
#include <string.h>

#include "plonkdrv.h"
#include "wine/debug.h"

WINE_DEFAULT_DEBUG_CHANNEL(plonk);

static BOOL force_display_devices_refresh;

/***********************************************************************
 *           plonk_get_screen_size
 *
 * Ask the host for the current screen size.
 */
void plonk_get_screen_size( int *width, int *height )
{
    int wh[2] = { 0, 0 };

    if (plonk_syscall1( PLONK_SYS_SCREEN, (long)wh ) < 0 || wh[0] <= 0 || wh[1] <= 0)
    {
        ERR( "PLONK_SYS_SCREEN failed (errno %d, got %dx%d), assuming 1024x768\n", errno, wh[0], wh[1] );
        wh[0] = 1024;
        wh[1] = 768;
    }
    *width = wh[0];
    *height = wh[1];
}

/***********************************************************************
 *           plonk_init_display_devices
 */
void plonk_init_display_devices( BOOL force )
{
    UINT32 num_path, num_mode;

    TRACE( "force=%d\n", force );

    if (force) force_display_devices_refresh = TRUE;
    /* trigger refresh in win32u */
    NtUserGetDisplayConfigBufferSizes( QDC_ONLY_ACTIVE_PATHS, &num_path, &num_mode );
}

/***********************************************************************
 *           plonk_resize_desktop
 */
void plonk_resize_desktop(void)
{
    RECT virtual_rect = NtUserGetVirtualScreenRect();

    NtUserSetWindowPos( NtUserGetDesktopWindow(), 0, virtual_rect.left, virtual_rect.top,
                        virtual_rect.right - virtual_rect.left, virtual_rect.bottom - virtual_rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_DEFERERASE );
}

/***********************************************************************
 *           PLONK_UpdateDisplayDevices
 */
BOOL PLONK_UpdateDisplayDevices( const struct gdi_device_manager *device_manager, BOOL force, void *param )
{
    static const char gpu_name[] = "Plonk display";
    struct gdi_gpu gpu = {0};
    struct gdi_adapter adapter = {0};
    struct gdi_monitor monitor = {0};
    DEVMODEW mode;
    int width, height;
    unsigned int i;

    if (!force && !force_display_devices_refresh) return TRUE;

    force_display_devices_refresh = FALSE;

    plonk_get_screen_size( &width, &height );
    TRACE( "force=%d screen %dx%d\n", force, width, height );

    for (i = 0; i < sizeof(gpu_name); i++) gpu.name[i] = gpu_name[i];
    device_manager->add_gpu( &gpu, param );

    adapter.id = 0;
    adapter.state_flags = DISPLAY_DEVICE_ATTACHED_TO_DESKTOP | DISPLAY_DEVICE_PRIMARY_DEVICE |
                          DISPLAY_DEVICE_VGA_COMPATIBLE;
    device_manager->add_adapter( &adapter, param );

    SetRect( &monitor.rc_monitor, 0, 0, width, height );
    /* TODO: the host has no way to report a work area (e.g. a task bar) yet */
    monitor.rc_work = monitor.rc_monitor;
    monitor.state_flags = DISPLAY_DEVICE_ATTACHED | DISPLAY_DEVICE_ACTIVE;
    device_manager->add_monitor( &monitor, param );

    memset( &mode, 0, sizeof(mode) );
    mode.dmSize = sizeof(mode);
    mode.dmFields = DM_DISPLAYORIENTATION | DM_BITSPERPEL | DM_PELSWIDTH | DM_PELSHEIGHT |
                    DM_DISPLAYFLAGS | DM_DISPLAYFREQUENCY | DM_POSITION;
    mode.dmDisplayOrientation = DMDO_DEFAULT;
    mode.dmDisplayFlags = 0;
    mode.dmBitsPerPel = 32;
    mode.dmPelsWidth = width;
    mode.dmPelsHeight = height;
    mode.dmDisplayFrequency = 60;
    mode.dmPosition.x = 0;
    mode.dmPosition.y = 0;
    device_manager->add_mode( &mode, TRUE, param );

    return TRUE;
}

/***********************************************************************
 *           PLONK_CreateDesktop
 *
 * The desktop always covers the whole host screen and needs no host window.
 */
BOOL PLONK_CreateDesktop( const WCHAR *name, UINT width, UINT height )
{
    TRACE( "%s %ux%u\n", debugstr_w(name), width, height );
    return TRUE;
}
