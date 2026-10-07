<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Benchmarks: what each one needs, per board

This is plan #2's inventory (the benchmarks run correctly on AM62P, i.MX8MP and
Pi 4). It lists, for glmark2, Slint and the AGL cluster sample, what each needs
from the driver, what the driver gives today, the latest board result and the
open gaps. It is an inventory, not a status report: HANDOFF.md owns the
current state.

Sources. `tools/cts/RESULTS.md` is not in date order, so every number below
names its section (quoted in `"..."`) or its record directory. `tools/cts/STATUS.md`
(Japanese) summarises the same runs. Line numbers are of this tree.

## What the driver provides (shared by all three benchmarks)

| item | where |
|---|---|
| Extension list, every name implemented: `GL_OES_EGL_image`, `GL_OES_standard_derivatives` (:49), the ETC1/ETC2 compressed formats, `GL_OES_vertex_array_object`, `GL_OES_mapbuffer` (:57), `GL_OES_fbo_render_mipmap`, `GL_EXT_texture_format_BGRA8888`, `GL_OES_rgb8_rgba8`, `GL_OES_required_internalformat`, `GL_EXT_texture_rg`, `GL_OES_texture_half_float` and `GL_EXT_color_buffer_half_float` (:108-109), `GL_EXT_debug_marker`; plus `GL_OES_depth_texture` and `GL_OES_packed_depth_stencil` (:161-162, claimed on the GPUs that have them, see :166) | `src/gl/state.rs:40-173` |
| **Not** in the list: `GL_OES_texture_npot`, `GL_OES_texture_float`, `GL_EXT_draw_buffers`, `GL_EXT_sRGB` | `src/gl/state.rs:102-109`, `:1094`, test at `:1413` |
| `REPORT_ES2` (the default of C and Zephyr builds): GL_VERSION is ES 2.0, and a non-power-of-two (NPOT) texture has no mip chain and samples as incomplete unless it is clamped to the edge on both axes and filtered without mipmaps | `src/lib.rs:64-81` |
| NPOT rule in the sampler state: incomplete when `REPEAT`/`MIRRORED_REPEAT` or a mipmap filter is used | `src/gl/draw.rs:2281-2351` |
| NPOT rule in the texture store: a level-0 image that is not a power of two gets one level; `glGenerateMipmap` on it is `GL_INVALID_OPERATION` (0x502) | `src/gl/texture.rs:3096` (also `:1888`, `:3214`, tests `:7598`, `:10566`) |

## glmark2

Sample `driver/samples/zephyr/glmark2` (default benchmark set, 33 entries, 800x600
on screen). Build needs `GLMARK2_SRC`, a glmark2 checkout pinned at
`22c527cb0556f3a1ac4445aaa52cc532760928d5` (`CMakeLists.txt:28`; the build
refuses another HEAD or local changes, `:39-51`). Shaders are built at run time
from option-substituted sources, so the image uses the in-process compiler
(`CONFIG_YAKOGL_SHADERC_ONLINE=y`, `prj.conf`).

