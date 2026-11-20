<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 4.2: the texture size uniform words 44-47 and the RET16_OF_32 split

What the code on this tip does with the four texture size uniform words —
the TMU width, height, depth and layer count a `textureSize` or a 3D
`textureGrad` asks the driver for — and with the return-size rule
`RET16_OF_32`. Words 46/47 landed at `298fe11c` (#1261), the size-only
split at `080240bc`. Read from our tree at `e42efea2`; Mesa is 26.2.1.
Line numbers drift; the names are the address.

Sources. `third_party/mesa-26.2.1/` holds the packet XML, the v3d format
table and `src/broadcom/`'s `cle/`, `clif/`, `qpu/`, `common/`, but not the
V3D driver's C: `v3d_uniforms.c` below is a path in the same 26.2.1 tree,
marked "(tree)" (`docs/notes/v3d-shadow.md` "Sources" says where it is).

## (a) What the online compiler emits

`src/shaderc/online/v3d/select.rs:93-99` names the four words with Mesa's
enum values: `QUNIFORM_TEXTURE_WIDTH` 44, `_HEIGHT` 45, `_DEPTH` 46,
`_ARRAY_SIZE` 47. `SOp::TexSizeWord { sampler, lane }` becomes contents
`WIDTH + lane` with `data` the sampler binding (`deferred_loads`,
`select.rs:2131`).

`prepare.rs:1678-1703` (`Op::TexSize`): every size lane loads its word and
minifies it by the shader's level, `max(word >> lod, 1)` (Mesa's
`ntq_minify`). A `sampler3D`'s third lane is the depth word (44 + 2 = 46),
minified like the other two; a `sampler2DArray`'s third lane asks for lane
index 3 — word 47, the layer count — which is not minified ("Don't minify
the array size"). The interpreter run `texture_size_of_a_volume_and_an_array`
(`chain.rs:4431-4439`) pins it: the volume's depth goes 8, 4, 1 at levels
0, 1 and 5, the array's count stays 8.

`textureGrad` emits the same words for another reason (`grad_lod`,
`prepare.rs:970-975`): the TMU has no gradient operand, so each gradient
lane is scaled by the level-0 size on its axis, a 3D sampler's third axis
being word 46. A 2D array's layer count is not a gradient axis: word 47
comes only from `textureSize`.

## (b) How the back end fills the table

`struct v3d_job_tmu` (`driver/src/v3d/v3d_job.h:263-271`) is one sampler
binding: the two record addresses plus `width`, `height`, `depth`,
`array_size` — the values the four words read, 0 meaning "not given", such
a word refused (field doc `:255-262`). `texture_units`
(`src/v3d/backend.rs:5914-6017`) fills one entry per binding and per
stand-in:

- a complete texture: `b.width`, `b.height` and `max(b.layers, 1)`, the GL
  core's level-0 numbers of the stack (`src/gl/draw.rs:4686-4719`; the V3D
  takes them unwrapped, `native_layered_textures`). `depth` is the layer
  count for a `GLES_TEXTYPE_3D`, `array_size` for a `GLES_TEXTYPE_2D_ARRAY`,
  each 1 for any other type (`:5957-5960`);
- an incomplete texture: 1 / 1 / 1 / 1 with the black stand-in record, GL
  ES 2.0 3.7.10 (`:5944-5955`, `:5957-5958`);
- a stand-in (black) unit — one the stream names with a P0/P1 or size word
  but no binding names — also 1 / 1 / 1 / 1 (`:5965-6014`): GL samples an
  unbound sampler as (0, 0, 0, 1).

## (c) How the job builder writes them

`write_uniforms` (`driver/src/v3d/v3d_job.c:432-519`) switches over the
stream; the width/height case (`:475-487`) and the depth/array case
(`:488-500`) look the binding up through `find_tmu` by the word's `data`
(the binding itself, not shifted like P0/P1's unit) and write the field.
A binding the draw did not give, and a field of 0, set `bad_field`, as does
the `default:` (`:512-514`) for every word the builder does not write.
`bad_field` rolls both lists back and answers `V3D_JOB_REFUSED`
(`:1062-1077`), which `src/v3d/backend.rs:3956-3958` maps to
`Attempt::Unsupported`, "the job builder refused the draw (a uniform,
program or primitive it does not write)" — GL_INVALID_OPERATION. The Rust
side never fills 0, so that branch is the C check's:
`tools/v3d/job_check.c:897-929` wants the four words over a two-binding
table to come out `9, 1, 1, 5` (binding 1's depth 9, binding 0's layer
count 5: "FS depth/array word") and must answer both "a depth word for a
binding the draw did not give" and "for a binding given without it" REFUSED,
nothing left. The size words have the same pair at `:864-896`.

