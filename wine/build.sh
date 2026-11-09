#!/usr/bin/env bash
# Build Wine's i386 PE DLLs (run inside WSL Debian). Needs: git flex bison clang lld gcc-mingw-w64-i686 mingw-w64-tools.
# Usage: wine/build.sh   (clones wine-9.0 to ~/wine-src, builds in ~/wine-build, strips into /tmp/wine-out)
set -e
[ -d ~/wine-src ] || git clone -q --depth 1 --branch wine-9.0 https://github.com/wine-mirror/wine.git ~/wine-src
rm -rf ~/wine-build && mkdir ~/wine-build && cd ~/wine-build
../wine-src/configure --enable-archs=i386 --without-x --without-freetype --without-alsa --without-pulse --without-cups \
  --without-sane --without-v4l2 --without-gstreamer --without-opencl --without-pcap --without-usb --without-wayland --disable-tests >cfg.log 2>&1
make -j"$(nproc)" __tooldeps__ >tools.log 2>&1
make -j"$(nproc)" include/all >inc.log 2>&1
D="ntdll kernel32 kernelbase win32u user32 gdi32 advapi32 sechost ucrtbase msvcrt imm32 rpcrt4"
make -j"$(nproc)" -k $(for d in $D; do echo dlls/$d/i386-windows/$d.dll; done) >dll.log 2>&1
mkdir -p /tmp/wine-out
for d in $D; do i686-w64-mingw32-strip -o /tmp/wine-out/$d.dll dlls/$d/i386-windows/$d.dll; done
