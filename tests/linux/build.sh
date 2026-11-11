#!/usr/bin/env bash
# Build the Linux i386 test programs (run inside WSL Debian; needs gcc-multilib). Copies the i386 loader + libc into root/ (git-ignored).
set -e
cd "$(dirname "$0")"
gcc -m32 -static -O1 -o hello hello.c
gcc -m32 -O1 -o hello_dyn hello.c
gcc -m32 -O1 -pthread -o multi multi.c
mkdir -p root
cp -L /lib32/ld-linux.so.2 /lib32/libc.so.6 /lib32/libm.so.6 /lib32/libdl.so.2 /lib32/libpthread.so.0 /lib32/libgcc_s.so.1 /lib32/librt.so.1 /lib32/libresolv.so.2 root/
