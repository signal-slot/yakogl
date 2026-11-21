<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the varying link refusals in dEQP-GLES2 shapes

Plan #377, item study-varylink (ROADMAP stage 5, issues #7 and #20). This
note covers the four refusals of section M of `docs/es2-cts-failures.md`
(the 37305608 tree, 2026-11-15) that come from the varying layout of a
vertex shader. Each refusal is a `MaliError` of
`src/shaderc/online/mali/mod.rs`:

| refusal text (`Valhall: ... is not lowered yet`) | variant | Fail | NS |
|---|---|---|---|
| `two varying lanes of equal value (Mesa's link merges them)` | `VaryingDuplicate` | 651 | 5 |
| `a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites)` | `VaryingNotFromAttribute` | 252 | 0 |
| `a gl_Position lane left unwritten` | `PositionLane` | 155 | 0 |
| `a varying lane left unwritten` | `VaryingForm` | 3 (+1 counted under `declarations`) | 0 |

The cases were read from `refusals.tsv` of
`tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/`. 1,067 rows carry one
of the four texts: 656, 252, 155 and 4. The five NotSupported rows are
`shaders.discard.dynamic_loop_*`. dEQP reports them as "Dynamic loops not
supported" because their vertex shader is refused. Each of these rows is
refused while the **vertex shader is compiled alone**: the text is
`backend.rs:981`'s "the code generator refused this vertex shader". A
refusal at link time would read "link error: the vertex shader for the
program's fragment shader" (`src/gl/program.rs:1754`).

