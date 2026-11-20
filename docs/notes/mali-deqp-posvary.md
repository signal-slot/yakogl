<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): unwritten gl_Position lanes and uniform-only varyings in dEQP-GLES2 (HOST-ONLY)

Plan #387, item posvary-ref: a study and references, no compiler change.
It covers the three vertex-shader refusals of
`docs/notes/mali-deqp-families.md` section 17 (families `varyuniform` and
`posunwritten`):

| `MaliError` | message | board, 37305608 |
|---|---|---|
| `VaryingNotFromAttribute` | `a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites)` | 252 Fail |
| `PositionLane` | `a gl_Position lane left unwritten` | 155 Fail |
| `VaryingForm` | `a varying lane left unwritten` | 4 (3 Fail, 1 Pass) |

Every count here is HOST-ONLY: `deqp-gles2` on the host against the Mali
back end's library (`tools/mali/deqp-host-measure.sh`), which compiles and
links each shader dEQP writes and reads no pixel. Nothing says a case passes
on the board. The tip is `9ac31f30` (after 22eb4440, the varying link), the
CTS is VK-GL-CTS `93bca01` with the yakogl target, the case list is the 10,597
cases of `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/refusals.tsv`
whose text is a compile refusal (the whole list, not only these texts, so a
case that moved into one of them from another text is counted).

## 1. What the tip does with the board's 411 cases

The board's first refusal is one of the three texts for 411 cases. At the
tip:

| now | cases |
|---|---|
| compile and link (the host verdict `Fail` is the black read-back, no refusal) | 374 (241 + 133) |
| refused by `a fused multiply-add other than ...` (family `367:FMA`, plan #367) | 16 |
| still refused by one of the three texts | 21 (11 + 6 + 4, of which 1 is a `Pass` case) |

Cases that reach one of the texts at the tip from another board text: 21 (14
from `an integer or boolean value`, 4 from `two varying lanes of equal value`,
2 from `a constant product in a shape Mesa's algebraic rules move or fold`,
1 from `a shader with more than 32 local variables`; the first refusal moved
because the compiler got further). So **42 cases carry the three texts at
the tip**: 32 `computed from no attribute`, 6 `gl_Position lane`, 4 `varying
lane left unwritten`. 40 of the 42 are board `Fail`, one is `NotSupported`
(`tmp_array...read_vertex`) and one is a `Pass` (a negative case that fails
to compile by design). The two big groups of the board's count are gone:
`vertex_arrays.multiple_attributes` (203 cases) and most of `shaders.operator`
no longer reach these texts.

The 16 `367:FMA` cases (`angle_and_trigonometry.{acos,asin,atan,atan2}.{highp,
mediump}_{vec2,vec3}_vertex`) have the same vertex shader as the six `fract`
cases of shape `fract_local` below (`in0 = a_in0.wy; res = f(in0);
color.yz = res`). Lowering the FMA family can expose them to the same
refusal: treat `fract_local` as up to 22 cases.

## 2. The shapes (HOST-ONLY, 42 cases)

The shape is the refusal's cause, found by probing (section 4), not by the
refusal text. A shape is named after its representative reference
(`tools/mali/testdata/deqp_posvary_<name>.{vert,frag,expect.json}`).

| shape | cases | dEQP groups | references |
|---|---|---|---|
| `flat_sep`: a varying that is a uniform expression on its own (a scalar or a vec4 of uniform and constant lanes), beside smooth varyings of attribute lanes | 16 | `fragment_ops.interaction.basic_shader.{26,27,29,41,51,96}` (6), `shaders.random.basic_expression.combined.{13,16,27,33,47,66,77}` (7), `shaders.random.conditionals.combined.75`, `shaders.random.scalar_conversion.combined.{46,84}` | `flat_sep_rnd13`, `flat_sep_kau_bs26`, `flat_sep_vec4_bs51` |
| `flat_lane`: uniform lanes inside a vec4 (or vec3) whose other lanes come from attributes | 13 | `shaders.random.basic_expression.combined.{12,30,49,53,86}` (5), `shaders.random.basic_expression.vertex.{27,46,86,89,93}` (5), `shaders.random.scalar_conversion.combined.{15,18,87}` (3) | `flat_lane_vec4_v86`, `flat_lane_vec4_c30` |
| `fract_local` (`gl_Position lane`, misnamed): `fract` of a local copy of an attribute swizzle written into lanes of a mediump colour | 6 | `shaders.operator.common_functions.fract.{highp,mediump,lowp}_{vec2,vec3}_vertex` | `fract_{highp,mediump,lowp}_{vec2,vec3}` |
| `unwritten`: a varying the vertex shader declares and never writes | 4 | `shaders.linkage.varying_{2,3,4}`, `shaders.declarations.invalid_declarations.varying_in_fragment_main` (`Pass`) | `unwritten_not_declared`, `unwritten_declared_unused`, `unwritten_declared_dead_read`, `unwritten_negative_vs` |
| `tex_uniform_w`: `vec4(texture2D(...).rgb, uniform)` | 2 | `shaders.struct.uniform.sampler_vertex`, `sampler_array_vertex` | `tex_uniform_w`, `tex_uniform_w_array` |
| `loop_phi`: a varying from a loop over uniform bounds and a constant array | 1 | `shaders.indexing.tmp_array.float_const_write_dynamic_loop_read_vertex` (`NotSupported` on the board) | `loop_phi_array` |
| total | 42 | | 18 references |

`flat_sep` and `flat_lane` are 29 of the 42 and both stop at the same
test, `flat && smooth` (section 4); the single-lane differences between them
are what the Mesa references show (section 3). The split of the 29 into the
two shapes and of the cases into `flat_sep` and `flat_lane` was made by
reading each vertex shader's final varying assignments (a lane is a uniform
lane when it is a uniform, a uniform expression, or a constant mixed with
one; an attribute lane when an attribute reaches it): it is a reading, not a
compiler output. The test pins the refusal of the 18 representatives only.