## (d) RET16_OF_32, split by whether a look-up reads the unit

`texture::refusal_for` (`src/v3d/texture.rs:375-403`) refuses a bound unit
whose format return size (`return_32`, `:534-536`: R32F, RGBA32F, RGBA32UI,
RGBA32I and the depth textures) disagrees with the return size the program
was compiled for (TMU Config P1 bit 0): the 32-bit format read at 16 is
`RET16_OF_32` (`:354-356`, a backend gap, `GAPS` at `:363`), the other way
`RET32_OF_16`.

The split is in `texture_units`. A unit a look-up reads is named by a P0/P1
word, and the rule above stands for it. A unit that only the size words
name (a `textureSize` lowers to no `Op::Tex`, so no P0/P1 word names it)
has no TMU output to agree with, and `texture_units` checks it against its
own format instead (`:5944-5950`), which always agrees: a size-only
sampler over a 32-bit texture draws; `black_typed_state` keeps the same
test for an incomplete texture read by a 32-bit look-up (`:6034-6037`).

Tests, one point each: `tests/es3_tex_words_v3d.rs` (which shapes carry
46/47, the C cases, the refusing `default:`; control
`controls_without_words_46_47_are_not_refused_that_way`),
`tests/es3_texsize3d_v3d.rs` (the stream per shape;
`the_job_builder_has_cases_for_depth_and_array_size`), and
`tests/v3d_tex_array.rs` (`the_job_tmu_carries_the_depth_and_the_layer_count`:
the entry of (b) — (4, 4, 1, 5) for a 4x4x5 array, (4, 4, 5, 1) for a volume).
`tests/es3_shadow_texsize_v3d.rs` is the 2D and cube half of the exemption;
`tests/es3_shadow_array_texsize_v3d.rs` the array half: a size-only
`sampler2DArrayShadow` of a R32F depth array draws in both stages and its
stream carries word 47 (`the_array_size_program_also_carries_word_47`), a
16-bit look-up stays refused
(`a_16_bit_look_up_of_a_32_bit_depth_array_is_still_refused_ret16_of_32`),
and so do the sampled-and-queried and sampling-only units
(`a_sampled_and_queried_array_shadow_unit_of_a_32_bit_depth_array_keeps_ret16_of_32`).
`tests/es3_sampling_r32f_ret16_v3d.rs`: the core keys a sampling look-up
32-bit from the bound format (`stage_lod_key`, `src/gl/draw.rs`), so a
consistent draw never meets the 16-bit key; its predicate-level pin and
RGBA8 control are there too.

## (e) The base level

The words carry the base level's size, as Mesa's `get_texture_size` does
(it minifies width, height and depth by the sampler view's base level
`u.tex.first_level`, `v3d_uniforms.c:63-72` (tree); the array size stays
unminified, `:73-79`). The GL core hands `GL_TEXTURE_BASE_LEVEL` in
`GlesTextureBind::base_level` (`build_common`, `src/gl/draw.rs`; an
immutable texture's clamped to its last level, ES 3.0.6 3.8.10), and
`texture_units` shifts `b.width`, `b.height` and a 3D texture's depth right
by it, each at least 1; the layer count goes as is (the job builder refuses
0). One spot covers the fragment, vertex and compute units.
`tests/es3_texsize_base_level_v3d.rs` and
`tests/es3_texsize_base_level_vs_v3d.rs` check it at BASE_LEVEL 0 and 1.

Only the size words follow the base level. Sampling still starts at level 0
(the texture state's base level is 0, and the completeness rule counts from
level 0: `mip_levels_needed`, `src/gl/draw.rs`);
`tests/es3_sample_base_level_v3d.rs` pins that.
