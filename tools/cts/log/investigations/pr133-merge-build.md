# PR #133: origin/work/int-pr49 + origin/main builds the three gl_test images

Date: 2026-10-12. Host-only check (no board). Nothing was pushed.

Question: does PR #133's branch, merged with main, build? The board
sessions will build their images from this merge plus a new gl_test case.

Answer: yes. `tools/check.sh --changed origin/main` is clean (147 ok,
0 failed, 0 gaps), the three gl_test images build with `-Werror` on our C
files, and `tests/es3_tex_levels.rs` passes (9 of 9) on all three GPU
features. One environment step was needed first (see "The firmware header").

## Commits

| What | Commit |
|---|---|
| int-pr49 tip (`origin/work/int-pr49`, the PR head) | `af9ecc535ac00d84fb3f0fa9af6db1918b8c9a2d` |
| main tip (`origin/main`) | `2fe4b8798b5faf2807f31dd486296f21045f4abf` |
| merge (`git merge origin/main` on int-pr49, scratch branch, local only) | `a408d402d64ef318c82be95712161189dac2da69` |

The merge was made by the `ort` strategy with no conflicts (`Auto-merging
src/gl/texture.rs`; 66 files changed). The scratch branch was not pushed.
The merge commit is not kept in this repository. Rebuild it with
`git merge origin/work/int-pr49` onto `origin/main` to reproduce it.

