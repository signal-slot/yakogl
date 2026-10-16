<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GLSL ES 3.00 coverage of the online shader compiler

How much of GLSL ES 3.00 `src/shaderc/online/**` takes, measured over the
dEQP `data/gles3/shaders/*.test` corpus (23 files). Measured on the host with
`tests/es3_shaderc_corpus.rs`, first on 2026-10-10 on main 32778b57. The tables
below were last re-measured on 2026-10-14 at commit `fa497e8` (it holds
gc-const-array, pp-splice and shaders-fold), on all three GPU features; the
commit messages of the gap work carry each step's before and after.

* Corpus: VK-GL-CTS commit `93bca01861b0e3ef3c387027a9791e6d065f900c`
  (the pinned checkout in the deps cache, `VK-GL-CTS/data/gles3/shaders/`).
  The `.test` files and the dEQP logic the test copies are Apache-2.0,
  Copyright (c) The Android Open Source Project and The Khronos Group Inc.
* Entry points: `SHC_ONLINE.compile` for each stage, then `SHC_ONLINE.link`
  (what glCompileShader and glLinkProgram reach). Nothing is rendered: a
  program counts as "built" when both stages compile and the link accepts them.
* Unit: one dEQP test case, i.e. one program. A `both` case gives two (`_vertex`
  tests the vertex shader with a generated fragment shader, `_fragment` the
  reverse), a case with `vertex` and `fragment` gives one. 5,596 programs; 0 skipped
  by the harness.

## Running it

```text
# always-on part (3 inline cases copied from the corpus); the corpus part skips
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_shaderc_corpus

# the corpus; DEQP_GLES3_REPORT writes one TSV line per unit that is not an
# expected-pass success (file, unit, expectation, verdict, first log line)
DEQP_GLES3_TEST_DIR=<VK-GL-CTS>/data/gles3/shaders \
DEQP_GLES3_REPORT=/tmp/es3-report.tsv \
  cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online \
  --test es3_shaderc_corpus -- --nocapture
```

The same with `gpu-vivante,shaderc-online` (GC7000UL) and
`gpu-v3d,shaderc-online,no-shader-table` (V3D 4.2). A `report-es2` build refuses
`#version 300 es` by design (`pp.rs out_of_scope`), so the file compiles to
nothing there. The test asserts no counts (they move as the compiler grows); it
prints the table.

How the test follows dEQP: the `.test` grammar of `gluShaderLibrary.cpp`
(`group`/`case`/`end`, `version`, `expect`, `values { input|output|uniform }`,
`both`/`vertex`/`fragment` `""..""` sources, `#` comments), and the
specialisation of `glsShaderLibraryCase.cpp`: `${DECLARATIONS}`, `${SETUP}`,
`${OUTPUT}`, `${POSITION_FRAG_COLOR}` for `both`; `${VERTEX_DECLARATIONS}`,
`${VERTEX_SETUP}`, `${VERTEX_OUTPUT}`, `${FRAGMENT_DECLARATIONS}`,
`${FRAGMENT_OUTPUT}`, `${FRAG_COLOR}` for complete programs; the generated
partner stage and `isOk` comparison functions (`genVertexShader`,
`genFragmentShader`, `genCompareFunctions`); `${X:single-line}`. Verdicts follow
`ShaderLibraryCase::compileAndLink`: `compile_fail` needs a compile error (a
program that compiles and then fails to link is "wrongly accepted"), `link_fail`
needs a link error, `pass` needs both to succeed.

## Per-file table, PowerVR (AM62P, `gpu-powervr`)

`pass ok`: expected to build, built. `pass refused`: expected to build, refused.
`fail refused`: expected to fail, refused (for any reason: see "Caveat").
`fail accepted`: expected to fail, built.

