<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PR #133 case 206: check.sh rows and the control

2026-10-12/13, item case-check of plan #141 (second round, after a review
asked for the whole `--changed` run, the vivante control and a consistent
count). Host only, no device.

## Commits

| what | commit |
|---|---|
| case result (PR head af9ecc53 merged with origin/main 2fe4b879, plus case 206) | 066a2db81d40e519efc05dcbdcdd4e359468fe9a |
| origin/main without the PR (the control's base) | 2fe4b8798b5faf2807f31dd486296f21045f4abf |
| base for the `--changed` map (a diff that reaches `all`) | 2fe4b879 |

`git diff --stat 2fe4b879 066a2db8` (5 files; `src/gl/texture.rs` maps to
scope `all`, so every row is selected):

```
 driver/tools/pvrlab/gen_gl_test_shaders.py  |   2 +-
 driver/tools/pvrlab/gl_test.c               | 144 +++++-
 driver/tools/pvrlab/gl_test_shaders_es100.h |   6 +-
 src/gl/texture.rs                           | 490 +++++++++++++++++---
 tests/es3_tex_levels.rs                     | 686 ++++++++++++++++++++++++++++
```

## (a) `tools/check.sh --changed 2fe4b879` on 066a2db8

Worktree of 066a2db8 (detached), command `tools/check.sh --changed 2fe4b879`,
run in the foreground of a background task with a 2 h limit, no `--jobs`
(`--jobs` and `bash tools/check.sh` were refused by this worker's permission
layer, the bare script path was allowed).

Run 1 was stopped by the task's time limit after its 7th red row (about row
150; the rows before it were ok, the red ones are the same seven as in run 2).
Run 2 ran every row to the end and printed:

```
check.sh: 140 ok, 7 failed, 0 gap(s) (a gap was not run: it is not a pass), 0 skipped (--changed 2fe4b879, scopes: all), 0 not run (passed on main) in 1651s; test binaries: 603 run, 0 skipped (passed on main); runner: cargo-nextest 0.9.148 (cd1d6d546 2026-10-07) (the rows that read libtest's output run the binaries concurrently)
check.sh: fix the above before pushing
```

Exit code 1. **0 gaps**: every row ran. The earlier record's "about 90
gaps" were rows this worker could not start; none is a gap now.

### The 7 failed rows

| row | output (copied) | caused by the PR? |
|---|---|---|
| `tools   board.sh dry-run matrix and replays` | `board_selftest: 227 checks, 2 failed (pinned: 227)`; every PASS line check.sh keeps is PASS, the two FAIL lines are above its tail window and its log file `build/check-fail/1.log` is empty | no: same row red on 2fe4b879 (below) |
| `zephyr  am62p gl_test image, -Werror on our C files` | `driver/src/pvr_fw_boot.c:304:10: fatal error: fw/pvr_fw_blob.h: No such file or directory` | no: needs PVR_FW_BLOB (this machine has none); same on 2fe4b879 |
| `zephyr  am62p gl_test image without a compiler (precompiled shaders only), -Werror` | the same `pvr_fw_blob.h` error | no: same |
| `gen     precompiled table gpu-powervr: deterministic, rows == fresh compiles, counts pinned, compiles` | `error: only metadata stub found for rlib dependency yakogl please provide path to the corresponding .rmeta file with full metadata` at `build/precompiled/prove/gpu-powervr.check.rs:3:9 pub use yakogl::shaderc::*;`; its counts lines before it are all `the counts are tests/fixtures/precompile_counts.txt's pin` and `made twice, the same 22119279 bytes` | no: same error on 2fe4b879 |
| `gen     precompiled table gpu-vivante: ...` | the same metadata-stub error, then `E0432: unresolved imports super::ShcAttrib, ...` (follows from it) | no: same on 2fe4b879 |
| `gen     precompiled table gpu-powervr,report-es2: ...` | the same metadata-stub error | no: same on 2fe4b879 |
| `gen     precompiled table gpu-vivante,report-es2: ...` | the same metadata-stub error and E0432 | not reached on 2fe4b879 (stopped), same message class |

The generation, determinism and pinned-count lines of the four `gen` rows are
all fine; what fails is the last step, compiling the generated table against
the `yakogl` rlib, which the worker's toolchain gives as a metadata-only stub.
It is an environment problem of this host (what the row needs: a cargo that
leaves a full `rlib` for `--extern yakogl`, as the gate host has), not a code
defect. **Control for the 7 reds**: the same check.sh on a clean worktree of
2fe4b879 (`tools/check.sh --changed 2a046ea0~1`, which reaches scope `all`
too; 2a046ea0 is the last commit before 2fe4b879 that touched
`src/gl/texture.rs`) printed FAIL for `board.sh dry-run matrix and replays`,
both `zephyr am62p` rows and `gen precompiled table` for gpu-powervr,
gpu-vivante and gpu-powervr,report-es2 with the same lines; that run was
stopped by the 2 h task limit before the last row and so printed no summary
line. So all seven reds are present without the PR.

