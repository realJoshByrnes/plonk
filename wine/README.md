# wine/ (separate, LGPL-2.1+ component)
Plonk does not contain Wine source. `build.sh` fetches Wine 9.0 (commit fd1153552d779e1ac14b0b1c48fbd1cde894eb0a) from the upstream mirror
and builds its i386 PE DLLs unmodified. Wine is licensed under the GNU LGPL 2.1 or later (see the Wine source tree's COPYING.LIB).
Built DLLs go in `wine/out/` (git-ignored). Plonk code talks to them only through the syscall/unix-call boundary.
