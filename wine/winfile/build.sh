#!/usr/bin/env bash
# Build Microsoft winfile (MIT) as a static i386 PE with mingw-w64. Run in WSL Debian. Output: ./winfile.exe
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
SRC=${SRC:-$HOME/winfile-src}
[ -d "$SRC" ] || git clone -q --depth 1 https://github.com/Microsoft/winfile "$SRC"
cd "$SRC"
git checkout -q .
# mingw: backslash and wrong-case includes
sed -i -f "$HERE/fixinc.sed" src/*.c src/*.cpp src/*.h src/*.rc src/lang/*
# case-insensitive include names used by the sources
mkdir -p shim; M=/usr/i686-w64-mingw32/include
ln -sf $M/windowsx.h shim/windowsX.h
make -j"$(nproc)" CC=i686-w64-mingw32-gcc CXX=i686-w64-mingw32-g++ WINDRES=i686-w64-mingw32-windres OS=Windows_NT \
  CFLAGS="-DUNICODE -D_UNICODE -DFASTMOVE -DSTRSAFE_NO_DEPRECATE -DWINVER=0x0600 -D_WIN32_WINNT=0x0600 -Ishim -w -fpermissive -O2" \
  LDFLAGS="-static -static-libgcc -static-libstdc++" >make.log 2>&1 || { grep -E "error|Error" make.log | head -20; exit 1; }
i686-w64-mingw32-strip -o "$HERE/winfile.exe" winfile.exe
