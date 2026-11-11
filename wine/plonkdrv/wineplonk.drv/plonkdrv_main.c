/*
 * wineplonk.drv unix-side initialisation
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

#include "ntstatus.h"
#define WIN32_NO_STATUS

#include "plonkdrv.h"
#include "wine/debug.h"

WINE_DEFAULT_DEBUG_CHANNEL(plonk);

static const struct user_driver_funcs plonkdrv_funcs =
{
    .pCreateDesktop = PLONK_CreateDesktop,
    .pCreateWindow = PLONK_CreateWindow,
    .pDestroyWindow = PLONK_DestroyWindow,
    .pProcessEvents = PLONK_ProcessEvents,
    .pSetCursor = PLONK_SetCursor,
    .pSetParent = PLONK_SetParent,
    .pSetWindowStyle = PLONK_SetWindowStyle,
    .pSetWindowText = PLONK_SetWindowText,
    .pThreadDetach = PLONK_ThreadDetach,
    .pUpdateDisplayDevices = PLONK_UpdateDisplayDevices,
    .pWindowMessage = PLONK_WindowMessage,
    .pWindowPosChanged = PLONK_WindowPosChanged,
    .pWindowPosChanging = PLONK_WindowPosChanging,
};

static NTSTATUS plonkdrv_unix_init( void *arg )
{
    int width, height;

    plonk_get_screen_size( &width, &height );
    TRACE( "screen %dx%d\n", width, height );

    __wine_set_user_driver( &plonkdrv_funcs, WINE_GDI_DRIVER_VERSION );
    return STATUS_SUCCESS;
}

const unixlib_entry_t __wine_unix_call_funcs[] =
{
    plonkdrv_unix_init,
};

C_ASSERT( ARRAYSIZE(__wine_unix_call_funcs) == plonkdrv_unix_func_count );

#ifdef _WIN64

const unixlib_entry_t __wine_unix_call_wow64_funcs[] =
{
    plonkdrv_unix_init,
};

C_ASSERT( ARRAYSIZE(__wine_unix_call_wow64_funcs) == plonkdrv_unix_func_count );

#endif /* _WIN64 */