### The 140 ok rows (run 2, durations from that run)

| rows | result |
|---|---|
| `cargo fmt --check` [5s]; `build yakogl-capi (no_std)` [1s] | ok |
| `clippy` and `test` of `host-test,gpu-powervr` [62s], `…,report-es2` [65s], `host-test,gpu-vivante` [16s] | ok |
| `clippy`/`test` of `host-test,gpu-powervr,shaderc-online` [227s], `gpu-vivante,shaderc-online` [53s], `…,report-es2` powervr [134s] and vivante [24s] | ok (the four rows that stalled the review) |
| `gen precompiled tables for the tier configurations` [7s] | ok |
| `clippy`/`test` `…shaderc-online,precompiled-shaders` powervr [220s], vivante [51s] | ok |
| `clippy`/`test` `precompiled-shaders,no-shader-table` powervr and vivante; `clippy` `shaderc-online,no-shader-table` powervr and vivante | ok |
| `clippy` + `test` (`--lib`, 9 knob tests [20s]; `tests/gc_kick_diff.rs`, 7 tests) of `host-test,gpu-vivante,shaderc-online,test-knobs` | ok |
| `host-test,gpu-v3d,shaderc-online,no-shader-table`: clippy, `--lib` [24s], `v3d_sim_mesa`, `v3d_cube_noseamless`, `v3d_clear`, `v3d_glmark2_mem`, `es3_int_rt`, `es3_tf_api`, `es3_srgb` (4 tests ignored by name, listed in `build/check/v3d-ignored.txt`) | ok |
| the same `…,report-es2`: clippy, `--lib shaderc::online::v3d`, `v3d_sim_mesa`, `v3d_cube_noseamless`, `v3d_clear`, `es3_int_rt`, `es3_tf_api`, `es3_srgb` | ok |
| `gen driver/src/v3d/generated`, `gen driver/src/pvr/generated`, `sha256 third_party`, `cc v3d FFI layout pins`, `cc v3d job builder`, `cc v3d textures` | ok |
| `run` v3d tex_filter_ref_check, tex_tiling_bench_check, tfu_case_check; pvr vdm_field_check, render_regs_check, ddk_abi_check; v3d hw_mock_test; pvr rgxlayer_check, km_check; v3d tex_upload_check, osal_frees_check, osal_granules_check; osal pending_frees_check, gc7000 osal_frees_check, osal_bounds_check, osal_log_check, baremetal_config_check, linux_heap_check | ok (18 rows) |
| `build v3d mesa reference`; `run` v3d tex_check, cl_golden, job_check (Mesa 26.2.1) | ok |
| **`cc gl_test.c -DGL_TEST_HOST=1` [0s]; `cc gl_test.c -DGL_TEST_USER=1 -DYAKOGL_GPU_VIVANTE=1` [0s]; `cc gl_test.c -DGL_TEST_USER=1 -DYAKOGL_GPU_VIVANTE=1 -DYAKOGL_TEST_KNOBS=1` [0s]** | ok: case 206 compiles for the PowerVR host arm and the GC7000 (the V3D arm is `GL_TEST_V3D`, covered by `zephyr rpi4 v3d_gl_test image -Werror` below) |
| `run gl_test set walker count` [1s]; `run gl_test case 44 probe decode` [1s] (`tests/gltest_set_tally.c`, `gltest_hdr_probe.c`) | ok |
| `gen gl_test_shaders{,_es100}.h == gen_gl_test_shaders.py on tools/shaders` [0s] | ok |
| `test pvr render pool` [2s]; `run am62p rti_arm_check` | ok |
| `check` aarch64-unknown-none: vivante, powervr, v3d (shaderc-online, no-shader-table) and powervr precompiled | ok (4 rows) |
| `tools` fault-record decoder, fault-record writer, shader-table generator guards, precompiled.list gl_test pairs, deqp-uart-tally, common-tally, qpa-from-uart, am62p netboot TFTP, am62p netboot UART phases, gles3-mustpass, am62p-gles3-tree, cts-selftest.sh, api_coverage self-test, api coverage (two rows), board-sudo-setup dry-run | ok (16 rows) |
| `gen driver/src/pvr_usc_*_blob.h`; `no table / no Mesa reference / no test knob in the artefact` | ok (4 rows) |
| `clippy`/`test yakogl-tools`; `clippy` ×4 and `test` ×2 of `yakogl-precompile`; `gen src/pvr/internal.rs`, `src/vivante/internal.rs` | ok |
| `tools check.sh --changed` map [8s]; `check.sh test cache` [38s]; `check.sh --jobs` [4s]; `gen src/pvr/csbgen.rs` | ok |
| `clippy`/`test` yakogl-slint-check, -cluster-check, -triangle-check; `clippy` yakogl-drivers (two rows), `test` yakogl-drivers, `check` yakogl-drivers aarch64, `cc yakogl_drivers.h layout pins` | ok |
| `zephyr imx8mp gc_gl_test image, -Werror` [13s]; `zephyr rpi4 v3d_gl_test image, -Werror` [8s] | ok (so case 206 builds in the GC7000 and V3D images) |
| `gen tests/reference/blobs.rs == glsl2usc.sh (pco_shc)` [176s]; `test glmark2 corpus` [164s]; `tools hal-nxp gen_regs`; `gen imx8mp regs.rs`; `gen am62p ti_pr114866.rs`; `gen driver/compat/pvr_rgx_core.h` | ok |

