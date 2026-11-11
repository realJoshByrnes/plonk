#!/bin/bash
# usage (from WSL): build.sh OUTDIR   (OUTDIR must hold templates.s from gen.py)
set -e
cd "$(dirname "$0")"
OUT="$1"
i686-w64-mingw32-gcc -O1 -o "$OUT/harness.exe" harness.c "$OUT/templates.s" -static
echo built
