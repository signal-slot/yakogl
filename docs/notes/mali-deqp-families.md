<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the dEQP-GLES2 compile-refusal families of plan #377

Plan #377 item `measure-families` (issue #20, stage 5 of issue #7), first
part. It records which cases of the whole-tree NanoPC-T6 run at `37305608`
(`docs/es2-cts-failures.md` section M; slices
`tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/`) each family of the plan
holds, by the first refusal per case in each slice's `refusals.tsv`. It is a
count of the board run's refusals, not yet the host measure of every shader
pair through the online compiler at the tip: that measure (the shapes within
each family) is still to do.

## How to redo it

    python3 -I tools/cts/mali-family-tree.py tools/cts/log/2026-11-15-t6-gles2-S{1,2,3,4}-37305608

prints a tree table (the form of `mali-gles2-tree.txt`) of rows `T6RF001..`
one family each; `tools/cts/mali-family-refusals.txt` maps refusal texts to
families, `tools/cts/mali-family-tree-selftest.sh` checks the tool. The table
passes `check-gles3-tree.py --mustpass tools/cts/gles2-mustpass.txt --max 400`
with 7,313 cases matched once, none twice, no empty row (the other cases are
not refused by these texts).

## Cases per family

| Family | Rows | Cases | Refusal texts (`src/shaderc/online/mali/mod.rs`) |
|---|---|---|---|
| `intval` | T6RF001-009 | 3,223 | an integer or boolean value |
| `varylink` | T6RF010-011 | 656 | two varying lanes of equal value |
| `unitype` | T6RF012-014 | 820 | a uniform other than a float, vec2, vec3, vec4 or mat4 (456); a mediump fragment shader uniform other than ... (285); a vertex shader uniform that is not highp (79) |
| `uniarray` | T6RF015 | 272 | a uniform array (269); a uniform read at a run-time index (3) |
| `constop` | T6RF016-017 | 621 | a constant operation on or to a value that is not a finite normal float (618); a constant fp16 does not hold (3) |
| `colourpair` | T6RF018 | 171 | a colour pair of a constant then a varying lane (142) or of a varying lane and 0.0 (29) |
| `vsloop` | T6RF019-020 | 497 | a loop in a vertex shader |
| `fsloop` | T6RF021 | 277 | a loop Mesa keeps in a mediump fragment shader (268); a break or continue behind a run-time condition (6); a loop past Mesa's unroll limits (3) |
| `varyuniform` | T6RF022 | 256 | a varying lane computed from no attribute (252); a varying lane left unwritten (4) |
| `mixprec` | T6RF023 | 192 | a fragment shader that mixes highp and mediump or lowp |
| `posunwritten` | T6RF024 | 155 | a gl_Position lane left unwritten |
| `negzero` | T6RF025 | 117 | a -0.0 read through a variable |
| `invariant` | T6RF026 | 12 | an invariant output |
| `misc` | T6RF027 | 44 | work registers (2), more than 2048 IR values (3), a comparison other than ... (3), a built-in input (1), a texture look-up other than texture2D (27), a texture result's lanes (2), a shape Mesa's algebraic rules rewrite without a reference (6) |
| Total | 27 rows | 7,313 | |

The counts include the NotSupported cases the refusal also gives.

## What is left out, and why

Refusals plan #367 holds through its glmark2 families stay out of the table
(`docs/notes/mali-glmark2-forecast.md` section 2): `ARITH_CONST`
("a constant product in a shape Mesa's algebraic rules move or fold", 463
cases in the slices), `ARITH` (1,431), `FRAGCOLOR_LANE` (793), `FS_OPERAND`
(142), `TEX_COORD` (109), `FMA` (100), `LOCALS` (160), the rebalance, control
flow and fragment varying load or conversion texts. They are listed in the
header of `tools/cts/mali-family-refusals.txt`. The integer, loop, link-rule
and constant-operation families are kept because the plan takes the dEQP
shapes #367's notes (`mali-int-loops.md`, `mali-varying-constants.md`) leave;
`misc` keeps `TEX_LANES` (not on the plan's exclusion list) and the
"rewrite without a reference" text (not `ARITH_CONST`'s text).

## Not done yet

- The host measure of every refused case's shader pair (feature set
  `host-test,gpu-mali,shaderc-online,no-shader-table`): the shapes within each
  family and what each unblocks.
- The Mesa 26.2.1 reference cases `tools/mali/testdata/deqp_<family>_*` and
  the per-family study notes (`mali-deqp-unitype.md` and its siblings exist
  for some families).
