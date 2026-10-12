<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PR #133: the three boards' gl_test images from 066a2db8

Date: 2026-10-12. Host-only item `prebuild-images` of plan #141. Nothing ran
on a board; the images were built, not booted. Nothing was pushed.

## Result

Six images built, rc=0 each, for the three GPUs: the default (whole-set)
image and a case-206 image per board. **Case 206 is compiled into all six**
(`gl_test.c` at 066a2db8 holds it; `es3levels` is in each binary). It
**runs** only in the case-206 images: it is not in the regression set's
`order` list (`gl_test.c:21963`), so the default AM62P and i.MX8MP images
(case 7) and the Pi 4 default list do not run it.

## Commit and tree

- Commit `066a2db81d40e519efc05dcbdcdd4e359468fe9a` (`066a2db8`,
  `refs/ccfarm/origin/heads/ccfarm/01a12f31-af26-7505-a35a-153b04454dae`,
  "gl_test: case 206 reads back 3D and 2D-array mip levels 1 and 2 (PR #133)").
- Tree: `$CCFARM_DEPS_DIR/pr133-images-066a2db8/wt`, made by
  `git worktree add --detach … 066a2db8`; `git describe --always --dirty` is
  `066a2db` (clean). The build scripts print `build 066a2db`.
- `driver/src/fw/pvr_fw_blob.h` was copied from `$CCFARM_DEPS_DIR/pvr_fw_blob.h`
  for the AM62P (git-ignored, not committed).
- Environment: `ZEPHYR_WS=$CCFARM_DEPS_DIR/zephyr-ws`,
  `WEST_VENV=$ZEPHYR_WS/.venv`, `ZEPHYR_SDK_INSTALL_DIR=$CCFARM_DEPS_DIR/zephyr-sdk-1.0.1`,
  `RUSTUP_TOOLCHAIN=nightly`, `$HOME/.cargo/bin` first on `PATH`.
  `BOARD_HDMI_TIMINGS`, `BOARD_UART`, `MESA_V3D_TREE` are not inputs of these
  builds and were not needed.

## Build lines

Run in the tree (the wrapper `$CCFARM_DEPS_DIR/pr133-images-066a2db8/scripts/build.sh
<board> whole|c206` sets the environment above and runs these):

Whole set (the baselines' line, GT row, the sample's default case list):

```
tools/cts/common-images.sh am62p  tools/cts/common-tree.txt $CCFARM_DEPS_DIR/pr133-images-066a2db8/img-am62p  GT
tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt $CCFARM_DEPS_DIR/pr133-images-066a2db8/img-imx8mp GT
tools/cts/common-images.sh pi4    tools/cts/common-tree.txt $CCFARM_DEPS_DIR/pr133-images-066a2db8/img-pi4    GT
```

Case 206 alone: the same west build `common-images.sh` runs, with the case
option added (`common-images.sh` takes no extra argument):

```
cd $ZEPHYR_WS && west build -p always -b <zephyr board> -d <dir> <tree>/driver/samples/zephyr/<sample> -- \
    -DZEPHYR_MODULES=$(tools/cts/zephyr-modules.sh $ZEPHYR_WS) -DYAKOGL_DIR=<tree> <arm options> <case option>
```

| Board | West board | Sample | Arm options | Case option |
|---|---|---|---|---|
| AM62P | `verdin_am62p/am62p54/a53` | `gl_test` | `-DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n` | `-DCONFIG_GL_TEST_CASES="206"` |
| i.MX8MP | `verdin_imx8mp/mimx8ml8/a53` | `gc_gl_test` | `-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y` | `-DCONFIG_GC_GL_TEST_CASE=206` |
| Pi 4 | `rpi_4b` | `v3d_gl_test` | none | `-DCONFIG_V3D_GL_TEST_CASES="206"` |

The images are checked in the build's own `.config`: the whole-set images
have `GL_TEST_CASE=7`/`GL_TEST_CASES=""` (AM62P), `GC_GL_TEST_CASE=7`
(i.MX8MP), `V3D_GL_TEST_CASES="5,58,59,66,67,102,3,9,11,12,13,4"` (Pi 4); the
case-206 images have `GL_TEST_CASES="206"`, `GC_GL_TEST_CASE=206`,
`V3D_GL_TEST_CASES="206"`. The build ids are those of the output below.

## Artefacts

Paths are relative to `$CCFARM_DEPS_DIR`; each is `<dir>/GT/zephyr/zephyr.bin`
(the build log is `<dir>/GT.log`).

| Board | Image | Directory | Bytes | sha256 | Cases that run | Case 206 compiled in | Build ids |
|---|---|---|---|---|---|---|---|
| AM62P | whole set | `pr133-images-066a2db8/img-am62p` | 2611840 | `df7109ac6dbb42023d3312811b274180e554376c9b1d6d6bb559fc7f52ae54ac` | the regression set (case 7); 206 not in it | yes | `PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=066a2db` |
| AM62P | case 206 | `pr133-images-066a2db8/img-am62p-206` | 2611840 | `1d0cc4edb7f4c850e7be32b2697a7256ecc1d8b4947b0677e9104ba105e1c202` | 206 | yes | same tree (not read from build.ninja here) |
| i.MX8MP | whole set | `pr133-images-066a2db8/img-imx8mp` | 1998048 | `bbc7491891536298bcb6af35c365cc19e90671b219bf770fd3cf55ed59cdd847` | the regression set (case 7); 206 not in it | yes | `GC_BUILD_STAMP=2026-10-12T10:30:29Z` |
| i.MX8MP | case 206 | `pr133-images-066a2db8/img-imx8mp-206` | 1998048 | `9bbfe7ef9131f719eff3132e972fb206b4b14b916ddf33e3e357325a07e5974e` | 206 | yes | not read |
| Pi 4 | whole set | `pr133-images-066a2db8/img-pi4` | 2136856 | `b1cd00fd18f681fbe495623014bb3b4096df2294094f2e8c0761bb1928c648c4` | `5,58,59,66,67,102,3,9,11,12,13,4`; 206 not in it | yes | `V3D_BUILD=066a2db` |
| Pi 4 | case 206 | `pr133-images-066a2db8/img-pi4-206` | 2136856 | `1569b52c7b12673b4055e96311f66688d9e330636a55582a127cf06f3fafe015` | 206 | yes | not read |

The whole-set and case-206 images of one board have the same size: the case
list is a runtime string, the code is the same.

`common-images.sh` also checked each whole-set image for its board's
watchdog arm line and a build line (rc=0 means both present). It does not do
that for the case-206 builds, which were made by hand.

## What this shows and does not

- Shows: `gl_test.c` with case 206 compiles and links into real images for
  AM62P (PowerVR), i.MX8MP (Vivante) and Pi 4 (V3D) from 066a2db8, with
  the same build the board sessions make (these are not the `-Werror` rows
  of `tools/check.sh`).
- Does not show: any run. No image was booted.
- **Pi 4:** case 206 prints `SKIP (the V3D does not sample 3D or 2D-array
  textures)` (`gl_test.c:7215`); an ES 2.0 library prints a SKIP too (line
  7230). The Pi 4 case-206 image is a boot proof, not a pixel check.
- **Whole-set runs do not exercise case 206.** The board sessions need the
  case-206 image (or a list that adds 206) for the PR's evidence, and the
  whole-set image for the regression set.
- The builds are not bit-reproducible: the i.MX8MP ids carry a build
  timestamp, so a rebuild changes its sha256. Use these files as built; the
  board sessions can fetch them with `fetch_from_farm` by the paths above.
