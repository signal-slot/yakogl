<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gl_test: the on-silicon GL suite

`gl_test.c` is the GL suite every board runs: each case draws through the public
GL ES API and checks pixels read back with `glReadPixels`. The Zephyr samples
`gl_test` (PowerVR), `gc_gl_test` (GC7000) and `v3d_gl_test` (V3D) call
`gl_test_run()`. The header comment lists every case.

- `gl_test_shaders.h`, `gl_test_shaders_es100.h`: the GLSL sources the cases
  use, generated from `tools/shaders/` by `gen_gl_test_shaders.py`. The offline
  shader tables match sources by SHA-256, so never edit them by hand: change the
  file under `tools/shaders/`, then regenerate.
- `runner/`: the gl_test runner (Rust, the crate `yakogl-gltest-runner`), the
  walk `gl_test_run()` calls after its bring-up: which cases run (the
  regression set `SET_ORDER`, the list `gl_test_set_list()` stored, one case),
  the skips by name, each case's `skipped` / `SKIP` / `PASS` / `FAIL` /
  `TIMEOUT` line, the tally a board's record reads and the `gl_test: all:`
  line. The case bodies stay C and are called through FFI; a case moves to
  Rust later by replacing its function and its arm of the C switch. Every
  build that links `gl_test.c` links the runner: the Zephyr images and the
  host harness through `libyakogl.a`'s `gl-test-runner` feature, panthor-gltest
  as a crate dependency. `cargo test -p yakogl-gltest-runner` pins the lines
  and the count on the host.

`tools/check.sh` compiles `gl_test.c` on the host for the PowerVR and GC7000
arms, and runs `gen_gl_test_shaders.py --check`, which regenerates both headers
and refuses a drift against the committed ones. The directory name is historical:
it once held a Linux kernel-module harness for the PowerVR, now removed.
