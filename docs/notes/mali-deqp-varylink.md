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
`tools/shaders/zz_varylink_probe/`. They were compiled through
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
   - The alone record of `es100/va_vec2.vert` and of the swizzle
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
