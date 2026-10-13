<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-es2-r1b2: why the gate is red at the tip, 2026-10-14, main cd738b1a

Details: `docs/investigations/gate-red-tip.md`. Diagnosis only; no file under
`src/`, `tests/`, `tools/` or `driver/` changed.

## State of main

Full `tools/check.sh --jobs 3` at `cd738b1a`: 148 rows, 145 ok, 3 failed, 0
gaps. All clippy rows (15 feature combinations), all `--lib` and integration
test rows (including `report-es2` and `gpu-v3d`) pass. The red rows are the
`check.sh --changed` self-test (row count pinned at 149, real 148) and the
two AM62P Zephyr image rows (no git-ignored `pvr_fw_blob.h` on this worker).

## Open work

- The clippy lints at `src/gl/draw.rs:3700/3871`, the `tf.rs`/`tf_tests.rs`
  lints and the `report-es2` `--lib` failure recorded in earlier reviews do
  not exist at the tip: `clippy-gate-fix` has nothing to fix on main itself.
- New item needed (tools only): fix the two row-count pins in
  `tools/check_scope_selftest.sh`; make the AM62P Zephyr rows a `gap` when
  `driver/src/fw/pvr_fw_blob.h` is absent; stop nested runs clobbering
  `build/check-fail/<n>.log`; correct the `--quick` comment.

## Known defects

- `tools/check_scope_selftest.sh` fails at the tip (lines 88 and 145).