| file | pass ok | pass refused | fail refused | fail accepted |
|---|---:|---:|---:|---:|
| arrays | 294 | 0 | 22 | 0 |
| conditionals | 21 | 0 | 20 | 0 |
| constant_expressions | 40 | 2 | 0 | 0 |
| constants | 100 | 0 | 45 | 0 |
| conversions | 1060 | 0 | 96 | 0 |
| declarations | 0 | 0 | 54 | 0 |
| fragdata | 0 | 0 | 4 | 0 |
| functions | 199 | 0 | 112 | 0 |
| indexing | 28 | 0 | 0 | 0 |
| invalid_implicit_conversions | 0 | 0 | 768 | 0 |
| invalid_texture_functions | 0 | 0 | 120 | 0 |
| keywords | 8 | 0 | 348 | 0 |
| large_constant_arrays | 16 | 0 | 0 | 0 |
| linkage | 85 | 0 | 34 | 0 |
| loops | 9 | 0 | 0 | 0 |
| negative | 0 | 0 | 12 | 0 |
| preprocessor | 292 | 0 | 264 | 0 |
| qualification_order | 15 | 0 | 64 | 0 |
| scoping | 34 | 0 | 48 | 0 |
| switch | 0 | 0 | 52 | 0 |
| swizzle_math_operations | 544 | 0 | 0 | 0 |
| swizzles | 648 | 0 | 0 | 0 |
| uniform_block | 64 | 2 | 72 | 0 |
| **total** | **3457** | **4** | **2135** | **0** |

So of the 3,461 programs that must build, 3,457 (99.9 %) do; of the 2,135 that
must be refused, all are (but see the caveat). The front end is
not what stands in front of the 8,835 AM62P GLES3 CTS failures: these are
compile/link-time verdicts only. Feature gaps that need a *draw* (integer
render targets, MRT, transform feedback, ...) are invisible here.

## The other two GPUs

Same front end, a different back end (and, for the V3D, the program-level
code generator, which refuses at link).

| file | GC7000UL ok / refused / fail ref. / fail acc. | V3D 4.2 ok / refused / fail ref. / fail acc. |
|---|---|---|
| arrays | 294 / 0 / 22 / 0 | 294 / 0 / 22 / 0 |
| conditionals | 21 / 0 / 20 / 0 | 21 / 0 / 20 / 0 |
| constant_expressions | 40 / 2 / 0 / 0 | 40 / 2 / 0 / 0 |
| constants | 100 / 0 / 45 / 0 | 100 / 0 / 45 / 0 |
| conversions | 1060 / 0 / 96 / 0 | 1060 / 0 / 96 / 0 |
| declarations | 0 / 0 / 54 / 0 | 0 / 0 / 54 / 0 |
| fragdata | 0 / 0 / 4 / 0 | 0 / 0 / 4 / 0 |
| functions | 199 / 0 / 112 / 0 | 199 / 0 / 112 / 0 |
| indexing | 28 / 0 / 0 / 0 | 28 / 0 / 0 / 0 |
| invalid_implicit_conversions | 0 / 0 / 768 / 0 | 0 / 0 / 768 / 0 |
| invalid_texture_functions | 0 / 0 / 120 / 0 | 0 / 0 / 120 / 0 |
| keywords | 8 / 0 / 348 / 0 | 8 / 0 / 348 / 0 |
| large_constant_arrays | 16 / 0 / 0 / 0 | 16 / 0 / 0 / 0 |
| linkage | 85 / 0 / 34 / 0 | 85 / 0 / 34 / 0 |
| loops | 9 / 0 / 0 / 0 | 9 / 0 / 0 / 0 |
| negative | 0 / 0 / 12 / 0 | 0 / 0 / 12 / 0 |
| preprocessor | 292 / 0 / 264 / 0 | 292 / 0 / 264 / 0 |
| qualification_order | 15 / 0 / 64 / 0 | 15 / 0 / 64 / 0 |
| scoping | 34 / 0 / 48 / 0 | 34 / 0 / 48 / 0 |
| switch | 0 / 0 / 52 / 0 | 0 / 0 / 52 / 0 |
| swizzle_math_operations | 544 / 0 / 0 / 0 | 544 / 0 / 0 / 0 |
| swizzles | 648 / 0 / 0 / 0 | 648 / 0 / 0 / 0 |
| uniform_block | 64 / 2 / 72 / 0 | 64 / 2 / 72 / 0 |
| **total** | **3457 / 4 / 2135 / 0** | **3457 / 4 / 2135 / 0** |

## Expected-pass programs refused, by root cause

"Units" are programs (a `both` case counts twice). Causes are from the first
error of each refusal and from reading the case source; the compile log has
no stage or construct more precise than that.

