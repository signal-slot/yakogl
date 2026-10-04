<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gl_test: the on-silicon GL suite

`gl_test.c` is the GL suite every board runs: each case draws through the public
GL ES API and checks pixels read back with `glReadPixels`. The Zephyr samples
`gl_test` (PowerVR), `gc_gl_test` (GC7000) and `v3d_gl_test` (V3D) call
`gl_test_run()`. The header comment lists every case.

- `gl_test_shaders.h`, `gl_test_shaders_es100.h`: the GLSL sources the cases
  use, generated from `tools/shaders/` by `gen_gl_test_shaders.py`. The offline
  shader tables match sources by SHA-256, so never edit them by hand.

`tools/check.sh` compiles `gl_test.c` on the host for the PowerVR and GC7000
arms. The directory name is historical: it once held a Linux kernel-module
harness for the PowerVR, now removed.
