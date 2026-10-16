<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# imx8mp-glmark2-gl-errors, 2026-10-16, branch ccfarm/01a1446c (diagnostic images `75a09580`, `9e20355b`, `b421b447` of branch ccfarm/01a14127)

Issue #814: the i.MX8MP's glmark2 raised GL errors ("15 or more" in every
common set: `cd738b1a`, `a63fcc5b`, `e3749a25`, `e9901869a`). Cause: the
GC7000's GPU memory. Fixed in the glmark2 image; measured on the board from
the diagnostic branch, not yet from main.

## State of main

- In the per-board table, the i.MX8MP glmark2 cell gains, first: **0 GL
  errors with the glmark2 image's 240 MiB carve-out, on branch `b421b447`
  (2026-10-16, two common sets, `records/imx8mp/common-b421b447{-r1,}`):
  score 42, 33 scenes ran, 0 failed, 0 incomplete-texture draws; SO, SV, CL
  probes pass; gl_test 48 / 49 (case 44) then 49 / 49. Rerun from main.**
  (`tools/cts/RESULTS.md`, "Common set", the bullet "i.MX8MP glmark2 GL
  errors (#814)".)
- The GL core keeps the first eight GL errors by entry point and client tag
  (`yakogl_gl_error_first`); glmark2 prints them as `glmark2: GL error K of
  N:` lines and, with the SNVS selector `0x800 | k`, stores error k as record
  kind 23 GL_ERROR (`driver/docs/glmark2.md`, "The first GL errors by name").
- The host harness models the GC7000's carve-out (`YAKOGL_GC_CARVEOUT_BYTES`
  in `src/osal/linux.c`, set by `tools/hostharness/glmark2/run.sh` from the
  i.MX8MP glmark2 image's overlay). The gate's glmark2 row fails on any GL
  error line and on an entry whose GC peak is above 3/4 of that carve-out.

## Open work

- Stage 2's "no GL error" on the i.MX8MP: met by `b421b447` on the board;
  closes when a common set built from main reads `GL errors raised 0` in the
  GM row (`tools/cts/board-stage.sh imx8mp common`).
- The i.MX8MP glmark2 item "Next: an i.MX8MP glmark2 run, whose record shows
  them" under #13: done (15 or more at 64 MiB; 0 at 240 MiB).

## Known defects

- #814, cause found, fixed in the glmark2 image: every GC allocation, the
  board layer's 800x600 window and 1280x800 screen copy included, comes from
  the one `gpu-carveout` pool, 64 MiB on the Verdin's device tree. On the
  host model terrain peaks at 51 MB and refract at 48 MB of the 58 MB left
  (one entry per process, frees returned at once); the board also holds
  frees until their kick completes and runs every entry in one fragmenting
  pool, and a refused allocation is a GL error. The glmark2 image now takes
  240 MiB at 0x70000000 (60 of the GC MMU's 64 slave tables;
  `KERNEL_VM_SIZE` 512 MiB). The same cause and step as the AM62P's 2573
  errors (#831). Not measured: which entry points raised the errors at
  64 MiB (the run with the new GL_ERROR record needs `SEL`, which the
  diagnostic worker could not pass); the host model at 64 MiB raises none,
  so it pins the fix by headroom, not by an error.
- Other GC images keep the board's 64 MiB (gl_test, Slint, cluster, dEQP:
  dEQP's `2d.2048x2048_l8_mipmap` ended on GL_OUT_OF_MEMORY there at
  `4f40f622`, its board conf says). A client whose working set nears 48 MiB
  needs the same overlay step.
- gl_test case 44 (#6) failed once in `b421b447`'s first common set and
  passed in the second; this change reaches gl_test's image only through
  the GL core's keeping of the first errors, which runs when an error is
  raised. Count it with the flaky cases 2 and 10.

## Next board session

- After this lands: `tools/cts/board-stage.sh imx8mp common` from main; GM
  must read `GL errors raised 0`. If it does not, name the errors:
  `SEL=$((0x800)) tools/board.sh imx8mp run <GM image> <logdir> gle0` (then
  `0x801`, ...), one boot of about 7 minutes each; `yakogl-tools record`
  names the entry point.