## 3. What Mesa 26.2.1's link does (the references)

Method (one reference): the dEQP vertex and fragment shaders of the
representative case, drawn once with `tools/mali/drm_shim_ref.sh run <mesa
work dir> <vs> <fs> deqp_posvary_<name> --uniform-values` (Mesa 26.2.1
panfrost on drm-shim, GPU `g610`, G310 words are identical,
`mali-g310-vs-g610.md`) and
`python3 -I tools/mali/pan_ref_diff.py extract <trace> --draw` for the
`expect.json` (`tools/mali/deqp_ref.sh --instrument shim` does the two in
one; its `MESA_SHIM_WORK` needs the `inst/` of a shim build, here
`mesa-pan/inst` from `drm_shim_ref.sh build`'s meson command). Both stages
are in each reference because the link moves things between them. The `.vert`
and `.frag` of a reference are the dEQP shaders verbatim (the `.qpa`),
except: `tex_uniform_w` and `tex_uniform_w_array` (gl_ref.py cannot set a
struct that holds a sampler, `--uniform-values does not set a S uniform`; the
struct's members are flattened to uniforms of the same order, the program's
varying shape is the same), and `unwritten_negative_vs` (the dEQP fragment
shader of that case does not compile, so the pair never links; its vertex
shader is paired with a trivial fragment shader).

Region sizes of the references (words; `vs.position`, `vs.varying`,
`fs.fragment`) and the link's layout as the shim's final NIR gives it:

| reference | words | the link's result |
|---|---|---|
| `flat_sep_rnd13` | 24 / 15 / 9 | the fragment shader reads `e = float(uniform)` too, yet loads only the smooth vec4 `c` (`LD_VAR_BUF_IMM.v4`): the bare uniform copy is not a varying any more, the fragment shader reads the uniform itself (9 words, the FAU) |
| `flat_sep_kau_bs26` | 24 / 27 / 12 | `e = j + 4.0` (an operation on a uniform) stays a varying: a smooth vec4 at offset 0 and a flat float (`LD_VAR_BUF_IMM.flat32`, index 16) |
| `flat_sep_vec4_bs51` | 24 / 34 / 21 | a smooth vec4 and a flat float, the same two loads |
| `flat_lane_vec4_v86` | 24 / 14 / 11 | `v_color = vec4(c, c, b, a - 5.0)`: the varying shrinks to a vec3, one `LD_VAR_BUF_IMM.v3`; the two uniform lanes left the varying |
| `flat_lane_vec4_c30` | 24 / 16 / 15 | the vec4 `d` with a uniform lane and constants: a vec3 smooth varying |
| `tex_uniform_w` (and `_array`) | 24 / 28 / 7 | one smooth vec4 load; the uniform alpha is a smooth lane of the same vec4 (no flat slot) |
| `loop_phi_array` | 104 / 109 / 7 | one smooth vec4; the whole loop is in the varying program |
| `fract_*` | 24 / 29 to 32 / 7 | one smooth vec4 of the colour; the `vs.varying` words are fp16 narrowings of the fract results (sizes differ by precision and vec2/vec3) |
| `unwritten_*` | 24 / (none) / 7 | no `vs.varying` region at all: the varying is removed, the fragment shader is the 7-word constant colour |

What it means (the first three rows repeat what `mali-varying-constants.md`
and `mali-partial-varyings.md` established for the constant case, now seen
for uniforms):

* A varying whose value is a **bare uniform copy** is moved into the
  fragment shader (the fragment shader reads the uniform), not stored.
* An **operation on uniforms** is not moved: it stays a varying and the
  fragment shader loads it flat (`.flat32`), after the smooth ones. This is
  the pair the compiler's `uniform_only` lane (`nir.rs` `Kind` of
  `uniform_only`) sets `flat` for and then refuses beside a smooth lane.
