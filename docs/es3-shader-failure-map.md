<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dEQP-GLES3 shaders: failure map

ES 3.0 plan, item `es3-shader-deqp-gap`. Input for `docs/es3-cts-failures.md`
(item `es3-inventory`). Documentation only: nothing was run for this file.
Every number is a count of names in `tools/cts/gles3-mustpass.txt`, a figure
copied from a file in the repository (cited with its line), or arithmetic on
those two (the arithmetic is shown). Where per-case data is missing the row
says so; nothing is estimated.

The only GLES3 board run is AM62P, main `396d667a`, 2026-09-26,
`tools/cts/RESULTS.md` §1r (heading at line 1437), summarised in
`tools/cts/STATUS.md:9` and `:250` (Japanese; `:250` gives "shaders 2,231").
The i.MX8MP and the Pi 4 have no GLES3 run. `396d667a` is not an object in
this clone (squashed history), so no code is cited at it. Sources and their
abbreviations:

- **R-group**: the per-group table, `RESULTS.md:1455-1494`; the shaders row is
  `:1486` (`shaders | 18385 | 2231 | 0 | 0`).
- **R-bucket**: "The 8,835 failures by message", `RESULTS.md:1496-1516`. Only
  the 16 largest of 78 messages are in the repository. They cover 7,530 cases
  (1,483 + 1,293 + 736 + 675 + 531 + 522 + 413 + 298 + 292 + 270 + 252 + 189 +
  181 + 159 + 133 + 103). The other 62 messages, 8,835 − 7,530 = 1,305 cases,
  are not listed anywhere. A bucket names the groups it reached, not how many
  cases each group has in it, except where a line says so. `:1518` splits the
  159 and 292 buckets for texture.shadow.
- **R-img**: the per-image table, `RESULTS.md:1550-1595`. The shaders images
  are rows 6 to 18 (`:1557-1569`). Which subgroups each image ran is the
  matching line of `tools/cts/am62p-gles3-tree.txt` (line N + 4 = image N;
  lines 3-4 say the filters match dEQP's whole tree and the tally restricts to
  the mustpass).
- **R-work**: the work-list rows for the §1r buckets, `RESULTS.md:989-998`.
  `:989` splits the 1,483 bucket by subgroup. `:995` and `:998` are later AM62P
  runs (`15dde808`, `a1f88596`), not `396d667a`.
- **R-host**: the host compile-and-link summary at `008b4652`,
  `RESULTS.md:1982-2003` (§4). HOST-ONLY, GC back end, no pixels.
- **COV**: `docs/es3-shaderc-coverage.md`, the compile/link table over the 23
  `.test` files at `3f78982e` (2026-10-08, after the board run). PowerVR table
  at lines 59-84, the refusal causes at `:130-139`, wrongly accepted at
  `:165-169`.
- **INV**: `docs/es3-cts-failures.md` (the inventory this file feeds). Its
  shaders row is `:57` (1,483 classified, 748 unclassified) and its derivate
  note `:121-129`.
- **HOST3**: `docs/es3-host-run.md`, the full host dEQP-GLES3 run of item
  `host-gles3-run`. It is **not on the integration tip**: it is commit
  `f9916ec` on branch `ccfarm/01a11dcf-05a5-759b-ac85-6632d0ce2cd8` (under
  review). It is used only in §5, every figure from it marked HOST-ONLY.
- **dEQP**: `$CCFARM_DEPS_DIR/VK_GL_CTS/modules/gles3/functional/` (the pinned
  VK-GL-CTS `93bca01861b0`; the directory in the deps cache is spelled
  `VK_GL_CTS`, while `docs/es3-host-env.md` and COV write `VK-GL-CTS`).
  **glshared**: `.../modules/glshared/`. **data**:
  `$CCFARM_DEPS_DIR/VK_GL_CTS/data/gles3/shaders/` (23 `.test` files).
  Group registration: `es3fFunctionalTests.cpp:195-235`; a `ShaderLibraryTest`
  loads `shaders/<name>.test` (`:153-170`).

## 1. What is known per subgroup, and what is not

