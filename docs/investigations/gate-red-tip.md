<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Why the gate is red at the integration tip: every `tools/check.sh` row

Item `pi4-es2-r1b2` of plan #20. Diagnosis only: no file under `src/`,
`tests/`, `tools/` or `driver/` changed.

- Measured at: `cd738b1ae7cf99166a6bdd86cf24dac04c4c4328` (the integration
  tip, branch `ccfarm/...` clean checkout, 2026-10-14).
- `git fetch origin` worked. `origin/main` is the same commit
  (`cd738b1a`), so every result below holds on `origin/main` by identity.
  Nothing was run on a second tree.
- Host: Linux, 64 cores, rustc/clippy 0.1.98 (2026-07-01), edition 2021,
  `rust-version` 1.88. `ZEPHYR_WS`, `PCO_SHC`, `GLMARK2_SRC` and `HAL_NXP_DIR`
  were set on this worker, so no row was a gap. No board was used.

## Result in one paragraph

At the tip the full `tools/check.sh --jobs 3` runs 148 rows: **145 ok, 3
failed, 0 gaps** (916 s, warm target dir, 659 test binaries). The three red
rows are not clippy or `--lib` failures. They are (1) the self-test of
`check.sh` itself, whose row count is pinned one too high, and (2, 3) the two
AM62P Zephyr image rows, which need the git-ignored PowerVR firmware header
that this workspace does not have. **None of the failures recorded in the
review results reproduces at the tip**: not the clippy `collapsible_if` pair
at `src/gl/draw.rs:3700/3871`, not `FAIL test host-test,gpu-powervr,report-es2
--lib`, not the lints in `tf.rs`/`tf_tests.rs`/`texture.rs`, not the
`check_table_absent` rows. They came from the review branches' own diffs or
from their older bases (see "The recorded failures" below).

## Failing rows

| row | failure | cause | proposed fix | owning item |
|---|---|---|---|---|
| `tools   check.sh --changed: the path-to-scope map and four plans` (`tools/check_scope_selftest.sh`) | 2 of 75 checks fail: `FAIL docs: only the shared rows: no line matches '^check.sh: 2 ok, 0 failed, 0 gap\(s\).*, 146 skipped (--changed HEAD, scopes: docs'` (tools/check_scope_selftest.sh:88) and `FAIL full plan: its row count: 148 rows (ok + gaps), expected 149` (:145) | The self-test pins the row count of `check.sh` (146 skipped + 2 ok, and 149 with the tier-table row). The real count at the tip is 147 without and 148 with the table row, one fewer than pinned, with no gap on this host. `tools/check.sh` and the self-test have not changed since `39d25d2f` (2026-10-12), which wrote the pins (+2 rows: `es3_occlusion_gl` x2). So either that commit's pin was off by one when written, or one row exists only on a host with something this worker lacks; no `if`/`gap` in `check.sh` that I read explains it (all are counted as ok or gap). Needs one run of the self-test on the gate host to tell. | Run `tools/check_scope_selftest.sh` once on the gate host and print the plan's row list (`YAKOGL_CHECK_PLAN=1 tools/check.sh --changed HEAD`). If the gate host also gives 148, change the two pins to 147/148 (lines 88 and 145 and the comment at 140). If it gives 149, find the row that this worker lacks and make that row a `gap` here too. | new item needed (owns `tools/check_scope_selftest.sh`, `tools/check.sh`) |
| `zephyr  am62p gl_test image, -Werror on our C files` (`zephyr_werror_image verdin_am62p/am62p54/a53 gl_test am62p ...`, `tools/check.sh:2552`) | `driver/src/pvr_fw_boot.c:304:10: fatal error: fw/pvr_fw_blob.h: No such file or directory` (log: `build/zephyr-werror/am62p.log`) | `driver/src/fw/pvr_fw_blob.h` is generated from the proprietary PowerVR firmware (`driver/tools/mkfwblob.sh`), is git-ignored and never committed. `.ccfarm/project.toml:155` lists it in `copy_from_origin`, but this workspace did not receive it (`driver/src/fw/` does not exist). The row only checks `ZEPHYR_WS`, not the header, so a missing header is a red row, where a missing `PCO_SHC` or west workspace is a `gap`. Same cause as the note in `docs/board-sessions/am62p-es2-r1c.md:202` ("the worker had it"). Not a defect of the tip's code. | Make the two AM62P rows a `gap` (with the text "no driver/src/fw/pvr_fw_blob.h: our C files were NOT compiled with -Werror") when the header is absent, like the existing `ZEPHYR_WS` and `PCO_SHC` gaps. Separately, the farm must copy the header to workers that run the full gate. | new item needed (owns `tools/check.sh`); the farm copy is a ccfarm setting, owner decision |
| `zephyr  am62p gl_test image without a compiler (precompiled shaders only), -Werror` (`zephyr_precompiled_only_image`, `tools/check.sh:2555`) | same: `pvr_fw_boot.c:304:10: fatal error: fw/pvr_fw_blob.h: No such file or directory` (log: `build/zephyr-werror/am62p-precompiled-only.log`) | same as the row above | same fix, same item | new item needed (as above) |

