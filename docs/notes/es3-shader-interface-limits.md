<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 shader-interface and uniform limits below the minimum (plan #359)

What each limit can honestly be per GPU, and what must exist before the
number moves. Read-only design; no reported value changes with this note.
`tests/es3_caps.rs` `known_gaps()` stays true.

## The query forms are not a second defect

`docs/notes/v3d-notpass-rest.md` G3 suspected the `*_getfloat` and
`*_getinteger64` rows of a defect of their own. `glGetFloatv` and
`glGetInteger64v` (`src/gl/state.rs`) answer a limit from the same
`get_integerv` arm as `glGetIntegerv` and only widen it (`get_floatv` holds
float state and returns 0 for a limit). The rows that fail are the limits
whose integer query is below its minimum, and nothing else.
`tests/es3_caps.rs` now asserts the float and the 64-bit answer equal the
integer one for every limit, on the three GPUs. Closing the integer limits
closes these rows.

## Varyings (MAX_VARYING_VECTORS 15, components 60/60, output 64)

* **V3D** (reports 8/32/32/32). The compiler already builds a 15-vec4
  fragment shader (es100/varyings15, 60 components, `chain.rs`). The record
  side does not carry it: `V3dFsProg.flat_shade_flags`, `noperspective_flags`
  and `centroid_flags` are one `u32` each (`src/v3d/ffi.rs`,
  `include/gles_shaderc.h:254`), i.e. 32 inputs, where Mesa keeps
  `V3D_MAX_FS_INPUTS / 24` words (`v3dx_emit.c` `emit_varying_flags`, 24 flags
  per word; `V3D_MAX_FS_INPUTS` 64). 60 components plus the two implicit
  point/line varyings need three words. The change is therefore: the three
  flags become arrays in `ShcV3dFs` and `V3dFsProg` (C header, `v3d_shc.c`,
  `tools/v3d/*` fixture generators, `tests/v3d_sim_mesa.rs`, regenerated
  `tests/fixtures`), the job builder emits one flags packet per word, and
  `sim.rs` follows the 64-input limit. 64 vertex output components with 15
  vectors is a separate step: it needs a sixteenth VPM output row that holds
  no varying (the output limit is `MAX_VARYING_VECTORS * 4` in `context.rs`
  today, so it needs its own constant).
  Not measurable on the host: the draw. The host shows the record words and
  the interpreter result; a board run of es100/varyings15 and a 15-vec4 draw
  is the evidence for the number.
* **Mali**: stays 8 (stage-5 plans own it).
* **PowerVR / GC7000UL**: 15 vectors and 60 components are carried. 64 output
  components need the 16th output row to exist beside the position. PowerVR:
  the VDM `vs_output_size` field holds 64 dwords (`tests/es3_pvr_varyings.rs`
  header), so 64 is reachable in the record; the USC output layout is
  `src/pvr/**` (not released to this plan). GC7000: `SHC_VIV_MAX_IO` 16 rows
  carry position beside 15 varyings, so a 16th varying has no row: 64 is not
  reachable without the output-row work (`src/vivante/**`). Both are issue
  drafts.

## Uniform blocks (MAX_UNIFORM_BLOCK_SIZE 16384, combined 50176/50048)

The core flattens a block into the stage's default uniform block and fills it
at draw time (`src/gl/program.rs`; `MAX_UNIFORM_DWORDS` 1536 dwords, 6144
bytes), so the largest block is the whole default block.

* **V3D**: the uniform stream (`ldunif`) is where a block lives today. A
  16384-byte block is 4096 dwords: more than the stream may hold, and a
  run-time index needs `ldunifa`, which the compiler lacks (the ignored tests
  in `tests/es3_ubo_limit_reachable.rs`). Mesa reads UBOs with TMU loads
  (`nir_to_vir` `ldunifa` for uniform-offset reads and general TMU for
  dynamic ones). Needed: a block-by-reference record (address, size) in the
  draw, the compiler's UBO load through `ldunifa`/TMU, the GL core handing
  the buffer's GPU address instead of copying it, and `reflect.rs` `fit_u8`
  lifted for arrays over 255 entries.
* **PowerVR**: blocks are in the uniform stream (1536 dwords). Needed: the
  same by-reference path (USC `ld` from memory through a PDS constant
  holding the address). `src/pvr/**`.
* **GC7000UL**: reads user blocks from memory with `load`
  (`docs/notes/gc-ubo-const-limit.md`), so the hardware path exists and the
  limit is the core's flattening. `src/vivante/**`.
* **Combined components** (ES 3.0 6.2): `blocks * MAX_UNIFORM_BLOCK_SIZE / 4 +
  MAX_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` = 12 * 4096 + 1024 = 50176
  (vertex) and 12 * 4096 + 896 = 50048 (fragment). It follows the block size
  and is the last number to move; `known_gaps()` names the block size as the
  root. `tests/es3_ubo_limit_reachable.rs` (`combined_components_follow_the_es3_formula`,
  ignored until the raise) pins the formula.

Until the by-reference path exists the honest values are the ones reported;
raising the number without it would be a limit the stack cannot link.

## Tests that pin this

* `tests/es3_caps.rs`: minimums, `known_gaps()`, the three query forms agree.
* `tests/es3_ubo_limit_reachable.rs` (V3D, PowerVR, GC7000UL; not Mali): a
  block of exactly the reported size links and reads back, one more vec4 does
  not link; five checks are ignored with their reasons and un-ignore with the
  raise.
