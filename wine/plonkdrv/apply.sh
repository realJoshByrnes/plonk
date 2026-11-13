#!/bin/bash
# Install wineplonk.drv into the Wine 9.0 tree, patch explorer, build the
# driver (PE + unix) and explorer for i386, and copy the results to out32/drv.
#
# Run inside WSL Debian:  bash /mnt/c/Users/jd/Projects/plonk/wine/plonkdrv/apply.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="${WINE_SRC:-$HOME/wine-src}"
TOOLS="${WINE_TOOLS:-$HOME/wine-build}"
BUILD="${WINE_BUILD32:-$HOME/wine-build32}"
OUT="${PLONK_DRV_OUT:-$HERE/../out32/drv}"
JOBS="${JOBS:-$(nproc)}"

if pgrep -x make >/dev/null; then
    echo "apply.sh: a make is already running, refusing to start another one" >&2
    exit 1
fi
for tool in autoconf autoheader; do
    if ! command -v "$tool" >/dev/null; then
        echo "apply.sh: $tool is missing (as root: apt-get install -y autoconf)" >&2
        exit 1
    fi
done

echo "== copying driver sources"
mkdir -p "$SRC/dlls/wineplonk.drv"
for f in "$HERE"/wineplonk.drv/*; do
    dst="$SRC/dlls/wineplonk.drv/$(basename "$f")"
    # only touch files that changed, to keep rebuilds minimal
    if ! cmp -s "$f" "$dst"; then
        tr -d '\r' < "$f" > "$dst"
    fi
done
# Wine drivers have an (empty) spec file
[ -e "$SRC/dlls/wineplonk.drv/wineplonk.drv.spec" ] || : > "$SRC/dlls/wineplonk.drv/wineplonk.drv.spec"

echo "== patching explorer"
if grep -q 'plonk' "$SRC/programs/explorer/desktop.c"; then
    echo "   already patched"
else
    (cd "$SRC" && tr -d '\r' < "$HERE/explorer.patch" | git apply -)
fi

echo "== patching wined3d (copy only changed front-buffer pixels)"
if ! grep -q plonk_shrink_to_changes "$SRC/dlls/wined3d/swapchain.c"; then (cd "$SRC" && tr -d '' < "$HERE/wined3d.patch" | git apply -); fi

echo "== regenerating makefiles and configure"
regen=0
grep -q 'dlls/wineplonk.drv' "$SRC/configure" || regen=1
[ "$SRC/dlls/wineplonk.drv/Makefile.in" -nt "$SRC/configure" ] && regen=1
if [ "$regen" = 1 ]; then
    # make_makefiles only sees files known to git (index only, nothing is committed)
    (cd "$SRC" && git add -N dlls/wineplonk.drv && tools/make_makefiles && autoreconf -f)
else
    echo "   configure is up to date"
fi

echo "== configuring $BUILD"
if [ "$regen" = 1 ] || ! grep -q 'dlls/wineplonk.drv' "$BUILD/Makefile"; then
    (cd "$BUILD" && "$SRC/configure" --host=i686-linux-gnu CC="gcc -m32" --with-wine-tools="$TOOLS" \
        --without-x --without-freetype --without-alsa --without-pulse --without-cups --without-sane \
        --without-v4l2 --without-gstreamer --without-opencl --without-pcap --without-usb \
        --without-wayland --without-gnutls --without-unwind --without-dbus --without-udev \
        --without-fontconfig --without-krb5 --without-inotify --disable-tests)
else
    echo "   already configured"
fi

echo "== building"
make -C "$BUILD" -j"$JOBS" dlls/wineplonk.drv/all programs/explorer/all

echo "== copying results to $OUT"
mkdir -p "$OUT"
cp -f "$BUILD/dlls/wineplonk.drv/i386-windows/wineplonk.drv" "$OUT/"
cp -f "$BUILD/dlls/wineplonk.drv/wineplonk.so" "$OUT/"
cp -f "$BUILD/programs/explorer/i386-windows/explorer.exe" "$OUT/"
ls -l "$OUT"
