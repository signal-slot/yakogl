<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali loops: Mesa references for the unreferenced S8 siblings (plan #387 item arith-lower-1)

Step S8 of `docs/notes/mali-loops-lowering.md` (vertex kept loops with float
counters, `LOOP_CAP`) had Mesa references for only six of the
`functional.shaders.loops.*_vertex` `basic_*_float` shapes. This item draws the
nine that were missing. No compiler code changed.

The sources are the generator's text (`es2fShaderLoopTests.cpp`, VK-GL-CTS
`93bca01`). The same generator reproduces the existing
`deqp_loops2_vs_{wh_dyn_basic_highp,for_dyn_basic_mediump,do_uni_basic_mediump,
do_uni_basic_highp,for_uni_basic_lowp}_float` vertex shaders byte for byte, so
the new ones are the template with another loop kind, bound or precision. The
fragment shader is the constant-colour pass-through of the loops2 pairs. No
dEQP checkout was available on the drawing host (only the `data/` of
`VK-GL-CTS-gles3`), so the texts were not read from a `.qpa`: the check above
is the evidence. The `.license` files say so.

References are `tools/mali/testdata/deqp_loops4_<shape>.{vert,frag,expect.json}`,
drawn with `tools/mali/deqp_ref.sh <vert> <frag> <prefix> --instrument shim`
(the shim built into `$CONNECTED_DEPS_DIR/mesa-shim` by
`tools/mali/drm_shim_ref.sh build`, Mesa 26.2.1) and pinned by
`tests/mali_deqp_loops4.rs`.

| reference | dEQP case (`functional.shaders.loops.`) | loop | words (position / varying / fragment) | refusal at the tip | S8 sub-step |
| --- | --- | --- | --- | --- | --- |
| `vs_wh_uni_basic_lowp_float` | `while_uniform_iterations.basic_lowp_float_vertex` | `while`, uniform bound, lowp counter | 24 / 24 / 8 | `LoopVertex` | `vs_kept_head` float counter, 16-bit lane (`:4184`, `:4213`) |
| `vs_wh_uni_basic_mediump_float` | `while_uniform_iterations.basic_mediump_float_vertex` | same, mediump | 24 / 24 / 8 | `LoopVertex` | same |
| `vs_wh_uni_basic_highp_float` | `while_uniform_iterations.basic_highp_float_vertex` | same, highp counter | 24 / 24 / 8 | `LoopVertex` | `vs_kept_head` float counter; the carried `res` is still 16-bit |
| `vs_for_dyn_basic_lowp_float` | `for_dynamic_iterations.basic_lowp_float_vertex` | `for`, bound `uf_one*one` from `a_one` | 24 / 27 / 8 | `LoopVertex` | float counter plus the attribute product in the head |
| `vs_for_dyn_basic_highp_float` | `for_dynamic_iterations.basic_highp_float_vertex` | same, highp | 24 / 26 / 8 | `LoopVertex` | same |
| `vs_wh_dyn_basic_lowp_float` | `while_dynamic_iterations.basic_lowp_float_vertex` | `while`, dynamic bound | 24 / 27 / 8 | `LoopVertex` | same |
| `vs_wh_dyn_basic_mediump_float` | `while_dynamic_iterations.basic_mediump_float_vertex` | same, mediump | 24 / 27 / 8 | `LoopVertex` | same |
| `vs_do_uni_basic_lowp_float` | `do_while_uniform_iterations.basic_lowp_float_vertex` | `do`-`while`, uniform bound | 24 / 24 / 8 | `LoopVertex` | the rotated form (`KeptForm.rotated`) with the float counter |
| `vs_for_uni_basic_mediump_float` | `for_uniform_iterations.basic_mediump_float_vertex` | `for`, uniform bound | 24 / 24 / 8 | `LoopVertex` | float counter (the lowp sibling `deqp_loops2_vs_for_uni_basic_lowp_float` has the same word count) |

The refusal is `MaliError::LoopVertex` ("a loop in a vertex shader") for all
nine, asserted by `refusals_pinned`. `compares` holds the `#[ignore]`d byte
compares; they become `CapEdit` compares (see `tests/mali_deqp_loops.rs`) when
S8 lands, and the entries move from `REFUSALS` to `MATCHED` in that commit.

The word counts of the varying program follow the bound, not the precision:
24 for a uniform bound, 26 (highp) or 27 (lowp, mediump) for a dynamic one, the
same as the loops2 siblings (`do_dyn_basic_highp` 26, `for_dyn_basic_mediump`
27). The position program is 24 words in every shape (no loop in it).

## What S8 can take from this

- Uniform-bound shapes (the first, second, third, eighth and ninth rows) have
  the same word count as `deqp_loops_vs_for_uni` and `vs_do_uni_*`: one S8
  float-counter lowering should serve all precisions and loop kinds, the
  `do`-`while` through the rotated form.
- The dynamic-bound shapes differ from the uniform ones by the `a_one`
  multiplies; their precision decides 26 against 27 words.
- Not drawn here (not named by the item): fragment variants, `do_while_dynamic`
  lowp/mediump (highp is `deqp_loops2_vs_do_dyn_basic_highp_float`), and the `basic_mediump_int` counters.
