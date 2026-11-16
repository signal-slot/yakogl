<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 shader-interface limits: varyings and vertex outputs, per GPU

Design, no code (plan #359). Line numbers are at the tip this note was written
on. `[M]` = measured by reading, `[U]` = unknown. The uniform-block and combined
uniform limits are in `docs/notes/ubo-limits.md`, `docs/notes/v3d-ubo-path.md`
and `docs/notes/gc-ubo-const-limit.md`; this note covers the rest of the
limits that sit below the ES 3.0 minimum.

## 1. What is reported and what ES 3.0 asks

`MAX_VARYING_VECTORS` is a per-GPU constant (`src/gl/context.rs:149-159` for
the V3D and Mali first cut, `src/gl/context.rs:125-146` for the PowerVR, 15 outside `report-es2`).
The three component limits are derived from it (`src/gl/context.rs:193-194`);
`MAX_VARYING_COMPONENTS` is read from the same constant in
`src/gl/state.rs:1254`.

| query | V3D, Mali | PowerVR | GC7000UL | ES 3.0 |
|---|---|---|---|---|
| `MAX_VARYING_VECTORS` | 8 | 15 | 15 (`src/gl/context.rs:119-122`) | 15 |
| `MAX_VARYING_COMPONENTS` | 32 | 60 | 60 | 60 |
| `MAX_FRAGMENT_INPUT_COMPONENTS` | 32 | 60 | 60 | 60 |
| `MAX_VERTEX_OUTPUT_COMPONENTS` | 32 | 60 | 60 | 64 |

ES 3.0 6.2 gives `MAX_VERTEX_OUTPUT_COMPONENTS` 64 and 15 varying vectors:
the vertex output limit is four components above the varying limit because
`gl_Position` takes a vector that is not a varying. Both stay derived from one
constant today, so raising the vertex output to 64 needs its own constant,
`MAX_VARYING_VECTORS * 4 + 4`, on every GPU that reports 15 (item c of the plan).

## 2. V3D: what 15 varyings and 64 outputs need

Mesa's v3d allows 64 input components a stage
(`third_party/mesa-26.2.1/src/broadcom/common/v3d_limits.h:32-34`, `V3D_MAX_FS_INPUTS`
and `V3D_MAX_VS_INPUTS`), so 15 vectors (60 components) and a 64-component
vertex output are inside the hardware's and Mesa's own limits [M].

- **Compiler.** The V3D compiler already builds a 60-component fragment shader:
  `src/shaderc/online/v3d/chain.rs:1698` pins es100/varyings15's fragment
  shader. The vertex side sizes the VPM output from the selected rows:
  `src/shaderc/online/v3d/chain.rs:335` (`vpm_output_rows` in units of 8, at
  least the input size) and the fields it fills at
  `src/shaderc/online/v3d/chain.rs:351-360`. A 64-component output is 16
  rows plus `gl_Position`, which the same formula carries [U: no compile test
  at 16 output vectors exists yet; the new test in item 2 pins it].
- **Shader record.** The fragment record carries the varying count as
  `number_of_varyings_in_fragment_shader = fs->num_inputs`
  (`driver/src/v3d/v3d_job.c:865`), a field wide enough for 15 [M].
- **Flag words.** The back end emits word 0 of the flat, non-perspective and
  centroid flags only, 24 varyings, "more than MAX_VARYING_VECTORS asks
  for" (`driver/src/v3d/v3d_job.c:496-512` and `driver/src/v3d/v3d_job.c:757-763`;
  `driver/src/v3d/v3d_job.c:800`). 15 fits word 0; no ES 3.0 limit asks for the second
  word.
- **Implicit varyings.** `gl_PointCoord` takes two slots in front of the
  declared ones for points (`driver/src/v3d/v3d_job.c:828-831`), so 15 declared
  varyings plus the point coordinate use 17 slots, still under 24 [M].
