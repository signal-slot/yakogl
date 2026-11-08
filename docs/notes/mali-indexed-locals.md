<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali Valhall: locals indexed at run time (case 80 matdyn)

**Status: design, implementation in `fs-indexlocal`** (plan #336, issue #33).
No code changed by this note. It maps `tools/mali/testdata/other_80_matdyn.expect.json`
(`fs.fragment`, 61 words listed, Mesa 26.2.1 on drm-shim) to the places in
`src/shaderc/online/mali/` that will produce each word, and lists what still
stands between case 80 and the reference once the indexed locals are lowered.
It continues `mali-remaining-families.md` F1 ("State after item `chain-80`")
and `mali-uniform-types.md` section 7 (the older `unitype_80` reading).

Source: `tools/mali/testdata/other_80_matdyn.frag` (its second half is the
subject: `arr[j] = arr[i] * 0.5`, `p[i]`, `col[i - 1]`, `v[j]`, `arr[2]`).

## 1. What Mesa does (the answer to "bcsel chain or scratch")

Not scratch, not a register index. Mesa 26.2.1 (gallium panfrost, the path
`drm_shim_ref.sh` runs) keeps no function-temp variable that is read or
written at a run-time index:

- `src/panfrost/compiler/bifrost/bifrost_nir.c:978-994` (the late pipeline of
  every Bifrost/Valhall shader): `if (nir_shader_has_local_variables(nir))`
  `nir_lower_indirect_derefs_to_if_else_trees(nir, nir_var_function_temp, 8)`
  (arrays of up to **8** elements, "after vars_to_ssa and copy propagation
  these end up as a handful of MUX instructions"), then
  `nir_opt_constant_folding`, `nir_lower_vars_to_ssa`, `nir_opt_dce`.
  Longer arrays go to scratch (`nir_lower_vars_to_scratch`); no sweep case
  has one.
- `src/compiler/nir/nir_lower_indirect_derefs_to_if_else_trees.c`
  (`emit_indirect_load_store_deref`): for an array of `n` elements and an index
  `idx` that is not a constant, a **binary search**: `mid = start + (end -
  start) / 2`, `if (ilt(idx, mid)) { [start, mid) } else { [mid, end) }`,
  recursively, with a leaf per element. A load ends in an `if_phi`, a store
  stores the value in the leaf only. Out of range is not an error: an index
  below 0 lands in element 0, one at or above `n` in element `n - 1`.
- The pass is not given `nir_var_shader_in/out` here (those are lowered earlier,
  `pan_shader.c:568`, and a fragment colour is not indexed in this case).
- A **vector's component** (`col[i - 1]`, `v[j]`) is not a deref in GLSL IR's
  NIR translation; `glsl_to_nir` emits `nir_vector_extract`
  (`nir_builder.h:969-984`): a constant index is a channel, anything else is
  `_nir_select_from_array_helper` (`nir_builder.h:947-958`), the same binary
  search as `bcsel(ilt(idx, mid), lower half, upper half)` over the channels.

After the standard loop (`bifrost_nir.c` optimisation passes:
`nir_lower_vars_to_ssa`, `nir_opt_peephole_select`, `nir_opt_algebraic`,
`nir_opt_constant_folding`, `nir_opt_cse`, `nir_opt_dce`) the if/else trees
are `bcsel` over `ilt(i, k)` masks. The rules that shape them are in
`nir_opt_algebraic.py`:

| rule | line | used for |
|---|---|---|
| `bcsel(a, b, bcsel(c, b, d))` -> `bcsel(a \| c, b, d)` | 2488 | last element of a store tree: `ior(d1, d2)` (word 12) |
| `bcsel(a, b, bcsel(c, d, b))` -> `bcsel(c & ~a, d, b)` | 2489 | middle element of a store tree: `iand(d2, inot(d1))` (words 8-10) |
| `bcsel(a, bcsel(b, c, d), d)` -> `bcsel(a & b, c, d)` and `bcsel(a, bcsel(b, d, c), d)` | 2486, 2487 | the same for the other nesting |
| `bcsel(a, bcsel(a, b, c), d)`, `bcsel(a, b, bcsel(a, c, d))` | 972, 973 | same condition twice |
| `bcsel(a, bcsel(b, c, d), bcsel(b, c, e))` and the three siblings | 974-977 | `col`: one `ilt` shared by the two lanes (words 17-23) |
| `bcsel(inot(a), b, c)` -> `bcsel(a, c, b)` | 971 | the `.not` never survives into a `CSEL` operand order |
| `op(bcsel(a, #b, #c), #d)` -> `bcsel(a, op(b, d), op(c, d))` and `nir_opt_constant_folding.c:59` `is_bcsel_with_two_constants` | 1026-1040 family | `arr[i] * 0.5` over the constants of `arr` folded into the select (words 1-5) |

## 2. The reference, word by word

FAU (`fs.fragment.fau`, 16 words; words 0-9 are zero because the shim sets no
values): `u2.w0 = u_k.x`, `u2.w1 = u_k.z`, `u3.w0 = u_k.y`, `u3.w1 = u_k.w`
(**not** x, y, z, w in order as `mali-uniform-types.md` section 7 says: words
42-50 read `4 - z` as `u2.w1` and `2/y` as `FRCP u3.w0`; the push order is the
order of use), `u4.w0 = u_c`, `u4.w1 = u_i` (a raw 32-bit int), `u5.w0 = 0.0625`,
`u5.w1 = 0.5`, `u6.w0 = 0.25`, `u6.w1 = 0.125`, `u7.w0 = 0.375`,
`u7.w1 = 0x3c003c00` (fp16 1.0 pair for the blend). Notation: `I = u_i`,
`J = I + 1`, `c1 = ilt(I, 1)`, `c2 = ilt(I, 2)`, `d1 = ilt(J, 1)`,
`d2 = ilt(J, 2)`, `L = I - 1`; `CSEL.u32.eq r, a, 0x0, t, e` is `a == 0 ? t : e`
and a mask is -1 where true (`.m1`).

The NIR below is reconstructed from the words (no `.stderr` is kept for this
reference; the command to dump it is in section 6).

| words | reference | meaning (NIR) | produced by |
|---|---|---|---|
| 0 | `MOV.i32.discard r0, 0x0` | the zero the first compares take as their accumulator | FAU limit rule, section 4 item 3 |
| 1-5 | `r2 = c2` (`ICMP_OR.s32.lt.m1 r2, u4.w1, 0x3020100.b2, r0`), `r1 = c2==0 ? .25 : .125`, `r3 = c1` (`..lt.m1 r3, u4.w1, 0x1000000.b3, r0`), `r1 = c1==0 ? r1 : 0.0625` | `X = arr[I] * 0.5`: the 3-element load tree `bcsel(c1, .125, bcsel(c2, .25, .5))` with the `fmul` folded in (0.0625, 0.125, 0.25) | read tree of `arr` (3.1), fold (3.4) |
| 6 | `IADD_IMM.i32 r4, u4.w1, #0x1` | `J = I + 1` | `fuse_add_imm` (exists) |
| 7 | `r5 = d1` (`..lt.m1 r5, r4, 0x1000000.b3, 0x0`) | the store tree's first compare | write tree (3.2) |
| 8 | `LSHIFT_OR.i32 r6, 0x0, 0x0.b00, r5.not` | `inot(d1)` | `Alu::Inot` (exists, `nir_fs.rs:1467`; check, section 4 item 4) |
| 9-10 | `r7 = d2`, `LSHIFT_AND.i32 r6, r7, 0x0.b00, r6^` | `iand(d2, inot(d1))` | rule 2489, `emit.rs:298` |
| 11 | `r6 = CSEL r6, 0x0, u6.w0, r1` | `arr[1]' = (J == 1 path) ? X : .25` | write tree |
| 12-13 | `LSHIFT_OR.i32 r8, r5, 0x0.b00, r7`, `r8 = CSEL r8, 0x0, r1, u5.w1` | `arr[2]' = ior(d1, d2) ? .5 : X` (rule 2488; `d1 \| d2` is **not** simplified to `d2`) | `emit.rs:299` |
| 14 | `r2 = CSEL r2, 0x0, r8, r6` | `c2 ? arr[1]' : arr[2]'` | read tree after the store |
| 15 | `r1 = CSEL r5, 0x0, u6.w1, r1` | `arr[0]' = d1 ? X : .125` | write tree |
| 16 | `r1 = CSEL r3, 0x0, r2, r1` | `arr[I]'` = `bcsel(c1, arr[0]', bcsel(c2, arr[1]', arr[2]'))` | read tree |
| 17-20 | `r2 = u2.w0`; `r2 = c1==0 ? u2.w1 : r2`; `r6 = u3.w0`; `r3 = c1==0 ? u3.w1 : r6` | `p[I].x = c1 ? x : z`, `p[I].y = c1 ? y : w` (the 2-column tree over the column's two lanes) | matrix-column read tree |
| 21-23 | `IADD_IMM r6, u4.w1, #-1`, `r6 = ilt(L, 1)`, `r2 = r6==0 ? r3 : r2` | `col[L]` = `bcsel(ilt(L, 1), p[I].x, p[I].y)` (`nir_vector_extract`) | lane read tree |
| 24-31 | `FADD_IMM` x4, 4 `FMA` | `(m*m)[1][0] - (m*m)[0][0]` of `m = m++` result | exists (`CHAIN_80`-style, `m *= m`) |
| 32-36 | `r4 = ilt(J, 3)` (`0x3020100.b3`), `r6 = u6.w0`, `r4 = r4==0 ? u7.w0 : r6`, `r5 = d1==0 ? u6.w1 : 0x0`, `r4 = d2==0 ? r4 : r5` | `v[J]` = `bcsel(d2, bcsel(d1, 0, .125), bcsel(ilt(J, 3), .25, .375))` | lane read tree of a 4-lane constant vector |
| 37-56 | `FMA` / `FCMP_OR.f32.lt.m1` / `CSEL` / `FADD` / `FRCP` / `MKVEC` / blend | `b = arr[I] + col[L]*0.25 + v[J] + arr[2]`, `g = (m[1][0]-m[0][0])/8 + t.x + t.y`, `r`, packing | exists, but see section 4 items 2, 3 |
| 57-60 | `ATEST`, `BLEND`, epilogue | | exists |

Notes on the values: `arr[2]` after the store is `arr[2]'` (r8, word 48 adds
it); `ilt(I, 1)`, `ilt(I, 2)` and `ilt(J, ...)` are each computed once (CSE:
words 2, 4, 7, 9 are the only four compares of the arrays; `c1` also serves
`p[I]`).

## 3. Where the code will change

Current state: `lower.rs` (shared by all four back ends) builds every
run-time index as an `ieq` ladder, over slots (a matrix's columns, an array's
elements, a struct's leaves are one local each, consecutive, under one name):

- a read of an array element or matrix column: `select_among`
  (`lower.rs:8304-8318`), `bcsel(ieq(i, k), els[k], ..)` from the last
  element back;
- a write: `store_among` (`lower.rs:8323-8333`) and `store_lref` /
  `SlotSel::Among` (`:8397-8410`), `bcsel(ieq(i, k), v, old)` into slot `k`,
  and `chain_write_at` (`:4944-4968`) for chains;
- a vector's component: `vector_lane_select` (`lower.rs:8350-8365`), also
  `ieq`; a component write `LaneSel::Dyn` (`:8404`, `:3003-3015`), an `ieq`
  against `(0, 1, 2, 3)` as a lane mask.

`glsl.rs` `indexed_read` (`glsl.rs:~150`) refuses the component form only, as
`MaliError::IndexedLocal`; the array and column forms compile today as an
`ieq` ladder (`ICMP_OR.u32.eq` / `CSEL`), which is why `es300/tmpidx.frag` and
`es300/dynidx.frag` are `ok` in `tools/mali/corpus-verdicts.txt` (verdicts only
move forward; they have no reference). The fragment walk handles an
`index: None` `LoadLocal` / `StoreLocal` only (`nir_fs.rs:3900-3968`; `nir.rs:2283-2299`
for the vertex), the front end never emits an indexed one for these locals, so
**the work is a rewrite of the ladders into Mesa's `ilt` trees, not a new
operation in the walks.** The comparison itself is already lowered:
`nir_fs.rs:1483` / `:2069` (`AluOp::Ilt` -> `Alu::Icmp(Lt)`), the constant byte
operand `0x3020100.bN` is the existing small-int immediate encoding
(`builtin_43/44/47/51/57`, `intloop_tri_int` carry it).

Do the rewrite in **`glsl.rs`**, Mali only (it already holds `indexed_read` and
the IR passes `passes()` before rebalance), not in `lower.rs`: the other back
ends' references and gate rows are made from the `ieq` ladder.

### 3.1 Read trees (arrays, columns, lanes)

Recognise `bcsel(ieq(i, 0), x0, bcsel(ieq(i, 1), x1, .. x_{n-1}))` with the
same `i` and constants ascending from 0 (the shape of `select_among`) and of
`vector_lane_select` (`x_k` = lane `k` of one value, swizzles differing).
Rewrite to `_nir_select_from_array_helper(x, i, 0, n)`:
`bcsel(ilt(i, mid), helper(start, mid), helper(mid, end))`,
`mid = start + (end - start) / 2`, a leaf at `end - start == 1`. Sizes seen:
n = 2 (`p[i]`, mid 1), 3 (`arr[i]`: `ilt(i, 1)` over `ilt(i, 2)`), 4
(`v[j]`: `ilt(j, 2)` over `ilt(j, 1)` and `ilt(j, 3)`). The compare's
constant is `ConstI32` and the operand order is `ilt(i, mid)`.
**Which `ilt` is shared** is the CSE of the walk (`nir_fs.rs` hashing of
`Alu` nodes; BIR `opt.rs:526` `cse`): `ilt(I, 1)` is used by `arr[I]`, `p[I]`,
and the second read of `arr`.

### 3.2 Write trees (arrays and columns)

`store_among` makes, for each slot `k`, `new_k = bcsel(ieq(i, k), v, old_k)`.
Replace the condition of slot `k` with the path of the same binary search:
`conj` of `ilt(i, mid)` (left) or `inot(ilt(i, mid))` (right) down to leaf `k`,
applied by building the exact nest Mesa has (each level a `bcsel` on that
level's compare of the subtree's phi) and then applying the rules of the table
in section 1 in that order (2486-2489, 972-977, 971) on the IR. For n = 3 this
is, for `j`:

```
arr0' = bcsel(d1, X, a0)                       -> CSEL d1 .. (word 15)
arr1' = bcsel(iand(d2, inot(d1)), X, a1)       -> words 8-11
arr2' = bcsel(ior(d1, d2), a2, X)              -> words 12-13  (note: a2 first)
```

`inot` is not simplified into a swapped `CSEL` here because `iand(d2, inot(d1))`
needs the mask value itself. The reference materialises `inot(d1)`
(word 8, `LSHIFT_OR r6, 0x0, 0x0.b00, r5.not`) and does not fold it into
`iand`. The loads of the `old` values come from the walk's SSA locals
(`Fs.locals`, `nir_fs.rs:3900`), so the constants `.125 .25 .5` are the
initial values of the three slots and the folds of section 3.4 act on them.

The out-of-range behaviour changes on purpose: `ieq` ladders leave every slot
unchanged for `j >= n`, Mesa stores into the last slot. Both are undefined in
GLSL ES; the reference is the oracle.

### 3.3 Component write (`LaneSel::Dyn`)

`ieq(i, (0, 1, 2, 3))` as a lane mask (`lower.rs:3010`, `:8404`) has no
reference in this case (no source writes a component at a run-time index), so
this note does not move it: it stays an `ieq` mask until a reference exists.
Only reads move.

### 3.4 Folds that must already exist

- `fmul(bcsel(c, #a, #b), #d)` -> `bcsel(c, a*d, b*d)` for words 1-5: check
  `nir_fs.rs` around `:1380` (`Bcsel` with two `Const` arms) and `:2079-2083`;
  if the fmul is not distributed, the constants of `arr` reach `FMA` instead
  of `CSEL`. Cover by a test of its own (section 5).
- `bcsel(c, b, b)` -> `b` (`:2082`, exists).
- The `iand`/`ior`/`inot` of 32-bit masks: `Alu::Iand` / `Alu::Ior` `emit.rs:298-299`,
  `Alu::Inot` `nir_fs.rs:1467`; `bool32` accepts `Ior`/`Iand` of masks
  (`:1398`).

## 4. What else stands between case 80 and the reference

Measured and read by `mali-remaining-families.md` F1 (steps 2-4); restated with
the places, in the order the compiler meets them:

1. **`IndexedLocal`** itself: remove `indexed_read` and `MaliError::IndexedLocal`
   from `glsl::passes` for reads; keep the error where a store with
   `index: Some` or a lane write the walks cannot take still reaches
   `nir_fs.rs:3461` / `:3968` / `nir.rs:2299` (they are defects, not gaps, after
   the rewrite: keep the variant for them).
2. **`MixedPrecision` from the `int` uniform `u_i`.** `mod.rs` counts any
   uniform not declared highp as mixed in a highp shader; `u_i` is an `int` at
   the fragment default `mediump`, and Mesa reads it as a 32-bit word
   (`ICMP_OR.s32 .., u4.w1`; `LowerPrecisionFloat16Uniforms` touches float
   uniforms only). Counting only float uniforms lets case 80 through but also
   `es100/ifelse_loop.frag` (same single refusal), which is **not** Mesa's yet
   (159 words against `mixed_ifelse_loop`'s 143). The item decides: count
   float uniforms only and move `ifelse_loop` forward in `corpus-verdicts.txt`
   if it compiles, or narrow the test to "an int uniform used as an index or
   a compare". Either way `tests.rs` gets a pin for the narrowed rule.
3. **FAU limit of the first compares (word 0).** `ICMP_OR.s32.lt.m1 r2,
   u4.w1, 0x3020100.b2, r0` has a uniform and a byte constant; the `or` operand
   is a *register* holding zero (`MOV.i32.discard r0, 0x0`), but from word 7 on
   the same compare takes `0x0` as an immediate because its first source is a
   register. So the rule is: a third zero source that would be a
   third FAU word is materialised once (`MOV r0, 0`, used while still needed).
   Check `fau.rs` (~`:753`) and `select.rs` for what a compare with a uniform
   plus a constant byte does; there is no other reference with this exact
   pair. Also: the `MOV` is marked `.discard` (the register is dead after
   its last use at word 38, `FCMP_OR .. r0^`): that is the RA's last-use mark
   (`ra.rs`).
4. **`s11 = w + w`** (word 43): `FMA.f32 r3, u3.w1, 0x40000000, r3^`. Mesa
   turns `fadd(a, a)` into `fmul(a, 2.0)` (`nir_opt_algebraic.py:274`) and fuses
   it. `nir.rs`'s 32-bit builder does not apply :274 to a source sum
   (`the_matrix_local_chain_compiles_in_the_source_order` shows `FADD w, w`).
   Add to the fragment walk's `Fadd` (`nir_fs.rs` `op32`, ~`:2062`).
5. **The uniform order in the FAU.** Mesa pushes `u_k` as `x, z, y, w` (order
   of use in the NIR after the passes: `x`, `z` in `m[1][0] ..`, then `y`, `w`),
   the compiler pushes in declaration order: first difference of the bypass
   measurement (`u5.w1` against `u6.w1`, word 1). The constants follow the
   uniforms: `1/16, 0.5, 0.25, 0.125, 0.375` (`u5.w0 .. u7.w0`) are in the
   order the NIR first uses them; the order falls out of the walk order
   (`fau.rs`), so first make the NIR walk order match (3.1 and 3.2 build the
   trees at the place of the first use).
6. After 1-5, 60 words against 61 were measured with 1 and 2 bypassed in a
   scratch build (first difference word 1, then the ladders): expect a
   residue in scheduling (words 7-16 interleave two trees) that no
   note predicts; it is the item's measurement.

## 5. Tests to add (in `fs-indexlocal`)

Host tests, in `src/shaderc/online/mali/tests.rs` unless said:

1. `glsl_rewrites_a_read_ladder_into_the_ilt_search`: the IR after `passes` of
   a three-element array, a two-column matrix and a four-lane vector read at
   a run-time index: `Ilt(i, 1)` / `Ilt(i, 2)` for n = 3, `Ilt(j, 2)` over
   `Ilt(j, 1)` and `Ilt(j, 3)` for n = 4, no `Ieq` left (the
   `glsl_leaves_a_chain_of_matrix_local_columns_in_order` style).
2. `glsl_rewrites_a_write_ladder_into_the_ilt_tree`: the three conditions of
   section 3.2 (`d1`; `iand(d2, inot(d1))`; `ior(d1, d2)`), by IR shape.
3. `the_run_time_array_read_compiles_like_mesa`: a small program with only the
   array part of `matdyn` (`arr[j] = arr[i] * 0.5`, `arr[i] + arr[2]`) against a
   **new** drm-shim reference (this case's words 0-16, 48, minus the matrix and
   lane parts), made with `drm_shim_ref.sh run` and `pan_ref_diff.py extract`
   (section 6): the pin that is checked before the whole case, so a mismatch in
   3.1/3.2 is not mixed with the matrix and the FAU order. Likewise a
   `p[i]`/`col[i - 1]`/`v[j]` part (words 17-23, 32-36).
4. `case_80_is_refused_at_its_indexed_local_reads` is replaced: first by the
   gap of the next refusal (`MixedPrecision` until 4.2 is done), finally by
   `other_80_matdyn` in the byte-compared list (`tests.rs:5636` region, the
   `(name, vert, frag, expect.json, false)` rows); do not add it before it is
   byte-equal. The `MaliError::IndexedLocal` row in `gap_cases` (`tests.rs:~1241`,
   the `v[i]` of a `u * u` vector) changes with it.
5. `tools/mali/corpus-verdicts.txt`: `es100/matdyn.frag`, `dynidx.frag`,
   `dynidx_loop.frag` leave "an indexed local variable" (they may land at the
   next gap or `ok`, never back); `es300/tmpidx.frag` and `es300/dynidx.frag`
   stay `ok`. `tools/mali/sweep_passsets.py --delta tools/cts/log` must exit 0
   (case 80 is a REFUSED -> PASS move, no PASS case leaves).
6. A `fmul(bcsel(c, k1, k2), k3)` fold test if 3.4 shows the fold absent.

## 6. Regenerating the NIR

The reconstructed NIR above can be checked against Mesa's own dump on a
machine with the tree built by `tools/mali/drm_shim_ref.sh build` (this
machine had Mesa 26.2.1's sources in the deps cache and no build):

```
tools/mali/drm_shim_ref.sh run <work> tools/mali/testdata/other_80_matdyn.vert \
    tools/mali/testdata/other_80_matdyn.frag <out>
```

`<out>.stderr` holds the NIR after the last pass and the disassembly
(`BIFROST_MESA_DEBUG=shaders,shaderdb`). Compare its `ilt` / `bcsel` nests
with sections 2 and 3 before writing the rewrite.

`unitype_80.expect.json` is the earlier recording of the same shader
(`tools/shaders/es100/matdyn.frag` as is); `other_80_matdyn` is the one the
case is pinned to.
