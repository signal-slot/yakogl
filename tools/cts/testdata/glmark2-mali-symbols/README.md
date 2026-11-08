<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Fixtures of glmark2-mali-symbols-selftest.sh

- `good/`: the text `tools/cts/glmark2-mali-symbols.sh --from-dumps` reads
  (`readelf -d` and `nm -D` of a dlopen-flavour glmark2-es2 with RUNPATH
  `$ORIGIN/../lib`, libGLESv2.so.2 exporting every name of `glad-names.txt`
  and `eglGetProcAddress`, and the libEGL.so.1 forwarder). The self-test
  mutates copies of it.
- `glad-names.txt`: the names glmark2 22c527cb's glad loads for EGL 1.0 to 1.4
  and GL_ES_VERSION_2_0 (`src/glad/src/egl.c`, `gles2.c`); the self-test writes
  a glad tree from it.
