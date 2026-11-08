<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mali-glmark2-build: fixtures of tools/cts/mali-glmark2-build-selftest.sh

`tools/cts/mali-glmark2-build.sh --check` reads a built tree with `readelf`
only. The self-test points `READELF` at `bin/readelf`, a fake that prints
`$FAKE_READELF_DIR/<basename of the file>.txt`, so no ELF and no cross
toolchain is needed.

| path | what |
| --- | --- |
| `bin/readelf` | the fake readelf |
| `tree/` | a built tree's layout: `bin/glmark2-es2-mali`, `lib/libGLESv2.so.2` (with the pair's contract-line text), `lib/libEGL.so.1`, `share/glmark2/{shaders,models,textures}`, `BUILD-INFO`. The files are text stand-ins; only their names, the contract text and `BUILD-INFO` are read |
| `readelf/good/*.txt` | `readelf -hldVW` of a real build (`--dev`, commit 78c917aa, Debian trixie g++ 14.2.0, the panthor-gles pair): the program, libGLESv2.so.2 and libEGL.so.1. The symbol-version table (one line per dynamic symbol) is left out; the version needs are kept. Maxima: GLIBC_2.38, GLIBCXX_3.4.32, CXXABI_1.3.9 |
| `readelf/glibc-too-new/glmark2-es2-mali.txt` | the good program's, with its GLIBC_2.38 need made GLIBC_2.42 (one above the board's 2.41) |
| `readelf/system-libegl/glmark2-es2-mali.txt` | the good program's, with a `NEEDED libEGL.so.1`: a program linked against the board's glvnd/Mesa EGL instead of dlopening the pair's |

The other cases (GLIBCXX_3.4.34, a NEEDED libpng16.so.16, another RUNPATH,
an x86-64 program, a libGLESv2.so.2 without the contract text, a stray
libEGL.so.1, a libEGL.so.1 without RUNPATH `$ORIGIN`, a BUILD-INFO that does
not match, no textures) are made by the self-test from these with `sed`.

Regenerate `readelf/good/` from a real build `<out>` with
`<toolchain>/bin/aarch64-linux-gnu-readelf -hldVW <out>/<file>`, dropping the
`Version symbols section` block, and update the maxima in `tree/BUILD-INFO`
and in the self-test's expected `--check` line.