`--changed origin/main` on the merge selects two paths, `src/gl/texture.rs`
and `tests/es3_tex_levels.rs` (the PR's own change), with scope "all": every
row runs.

## Environment used

| Name | Value | Result |
|---|---|---|
| `ZEPHYR_WS` | `$CCFARM_DEPS_DIR/zephyr-ws` | present |
| `ZEPHYR_SDK_INSTALL_DIR` | `$CCFARM_DEPS_DIR/zephyr-sdk-1.0.1` | present |
| `PVR_FW_BLOB` | `$CCFARM_DEPS_DIR/pvr_fw_blob.h` | present, but see below |
| `GLMARK2_SRC`, `HAL_NXP_DIR`, `GLSLANG`, `PCO_SHC` | set | used by their rows |

Not set on this machine and not needed by any row that ran: `CC_aarch64_unknown_none`,
`DEQP_WORK`, `MESA_V3D_TREE`, `NOSTD_PATCH_DIR`. No row reported a gap for
them.

### The firmware header (first run: 2 failures; second run: 0)

The first full run of `tools/check.sh --changed origin/main` had two
failures, both in the AM62P images and both the same error:

```
FAIL  zephyr  am62p gl_test image, -Werror on our C files [46s]
driver/src/pvr_fw_boot.c:304:10: fatal error: fw/pvr_fw_blob.h: No such file or directory
(the whole log: build/zephyr-werror/am62p.log)
FAIL  zephyr  am62p gl_test image without a compiler (precompiled shaders only), -Werror [43s]
driver/src/pvr_fw_boot.c:304:10: fatal error: fw/pvr_fw_blob.h: No such file or directory
(the whole log: build/zephyr-werror/am62p-precompiled-only.log)
check.sh: 145 ok, 2 failed, 0 gap(s) ... in 1627s
```

Cause: the AM62P build includes `fw/pvr_fw_blob.h` from `driver/src/fw/`. The
file is not in git (it is the TI/Imagination firmware). `check.sh` does not
read `PVR_FW_BLOB`; it only looks at the checkout path. This is not a
problem of the merge. I copied the header from `PVR_FW_BLOB` to
`driver/src/fw/pvr_fw_blob.h` (the path is git-ignored, so the tree stayed
clean) and ran `tools/check.sh --changed origin/main` again.

Board sessions that build an AM62P image must do the same copy first.

An earlier attempt was killed after 10 minutes by the tool's default
background limit, not by a failure. 41 rows had passed by then, all ok.

## `tools/check.sh --changed origin/main` (second run, final)

```
check.sh: 147 ok, 0 failed, 0 gap(s) (a gap was not run: it is not a pass),
0 skipped (--changed origin/main, scopes: all), 0 not run (passed on main)
in 865s; test binaries: 603 run, 0 skipped (passed on main); runner: the
binaries concurrently (no cargo-nextest)
check.sh: clean
```

Every row passed. Rows by group (all "ok"):

- cargo fmt --check; yakogl-capi no_std build.
- clippy and test for the host-test configurations: gpu-powervr, gpu-vivante
  and gpu-v3d, each with the shader back ends check.sh lists (shaderc-online,
  precompiled-shaders, report-es2, no-shader-table, test-knobs).
- The v3d test binaries (v3d_sim_mesa, v3d_cube_noseamless, v3d_clear,
  v3d_glmark2_mem, es3_int_rt, es3_tf_api, es3_srgb).
- gen / sha256 rows (generated headers equal their generators; third_party
  files equal upstream).
- cc and run rows for the C side: v3d, pvr, osal, gc7000 checks.
- **gl_test host cc rows** (the ones that compile `gl_test.c`):
  - `cc gl_test.c -DGL_TEST_HOST=1` ok
  - `cc gl_test.c -DGL_TEST_USER=1 -DYAKOGL_GPU_VIVANTE=1` ok
  - `cc gl_test.c -DGL_TEST_USER=1 -DYAKOGL_GPU_VIVANTE=1 -DYAKOGL_TEST_KNOBS=1` ok
  - `run gl_test set walker count` ok; `run gl_test case 44 probe decode` ok
  - `gen gl_test_shaders{,_es100}.h == gen_gl_test_shaders.py` ok
- check rows on aarch64-unknown-none (osal-zephyr with each GPU).
- tools self-tests (cts-selftest, board.sh dry run, check.sh's own tests).
- yakogl-tools, yakogl-precompile, yakogl-drivers, slint/cluster/triangle checks.
- precompiled tables (powervr, vivante, with and without report-es2).
- **The Zephyr images, `-Werror` on our C files** (second run):

| Image row | Result | Time |
|---|---|---|
| `zephyr  am62p gl_test image, -Werror on our C files` (verdin_am62p/am62p54/a53, `gl_test`) | ok | 11s |
| `zephyr  am62p gl_test image without a compiler (precompiled shaders only), -Werror` | ok | 16s |
| `zephyr  imx8mp gc_gl_test image, -Werror on our C files` (verdin_imx8mp/mimx8ml8/a53, `gc_gl_test`) | ok | 13s |
| `zephyr  rpi4 v3d_gl_test image, -Werror on our C files` (rpi_4b, `v3d_gl_test`) | ok | 10s |

The first run built the imx8mp and rpi4 images ok too (51 s each); the am62p
pair failed only for the missing header above. The image builds are the
`zephyr_werror_image` rows of check.sh, so they are the same builds this
item asked for (`west build -p always`, `-DYAKOGL_WERROR=ON`). The image
files stay in `build/zephyr-werror/{am62p,am62p-precompiled-only,imx8mp,rpi4}`
of the scratch workspace and are not committed.

Ignored tests listed by the v3d rows (not failures, from
`build/check/v3d-ignored.txt`): `gl::tf_capture_tests::capture_cost`,
`...a_builtin_on_a_vec2_records_two_lanes`,
`...decoder_agrees_with_mesa_on_every_field_combination` (needs `V3D_QPU_REF`),
`...listing_matches_mesa_on_the_corpus` (needs `V3D_KEEP_DIR`).

## `cargo test --test es3_tex_levels` on the merge

A bare `cargo test --test es3_tex_levels` does not compile: the crate needs
one GPU feature and one shader back end (the test file header gives the
command). This is how the crate is built, not a merge problem. Run with the
features of the header, on each GPU:

```
cargo test --no-default-features --features host-test,gpu-powervr --test es3_tex_levels
  test result: ok. 9 passed; 0 failed; 0 ignored
cargo test --no-default-features --features host-test,gpu-vivante --test es3_tex_levels
  test result: ok. 9 passed; 0 failed; 0 ignored
cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_tex_levels
  test result: ok. 9 passed; 0 failed; 0 ignored
```

The nine tests: `every_level_of_a_3d_texture_keeps_its_texels`,
`every_level_of_a_2d_array_keeps_its_texels`,
`stacked_3d_levels_read_back_through_a_framebuffer`,
`sub_images_reach_every_level_of_a_3d_storage_texture`,
`generate_mipmap_filters_a_3d_texture_in_depth_too`,
`copy_tex_sub_image_3d_writes_a_slice_of_a_level_above_0`,
`a_3d_level_of_another_size_or_format_is_not_written`,
`a_3d_level_past_a_stacked_chain_is_recorded_only`,
`a_powervr_npot_array_records_its_levels_only`.

## What this does not show

- No board ran. The images were built, not booted.
- The new gl_test case for PR #133 does not exist yet (it is the `case`
  item); these rows ran the existing gl_test.c.
- The merge was made against main `2fe4b879`. If main moves before the board
  sessions, redo the merge and the check.
