#!/usr/bin/env bash
# Re-pack wine/root.tar from ~/wine-root without rebuilding Wine (run inside WSL Debian).
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
cd ~/wine-root
for l in ld-linux.so.2 libc.so.6 libm.so.6 libdl.so.2 libpthread.so.0 libgcc_s.so.1 librt.so.1 libresolv.so.2; do
  cp -L "/lib32/$l" lib/
done
tar cf "$HERE/root.tar" usr lib
ls -la "$HERE/root.tar"