Needs, from the records: the 800x600 window as the board's buffer
(`main.cpp:176`); no MSAA; a depth/stencil window config, which the PowerVR swap
drops by default (`main.cpp:667`, RESULTS "The swap drops the window's
depth/stencil by default"); `GL_OES_mapbuffer` for the two `buffer`
`update-method=map` entries 17 and 19; a depth-only framebuffer and a depth
texture for `shadow` (entry 23) and `refract` (entry 24; RESULTS "Pi 4 common
set round 3: the glmark2 cell is green"); two 1600x1200 textures (a depth and an
RGBA one, 7,680,000 bytes each) for `refract`; 800x600 textures with a mip chain
for `terrain` (22).

| board | driver support today | latest result | open gaps |
|---|---|---|---|
| AM62P (PowerVR) | mapbuffer `state.rs:57`; depth texture `state.rs:161-162`; z-only render (RESULTS "AM62P: a framebuffer with no colour attachment, the PowerVR z-only render, gl_test case 177"); reclaim of held frees on by default (RESULTS "AM62P: the reclaim default, common round") | **457, 33 ran / 0 failed / 0 unsupported, resets 0**, record `594b1201 fc3f01c9 2184003f 03200258`, image `edcf0604`, tree `8e7257f8`; run `build/common-am62p-20261004-145517/`, section "AM62P: the reclaim default, common round (8e7257f8 ...) — glmark2 cell AMBER" (RESULTS.md:766). Amber because of refract, below | **refract**: its 7600 KiB depth buffer was refused in 110 of 110 frames (largest free range 3936 KiB of about 11.6 MiB; fragmentation). Host fix `541c9d3b` (large blocks from the carve-out's end) is on the integration branch, host-tested only; **no board run of it yet**. **terrain** samples black (NPOT, below). Shaders: no host compile check |
| i.MX8MP (GC7000) | mapbuffer; depth-only framebuffer via a dummy colour surface (STATUS.md, 10/4 "shadow" entry); no depth-texture limit seen | **37, 33 / 0 / 0, status pass**, record `594b120e fc3f0025 2184003f 03200258`, image `e3dc9ce0782f8699...`, tree `85c8d206`; section "i.MX8MP common set round 4: the glmark2 cell is green" (RESULTS.md:490), run `build/common-imx8mp-20261004-134652/`. Score is low (about 24 ms a frame floor, STATUS.md) | **terrain** black (NPOT). Score 37 is far below the other boards: not a "correctness" gap, but unexplained |
| Pi 4 (V3D 4.2) | mapbuffer; depth texture sampled at the TMU's 32-bit output with a CPU read-back (RESULTS "Pi 4: depth textures on the V3D, gl_test case 169"); depth-only framebuffer (case 177) | **183, 33 / 0 / 0, status pass**, record `594b1201 fc3f00b7 2184003f 03200258`, image `a88a2ff489799858...`, tree `1fa941e3`; section "Pi 4 common set round 3: the glmark2 cell is green" (RESULTS.md:464), run `build/common-pi4-20261004-145117/`; confirmed again at `e351cd83`, "Pi 4: the OSAL looks at the job in flight ... gl_test case 180" | **refract and terrain** overflowed the 48 MiB carve-out (refract peak about 56.2 MB; 39 refusals of 7,684,096 B, 399 for terrain: RESULTS.md:464-488, 849-854). The carve-out was raised to 96 MiB in `boards/rpi_4b.conf` (`ca4074a0`); **no board run of it yet**. **terrain** black (NPOT) |

### glmark2 gap: terrain is black on every board

RESULTS "Pi 4: the OSAL looks at the job in flight ... gl_test case 180" (end of
section): terrain logs `GL error 0x502 in glGenerateMipmap` 16 times and
`a texture on unit 0 is incomplete and samples (0,0,0,1) ... 800x600, levels 1 of 1`;
"the same in the AM62P's land192 round". The cause is in the code: the 800x600
texture is NPOT, a `REPORT_ES2` build refuses its mip chain
(`src/gl/texture.rs:3096`) and the sampler marks it incomplete
(`src/gl/draw.rs:2281-2351`); `GL_OES_texture_npot` would lift both and is not in
`state.rs:40-173` (`src/lib.rs:79-81` says so). The i.MX8MP record has the same
cause; its console was not read for the 0x502 line (no UART, SNVS record only).
Every glmark2 cell is therefore "green by record, terrain's picture wrong".

### glmark2 gap: no host check of its shaders

glmark2 builds its shaders at run time, so none of them is in `tools/shaders/`,
the corpus of `tests/online_corpus.rs`. That test is `cfg`'d to
`gpu-powervr` or `gpu-vivante` (`tests/online_corpus.rs:16`), so the V3D has no
corpus compile at all, glmark2 or not. `tools/precompile` (the precompiled tier)
likewise covers the corpus, not glmark2.

## Slint

Sample `driver/samples/zephyr/slint` (the FemtoVG renderer through
glow on GL ES 2.0). Build needs `SLINT_NOSTD_DIR` (the no_std Slint workspace)
and `NOSTD_PATCH_DIR` (the femtovg/glow forks), both recreated from
`nostd/README.md` (`CMakeLists.txt:13-46`), and a Rust nightly with rust-src
(RESULTS "AM62P: Slint and cluster after the join removal": nightly 1.101.0,
2026-10-04). The window is 1024x600 (the Pi 4 view is 1024x608, pitch 5120:
RESULTS "The common set from the bootloader ..."). Needs: FemtoVG's shaders
(the AM62P offline table missed six NO_DISCARD sources, RESULTS "Cluster
acceptance run", last paragraph; precompiled since "AM62P: the precompiled tier
in front of the compiler"), `GL_OES_standard_derivatives` (`state.rs:49`),
`GL_OES_vertex_array_object`, BGRA textures. No depth/stencil, no MSAA, no
mapbuffer. The sample passes when the PROBES record (5 colour points) says pass.

| board | driver support today | latest result | open gaps |
|---|---|---|---|
| AM62P | as above; the precompiled tier (`CONFIG_YAKOGL_PRECOMPILED_SHADERS`) | PROBES pass `594b0601 08420862 00210842 c5920883`, 703 frames in 10002 ms, resets 0, record `20261006-1253-slint-precompiled-17605c36-81207642` (RESULTS "AM62P: Slint and the cluster from precompiled shaders", `17605c36`); before: `20261005-2251-slint-7dada1ff-82cb9e17`. The board layer has no scan-out view, so the view row is the copy path again | none known |
| i.MX8MP | as above | PROBES pass `594b0601 08420882 00210842 98080883`, record = SNVS word, tree `85c8d206`, "i.MX8MP common set round 4: the glmark2 cell is green" (RESULTS.md:490-501), run `build/common-imx8mp-20261004-134652/` | frames/s and the picture are not in the record (no UART) |
| Pi 4 | as above; scan-out view | PROBES pass `594b0601 08420862 00210842 c5920883`, copy 659 frames in 10001 ms and view 1523 in 10003 ms ("Pi 4 common set round 2" in "The common set from the bootloader ..."); round 3 same words, tree `1fa941e3` ("Pi 4 common set round 3 ...") | none known |

## AGL cluster

Sample `driver/samples/zephyr/cluster` (`agl-slint-cluster`). Build needs
`AGL_SLINT_CLUSTER_DIR`, `SLINT_NOSTD_DIR`, `NOSTD_PATCH_DIR` and `NOTO_SANS_DIR`
(Noto Sans Regular, Light and ExtraLight; `CMakeLists.txt:12-32`) and a nightly
toolchain. The window is the panel's own size, set in
`boards/<board>.conf` and declared in `cluster/Kconfig`
(`CLUSTER_PANEL_WIDTH` / `CLUSTER_PANEL_HEIGHT`, default 0 = does not build):
1024x600 on AM62P, 1280x800 on i.MX8MP, 1920x720 on Pi 4. The check needs a host
reference per size (`check/src/references.rs`). Needs: the same GL features as
Slint; larger windows and more shaders (583 precompiled rows on AM62P).

| board | driver support today | latest result | open gaps |
|---|---|---|---|
| AM62P | as Slint | PROBES pass `594b0601 008510c4 19050085 110c6d5f`, 5509 frames in 60014 ms, 91.8 fps, 56.4 Mpx/s, resets 0, record `20261006-1256-cluster-precompiled-17605c36-771f77fb` (RESULTS "AM62P: Slint and the cluster from precompiled shaders"); first frame about 10.4 s before the precompiled tier ("Cluster acceptance run") | none known |
| i.MX8MP | as Slint | PROBES pass `594b0601 10c410c4 1905f7bf b1076d5f` at 1280x800, tree `031de9a2`, run `build/clurec-imx8mp-20261004-053011/cluster/`; also `build/clu-imx8mp-20261004-015558/` (RESULTS "Cluster acceptance run, one sample on three boards (2026-10-03)") | fps, smoothness and the no-copy proof (APP record bit 16) not readable: no UART and one SNVS record |
| Pi 4 | as Slint; scan-out view | PROBES pass `594b0601 00a710c4 10c40086 7b932967`, 34/34 probes, 8395 frames in 60019 ms (139.8 fps), copy-free (bit 16 clear), tree `031de9a2`, run `build/clurec-pi4-20261004-043840/20261004-0438-cluster-11236a24/` (same section) | none known |

Gap: the cluster joined the common set (`tools/cts/common-tree.txt`, image CL,
commit `c7c80ba1`) only on 2026-10-07; no common-set round with the CL row has run
on any board, so its board results above are from its own acceptance runs. The
earlier 4-image rounds do not contain it.

## Open gaps, in one list

| # | gap | boards | evidence | state |
|---|---|---|---|---|
| 1 | terrain samples a black texture: `glGenerateMipmap` 0x502 on an 800x600 NPOT texture in `REPORT_ES2` builds; `GL_OES_texture_npot` missing | all three | `src/lib.rs:79-81`, `src/gl/texture.rs:3096`, `src/gl/draw.rs:2281`, RESULTS "Pi 4: the OSAL looks at the job in flight ... gl_test case 180" | open |
| 2 | refract's depth-pass draw refused every frame (pool fragmentation, 7600 KiB against 3936 KiB) | AM62P | RESULTS.md:766-788 | host fix `541c9d3b` on integration, not board-verified |
| 3 | refract and terrain exceed the V3D carve-out | Pi 4 | RESULTS.md:464-488, 849-854 | carve-out 96 MiB in `boards/rpi_4b.conf` (`ca4074a0`), not board-verified |
| 4 | the cluster was not in the common set | all | `tools/cts/common-tree.txt` | in the file since `c7c80ba1`; no round run |
| 5 | glmark2 shaders in no host compile check; V3D has no corpus compile | all / Pi 4 | `tests/online_corpus.rs:16` | open |
| 6 | i.MX8MP records carry no fps, GPU-reset count or no-copy bit (no UART) | i.MX8MP | RESULTS "Cluster acceptance run" | by design of the board |
