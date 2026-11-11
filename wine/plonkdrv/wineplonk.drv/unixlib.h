/*
 * wineplonk.drv unix call interface
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */

#ifndef __WINE_PLONKDRV_UNIXLIB_H
#define __WINE_PLONKDRV_UNIXLIB_H

#include <stdarg.h>
#include "windef.h"
#include "winbase.h"
#include "winternl.h"
#include "wine/unixlib.h"

enum plonkdrv_unix_func
{
    plonkdrv_unix_func_init,
    plonkdrv_unix_func_count,
};

#define PLONKDRV_UNIX_CALL(func, params) WINE_UNIX_CALL(plonkdrv_unix_func_ ## func, params)

#endif /* __WINE_PLONKDRV_UNIXLIB_H */