- **Frame-side struct.** The compiler's result reaches the C side through
  `driver/src/v3d/v3d_frame.h:178` (`uses_implicit_point_line_varyings`) and
  the flag words in the same struct; none of them is sized by the varying count [M].

Conclusion [M]: raising the V3D's `MAX_VARYING_VECTORS` to 15 is a constant
change in `src/gl/context.rs` plus a vertex-output constant of 64; nothing in the
compiler, record or flag words limits it. What is not measured is a
draw that reads all 15 vectors on the board, including a flat and a centroid
varying past index 8. That is the first board check after the change.

## 3. PowerVR and GC7000UL: is 64 vertex outputs reachable?

- PowerVR: the PPP varying count is 8 bits and the VDM vertex output size is
  8 bits in units of 4 dwords (comment at `src/gl/context.rs:125-146`), so a
  64-dword output fits the encodings [M]. What limits it is the USC output
  register count the vertex program may use: [U] until a 16-vector vertex
  shader is compiled for it.
- GC7000UL: [U]. The `vivante_imx8mp_limits` test pins 60; the code that
  decides it sits under `src/vivante/**`.

Both are issue drafts (section 5), not changes: those directories belong to
other plans.

## 4. The float and 64-bit query forms

`glGetFloatv` answers a pname that `get_floatv` does not know by calling
`get_integerv` and widening (`src/gl/state.rs:1480-1498`);
`glGetInteger64v` calls `get_integerv` directly (`src/gl/state.rs:1398-1410`).
`get_floatv` (`src/gl/state.rs:960-1006`) lists only real-valued state, none
of the limits above. So the three forms of a limit return the same number
from one match arm [M]. There is no separate defect in these paths for
a limit: dEQP's `*_getfloat` and `*_getinteger64` rows of these limits fail
because the integer value is below the minimum, the same as the
`implementation_limits` row, and they pass when the value does. The one
distinction the code makes is for unsigned state
(`src/gl/state.rs:1022-1029`), the four stencil masks, which no limit uses.
The note `docs/notes/v3d-notpass-rest.md` G3 suspects another defect; this
reading does not find one. A host test that queries each limit in all three
forms and compares them (item 3) pins it, and a failing one would be the
defect.

## 5. Order of work and issue drafts

Done on main (plan #359): items 1-3 below, each pinned by a host test
(`tests/es3_v3d_varyings.rs`, `tests/es3_query_forms.rs`,
`tests/es3_limits_v3d.rs`, `tests/es3_caps.rs` `known_gaps()`,
`tests/es2_report_limits.rs`; the report-es2 answer stays 8 / 32 / 6144 / 1536).

1. V3D: `MAX_VARYING_VECTORS` 15 and `MAX_VERTEX_OUTPUT_COMPONENTS` 64 under
   `!report-es2`.
2. Float/int64: no separate defect; the three forms share one arm
   (section 4).
3. V3D uniform blocks: `MAX_UNIFORM_BLOCK_SIZE` 16384, combined components
   50688 (`gl/limits.rs`).
4. Issue drafts for what is left (each waits for its owner of
   `src/pvr/**` or `src/vivante/**`):
   - "PowerVR: MAX_UNIFORM_BLOCK_SIZE 16384 and the combined uniform
     components 50176 / 50048": the user blocks must be read from memory
     (or behind the constant file); report 6144 / 1536 until then.
   - "GC7000UL: the same", design in `docs/notes/gc-ubo-const-limit.md` (`load`).
   - "PowerVR and GC7000UL: MAX_VERTEX_OUTPUT_COMPONENTS 64": needs a
     16-vector vertex compile (16 output rows, one the position) and the VDM
     output size / SHC io rows to carry it; reports 60 until then.
   - "Mali: varying limits": stays with the Mali plans (8 / 32 / 32).
   - "Host dEQP before/after on implementation_limits, state_query.integers*,
     ubo and the varying groups for v3d, pvr, vivante": needs the patched
     VK-GL-CTS 93bca01 build (`tools/cts/README.md` 2b), not present in the
     dependency cache; the Pi 4 board rows are to be re-run.
