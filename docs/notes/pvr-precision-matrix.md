<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: the 144 matrix `builtin_functions.precision` not-pass lines (SH-PREC-MAT)

Plan #400, item `design-matprec`. This note gives the cause and the fix design.
It builds on the host triage of item `triage-matprec` (which left a report but
no file) and on items `tri-matqw`, `matqw-uscexec` and `modf-mrt-locate`.
Nothing here ran on a board, and no file under `src/`, `tests/` or `tools/`
changed.

What it was read against:

- **Code:** integration tip `14e7a742`, 2026-11-28, and the board build
  `b25997e4`.
- **dEQP:** VK-GL-CTS `93bca01`, the revision tools/cts/README.md names.

`tools/cts/am62p-notpass-causes.py` files these lines under `SH-PREC-MAT`,
"matrix precision functions: results outside dEQP's bound", status `open`. The
non-matrix precision lines (`SH-PREC`, 20) are in `pvr-precision-scalar.md`.

## 1. Summary

None of the 144 lines is a precision defect. dEQP's shader executor writes the
bits of each matrix column to its own 32-bit integer colour attachment. Every
matrix result larger than `mat2` therefore needs more than the BXS-4-64 pixel's
four 32-bit output registers. On the PowerVR such a draw was refused:

- **54 lines, the `mat4*` sizes:** the link refused them first, because
  GL_MAX_DRAW_BUFFERS was 3.
- **90 lines, the other sizes:** the draw was dropped with no GL error. dEQP
  then read back whatever the renderbuffers held. A finite wrong value counts
  as a QualityWarning in dEQP (section 2.2).

The matrix arithmetic itself is exact (section 4). The fix is PowerVR
back-end and compiler work that spills the attachments past the four registers
to tile buffers. It is not shared front-end lowering. It is implemented on the
branch of item `pvr-modf-mrt` (commit `5bcab1fc2`, in review, not at the tip).
Section 5 adds the host test that branch lacks for the 90 two- and
three-column shapes.

## 2. The cases

### 2.1 Where they are in the TSV

All 144 lines are
`dEQP-GLES3.functional.shaders.builtin_functions.precision.{matrixcompmult,outerproduct,transpose}.{lowp,mediump,highp}_{vertex,fragment}.<size>`.

- **Rows:** tree rows 7 (`matrixcompmult`), 8 (`outerproduct`) and 9
  (`transpose`) of `tools/cts/am62p-gles3-tree.txt`. Rows are counted from 0
  over its non-comment lines.
- **Build and run:** b25997e4, run 2026-11-22. The TSV header gives this for
  rows 0-10.
- **Not affected:** `determinant` and `inverse` (rows 6-7) have no not-pass
  line. The `mat2` cases of the three functions (18) pass.

### 2.2 How dEQP produces the two messages

**The output layout.** dEQP names each case after the matrix type the shader
returns (`MatrixFuncCaseFactory::addCase`, `glsBuiltinPrecisionTests.cpp`).
GLSL spells matCxR as C columns of R rows.

The executor is `FragmentOutExecutor` in `glsShaderExecUtil.cpp`. A
`VertexShaderExecutor` is one too (`:732`), so both stages go through the
fragment shader's outputs. Under ES there are no float targets, so it writes
`o_out0_c = floatBitsToUint(out0[c])`, one output for each column `c`
(`:365-380`, `:541-545`, `:600`).

The attachment format comes from `getRenderbufferFormatForOutput`
(`:564-595`):

- a `vec2` column goes to RG32UI;
- a `vec3` or `vec4` column goes to RGBA32UI ("No RGB variants available").

**The two messages.** Both come from `glsBuiltinPrecisionTests.cpp`:

- **Fail.** "Shader compilation failed" is `if (!executor->isOk())` (`:4967`).
  That is the executor's compile and link check.
- **QualityWarning.** "Shader output 0 has low-quality shader precision"
  (`:5040`) is raised when the value is outside the reference interval but
  inside its *warning* interval.

The warning interval is unbounded unless a function sets it. `tcu::Interval`
initialises `m_warningLo`/`m_warningHi` to ∓infinity
(`framework/common/tcuInterval.hpp:73-103`), and `containsWarning` (`:193`)
fails only on a NaN. So in this family a QualityWarning means **any finite
wrong value**. `transpose` only moves values, so it cannot lose precision.

## 3. Groups

The grouping axis is the result's size, because the size fixes the
attachment layout. Each size has the same 18 cases: 3 functions × 3 precision
qualifiers × 2 stages.

The function and the precision qualifier do not split the outcome. Every
function has 48 lines (16 at each of lowp, mediump and highp). Within a size,
every line has the same outcome.

### 3.1 The group table

