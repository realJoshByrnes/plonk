#!/bin/bash
# Build wineoss.drv (Wine's OSS audio driver) for i386 and install it into ~/wine-root. Plonk's kernel emulates /dev/dsp and
# /dev/mixer (crates/host/src/linux.rs) and the page plays the PCM. glibc only ships the OSSv3 header, so ./sys/soundcard.h
# adds the few OSSv4 types configure and the driver need. Run inside WSL Debian after wine/build32.sh; then run wine/repack.sh.
# The prefix needs a system32\wineoss.drv link (see the hard-link entry in wine/prefix.tar) for the driver to load.
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$HOME/oss4/include" && cp -r "$HERE/sys" "$HOME/oss4/include/"
cd "$HOME/wine-build32"
OSSLIBDIR="$HOME/oss4" ../wine-src/configure --host=i686-linux-gnu CC="gcc -m32" --with-wine-tools="$HOME/wine-build" \
    --without-x --without-freetype --without-alsa --without-pulse --without-cups --without-sane --without-v4l2 --without-gstreamer \
    --without-opencl --without-pcap --without-usb --without-wayland --without-gnutls --without-unwind --without-dbus --without-udev \
    --without-fontconfig --without-krb5 --without-inotify --disable-tests >/dev/null
make -j"$(nproc)" dlls/wineoss.drv/all
cp dlls/wineoss.drv/i386-windows/wineoss.drv "$HOME/wine-root/usr/local/lib/wine/i386-windows/"
cp dlls/wineoss.drv/wineoss.so "$HOME/wine-root/usr/local/lib/wine/i386-unix/"
