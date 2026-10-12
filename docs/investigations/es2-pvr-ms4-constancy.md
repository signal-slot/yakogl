<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P ms4 constancy: pointer

The host diagnosis of `multisample.constancy_alpha_to_coverage` and
`constancy_both_inverted` is already on main, and so is its root cause:

* Record (what the cases check, ranked suspects with evidence, the cause):
  `tools/cts/log/investigations/es2-pvr-ms4-constancy.md`.
* Cause: the 1,015th of the case's 1,024 draws overflowed the 768 KiB PDS
  arena and opened a second kick whose load quad wrote one resolved colour
  to all four samples. Fixed by `a4f64950` (a frame uploads a PDS program it
  already holds once; gl_test 198 `a2cconst`).
* Host tests: `tests/es2_pvr_ms4_constancy.rs`, `tests/usc_limits.rs`.
* Board check: the ms4 image of `tools/cts/am62p-deqp-images.sh`; all 16
  cases should pass.

This file only points there, so the two records do not drift apart.