Two more observations, not rows:

- **The "whole output" pointer of a red row can be empty.** `check.sh` copies a
  failed row to `build/check-fail/<n>.log` (`tools/check.sh:443-447`), and the
  nested runs inside `check_scope_selftest.sh` / `check_pool_selftest.sh`
  (their `demo fails` rows) write the same directory. In this run
  `build/check-fail/1.log` was 0 bytes and `2.log` held `demo output` right
  after the outer self-test row failed, so `(whole output: ...)` pointed at the
  wrong file. Fix: name the file by the run's `$$` (`build/check-fail/$$-<n>.log`)
  or point nested runs at another directory. Owner: new item needed.
- **`--quick` is not "the PowerVR build only".** The header says 5 s; the run
  executed 117 rows (all the V3D, C, tools and artefact rows) in 412 s. The
  comment at the top of `tools/check.sh` is out of date. Owner: new item needed
  (comment only).

## The recorded failures, and why they do not reproduce here

| recorded failure | at the tip | likely origin | owning item |
|---|---|---|---|
| `FAIL clippy yakogl-precompile gpu-vivante: 2 warning(s) (src/gl/draw.rs:3700, 3871: collapsible if)` | `cargo clippy -p yakogl-precompile --features gpu-vivante --all-targets` finishes with 0 warnings. `src/gl/draw.rs:3698-3700` is `if ... { continue; }`, not a nested `if`; there is no `collapsible_if` hit anywhere in `src/gl`. | Those lines are in the review branch's own `draw.rs`, not in the tip. The tip's last `draw.rs` change is `5466c8dc` (#423). | the review item that changed `draw.rs`; `clippy-gate-fix` for lints that are real on the branch it lands on |
| `FAIL test host-test,gpu-powervr,report-es2 (target --lib; src/ not changed by this commit)` | `--lib` passes (1871 passed, 1 ignored), once alone and once in the full gate (the row also passes with `--lib` run again by the full `cargo test`); the same for all seven CONFIGS rows and the V3D rows. | Not reproduced in 3 runs. A red `--lib` with `src/` unchanged points to a flaky or order-dependent test, or to a cached binary from an older base (the row skips binaries that "passed on main with the same key"). `check.sh` keeps no failing test name when the later binaries pass (`tools/check.sh:438-441` says so itself). | new item needed if it recurs: capture `build/check-fail/*.log` of the failing run before anything else runs |
| lints in `src/gl/tf.rs`, `src/gl/tf_tests.rs`, `src/gl/draw.rs`; `|| true` near `src/gl/texture.rs:9200`; duplicated `#[cfg(feature = "gpu-powervr")]` in `draw.rs` | clippy: 0 warnings in all 15 clippy rows. `src/gl/texture.rs` has no `|| true`. `draw.rs` has no two consecutive identical `cfg` lines. | They are in the plan-19 branches (`work/es3-tf-capture`, based on `c17714e9`, 2 commits ahead and 426 files different from the tip). The branch's own history already has a fix commit for `tf.rs` (`17294985`). | `tf.rs`/`tf_tests.rs`: plan 19 (tf-capture/tf-infolog); `draw.rs`/`texture.rs`: `clippy-gate-fix` |
| `check_table_absent` / "no table" checks break under LTO | `no table in the artefact`, `no Mesa reference in the artefact`, `no test knob in the artefact` pass (1 s to 199 s in the full run, 47 s cold in `--quick`). | Not reproduced at the tip; the rows ran with the release archives built from this tree. | none at the tip |

## Every row run

All commands were run in the repository root at `cd738b1a`. "Standalone" means
I ran the command myself; "gate" means a row of the full
`tools/check.sh --jobs 3` (this run, 148 rows). Every row of that run is
listed; the columns give the result. Rows before "Tools" are the CONFIGS/Rust
rows.

### Clippy (all `--all-targets` unless noted): 0 warnings each

Standalone and in the gate, identical result.

| row | result |
|---|---|
| `cargo fmt --check` | ok |
| `cargo clippy --no-default-features --features host-test,gpu-powervr --all-targets` | ok |
| `... host-test,gpu-powervr,report-es2` | ok |
| `... host-test,gpu-vivante` | ok |
| `... host-test,gpu-powervr,shaderc-online` | ok |
| `... host-test,gpu-vivante,shaderc-online` | ok |
| `... host-test,gpu-powervr,shaderc-online,report-es2` | ok |
| `... host-test,gpu-vivante,shaderc-online,report-es2` | ok |
| `... host-test,gpu-powervr,shaderc-online,precompiled-shaders` (gate only) | ok |
| `... host-test,gpu-vivante,shaderc-online,precompiled-shaders` (gate only) | ok |
| `... host-test,gpu-powervr,precompiled-shaders,no-shader-table` `--lib --profile test` (gate only) | ok |
| `... host-test,gpu-vivante,precompiled-shaders,no-shader-table` `--lib --profile test` (gate only) | ok |
| `... host-test,gpu-vivante,shaderc-online,no-shader-table --lib` | ok |
| `... host-test,gpu-powervr,shaderc-online,no-shader-table --lib` | ok |
| `... host-test,gpu-vivante,shaderc-online,test-knobs` | ok |
| `... host-test,gpu-v3d,shaderc-online,no-shader-table` | ok |
| `... host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2` | ok |
| `cargo clippy -p yakogl-tools --all-targets` | ok |
| `cargo clippy -p yakogl-precompile --features gpu-powervr / gpu-vivante / gpu-powervr,report-es2 / gpu-vivante,report-es2 --all-targets` | ok (4 rows) |
| `yakogl-slint-check`, `yakogl-cluster-check`, `yakogl-triangle-check`, `yakogl-drivers`, `yakogl-drivers` each shipped feature arm | ok (gate) |

### Library and integration tests

`cargo test --no-fail-fast --no-default-features --features <F>` standalone
(all binaries) and in the gate; the `--lib` count is from the standalone run.

| features | `--lib` | all binaries |
|---|---|---|
| `host-test,gpu-powervr` | 1989 passed, 1 ignored | ok |
| `host-test,gpu-powervr,report-es2` | 1871 passed, 1 ignored | ok |
| `host-test,gpu-vivante` | 1867 passed | ok |
| `host-test,gpu-vivante,report-es2` | 1753 passed | (not run alone; `--lib` only) |
| `host-test,gpu-powervr,shaderc-online` | not alone | ok |
| `host-test,gpu-vivante,shaderc-online` | not alone | ok |
| `host-test,gpu-powervr,shaderc-online,report-es2` | 2567 passed, 1 ignored | ok |
| `host-test,gpu-vivante,shaderc-online,report-es2` | 2513 passed, 1 ignored | ok |
| `host-test,gpu-v3d,shaderc-online,no-shader-table` | 2784 passed, 4 ignored | ok (gate: lib row 2460 minimum met; 9 integration binaries ok) |
| `host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2` | 2558 passed, 10 ignored | ok (gate: `--lib shaderc::online::v3d` and 7 integration binaries ok) |
| `host-test,gpu-vivante,shaderc-online,test-knobs` | 9 knob tests by name ok | `gc_kick_diff.rs` 7 tests ok |
| `host-test,gpu-powervr,shaderc-online,precompiled-shaders` (gate) | ok | ok |
| `host-test,gpu-vivante,shaderc-online,precompiled-shaders` (gate) | ok | ok |
| no-compiler `shaderc::precompiled`, both GPUs (gate) | ok | ok |
| `-p yakogl-precompile --features gpu-powervr` / `gpu-vivante` | ok | ok |
| `-p yakogl-tools`, `yakogl-slint-check`, `yakogl-cluster-check`, `yakogl-triangle-check`, `yakogl-drivers` | ok | ok |

The 4 ignored V3D tests are listed by the row in `build/check/v3d-ignored.txt`
(`capture_cost`, `a_builtin_on_a_vec2_records_two_lanes`,
`decoder_agrees_with_mesa_on_every_field_combination`,
`listing_matches_mesa_on_the_corpus`), each with its reason.

### Other rows (gate, all ok unless marked)

- `build yakogl-capi (no_std)`; `gen precompiled tables for the tier configurations`.
- Product arms: `check osal-zephyr,gpu-{vivante,powervr,v3d},shaderc-online,no-shader-table` and `osal-zephyr,gpu-powervr,precompiled-shaders` for `aarch64-unknown-none` (4 rows); `yakogl-drivers` for `aarch64-unknown-none-softfloat`.
- Generated files: `driver/src/v3d/generated`, `driver/src/pvr/generated`, `gl_test_shaders{,_es100}.h`, `pvr_usc_*_blob.h`, `src/pvr/internal.rs`, `src/vivante/internal.rs`, `src/pvr/csbgen.rs`, `imx8mp regs.rs`, `ti_pr114866.rs`, `pvr_rgx_core.h`, `tests/reference/blobs.rs` (pco_shc, 225 s); `third_party` sha256.
- Precompiled table proofs, 4 rows (`gpu-powervr`, `gpu-vivante`, each with `report-es2`).
- C checks: `cc` v3d FFI pins, job builder, textures; `yakogl_drivers.h` pins; `gl_test.c` x3; v3d `tex_filter_ref`, `tex_tiling_bench`, `tfu_case`, `tex_upload`, `osal_frees`, `osal_granules`, `hw_mock`, `tex_check`, `cl_golden`, `job_check`; pvr `vdm_field`, `render_regs`, `ddk_abi`, `rgxlayer`, `km`, pool (28 tests), `rti_arm`; osal `pending_frees`, `bounds`, `log`, `baremetal_config`, `linux_heap`; `gc7000 osal_frees`; `gl_test` set walker count and case 44 probe.
- Artefact rows: `no table in the artefact`, `no Mesa reference in the artefact`, `no test knob in the artefact`.
- Tools: fault-record x2, shader-table guards, `precompiled.list` pairs, `deqp-uart-tally`, `common-tally`, `qpa-from-uart`, TFTP server, UART phases, `gles3-mustpass`, `am62p-gles3-tree`, `cts-selftest.sh`, `api_coverage.py` x3, `board-sudo-setup.sh --dry-run`, `hal-nxp gen_regs`, `board.sh dry-run matrix and replays` (`tools/board_selftest.sh`, 25 s, **ok**), `check.sh test cache`, `check.sh --jobs`.
- Zephyr: `imx8mp gc_gl_test image` ok, `rpi4 v3d_gl_test image` ok.
- `glmark2 corpus generated from GLMARK2_SRC ...` ok (65 s).
- **Failed:** `check.sh --changed ...` (self-test), `zephyr am62p gl_test image`, `zephyr am62p gl_test image without a compiler` (table above).

Total: 145 ok + 3 failed = 148 rows, 0 gaps. A previous `tools/check.sh --quick`
run in the same tree gave 117 ok, 1 failed (the same self-test row).

## What to do next

1. `clippy-gate-fix` has nothing red to fix at the tip. Its scope in
   `draw.rs`/`texture.rs` should be limited to what is real on the branch
   it lands on; re-run the clippy rows there, because the recorded
   lines are not at the tip.
2. Redoing review items: expect exactly the three red rows above on a worker
   without the firmware header, and the self-test pin on every worker. A
   result whose own diff is clean should be judged on the other 145 rows.
3. A new item is needed for the three `tools/check.sh` / self-test fixes and
   the `build/check-fail` path clash (all in `tools/`; none touches `src/`).
4. The farm decision (copy `pvr_fw_blob.h` to full-gate workers) is the
   owner's; it is a ccfarm project setting, not a repository change.
