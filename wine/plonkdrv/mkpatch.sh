#!/bin/bash
# Regenerate explorer.patch from a pristine wine-src checkout of desktop.c.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="${WINE_SRC:-$HOME/wine-src}"
cd "$SRC"
git checkout programs/explorer/desktop.c
sed -i 's|^static const WCHAR default_driver.*$|static const WCHAR default_driver[] = L"plonk,mac,x11";|' programs/explorer/desktop.c
git diff programs/explorer/desktop.c > "$HERE/explorer.patch"
git checkout programs/explorer/desktop.c
cat "$HERE/explorer.patch"