The layout column shows where the bits go. "r0-3" means output registers 0-3;
"tb N" means tile buffer N. "Program" is the end-of-tile program that layout
needs (`src/pvr/mrt_layout.rs` `program_for`).

| group | sizes | attachments (dwords) | layout past 4 registers | program | TSV outcome, message | cases |
|---|---|---|---|---|---|---|
| G1 | `mat4x2` | 4 × RG32UI (8) | r0-1, r2-3, tb 0, tb 1 | `eot4_tb2` (4, 2) | Fail, "Shader compilation failed at glsBuiltinPrecisionTests.cpp:4967" | 18 |
| G1 | `mat4x3`, `mat4` | 4 × RGBA32UI (16) | r0-3, tb 0, 1, 2 | `eot4_tb3` (4, 3) | Fail, same message | 36 |
| G2 | `mat2x3`, `mat2x4` | 2 × RGBA32UI (8) | r0-3, tb 0 | `eot2_tb1` (2, 1) | QualityWarning, "Shader output 0 has low-quality shader precision" | 36 |
| G3 | `mat3x2` | 3 × RG32UI (6) | r0-1, r2-3, tb 0 | `eot3_tb` (3, 1) | QualityWarning, same message | 18 |
| G4 | `mat3`, `mat3x4` | 3 × RGBA32UI (12) | r0-3, tb 0, tb 1 | `eot3_tb12` (3, 2) | QualityWarning, same message | 36 |
| **total** | | | | | 54 Fail + 90 QualityWarning | **144** |

For comparison, `mat2` (2 × RG32UI, 4 dwords) fits the registers and passes:
18 cases, not in the TSV.

### 3.2 Checking the counts

To check the counts:

```sh
python3 -I tools/cts/am62p-notpass-causes.py docs/es3/am62p-gles3-notpass.tsv
```

prints `SH-PREC-MAT 144`.

```sh
grep -P 'precision\.(matrixcompmult|outerproduct|transpose)' docs/es3/am62p-gles3-notpass.tsv | cut -f1,3 | sort | uniq -c
```

gives 54 Fail and 90 QualityWarning, with the two messages above.

## 4. Causes

### 4.1 The arithmetic is exact

The lowerings section 1 asked about, and why each is ruled out:

- **mediump done at 16 bits.** The PowerVR selection has no F16 operand type
  (`src/shaderc/online/powervr/sel.rs:173-186` at `14e7a742`), so every
  qualifier computes at f32. The highp cases fail exactly like lowp and
  mediump, and `mat2` passes at all three qualifiers.
- **Fused versus separate multiply-add, or division by reciprocal.** The front
  end does neither for these functions:
  - `matrixCompMult` is one `Fmul` per column
    (`src/shaderc/online/lower.rs:6816-6823` at `14e7a742`);
  - `outerProduct` is `c * rv[j]`, one `Fmul` per column (`:6802-6813`);
  - `transpose` only moves components (`:6794-6800`, `transpose_columns`
    `:6880-6892`).

  There is no add, no FMA and no reciprocal.
- **The inverse/determinant expansion.** `determinant` and `inverse` have no
  not-pass line.
- **Host evidence.** `tests/pvr_mat4_precision.rs`
  `highp_fragment_cases_match_the_cpu_in_the_byte_model` passes at
  `14e7a742`. It runs the 54 four-column programs' USC bytes through the host
  byte model under the R32UI variant and compares the bits with the CPU's. Item
  `matqw-uscexec` ran 324 probes of the same functions through `tests/usc_exec`
  with 0 mismatches.

### 4.2 G1, at b25997e4: the link refused a fourth draw buffer (54 Fail)

- `src/backend.rs:910-913` at `b25997e4` set the PowerVR arm of
  `DRAW_TARGETS` to `3`.
