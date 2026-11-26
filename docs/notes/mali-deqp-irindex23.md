<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the `deqp_irindex2` and `deqp_irindex3` references, their dEQP cases and root causes

**HOST-ONLY.** Plan #402 item `note-irindex23`. Tip **`22c5fc5f`**; line
numbers are that commit's. No compiler or test change. It adds to
`docs/notes/mali-deqp-irindex.md` (section 5 there names the root causes,
numbered 5.1 to 5.8); it does not edit it.

## Method

The four references were compiled through `tests/mali_ref::compile_pair`
(`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_irindex3 refusals_pinned`)
in a scratch worktree of the tip, not committed. In it every
`MaliError::Defect(Defect::IrIndex)` construction in `nir_fs.rs` printed its
`file:line` and a `RUST_BACKTRACE`-style backtrace, and `compile_pair` printed
a marker, so the last site before each compile's error could be read. Most
constructions fire and are caught on the way (each compile printed 190 to 300
events), so only the **last** site of a compile is its refusal. For
`deqp_irindex2_op_val` the site is the one section 5.8 of the irindex note
traced at `670913e1`; I re-read `nir_fs.rs:15732` at the tip (unchanged) and
did not re-trace it.

## Summary

| Reference | Source shape | dEQP cases | Pinned refusal | Root cause (irindex note §5) | Site at `22c5fc5f` |
|---|---|---|---|---|---|
| `deqp_irindex2_op_val` | mediump fragment colour `l - vec4(k,-0.5,-7,c) - vec4(int(j), i, float(h), g)` with two int uniforms and a dead `ivec3` assignment | 2 | `Defect(IrIndex)`, `tests/mali_deqp_irindex2.rs` (`op_val`) | **5.8** an int uniform pair under `fneg` has no value | `nir_fs.rs:15732` (`operand`) from `op` `:15836`, `select` `:12271` |
| `deqp_irindex3_mat4_mat4` | `v_m0 + v_m1`, two mediump mat4 varyings, `.xyz/.yzw/.zwx/.wxy` column sum | about 9 | `Defect(IrIndex)` | **5.2** unread lane in the sum-split test | `nir_fs.rs:9642` (`index`) from `split_lanes` `:9966`, `split_sum` `:9995`, `lane_tree` `:11206` |
| `deqp_irindex3_mat4_addassign` | `res = v_m0; res += v_m1;` same sum | about 6 | `Defect(IrIndex)` | **5.2** | same as above |
| `deqp_irindex3_mat4_preinc` | `m = v_mat4; res = ++m;` same sum, vertex shader passes `a_mat4` | about 8 | `Defect(IrIndex)` | **5.2** | same as above |

All four pins are the whole message
`Valhall back end defect: an invariant of the pipeline did not hold (IrIndex)`.

## `deqp_irindex2_op_val`

- **Source.** The fragment shader of
  `dEQP-GLES2.functional.shaders.random.scalar_conversion.fragment.56`,
  verbatim from the dEQP `.qpa` (its `.license` says so). Its `combined.56`
  partner has the same fragment shader; the vertex shader file is the pair's.
- **Cases.** `refusals.tsv` of the four slices holds `scalar_conversion.fragment.56`
  and `scalar_conversion.combined.56` as the two `Defect(IrIndex)` cases of
  this shape (the header of `tests/mali_deqp_irindex2.rs` counts 2 of the 206).
- **Refusal pinned.** `Defect(IrIndex)` (`op_val` in `REFUSED` of
  `tests/mali_deqp_irindex2.rs`).