* **Constants** are propagated into the fragment shader and not stored
  (already lowered: `p02`, `p06`, `p15` of the probes, section 4).
* A varying the vertex shader **never writes** is removed whether or not
  the fragment shader declares it, as long as the fragment shader's
  reads of it are dead (`varying_4` reads it in a branch on a uniform that
  the link cannot fold, and the program still has no `vs.varying`); an
  unwritten lane inside a vec4 that the fragment shader reads (probe `p20`)
  has no reference yet.
* Which GLSL lane lands in which slot of `flat_sep_vec4_bs51` and the two
  `flat_lane` references is to be read off each reference's NIR
  (`BIFROST_MESA_DEBUG=shaders` in `drm_shim_ref.sh`'s `.stderr`) by the step
  that lowers it; the byte compare decides it.

## 4. The causes at the tip, by probe

All probes were compiled with `tests/mali_ref::compile_pair` (a scratch
test, not committed; vertex shader of the form `gl_Position = p; <body>`,
fragment shader reading every varying; `a` an attribute vec4, `u` a
uniform float, `ul` a uniform vec4).

`VaryingNotFromAttribute` (`nir.rs` of `build`): the refusal is raised at
five sites; for the 32 cases it is the last test of the varying loop,
`if flat && smooth` (`nir.rs:5494`), where `flat` is set by a lane of
uniforms alone (`uniform_only`, `:5469`) and `smooth` by any lane that comes
from an attribute or a texture. Probes:

| probe | vertex body | result |
|---|---|---|
| `p04` / `p09` / `p11` / `p16` | only uniform and constant lanes (`vec4(u,u,u,u)`, `f = u`, `vec4(u, 0.5, u, 1.0)`, `g = ul`) | compiles |
| `p02` / `p06` / `p15` | attribute lanes with constants (`vec4(a.xy, 0.5, 1.0)`, `f = 0.75; g = a`) | compiles |
| `p01` / `p03` / `p05` / `p07` / `p12` / `p17` | a uniform lane or uniform expression (`u`, `u * 2.0 + 1.0`, `ul`) beside an attribute lane, in one vec4 or in two varyings | refused |
| `p13` | two different constants beside an attribute vec4 | refused by another text (`mixprec`) |

So the 29 cases of `flat_sep` and `flat_lane` are one rule: flat beside
smooth. The dEQP programs also contain constants beside them; probe `p12`
(uniform, constant and attribute together) is refused for the same reason.
The other sites are for `tex_uniform_w` (a lane that is flat beside a `tex`
divergent lane, same `flat && smooth`) and `loop_phi` (the lane of a loop's
phi with no attribute, no `tex` and no indexed read: the arm
`!reads_attribute && !reads_tex && !reads_indexed`, `:5470-5474`), the
latter confirmed only by the lane's form, not by an instrumented run.
`mod.rs:1839` (`vs_constants` when the constants table is full) and the
mirror in `nir_fs.rs:7992` (`flat && smooth` of the fragment loads) are the
same rule on the other side of the link: the step that lifts the vertex
refusal has to lift the fragment one with it.

`PositionLane` is misnamed. `gl_Position` is fully written in all six cases
(`gl_Position = a_position`). The text is the one error `alu_sized` returns
when an operand of an operation is `Val::Undef` (`nir.rs:2120`). Probes on
the `fract` vertex shader (`in0 = a_in0.wy; res = fract(in0); color.yz =
res; v_color = color`):

| probe | change | result |
|---|---|---|
| `f01` the dEQP shader, `f02` without `* 0.8 + 0.1`, `f05` `.xy`, `f06` `color.xy = res` | | refused |
| `f03` | `vec4(0.0, fract(a_in0.wy), 1.0)`, no local | compiles |
| `f04` | `floor` for `fract` | compiles |
| `f11` | `fract(a_in0.wy)` into `res` with no `in0` local | compiles |
| `f12` | `highp vec4 color` | compiles |
| `f07`, `f08`, `f09`, `f10` | scalar, vec4, vec2 straight into the constructor, no `fract` | compile |

So the `Undef` operand arises from the `fract` of a **local vec2 copy** of an
attribute swizzle, written by lanes into a **mediump** vec4 (the highp
`f2f16` narrowing of the lanes written separately). It is in the vertex
walk's fp16 handling (family `mixprec`, vertex side), not in gl_Position.
The first action of the step is an `eprintln!` at `nir.rs:2120` naming the
operation; the text is to be renamed in the same change (it is wrong for
every other `Undef` operand too).

`VaryingForm` (`nir.rs:5421`, `:5437`, `motion.rs:216`, `:225`): the
`varyings` table has no entry at the vertex shader's location of a declared
varying that no statement stores to (`unwritten`). Probes `p18` to `p21`:
unwritten alone (`f` never stored, the fragment shader constant), unwritten
and read by the fragment shader, and a lane left out of a vec4 that the
fragment shader reads (`v.xy = a.xy`) are all refused. Mesa removes the
first two (the references have no `vs.varying`); the third has no reference
(no dEQP case in the 42 reaches it; `p20`, `p21` are shapes for the step to
draw before lowering the lane case).

## 5. What `tests/mali_ref` can compare

`tests/mali_deqp_posvary.rs` pins every reference as refused with its whole
message (`refusals_pinned`) and keeps one `#[ignore]`d byte-compare test per
reference; a lowering step moves its references to a plain `#[test]`. The
compare reaches the three regions of a draw, so these can be compared as
they stand: all `flat_sep`, `flat_lane`, `fract_*`, `loop_phi`, `tex_*`, and
the four `unwritten_*` ones, whose reference has no `vs.varying` region:
`mali_ref::differences` already handles that (it fails a pair that has a
varying program or a `secondary_offset` where the reference has none).

What it cannot reach, from `mali_ref/mod.rs`'s header and
`mali_deqp_intval.rs`'s `rsg_cond_fs`: the unit tests' `compile_linked`
path (the vertex shader's constant lanes through `vs_constants` and
`compile_fs_for_vs`, private) and the fragment shader of a program whose
varyings Mesa's link removed as dead. Of these shapes, `flat_sep_rnd13`
(the uniform copy that leaves the varying; the fragment shader reads the
uniform) depends on the first (`fs_constants`-style move of a flat lane,
`link.rs` `VaryingConsts::push_flat` / `at_fragment`): `mali_ref` runs the GL
core's link, which does call it, so it is expected to be comparable, and the
first run after lowering decides; if it is not, the unit tests of
`src/shaderc/online/mali/tests.rs` (`check_linked_for`) carry it, as for
`rsg_cond_fs`. The six `fract_*` and the two `tex_*` need nothing of the
link.

## 6. Lowering map and steps (ranked by cases)

Counts are the cases a step can unblock at most (HOST forecast: the first
refusal after it can be another one). The files are
`src/shaderc/online/mali/`.

1. **`flat_sep`, then `flat_lane`: a flat lane beside smooth ones** (29
   cases). `nir.rs` `build` (the loop of `:5421-5500`): the `flat && smooth`
   refusal (`:5494`) becomes the layout the references show: smooth
   varyings first, flat lanes after (`compact_layout`, `nir.rs:5650`;
   `generic_layout` in `select.rs:151`), a flat lane stored with the
   `ffma(x, 0.0, x)` conversion (the code after `:5540`, already there for
   the all-flat case); the bare uniform copy is moved to the fragment
   shader (`link.rs` `VaryingConsts::push_flat` and `at_fragment`, `mod.rs`
   `compile_fs_for_vs`/`vs_constants`, `:1839`), the operation on uniforms
   stays flat. `nir_fs.rs:7992` (`flat && smooth` of the loads): the fragment
   loads, `LD_VAR_BUF_IMM.flat32` after the smooth `.v4`/`.v3`. Sub-steps, one
   session each: (a) `flat_sep_rnd13`, `flat_sep_kau_bs26` (a scalar flat
   varying beside one smooth vec4: 12 of the 16 `flat_sep` cases are of this
   form by reading); (b) `flat_sep_vec4_bs51` and the other whole-vec4
   flat cases; (c) `flat_lane_*`, the lanes inside a vec4 (13 cases:
   `nir.rs` `compact_layout` and the partial-vec4 `masks` with the
   shrinking to a vec3 of `flat_lane_vec4_v86`).
2. **`fract_local`** (6, up to 22 with the 16 `367:FMA` cases that share
   the shape). `nir.rs:2120` `alu_sized`: find the `Undef` operand of the
   fp16 narrowing of a lane write into a mediump vec4 (`color.yz = res`
   where `res` is `fract(in0)` of a local); the vertex walk's `vec16.rs`
   (the fp16 pairs) or the local-variable lane writes (`nir.rs` locals) is
   the likely place. Rename `PositionLane`'s message for the other users.
   Six references, all precisions.
3. **`unwritten`** (4; 3 Fail). `nir.rs:5421` and `:5437`, `motion.rs:216`
   and `:225`: a declared varying with no store is removed from the
   layout (Mesa: no `vs.varying`, the fragment shader's reads of it are
   dead or its declaration unused). The three real cases, then
   `unwritten_negative_vs` (a constant `gl_Position = vec4(1.0)` with the
   varying unwritten: pinned only; its dEQP case is a `Pass` whatever the
   vertex shader does).
4. **`tex_uniform_w`** (2): after step 1, the `flat && smooth` rule with a
   texture lane; the uniform alpha is one lane of the smooth vec4 in Mesa
   (the references show a single smooth load), so it is the *smooth* side of
   the rule that needs the uniform lane, the opposite of step 1.
5. **`loop_phi`** (1, `NotSupported` on the board): the `!reads_attribute`
   arm for a loop's phi (`nir.rs:5470`): store it as Mesa does, in the
   varying program with the loop (`vs.varying` 109 words).

After steps 1 to 3 the three texts are gone from the dEQP-GLES2 refusal
table except `tex_uniform_w` and `loop_phi`; as the board's count of
the three texts was 411, the 374 that compile already at the tip are
what the next board rerun turns into passes or fails by pixels, not these
steps.

## 7. Reproducing this note

```sh
# the measure (HOST-ONLY), CTS 93bca01 with fetch_sources.py and the yakogl target
tools/mali/deqp-host-measure.sh --stage build --cts $DEQP_WORK --out $OUT
tools/mali/deqp-host-measure.sh --stage run --cts $DEQP_WORK --out $OUT --jobs 4
tools/mali/deqp-host-measure.sh --stage extract --cts $DEQP_WORK --out $OUT
# the 42: measure.tsv rows whose host_refusal holds one of the three texts;
# the shaders of each case: the ShaderSource elements of $OUT/c_*.qpa
# the references (one):
tools/mali/drm_shim_ref.sh run <mesa work dir> v.vert f.frag out/deqp_posvary_<name> --uniform-values
python3 -I tools/mali/pan_ref_diff.py extract out/deqp_posvary_<name>.trace --draw \
    > tools/mali/testdata/deqp_posvary_<name>.expect.json
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_posvary
```

Caveats. The host run measured the tip `9ac31f30` with the same library as
the board's `gpu-mali shaderc-online`; the CTS was fetched as a one-commit
checkout of 93bca01 and `fetch_sources.py`. The host build drew the Mesa
references with `drm_shim_ref.sh`'s meson options on this host (gcc 15,
LLVM 21, no `LLVM_DIR`); Mesa's output does not depend on them. The
`expect.json` of each reference was drawn once; the method of the other
`deqp_*` references (two draws byte-identical) was not repeated.