- A four-column result declares `layout(location = 3)`. The link refuses it in
  `src/gl/program.rs:1709` at `b25997e4` ("fragment output at location 3 ends
  at 4, beyond the 3 draw buffers").
- `executor->isOk()` is false, which gives Fail.

This is **fixed at the tip** by `c72c90f8` ("pvr: DRAW_TARGETS is 4 in the ES
3.0 report"; `src/backend.rs:914-921` at `14e7a742`). `c72c90f8` is not in
`472630df`, the build of board-a/b/c/d.
`tests/pvr_mat4_precision.rs` `the_54_cases_compile_and_link_at_four_targets`
pins the link (it passes at `14e7a742`, 4 of 4 tests).

Past the link, G1 meets the cause of G2-G4. Its layout (4, 2) or (4, 3) already
has a program at the tip (`program_for`, `src/pvr/mrt_layout.rs:193-200`).

### 4.3 G2-G4, and G1 at the tip: several attachments of one integer layout past four registers are refused (90 QualityWarning)

When every attachment has the same integer layout, the GL core keys the draw
with that one layout's key. The PowerVR then refuses it at three points:

1. **The compiler.** `pack_int_output` packs every output into the pixel's
   registers. It refuses past four dwords: `src/shaderc/online/powervr/chain.rs:1470-1471`
   at `14e7a742` (`:1262` at `b25997e4`), "integer colour outputs past the
   pixel's four output registers".
2. **The back end.** `int_targets_drawn` refuses the draw with
   `REFUSE_ONE_LAYOUT_PAST_FOUR`: `src/pvr/backend.rs:7465` at `14e7a742`
   (`:6708` at `b25997e4`). The message is defined at
   `src/pvr/target_keys.rs:29`.
3. **The end-of-tile programs.** Even past those two refusals, two layouts
   have no program. `program_for` (`src/pvr/mrt_layout.rs:193-200`) knows
   (3, 1) and (4, 1..3) but not:
   - (2, 1), needed by G2;
   - (3, 2), needed by G4.

   G3's (3, 1) `Eot3Tb` exists. The programs were captured in
   `tools/pco_shc/usc_blobs.txt` (`eot2_tb1` line 10, `eot3_tb12` line 11),
   but nothing uploads them. `tests/es3_mrt4_budget_pvr.rs:928` pins
   "RGBA32UI x4" as `NoVariant`.

The program does not compile, so the GL core drops the draw with no GL error.
The executor's renderbuffers keep their old contents, which are finite.
Section 2.2 explains why that is a QualityWarning.

What this means for each build:

| build | G1 (mat4*) | G2-G4 |
|---|---|---|
| `b25997e4`, and `472630df` (board-a) | 54 Fail | 90 QualityWarning |
| tip `14e7a742` | 54 QualityWarning (they link now, and the draw is refused) | 90 QualityWarning |

Neither row has been checked on a board after `b25997e4`.

## 5. Fix design

### 5.1 The change

The change is in the PowerVR back end only. It is what item `pvr-modf-mrt`
implements (commit `5bcab1fc2`, "pvr: integer colour attachments past the four
output registers spill to tile buffers", in review). Its file set, in order:

1. **The driver blob.** In `tools/pco_shc/gen_usc_blobs.py` and the
   regenerated `driver/src/pvr_usc_eot_blob.h`, `eot2_tb1` and `eot3_tb12` get
   table slots, and a new `pvr_usc_eot_tb_n_spills[]` says how many tile-buffer
   addresses each program has. `driver/src/pvr_render.c` patches that many
   addresses on upload. `struct pvr_render_static` is unchanged.
2. **The layout table.** In `src/pvr/mrt_layout.rs`, `program_for` gains
   (2, 1) → `Eot2Tb1` and (3, 2) → `Eot3Tb12`. Every layout of up to four
   targets then has a program.
3. **The compiler.** In `src/shaderc/online/powervr/chain.rs`, a layout key
   with several outputs past four registers (at most four outputs, no colour
   mask) compiles as the per-target list `[Int(rt); n]` through
   `target_layout`. That list writes the attachments that fit to registers and
   stores the rest to tile buffers 0, 1, 2. Outputs that fit keep
   `pack_int_output`'s program byte for byte.
4. **The back end.** In `src/pvr/backend.rs`, `int_targets_drawn` takes the
   registers and tile buffers from `mrt_layout::layout` at any count and checks
   the record's `num_tile_buffers`. The end-of-tile emit sources each
   attachment after a spilled one from its tile buffer, and picks the program
   by (emitted, spilled).
5. **Shared keying.** In `src/target_key.rs`, `shc_mixed_out_places` and
   `shc_spill_has_program` are added, and `src/pvr/target_keys.rs` accepts
   them. This matters for mixes such as fragment_out. The SH-PREC-MAT lists
   are all one layout and do not need it.

No `src/gl/**` change is needed. G1 needs nothing beyond `c72c90f8`, which is
already on main.

### 5.2 Shared front-end lowering?

**No.** `src/shaderc/online/lower.rs` and the rest of the shared front end are
unchanged, so Vivante, V3D and Mali compiler output and their
`tests/reference/*.rs` do not move.

The one shared file is `src/target_key.rs`. It is GL-core target keying, not
compiler lowering. With it, the other GPUs key a float-and-integer list that
spills past four registers as a mix, as they already keyed a mix inside four
registers. `5bcab1fc2` pins that change:

- `tests/mixed_target_keys_parity.rs` is updated for the new keys;
- `tests/es3_int_mixed_other_gpus.rs` is not touched;
- `src/pvr/internal.rs` verifies unchanged.

Whoever merges it must confirm that against the then-current main.

### 5.3 Host test

**Already on the `pvr-modf-mrt` branch:** `tests/pvr_int_spill.rs` covers the
54 G1 programs into four attachments, two RGBA32UI vectors and three RGBA32UI
vectors. `tests/es3_pvr_int_mixed_gl.rs` covers three RG32UI. Neither runs the
90 dEQP programs of G2-G4 themselves.

**New, `tests/pvr_mat23_precision.rs`:** the G2-G4 counterpart of
`tests/pvr_mat4_precision.rs`. Feature set
`host-test,gpu-powervr,shaderc-online`, not `report-es2`. It generates the 90
executor programs the way `pvr_mat4_precision.rs` generates the 54:

- functions: `matrixcompmult`, `outerproduct`, `transpose`;
- precision: lowp, mediump, highp;
- stage: vertex, fragment;
- sizes: `mat2x3`, `mat2x4`, `mat3x2`, `mat3`, `mat3x4`.

It draws each one through the GL core and the real `pvr_backend_draw` with the
kick stood in for, as `tests/pvr_int_spill.rs` does. It asserts:

1. **Link and draw.** Every program links at the build's own draw-target count
   and draws with no GL error and no `yakogl/pvr: draw refused` line. There is
   one kick, and it has every target.
2. **The layout.** The fragment program's record under the layout's key gives:

   | sizes | registers | `num_tile_buffers` | program |
   |---|---|---|---|
   | `mat2x3`, `mat2x4` | 4 | 1 | `Eot2Tb1` |
   | `mat3x2` | 4 | 1 | `Eot3Tb` |
   | `mat3`, `mat3x4` | 4 | 2 | `Eot3Tb12` |

   The frame's end-of-tile DOUTU matches that program's static slot.
   `p_query`, as in `tests/pvr_int_spill.rs` `slot`, reads it.
3. **The PBE state.** Every spilled attachment's PBE state takes its source at
   dword 0 of its tile buffer. Every other attachment's source is at its
   register start.
4. **The values.** For the highp fragment cases, the USC bytes under the R32UI
   variant (3 or 2 columns, which fit the registers) run through the host byte
   model (`tests/usc_exec` `run_binary`). Each column's first component must be
   bit-equal to the CPU's `floatBitsToUint` of the reference. The byte model
   has no tile-buffer store, so the spilled dwords are checked by item 3 only.
5. **The regression row.** `mat2` (2 × RG32UI, 4 dwords) still compiles with
   `pack_int_output`'s program, 4 registers, `num_tile_buffers` 0 and
   `Plain { targets: 2 }`.

It fails at `14e7a742`: item 1 hits `REFUSE_ONE_LAYOUT_PAST_FOUR`. It passes
on the `pvr-modf-mrt` branch.

The test needs a `tools/check.sh` row (`--test pvr_mat23_precision` with that
feature set). `tools/check.sh` is held by plan #387, so the row goes in with
#387's agreement or in that plan.

### 5.4 Board check

Run AM62P tree rows 7, 8 and 9 at a build that contains both `c72c90f8` and
the `pvr-modf-mrt` change. The expectation is 144 Pass. The 18 `mat2` cases
must stay Pass.

This is not measured. The byte model has no tile-buffer store, so only the
board shows that the spilled columns' bits come back.

## 6. Issue text

The new issue named by `modf-mrt-locate` is still valid after `c72c90f8`
resolved half of it. Proposed new text:

**Title:** `pvr: integer colour attachments of one layout past four output
registers are not drawn (dEQP-GLES3 precision matrix, 144 AM62P cases)`

> dEQP's shader executor writes each column of a matrix result through
> floatBitsToUint into its own RG32UI or RGBA32UI attachment
> (glsShaderExecUtil.cpp:564-595), so every result larger than mat2 passes the
> BXS-4-64 pixel's four output registers. pack_int_output refuses that
> (src/shaderc/online/powervr/chain.rs:1470), int_targets_drawn refuses it with
> REFUSE_ONE_LAYOUT_PAST_FOUR (src/pvr/backend.rs:7465), and the GL core drops
> the draw with no error. The 90 two- and three-column cases read stale
> renderbuffer contents and report "low-quality shader precision" (dEQP's
> warning interval is unbounded). The 54 four-column cases failed to link at
> b25997e4 (DRAW_TARGETS 3, fixed by c72c90f8) and are dropped the same way at
> the tip. The arithmetic is exact (tests/pvr_mat4_precision.rs). Fix:
> pvr-modf-mrt (5bcab1fc2), the tile-buffer spill with the eot2_tb1/eot3_tb12
> programs; host test tests/pvr_mat23_precision.rs
> (docs/notes/pvr-precision-matrix.md section 5.3); board check rows 7-9.
