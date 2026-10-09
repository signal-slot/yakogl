# GLSL ES 3.00 coverage of the online shader compiler

How much of GLSL ES 3.00 `src/shaderc/online/**` takes, measured over the
dEQP `data/gles3/shaders/*.test` corpus (23 files). Measured on the host with
`tests/es3_shaderc_corpus.rs`, 2026-10-10, on main 32778b57 and each commit of
the ES 3.00 gap work after it (the tables are the latest; the commit messages
carry each step's before and after).

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
| linkage | 83 | 2 | 34 | 0 |
| loops | 9 | 0 | 0 | 0 |
| negative | 0 | 0 | 12 | 0 |
| preprocessor | 288 | 4 | 264 | 0 |
| qualification_order | 8 | 7 | 64 | 0 |
| scoping | 34 | 0 | 48 | 0 |
| switch | 0 | 0 | 52 | 0 |
| swizzle_math_operations | 544 | 0 | 0 | 0 |
| swizzles | 648 | 0 | 0 | 0 |
| uniform_block | 64 | 2 | 72 | 0 |
| **total** | **3444** | **17** | **2135** | **0** |

So of the 3,461 programs that must build, 3,444 (99.5 %) do; of the 2,135 that
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
| conversions | 1060 / 0 / 96 / 0 | 952 / 108 / 96 / 0 |
| declarations | 0 / 0 / 54 / 0 | 0 / 0 / 54 / 0 |
| fragdata | 0 / 0 / 4 / 0 | 0 / 0 / 4 / 0 |
| functions | 199 / 0 / 112 / 0 | 187 / 12 / 112 / 0 |
| indexing | 28 / 0 / 0 / 0 | 28 / 0 / 0 / 0 |
| invalid_implicit_conversions | 0 / 0 / 768 / 0 | 0 / 0 / 768 / 0 |
| invalid_texture_functions | 0 / 0 / 120 / 0 | 0 / 0 / 120 / 0 |
| keywords | 8 / 0 / 348 / 0 | 8 / 0 / 348 / 0 |
| large_constant_arrays | 14 / 2 / 0 / 0 | 16 / 0 / 0 / 0 |
| linkage | 85 / 0 / 34 / 0 | 71 / 14 / 34 / 0 |
| loops | 9 / 0 / 0 / 0 | 9 / 0 / 0 / 0 |
| negative | 0 / 0 / 12 / 0 | 0 / 0 / 12 / 0 |
| preprocessor | 288 / 4 / 264 / 0 | 288 / 4 / 264 / 0 |
| qualification_order | 15 / 0 / 57 / 7 | 8 / 7 / 57 / 7 |
| scoping | 34 / 0 / 48 / 0 | 34 / 0 / 48 / 0 |
| switch | 0 / 0 / 52 / 0 | 0 / 0 / 52 / 0 |
| swizzle_math_operations | 544 / 0 / 0 / 0 | 544 / 0 / 0 / 0 |
| swizzles | 648 / 0 / 0 / 0 | 648 / 0 / 0 / 0 |
| uniform_block | 64 / 2 / 72 / 0 | 64 / 2 / 72 / 0 |
| **total** | **3451 / 10 / 2128 / 7** | **3312 / 149 / 2128 / 7** |

## Expected-pass programs refused, by root cause

"Units" are programs (a `both` case counts twice). Causes are from the first
error of each refusal and from reading the case source; the compile log has
no stage or construct more precise than that.

| cause | class | PowerVR | GC7000UL | V3D | files | example (first error) |
|---|---|---:|---:|---:|---|---|
| `gl_MaxDrawBuffers` is 1 | back-end limit | 2 | 2 | 2 | constant_expressions | `other.complex_{vertex,fragment}`: `'case' : two case labels have the same value` |
| `centroid in` fragment input | lowering gap | 9 | 0 | 9 | linkage, qualification_order | `varying.interpolation.centroid`: `Usc(FragmentInputsNotBuilt)` (PowerVR), `NotYet("centroid varying")` (V3D) |
| Backslash-newline inside a preprocessing token | unsupported construct | 4 | 4 | 4 | preprocessor | `line_continuation.preprocessing_token_*` (`#def\` + newline + `ine`): `'def' : invalid preprocessor directive`; `line_continuation.token_*`: `'oo' : expected ';'` |
| Uniform (instance) name of 63+ characters | back-end limit | 2 | 2 | 2 | uniform_block | `valid.long_instance_name_*`: link: the name is as long as the 64-byte reflection field (yakogl task #47) |
| Matrix-typed vertex attribute (the generated `a_in0` of the matrix `input`s) | back-end limit (V3D) | 0 | 0 | 132 | conversions 108, functions 12, linkage 12 | `matrix_to_matrix.mat4x3_to_mat4_*`: `NotYet("an attribute location with no declaration")` |
| Register pressure of a 128-element constant array index | back-end limit (GC7000UL) | 0 | 2 | 0 | large_constant_arrays | `indexing.vec4_128_vertex`: `Abi(LoadBalancingTempsUnmeasured { temps: 34, temps_max: 21 })` |

No expected-pass program was refused by the
type checker (sema) on any GPU: there is no "sema bug" among the 17/10/149
refusals. The `\` splice (row 4) is in `pp.rs`/the lexer, which is the
"unsupported construct" class; no `#version 300 es` *syntax* is refused.

### Notes behind the rows

* `gl_MaxDrawBuffers`: `constant_expressions.other.complex` uses
  `case clamp(min(gl_MaxDrawBuffers, 4), 1, 6):` next to `case 1:` and expects
  the limit to be 4 (ES 3.0 minimum). The value the front end constant-folds is 1 on all three GPUs, so two labels fold to 1. It is the MRT item
  (`mrt4`), not a switch bug.
* The `centroid` rows are all `centroid in` in the fragment shader (`flat in`
  and `smooth in` build). Every file where `centroid` appears is in the table.
* V3D: the 132 `an attribute location with no declaration` refusals are all
  programs whose generated vertex shader declares a matrix attribute
  (`conversions.matrix_to_*`, and the complete programs in `functions`/`linkage`
  that use one). The cause is read from the generated source; the V3D back-end
  message does not name the matrix.
* GC7000UL builds the centroid inputs that PowerVR and V3D refuse, and it is
  the only GPU refusing a large constant array.
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
| `invariant` accepted on a fragment shader input | sema bug | 0 (refused, but by the back end, see below) | 7 | 7 (refused at link by `centroid varying`, not at compile) | `qualification_order:variables.invalid.interp_storage_invariant_input` |

## Caveat: refused for the right reason?

"fail refused" counts a refusal for any reason, and the log carries only one.
Reading the messages of the 2,128 PowerVR refusals, almost all name the rule the
case tests. The exceptions, where the refusal comes from a stage past sema and
the case would not be refused if that stage had the construct (so a back end
that grows the feature will turn them into "fail accepted"): 16 on PowerVR, 9 on GC7000UL and V3D.

* 7: `qualification_order:variables.invalid.*_invariant_input` (PowerVR:
  `Usc(FragmentInputsNotBuilt)` from the `centroid in`; same cases as the
  "invariant on a fragment input" row above).
* 4 + 4 + 1: `functions:invalid.local_function_{prototype,definition}` and
  `scoping:invalid.use_variable_from_{if,else}_in_outer_scope` are refused by the
  lowering (`this statement is not lowered yet`, `reading this is not lowered
  yet`) and `linkage:varying.rules.invalid_type_array_struct` (`an array of
  structures ... is not lowered yet`). The scoping pair is a sema bug (an
  out-of-scope name is found), hidden by the lowering; the function-in-function
  pair is an unsupported construct. GC7000UL and V3D show only the first 9; PowerVR adds the seven above.
* 10 recursion cases (`functions:invalid.*recursion*`) are refused as `this call
  nests too deeply to inline`, i.e. by the inliner's depth bound, not by a recursion check.

## What this says for ES 3.0

* The ES 3.00 front end (preprocessor, lexer, parser, sema, constant folder) is
  in good shape: 3,444 of 3,461 expected-build programs build on the PowerVR.
  Shader compilation is not where most of the AM62P's GLES3 failures come from;
  the handful below is what this corpus can show. Failures that need a draw or
  the API (integer targets, MRT, transform feedback varyings) are not in it.
* Work that closes the table:
  1. `mrt4`: `gl_MaxDrawBuffers` of 4 turns the two `complex` cases green.
  2. Centroid fragment inputs on PowerVR and V3D (9 programs each).
  3. Preprocessor: backslash-newline splice inside a token (4).
  4. Reflection names of 64 or more characters (2).
  5. Sema: `invariant in` on a fragment input; name scoping that lets a block
     variable be read outside its block (2); recursion by call graph.
  6. V3D only: matrix vertex attributes (132 programs, the biggest single
     refusal in the whole table).
