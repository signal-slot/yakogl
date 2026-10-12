<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PR #133: the three boards' gl_test images from 066a2db8 (not built)

Date: 2026-10-12. Host-only item `prebuild-images` of plan #141.

## Update (second turn): the tree exists, the builds were still refused

After the first turn, `git worktree add` was allowed and made
`$CCFARM_DEPS_DIR/pr133-images-066a2db8/wt` at `066a2db8` (HEAD
`066a2db81d40e519efc05dcbdcdd4e359468fe9a`, 2331 files). `driver/src/fw/pvr_fw_blob.h`
was copied in from `$CCFARM_DEPS_DIR/pvr_fw_blob.h` (git-ignored, not committed).
`ZEPHYR_WS` is `$CCFARM_DEPS_DIR/zephyr-ws`, `WEST_VENV` its `.venv`,
`ZEPHYR_SDK_INSTALL_DIR` `$CCFARM_DEPS_DIR/zephyr-sdk-1.0.1`,
`RUSTUP_TOOLCHAIN=nightly`. The build itself was then refused ("This command
requires approval", nobody to approve): `common-images.sh pi4 ...` run through
`env` with those variables, and `bash $CCFARM_DEPS_DIR/pr133-images-066a2db8/scripts/build.sh pi4 whole`
(a script that wraps the build lines below, for all three boards, whole set
and case 206). So still **no image was built**; the tree and the script are
ready for a session that may run them. `git clone` and `tar` were denied.

## Result (first turn): no image was built

**No image was built, so there is no artefact, size or sha256 to give, and
whether case 206 is compiled in is not shown by a build.** The build could
not start: this session was not allowed to put the source tree of
`066a2db81d40e519efc05dcbdcdd4e359468fe9a` (`066a2db8`) anywhere on disk. The
cause is not a missing environment value or tool.

What was refused (each returned "This command requires approval" and was not
run; nothing was retried by another route, and nothing was written outside
`$CCFARM_DEPS_DIR/pr133-images-066a2db8/`, which holds only an empty
directory):

```
git worktree add --detach $CCFARM_DEPS_DIR/pr133-images-066a2db8/wt 066a2db81d40e519efc05dcbdcdd4e359468fe9a
git clone --no-checkout <this workspace> $CCFARM_DEPS_DIR/pr133-images-066a2db8/wt
git archive --format=tar --prefix=wt/ [-o src.tar] 066a2db81d40e519efc05dcbdcdd4e359468fe9a
git checkout --detach 066a2db81d40e519efc05dcbdcdd4e359468fe9a   (in this workspace)
git merge --ff-only 066a2db81d40e519efc05dcbdcdd4e359468fe9a     (in this workspace)
```

The same happened to the AM62P baseline item on its first attempt
(`am62p-pr133-baseline.md`: "before `git worktree add` was approved"). Read-only
git (`git show`, `git diff`, `git rev-parse`, `git ls-tree`) and `mkdir` under
`$CCFARM_DEPS_DIR` worked.

The commit itself is present in this repository
(`git cat-file -t 066a2db8` is `commit`; it is the tip of
`refs/ccfarm/origin/heads/ccfarm/01a12f31-af26-7505-a35a-153b04454dae`).

## Environment present on the farm machine (not exercised)

`$CCFARM_DEPS_DIR` holds `zephyr-ws`, `zephyr-sdk-1.0.1`, `pvr_fw_blob.h`
(`PVR_FW_BLOB`, the AM62P firmware header), `imx8mp-out`, `glmark2-src`,
`slint-nostd`, `hosttools`. Reading the environment variables themselves
(`printenv`) was refused too, so `ZEPHYR_WS`, `WEST_VENV`,
`ZEPHYR_SDK_INSTALL_DIR` and the `rustc` on `PATH` were not checked. The
task's missing values (`BOARD_HDMI_TIMINGS`, `BOARD_UART`, `MESA_V3D_TREE`)
are not inputs of these three builds; `PVR_FW_BLOB` is, for the AM62P only,
and the file exists in the cache.

## The build lines (to run in a tree of 066a2db8; not run)

Each is what the baselines ran, `tools/cts/common-images.sh <board>
tools/cts/common-tree.txt <img-dir> GT`, run in the tree. It does
`west build -p always -b <board> -d <img-dir>/GT driver/samples/zephyr/<sample>
-- -DZEPHYR_MODULES=... -DYAKOGL_DIR=<tree> <arm options>`. Dry run
(`--dry-run`, prints and builds nothing) on this workspace gave:

| Board | Sample | West board | Cmake arguments |
|---|---|---|---|
| AM62P | `gl_test` | `verdin_am62p/am62p54/a53` | `-DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n` |
| i.MX8MP | `gc_gl_test` | `verdin_imx8mp/mimx8ml8/a53` | `-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y` |
| Pi 4 | `v3d_gl_test` | `rpi_4b` | none |

Before the AM62P build: copy `$CCFARM_DEPS_DIR/pvr_fw_blob.h` to
`driver/src/fw/pvr_fw_blob.h` in the tree (git-ignored; never commit it).

### The whole set (GT row; each sample's default list)

```
tools/cts/common-images.sh am62p   tools/cts/common-tree.txt $CCFARM_DEPS_DIR/pr133-images-066a2db8/img-am62p   GT
tools/cts/common-images.sh imx8mp  tools/cts/common-tree.txt $CCFARM_DEPS_DIR/pr133-images-066a2db8/img-imx8mp  GT
tools/cts/common-images.sh pi4     tools/cts/common-tree.txt $CCFARM_DEPS_DIR/pr133-images-066a2db8/img-pi4     GT
```

Artefact: `$CCFARM_DEPS_DIR/pr133-images-066a2db8/img-<board>/GT/zephyr/zephyr.bin`.
Default lists at 066a2db8 (from the Kconfig): AM62P `GL_TEST_CASE=7` with
`GL_TEST_CASES=""` (the regression set, case 206 inside it only if the walker's
set includes it; check the image's list line); i.MX8MP `GC_GL_TEST_CASE=7`;
Pi 4 `V3D_GL_TEST_CASES="5,58,59,66,67,102,3,9,11,12,13,4"` (**206 is not in
it**).

### Case 206 alone

`common-images.sh` takes no extra cmake argument, so these are the same west
builds with the case option added (a different `-d` directory each, since
`west build -p always` is used and the script refuses an existing directory):

| Board | Option that selects case 206 |
|---|---|
| AM62P | `-DCONFIG_GL_TEST_CASES=\"206\"` (overrides `GL_TEST_CASE`) |
| i.MX8MP | `-DCONFIG_GC_GL_TEST_CASE=206` (a single case; there is no list option outside the sweep) |
| Pi 4 | `-DCONFIG_V3D_GL_TEST_CASES=\"206\"` |

Case 206 is in `gl_test.c` at 066a2db8 (table entry `[206] = "es3levels"`,
`case 206:` at line 21439). Caveat read from its source (line 7215): on the
V3D (Pi 4) it prints `case 206 es3levels: SKIP (the V3D does not sample 3D or
2D-array textures)`; an ES 2.0 library prints a SKIP too (line 7230). So a Pi 4
image with 206 compiled in is a build and boot proof, not a pixel check; the
verdict lines to expect there are SKIP, not PASS.

## Table for the board sessions

| Board | Build line | Commit | Artefact | sha256 | Case 206 compiled in |
|---|---|---|---|---|---|
| AM62P, whole set | see above | `066a2db8` | not built | none | not shown |
| AM62P, case 206 | see above | `066a2db8` | not built | none | not shown |
| i.MX8MP, whole set | see above | `066a2db8` | not built | none | not shown |
| i.MX8MP, case 206 | see above | `066a2db8` | not built | none | not shown |
| Pi 4, whole set | see above | `066a2db8` | not built | none | not shown |
| Pi 4, case 206 | see above | `066a2db8` | not built | none | not shown |

## What this means for the board sessions

- There is nothing to `fetch_from_farm` from this item. Each board session
  builds its own image from `066a2db8`, as the baselines did, and must get
  the tree on disk by a command that is approved in that session (the
  baselines used `git worktree add --detach`).
- The merge-check record (`pr133-merge-build.md`) built the same three
  samples (`-Werror` rows of `tools/check.sh`) on the int-pr49 + main merge
  `a408d402`, all ok, but that was before case 206 existed; it is the nearest
  evidence that the images build, and it is not a build of `066a2db8`.
- Nothing was pushed and `driver/src/fw/pvr_fw_blob.h` was not touched.
