#!/usr/bin/env bash
# Build Plonk's own small stub DLLs (run inside WSL Debian): wine/stubs/build.sh  -> copies into wine/out
set -e
cd "$(dirname "$0")"
OUT=../out
i686-w64-mingw32-gcc -shared -O1 -nostdlib -o dsound.dll dsound.c dsound.def -lkernel32 -Wl,--kill-at -Wl,-e,_DllMain@12
i686-w64-mingw32-strip -o $OUT/dsound.dll dsound.dll
rm -f dsound.dll
ls -l $OUT/dsound.dll | awk '{print "dsound.dll", $5, "bytes"}'
