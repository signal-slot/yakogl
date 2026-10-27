<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: the debug switches and the test knobs (design clean-up 3)

The PowerVR backend has no debug or experiment switch in product code since
design clean-up 3 (2026-10-26), the PowerVR's turn after the GC7000's
clean-ups 1 and 2 (`src/vivante/VIVANTE_NOTES.md`, "The debug switches and
the test knobs"). This note records what became the design, what stayed as a
test knob, what was retired, and the contract the gate reads.

## The contract: a knob lives in its GPU's debug namespace

A test knob is a C symbol the on-silicon suites set (gl_test.c on each
board, the dEQP sample), defined only with the crate's `test-knobs` feature
(`CONFIG_YAKOGL_TEST_KNOBS`). Its name is in its GPU's debug namespace:
`gc_debug_*` (and the `gc_perf_*` / `gc_kick_diff_*` instruments) on the
GC7000, `pvr_debug_*` on the PowerVR. The gate's row "no test knob in the
artefact" (`tools/check.sh` `check_knobs_absent`) reads each GPU's product
archive (`yakogl-capi` with the Zephyr image's features, the in-process
compiler and no table) and requires that it defines no symbol of the
namespace, and that the same archive with `test-knobs` defines exactly the
listed knobs (the control: a pattern that stopped matching would find
neither).

The namespace is the whole contract. A product switch outside it -- the
in-process compiler's `pvr_native_cube`, `pvr_native_array`, `pvr_native_3d`,
`pvr_proj_divide`, `pvr_proj_lod` and their setters (`src/shaderc/mod.rs`),
which go with the compiler after the shader-compiler rename -- is not the
row's business, and the compiler may gain one without touching the gate.
The earlier form of this clean-up listed every `pvr_*` symbol the product
archive may define, and main's `pvr_native_3d` broke it the day it merged;
the namespace removes that coupling.

## The PowerVR's test knobs

Two switches, in `src/pvr/backend.rs`, with their setters:

| knob | default | the other arm | measured by |
|---|---|---|---|
| `pvr_debug_split_zs` / `pvr_debug_set_split_zs(bool) -> bool` | 1: a framebuffer's depth and stencil are kept apart when they are two renderbuffers (the stencil through `CR_ISP_STENCIL_LOAD_BASE`) | 0: one packed D24S8 buffer per pair, the AM62P's 14 `fbo.render` failures (a60104c2) | gl_test 123 `zsapart`, both arms |
| `pvr_debug_narrow_la` / `pvr_debug_set_narrow_la(int) -> int` | 1: L8 / A8 / LA8 stored one or two bytes a texel, pack info 0x140 | 0: expanded into RGBA8; 2: narrow with pack info 0x11f | gl_test 132 `narrowla`, all three arms |

Eight backend options (`enum pvr_backend_option`, `src/pvr/pvr_backend.h`,
marked TEST KNOB) take effect only with the feature: `PRETRANSFORM`,
`DEPTH_CLIP`, `PPP_CTRL`, `NO_COALESCE`, `DEBUG`, `DS_DUMP`, `PRIM_BUDGET`
and `PBE_TWIDDLE`. `pvr_backend_set_option` refuses them without it (one log
line, nothing stored), so `Backend::opt` reads 0 and the code behind each
folds away in a product build; the host test
`a_test_knob_option_is_inert_without_the_feature` pins that. gl_test.c calls
every knob under `GL_TEST_KNOBS` (`CONFIG_YAKOGL_TEST_KNOBS` on Zephyr,
`YAKOGL_TEST_KNOBS` on a host or CMake build); without it a case runs the
library's default arm and says so.

Product options, not knobs: `ONE_IN_FLIGHT` (glmark2's fence-wait
measurement, `docs/notes/pvr-fence-wait.md`), `IF_LOWERING` (the compiler's
if-lowering choice) and `FS_TEMPS_MIN` (the online-vs-table temporaries A/B,
a product option until the shader-compiler rename). The measurement
infrastructure (`pvr_render_perf_read`, the workload counters, the
depth/stencil report) stays.

## What used to be a switch is the design now

Five switches, each hard-coded at the default the AM62P measured:

- *The masked colour clear* (`pvr_debug_masked_clear`, on): a colour clear
  writes only the channels `glColorMask` had on at the glClear, by drawing
  the clear with the masked clear program for the mask, and a fragment
  shader that reads the pixel runs TRANSLUCENT (dEQP color_clear 19/19,
  gl_test 102, 103, 107; Zephyr f9f4896f). Every backend applies the mask
  now, so the GL core's `masked_clear_applied` query and its "every channel
  is cleared" log line went too.
- *The point sprite on a y-up target* (`pvr_sprite_yup`, SPRITE_10UV): a
  point on a target stored y-up is drawn as `PVR_TA_OBJTYPE_SPRITE_10UV`, so
  `gl_PointCoord.t` starts at the window's top there too (dEQP
  builtin_variable.pointcoord passes with it and fails with SPRITE_UV and
  SPRITE_01UV; AM62P Linux, 27ebd89a).