Everything below was read at the integration tip 38ad30eb, whose
`src/shaderc/online/mali/**` equals origin/main dab58772. The Mesa
references are Mesa 26.2.1 (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`).
The dEQP sources are VK-GL-CTS 93bca01, the revision RESULTS.md uses,
fetched into `$CONNECTED_DEPS_DIR/VK-GL-CTS` with a sparse checkout of
`modules/gles2`, `modules/glshared` and `data/gles2/shaders`.

## What plan #367 holds, and what it leaves

Read first:

- `docs/notes/mali-varying-constants.md`: plan #154 varyconst, and the F5
  section of plan #185 ref-varymerge.
- `docs/notes/mali-partial-varyings.md`: the link's lane rules, steps 1-6.
- `docs/notes/mali-glmark2-forecast.md`: #367's glmark2 measure.

What they hold:

- **#367's "varying link rules"** are the link-time rules: a partly read
  varying, constant lanes propagated, and lanes of uniforms alone made
  flat. They are lowered already. The forecast (section "Confirmed: the
  link rules pass where they are reached") finds that no glmark2 program
  is refused by them.
- None of the four refusal texts appears in
  `tests/fixtures/mali_glmark2_verdicts.txt`.
- **No `gm2_varylink_*` reference exists.** None is in
  `tools/mali/testdata/` at 38ad30eb or at origin/main dab58772, and no
  commit in this clone's refs names one.

So #367 holds no shape of these four refusals, and every dEQP shape below
is this plan's. One overlap with earlier work needs care.
`mali-varying-constants.md` § "Equal lanes and sampler-struct lanes"
already made two references, and both are dEQP shapes:

- `tools/mali/testdata/varymerge_equal.*` is `vertex_arrays.single_attribute`'s
  shader (its header says so).
- `varymerge_structsampler.*` is the same text as
  `tools/shaders/es100/vtex_struct_sampler.vert`, which is
  `shaders.struct.uniform.sampler_vertex`.

The lowering that note proposed (item `varying-merge`) was never done:
`va_vec2.vert` and `vtex_struct_sampler*.vert` still carry these refusals
in `tools/mali/corpus-verdicts.txt`. This note reuses both references.

## The refusal sites

All are in `src/shaderc/online/mali/`.

| site | condition | variant |
|---|---|---|
| `nir.rs:1967-1968` (`alu_sized`, reached from every `Nir::alu`) | an operand of an ALU op is `Val::Undef` | `PositionLane` (misnamed, see shape P) |
| `nir.rs:3400-3402` (`build`) | a `gl_Position` lane is `Val::Undef` after the walk | `PositionLane` |
| `nir.rs:3450` | a laid-out varying has no lanes at all (never stored) | `VaryingForm` |
| `nir.rs:3466` | a stored lane is `Val::Undef` | `VaryingForm` |
| `motion.rs:216`, `:225` | a moved expression's varying has no lanes | `VaryingForm` (not reached by these rows) |
| `nir.rs:3468` | a stored constant lane that is not under an `if` (the program build drops these first; reached only through a layout that keeps it) | `VaryingNotFromAttribute` |
| `nir.rs:3481-3483` | a lane with no attribute, look-up or run-time index behind it, and not `uniform_only` (a phi of uniforms) | `VaryingNotFromAttribute` |
| `nir.rs:3501-3502` | a flat lane (constant under an `if`, or `uniform_only`) and an interpolated lane both stored | `VaryingNotFromAttribute` |
| `nir_fs.rs:6505-6506` (`layout`) | the same mix on the fragment side | `VaryingNotFromAttribute` |
| `mod.rs:1700-1702` (`vs_constants`) | more constant/flat/equal entries than `link::VaryingConsts` holds | `VaryingNotFromAttribute` |
| `nir.rs:3487-3489` | two stored lanes hold the same `Val` | `VaryingDuplicate` |

Three code paths feed the duplicate check:

- **Equal lanes are recorded only for interpolated values.** `lane_at`
  (`nir.rs:3368-3378`) returns `None` for a convergent lane (`:3374`
  needs `!vs.convergent(n)`) and for a phi or select. So two lanes that
  hold the same uniform value are never put in an equal class
  (`link::VaryingConsts::push_equal`, `link.rs:199`).
- **A shader compiled alone stores every lane.** `build` gets `reads ==
  None`, and `compact_layout` (`nir.rs:3634-3640`) then stores every lane
  of every varying.
- **The alone compile drops only constant lanes.** The "own reads" of a
  shader compiled alone (`nir.rs:3422-3436`) remove its constant lanes,
  but not the lanes `push_equal` recorded.

As a result, two equal lanes reach the `seen` check (`:3487`) even when
the program link could merge them.

How the program's vertex shader is made. `vs_for_fragment`
(`backend.rs:1998-2023`) rebuilds the program's vertex shader for the
fragment shader's reads whenever `vs_consts_of` finds a constant, flat or
equal entry. The alone compile's record is then only the shader object's
record, which glCompileShader needs; the link replaces it (as for case
78, `mali-varying-constants.md` item 3).

## The shapes

Shapes were grouped from case names and the generating sources. A
representative of each was compiled on the host (method below). The
column "≤" marks a split the next item has to measure:
`draw.random.*` and `shaders.random.*` pick their arrays and expressions
at random.

| shape | refusal / site | dEQP rows | cases | generator (VK-GL-CTS 93bca01) |
|---|---|---|---|---|
| **E1** equal lanes of an attribute value, vertex shader compiled alone | `VaryingDuplicate`, `nir.rs:3487` (alone: `compact_layout` keeps all lanes) | `shaders.swizzles.vector_swizzles.*_vertex` with a repeated component (126), `indexing.vector_subscript.*_vertex` (27, `v_color = vec3(res)`), `conversions.scalar_to_{vector,matrix}.*_vertex` (18), `matrix.mul.*_vec2_mat2_vertex` (6), `discard.*` (25: `v_color.xyz` equals `v_coords.xyz`), `attribute_location.*` (7), `linkage.varying_differing_precision_3` (1) | 210 | `glsShaderLibraryCase.cpp:290-348` (`out0 = in0.xx`), `es2fShaderIndexingTests.cpp:749-850`, `es2fShaderDiscardTests.cpp:83-92`, `glsAttributeLocationTests.cpp:225-247` |
| **E2** equal lanes of a uniform value (convergent) | `VaryingDuplicate`, `nir.rs:3374` (convergent lanes kept out of the classes) then `:3487` | `vertex_arrays.single_attribute.*` (368: `v_color = vec4(u_colorScale * vec3(1.0), 1.0)`), `draw.draw_{arrays,elements}.*.single_attribute` (14: `vec4(u*vec3(1.0), 1.0) * 0.5 + 0.5`) | 382 | `glsVertexArrayTests.cpp:843-974`, `glsDrawTest.cpp:1700-1807` |
| E1/E2 ≤ | `VaryingDuplicate` | `draw.random.*` (51), `shaders.random.*` (13) | 64 | `es2fDrawTests.cpp:550-700`, `glsRandomShaderProgram.cpp` |
| **C** a convergent lane beside interpolated lanes of one varying | `VaryingNotFromAttribute`, `nir.rs:3501-3502` (`flat && smooth`) | `vertex_arrays.multiple_attributes.*` (203: every array `vec2`, so `v_color.z = u_colorScale` beside `x, y` from `a_1.xy`), `draw.draw_{arrays,elements}.*.{multiple,default}_attribute`, `.first.*`, `.indices.*` (36), `shaders.struct.uniform.sampler{,_array}_vertex` (2: the texel's `rgb` beside the uniform `s.a`), `shaders.random.*` (10), `fragment_ops.interaction.basic_shader.20` (1) | 252 | `es2fVertexArrayTest.cpp:491-786` (all `OUTPUTTYPE_VEC2`), `glsDrawTest.cpp:1776-1781`, `es2fShaderStructTests.cpp:1876-1892` |
| **P** an ALU op over an undefined lane | `PositionLane`, `nir.rs:1967-1968` | `shaders.operator.*.{float,vec2,vec3}_vertex` of `unary_operator` (72), `common_functions` (50), `angle_and_trigonometry` (16), `geometric` (8), `exponential` (4), `sequence` (3); `functions.overloading.user_func_arg_complex_types_vertex`, `qualification_order.parameters.valid.storage_precision_vertex` (1 each) | 155 | `es2fShaderOperatorTests.cpp:590-739` |
| **U** a declared varying never written | `VaryingForm`, `nir.rs:3450` | `shaders.linkage.varying_{2,3,4}`, `shaders.declarations.invalid_declarations.varying_in_fragment_main` | 4 | `data/gles2/shaders/linkage.test:25-86` |

656 + 252 + 155 + 4 = 1,067. Not one of these rows is a `gl_Position`
lane that the shader leaves unwritten: every vertex shader in the table
writes `gl_Position` whole.

### Host probes (how the sites were confirmed)

Hand-made copies of each generator's output were placed under a scratch
directory of `tools/shaders/` (not kept). They were compiled through
`tools/mali/corpus-verdicts.sh`, which is the alone compile
(`host-test,gpu-mali,shaderc-online,no-shader-table`), and the directory
was removed afterwards. Verdicts at 38ad30eb:

| probe (generator arm) | verdict |
|---|---|
| swizzles `mediump_vec2_xx` (`out0 = in0.xx`) | equal value (E1) |
| swizzles `mediump_vec2_xy` | ok |
| discard vertex shader | equal value (E1) |
| vertex_arrays single, `a_0` float | equal value (E2) |
| draw single_attribute | equal value (E2) |
| vertex_arrays multiple, three `vec2` arrays | no attribute (C) |
| draw multiple_attributes | no attribute (C) |
| operator `abs` mediump vec2 / vec3 (with and without the `* 0.5 + 0.5`) | gl_Position lane (P) |
| operator `abs` mediump float, and `abs` vec4 over a `yzwx` local | ok |
| `pre_increment_effect` float (`res = in0; ++res;`) | gl_Position lane (P) |
| `res = res + 1.0` on the same float | ok |
| `vec4(abs(a_in0.wy), 0.0, 1.0)` (no local) | ok |
| `mediump vec2 in0 = a_in0.wy; vec4(abs(in0), 0.0, 1.0)` | gl_Position lane (P) |
| the same with `-in0` | ok |
| `color.yz = a_in0.wy` into a `vec4` local, then `* 0.5 + 0.5` | ok |
| linkage `varying_2` vertex shader | varying lane left unwritten (U) |

What P is: an ALU op the front end emits wider than the defined lanes of
its operand. Two cases show it: `abs` of a `vec2` local, and `++` of a
float local. The walk (`nir.rs:3003-3010`) computes the op per lane up to
the op's width. A lane past the operand's is `Val::Undef`, and
`alu_sized` refuses it under the wrong name. The `Fmul` arm already maps
such a lane to `Val::Undef` (`nir.rs:3014-3017`, the case of
`normalize`'s product); the `Fabs`, `Fadd`, `Ffloor`, `Fneg`-through-`alu`
arms do not. Where the extra width comes from (the `builtin_call.rs`
`componentwise` emit, or the `++` desugaring in `sema.rs:1610`) was not
traced to the line. The lowering item confirms it with an IR dump of the
two probes before it changes anything. The four operator families above
pass at vec4, which is consistent with this reading.

## What Mesa 26.2.1 does

The GL link (`gl_nir_linker.c:1517-1525`) lowers the IO of both stages to
intrinsics (`nir_lower_io_passes`). It then runs `nir_opt_varyings_bulk`
(`nir_opt_varyings.c:6168-6250`), which scalarises the varyings and
optimises each stage first. Inside `nir_opt_varyings`
(`nir_opt_varyings.c:6003-6135`) the order is:

1. **Divergence.** `nir_divergence_vertex` analysis of the producer
   (`:6021-6027`). A store whose value and block are not vertex-divergent
   marks its scalar slot `convergent32/16` (`gather_outputs`,
   `:1719-1746`).
2. **Dead varyings.** `remove_dead_varyings` (`:2112-2290`) runs inside
   `init_linkage`.
   - A lane the fragment shader never loads is not stored.
   - A load with no store becomes `undef` (`:2219-2260`).
   - **Shape U**: GLSL's never-written output is stored from an undefined
     temporary at the end of the shader. `nir_opt_undef` drops store
     channels whose value is undef (`nir_opt_undef.c:143-178`), so the
     varying has no store at all. The fragment shader's read of `var`
     (`varying_4`) is `undef`.
3. **Constants and uniform expressions.** `propagate_uniform_expressions`
   (`:2887-2990`) moves a lane into the consumer when the lane is a
   uniform expression of cost ≤ `max_varying_expression_cost`.
   - Constants and undefs cost nothing (`is_uniform_expression`,
     `:2845-2877`).
   - A uniform `load_deref` costs 2 (`default_varying_estimate_instr_cost`,
     `:5871-5881`).
   - Panfrost sets neither `max_varying_expression_cost` nor
     `varying_expression_max_cost`; nothing under `src/panfrost` or
     `src/gallium/drivers/panfrost` names them. So the limit is 0: **only
     constant lanes are propagated, and a uniform lane stays a varying.**
     This is why `varyconst_117` and the others are flat varyings, not
     fragment uniforms.
4. **Duplicates.** `deduplicate_outputs` (`:3252-3440`) runs over every
   removable scalar slot. Its key is the set of `<block, value>` of the
   slot's stores (`:3334-3359`), with the value resolved through movs
   (`nir_scalar_resolved`). The table is kept per input qualifier
   (`get_input_qualifier`, `:3042-3136`): the fragment shader's
   `load_interpolated_input` (smooth) or `load_input` (flat).
   - The key does not look at divergence. **Two lanes of one uniform
     value merge as surely as two lanes of one attribute value (E1, E2).**
   - The loads of the duplicate are redirected to the first slot
     (`:3383-3423`), and its stores are removed.
   - This runs before the convergent tidy-up (`:6071-6076`). So E2's
     lanes are first merged, while all three are still smooth, and only
     then promoted.
5. **`tidy_up_convergent_varyings`** (`:1853-1935`). A slot that is
   interpolated and convergent loses its interpolated bit and keeps the
   convergent bit: it can be promoted to flat.
6. **Compaction.** `compact_varyings` (`:5491-5566`) places the slots.
   Panfrost sets none of `nir_io_has_flexible_input_interpolation_except_flat`,
   `nir_io_mix_convergent_flat_with_interpolated` or
   `nir_io_always_interpolate_convergent_fs_inputs`, so it uses
   `fs_assign_slot_groups_separate_qual` (`:5322-5413`). That function:
   - places interpolated slots first, per qualifier, and remembers the
     unused 32-bit components of their vec4s (`fs_assign_slots`,
     `:5187-5217`);
   - then places flat slots;
   - then places **the convergent slots**: first in unused components of
     flat vec4s (`:5378-5384`), **then in unused components of
     interpolated vec4s, as interpolated lanes** (`:5389-5396`), and only
     the rest in new flat vec4s (`:5407-5412`).
   - A convergent lane that ends in a flat vec4 has its load rewritten to
     `load_input`. When the fragment shader's use preserves NaNs, the
     producer's store becomes `build_convert_inf_to_nan`, which is
     `ffma(x, 0.0, x)` (`:4963-5003`, `:1094-1117`). A convergent lane
     placed in an interpolated vec4 keeps `load_interpolated_input`. Its
     store is the plain value, with no `ffma`.
7. **Panfrost's layout.** `pan_varying_collect_formats` and
   `pan_build_varying_layout_compact`
   (`pan_nir_collect_varyings.c:338-448`) give each generic slot its
   `ncomps` lanes. The slot is at an offset aligned to the next power of
   two of its size (`:431-436`): a 3-lane fp32 slot is 12 bytes at a
   16-byte alignment. A flat slot's descriptor is an integer format
   (`*32UI`, `mali-varying-constants.md`, last paragraph of "What is
   lowered").

### Per shape

- **E1.** At the program link, the lanes merge (step 4). The fragment
  shader reads both from the first. The link step already does this here:
  `push_equal` and nir_fs.rs `layout`, pinned by `fsload_dup*`. The gap
  is the **alone compile**, which has no fragment shader, so Mesa has no
  counterpart: in GL the alone object is never drawn, and the link
  rebuilds it. A shader alone must just not refuse. Its natural layout is
  the link's without the fragment shader: its own lanes, less constant
  lanes, less the later member of each equal class. This is the same
  trick `nir.rs:3412-3436` plays for constants. (That block runs today
  only when a constant lane exists, `:3423`; it must also run when an
  equal lane does.)
- **E2.** The lanes merge (step 4). The one survivor is convergent with
  no interpolated neighbour, so it is a new flat vec4: a 1-lane flat
  varying (step 6). The `w = 1.0` lane is propagated (step 3).
  `varymerge_equal.expect.json` draws exactly the vertex_arrays arm:
  - `vs.varying`: `FMA.f32 r0, u3.w1, 0x0, u3.w1`, `LEA_BUF_IMM`, one
    4-byte `STORE`;
  - `fs.fragment`: one `LD_VAR_BUF_IMM.f32.src_flat32` lane, the
    `(x, x)` word `IADD.v2u16 .h00`, `(x, 1.0)` `MKVEC.v2i16`.

  The draw arm stores `ffma(u, 0.5, 0.5)` (the `vec4(...) * 0.5 + 0.5`
  folded per lane) through the same `ffma(x, 0.0, x)`. It has no
  reference yet.
- **C.** The attribute lanes `x, y` take an interpolated vec4 with two
  unused components. The convergent lane `z` fills component 2 **as an
  interpolated lane** (step 6, `:5389-5396`). The varying stays one
  smooth `vec3`, and `z` is stored as the plain uniform value (a `MOV`).
  `varymerge_structsampler.expect.json` shows exactly this for the
  struct-sampler arm: four stores, the fourth a `MOV` of `s.a`, then a
  smooth `.v4` load. That note's section (b) reads it as a struct
  special case; it is the general rule. A convergent lane goes flat only
  when no interpolated vec4 has room, which is E2 and `varyconst_117`
  and the others.
- **P.** Mesa never computes a lane past the operand's. GLSL IR's
  expression has the operand's width. `nir_lower_alu_width` scalarises
  only the components that are used. Any undef left is folded or dropped
  by `nir_opt_undef` (`:44-63`, `:93-110`, `:246-281`). So nothing is
  emitted for it.
- **U.** No store and no varying slot (step 2). The fragment shader reads
  an undefined value, which the fragment family decides.

## Lowering plan (item lower-varylink, one session)

All changes are in `src/shaderc/online/mali/{nir.rs,nir_fs.rs,link.rs,mod.rs}`.
In the order of rows per risk:

1. **E1, alone compile (210 + part of 64).** Extend the own-reads block
   (`nir.rs:3422-3436`): a shader compiled alone (`reads == None`) is
   built for its own lanes less its constant lanes **and less each lane
   that `push_equal` recorded** (`vs.consts.class(l, c) != (l, c)`). Then
   `compact_layout` stores one lane per class, and the `seen` check stays
   as a guard. No new words: the program link already rebuilds the shader
   for the fragment shader's reads, because `vs_consts_of` is non-empty
   once an equal entry exists (`backend.rs:2000-2003`). Check:
   - The alone record of `tools/shaders/es100/va_vec2.vert` and of the swizzle
     `in0.xx` shader compiles.
   - `fsload_dup*` stays byte-equal.
   - The new `deqp_varylink_swz_xx` and `deqp_varylink_discard`
     references are equal through `check_linked`.
2. **C, a convergent lane fills an interpolated vec4 (252).** Replace
   the `flat && smooth` refusal on both sides (`nir.rs:3501-3502`,
   `nir_fs.rs:6505-6506`) with Mesa's placement:
   - Interpolated lanes are laid out first. Each convergent lane (today's
     `uniform_only`, and a constant under an `if`) takes the next unused
     32-bit component of an interpolated vec4, in scalar-slot order. It
     is then not flat: no `push_flat` in `vs_constants`
     (`nir.rs:3348`), no `ffma(x, 0.0, x)`, and a plain store.
   - Only the lanes left over form flat vec4s after the interpolated
     ones.
   - The `push_flat` decision must therefore be made after the lanes are
     counted, in `vs_constants` and in `build` alike, because both share
     the walk.

   Targets:
   - `varymerge_structsampler.expect.json` (its fragment side; its
     vertex side also needs the struct sampler's vertex look-up, so the
     two struct cases may stop at the next refusal);
   - the new `deqp_varylink_va_multi3` and `deqp_varylink_draw_multi`;
   - the `varyconst_*` population and `fsload_qual*` unchanged.
3. **E2, convergent equal lanes merge (382 + part of 64).** In `lane_at`
   (`nir.rs:3368-3378`), let a convergent lane that is not a phi or
   select into the classes. Mesa keys on the value, not on divergence.
   The class representative is then the flat lane (rule 2 above decides
   flat or interpolated after merging, as Mesa's step 4 runs before step
   6). The fragment shader duplicates the loaded register for each member
   (`nir_fs.rs` `Fs::lane_of`, and the existing `(x, x)` word).
   Targets: `varymerge_equal.expect.json` (vertex_arrays) and the new
   `deqp_varylink_draw_single`.
4. **P, an ALU over an undefined lane (155).** First dump the IR of the
   two probes (`abs` of a `vec2` local, `++` of a float local) to find
   which op is wider than its operand. Then either:
   - fix the width where the front end emits it, if the IR is wrong
     there (that touches `src/shaderc/online/{builtin_call.rs,sema.rs}`,
     outside `mali/**`: say so in the commit);
   - or give every lane-wise arm of the walk the `Fmul` arm's rule
     (`nir.rs:3017`): an undefined operand lane makes an undefined result
     lane. The refusal then moves to where an undefined lane is stored
     (`nir.rs:3401`, `:3466`).

   Either way, rename the refusal at `nir.rs:1968`: it is not a
   `gl_Position` lane (a new variant, for example
   `MaliError::UndefOperand`, and its text). Changing a text moves the
   section M rows to the new text, which doc-m reads. Target:
   `deqp_varylink_op_abs_vec2`. Expect most of the 155 to reach their
   next refusal: transcendentals for `acos`/`atan`, and `mod`, which are
   other families.
5. **U, never-written varyings (4).** A varying with no lanes
   (`nir.rs:3450`), or an undefined lane (`:3466`), is not stored: its
   mask bit is cleared, as `nir_opt_undef` does. A fragment read of it
   reads whatever the slot holds. Target: `deqp_varylink_unwritten`
   (`linkage.varying_4`'s pair). Small; do it last, or hand it to
   lower-misc if the session runs long.

Sizing: steps 1-3 are the session (844 rows, all in the layout code,
three references already or newly drawn). Step 4 is independent of 1-3
and can be its own commit in the same session if the IR dump points at a
one-line width fix. Otherwise move it to lower-misc and keep its
reference. Every step moves `tools/mali/corpus-verdicts.txt` forward in
the same commit:

- step 1: `va_vec2.vert`, `discard_dynloop.vert` (to its loop refusal);
- step 2: `vtex_struct_sampler*.vert` (to the next refusal).

glmark2 has no shape here, so `tests/fixtures/mali_glmark2_verdicts.txt`
should not move. If it does, it moves forward in the same commit.

## Reference cases needed (item ref-varylink)

Make them with `tools/mali/drm_shim_ref.sh run` and `pan_ref_diff.py
extract --draw`, as in `mali-varying-constants.md` § "How the references
were made". Each is a `.vert`/`.frag` pair copied from the dEQP generator
(ES 1.00 arm, with the generator named in a header comment, as
`varymerge_equal.vert` does), plus `.expect.json` and `.stderr` with the
NIR and the disassembly.

| reference | pair | shape | why |
|---|---|---|---|
| `deqp_varylink_swz_xx` | swizzles `mediump_vec2_xx` vertex + its generated fragment shader (`v_out0` compared with the reference) | E1 | equal attribute lanes through the link; the alone record compiles |
| `deqp_varylink_discard` | `es2fShaderDiscardTests.cpp:83-92` + `discard.basic_uniform`'s fragment shader | E1 | equal lanes across two varyings (`v_color.xyz`, `v_coords.xyz`); its fragment shader's discard is already lowered |
| `deqp_varylink_draw_single` | `glsDrawTest.cpp` single_attribute + its fragment shader | E2 | the flat lane of `ffma(u, 0.5, 0.5)` and its `ffma(x, 0.0, x)` |
| `deqp_varylink_va_multi3` | vertex_arrays multiple_attributes, three `vec2` arrays | C | a convergent lane in an interpolated vec4: plain store, smooth `.v3` load, 12-byte slot |
| `deqp_varylink_draw_multi` | draw multiple_attributes | C | the same after the `* 0.5 + 0.5` fold (an `FMA` per lane, the `z` lane on a uniform) |
| `deqp_varylink_op_abs_vec2` | operator `abs.mediump_vec2_vertex` | P | the words once the undefined lanes are gone (`color.yz = res; color * 0.5 + 0.5`) |
| `deqp_varylink_unwritten` | `linkage.varying_4` | U | no slot for `var`; the fragment shader's `vec4(var)` under `if (u_false)` |

Already made, reused: `varymerge_equal` (E2, vertex_arrays arm) and
`varymerge_structsampler` (C, struct arm). `vertex_arrays.single_attribute`
and `.multiple_attributes` set `gl_PointSize = 1.0`, and the draw tests
draw points among other primitives. Pass the point-size output through as
the existing references do.

## References (item ref-varylink, 2026-11-16)

Seven triples `tools/mali/testdata/deqp_varylink_<shape>.{vert,frag,expect.json}`,
one for each row of the table above. The `measure` item had no
`tools/mali/testdata/deqp_shapes/` pair for any of them at the time, so
each pair was rebuilt from VK-GL-CTS 93bca01861b0e3ef3c387027a9791e6d065f900c
(`git fetch --depth 1` of that commit; the files below were read with
`git show`). Each text is the generator's ES 1.00 output, whitespace
included, plus a header comment naming the case and the generator lines.
In `swz_xx.frag` the comment comes after `#version 100`. All seven cases
are rows of `refusals.tsv` in the 37305608 tree, each with its shape's
refusal.

| reference | case (dEQP-GLES2.functional.) | source (rebuilt) | shape |
|---|---|---|---|
| `swz_xx` | `shaders.swizzles.vector_swizzles.mediump_vec2_xx_vertex` | `data/gles2/shaders/swizzles.test:27-46` through `glsShaderLibraryCase.cpp:285-356` (VS) and `:233-282` (FS), `gluShaderLibrary.cpp:473-557` (indent removal), `:1912-1991` (`isOk`) | E1 |
| `discard` | `shaders.discard.basic_dynamic` | `es2fShaderDiscardTests.cpp:83-92` (VS), `:218-227` with `:392-393` (FS) | E1 |
| `draw_single` | `draw.draw_arrays.triangles.single_attribute` | `glsDrawTest.cpp:1698-1807`, `:1809-1823`, ES 1.00 arm `:1837-1846`; array of `es2fDrawTests.cpp:164-173` | E2 |
| `va_multi3` | `vertex_arrays.multiple_attributes.attribute_count.3` | `glsVertexArrayTests.cpp:843-974`, `:976-1018`; arrays of `es2fVertexArrayTest.cpp:507-528` | C |
| `draw_multi` | `draw.draw_arrays.triangles.multiple_attributes` | `glsDrawTest.cpp:1698-1823`; arrays of `es2fDrawTests.cpp:200-220` | C |
| `op_abs_vec2` | `shaders.operator.common_functions.abs.mediump_vec2_vertex` | `es2fShaderOperatorTests.cpp:590-745`, op `:1987-2081`, `abs` at `:1778`, swizzle `:501` | P |
| `unwritten` | `shaders.linkage.varying_4` | `data/gles2/shaders/linkage.test:63-86` through `glsShaderLibraryCase.cpp:441-478` | U |

`discard` is `basic_dynamic`, not the table's `basic_uniform`.
`basic_uniform`'s fragment shader reads only `v_color`, so Mesa's link
drops `v_coords` and no lanes of the two varyings are merged. All 25
discard rows share the vertex shader, and `basic_dynamic` reads
`v_coords.xy`, so it is the case that shows the merge across the two
varyings.

Commands, for each `<s>`. The shim install is `$CONNECTED_DEPS_DIR/mesa-shim`,
built from `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1` with the meson options
of `drm_shim_ref.sh build`, because the tarball download was not available
on this host. That build draws `varymerge_equal` byte-equal to its
committed `expect.json` in all three regions. Only the FAU uniform words
differ, because that reference was drawn without `--uniform-values`.

```sh
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim \
    tools/mali/testdata/deqp_varylink_<s>.vert tools/mali/testdata/deqp_varylink_<s>.frag \
    <out>/deqp_varylink_<s> --uniform-values          # unwritten: without --uniform-values
python3 -I tools/mali/pan_ref_diff.py extract <out>/deqp_varylink_<s>.trace --draw \
    > tools/mali/testdata/deqp_varylink_<s>.expect.json
PAN_SHC=<pan_shc> tools/mali/glsl2pan.py <vert> <frag> > <out>/<s>.json 2> <out>/<s>.disasm
python3 -I tools/mali/pan_ref_diff.py diff <out>/<s>.json <out>/<s>.disasm <out>/deqp_varylink_<s>.trace --draw
```

`unwritten` is drawn without `--uniform-values`, because gl_ref.py does not
set a bool uniform (`u_false`). Its value stays 0, which is false, the
case's value. Every draw is of triangles: for `draw_*` and `va_multi3`,
`vs.position` is the `no_psiz` program the draw runs.

What each pins. The layout below is from the shim driver's program: the
NIR `decl_var`s in the `.stderr`, the stores of `vs.varying`, the
fragment shader's `LD_VAR`, and the varying buffer descriptor ("Vertex
packet") in the `.trace`. It is compared with pan_shc's
`vs.varying_layout` and `fs.varying_formats`:

| reference | regions (instructions) | Mesa's varying layout | pan_shc |
|---|---|---|---|
| `swz_xx` | position 24, varying 3, fragment 15 | one smooth lane, `VAR0.x`, 4 B, `R32F`. The varying program stores `in0.x` once (`LD_ATTR_IMM.f32 index:1`, one `STORE.i32`). The FS loads one `.f32` lane and uses it for both of `out0`'s lanes | layout equal (`ncomps` 1, 4 B); `vs.varying` byte-equal; FS differs (the `mediump` gap) |
| `discard` | position 24, varying 15, fragment 9 | `v_color` is gone: its `xyz` are `v_coords.xyz` and its `w` is the propagated 1.0. `v_coords` stays whole: `VAR0.xyzw`, smooth, 16 B, `RGBA32F`, four stores of `a_coords.xyzw`, one `.v4` load | 3 lanes, 12 B: the SPIR-V path drops `w`. `vs.varying` 7 of 15 differ, FS 3 of 9. **The shim's 4 lanes are the reference** |
| `draw_single` | position 25, varying 17, fragment 7 | **no merge**: `VAR0.xyzw` **flat**, 16 B, **`RGBA32UI`**. The store is `ffma(u, 0.5, 0.5)`, then `ffma(x, 0.0, x)`, stored 3 times, then 1.0 stored (`IADD_IMM 0x3F800000`). The FS has one `LD_VAR_BUF_IMM.v4.src_flat32` | layout equal, FS format `alu_type` 0x24 (uint32); `vs.varying` and FS byte-equal |
| `va_multi3` | position 25, varying 17, fragment 8 | `VAR0.xyz` smooth, 12 B, `RGB32F`. `z` is a plain `MOV` of `u_colorScale`, with no `ffma(x, 0.0, x)`. `w` = 1.0 is propagated. The FS has one `.v3` smooth load | layout equal; `vs.varying` 2 of 17 differ (FMA operand order); FS byte-equal |
| `draw_multi` | position 25, varying 21, fragment 7 | `VAR0.xyzw` smooth, 16 B, `RGBA32F`. `z` = `u * 0.5 + 0.5` is stored plain (`FADD_IMM`) in the interpolated vec4, and `w` = 1.0 is **stored**, not propagated | layout equal (4 lanes); `vs.varying` 17 of 21 differ; FS byte-equal |
| `op_abs_vec2` | position 24, varying 27, fragment 7 | `VAR0.xyzw` smooth, 16 B, `RGBA32F`. Four stores: `x` = 0.0 * 0.5 + 0.5 and `w` = 1.0 * 0.5 + 0.5 are **stored**, computed in fp16 (`FMA.v2f16`, `F16_TO_F32`). Only `abs` of the two defined lanes is computed; no undefined lane reaches an op | layout equal; `vs.varying` differs (`mediump`) |
| `unwritten` | position 24, fragment 7; no `vs.varying`, no `secondary_offset` | no varying slot. The FS's `if (u_false) vec4(var)` folds to `vec4(1.0)`: `ATEST`, two `MOV`s of the fp16 pair 1.0, `BLEND` | none: glsl2pan's link refuses (`FS input 'var' has no VS output of that name`) |

What this changes for lower-varylink:

- **E2 merges only lanes that resolve to one scalar.** `varymerge_equal`
  merges because `vec4(u * vec3(1.0), 1.0)` folds to `vec4(u, u, u, 1.0)`,
  whose lanes resolve to the scalar `u` and a constant.
  - The draw arm's `vec4(u * color, 1.0) * 0.5 + vec4(0.5)` is one vec4
    `ffma`. Its lanes are components 0-3 of that def. They are distinct
    keys for `deduplicate_outputs` and not constants for step 3, so Mesa
    keeps all four lanes. (Reading of `nir_scalar_resolved`; the link
    NIR itself was not printed.)
  - So the lowering must not merge equal values that come from distinct
    components of one vector op, and must not propagate a constant lane
    of such an op. `draw_single` pins four flat lanes and
    `varymerge_equal` pins one.
  - The same rule makes `draw_multi` and `op_abs_vec2` store their
    constant lanes.
- **C, as the study says, where a lane is a uniform alone**: `va_multi3`'s
  `z` and `draw_multi`'s `z` go into the interpolated vec4 as plain
  stores. No `ffma(x, 0.0, x)` appears in either.
- **E1 across two varyings** (`discard`): the merged varying keeps the
  surviving varying's declared lanes, including `v_coords.w`, which the FS
  never reads.
- **Flat descriptor**: `draw_single`'s varying buffer descriptor is
  `RGBA32UI` where a smooth one is `RGBA32F`. This is the integer format
  the last item of "Left open" names. Only this table records it: the
  `.expect.json` holds no descriptors, and the `.trace` is not committed
  (rerun the first command to get it).

## Left open

- The exact source of P's extra width (step 4's first action).
- The `draw.random` and `shaders.random` split between E1, E2 and C (64
  and 10 rows). This is the measure item's
  (`docs/notes/mali-deqp-families.md`): it records the first refusal per
  case.
- Frame side: a flat slot's descriptor is an integer format in Mesa, and
  the record does not say which slots are flat. This is unchanged from
  `mali-varying-constants.md`; only E2 adds flat slots. Whether
  `LD_VAR_BUF_IMM .src_flat32` reads the format at all is the board
  session's to show.

## Lowering map (at 008306cb)

Item map-varylink of plan #377, for lower-varylink. Read at 008306cb, the
tip with the references `deqp_varylink_*`, `tests/mali_deqp_varylink.rs`
and #367's glmark2 link work. Every `file:line` is at that commit. Nothing
was built or run for this map. A statement that is a reading of the code
and not a measurement says so, and names the check the lowering starts with.

### What moved since the study (38ad30eb)

- **E1's alone-compile step is in.** The "own reads" block drops every lane
  `push_equal` recorded (`dup`, `nir.rs:3572`; `reads`, `nir.rs:3573-3585`).
  `discard_dynloop.vert` is `ok` at `tools/mali/corpus-verdicts.txt:33`.
  This is the study's plan step 1, done by 420ea31e for
  `gm2_arithconst_3`.
- **#367 holds no shape of these four refusals**, as the study found.
  `gm2_varylink_*` (`const_*`, `constw_*`, `dup_attr`, `dup_scaled`,
  `dup_uniform`, `pack_*`, `partial_*`, `texlane`, `unread_after`,
  `dummy_unread`), `varyconst_*`, `fsload_dup*` and `fsload_qual*` exist
  and pass. None of the four texts is in
  `tests/fixtures/mali_glmark2_verdicts.txt`. They are the guards of (e).
- The study's `nir.rs` line numbers moved by about +80; the table below
  replaces them.

### (a) The raise sites

Each is a `MaliError` variant of `mod.rs:242-251`, its text at
`mod.rs:499-507`; the variant list the tests match is `mod.rs:369-373` and
`tests.rs:1129-1133`.

| variant (dEQP rows) | site: file:function, condition | reached by |
|---|---|---|
| `VaryingDuplicate` (E1, E2: 651 + 5 NS) | `nir.rs:build:3647-3648`: a stored lane has the same `Val` as one stored before (`seen`, `:3592-3593`). `Val` is node and lane, and `Nir` does `nir_opt_cse` (header `nir.rs:24`), so equal ALU results are one node | the alone compile (`reads == None`, `compile_lowered`, `mod.rs:1648`) and the program's vertex shader (`reads == Some`, `compile_vs_for_reads`, `mod.rs:1773`, from `backend.rs:vs_for_fragment:2013`) |
| `VaryingNotFromAttribute` (C: 252) | `nir.rs:build:3661-3662`: `flat && smooth`. `flat` is set by a lane of uniforms alone (`vs.uniform_only`, `:3640`; `:1741`) or a phi constant (`:3620`); `smooth` by any other lane (`:3644`) | the same two paths |
| `VaryingNotFromAttribute` (other sites) | `nir.rs:build:3628`, a constant lane stored when not `late_fold`; `:3641-3643`, a lane reading no attribute, texture or indexed uniform (a phi of uniforms); `mod.rs:lowered_vs_constants:1709-1710`, `consts.full()` (past `MAX_CONSTS` = 16, `link.rs:114`); `nir_fs.rs:layout:6510-6511`, `flat && smooth` seen from the fragment side (`:6485-6491`) | `vs_constants` / `fs_constants` (`mod.rs:1735`), the fragment compile |
| `PositionLane` (P: 155) | `nir.rs:Nir::alu_sized:1966-1969`: an operand of an ALU op is `Val::Undef`. **The name is wrong**: it is no `gl_Position` lane | `walk`'s lane loop (`nir.rs:3130`): its arms (`Fabs` `:3254`, `Fadd`, `Ffloor`, ...) call `vs.alu` with `lane(w, s[k], l)` (`:2306`) for every `l < width`, past the operand's width. Only `Fmul` guards it (`:3140`) |
| `PositionLane` (the real one) | `nir.rs:build:3547-3549`: `gl_Position` has an `Undef` lane after the walk. None of the 155 meets it. Pinned by `tests.rs:1193` (`gl_Position.xy = p`) | `build` |
| `VaryingForm` (U: 3 + 1) | `nir.rs:build:3601`: a laid-out varying (`generic_layout`, `select.rs:139`, built from the declarations at `nir.rs:3556`) has no entry in `varyings`, the walk never stored it. `:3617`: a stored lane is `Undef`. `motion.rs:216`, `:225`: a moved expression reads a varying with no lanes | `build`, both paths. Pins `tests.rs:1239`, `:2856` |

The fragment side (`nir_fs.rs:layout`, `:6288`) never raises
`VaryingDuplicate`. It places equal lanes once per **(class, interpolation
qualifier)** (`:6458-6469`, `FsConsts::class`) and loads a lane flat when
the fragment shader declares it `flat` or `consts.flat(loc, c)` says the
vertex lane is uniform alone (`:6477`).

### (b) The rules as the references show them, and what to change

One reading drives E1, E2 and C. **A lane's identity is where Mesa's scalar
slot resolves, not its value.** `deduplicate_outputs` keys on
`nir_scalar_resolved` of each store (`nir_opt_varyings.c:3334-3359`), which
looks through `mov` and `vec` to the producing `(def, component)`:

| reference | lanes | Mesa |
|---|---|---|
| `varymerge_equal` | `vec4(u * vec3(1.0), 1.0)` folds to `(u, u, u, 1.0)` | merged: one flat lane, `w` propagated |
| `deqp_varylink_swz_xx` | `in0.xx` | merged: one smooth lane (`R32F`, 4 B) |
| `deqp_varylink_discard` | `v_color = vec4(a.xyz, 1.0)`, `v_coords = a` | `v_color` gone, `v_coords` whole (`RGBA32F`, 16 B, 4 stores) |
| `deqp_varylink_draw_single` | `vec4(u * c, 1.0) * 0.5 + 0.5`, one vec4 `ffma` | **not merged**: four flat lanes (`RGBA32UI`), `ffma(u, 0.5, 0.5)` then `ffma(x, 0.0, x)` stored 3 times, 1.0 stored (`IADD_IMM`) |

The last row is the new fact. Our three lane-wise `ffma`s are **one node
after CSE**, so their `Val`s are equal and `seen` (`nir.rs:3647`) refuses
them. In Mesa they are components 0-2 of one vector def: distinct keys. The
`w` lane is component 3 of the same def, not a constant, so it is stored and
not propagated. This is a **reading of `nir_scalar_resolved`**; the link
NIR was not printed. The check to start with: print the shim's link NIR of
`draw_single` and `varymerge_equal` (`NIR_DEBUG=print` in
`drm_shim_ref.sh run`) and confirm that the four stores of the first are
`(ssa_n, 0..3)` of one `ffma` and the three of the second are `(u, 0)`.

If it holds, the key is **`(IR value, component)` after swizzles and
`vec`/`mov`**, not `Val`. `walk` (`nir.rs:2964`) has the pieces: each IR
value has four lanes `w.vals[id]`, and `lane` (`nir.rs:2306`) follows a
swizzle. Carry a tag per varying lane (`origin`, beside `VsState::varyings`)
and compare tags in `lane_at` (`nir.rs:3515`) and in `seen` (`:3647`).

**E1 (`swz_xx`, `discard`): attribute lanes of one origin merge.**
- Rule: smooth lanes of one origin merge across components and across
  varyings; the survivor is the first in scalar-slot order. `discard` does it
  across two varyings, with `v_color.w = 1.0` propagated.
- The alone path already drops them (`nir.rs:3572-3585`), and `fsload_dup*`
  and `gm2_arithconst_3` pass. Yet the pins say these pairs still reach
  `VaryingDuplicate`, so the site is **not** the alone path the study named.
  It is `nir.rs:3648` with `reads == Some`, the program's vertex shader.
  **Not determined without a run**: the mask that arrives for `swz_xx`.
  First action: print `(reads, equal[..nequal])` at `nir.rs:3573` for both
  pairs. The candidates are that the fragment record's read mask
  (`fragment_reads`, `mod.rs:1826`; `FsRead.mask` is `keep`, `nir_fs.rs:6495`)
  still names the merged lane, or that `reads_at_vertex_locations`
  (`mod.rs:1850`) returns it. `swz_xx.frag` reads `out0` whole (`isOk(out0,
  ...)`), which is the case that differs from `fsload_dup*`.
- Change: `nir.rs:build` (`lane_at`, `dup`, the `reads` block) and the
  read-mask functions above. A lane of the vertex shader that `reads` names
  but whose origin equals a stored lane's is skipped by `seen`, not
  refused. Keep the refusal for the join case (`fsload_dup_phi`,
  `tests.rs:2440-2462`).

**E2 (`draw_single`): equal values of different components stay apart.**
- Rule: convergent lanes merge only when their origin is one scalar
  (`varymerge_equal`). Lanes that are components of one vec op are kept,
  all promoted to flat. A constant component of such an op is stored, not
  propagated (`draw_single`'s `w`, `draw_multi`'s `w`, `op_abs_vec2`'s `x`
  and `w`).
- Change: `lane_at` (`nir.rs:3515-3524`): drop the `!vs.convergent(n)`
  guard for lanes whose origins are equal, so `varymerge_equal` merges and
  `va_vec2.vert` leaves `corpus-verdicts.txt:125`; `seen` (`:3647`) compares
  origins, so `draw_single`'s three stores pass; in the constant pass
  (`nir.rs:3484-3498`) a `Val::Const` that is a component of a non-constant
  vec op is not `consts.push`ed. The last is the existing `late_fold` arm
  (`nir.rs:3623-3627`, "stored as it is", `gm2_arithconst_4`), which sets
  `smooth`. In a flat vec4 it must set `flat`.
- Descriptor: the reference's `RGBA32UI` and `LD_VAR_BUF_IMM.v4.src_flat32`.
  The word `src_flat32` comes from `nir_fs.rs:6477`. The record's
  `varyings` (`mod.rs:1569-1580`) carry no flat flag and `src/mali/program.rs`
  has no integer varying format, so the format is not produced; the
  byte-compare reads words and layout, not the descriptor, so
  `draw_single` can turn green with the descriptor still float. That is a
  board risk, (e).

**C (`va_multi3`, `draw_multi`): a convergent lane fills an interpolated vec4.**
- Rule: with interpolated lanes present, a convergent lane takes an unused
  component of an interpolated vec4 as an interpolated lane (plain store, no
  `ffma(x, 0.0, x)`); only the leftovers form flat vec4s (study step 6,
  `nir_opt_varyings.c:5389-5396`). `va_multi3`: `VAR0.xyz` smooth `RGB32F`,
  `z` a `MOV` of the uniform. `draw_multi`: `VAR0.xyzw` smooth, `z` =
  `u*0.5+0.5` (`FADD_IMM`), `w = 1.0` stored.
- Change: decide flat or smooth **per slot after the lanes are counted**,
  not per lane in the walk. `flat` and `smooth` (`nir.rs:3592`, set at
  `:3620`, `:3640`, `:3644`) become a per-varying decision, and the two
  `flat && smooth` raises (`nir.rs:3661`, `nir_fs.rs:6510`) go. A convergent
  lane that lands in a smooth vec4 must not be `push_flat`ed
  (`nir.rs:3495`), or the fragment side loads it `.src_flat32`
  (`FsConsts::flat`, `link.rs:299`). So the placement is one function in
  `link.rs` (which owns `VaryingConsts`) that both `build` and
  `nir_fs.rs:layout` call; two copies of the fact are what raise today.

**P (`op_abs_vec2`): an undefined operand lane gives an undefined lane.**
- Rule: Mesa's ALU width is the operand's, so nothing is emitted for a lane
  past it (the `abs` of a `vec2` is two lanes).
- Change at the one site, the `nir.rs:3130` loop: before the `match a`, an
  op whose operand lane is `Val::Undef` gives `Val::Undef`, as `Fmul` does
  at `:3140`. `alu_sized` keeps its guard under its own variant (for
  example `MaliError::UndefOperand`; `mod.rs:242`, `:369`, `:499` and
  `tests.rs:1129` change with it). The refusal then moves to where an
  undefined lane is stored (`nir.rs:3617`, `:3547`).
- Not known: where the wider IR comes from (`builtin_call.rs`
  `componentwise` or `sema.rs:1610`, the study's open item). If `width` is
  wrong in the IR, the fix is at `m.comps_of` (`nir.rs:3121`); the guard is
  right either way.
- The reference computes `x` and `w` in fp16 (`FMA.v2f16`, `F16_TO_F32`):
  a mediump vertex shader. The next refusal after P may be that arithmetic,
  so `op_abs_vec2` may stay ignored with a new pin. Then P is a unit test
  and the pin moves.

**U (`unwritten`): no store, no slot.**
- Rule: a declared varying the shader never writes has no `vs.varying` and
  no slot; the fragment read folds (to 1.0 in the reference).
- Change: `nir.rs:3601` and `:3617` stop refusing and clear the varying's
  mask (`compact_layout`, `nir.rs:3794`), as `nir_opt_undef` drops the
  store. The fragment side has no "undefined" lane (`FsConsts` names
  constants and flat lanes, `link.rs:299`), so it needs one, or a constant
  as Mesa's fold gave. Whether our fragment compile folds
  `if (u_false) vec4(var)` as Mesa does is **not known**; `unwritten` is
  the only reference. The study's advice (hand it to lower-misc if the
  session runs long) holds.
- `tests.rs:1239` (`v.xy = p.xy` into a `vec4`) is a partly written
  varying, which no reference covers. Keep that pin.

### (c) Tests each rule un-ignores (`tests/mali_deqp_varylink.rs`)

| rule | tests | `refusals_pinned` (`:60`) |
|---|---|---|
| E1 | `swz_xx` (`:18`), `discard` (`:24`) | the two `EQUAL` pins leave |
| E2 | `draw_single` (`:30`) | its `EQUAL` pin leaves |
| C | `va_multi3` (`:36`), `draw_multi` (`:42`) | both `NO_ATTRIBUTE` pins leave |
| P | `op_abs_vec2` (`:48`) | the `POSITION` pin leaves, or moves to the next refusal |
| U | `unwritten` (`:54`) | the `VARYING` pin leaves |

`refusals_pinned` is edited with each rule: an entry stays only while its
reference stays refused. E1 and E2 share the `seen` change and land
together. `swz_xx` may still differ in the fragment words (the study: pan_shc's
fragment differs in "the mediump gap", the `isOk` compare), so it may move
to another pin instead of turning green. (`swz_xx` is green since the
fragment walk lowers the `isOk` check, `docs/notes/mali-deqp-intval.md`
section 7: the two lanes of `out0` are one lane after the link, so their
`f2fmp` is one conversion of that lane alone, `nir_fs.rs`
`convert_inputs16`.) `draw_single` is the cleanest: the
study's table says the pan_shc words are byte-equal and only the format
differs.

### (d) What else moves

- `tools/mali/corpus-verdicts.txt`: `va_vec2.vert` (`:125`, E2);
  `vtex_struct_sampler{,_array}.vert` (`:130-131`, C, to their next refusal,
  the struct sampler's vertex look-up). `discard_dynloop.vert` (`:33`) is
  already `ok`.
- `tools/mali/testdata/cases.txt`: each reference that turns byte-equal,
  and `varymerge_equal` and `varymerge_structsampler` (E2 and C targets).
- **glmark2 verdicts** (`tests/fixtures/mali_glmark2_verdicts.txt`): none of
  the four texts is in it, so a correct lowering moves nothing. The
  passing programs at risk if `lane_at` or `seen` is too loose or too
  tight: `gm2_arithconst_3` (three equal colour lanes), `gm2_arithconst_4`
  (late-folded constants, each stored), `gm2_varylink_dup_*`,
  `gm2_varylink_const*` and `varyconst_*` (flat lanes of uniforms).
- **Colourpair** (`docs/notes/mali-deqp-colourpair.md:226-246`): for the
  operator harness (`ck_lane`, `ck_lane_mediump`, `lane_zero_load`: 141 and
  part of 29 rows) Mesa **keeps a vertex constant lane as a varying when
  the vertex shader's local is `mediump vec4`** (`ck_lane_highpcolour`
  drops it). That is a rule about which constant lanes `build` pushes
  (`nir.rs:3484-3498`, `consts.push`), the loop E2 and C edit. Both
  rules say "this constant is not propagated": E2 when it is a component
  of a non-constant vec op, colourpair when it was written through a
  mediump value. Put them in one predicate in `nir.rs`, owned by whichever
  lowering lands first, tested with the other's references (`ck_lane` and
  `draw_single`). They are not the same condition (`draw_single`'s `w` is
  `ffma(1.0, 0.5, 0.5)` of a vec op, folded to `Const` by our walk), so
  check both before merging. The colourpair note's mechanism is
  unverified in Mesa's source.

### (e) Risks

1. **The `Val` to origin change is the whole risk.** `seen`, `lane_at`,
   `own` and the fragment `class` all compare lanes. Changing one and not
   the others lets the stages disagree on the slot count, which shows as a
   wrong varying offset on the board, not as a failure of one case. Guards
   that must stay byte-equal: `fsload_dup`, `fsload_dup_if{,2,3,4}`,
   `fsload_dup_phi` (still refused), `fsload_dup_swz`, `fsload_qual1-4`,
   `gm2_varylink_dup_*`, `gm2_arithconst_3`, `varymerge_equal`.
2. **The `nir_scalar_resolved` reading is unverified.** If Mesa keeps
   `draw_single`'s lanes apart for another reason (the `ffma(x, 0.0, x)` of
   `build_convert_inf_to_nan`, `nir_opt_varyings.c:4963-5003`, or
   mediump), the origin key is wrong for other shapes. Print the link NIR
   before changing the key.
3. **`PositionLane` is a name section M of `docs/es2-cts-failures.md`
   and the corpus carry.** A new variant moves 155 rows to a new text;
   doc-m reads it. `mod.rs:369`, `tests.rs:1129` and any
   `corpus-verdicts.txt` line with the old text move in the same commit.
4. **Flat-slot descriptors** (see E2): more flat vec4s are right on the
   host and wrong on the board until the frame writes an integer format for
   them. The board session should draw `draw_single` and report.
5. **The layout is computed twice** (`nir.rs`, `nir_fs.rs:layout`, with a
   vertex half in `link.rs`), and every rule is a change to both. A unit
   test that lays one vertex shader out both ways and compares slot counts
   would catch a disagreement the references catch only for their own
   shapes.
6. **C and colourpair share a loop** (see (d)).
7. **Sizing.** E1, E2 and C are one change (origin tag plus one placement
   function) and one session. P is independent and can be its own commit.
   U is small and may go to lower-misc.

## Lowering (item lower-varylink, 2026-11-19)

Done at 80403eba. The spike's patch was not in the clone, so the lowering
was derived again from this note and the references. Nothing here was run
on a board.

### The rule: a lane is as divergent, and as constant, as its definition

Mesa's link works on vectors (`nir_lower_alu_width` runs in the back end,
after it), and both of its questions are asked of a **definition**, not of
a lane:

- `nir_vertex_divergence_analysis` marks a definition divergent or not.
  Every component of a vector operation that reads an attribute in one
  lane is divergent, a lane of uniforms alone or a constant included.
- `nir_scalar_resolved` looks through `mov` and `vec`, and stops at an
  operation. A component of a vector operation is its own key for
  `deduplicate_outputs`, and it is no constant for
  `propagate_uniform_expressions`, whatever the back end folds it to
  later.

This one reading gives E2 and C, with no placement function. The study's
step 6 (a convergent lane fills the unused components of an interpolated
vec4) is not what the references show: `va_multi3`'s `z` and
`draw_multi`'s `z` and `w` are lanes of a divergent vector operation, so
they are interpolated from the start. The reading is of Mesa's source
(`nir_opt_varyings.c`, `nir_divergence_analysis.c`); the link NIR was not
printed. It is held by the references below, which is all that tests it.

In the code (`nir.rs` `walk`):

- **`vec_origins`** gives every lane of an IR value an origin: 0 for a
  scalar of its own, or `1 + 4 * value + lane` for a component of a vector
  operation. The origin follows the lane through swizzles, locals and the
  output store (`origins`, `local_origins`, `varying_origins`). An
  operation of one lane, or one whose lanes are all constants (folded), has
  no origin. `a * 1.0` and `a + 0.0`, the constant the same in every lane,
  pass `a`'s origins through (nir_opt_algebraic matches a vector constant
  lane by lane): `varymerge_equal`'s `u * vec3(1.0)` and
  `gm2_arithconst_3`'s `diffuse * Albedo.rgb` of ones still merge.
- **`divergent`**: the varying lanes whose origin is a vector operation
  with a lane that reads an attribute. Such a lane is interpolated: no
  `push_flat`, no `ffma(x, 0.0, x)`, and it counts as `smooth` in `build`.
- **`kept` / `kept_flat`**: a constant lane with an origin is stored by the
  vertex shader and not propagated (`kept`, the mask lower-intval-2 made
  for the mediump case). When its vector operation is of uniforms alone it
  is stored flat (`kept_flat`, with `push_flat` so that the fragment shader
  loads the slot `.src_flat32`).
- **`lane_at` and `seen`** key on (origin, value). Convergent lanes are let
  into the equal classes (Mesa's key ignores divergence), so lanes of one
  uniform value merge; two components of one vector operation never do,
  equal or not.
- The inf-to-NaN conversion of a flat lane is made once per value
  (`nir_opt_cse`): `draw_single` stores one `FMA.f32 r0, r0^, 0x0, r0^`
  three times.
- The origin can only change which of two right programs is built: a lane
  is merged only with a lane of the same value, and a kept constant is
  stored where it would have been propagated. Both stages read the same
  `VaryingConsts`, so they cannot disagree on a slot.

A shader with a late fold (`vs_fmul`, the `AttributeNumbering` family)
keeps its own rule for constant lanes in both of its walks: the new
`kept` rule is off for it. Otherwise dEQP `random.basic_expression.vertex.96`
would compile, which is that family's to decide
(`a_late_fold_is_refused_only_where_the_attribute_numbering_depends_on_it`).

### Two things the references needed beside the link

- **Mesa joins adjacent uniform loads** (`nir_opt_load_store_vectorize`,
  bifrost_nir.c:298-310, in `bi_optimize_late`, which runs on the whole
  vertex shader before the IDVS variants are cut). `u_coordScale` (word 0,
  read by the position program) and `u_colorScale` (word 1, read by the
  varying program) are one two-word load, and each variant that reads a
  word of it pushes both. The FAU is the viewport's six words, both
  uniform words, the position program's constants, then the varying
  program's own copy of both words: 14 words with the pad, and the varying
  program reads `u3.w1`. Modelled in `select.rs` `UboLayout::join_loads`
  and `fau.rs` `push_ubo`: 32-bit rows of whole uniforms only; an
  element's or a column's load is aligned to 4 bytes and joins nothing; an
  fp16 row is left apart (no reference). This is the IDVS_PUSH_RANGE cause
  of `tests/mali_fau_page.rs`: `gm2_faupage_4` is byte-equal now.
- **The constant-up rule on a vector with a constant lane.**
  `vec4(u * color, 1.0) * 0.5` is `fmul(fmul(u, a), 0.5)` in each lane
  once the back end has scalarised it, so `vs_fmul_const_up` now runs for
  every lane but one whose zero only the back end folds (`draw_multi`:
  `u * 0.5` once, then each lane's `FMA`).

`tests/mali_ref` compares a shader that writes gl_PointSize by its
position program without the store (`MaliShader::no_psiz_offset`), with
the secondary offset counted from that program: gl_ref.py draws triangles.

### Shape P

The walk's lane loop gives an undefined lane for any lane-wise operation
with an undefined operand lane (the `Fmul` arm's rule, now for every arm
but the reductions). No lane past an operand's is computed. The
`PositionLane` variant and its text are unchanged: `alu_sized` still
raises it for an undefined operand that reaches it another way, and the
rename would move section M's rows.

### Results

| reference | state |
|---|---|
| `deqp_varylink_swz_xx` (E1) | byte-equal (already) |
| `deqp_varylink_draw_single` (E2) | byte-equal |
| `varymerge_equal` (E2, vertex_arrays arm) | byte-equal, now tested |
| `deqp_varylink_va_multi3` (C) | byte-equal |
| `deqp_varylink_draw_multi` (C) | byte-equal |
| `deqp_varylink_op_abs_vec2` (P) | compiles; the fragment program is Mesa's, the vertex program is fp32 where Mesa's is fp16 (mixprec's vertex side). Pinned as a difference |
| `deqp_varylink_discard` (E1) | refused by the fragment shader's mediump discard (another family) |
| `deqp_varylink_unwritten` (U) | refused, `a varying lane left unwritten`: left to lower-misc |
| `varymerge_structsampler` (C, struct arm) | refused, `a varying lane computed from no attribute`: the texel's lanes beside a uniform that is a scalar of its own. This is the study's step 6 proper, which no dEQP row of the measure needs |

What moved beside them:

- `tools/mali/corpus-verdicts.txt`: `va_vec2.vert` is `ok`.
  `vtex_struct_sampler{,_array}.vert` did not move.
- glmark2: `ideas` vertex shader a09e was the fixture's one "equal lanes"
  row (a row made after the study). Its text (`gm2_cf_ifuni_vs`) and #367's
  reduced shape (`gm2_varylink_dup_uniform`) are byte-equal, and so are
  `gm2_fma_3` and `gm2_fma_6`. The fixture row is `ok`, edited by hand:
  the glmark2 corpus could not be generated in this session.
- `sel_vc_zero` and `uniarray_vs`'s fragment program are byte-equal (the
  colourpair step, `mali-deqp-colourpair.md`).

### Left

- U, and the convergent scalar beside interpolated lanes
  (`varymerge_structsampler`): `flat && smooth` is still refused on both
  sides.
- The flat slot's descriptor format (`RGBA32UI`) is still not produced by
  the frame. `draw_single`'s four flat lanes are now drawn, so the board
  session is the first to show whether `.src_flat32` reads a float
  descriptor correctly.
- The vertex shader's fp16 arithmetic (`op_abs_vec2`): mixprec.

## The references left (item spike-varylink-rest, 2026-11-21)

From 7b2246c0, host only. Two rules close P and most of U. The discard
needs its own lowering.

**P, `op_abs_vec2`: byte-equal.** A mediump `abs` whose computed lanes are
mediump attribute lanes (through a mediump local: `in0 = a_in0.wy`) or
16-bit lanes is fp16, one lane at a time (`nir.rs` `vs_abs16` /
`vs_fabs16`). Each lane is narrowed alone (`FADD.f32 r0.h0, r3^,
0x0.neg`), then its `fabs` is one 16-bit lane, which emit.rs makes
`FADD.v2f16 r0, r0^.abs.h00, 0x0.neg`. The colour's `* 0.5 + 0.5` then
takes the existing two-lane path (`vs_pairs16` / `vs_alu16`, `MKVEC.v2i16
0x0.h0, r0.h0`, the late fusion to `FMA.v2f16`), and each stored lane
gets its own `F16_TO_F32`. The FAU's `3c003c00` (the 1.0 pair of `MKVEC
r0.h0, u5.w1.h0`) comes out as Mesa's.

**U, a varying declared and never written: the link rule.** Each lane of
the varying is an undef (`link::VaryingConsts::push_undef`, tag
`UNDEF_LANE`), carried with the constants to the program's fragment
shader. Mesa does the same: `remove_dead_varyings` leaves its loads
`nir_undef`. The vertex shader stores no slot for it, both alone (the
`own` reads leave it out) and for the program. The fragment shader's load
of such a lane is `Val::Undef` (`nir_fs.rs`, `LoadInput`). The colour join
of an `if` takes the other side's lane (`walk_if16`'s `agree`,
`nir_opt_undef`). Results:

- `deqp_posvary_unwritten_{not_declared,declared_unused,negative_vs}` are
  byte-equal (tested in `tests/mali_deqp_posvary.rs`).
- In `deqp_varylink_unwritten` and `deqp_posvary_unwritten_declared_dead_read`,
  the program's fragment shader is Mesa's 7 words (checked by calling
  `compile_fs_for_vs` directly). The pair is still refused, with `control
  flow` instead of `a varying lane left unwritten`: the fragment shader
  alone (glCompileShader, before any vertex shader is known) selects its
  colour between `vec4(var)` and `vec4(1.0)` under the boolean uniform
  `u_false`. `walk_if16` takes a colour that differs between the sides
  only under a 16-bit float compare. What is still needed is a lowering of
  that select under a 32-bit boolean mask, or a fragment-shader record
  whose refusal waits until the link. Neither has a reference here.

**E1, `discard`: refused, unchanged.** Mesa rewrites `0.0 < x + y` to
`-x < y` (nir_opt_algebraic.py:3663, `fadd(is_used_once)`). The compare's
operands are then the two halves of the colour's own `V2F32_TO_V2F16 r0`
of `v_coords.xy` (v_color.xyz is merged into v_coords by the link):
`DISCARD.f32.lt r0.neg.h0, r0.h1`, before z is narrowed. With the rewrite
done by hand in the source, it is still refused, so three pieces are
missing:

1. the rewrite, after liveness, on a discard's compare;
2. a `cmp_leaf` operand that is a negated varying lane;
3. compare operands that are halves of a pair the colour converts
   (`Leaf::Half`), which `Fs::discard` refuses today as "two varying
   lanes" (`discard16_two_lanes`, where Mesa compares at fp32 because
   `flt(f2fmp a, f2fmp b)` goes back to fp32; the `fneg` stops that rule
   here).

Moved beside them: no glmark2 verdict (69 shaders, 42 compiled, 14 of 33
scenes compile-clean, as before) and no `tools/mali/corpus-verdicts.txt`
line. The unit test `each_mali_link_outcome_is_its_own_line` now asserts
that an unwritten varying has no stored slot, where it used to assert a
refusal.
