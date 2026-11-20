<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# G310v2 against G610 frame traces (not G610 references)

These are pandecode traces of one draw of `../tri.vert` / `../tri.frag`.
Mesa 26.2.1's gallium panfrost drew it on its drm-shim twice, once as each
GPU. The `.g610.trace` / `.g310v2.trace` suffix names the GPU. Nothing here
is a reference that `tests/` or `ref_cases.py` reads. They are the sample of
`docs/notes/mali-g310-vs-g610.md`, the four frame differences that the
G310's smaller tile buffer makes (G310v1-v3; v4 and v5 draw the G610's
bytes).

| File | Drawn with |
| --- | --- |
| `mrt3.g610.trace` | `tools/mali/drm_shim_ref.sh run <work> tools/mali/testdata/tri.vert tools/mali/testdata/tri.frag mrt3.g610 --mrt 3` |
| `mrt3.g310v2.trace` | the same with `--gpu g310v2` |
| `ms4.g610.trace` | the same as `mrt3.g610` with `--samples 4` in place of `--mrt 3` |
| `ms4.g310v2.trace` | the same with `--gpu g310v2` |

`<work>` is a `drm_shim_ref.sh build` of the 26.2.1 tarball. A redraw gives
the same bytes except the `Shader 0x... (GPU VA ...)` host pointers, which
differ from run to run. Compare them with `diff` and ignore those lines:

- `mrt3`: `Effective Tile Size` 1024 against 512, `Color Buffer Allocation`
  12288 against 6144, and the second and third render targets' `Internal
  Buffer Offset` 4096 and 8192 against 2048 and 4096.
- `ms4`: tile 512 against 256 (the Z tile buffer bound), allocation 8192
  against 4096, and the tiler context's `Hierarchy Mask` 254 against 255.
