<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Frames whose tile follows the GPU's tile buffer: Mesa 26.2.1 references

pandecode traces of Mesa 26.2.1's gallium panfrost on its drm-shim, drawn
as the GPU the file name ends in (`drm_shim_ref.sh run --gpu`). They are
the frames whose tile size, colour buffer allocation, render-target
tile-buffer offsets or tiler hierarchy mask depend on the model's tile
buffer (docs/notes/mali-g310-vs-g610.md): G310v1 has half the G610's
(`MODEL_TB_SIZES(16384, 8192)` against `(32768, 16384)`,
src/panfrost/model/pan_model.c:88-96), G310v4 the G610's (:97). The frame
builder's tests compare its framebuffer parameters, render targets and
tiler context with them (src/mali/frame_mrt_tests.rs: three and four
targets; src/mali/frame_split_tests.rs: the 4x frame).

| File | Drawn with |
| --- | --- |
| `mrt3.g310v1.trace` | `tools/mali/drm_shim_ref.sh run <work> tools/shaders/es300/mrt3.vert tools/shaders/es300/mrt3.frag mrt3.g310v1 --mrt 3 --gpu g310v1` |
| `mrt3.g310v4.trace` | the same with `--gpu g310v4` |
| `mrt4.g610.trace` | `tools/mali/drm_shim_ref.sh run <work> tools/mali/testdata/tile-budget/mrt4.vert tools/mali/testdata/tile-budget/mrt4.frag mrt4.g610 --mrt 4 --gpu g610` |
| `mrt4.g310v1.trace`, `mrt4.g310v4.trace` | the same with `--gpu g310v1`, `--gpu g310v4` |
| `ms4.g610.trace` | `tools/mali/drm_shim_ref.sh run <work> tools/mali/testdata/tri.vert tools/mali/testdata/tri.frag ms4.g610 --samples 4 --gpu g610` |
| `ms4.g310v1.trace`, `ms4.g310v4.trace` | the same with `--gpu g310v1`, `--gpu g310v4` |

The G610's three-target frame is `../fsout_106_mrt3.trace`. `<work>` is a
`drm_shim_ref.sh build` of the 26.2.1 tarball
(`third_party/mesa-26.2.1/UPSTREAM`). A redraw gives the same bytes but the
`Shader 0x... (GPU VA ...)` host pointers. Only the `.trace` of each draw is
kept.

What the tile buffer moves (G610 and G310v4 against G310v1):

| Frame | Effective Tile Size | Color Buffer Allocation | Internal Buffer Offsets | Hierarchy Mask |
| --- | --- | --- | --- | --- |
| `mrt3` | 1024 / 512 | 12288 / 6144 | 4096, 8192 / 2048, 4096 | 254 / 254 |
| `mrt4` | 1024 / 512 | 16384 / 8192 | 4096, 8192, 12288 / 2048, 4096, 6144 | 254 / 254 |
| `ms4` | 512 / 256 | 8192 / 4096 | (one target) | 254 / 255 |

The `ms4` tile is the Z budget's (pan_desc.c:617-625, 4 bytes of depth a
sample): 8192 >> 4 = 512 on the G610, 4096 >> 4 = 256 on the G310v1, below
the colour budget's 1024 and 512.
