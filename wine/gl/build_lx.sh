#!/usr/bin/env bash
# Build Plonk's opengl32.dll for the real-Wine path and put it into the packed root (run inside WSL Debian, then wine/repack.sh).
set -e
cd "$(dirname "$0")"
i686-w64-mingw32-gcc -shared -O1 -nostdlib -o opengl32_lx.dll opengl32_lx.c opengl32_lx.def -luser32 -lkernel32 -Wl,--kill-at -Wl,-e,_DllMain@12
i686-w64-mingw32-strip opengl32_lx.dll
~/wine-build/tools/winebuild/winebuild --builtin opengl32_lx.dll
cp opengl32_lx.dll ~/wine-root/usr/local/lib/wine/i386-windows/opengl32.dll
rm -f opengl32_lx.dll
ls -l ~/wine-root/usr/local/lib/wine/i386-windows/opengl32.dll