**No per-case result of the GLES3 run exists in the repository.** The logs
stayed in a worktree's `build/g3-396d667a/` (`RESULTS.md:1597-1598`).
`docs/board-sessions/am62p-gles3-percase.md` tried to produce the per-case
list and got no console output (`:5-13`; "Per-subgroup first-message table:
Not available", `:129-132`). `tools/cts/testdata/` holds parser fixtures only,
and the one `*.results.tsv` under `tools/cts/` is the i.MX8MP GLES2 table.
So this file has three kinds of per-subgroup fact, and nothing else:

1. **Exact, from a bucket that names subgroups.** R-work `:989` splits the
   1,483 bucket into builtin_functions 1,461 and builtin_variable 22. R-bucket
   `:1516` (103) names only shaders.derivate. R-bucket `:1514` (159) names only
   texture.shadow and texture_functions, and `:1518` gives texture.shadow's
   share as 144, so texture_functions has 159 − 144 = **15** in it.
2. **Exact zero, from an image with no failure.** An image's fail figure
   counts every case its filter matched in dEQP's whole tree, which is a
   superset of the mustpass names in those subgroups (image 7 reports 816 +
   1,762 = 2,578 results against 1,949 mustpass names: 1,539 + 29 + 41 + 340).
   Every mustpass case has a result (`RESULTS.md:1439-1440`: only lifetime has
   none). So an image with 0 fails proves 0 mustpass fails in each of its
   subgroups, and an image's fail figure is an **upper bound** on the mustpass
   fails of its subgroups together. INV `:88-95` sets the image rows aside
   because they do not reconcile with the group table; the reason is this
   superset, and it is why they are used here only as bounds and zeros, never
   as counts. For the shaders-only images the superset shows directly:
   images 7, 8, 9, 14, 15 and 16 fail 1,762 + 328 + 16 + 24 + 71 + 32 = 2,233,
   more than the whole group's 2,231 even before texture_functions (≥ 15,
   image 18) is added, so at least 2 + 15 = 17 of the 2,233 are cases outside
   the mustpass.
3. **Upper bounds from shared images.** Image 6 (`:1557`, 96 fails) ran
   primitive_restart, rasterization, rasterizer_discard, read_pixels,
   samplers, shader_api and shaders.arrays (tree line 10). The six non-shaders
   groups fail 0 + 21 + 51 + 24 + 0 + 0 = 96 in R-group (`:1480-1485`), so
   arrays has 96 − 96 = **0**. Image 18 (`:1569`, 516 fails) ran
   texture_functions, uniform_block, state_query and texture.compressed (tree
   line 22); state_query fails 63 (`:1487`), so texture_functions +
   uniform_block + texture.compressed ≤ 516 − 63 = 453.

The whole group: 20,616 mustpass names = 18,385 pass + 2,231 fail + 0 not
supported (R-group `:1486`). Of the 2,231, 1,461 + 22 + 103 + 15 = **1,601**
are tied to a subgroup by a bucket; the other **630 are not attributed** to a
subgroup by any file in the repository. (INV `:57` counts 748 unclassified =
2,231 − 1,483; this file takes the derivate 103 and the texture_functions 15
out of that, 748 − 103 − 15 = 630.)

## 2. Subgroup table

"mustpass" counts `dEQP-GLES3.functional.shaders.<subgroup>.*` names in
`tools/cts/gles3-mustpass.txt`; the column sums to 20,616. "AM62P fails" is
exact only where it says so; "≤ N (img k, shared)" is a bound that covers all
the subgroups of image k together. Bucket messages are quoted from R-bucket.
"Source" is where the cases come from: **L** a `.test` file under **data**,
**G** a C++ generator under **dEQP** (the third-level split is in §4).
"Example" is a failing case where a bucket or a source identifies one;
otherwise it is the subgroup's first mustpass name, marked (first), because no
failing example is recorded. Classes: **missing GL API**, **compiler
refusal**, **wrong result** (back-end codegen), **precision**, **unknown**.

