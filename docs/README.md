<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Documentation

What each place holds. Every file here describes the current state or a
design the code follows; there are no run logs or session diaries.

| Where | What |
| --- | --- |
| `es2-cts-failures.md`, `es3-cts-failures.md` | dEQP-GLES2 and dEQP-GLES3: what is accepted, what fails now and why. |
| `es3/` | ES 3.0 designs per feature, the board recipes, and the current per-case failure lists (`*.tsv`). |
| `online-shaderc/` | The on-board shader compiler: front end, IR, back ends, ISAs and ABIs. |
| `notes/` | Designs and findings that the code relies on (comments cite them): the PowerVR render pool and fence wait, the GC7000 instruments, board bring-up and netboot, and `notes/am62p-flip-latch.md` (AM62P: a flip that never scans the buffer being drawn; four buffers and `display_busy`), and `notes/mali-uniform-index.md` (Mali Valhall: Mesa's design for a uniform read at a run-time index, `LD_PKA` from UBO 0 beside the FAU push, #1385), `notes/mali-cube-textures.md` (Mali Valhall: Mesa's design for cube-map textures, case 113, #1385), and `notes/mali-highp-fragcoord.md` (Mali Valhall: Mesa's design for highp fp32 fragment arithmetic and `gl_FragCoord`, S2 and S4, #1384), `notes/mali-discard.md` (Mali Valhall: Mesa's design for `discard`, the draw flags, early-z and forward pixel kill on the frame side, `DISCARD` and the helper threads on the compiler side, S6, #1384), `notes/mali-lowering-map.md` (Mali Valhall: the transcription map of Mesa's compiler passes for S2-S6 and the femtovg per-program needs, #1384), `notes/mali-frame-refusals.md` (Mali Valhall: Mesa's design for the frame-side refusals, occlusion queries, alpha-to-coverage, blend shaders, 4x, #1386), `notes/mali-texture-refusals.md` (Mali Valhall: Mesa's design for the texture refusals, vertex-shader look-ups, a coordinate form, the depth texture, #1385) `notes/mali-varying-constants.md` (Mali Valhall: Mesa's link of varyings computed from no attribute and the gl_FragColor lane shapes, cases 63, 75, 78, 101, 117, 181, 185, 191, 199, #1384) and `notes/mali-uniform-types.md` (Mali Valhall: Mesa's words for an `int` uniform read at a run-time index and for uniforms past the FAU, cases 100, 116, 166, #1385). `notes/mali-texture-lookups.md` (Mali Valhall: Mesa's design for a second look-up, `textureSize`, 3D and 2D-array look-ups, cases 2, 46, 48, 206, #1385). `notes/mali-loop-break.md` (Mali Valhall: Mesa's `complex_unroll` of a loop with a run-time break, case 87, the whole loop becomes selects, no branch, drm-shim reference `case87_databreak.*`). `notes/mali-blend-shaders.md` (Mali Valhall: Mesa's blend shaders for the 4x constant blend and the blend equations without fixed function, cases 211, 190, #1384 S6, #1385). `notes/mali-builtin-inputs.md` (Mali Valhall: Mesa's lowering of the built-in inputs `gl_VertexID`, `gl_InstanceID`, `gl_FrontFacing`, `gl_PointCoord`, #1384 S6). `notes/mali-alu-ops.md` (Mali Valhall: the ops behind the 16 `arithmetic` refusals and Mesa's selection for each, #1384). `notes/mali-int-and-size.md` (Mali: integer and boolean values and shaders over the 256-value cap, cases 42, 72, 79, 112, 184, 193, 194, 195). `notes/mali-frame-formats.md` (Mali frame refusals: face culling, per-instance attributes and RGBA16F targets, cases 4, 52, 54). `notes/mali-precision.md` (Valhall precision: fp16 vertex uniforms and mediump `gl_FragCoord`, cases 19, 82, 84, 119, #1384). `notes/mali-fragment-outputs.md` (Mali: several fragment outputs and the fragment operand shapes of cases 55, 91, 192, #1384). `notes/mali-uniform-arrays.md` (Mali Valhall: Mesa's design for a float uniform array read at a run-time index, case 108, #1385). `notes/mali-vs-texture-lanes.md` (Mali Valhall: Mesa's words for a vertex-stage look-up at a coordinate with a constant lane, case 149, #1385). `notes/mali-algebraic-folds.md` (Mali Valhall: Mesa's algebraic folds of constant products, cases 189, 192). `notes/mali-rebalance.md` (Mali Valhall: Mesa's `do_rebalance_tree` on chains of three or more additions or multiplications). `notes/mali-mixed-precision.md` (Mali Valhall: Mesa's mixed highp/mediump fragment code, non-fp16-exact colour constants and non-finite folds). `notes/mali-texel-fetch.md` (Mali Valhall: Mesa's `texelFetch`, `textureOffset`, integer samplers and array-layer look-ups, and the descriptors the frame needs). `notes/mali-int-loops.md` (Mali Valhall: Mesa's integer loop counters, int comparisons and loops that end at a run-time condition). `notes/mali-int-uniforms.md` (Mali Valhall: Mesa's lowering of `int`, `uint` and `bool` uniforms). `notes/mali-fragcoord-mediump.md` (Mali Valhall: Mesa's lowering of mediump `gl_FragCoord` in the internal blit and mipmap shaders and three gltest cases). `notes/gl-draw-fixed-cost.md` (the fixed cost of a glDrawElements by step and back end, glmark2 `ideas`, and the ranked fixes). `notes/am62p-pvr-mipmap-kicks.md` (AM62P: the design for fewer kicks in the PowerVR `glGenerateMipmap`, GPU for the large levels and a CPU tail with the GPU's filter, transfer queue as the fallback). `notes/pvr-3d-native.md` (PowerVR: 3D textures as the TPU's own TEXTYPE_3D behind `pvr_native_3d`, the choice over grid mip levels and wrap, the Mesa layout, what a board must check). `notes/pvr-draw-pds.md` (PowerVR: the kick-USC and coefficient PDS programs generated once per frame and fragment program, fix F4 of the fixed cost, with its host before/after) and `notes/pvr-fw-window.md` (PowerVR: the firmware's cached 0xC and uncached 0x4 windows onto its 32 MiB heap, what the firmware image shows about its own cache handling, and the mechanism of the cached-window defect on the AM62P). `notes/blit-depth-window.md` (glBlitFramebuffer depth from the window and from cube or array depth textures: what the code says, what a board must measure first). `notes/pvr-prefilter-stall.md` (PowerVR: the online Qt prefilter program against pco's, a NaN level sampled in a flattened arm and 250 temporaries, ranked, with the board tests that separate them, #1635). `notes/pvr-test-knobs.md` (PowerVR: design clean-up 3, the switches hard-coded at their measured defaults, the `pvr_debug_*` test knobs and the TEST KNOB options behind `test-knobs`, the retired option slots, and the namespace contract the gate's "no test knob in the artefact" row reads). `notes/pvr-depth-sample.md` (PowerVR: a rendered depth texture sampled by the TPU from the ISP's D24S8 buffer as X8U24, Mesa's design, no CPU resolve: glmark2 shadow on the AM62P, #40). `notes/am62p-direct-boot.md` and `notes/imx8mp-direct-boot.md` (power-on into Zephyr with no U-Boot proper and no Linux: the boot chains, the second-stage loader `driver/samples/zephyr/boot_stage2` that carries the compressed image, the board procedures and recovery). |
| `release/` | Release stage: building guide, checklist, licence and redaction audits. |
| `ci.md`, `source-form.md`, `standalone-boot.md`, `v3d-noseamless-cube.md` | The CI gate, the source-form rule, standalone boot, one V3D design. |

Elsewhere: board results are `tools/cts/RESULTS.md` (current results only)
and its Japanese summary `tools/cts/STATUS.md`; the stages are `ROADMAP.md`;
the driver's own documents are under `driver/docs/`; component notes sit next
to their code (`src/*_NOTES.md`, `src/*/*_NOTES.md`, `tools/*/README.md`).

## Where a new finding goes

- A board run: its row in `tools/cts/RESULTS.md` (replacing the old one), the
  summary in `tools/cts/STATUS.md`, and a comment on its GitHub issue. No
  session file.
- A design, or a finding the code depends on: `notes/` (or next to the code),
  cited from the code.
- Open work and known defects: GitHub issues, never a file here.
- Anything else (how a run went, what was tried): the commit message.

## Removed records

Board-session records, investigations and diaries that only recorded past
runs were removed on 2026-10-16 (`docs/board-sessions/`,
`docs/investigations/`, `tools/cts/log/`, and the old measurement log in
`tools/cts/RESULTS.md`). Comments that still name one write it as
`ab43a20c5:<path>`. The Mali board captures (`tools/mali/board-ref/`) and
the last two session records were removed on 2026-10-26; comments name
them as `0dce654a8:<path>`. The excerpts the tests read are kept in
`tools/mali/board-dump/`.

These hashes, and every commit hash the docs, records and code comments
cite from before the first public commit, name commits of the non-public
working repository the project was developed in. They cannot be read in
the public repository (CONTRIBUTING.md, "Branches and commits").