| cause | class | PowerVR | GC7000UL | V3D | files | example (first error) |
|---|---|---:|---:|---:|---|---|
| `gl_MaxDrawBuffers` is 1 | back-end limit | 2 | 2 | 2 | constant_expressions | `other.complex_{vertex,fragment}`: `'case' : two case labels have the same value` |
| Backslash-newline inside a preprocessing token (done: pp-splice) | unsupported construct | 0 | 0 | 0 | preprocessor | was 4 each: `line_continuation.preprocessing_token_*` (`#def\` + newline + `ine`): `'def' : invalid preprocessor directive`; `line_continuation.token_*`: `'oo' : expected ';'` |
| Uniform (instance) name of 63+ characters | back-end limit | 2 | 2 | 2 | uniform_block | `valid.long_instance_name_*`: link: the name is as long as the 64-byte reflection field (yakogl task #47) |
| Register pressure of a 128-element constant array index (done: gc-const-array) | back-end limit (GC7000UL) | 0 | 0 | 0 | large_constant_arrays | was 2 on the GC7000UL: `indexing.vec4_128_vertex`: `Abi(LoadBalancingTempsUnmeasured { temps: 34, temps_max: 21 })` |

No expected-pass program was refused by the
type checker (sema) on any GPU: there is no "sema bug" among the 4/4/4
refusals (8/10/8 before pp-splice and gc-const-array). The `\` splice
was in `pp.rs`/the lexer, the "unsupported construct" class; no
`#version 300 es` *syntax* is refused.

### Notes behind the rows

* `gl_MaxDrawBuffers`: `constant_expressions.other.complex` uses
  `case clamp(min(gl_MaxDrawBuffers, 4), 1, 6):` next to `case 1:` and expects
  the limit to be 4 (ES 3.0 minimum). The value the front end constant-folds is 1 on all three GPUs, so two labels fold to 1. It is the MRT item
  (`mrt4`), not a switch bug.
* `centroid in` builds on every GPU. The PowerVR iterates it with `ditrp`
  at `iter_mode` centroid, as pco does; the V3D multiplies by the centroid
  W (rf1) and sets the input's `centroid_flags` bit, as Mesa's v3d does;
  the GC7000UL iterates it as smooth (no distinction in its back end).
  Single-sampled, the centroid of a covered pixel is its centre, so all
  three give the smooth value there; multisampled, the PowerVR and the V3D
  move the point into the covered samples.
* A matrix vertex attribute, square or ES 3.00's `matCxR`, takes C
  locations of R components (`v3d::chain::attribute_locations`, the GL
  core's `draw.rs vertex_inputs` per column); the V3D refused the
  non-square ones until then (132 programs: `conversions.matrix_to_*` and
  the matrix inputs of `functions` and `linkage`).
* No GPU refuses a large constant array any more; the GC7000UL was the only one
  that did, until gc-const-array.
* `trunc` (`Ftrunc`) is selected on every back end: the PowerVR as pco's
  `lower_ftrunc` select (`x < 0 ? ceil(x) : floor(x)`, four groups), the V3D as
  `FTRUNC`; the V3D also selects the integer shifts (`SHL`, `ASR`, `SHR`),
  integer and unsigned min/max, `~` and the unsigned orderings. The
  `indexing.moredynamic.*_out_parameter_*` programs were refused for their
  `trunc`, not for the indexed out argument. dEQP's
  `shaders.operator.common_functions.trunc` (24 programs, both stages, three
  precisions) is the same operation; it is not in this corpus.

## Expected-fail programs built (wrongly accepted)

| cause | class | PowerVR | GC7000UL | V3D | example |
|---|---|---:|---:|---:|---|
| (none) | | 0 | 0 | 0 | |

`invariant` on an ES 3.00 fragment input is refused by sema (section 4.6.1:
only outputs may be invariant), in the declaration and in `invariant x;`:
the 7 `qualification_order.variables.invalid.*_invariant_input` cases.

## Caveat: refused for the right reason?

"fail refused" counts a refusal for any reason, and the log carries only one.
Reading the messages of the 2,135 refusals, almost all name the rule the
case tests. The exceptions, where the refusal comes from a stage past sema and
the case would not be refused if that stage had the construct (so a back end
that grows the feature will turn them into "fail accepted"): 1 on each GPU,
and the 10 recursion cases below.

* 1: `linkage:varying.rules.invalid_type_array_struct` (`an array of
  structures ... is not lowered yet`).
* Sema refuses `functions:invalid.local_function_{prototype,definition}` (ES
  3.00 section 6.1: a function is declared at global scope only) and
  `scoping:invalid.use_variable_from_{if,else}_in_outer_scope` (an `if` arm
  and a `do` body are each a scope, the grammar's `statement_with_scope`);
  the lowering refused them before, for want of the construct.
* 10 recursion cases (`functions:invalid.*recursion*`) are refused as `this call
  nests too deeply to inline`, i.e. by the inliner's depth bound, not by a recursion check.

## Texture look-ups (not in this corpus)

The `.test` corpus samples no texture, so the look-up forms of dEQP's
`texture.*` (glsTextureTestUtil's programs) and `shaders.texture_functions`
are not counted above. What each back end selects, by form (a refusal names
the form in the log):

* PowerVR: every float look-up of glsTextureTestUtil (2D, cube, 2D array, 3D,
  each shadow sampler, with and without a bias). `texture`, `textureLod`,
  `texelFetch` and `textureOffset` on 2D; the stacked images (cube, 2D
  array, 3D) without an offset. A shadow look-up is the plain look-up of
  the depth and a compare in the shader (pco's `lower_tex_shadow`), on 2D,
  cube and 2D array, with exact plane gradients when the coordinate is a
  smooth varying. A bias on a 3D texture applies to the grid image's
  implicit level, as on the cube's and the 2D array's stacked images.
  `textureProj` with a level or a bias, in a vertex shader, on a 3D texture
  or on a 2D shadow sampler divides in the shader (`frcp`, a `fmul` per
  lane, the reference too), then samples as the unprojected form; the
  plain fragment `textureProj` on 2D keeps `smp`'s own divide. Refused: an
  integer result (`isampler*`/`usampler*`, the *formats* item), gradients
  (`textureGrad`, `textureProjGrad`), and an offset on a stacked image
  (needs the per-layer image of `docs/es3/pvr-2darray.md`).
* GC7000UL: refuses every shadow sampler, integer result, gradient and
  offset, and every ES 3.00 `textureProj` / `textureProjLod` (ES 1.00's
  `texture2DProj` builds).
* V3D: refuses every look-up other than 2D and cube, integer results,
  shadow samplers, offsets and gradients.

## What this says for ES 3.0

* The ES 3.00 front end (preprocessor, lexer, parser, sema, constant folder) is
  in good shape: 3,457 of 3,461 expected-build programs build on each of the three GPUs.
  Shader compilation is not where most of the AM62P's GLES3 failures come from;
  the handful below is what this corpus can show. Failures that need a draw or
  the API (integer targets, MRT, transform feedback varyings) are not in it.
* Work that closes the table:
  1. `mrt4`: `gl_MaxDrawBuffers` of 4 turns the two `complex` cases green
     (open, 2 programs per GPU).
  2. Preprocessor: backslash-newline splice inside a token (4). **Done**
     (pp-splice, 2026-10-14): all 4 programs build on the three GPUs.
  3. Reflection names of 64 or more characters (open, 2 programs per GPU).
  4. GC7000UL: the 128-element constant array index (2). **Done**
     (gc-const-array, 2026-10-14): `large_constant_arrays` is 16 / 0 on the
     GC7000UL, as on the other two (3,457 / 4 / 2,135 / 0; 3,453 / 8 before
     pp-splice). It did not use the address register with relative
     addressing that this item proposed: on the i.MX8MP `movar` then
     `u[base][a.x]` read element 0 for every index
     (`src/vivante/VIVANTE_NOTES.md`). The refusal was register pressure made
     by `alloc.rs` `sink_definitions`, which moved all 127 compares of the
     `acc = (i == k) ? c_k : acc` chain before the selects (35 and 34
     temporaries, limit 21); it now keeps the chain in order (3
     temporaries). See `docs/handoff/20261014-gc-const-array.md` (folded
     into `docs/HANDOFF.md`).
  5. Sema, behind refusals that come from a later stage today: recursion by
     call graph (10, refused by the inliner's depth bound; open).
  6. shaders-fold (the largest dEQP-GLES3 `shaders` bucket in `fold`,
     `fmath`, `builtin_call` and `loop_count`): **done** (2026-10-14).
     `round` / `roundEven` at highp were exact only below 2^23
     (`floor(x + 0.5)`); 16 `builtin_functions.common.round*` /
     `roundeven*` cases on the AM62P. It is a value fix, so no count in
     this table moves (`es3_shaderc_corpus` pass ok is 3,457 before and
     after); `tests/es3_shaderc_fold.rs` pins it. See
     `docs/handoff/20261014-shaders-fold.md`.
