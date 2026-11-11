#!/usr/bin/env bash
# Build all of Wine 9.0 for i386 Linux (Unix side + PE side + the Plonk display driver) and pack it as wine/root.tar, the
# guest file tree used by crates/host/src/linux.rs. Run inside WSL Debian after wine/build.sh (which provides the build tools).
# Needs (as root): dpkg --add-architecture i386; apt install gcc-multilib libfreetype-dev:i386 fontforge-nox autoconf
# The driver must already be in the source tree: run wine/plonkdrv/apply.sh once first.
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
mkdir -p ~/wine-build32 && cd ~/wine-build32
PKG_CONFIG_LIBDIR=/usr/lib/i386-linux-gnu/pkgconfig:/usr/share/pkgconfig ../wine-src/configure --host=i686-linux-gnu CC="gcc -m32" \
  --with-wine-tools=../wine-build --without-x --with-freetype --without-alsa --without-pulse --without-cups --without-sane \
  --without-v4l2 --without-gstreamer --without-opencl --without-pcap --without-usb --without-wayland --without-gnutls --without-unwind \
  --without-dbus --without-udev --without-fontconfig --without-krb5 --without-inotify --disable-tests >cfg.log 2>&1
# -k: the bitmap .fon fonts need a FreeType-enabled sfnt2fon in the tools build, which we do not have; everything else builds
make -k -j"$(nproc)" >full.log 2>&1 || echo "make reported errors (expected: fonts/*.fon)"
rm -rf ~/wine-inst
make -k install-lib DESTDIR="$HOME/wine-inst" >inst.log 2>&1 || echo "install reported errors (expected: fonts/*.fon)"
rm -rf ~/wine-root && mkdir -p ~/wine-root/usr ~/wine-root/lib
cp -a ~/wine-inst/usr/local ~/wine-root/usr/
cd ~/wine-root/usr/local
find lib/wine/i386-windows -type f -print0 | xargs -0 -P8 -n50 i686-w64-mingw32-strip 2>/dev/null || true
find lib/wine/i386-unix bin -type f -print0 | xargs -0 -n50 strip 2>/dev/null || true
mkdir -p ~/wine-root/usr/local/share/wine/fonts && cp ~/wine-build32/fonts/*.ttf ~/wine-root/usr/local/share/wine/fonts/ # make install skips them when the .fon step fails
cd ~/wine-root
for l in libfreetype.so.6 libz.so.1 libpng16.so.16 libbrotlidec.so.1 libbrotlicommon.so.1 libbz2.so.1.0; do
  cp -L "/usr/lib/i386-linux-gnu/$l" lib/
done
tar cf "$HERE/root.tar" usr lib
ls -la "$HERE/root.tar"
ls usr/local/share/wine/fonts | tr '\n' ' '
ls usr/local/bin lib | tr '\n' ' '