- **Root cause.** 5.8. `c - vec4(int(j), i, ...)` becomes an `fneg` of a
  `Leaf::IntWide` pair; `operand` reads its `val`, which is `None` because
  `IntWide` gets a value only as a colour root (`nir_fs.rs:12315-12323`) and
  `op`'s operand set-up does not make it. The site is the `| P::Op(..) =>
  val[x].ok_or(Defect::IrIndex)?` arm at `nir_fs.rs:15732`.
- **Minimal pair** (irindex note 5.8): `vec4(1.0) - vec4(j, i, 0.0, 0.0)`
  refuses, `+` compiles.

## `deqp_irindex3_*` (hand-written)

- **Source.** The three shader pairs are **hand-written** in the form of the
  dEQP `matrix.*` fragment shaders, not taken from a `.qpa` (the `.license`
  files say so). `mat4_mat4` stands for
  `shaders.matrix.{add,sub}.dynamic_{lowp,mediump,highp}_mat4_mat4`,
  `mat4_addassign` for `shaders.matrix.{add,sub}_assign.dynamic_*_mat4`, and
  `mat4_preinc` for `shaders.matrix.{pre,post}_{increment,decrement}.dynamic_{lowp,mediump}_mat4`.
  The counts (9, 6 and 8) are the header's "counted by eye" approximations of
  40 `matrix.*` IrIndex rows; the case names above come from that header and
  from the slices' `refusals.tsv`, **not re-confirmed against a dEQP build**
  (the header already asks the fixing item to do so). The highp rows are in
  the list but the mediump path is what the references draw.
- **Refusal pinned.** All three: `Defect(IrIndex)` (`REFUSED`).
- **Root cause, all three: 5.2**, the same as `deqp_irindex_mat4_pair_unread`.
  The last `Defect::IrIndex` of each of the three compiles is
  `nir_fs.rs:9642:122` (`index`, the `ok_or` on `f.lane_of`) called from
  `split_lanes` `:9966:46`, `split_sum` `:9995:20` and `lane_tree` `:11206:40`
  (then `nir_fs.rs:11317`, `:10260`, `:11763`, `mod.rs:1737`). `split_lanes`
  looks up both input lanes with `?` (`let ((k, i), (e, j)) = (index(f, v,
  i)?, index(f, w, j)?)`). The `.zwx` and `.wxy` swizzles read a pair's lane
  without the other, so a lane has no `lane_of` entry and the lookup errors
  where the question "is this pair a split sum" should answer `false`. The
  `++m`, `+=` and `a + b` forms differ only in the nodes in front of the sum;
  the vec4 minimal pair of note 5.2 (`(m + w).zwx` refuses, `.zwz` compiles)
  is the same invariant.

## Which references the proto-irindex fix covers

- **`deqp_irindex3_mat4_mat4`, `mat4_addassign`, `mat4_preinc`:** covered by
  the fix direction of irindex note 5.2 (`split_lanes` answers `Ok(false)`
  when a lane has no `lane_of` entry). It moves the three to the same place
  as `deqp_irindex_mat4_pair_unread`; whether the words then equal Mesa's is
  the lowering's question, so each may land in `DIFFERS` rather than
  `LOWERED`. The three tests need `#[ignore]` removed and the `REFUSED`
  entries moved in the same commit (`refusals_pinned` fails otherwise).
- **`deqp_irindex2_op_val`: needs more.** 5.2's fix does not touch it. It
  needs 5.8's separate change (an `IntWide` arm in `op`'s operand set-up,
  `nir_fs.rs:15799-15833`, building the value as `:12315-12323` does).
  If the fix item covers only 5.1 to 5.5, `op_val` stays `#[ignore]`d.
- The mediump sum split is the largest cause among the matrix cases, but
  whether the gap past the defect is only a different word order or a
  mixed-precision refusal (note 5.x "past the defect") is open until the
  fix is run; `load_vec3_cmp` and `sqrt_op_value` already show the next
  refusal can be the mixed-precision text, which is outside the IrIndex fix.

## Caveats

- The irindex3 case names and counts are the header's, not re-measured with a
  dEQP build.
- The `op_val` site is re-read, not re-traced, at the tip.
- `index` has two `ok_or(IrIndex)` constructions (`:9642` and `:9645`); only
  `:9642` fired as a final refusal here.
