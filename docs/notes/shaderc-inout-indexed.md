<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Indexed element as an out/inout argument (RC13), 2026-10-14

Measured at the integration tip `cd738b1ae7cf`, on the host, with
`tests/es3_shaderc_corpus.rs` and the pinned dEQP corpus
(VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`,
`data/gles3/shaders/`). Investigation only: no file under `src/` or `tests/`
changed.

## Result in one paragraph

**There is nothing left to fix.** `docs/es3-cts-failures.md` RC13 lists
"Indexed element passed as out/inout argument" as 8 / 0 / 8 programs
(PowerVR / GC7000UL / V3D). At the tip all 8 programs build on all three
GPU features. The row is stale, and its cause was mis-named: the 8 programs
were refused for `trunc`, not for the indexed argument. The indexed
copy-in/copy-out (`LRef`) was already in `lower.rs` and already correct.
`b1cf9e4` ("shaderc: trunc on the PowerVR and the V3D ...", 2026-10-10)
closed them. `docs/es3/shaderc-coverage.md` says so
("The `indexing.moredynamic.*_out_parameter_*` programs were refused for
their `trunc`, not for the indexed out argument"), but the RC13 table and
its "no item yet" lines were not updated.

## The 8 programs

`indexing.test`, group `moredynamic`. Four cases, each `both` = a `_vertex`
and a `_fragment` program:

| case (line in `indexing.test`) | the call | dEQP expected |
|---|---|---|
| `builtin_fncall_out_parameter` (78) | `modf(5.5, v[u_zero + 3])` | `v = (1,2,3,5)`, `f = 1.0` |
| `user_defined_fncall_out_parameter` (97) | `foo(v[u_zero + 3])`, `foo(out float f){ modf(5.5, f); }` | `v = (1,2,3,5)`, `f = 1.0` |
| `user_defined_fncall_inout_parameter` (119) | `foo(v[u_zero + 2])`, `foo(inout float f){ float g = f + 2.5; modf(g, f); }` | `v = (1,2,5,4)`, `f = 1.0` |
| `user_defined_fncall_inout_parameter_with_index_with_side_effects` (196) | `foo(v[funcWithSideEffects()])`, same `foo` | `v = (1,2,5,4)`, `sideEffectCounter == 1`, `f = 1.0` |

Every case: `vec4 v = vec4(1.0, 2.0, 3.0, 4.0);` and
`f = 1.0 - distance(v, expected)`, so `f == 1.0` means the element was
written, exactly once.

All four call `modf`, which lowers to `AluOp::Ftrunc`
(`lower.rs modf`, line ~8145). That is the common factor.

## Verdicts and refusal messages

At the tip (`cargo test --test es3_shaderc_corpus` with
`DEQP_GLES3_TEST_DIR` and `DEQP_GLES3_REPORT`; the report lists every unit
that is not an expected-pass success, and none of the 8 is in it):

| GPU feature | `indexing` row | 8 programs | refusal message |
|---|---|---|---|
| `gpu-powervr,shaderc-online` | 28 pass ok, 0 refused | built | none |
| `gpu-vivante,shaderc-online` | 28 pass ok, 0 refused | built | none |
| `gpu-v3d,shaderc-online,no-shader-table` | 28 pass ok, 0 refused | built | none |

Totals at the tip (pass ok / pass refused / fail refused / fail accepted):
PowerVR 3453 / 8 / 2135 / 0; GC7000UL 3451 / 10 / 2135 / 0; V3D 3453 / 8 /
2135 / 0. The refused programs are other causes: `constant_expressions.other.complex`
(2, all three), `preprocessor.line_continuation.*` (4, all three; item
pp-splice), `uniform_block.valid.long_instance_name` (2, all three), and on
GC7000UL only `large_constant_arrays.indexing.{float,vec4}_128_vertex` (2;
item gc-const-array). None is an out/inout argument.

Message before `b1cf9e4` (read from the parent commit's sources, not
re-run: a parent build was not made):

* PowerVR: the shape table had `Ftrunc` as `Shape::Lower`; the builder
  returned `BuildError::Unsupported("lowering into several groups")`
  (`powervr/assemble.rs`).
* V3D: `v3d/prepare.rs` refused `Ftrunc` by name (the op-name table lists
  `Ftrunc`, the supported set did not).
* GC7000UL: `Ftrunc` was selected (etnaviv `TRUNC`-style sequence), so the
  GC7000UL built them. That is the answer to "why GC7000UL differs": it
  never hit the missing `trunc`, not that its indexed lowering is different.
  The front end (`sema.rs`, `lower.rs`) is shared and identical for all
  three.

The commit message of `b1cf9e4` gives the same numbers: PowerVR 3436 ->
3444 (these 8), V3D 3302 -> 3312 (the same 8 plus 2 bitwise cases),
GC7000UL 3451 -> 3451.

## Root cause

1. Front end, shared by all three GPUs, accepts indexed out/inout
   arguments. In `lower.rs` the inliner, when binding an argument to an
   `out`/`inout` parameter (line ~4310), calls `resolve_lref(arg)` for a
   one-slot parameter that is not a whole variable. `resolve_lref`
   (line ~8161) evaluates the dynamic index once (`index_value`) and returns
   an `LRef { slots: SlotSel, lane: LaneSel }`: `SlotSel::Among` for
   `a[i]` / `m[i]`, `LaneSel::Dyn` for `v[i]`, `LaneSel::At`/`Swz` for
   `v.y` / `v.zx`. `inout` reads it once into the parameter slot with
   `load_lref` (copy in, line ~4458); after the callee's scope is popped, the
   loop at line ~4506 loads the parameter and calls `store_lref` (copy out).
   Both use the same `LRef`, so the index runs once (the
   `sideEffectCounter == 1` condition). `modf` uses the same path
   (`modf`, line ~8145). `sema.rs` only checks that an out argument is an
   l-value (line ~1932, ~730); it does not look at the index.
2. Back ends: the refusals were an unrelated missing `Ftrunc` on the
   PowerVR and V3D (above), fixed by `b1cf9e4`.
3. The RC13 table recorded the refusal as the indexed argument because the
   cases are named `*_out_parameter`. The coverage doc's fix note
   (`b1cf9e4`) already corrected it; `es3-cts-failures.md` was not
   re-measured.

The unit tests that pin the indexed path are in `lower.rs`,
`mod lvalue_ref_tests` (`an_element_argument_is_resolved_once`,
`modf_writes_the_whole_part_through_any_destination`,
`swizzles_and_members_as_destinations`), run by the host eval.

## Fix design

None needed for the 8 programs. For the record, where the copy-in/copy-out
of an indexed l-value belongs, and it is where it already is:

* `lower.rs`: `resolve_lref` (resolve once), `load_lref` / `store_lref`,
  the argument binding in the inliner (`Bound.lref`, copy in at ~4458, copy
  out at ~4506) and `modf`.
* `sema.rs`: no change; it only requires an l-value.
* Back ends: no change; they see ordinary `LoadLocal` / `StoreLocal`
  (`Among` is a select chain over the slots).

Shapes `resolve_lref` still returns `None` for (the caller then refuses with
"an out argument that is not a variable is not lowered yet"): a whole
array/matrix element that is a struct or an array of structs
(`s[i]` as one out argument for a multi-slot parameter, `run != Single`),
a uniform, a value-bound (`l.value.is_some()`) local, a function-local `Run`
that is not `Single`, and a swizzle of a swizzle's lane. I did not find a
dEQP-GLES3 `.test` program in the 23 files that needs these (the corpus
refuses none). They are follow-up work for whoever holds `lower.rs`
(plan #18/#19), if a program appears.

## Minimal GLSL repro and expected values

```glsl
#version 300 es
precision mediump float;
uniform int u_zero;
int sideEffectCounter = 0;
int funcWithSideEffects() { sideEffectCounter++; return 2; }
void foo(inout float f) { float g = f + 2.5; modf(g, f); }   // g = 5.5, f = 5.0
layout(location = 0) out float o;
void main() {
    vec4 v = vec4(1.0, 2.0, 3.0, 4.0);
    foo(v[funcWithSideEffects()]);               // index evaluated once
    vec4 expected = vec4(1.0, 2.0, 5.0, 4.0);
    o = 1.0 - distance(v, expected);
    if (sideEffectCounter != 1) o = 0.0;
}
```

Expected: `o == 1.0`, `v == (1, 2, 5, 4)`, `sideEffectCounter == 1`.
The other three: `modf(5.5, v[u_zero + 3])` gives `v == (1, 2, 3, 5)`;
`foo(out float f){ modf(5.5, f); }` with `v[u_zero + 3]` gives the same;
`foo(inout ...)` with `v[u_zero + 2]` gives `(1, 2, 5, 4)`. `u_zero` is 0.

## Reproduce

```text
DEQP_GLES3_TEST_DIR=<VK-GL-CTS>/data/gles3/shaders \
DEQP_GLES3_REPORT=/tmp/r.tsv \
  cargo test --no-default-features --features host-test,<gpu-powervr|gpu-vivante|gpu-v3d,no-shader-table>,shaderc-online \
  --test es3_shaderc_corpus -- --nocapture
grep -c 'indexing' /tmp/r.tsv      # 0: no indexing program is refused
```

Only the shader compiler was exercised. Nothing was drawn or run on a
board, and the programs' output values were not checked on a GPU: "built"
means both stages compile and link, as the corpus defines it. The values
above are from the `.test` file; the host evaluator checks the same shape in
the `lvalue_ref_tests` unit tests.

## Suggested follow-up (not done here)

* Correct the RC13 row in `docs/es3-cts-failures.md` (8 / 0 / 8 -> 0 / 0 / 0,
  fixed by `b1cf9e4`) and the "indexed out/inout arguments" entry in the
  "no item yet" line (~435). I did not edit it: it is outside this item.
* If wanted, a corpus-level pin: `tests/es3_shaderc_corpus.rs` has no
  per-program assertion for `indexing.moredynamic.*`.