- *The multi-target clear* (`pvr_mrt_clear`, on): a clear of several 8-bit
  attachments uploads the colour once per attachment and runs the n-dword
  clear program (gl_test 106 mask 0 with it, 6 without; cece5c23).
- *The integer depth bias* (`pvr_dbias_int`, on): `ISP_CTL.dbias_is_int` is
  set in every frame with a depth/stencil buffer (dEQP polygon_offset 14/0
  with it, 10/4 without; RESULTS 1i, a53c0122 / f963dee5).
- *The swap drops the window's depth/stencil* (`pvr_swap_drops_zs`, on):
  `pvr_backend_surface_swapped` always marks the swapped frame's kick to
  store none (EGL 1.5 3.10.1; glmark2 315 -> 550, gl_test 37/38 both ways).
  The opt-outs `CONFIG_GLMARK2_PVR_KEEP_ZS` and `CONFIG_GL_TEST_PVR_KEEP_ZS`
  went with it.

## Retired options

Five option slots keep their numbers so every later option keeps its own
(`src/pvr/ffi_layout_check.c` pins them); setting one changes nothing:
`RETIRED_TERM0` (5, a raw TA_STATE_TERMINATE0 override nothing set),
`RETIRED_MRT_STRIDE` (9, a bring-up override of the pixel output width),
`RETIRED_FORCE_MIP0` (10, every texture sampled from level 0),
`RETIRED_DBIAS_INT` (11) and `RETIRED_SWAP_DROPS_ZS` (14), the two whose
measured default is the design above. `RETIRED_NO_SUBMIT` (6) was retired
earlier, with pvrgpu.ko.

The Kconfig switches that set the retired ones from an image went with them:
`DEQP_PVR_MASKED_CLEAR`, `DEQP_PVR_DBIAS_INT`, `GL_TEST_PVR_MASKED_CLEAR`,
`GL_TEST_PVR_DBIAS_INT`, `GL_TEST_PVR_KEEP_ZS`, `GLMARK2_PVR_KEEP_ZS`. The
AM62P gl_test image's default set is unchanged: every switch it set is now
the hard-coded default, and the knobs it still sets (case 62's clip sweep,
123, 132, the split cases' primitive budget) are in its library through
`CONFIG_GL_TEST_KNOBS`.

## How the gate checks it

- `clippy` and `test` rows for `host-test,gpu-powervr,shaderc-online,test-knobs`
  (`KNOBS_POWERVR_CONFIG`), the knob-driven tests named so that one that
  stops being compiled fails the row.
- `cc gl_test.c -DGL_TEST_HOST=1 -DYAKOGL_TEST_KNOBS=1`, the gl_test image's
  arm, beside the arm without the knobs.
- "no test knob in the artefact", for the PowerVR as for the GC7000
  (`KNOB_POWERVR_SYMBOLS`).