| subgroup | mustpass | AM62P fails (396d667a) | bucket (R-bucket line) | source | suspected cause (class) | example case | host test that would reproduce it |
|---|---:|---|---|---|---|---|---|
| arrays | 308 | **0** exact (image 6 arithmetic, §1; `RESULTS.md:1557`, `:1480-1485`) | — | L `arrays.test` | — (COV `:61`: 294 built, 0 refused) | — | `tests/es3_shaderc_corpus.rs` (compile and link only) |
| builtin_functions | 1,539 | **≥ 1,461** exact lower bound (R-work `:989`); ≤ 1,740 (image 7's 1,762 − builtin_variable's 22, `:1558`) | "Failed to set up framebuffer object: GL_INVALID_ENUM (glsShaderExecUtil.cpp:670)" 1,483 (`:1501`) | G `es3fShaderCommonFunctionTests.cpp` (common), `es3fShaderPackingFunctionTests.cpp` (pack_unpack), `es3fBuiltinPrecisionTests.cpp` → glshared `glsBuiltinPrecisionTests.cpp:4953` (precision); every case runs through the shader executor | **missing GL API**: the executor renders each output into an integer renderbuffer, R32I .. RGBA32UI (`getRenderbufferFormatForOutput`, glshared `glsShaderExecUtil.cpp:564`, used at `:663-666`), and `glRenderbufferStorage` refused the integer formats. CAUSE SHOWN in code (`:1501`, `:989`). After that gap the 30 `modf` cases (common 24, precision 6; `es3fShaderCommonFunctionTests.cpp:718` declares `out1`) need two draw buffers (`glsShaderExecUtil.cpp:674-677`); the other 1,509 need one | `builtin_functions.common.abs.float_highp_fragment` (first; any case of the group hits the executor set-up) | `tests/es3_int_rt.rs` (the GL half, item int-rt-gl, merged); `MODULE=gles3 tools/cts/host-deqp-run.sh builtin_functions` (the set-up error is an API verdict). Pixels need the board |
| builtin_variable | 29 | **≥ 22** exact lower bound (R-work `:989`); ≤ 29 | same bucket (`:1501`) | G `es3fShaderBuiltinVarTests.cpp`: the 22 `max_*`/`min_*` cases (11 constants × vertex/fragment, `:1144-1146`) use the executor (`createGetConstantExecutor`, `:119-131`); depth_range, vertex_id, fragcoord, pointcoord, frontfacing (7) render | **missing GL API** (as above) for the 22. The 7 render cases: not in any bucket; unknown | `builtin_variable.max_draw_buffers_fragment` | as above, filter `builtin_variable` |
| conditionals | 41 | unknown; ≤ 279 with constant_expressions (image 7: 1,762 − 1,483) | none of the 16 names it | L `conditionals.test` | unknown (COV `:62`: 21 built, 20 refused as expected) | `conditionals.if.complex_condition_fragment` (first) | `tests/es3_shaderc_corpus.rs` |
| constant_expressions | 340 | unknown (image 7 bound above) | none of the 16 names it | G `es3fShaderConstExprTests.cpp:46` (builtin_functions, 302) + L `constant_expressions.test` loaded at `:345-347` (38) | unknown. COV `:63`, `:132` refuses 2 programs, `other.complex_*` (`gl_MaxDrawBuffers` folded below 4, `constant_expressions.test:452`), but **that case is not in the mustpass**: the only `other` names are `other.switch_case_{fragment,vertex}` (`.test:352`); so the refusal explains none of the 340. `gl_MaxDrawBuffers` folds to `MAX_DRAW_BUFFERS` = 3 at the tip (`src/shaderc/online/reflect.rs:1262`, `src/gl/context.rs:139,152`); COV `:150` measured 1 at 3f78982e; not reconciled here | `constant_expressions.builtin_functions.angle_and_trigonometry.acos_float_fragment` (first) | `tests/es3_shaderc_corpus.rs` (the `.test` part); host-deqp-run.sh `constant_expressions` (the C++ part) |
| constants | 143 | unknown; ≤ 225 with the other image 8 subgroups except derivate (328 − 103, `:1559`) | none of the 16 | L `constants.test` | unknown (COV `:64`: 0 refused) | `constants.bool_0_fragment` (first) | `tests/es3_shaderc_corpus.rs` |
| conversions | 1,156 | unknown (image 8 bound) | none of the 16 | L `conversions.test` | unknown on the AM62P (COV `:65`: 0 refused on PowerVR; on V3D 108 refused, matrix attributes, COV `:103`, `:137`) | `conversions.matrix_combine.bool_bool_bool_bool_bool_bool_to_mat2x3_fragment` (first) | `tests/es3_shaderc_corpus.rs` |
| declarations | 50 | unknown (image 8 bound) | none of the 16 | L `declarations.test` (all `invalid_declarations`: the compile must fail) | unknown (COV `:66`: all 54 programs refused as expected) | `declarations.invalid_declarations.in_in_fragment_main` (first) | `tests/es3_shaderc_corpus.rs` |
| derivate | 282 | **103** exact (`:1516`); plus an unknown share of "Fail (no message)" 133 (`:1515`) | "checkFramebufferStatus == COMPLETE (es3fShaderDerivateTests)" 103 (`:1516`); "Fail (no message)" (`:1515`, share unknown) | G `es3fShaderDerivateTests.cpp` | **missing GL API**, two gaps in one check (`:757`): (a) the `fbo_float` and `texture.float_*` surfaces are an **RGBA32UI** renderbuffer (`:92`, `:732`, configs `:1796`, `:1826-1827`), the integer render-target gap, 48 + 20 + 20 = 88 names; (b) the `texture.msaa4` surface is a 4-sample RGBA8 renderbuffer (`:754`, `:1825`) and MAX_SAMPLES is 1 (R-host `:2008`), 15 names. 88 + 15 = **103 = the bucket**, by name count (the same cases is a reading, not shown). `.fbo.` cases (13) use the default framebuffer (`:1793`). R-bucket's "float render targets" and INV `:121-129` (RGBA32F/16F, EXT_color_buffer_float) misread the surface: the RGBA32F/RGBA16F at `:1411` is the *source texture* of `texture.*`, not the target | `derivate.dfdx.fastest.fbo_float.float_lowp` (RGBA32UI); `derivate.dfdx.texture.msaa4.float_mediump` (4 samples) | host-deqp-run.sh `derivate` (completeness is an API verdict); `tests/es3_int_rt.rs` for RGBA32UI completeness; a MAX_SAMPLES test does not exist |
| discard | 25 | unknown (image 8 bound) | none of the 16 | G `es3fShaderDiscardTests.cpp` | unknown | `discard.basic_always` (first) | host-deqp-run.sh `shaders.discard` (compile only; the verdict is a picture) |
| fragdata | 7 | unknown (image 8 bound) | none of the 16 | G `es3fShaderFragDataTests.cpp` (`valid_*`, 3) + L `fragdata.test` (4, loaded at `:371-372`) | unknown. The 3 `valid_*` cases draw into two colour attachments with `glDrawBuffers` (`:295-302`), so they depend on item mrt4 if `MAX_DRAW_BUFFERS` was below 2 at 396d667a (not recorded; the ES2 analogue is `RESULTS.md:1638`). COV `:67`: the 4 `.test` programs refused as expected | `fragdata.valid_static_index` | host-deqp-run.sh `fragdata` (the draw-buffers error is an API verdict) |
| fragdepth | 17 | unknown (image 8 bound) | none of the 16 | G `es3fFragDepthTests.cpp:393` | unknown | `fragdepth.compare.const` (first) | host-deqp-run.sh `fragdepth` (compile only) |
| function | 4 | unknown (image 8 bound) | none of the 16 | G `es3fShaderFunctionTests.cpp:166` | unknown | `function.global_variable_aliasing_fragment` (first) | host-deqp-run.sh `shaders.function.` |
| functions | 305 | unknown (image 8 bound) | none of the 16 | L `functions.test` | unknown (COV `:68`: 0 refused on PowerVR; recursion cases refused by the inliner's depth bound, COV `:189-190`) | `functions.array_arguments.copy_global_in_on_call_fragment` (first) | `tests/es3_shaderc_corpus.rs` |
| indexing | 756 | unknown; ≤ 16 with the other image 9 subgroups (`:1560`) | none of the 16 | G `es3fShaderIndexingTests.cpp` (728) + L `indexing.test` (moredynamic 28, loaded at `:1256-1258`) | **compiler refusal** possible for the 2 `moredynamic.user_defined_fncall_out_parameter_*` names (8 programs: "lowering into several groups", COV `:69`, `:134`); the rest unknown | `indexing.moredynamic.user_defined_fncall_out_parameter_fragment` | `tests/es3_shaderc_corpus.rs` (moredynamic); host-deqp-run.sh `indexing` |
| invalid_implicit_conversions | 768 | unknown (image 9 bound ≤ 16) | none of the 16 | L `invalid_implicit_conversions.test` (the compile must fail) | unknown (COV `:70`: all 768 refused as expected) | `invalid_implicit_conversions.invalid_implicit_conversions.add_float_int_float_fragment` (first) | `tests/es3_shaderc_corpus.rs` |
| invariance | 36 | unknown (image 9 bound) | none of the 16 | G `es3fShaderInvarianceTests.cpp` | unknown | `invariance.highp.common_subexpression_0` (first) | board (a picture comparison) |
| keywords | 282 | unknown (image 9 bound) | none of the 16 | L `keywords.test` | unknown (COV `:72`: 0 refused) | `keywords.allowed_keywords.image1DArrayShadow_fragment` (first) | `tests/es3_shaderc_corpus.rs` |
| large_constant_arrays | 16 | unknown (image 9 bound) | none of the 16 | L `large_constant_arrays.test` | unknown on PowerVR (COV `:73`: 0 refused; GC7000UL refuses 2, COV `:139`) | `large_constant_arrays.indexing.float_128_fragment` (first) | `tests/es3_shaderc_corpus.rs` |
| linkage | 114 | unknown (image 9 bound) | none of the 16 | L `linkage.test` | **compiler refusal** of `centroid in` (1 mustpass name, `varying.interpolation.centroid`; COV `:74`, `:133`; item pvr-centroid) and **missing link check** for 7 interface-mismatch cases wrongly accepted at 3f78982e (COV `:167-168`; fixed by item link-iface, merged) | `linkage.varying.interpolation.centroid` | `tests/es3_shaderc_corpus.rs`, `tests/es3_link_iface.rs` |
| loops | 579 | **0** exact (image 10: 1,425 / 0, `:1561`) | — | G `es3fShaderLoopTests.cpp` (570) + L `loops.test` (custom 1, short_circuit 8; loaded at `:1238-1240`) | — | — | — |
| matrix | 2,646 | **0** exact (images 10 and 11 both 0, `:1561-1562`) | — | G `es3fShaderMatrixTests.cpp` | — | — | — |
| metamorphic | 6 | **0** exact (image 11) | — | G `es3fShaderMetamorphicTests.cpp` (reads `graphicsfuzz/*.frag` resources, `:100`, `:355`) | — | — | — |
| negative | 12 | **0** exact (image 11) | — | L `negative.test` | — | — | — |
| operator | 6,402 | unknown; **0** exact for angle_and_trigonometry and binary_operator up to `right_shift_assign_effect` (images 12 and 13: 0 fails, `:1563-1564`); ≤ 24 for the rest of binary_operator plus bool_compare, common_functions, exponential, float_compare, geometric, int_compare, selection and sequence (image 14, `:1565`); unary_operator ≤ 71 with image 15 (`:1566`) | "Failed to compile shader program (glsShaderRenderCase)" 189: "shaders.operator (trunc ...)" (`:1512`; share unknown) | G `es3fShaderOperatorTests.cpp` | **compiler refusal** of `trunc` (`:1512`). The mustpass has 24 `operator.common_functions.trunc.*` names, equal to image 14's 24 fails; not shown to be the same cases | `operator.common_functions.trunc.highp_vec2_fragment` | host-deqp-run.sh `operator.common_functions.trunc` |
| precision | 64 | unknown; ≤ 71 with unary_operator, preprocessor and qualification_order (image 15) | none of the 16 | G `es3fShaderPrecisionTests.cpp:830` | **missing GL API** likely: every case renders into an R32UI or R32I renderbuffer (`:264`, `:521`, `:716`; the float cases pack into R32UI, `:23`), the integer render-target gap again. No listed bucket names it, so its message is among the 62 unlisted (§5 names it HOST-ONLY). R-host `:2001`: 0 of 64 refused at compile | `precision.float.highp_add_fragment` (first) | host-deqp-run.sh `shaders.precision` (the set-up error is an API verdict); `tests/es3_int_rt.rs` (R32I/R32UI storage) |
| preprocessor | 546 | unknown (image 15 bound) | none of the 16 | L `preprocessor.test` | **compiler refusal** for 2 `line_continuation` names (`preprocessing_token_*`, `token_*`: 4 programs, COV `:77`, `:135`) at 3f78982e; fixed by item pp-splice (merged) | `preprocessor.line_continuation.preprocessing_token_fragment` | `tests/es3_shaderc_corpus.rs`, `tests/es3_shaderc_pp.rs` |
| qualification_order | 79 | unknown (image 15 bound) | none of the 16 | L `qualification_order.test` | **compiler refusal** of 7 `centroid in` programs on PowerVR (COV `:78`, `:133`, `:179-181`: the `variables.invalid.*_invariant_input` cases, 7 mustpass names, are refused for `centroid`, not for `invariant`; item pvr-centroid) | `qualification_order.variables.invalid.interp_storage_invariant_input` | `tests/es3_shaderc_corpus.rs` |
| random | 1,823 | unknown; ≤ 32 with return and scoping (image 16, `:1567`) | "Image comparison failed" 675 lists shaders.random (`:1504`; share unknown) | G `es3fRandomShaderTests.cpp` | **wrong result (back-end codegen)** or **precision**: a rendered picture differs. Which of the nine random subgroups is not recorded | `random.all_features.fragment.0` (first) | board only (the host renders black, `tools/cts/host-deqp-run.sh` header). ES 2.0 form of a repro: `tests/deqp_repro_shaders.rs` |
| return | 38 | unknown (image 16 bound) | none of the 16 | G `es3fShaderReturnTests.cpp` | unknown | `return.conditional_return_always_fragment` (first) | board |
| scoping | 71 | unknown (image 16 bound) | none of the 16 | L `scoping.test` | unknown (COV `:79`: 0 refused; two invalid cases refused by the lowering, not sema, COV `:182-188`) | `scoping.invalid.conflict_function_struct_fragment` (first) | `tests/es3_shaderc_corpus.rs` |
| struct | 89 | **0** exact (image 17: 1,466 / 0, `:1568`) | — | G `es3fShaderStructTests.cpp` | — | — | — |
| switch | 173 | **0** exact (image 17) | — | G `es3fShaderSwitchTests.cpp` (121) + L `switch.test` (52 programs, COV `:80`; loaded at `:480-481`) | — | — | — |
| swizzle_math_operations | 544 | **0** exact (image 17) | — | L `swizzle_math_operations.test` | — | — | — |
| swizzles | 648 | **0** exact (image 17) | — | L `swizzles.test` | — | — | — |
| texture_functions | 560 | **≥ 15** exact lower bound (159 − 144, `:1514`, `:1518`); ≤ 453 with uniform_block and texture.compressed (image 18, §1) | "Texture upload failed: GL_INVALID_OPERATION (gluTexture)" 522 (isampler/usampler, `:1506`); "Failed to compile shader program (glsShaderRenderCase)" 189 (sampler3D bias, `:1512`); "Texture upload failed: GL_INVALID_ENUM (gluTexture)" 159 (`*shadow`, `:1514`; 15 here); "Fail (no message)" 133 (`:1515`). Shares of the first, second and fourth unknown | G `es3fShaderTextureFunctionTests.cpp` (442) + L `invalid_texture_functions.test` (`invalid`, 118; loaded at `:2975-2976`) | **missing GL API**: integer texture formats refused at upload (186 mustpass names outside `invalid` use `isampler`/`usampler`) and depth formats (42 names use a shadow sampler; 15 failed that way). **compiler refusal**: bias on sampler3D (12 `sampler3d_bias_*` names) and the shadow forms (R-work `:998`: 35 shadow forms + 2 `sampler2darrayshadow` at a1f88596, not 396d667a; text at `src/shaderc/online/powervr/select.rs:1165`). **wrong result**: 2D arrays sampled as one stacked image (82 `2darray` names; R-work `:995`). R-host `:1987-1989`: "ES3 texture look-ups 1,167" and "textureSize 38" refused at 008b4652 (HOST-ONLY, all groups) | `texture_functions.texture.isampler2d_fragment`; `texture_functions.texture.sampler3d_bias_float_fragment`; `texture_functions.texture.sampler2dshadow_fragment` | host-deqp-run.sh `texture_functions` (upload errors and refusals are API/compile verdicts); `tests/es3_sized_formats.rs` (upload, item sized-formats-gl, merged) |
| uniform_block | 118 | unknown (image 18 bound) | none of the 16 | L `uniform_block.test` | **compiler/link refusal** of the 2 `valid.long_instance_name_*` names (64-byte reflection name, COV `:83`, `:136`); the rest unknown | `uniform_block.valid.long_instance_name_fragment` | `tests/es3_shaderc_corpus.rs` |
| **total** | **20,616** | **2,231** (R-group `:1486`; STATUS `:250`) | | | | | |

Notes on the table:

- COV counts programs (a `both` case gives two), measured at 3f78982e, after
  the board run. A COV refusal is a candidate cause at `396d667a`, not a shown
  one: the front end changed between the two (pp-splice and link-iface landed
  after the run), so COV cannot give `396d667a` counts.
- The 189 bucket (`:1512`) names operator (trunc) and texture_functions
  (sampler3D bias) and is not split; the names it cites cover at most 24 + 12
  = 36 of its 189, so it holds more than those two readings. The 133 "no
  message" bucket (`:1515`) spans attribute_location, instanced,
  rasterizer_discard, derivate and texture_functions and is not split either.
- None of the 16 listed messages names conditionals, constants, conversions,
  declarations, discard, fragdata, fragdepth, function, functions, indexing,
  invalid_implicit_conversions, invariance, keywords, large_constant_arrays,
  linkage, precision, preprocessor, qualification_order, return, scoping or
  uniform_block. Their failures, if any, are among the 62 unlisted messages
  (1,305 cases over all groups), or in the shared 133 / 675 buckets.

## 3. Root causes ranked by AM62P fail count

Only firm counts are ranked. The order within the unattributed 630 is not
known.

1. **Integer colour render targets (missing GL API, then back end): 1,483
   exact + 88 by name, and 15 four-sample renderbuffers.** 1,461
   builtin_functions + 22 builtin_variable (R-work `:989`, bucket `:1501`,
   CAUSE SHOWN) + derivate's 103 (`:1516`), which the names split into 88
   RGBA32UI surfaces and 15 `texture.msaa4` surfaces (§2, derivate row). So
   ≥ 1,571 on integer targets and 15 on MAX_SAMPLES = 1, 1,586 together
   (71.1 % of 2,231). The GL half is done on the host (item int-rt-gl,
   `tests/es3_int_rt.rs`); the back-end halves are int-rt-pvr, int-rt-v3d and
   int-rt-gc; the 30 `modf` cases also need two draw buffers (item mrt4).
   shaders.precision (64, R32I/R32UI renderbuffers) is probably the same gap,
   with no board count (§5 gives a HOST-ONLY one). No item covers the
   4-sample surface (INV RC8 lists MSAA as "no item yet").
2. **texture_functions: integer and depth upload, shadow and 3D-bias
   refusals, 2D-array sampling: ≥ 15 exact, ≤ 453 (shared).** Buckets
   `:1506`, `:1512`, `:1514`, `:1515`. Plan items: sized-formats-gl (done, GL
   core), formats-pvr (sampling), pvr-shadow, pvr-2darray.
3. **Unattributed: 630 = 2,231 − 1,601.** Located by image, not counted:
   image 7 rest (conditionals, constant_expressions, the builtin_* cases past
   the bucket) ≤ 279; image 8 rest ≤ 225; image 15 (unary_operator,
   precision, preprocessor, qualification_order) ≤ 71; image 16 (random:
   "Image comparison failed", wrong result; return; scoping) ≤ 32; image 14
   (operator, `trunc` refusal per `:1512`) ≤ 24; image 9 ≤ 16; image 18
   (texture_functions beyond the 15, uniform_block) ≤ 438. The bounds include
   non-mustpass cases (§1) and sum to more than 630.
4. **Exactly zero:** arrays, loops, matrix, metamorphic, negative, struct,
   switch, swizzle_math_operations, swizzles (308 + 579 + 2,646 + 6 + 12 + 89
   + 173 + 544 + 648 = 5,005 names), plus operator.angle_and_trigonometry and
   the binary_operator subgroups of images 12 and 13.

## 4. Third level: case counts and sources

Mustpass names per `shaders.<subgroup>.<child>`. Subgroups whose cases sit
directly under them are marked "leaf". Each row's children sum to the
subgroup's count in §2. L = `.test` under **data**, G = C++ under **dEQP**.
Fail counts at this level are not recorded anywhere in the repository.

| subgroup | children (mustpass names) | source |
|---|---|---|
| arrays (308) | compare 180, complex_expression 8, constructor 28, declaration 26, invalid 18, length 8, return 20, unnamed_parameter 20 | L arrays.test |
| builtin_functions (1,539) | common 372, pack_unpack 20, precision 1,147 | G es3fShaderCommonFunctionTests / es3fShaderPackingFunctionTests / es3fBuiltinPrecisionTests (`es3fFunctionalTests.cpp:182-184`) |
| builtin_variable (29) | leaf 29 (22 `max_*`/`min_*` executor cases + depth_range 2, fragcoord 2, frontfacing, pointcoord, vertex_id) | G es3fShaderBuiltinVarTests |
| conditionals (41) | if 21, invalid_if 20 | L conditionals.test |
| constant_expressions (340) | builtin_functions 302 (G es3fShaderConstExprTests); complex_types 10, operators 18, other 2, trivial 8 (L constant_expressions.test) | G + L |
| constants (143) | leaf 143 | L constants.test |
| conversions (1,156) | matrix_combine 138, matrix_to_matrix 162, scalar_to_matrix 72, scalar_to_scalar 32, scalar_to_vector 96, vector_combine 272, vector_illegal 96, vector_to_scalar 96, vector_to_vector 192 | L conversions.test |
| declarations (50) | invalid_declarations 50 | L declarations.test |
| derivate (282) | dfdx 82, dfdy 98, fwidth 102. By surface: default framebuffer (`constant`, `linear`, `in_function`, `static_*`, `uniform_*`, `.fbo.`, `fastest/nicest.default`, `texture.basic`) 179; RGBA32UI (`fbo_float` 48, `texture.float_fastest` 20, `texture.float_nicest` 20) 88; 4-sample RGBA8 (`texture.msaa4`) 15 | G es3fShaderDerivateTests (`:1791-1827`) |
| discard (25) | leaf 25 | G es3fShaderDiscardTests |
| fragdata (7) | leaf 7: valid_{dynamic,static,uniform}_index (G es3fShaderFragDataTests); invalid_assign_to_1, write_fragcolor_and_fragdata_{simple,static_if,unused_func} (L fragdata.test) | G + L |
| fragdepth (17) | compare 9, write 8 | G es3fFragDepthTests |
| function (4) | leaf 4 | G es3fShaderFunctionTests |
| functions (305) | array_arguments 22, control_flow 22, datatypes 61, declarations 16, invalid 108, misc 8, overloading 16, qualifiers 52 | L functions.test |
| indexing (756) | matrix_subscript 288, moredynamic 28 (L indexing.test), tmp_array 128, uniform_array 32, varying_array 64, vector_subscript 216 | G es3fShaderIndexingTests + L |
| invalid_implicit_conversions (768) | invalid_implicit_conversions 768 | L invalid_implicit_conversions.test |
| invariance (36) | highp 12, lowp 12, mediump 12 | G es3fShaderInvarianceTests |
| keywords (282) | allowed_keywords 8, invalid_identifiers 6, keywords 146, reserved_keywords 122 | L keywords.test |
| large_constant_arrays (16) | indexing 16 | L large_constant_arrays.test |
| linkage (114) | uniform 39, varying 75 | L linkage.test |
| loops (579) | custom 1 (L), do_while_constant_iterations 62, do_while_dynamic_iterations 62, do_while_uniform_iterations 62, for_constant_iterations 64, for_dynamic_iterations 64, for_uniform_iterations 64, short_circuit 8 (L), while_constant_iterations 64, while_dynamic_iterations 64, while_uniform_iterations 64 | G es3fShaderLoopTests + L loops.test |
| matrix (2,646) | add 324, add_assign 54, determinant 18, div 324, div_assign 54, inverse 18, matrixcompmult 54, mul 972, mul_assign 18, negation 54, outerproduct 54, post_decrement 54, post_increment 54, pre_decrement 54, pre_increment 54, sub 324, sub_assign 54, transpose 54, unary_addition 54 | G es3fShaderMatrixTests |
| metamorphic (6) | bubblesort_flag 2, synthetic 4 | G es3fShaderMetamorphicTests |
| negative (12) | c_cast 12 | L negative.test |
| operator (6,402) | angle_and_trigonometry 284, binary_operator 4,126, bool_compare 30, common_functions 724, exponential 112, float_compare 108, geometric 116, int_compare 108, selection 80, sequence 48, unary_operator 666 | G es3fShaderOperatorTests |
| precision (64) | float 16, int 24, uint 24 | G es3fShaderPrecisionTests |
| preprocessor (546) | basic 18, builtin 16, comments 16, conditional_inclusion 18, conditionals 18, definitions 14, directive 24, expressions 52, extensions 26, function_definitions 6, function_redefinitions 12, invalid_conditionals 58, invalid_definitions 6, invalid_expressions 16, invalid_function_definitions 32, invalid_ops 18, invalid_redefinitions 2, line_continuation 10, object_redefinitions 30, operator_precedence 90, pragmas 8, predefined_macros 14, recursion 10, semantic 4, undefined_identifiers 28 | L preprocessor.test |
| qualification_order (79) | parameters 24, variables 55 | L qualification_order.test |
| random (1,823) | all_features 136, basic_expression 300, comparison_ops 100, conditionals 300, exponential 200, scalar_conversion 298, swizzle 100, texture 199, trigonometric 190 (by stage: fragment 810, vertex 714, combined 299) | G es3fRandomShaderTests |
| return (38) | leaf 38 | G es3fShaderReturnTests |
| scoping (71) | invalid 40, valid 31 | L scoping.test |
| struct (89) | local 58, uniform 31 | G es3fShaderStructTests |
| switch (173) | leaf 173 (52 from L switch.test, 121 from G es3fShaderSwitchTests) | G + L |
| swizzle_math_operations (544) | vector_add 136, vector_divide 136, vector_multiply 136, vector_subtract 136 | L swizzle_math_operations.test |
| swizzles (648) | vector_swizzles 648 | L swizzles.test |
| texture_functions (560) | invalid 118 (L invalid_texture_functions.test), texelfetch 24, texelfetchoffset 24, texture 55, texturegrad 35, texturegradoffset 28, texturelod 22, texturelodoffset 23, textureoffset 34, textureproj 39, textureprojgrad 26, textureprojgradoffset 26, textureprojlod 14, textureprojlodoffset 23, textureprojoffset 31, texturesize 38 (6 of them shadow samplers) | G es3fShaderTextureFunctionTests + L |
| uniform_block (118) | invalid 64, valid 54 | L uniform_block.test |

All 23 `.test` files under **data** are used: 17 by `ShaderLibraryTest`
(`es3fFunctionalTests.cpp:197-213`), and fragdata, indexing, loops, switch,
constant_expressions and invalid_texture_functions by the C++ groups that load
them (cited in §2).

## 5. HOST-ONLY cross-check: the host run on the review branch

HOST3 (`docs/es3-host-run.md` at `f9916ec`, branch
`ccfarm/01a11dcf-05a5-759b-ac85-6632d0ce2cd8`, not merged when this was
written) ran the whole mustpass on the host for the three GPUs with the
recording OSAL: API errors and compile/link verdicts are real, every picture
verdict is black and meaningless. Nothing below is an AM62P figure; it is
listed because it says which §2 "unknown" rows are API or compile failures.
Re-check against the merged file; the line numbers are those of `f9916ec`.

- The 1,483 executor bucket: **1,483 on the PowerVR host** (HOST3 "sixteen
  largest" table, row 1), the same count as the board. On the V3D build 639,
  because the V3D back end refuses many of those programs first.
- derivate's 103: **103 on all three GPUs** ("SAME on every GPU"). The
  message is dEQP's completeness check; HOST3 repeats R-bucket's "float render
  targets" reading, which §2 corrects.
- texture_functions upload: the 522 GL_INVALID_OPERATION bucket **522 on
  every GPU**; the 159 GL_INVALID_ENUM (depth formats) bucket **0 on the
  host** ("GONE": depth formats upload now; whether that is a change since
  396d667a or a host-path difference is not separated there).
- The 189 glsShaderRenderCase compile bucket: 187 PowerVR, 178 GC7000, 4,216
  V3D. HOST3 reads the PowerVR 187 as modf's "lowering into several groups"
  (59 + 59 refusals) and the 3D bias.
- **shaders.precision: "Post framebuffer setup: GL_INVALID_ENUM
  (es3fShaderPrecisionTests)" 40 PowerVR / 64 GC7000 / 16 V3D** (HOST3 "API
  messages below §1r's cut"). This is the R32UI/R32I renderbuffer of §2's
  precision row, confirmed as an API refusal on the host; on the board it is
  one of the 62 unlisted messages.
- PowerVR compile refusals on failing cases (HOST3 "Compile refusals"): the
  texture-fetch catch-all 277 + 85 (shadow, 3D bias, integer result,
  gradients; "shadow compare is no longer the cause in texture.shadow"),
  "lowering into several groups" 59 + 59, `FragmentInputsNotBuilt` 57
  (linkage `varying.interpolation.centroid` and
  `rules.differing_interpolation_*`), "exact gradients without the W plane" 2
  (sampler2dshadow / samplercubeshadow), the 63-character uniform name 1 + 1
  (uniform_block `long_instance_name_*`).
- Host-only V3D refusals that this map does not count (no Pi 4 GLES3 run):
  3,427 integer/trunc ALU ops, 1,052 matrix attributes, 888 non-2D/cube
  look-ups, 319 shadow look-ups, 133 integer look-ups (HOST3 "V3D" table).

## 6. What would close the gaps in this document

- The per-case tsv that `docs/board-sessions/am62p-gles3-percase.md:81-121`
  describes (case, verdict, first message, image), from a board session with
  the console on the right UART (`:46-79`). With it every "unknown" above
  becomes a count. Items oq-gc and board-am62p run the full tree.
- Merging HOST3 turns §5 into citable per-case API and compile verdicts for
  the generated groups COV cannot reach (builtin_functions, derivate,
  texture_functions, operator, precision). It cannot judge pictures (random,
  invariance, return, discard, fragdepth).
- The derivate reading in INV `:121-129` (RGBA32F/RGBA16F,
  EXT_color_buffer_float) and R-bucket `:1516` ("float render targets") should
  follow §2: the target is RGBA32UI (int-rt-*), plus 15 four-sample surfaces
  (no item).