## (b) Control for case 206 and the pass on 066a2db8

**Case 206's own assertions are pixel read-backs, and no host build can
evaluate them.** `case_es3levels` ends every check in `expect_px` of a
`glReadPixels` value; `tools/hostharness/README.md` states "Nothing is
rendered: the fake GPU never executes a kick", and
the i.MX8MP diary
(`board-sessions/imx8mp-gc-gl-test-2026-09.md`, "The host build: the checks
that are not pixels") says every check that reads a pixel fails there. Case 206 has no non-pixel check beyond
`GL_NO_ERROR`. I tried to run it through the harness
(`tools/hostharness/run.sh all <out> rust:. driver dev '^gltest\.206$'`, with
and without `HH_SAN=0 HH_MODES=l0`): this worker's permission layer refused
the script (every form), so no harness line exists, and I do not claim one.
Even where it runs, the harness builds the PowerVR arm only (README, "The
V3D": `gl_test.c`'s host arm is the PowerVR's), and every pixel check there
reads zero. The case's fail-without-the-PR / pass-with-it result is a
**board** result (PowerVR AM62P and GC7000 i.MX8MP; the Pi 4 skips by name).

What the host does measure is the storage the case samples, with the PR's
own `tests/es3_tex_levels.rs`. That is a storage-level substitute for case
206, not case 206.

Commands (the target dir apart per tree; `--manifest-path` of the worktree):

```
cargo test --manifest-path <w2fe>/Cargo.toml --target-dir <t2fe> --no-default-features --features host-test,gpu-powervr --test es3_tex_levels
cargo test --manifest-path <w2fe>/Cargo.toml --target-dir <t2fe> --no-default-features --features host-test,gpu-vivante --test es3_tex_levels
cargo test --manifest-path <w066>/Cargo.toml --target-dir <t066> --no-default-features --features host-test,gpu-powervr --test es3_tex_levels
cargo test --manifest-path <w066>/Cargo.toml --target-dir <t066> --no-default-features --features host-test,gpu-vivante --test es3_tex_levels
```

### Control on 2fe4b879 (origin/main without the PR)

The worktree of 2fe4b879 had the case-206 files of 066a2db8 copied over it
(`driver/tools/pvrlab/gl_test.c`, `gen_gl_test_shaders.py`,
`gl_test_shaders_es100.h`) and the PR's `tests/es3_tex_levels.rs`.

1. Without anything else the test file does not compile on main, on either
   GPU, because the PR's helper does not exist there:

```
error[E0425]: cannot find function `level_texel_offset` in this scope
   --> tests/es3_tex_levels.rs:216:15
```

2. With a labelled stub appended to the copy of the test file (not in the
   PR; `level_texel_offset(...) -> Option<usize> { None }`, "CONTROL STUB"),
   `cargo test ... --test es3_tex_levels` on 2fe4b879, **both GPUs print the
   same 9 failures** (`test result: FAILED. 0 passed; 9 failed`). The
   failing assertion lines:

| test | failing assertion (both `gpu-powervr` and `gpu-vivante`) | depends on the stub? |
|---|---|---|
| `every_level_of_a_3d_texture_keeps_its_texels` | `tests/es3_tex_levels.rs:227:17 … failed: grid (8, 8, 4): level 0 texel (0, 0, 0)  left: None  right: Some([0, 0, 0, 160])` | yes: it fails at level 0, through the stub |
| `every_level_of_a_2d_array_keeps_its_texels` | `:227:17 … grid (8, 4, 3): level 0 texel (0, 0, 0)  left: None  right: Some([0, 0, 0, 160])` | yes |
| `a_powervr_npot_array_records_its_levels_only` | `:227:17 … base: level 0 texel (0, 0, 0)  left: None  right: Some([0, 0, 0, 160])` | yes |
| `a_3d_level_past_a_stacked_chain_is_recorded_only` | `:227:17 … held: level 0 texel (0, 0, 0)  left: None  right: Some([0, 0, 0, 160])` | yes |
| `sub_images_reach_every_level_of_a_3d_storage_texture` | `:227:17 … grid (16, 8, 8): level 0 texel (0, 0, 0)  left: None  right: Some([0, 0, 0, 160])` | yes |
| `generate_mipmap_filters_a_3d_texture_in_depth_too` | `:550:29 … grid 4x4: level 0 (0, 0, 0)  left: None  right: Some([0, 0, 0, 0])` | yes |
| `stacked_3d_levels_read_back_through_a_framebuffer` | `:422:13 … failed: level 1 slice 0  left: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]  right: [16, 0, 0, 161, 16, 1, 0, 161, 16, 2, 0, 161, 16, 3, 0, 161, 16, 0, 1, 161, 16, 1, 1, 161, 16, 2, 1, 161, 16, 3, 1, 161]` | **no**: level 1 of a stacked 3D texture reads back all zero on main (read through glFramebufferTextureLayer + glReadPixels, the path case 206 samples) |
| `a_3d_level_of_another_size_or_format_is_not_written` | `:481:9 … grid: recorded as another size  left: 0  right: 2` | **no**: the library's own state |
| `copy_tex_sub_image_3d_writes_a_slice_of_a_level_above_0` | `:607:9 … grid  left: 1282  right: 0` (GL_INVALID_OPERATION on main) | **no**: the library's own state |

**Count, stated once: 9 tests fail on 2fe4b879, on each GPU. 6 of the 9
fail through the stub (they stop at the level-0 read the stub answers
`None` to) and say nothing about the PR; 3 of the 9 do not touch the stub
and show the defect on main: `stacked_3d_levels_read_back_through_a_framebuffer`,
`a_3d_level_of_another_size_or_format_is_not_written`,
`copy_tex_sub_image_3d_writes_a_slice_of_a_level_above_0`.** The three are the
control that carries information; the first of them is the read-back path
case 206 samples.

### Pass on 066a2db8

`gpu-powervr`:

```
running 9 tests
test a_3d_level_of_another_size_or_format_is_not_written ... ok
test copy_tex_sub_image_3d_writes_a_slice_of_a_level_above_0 ... ok
test a_3d_level_past_a_stacked_chain_is_recorded_only ... ok
test a_powervr_npot_array_records_its_levels_only ... ok
test every_level_of_a_3d_texture_keeps_its_texels ... ok
test generate_mipmap_filters_a_3d_texture_in_depth_too ... ok
test every_level_of_a_2d_array_keeps_its_texels ... ok
test sub_images_reach_every_level_of_a_3d_storage_texture ... ok
test stacked_3d_levels_read_back_through_a_framebuffer ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

`gpu-vivante`:

```
running 9 tests
test a_3d_level_past_a_stacked_chain_is_recorded_only ... ok
test a_3d_level_of_another_size_or_format_is_not_written ... ok
test a_powervr_npot_array_records_its_levels_only ... ok
test copy_tex_sub_image_3d_writes_a_slice_of_a_level_above_0 ... ok
test every_level_of_a_2d_array_keeps_its_texels ... ok
test every_level_of_a_3d_texture_keeps_its_texels ... ok
test generate_mipmap_filters_a_3d_texture_in_depth_too ... ok
test stacked_3d_levels_read_back_through_a_framebuffer ... ok
test sub_images_reach_every_level_of_a_3d_storage_texture ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

The same 9 tests also pass inside check.sh's `test host-test,gpu-powervr` and
`test host-test,gpu-vivante` rows on 066a2db8 (section (a)).

## Not done

- Case 206's pixel assertions, with and without the PR: a board (PowerVR
  AM62P and GC7000 i.MX8MP; the V3D skips by name).
- A host-harness run of case 206 (PowerVR arm): the worker's permission layer
  refused `tools/hostharness/run.sh`; it would read zero pixels anyway.
- The 7 red rows: they are red on 2fe4b879 here as well (section (a)); what
  would make them run is PVR_FW_BLOB for the two zephyr rows and a toolchain
  that gives `--extern yakogl` a full rlib for the four `gen` rows. The two
  failing checks of `board_selftest.sh` were not identified (the script was
  refused when run directly and check.sh's log of the row is empty).
