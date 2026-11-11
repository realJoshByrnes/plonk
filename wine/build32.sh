#!/usr/bin/env bash
# Build Wine's Unix side as i386 Linux binaries (run inside WSL Debian, after wine/build.sh; needs gcc-multilib).
# Output: wine/out32/{wineserver,wine,ntdll.so,win32u.so,nls/} (git-ignored), run by crates/host/src/linux.rs.
set -e
OUT="$(cd "$(dirname "$0")" && pwd)/out32"
mkdir -p ~/wine-build32 && cd ~/wine-build32
../wine-src/configure --host=i686-linux-gnu CC="gcc -m32" --with-wine-tools=../wine-build --without-x --without-freetype --without-alsa \
  --without-pulse --without-cups --without-sane --without-v4l2 --without-gstreamer --without-opencl --without-pcap --without-usb \
  --without-wayland --without-gnutls --without-unwind --without-dbus --without-udev --without-fontconfig --without-krb5 --without-inotify \
  --disable-tests >cfg.log 2>&1
make -j"$(nproc)" server/wineserver loader/wine dlls/ntdll/ntdll.so dlls/win32u/win32u.so >unix.log 2>&1
mkdir -p "$OUT/nls"
cp server/wineserver loader/wine dlls/ntdll/ntdll.so dlls/win32u/win32u.so "$OUT/"
cp ../wine-src/nls/*.nls "$OUT/nls/"
