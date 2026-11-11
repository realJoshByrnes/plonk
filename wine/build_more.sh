#!/usr/bin/env bash
# Incrementally build extra Wine DLLs in the existing ~/wine-build tree (run inside WSL Debian): wine/build_more.sh dll1 dll2 ...
# Output: stripped DLLs in /tmp/wine-out and a per-DLL import list on stdout (to find the next dependencies).
set -e
cd ~/wine-build
make -j"$(nproc)" -k $(for d in "$@"; do echo dlls/$d/i386-windows/$(case $d in *.*) echo $d;; *) echo $d.dll;; esac); done) >more.log 2>&1 || true
mkdir -p /tmp/wine-out
for d in "$@"; do
  n=$(case $d in *.*) echo $d;; *) echo $d.dll;; esac)
  f=dlls/$d/i386-windows/$n
  if [ -f "$f" ]; then
    i686-w64-mingw32-strip -o /tmp/wine-out/$n "$f"
    echo "$d: $(i686-w64-mingw32-objdump -p "$f" | grep 'DLL Name' | awk '{print $3}' | tr '\n' ' ')"
  else
    echo "$d: BUILD FAILED (see ~/wine-build/more.log)"
  fi
done
