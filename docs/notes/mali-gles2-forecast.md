<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the expected dEQP-GLES2 refusals, by mustpass group

Date 2026-11-07. Plan #327 item `mali-gles2-forecast`, for the first
dEQP-GLES2 run of the `gpu-mali` back end on the NanoPC-T6 (issues #20 and
#7). A forecast made before any board result, from the source alone: no board
run, `gh` was not run, no file but this note changed.

**Derived from main `0a922ae299ce`** (`tools/cts/imx8mp-gles3-merge.py merges
Armadillo-X2 three-word rows`). `src/mali/`, `src/shaderc/online/mali/` and
`tools/cts/gles2-mustpass.txt` are byte-identical in the main tip `81bfac76`
that `0a922ae2` is an ancestor of, so the counts hold there too. When
`src/mali/**` or `src/shaderc/online/mali/**` move, the causes (section 2)
move with them; the counts are only as old as that commit.

## What this is for

Plan #309 owns the back end's *known refusals*: what the Mali back end or its
shader compiler rejects on purpose with a named error. The board results will
show many non-Pass cases. This note lets the tally tell (a) a refusal that is
already known and owned from (b) a new failure cause that needs a new issue
(section 4). It is a forecast, not a measurement: it says which *class* of
refusal a group will hit, not which of the compiler's ~60 `MaliError` variants
the first refusal will be (the compiler stops at the first gap). Every count
below is the count of mustpass names matched by the rules of section 5; the
rules are what the forecast stands on, and the board decides.

## 1. By mustpass group

Group level as the tree tables use it: the top-level group, and
`fragment_ops.*`, `shaders.*`, `texture.*` by their second level. 17,165
cases, 76 groups. Columns: **pass** = likely pass; **refused** = a named
refusal is expected (the causes of section 2); **hang** = hang-risk
(section 3); **fail** = a known FAIL, not a refusal (section 2, cause
`MATRIX_ATTRIB`).

| group | cases | pass | refused | hang | fail | refused by cause |
|---|---:|---:|---:|---:|---:|---|
| `attribute_location` | 57 | 33 | 0 | 0 | 24 | |
| `buffer` | 49 | 49 | 0 | 0 | 0 | |
| `clip_control` | 8 | 8 | 0 | 0 | 0 | |
| `clipping` | 602 | 602 | 0 | 0 | 0 | |
| `color_clear` | 19 | 19 | 0 | 0 | 0 | |
| `debug_marker` | 3 | 3 | 0 | 0 | 0 | |
| `default_vertex_attrib` | 40 | 24 | 0 | 0 | 16 | |
| `depth_range` | 28 | 28 | 0 | 0 | 0 | |
| `depth_stencil_clear` | 11 | 11 | 0 | 0 | 0 | |
| `dither` | 20 | 20 | 0 | 0 | 0 | |
| `draw` | 101 | 101 | 0 | 0 | 0 | |
| `extensions` | 1 | 1 | 0 | 0 | 0 | |
| `fbo` | 709 | 709 | 0 | 0 | 0 | |
| `flush_finish` | 5 | 5 | 0 | 0 | 0 | |
| `fragment_ops.blend` | 1060 | 215 | 845 | 0 | 0 | BLEND_SHADER 845 |
| `fragment_ops.depth` | 8 | 8 | 0 | 0 | 0 | |
| `fragment_ops.depth_stencil` | 621 | 621 | 0 | 0 | 0 | |
| `fragment_ops.interaction` | 100 | 100 | 0 | 0 | 0 | |
| `fragment_ops.random` | 100 | 100 | 0 | 0 | 0 | |
| `fragment_ops.scissor` | 17 | 17 | 0 | 0 | 0 | |
| `fragment_ops.stencil` | 17 | 17 | 0 | 0 | 0 | |
| `implementation_limits` | 16 | 16 | 0 | 0 | 0 | |
| `lifetime` | 33 | 33 | 0 | 0 | 0 | |
| `light_amount` | 19 | 0 | 19 | 0 | 0 | ARITH 19 |
| `multisample` | 16 | 16 | 0 | 0 | 0 | |
| `multisampled_render_to_texture` | 1 | 1 | 0 | 0 | 0 | |
| `negative_api` | 246 | 246 | 0 | 0 | 0 | |
| `polygon_offset` | 10 | 10 | 0 | 0 | 0 | |
| `prerequisite` | 3 | 3 | 0 | 0 | 0 | |
| `rasterization` | 52 | 52 | 0 | 0 | 0 | |
| `read_pixels` | 8 | 8 | 0 | 0 | 0 | |
| `render_target` | 1 | 1 | 0 | 0 | 0 | |
| `renderer` | 1 | 1 | 0 | 0 | 0 | |
| `shader_api` | 38 | 38 | 0 | 0 | 0 | |
| `shaders.algorithm` | 4 | 0 | 4 | 0 | 0 | CONTROL_FLOW 4 |
| `shaders.builtin_variable` | 22 | 0 | 22 | 0 | 0 | INT_VALUE 16, SPECIAL_INPUT 4, FS_OPERAND 2 |
| `shaders.conditionals` | 41 | 20 | 21 | 0 | 0 | CONTROL_FLOW 21 |
| `shaders.constant_expressions` | 462 | 0 | 462 | 0 | 0 | ARITH 430, FS_OPERAND 32 |
| `shaders.constants` | 79 | 0 | 79 | 0 | 0 | FS_OPERAND 63, INT_VALUE 16 |
| `shaders.conversions` | 524 | 54 | 470 | 0 | 0 | INT_VALUE 400, FS_OPERAND 70 |
| `shaders.declarations` | 6 | 6 | 0 | 0 | 0 | |
| `shaders.discard` | 25 | 0 | 25 | 0 | 0 | DISCARD_FORM 25 |
| `shaders.fragdata` | 7 | 1 | 6 | 0 | 0 | FRAG_OUTPUT 6 |
| `shaders.function` | 4 | 0 | 4 | 0 | 0 | CONTROL_FLOW 4 |
| `shaders.functions` | 256 | 83 | 173 | 0 | 0 | FS_OPERAND 131, CONTROL_FLOW 22, INT_VALUE 16, ARITH 4 |
| `shaders.indexing` | 568 | 0 | 568 | 0 | 0 | INDEXED_LOCAL 472, VARYING_FORM 64, UNIFORM_INDEXED 32 |
| `shaders.invalid_implicit_conversions` | 256 | 256 | 0 | 0 | 0 | |
| `shaders.invariance` | 36 | 0 | 36 | 0 | 0 | INVARIANT 36 |
| `shaders.keywords` | 95 | 95 | 0 | 0 | 0 | |
| `shaders.linkage` | 49 | 23 | 26 | 0 | 0 | UNIFORM_TYPE 26 |
| `shaders.loops` | 625 | 0 | 417 | 208 | 0 | LOOP_VERTEX 312, LOOP_FORM 105 |
| `shaders.matrix` | 738 | 0 | 738 | 0 | 0 | ARITH 618, UNIFORM_TYPE 120 |
| `shaders.misc` | 2 | 0 | 2 | 0 | 0 | ARITH 2 |
| `shaders.operator` | 2949 | 0 | 2949 | 0 | 0 | ARITH 1792, INT_VALUE 1050, CONTROL_FLOW 107 |
| `shaders.preprocessor` | 516 | 516 | 0 | 0 | 0 | |
| `shaders.qualification_order` | 34 | 23 | 11 | 0 | 0 | FS_OPERAND 11 |
| `shaders.random` | 1898 | 0 | 1898 | 0 | 0 | ARITH 1098, CONTROL_FLOW 300, INT_VALUE 300, TEX_KIND 200 |
| `shaders.reserved_operators` | 24 | 24 | 0 | 0 | 0 | |
| `shaders.return` | 38 | 0 | 38 | 0 | 0 | CONTROL_FLOW 38 |
| `shaders.scoping` | 76 | 44 | 32 | 0 | 0 | FS_OPERAND 32 |
| `shaders.struct` | 102 | 0 | 102 | 0 | 0 | FS_OPERAND 42, CONTROL_FLOW 36, UNIFORM_TYPE 24 |
| `shaders.swizzles` | 648 | 0 | 648 | 0 | 0 | FS_OPERAND 648 |
| `shaders.texture_functions` | 17 | 1 | 16 | 0 | 0 | TEX_KIND 8, TEX_VERTEX 8 |
| `shading_language_version` | 1 | 1 | 0 | 0 | 0 | |
| `state_query` | 396 | 396 | 0 | 0 | 0 | |
| `texture.completeness` | 40 | 40 | 0 | 0 | 0 | |
| `texture.filtering` | 248 | 248 | 0 | 0 | 0 | |
| `texture.format` | 36 | 36 | 0 | 0 | 0 | |
| `texture.mipmap` | 112 | 88 | 24 | 0 | 0 | TEX_KIND 24 |
| `texture.size` | 74 | 74 | 0 | 0 | 0 | |
| `texture.specification` | 192 | 192 | 0 | 0 | 0 | |
| `texture.vertex` | 90 | 45 | 45 | 0 | 0 | TEX_VERTEX 45 |
| `texture.wrap` | 15 | 15 | 0 | 0 | 0 | |
| `uniform_api` | 1267 | 0 | 1267 | 0 | 0 | UNIFORM_TYPE 919, FS_OPERAND 348 |
| `vendor` | 1 | 1 | 0 | 0 | 0 | |
| `version` | 1 | 1 | 0 | 0 | 0 | |
| `vertex_arrays` | 571 | 571 | 0 | 0 | 0 | |
| **total** | **17165** | **5970** | **10947** | **208** | **40** | |

5970 + 10947 + 208 + 40 = 17165. The "likely pass" column is the weakest
claim of the note. Part of it never reaches the back end: the negative
compile tests the shared front end rejects whatever the GPU (`shaders.
invalid_implicit_conversions`, `keywords`, `reserved_operators`,
`declarations`, the `invalid` names of `preprocessor`, `functions`,
`scoping`, `conditionals`, `linkage`, `qualification_order`, and the
valid preprocessor cases that render a constant colour: 1,146 cases in the
`shaders.*` groups that are pass). The rest is GL-core work with the default dEQP shaders (a
position and a colour or texture coordinate through a varying), where the
sweep's PASS set (`tools/cts/RESULTS.md`, 'Mali', 132 of 256) is the
evidence: clearing, the frame, 2D and cube textures, depth/stencil, culling,
float attributes. It rests on three assumptions no board has checked:

- The GL core converts what the back end does not store natively. The back
  end takes float32 attributes only (`native_attrib_formats`,
  `src/mali/backend.rs`: `NOT_BUILT_ATTRIBUTE`), textures stored as RGBA8,
  sRGB8_A8, R32F or ETC2 (`texel_format`, `NOT_BUILT_TEXTURE_FORMAT`) and
  targets RGBA8888, XRGB8888, RGB565 or RGBA16F
  (`NOT_BUILT_TARGET_FORMAT`). `vertex_arrays` (571, byte/short/fixed
  inputs), `texture.format` (36), `texture.wrap` and `fbo` (RGBA4, RGB5_A1
  renderbuffers) are forecast to pass on the premise that the core widens
  them. If it does not, those groups move to refused with the three messages
  above (they are `src/mali` strings, not `MaliError`).
- The context has no multisampled config, so `multisample` (16) and
  `multisampled_render_to_texture` (1) run single-sampled or are reported
  NotSupported by dEQP, not refused (`NOT_BUILT_SAMPLES` takes 1 or 4).
- The default dEQP shaders of the non-shader groups are inside the compiler's
  references. `light_amount` (lighting arithmetic) is the one non-shader group
  forecast refused for its own shaders.

## 2. The refusal causes

Messages are the exact strings of `MaliError::message`
(`src/shaderc/online/mali/mod.rs`) or the named constant in `src/mali/`.
"Source" is the file and function that produces the error first (a cause may
have more producers: `grep -n 'MaliError::<Variant>'`). A compile refusal is
`-ENOTSUP` at link time, so the GL program fails to link and dEQP reports it
as a link/compile failure; a frame refusal (`mali frame:` and `mali:` strings)
is raised at the draw. The cases count is from section 1; "owner" is plan #309
for every refusal in `src/mali/**` or `src/shaderc/online/mali/**` (its scope),
"none yet" where no plan or issue is known to hold the cause. #309's own item
list was not read (no `gh`): the owner column is the scope, not a checked
assignment.

| cause | message | source | cases | owner |
|---|---|---|---:|---|
| `ARITH` (`MaliError::Arithmetic`) | `Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet` | `src/shaderc/online/mali/nir_fs.rs` `alu32_lanes` (vertex: `vec16.rs` `chunk`) | 3963 | plan #309 |
| `INT_VALUE` (`IntValue`) | `Valhall: an integer or boolean value is not lowered yet` | `src/shaderc/online/mali/nir.rs` `build` (vertex); `nir_fs.rs` `mask_op` (fragment) | 1798 | plan #309 |
| `FS_OPERAND` (`FsOperandForm`) | `Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet` | `src/shaderc/online/mali/nir_fs.rs` `vectorise` | 1379 | plan #309 |
| `UNIFORM_TYPE` (`UniformType`) | `Valhall: a uniform other than a float, vec2, vec3, vec4 or mat4 is not lowered yet` | `src/shaderc/online/mali/nir.rs` `build` | 1089 | plan #309 |
| `BLEND_SHADER` | `mali frame: a blend shader other than a constant colour's (min/max, SRC_ALPHA_SATURATE, unmatched factors, separate colour and alpha, a masked, 4x or non-RGBA8 target) is not built yet` (`NOT_BUILT_BLEND_SHADER`) | `src/mali/frame.rs` `check_draw` | 845 | plan #309 |
| `CONTROL_FLOW` (`ControlFlow`) | `Valhall: control flow (if, a loop, a return before the end) is not lowered yet` | `src/shaderc/online/mali/nir_fs.rs` `walk` | 532 | plan #309 |
| `INDEXED_LOCAL` (`IndexedLocal`) | `Valhall: an indexed local variable is not lowered yet` | `src/shaderc/online/mali/glsl.rs` `passes` | 472 | plan #309 |
| `LOOP_VERTEX` (`LoopVertex`) | `Valhall: a loop in a vertex shader is not lowered yet` | `src/shaderc/online/mali/nir.rs` `build` | 312 | plan #309 |
| `TEX_KIND` (`TexKind`) | `Valhall: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query) is not lowered yet` | `src/shaderc/online/mali/nir_fs.rs` `sampler_index` | 232 | plan #309 |
| `LOOP_FORM` (`LoopForm`) | `Valhall: a loop past Mesa's unroll limits or with two run-time breaks, or a kept loop with more than two breaks, a continue other than the step's at its top or the colour written inside, is not lowered yet` | `src/shaderc/online/mali/nir_fs.rs` `walk_passes` | 105 | plan #309 |
| `VARYING_FORM` (`VaryingForm`) | `Valhall: a varying lane left unwritten is not lowered yet` | `src/shaderc/online/mali/motion.rs` `apply` | 64 | plan #309 |
| `TEX_VERTEX` (`TexVertex`) | `Valhall: a vertex shader texture look-up other than one 2D look-up at a level (a second one, a size query) is not lowered yet` | `src/shaderc/online/mali/nir.rs` `build` | 53 | plan #309 |
| `INVARIANT` (`Invariant`) | `Valhall: an invariant output (Mesa keeps its arithmetic exact, unfused) is not lowered yet` | `src/shaderc/online/mali/nir.rs` `build` | 36 | plan #309 |
| `UNIFORM_INDEXED` (`UniformIndexed`) | `Valhall: a uniform read at a run-time index other than a float array's element or a mediump fragment shader's vec4 array first in the block read once, indexed by an int uniform, is not lowered yet` | `src/shaderc/online/mali/nir_fs.rs` `walk_nodes` | 32 | plan #309 |
| `DISCARD_FORM` (`DiscardForm`) | `Valhall: a discard other than a highp fragment shader's \`if (a < b) discard;\` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's) is not lowered yet` | `src/shaderc/online/mali/nir_fs.rs` `discard` | 25 | plan #309 |
| `FRAG_OUTPUT` (`FragOutput`) | `Valhall: a fragment output other than gl_FragColor is not lowered yet` | `src/shaderc/online/mali/nir_fs.rs` `layout` | 6 | plan #309 |
| `SPECIAL_INPUT` (`SpecialInput`) | `Valhall: a built-in input (gl_FragCoord, gl_FrontFacing, gl_PointCoord, gl_VertexID, gl_InstanceID) is not lowered yet` | `src/shaderc/online/mali/nir_fs.rs` `layout` | 4 | plan #309 |
| `MATRIX_ATTRIB` (a FAIL, not a refusal) | none: pixels wrong. `verdict case 63 matattr: FAIL ... got 0xff800000 want 0xff804000`, matrix attribute columns 1..n read 0 | `tools/cts/RESULTS.md` 'Mali' (a draw, not a refusal) | 40 | none yet (issue #32 holds the symptom) |
| **refusals** | | | **10947** | |

The refusal cases sum to 10947: the sixteen `MaliError` rows are 10102,
plus `BLEND_SHADER` 845. `MATRIX_ATTRIB` is the 40
cases of `attribute_location` and `default_vertex_attrib` whose names carry
`mat`; they are not in the 10947.

The rows are the *class* a group is expected to hit. The same shader may be
refused by a sibling variant of the same family (`Arithmetic` instead of
`FsOperandForm`, `MixedPrecision` for a mediump/highp mix, `CompareForm`,
`IfForm`, `AlgebraicForm`, `FragmentTooLarge`, ...). All of those messages
end `is not lowered yet` and are plan #309's by scope; see section 4.

## 3. Hang-risk: 208 cases

All 208 are the fragment-stage loops whose bound is not a constant, in
`shaders.loops`:

| group | cases |
|---|---:|
| `do_while_dynamic_iterations.*_fragment` | 34 |
| `for_dynamic_iterations.*_fragment` | 35 |
| `while_dynamic_iterations.*_fragment` | 35 |
| `do_while_uniform_iterations.*_fragment` | 34 |
| `for_uniform_iterations.*_fragment` | 35 |
| `while_uniform_iterations.*_fragment` | 35 |
| total | 208 |

(The earlier, unrecorded forecast said "about 210"; the count by name is
208.) Why they are a risk: a highp fragment loop with a run-time bound
compiles (`loops-rt`; sweep cases 45 and 98 PASS) as a *kept* loop whose only
exit is the GLSL condition, copied from Mesa. A wrong exit leaves the shader
running until panthor's job timeout kills the group: case 104 (`divloop`) did
that on this board (`verdict case 104 divloop: FAULT work lost 2`, one
`panthor ... job timeout` per run, GPU usable afterwards;
`docs/notes/mali-loop-board-failures.md`, `docs/notes/mali-panthor-kmsg.md`).
Each such case in a dEQP chunk is therefore a possible job timeout, and the
chunk is the unit that survives it (`tools/cts/mali-deqp-run.sh`: a hang or
a fault moves the chunk to `hang` or `fault` and the board stays usable over
ssh). A mediump loop is refused first (`LoopMediump`, "a loop Mesa keeps ...
in a mediump fragment shader"), so the real number of hangs is at most 208
and likely smaller; those that are refused instead count under `LOOP_FORM`
by the rule "the first message that matches is not new" (section 4), not as
new causes. The loop cap that gl_test expects (`UNIFORM_LOOP_CAP` 4096 in
the other back ends) is described in `mali-loop-board-failures.md`; whether
the Mali back end has it at this commit is for the loop work to say, not for
this note.

The 312 vertex-stage loops (`*_vertex`) are not hang-risks: the compiler
refuses every loop in a vertex shader (`LOOP_VERTEX`). The 105 fragment
loops with constant iteration counts and the one `custom` case are counted
refused (`LOOP_FORM`: the unroll limits); a small one that fits would pass,
which is a forecast miss, not a new cause. Nothing else is a hang-risk: the
other refused groups never reach the GPU, and the likely-pass groups run
the shapes the sweep already passes. Two further places a chunk could stick
that are not forecast: a draw with a kept loop in a likely-pass group, and
panthor's own reset (see the kmsg note), both visible in the chunk's status.

## 4. Reading this against the board results

A non-Pass is a **known cause** (no new issue) when it matches a row of
section 2 by message, whatever group it falls in:

1. The case's message is a `MaliError` message (ends `is not lowered yet`,
   whichever of the ~60 variants), or one of the `src/mali/` constants
   (`mali: ...`, `mali frame: ... is not built yet`). That is #309's scope.
   If the group was forecast pass, it is a forecast miss: record it under
   the group, not as a cause. Cite the variant in the tally so that #309
   gets the count per variant.
2. A fragment loop in `shaders.loops` that ends as a chunk `hang`, `fault`
   or a `job timeout` line is the hang-risk of section 3, not a new cause,
   unless it is outside those 208 names.
3. A case of `attribute_location` or `default_vertex_attrib` with `mat` in
   its name that fails on pixels is `MATRIX_ATTRIB` (#32).

A non-Pass is a **new cause** (a new issue, one per cause) when:

- it is a Fail with no refusal message (wrong pixels, a GL error, a bad
  query) in a group forecast pass: a defect of the GL core or the frame, not
  a gap;
- it is a refusal whose message is in neither `MaliError::message` nor the
  `src/mali` constants;
- it is a crash of `deqp-gles2`, a `fault` chunk whose panthor lines name an
  MMU, firmware or GPU fault (not a job timeout), or a reset not preceded by
  a case of section 3;
- it is a `Fail` in the negative compile tests (the front end accepts what
  GLSL ES 1.00 forbids, or rejects what it allows): not Mali, a front-end
  bug on every back end.

A forecast pass that becomes a refusal moves the cases between groups of
this note, not between owners. Add the new causes to this note's table
when they are opened as issues, so that the next run reads the new list.

## 5. How the counts were made

A throwaway script (not committed) read `tools/cts/gles2-mustpass.txt` (17,165
distinct names, checked) and put every name in exactly one of four outcomes:
pass, refused with one cause, hang-risk, known FAIL. The groups of section 1
are the first one or two components after `dEQP-GLES2.functional.`. The
rules, in the order they apply:

- **Negative compile tests** (pass): any name with `invalid` or `illegal` in
  it after the group; the groups `invalid_implicit_conversions`,
  `reserved_operators`, `keywords`, `declarations`; `shaders.linkage`
  names not starting `uniform_struct`; all of `shaders.preprocessor` (valid
  preprocessor tests render a constant colour; low confidence).
- **`shaders.loops`**: `_vertex` is `LOOP_VERTEX`; a `_fragment` name with
  `dynamic_iterations` or `uniform_iterations` is hang-risk; the remaining
  fragment names (constant iterations, `custom`) are `LOOP_FORM`.
- **`shaders.operator`**: `int_compare` `INT_VALUE`; `selection` and `sequence`
  `CONTROL_FLOW`; a name with an `int`, `ivec`, `uint`, `bool` or `bvec` word
  `INT_VALUE`; the rest `ARITH`. **`shaders.random`**: `conditionals`
  `CONTROL_FLOW`, `texture` `TEX_KIND`, `scalar_conversion` `INT_VALUE`, the
  rest `ARITH`. **`shaders.matrix`**: `uniform` with `mat2` or `mat3`
  `UNIFORM_TYPE`, the rest `ARITH`. **`shaders.swizzles`**: `FS_OPERAND`.
  **`shaders.indexing`**: `varying_array` `VARYING_FORM`, `uniform_array`
  `UNIFORM_INDEXED`, the rest `INDEXED_LOCAL`. **`shaders.conversions`**,
  **`constants`**: an int/bool word `INT_VALUE`, else `FS_OPERAND`.
  **`shaders.constant_expressions`**: `builtin_functions` `ARITH`, else
  `FS_OPERAND`. **`shaders.functions`**: `control_flow` `CONTROL_FLOW`,
  `datatypes` with an int/bool word `INT_VALUE`, `overloading.builtin_*`
  `ARITH`, the rest `FS_OPERAND`. **`shaders.struct`**: `dynamic` or `loop`
  `CONTROL_FLOW`, `uniform.*` `UNIFORM_TYPE`, the rest `FS_OPERAND`.
  `conditionals`, `return`, `algorithm`, `function`: `CONTROL_FLOW`;
  `discard`: `DISCARD_FORM`; `invariance`: `INVARIANT`; `fragdata`:
  `FRAG_OUTPUT`; `texture_functions`: `TEX_VERTEX` for `_vertex`, else
  `TEX_KIND`; `builtin_variable`: `fragcoord*`, `frontfacing*`
  `SPECIAL_INPUT`, `max_*` `INT_VALUE`, else `FS_OPERAND`; `scoping`,
  `qualification_order`, `misc` and the rest: `FS_OPERAND` (`ARITH` for
  `misc`).
- **`fragment_ops.blend`**, modelled on `src/mali/blend.rs` (`needs_shader`:
  `can_fixed_function`, then the constant colour's channels differ; `builds`:
  the few shaders the back end makes). `equation_src_func_dst_func` (630):
  a case needs a blend shader when a factor is SRC_ALPHA_SATURATE, or the
  factors are neither the same base up to an invert nor ONE/ZERO (the
  `dst_color`/`src_color` pair is the 2src form and is fixed function), or
  it reads `CONSTANT_COLOR` (assumption: dEQP's blend colour has distinct
  channels, so the constant is not homogeneous). The few it needs and
  `blend::builds` makes (ADD, the factors ZERO, ONE, the constant colour or its
  inverse, one of them the constant, not the same twice) are pass; the
  rest are `BLEND_SHADER`. Result: 206 pass, 424 refused.
  `rgb_func_alpha_func` (421): all `BLEND_SHADER`, a guess: the names do
  not give the fixed side, and separate colour/alpha factors need a shader
  unless they match. `rgb_equation_alpha_equation` (9): pass (fixed
  function). The other `fragment_ops` groups are pass.
- **`texture.vertex`**: `2d` pass, `cube` `TEX_VERTEX`. Other `texture.*`
  names with `bias` or `projected`: `TEX_KIND` (24, all in `mipmap`); the
  rest pass, cube included (sweep case 113 passes a cube look-up; the
  message does list "a cube map", so a cube miss is a known cause).
- **`uniform_api`**: a name with a type word outside float, vec2, vec3, vec4,
  mat4 (int, bool, ivec, bvec, sampler, mat2, mat3) or in `random`:
  `UNIFORM_TYPE`; the rest `FS_OPERAND` (the generated value checks compare
  and branch). The `info_query` cases only need a link, so the count is an
  upper bound for refusals only if the shader reads the uniform; they are
  counted refused.
- **`light_amount`**: `ARITH`. `attribute_location` and
  `default_vertex_attrib`: a `mat` in the name is `MATRIX_ATTRIB`. All other
  groups: pass.

The largest uncertainties, in cases: `rgb_func_alpha_func` 421 (blend shader
or fixed function), `uniform_api` 1267 (link-only cases may pass), the
`shaders.operator` and `shaders.random` float cases (about 2,900 are
`ARITH` by group, some may be `FsOperandForm`, `MixedPrecision` or even
compile), and the pass forecast of the GL-core groups (section 1).

## What the note does not claim

- No case is claimed to pass because it compiles: compiling is necessary, not
  sufficient (a frame need or a wrong pixel still fails; see
  `tools/mali/sweep_predict.py`).
- The refusals are not claimed to be 10,947 distinct defects. Five families
  (`ARITH`, `INT_VALUE`, `FS_OPERAND`, `UNIFORM_TYPE`, `BLEND_SHADER`) hold
  9,074 of them; the compiler is a transcription of Mesa for the shapes the
  references draw (`src/shaderc/online/mali/mod.rs` header), so dEQP's
  generated shaders, which no reference draws, are the expected normal
  case, not a bug list.
- The fixes belong to plan #309 and later plans, not to the dEQP run.
