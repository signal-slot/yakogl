<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the "texture coordinate" refusal of the G310 run, its forms, Mesa's words and the lowering order (design)

**Status: design, no code.** Plan #387, item `texcoord-design`. Nothing under `src/` changed. The
new references are `tools/mali/testdata/deqp_texcoord_*` (15 pairs), pinned by
`tests/mali_deqp_texcoord.rs`. They fill the gaps that the earlier design leaves:
[`mali-fmaloc-lowering.md`](mali-fmaloc-lowering.md) (step T1, its four `deqp_fmaloc_tc_*`
references, `tests/mali_deqp_fmaloc.rs`). That design stays the owner of the cube-face mechanism, and
this note does not repeat it. The references and forms were measured HOST-ONLY at **`6e9e515c`**, and
the case counts and sites again at main **`56b5878a`**, after `lower-train-2` (`a27396fa`) landed.
Nothing here was run on a board. The board rows it starts from are the Verdin iMX95 (Mali-G310) ones. Mesa paths
are relative to `src/` of the 26.2.1 tree.

## 1. The text, the cases, the sites

The text is `MaliError::TexCoord` (`src/shaderc/online/mali/mod.rs:484`): "a texture coordinate
other than a vec2 varying read whole or two constants".

- **Board lines.** The consoles of the Verdin iMX95 whole-tree run
  (`tools/cts/log/2026-11-2{4,5}-imx95-gles2-*-e63ab28e`) hold **202** lines of this text.
  `refusals.tsv` of those slices records **186** cases, all `Fail` and all must-pass
  (`tools/cts/gles2-mustpass.txt`). The plan's "202 G310 lines" is the line count; this note
  counts cases. The rerun at `a3d8d2de` (item `constprod-lower-1`, in review) is not in the tree.
  Its records were not available here.
- **Groups of the 186.** `texture.specification` 86 (the `*_cube` cases of `basic_teximage2d`,
  `basic_texsubimage2d`, `texsubimage2d_empty_tex`, `{teximage2d,texsubimage2d}_align`,
  `random_teximage2d`), `uniform_api` 67 (`value` 52, `info_query` 11, `random` 4: every
  `samplerCube_fragment` / `sampler2D_samplerCube_{fragment,both}` case),
  `shaders.random.texture.fragment` 29, `shaders.discard.*_texture` 4.
- **Host measure.** `tools/mali/deqp-host-measure.sh --cts $CONNECTED_DEPS_DIR/VK-GL-CTS-host
  --cases <the 186> --slice <the six e63ab28e slices>`, using the deqp-gles2 that
  `deps-env.sh --build HOST_DEQP_CTS` built (VK-GL-CTS `93bca018`). All 186 cases began. At
  `6e9e515c` all 186 had this text first, in the fragment stage, case for case as on the board.
  At `56b5878a` **184 have it first**. The other two, `shaders.discard.static_loop_texture` and
  `function_static_loop_texture`, now get past the coordinate (`lower-train-2`'s TEX_COORD2
  `varying_coord_tree`) and meet `Defect(IrIndex)` first. That is the text the bypass probe of
  `6e9e515c` showed behind this check for them (section 2, F5).
- **Refusal sites.** A scratch probe, not committed, made every `Err(MaliError::TexCoord)` of
  `nir_fs.rs` print its line, with the line numbers left unchanged. It then compiled the measure's
  58 distinct programs (`programs.tsv`) through `tests/mali_ref`'s `compile_pair` at `6e9e515c`.
  Between `6e9e515c` and `56b5878a`, `cube_dir_measured` and the two cube arms are unchanged, and
  the only change at the select site is `varying_coord_tree` (`a27396fa`). So the two cases that
  left the text came from that site. The lines below are those of `56b5878a`:

| Site at `56b5878a` (`6e9e515c`) | Arm | Cases | Groups |
|---|---|--:|---|
| `nir_fs.rs:8529` (8396) | the mediump `textureCube(s, dir)` arm (implicit level): `cube_dir_measured` is false | **162** | `texture.specification` 86, `uniform_api` 67, `random.texture` 9 |
| `nir_fs.rs:8492` (8359) | the mediump `textureCube(s, dir, bias)` arm: `cube_dir_measured` is false | **16** | `random.texture` 16 |
| `nir_fs.rs:12960` (12539) | `select`'s lone computed 2D coordinate: two different lanes, and neither `frag_coord_tree` nor (since `a27396fa`) `varying_coord_tree` holds | **6** (8 at `6e9e515c`) | `shaders.discard` 2 (`basic_texture`, `function_texture`), `random.texture` 4 |
| **Total** | | **184** (186) | |

No other `TexCoord` site is reached. `nir_fs.rs` has 13 such returns; the other ten (at `56b5878a`:
8436, 8564, 8601, 8606, 8645, 8693, 8720, 8729, 10387, 12608) see no dEQP-GLES2 case of this run. Each of the 19
references the two test files pin stops at the site of its group: the four `deqp_fmaloc_tc_*` at
8529 (`tc_2d_scalar` at 12960), and the 15 `deqp_texcoord_*` as in section 2.

## 2. The forms and the references

There are 58 distinct programs; 15 new references and the four earlier ones cover the forms.
Counts are cases whose first refusal is this text (F5 at `56b5878a`, the rest unchanged since
`6e9e515c`). Site numbers are those of `56b5878a`. "Next" is the text the case meets once the
site's check is **bypassed**: a second scratch probe replaced the three checks by
`dir.contains(&Val::Undef)` and `false`. That is not a lowering. It shows only what is behind
the check.

| Form | Site | Cases | References (`deqp_*`, words of `fs.fragment`) | Next text with the check bypassed |
|---|---|--:|---|---|
| **F1** cube face `vec3(±1, ±y, ±x)` of `v.xy*2-1` | 8529 | 86 | `fmaloc_tc_cube_face` (fmaloc 1.1) | `gl_FragColor` lane 86: the select does not build the direction (fmaloc 3.1 mechanism 1 is needed, not only the predicate) |
| **F2** direction of constant and uniform words only (`vec3(0.0)` of `uniform_api`; a folded random direction) | 8529 | 69 (uapi 67, random 134, 22) | `fmaloc_tc_cube_const` (28), `texcoord_uapi_struct_cube` (38), `_uapi_struct_cube_both` (40), `_uapi_array_in_struct` (104), `_uapi_struct_in_array` (82), `_uapi_unused_cube_elem` (38), `_rnd_add_const` (18), `_rnd_swizzle_const` (19) | compiles 9 (uapi 7, random 2); `gl_FragColor` lane 40 (the harness over two or more look-ups); `TexArrayElement` 20 (a cube array element other than the first) |
| **F3** mixed direction lanes (sums of varyings, a vector of a uniform, an `int()` and a varying, `gl_FragColor` lanes) | 8529 | 7 (random 111, 16, 51, 38, 82, 24, 2) | `fmaloc_tc_cube_rnd_sub`, `texcoord_rnd_int_lane` (19) | `gl_FragColor` lane 4, `FsOperandForm` 1, `TexKind` 1, this text 1 (111, at a later site) |
| **F4** the same direction forms with a bias | 8492 | 16 (random) | `texcoord_rnd_bias_const` (30), `_rnd_bias_ivec` (26), `_rnd_bias_mul` (26) | `TexKind` 16: every bias here is a constant or a computed value, not a uniform lane |
| **F5** computed 2D coordinate of two different lanes | 12960 | 6 (discard 2, random 11, 12, 43, 85; 8 at `6e9e515c`) | `fmaloc_tc_2d_scalar`, `texcoord_disc_texture` (17), `_rnd_2d_const_sub` (14), `_rnd_2d_splat_mul` (14), `_rnd_2d_vec_sub` (15) | compiles 2 (43, 85); `Defect(IrIndex)` 1 (random 12); `FsVaryingLoad` 2 (discard); `FsOperandForm` 1 (11). At `6e9e515c` the two discard static-loop cases also stopped here, with `Defect(IrIndex)` behind the check; since `a27396fa` they meet that text first |

With the checks bypassed, four of the shapes that compile were compared with their references
(`mali_ref::differences`):

- `fmaloc_tc_cube_const`, `texcoord_rnd_swizzle_const` and `texcoord_rnd_2d_const_sub` are
  **byte-equal**. For them the check is the only thing missing: the select already emits
  Mesa's cube rows over constant and FAU operands, and the fp16 pair `FADD.v2f16`.
- `texcoord_rnd_2d_splat_mul` **differs** in the fragment program: Mesa writes `vec2(2,2) * l` as
  `FADD.v2f16 r0, r0^, r0^` (`fmul x, 2.0 -> fadd x, x`).
- `texcoord_rnd_add_const` **differs** in `vs.varying`: we emit a varying program and Mesa none.
  It is a vertex-side difference, not the coordinate.

The uapi pairs with several look-ups stop at the `gl_FragColor`-lane text even with the check
bypassed (`texcoord_uapi_struct_cube`, measured).

Every `deqp_texcoord_*` reference is IDENTICAL on the G310v2 and the G610 (a G310 check on Mesa
26.2.1's drm-shim for both GPUs, as `tools/mali/g310_check.sh` does: 15 identical, 0 differ).

## 3. What Mesa emits for each form

From the `fs.fragment` text of each `expect.json`:

- **F1** (fmaloc 1.1, `tc_cube_face`): the fp16 pair `FMA.v2f16 .., 2.0, -1.0`, the constant lane as
  a `CUBEFACE1` operand, then `build_cube_desc`: `CUBEFACE1`, `CUBEFACE2_V9`, `CUBE_SSEL`,
  `CUBE_TSEL`, `FRCP.f32`, `FMA.f32 0.5*rcp`, two `FMA.f32.clamp_0_1 .., 0.5`, `MUX.i32.bit`, and
  `TEX_SINGLE .cube.computed`.
- **F2**: **no fold.** The cube rows take the constant words and FAU words as operands
  (`CUBEFACE1 r1, 0x0, 0x0, 0x0`; `rnd_swizzle_const`'s `CUBEFACE1 r0, u2.w0, 0x3F800000,
  0x3F800000`; `rnd_add_const`'s `IADD_IMM.i32 r0, 0x0, #0xC0700000` then `CUBEFACE1 r1, r0,
  0x3F800000.neg, u2.w0`). Where the folded constant cannot be an operand, the `MUX.i32.bit` mask
  is a constant `IADD_IMM.i32 .., #0x1FFFFFFF`. If the sampler is the only one, its descriptor
  word is an immediate (`IADD_IMM.i32 r2, 0x0, #0x20001800`); otherwise it is a FAU word. With
  several cube look-ups at `vec3(0.0)`, **one** cube sequence is computed and each later look-up
  copies its two words (`MOV.i32 r4, r2`, `MOV.i32 r5, r1`): Mesa's CSE
  (`uapi_array_in_struct`, `uapi_struct_in_array`). The 2D look-ups at `vec2(0.0)` beside it are
  `MOV.i32 r0, 0x0`, `MOV.i32 r1, 0x0`, `TEX_SINGLE .2d.computed`, the form
  `TexCoord::Const` already has. A cube array element other than the first (`u_var.m3[1]`) is only
  another descriptor FAU word (`MOV.i32 r4, u3.w1`). The compare harness after each look-up is
  `FADD_IMM.v2f16` / `FADD.v2f16` of the texel and the constant, `FMAX.v2f16 .abs`, a `FMAX.v2f16`
  tree over all look-ups, one `FCMP_OR.v2f16.lt`, then `IADD.v2u16` and `MKVEC.v2i16` for the
  colour. In `uapi_struct_cube_both`, the same look-ups in the vertex shader leave the fragment
  harness reading `v_vtxOut` as a flat load (`LD_VAR_BUF_IMM.src_flat32`, `MUX.v2i16`).
- **F3**: each lane is converted to fp32 on its own: `S32_TO_F32` of an int uniform, `FROUND.f32.rtz`
  for `int(b)` of a varying, raw fp32 varying lanes (`rnd_int_lane`). Sums are fp16 pairs widened per
  lane (fmaloc 3.1 mechanism 3). The cube rows are as in F1.
- **F4**: as F3, then `TEX_SINGLE .cube.computed_bias` with the third staging word.
  `rnd_bias_const` takes the constant bias from FAU (`MOV.i32 r4, u2.w1`). `rnd_bias_ivec`'s
  computed bias is `build_lod_bias_clamp`'s chain (`FMA.f32.clamp_m1_1 .., u4.w0`, `FMA.f32 ..,
  u4.w1`, `F32_TO_S32.rtz`, `U16_TO_U32`). `rnd_bias_mul`'s bias `b - b` folds to 0, so the
  look-up is `.computed` (no bias word).
- **F5**: the converted pair in one fp16 op, then `F16_TO_F32` per lane and the two staging moves.
  `rnd_2d_const_sub` is `FADD.v2f16 r0, 0x3C000000, r0^.neg` (the constant pair `(0, 1.0)` as
  one word). `rnd_2d_splat_mul` is `FADD.v2f16 r0, r0^, r0^`. `rnd_2d_vec_sub` is `FADD.f32 r0.h0, r0^,
  0x0.neg` (narrow a scalar varying), `MKVEC.v2i16 r0, r0^.h0, u2.w0.h0` with a pushed fp16
  constant, then `FADD_IMM.v2f16 #0xBC008000`. In `disc_texture`, Mesa's linker merges `v_color`
  (`vec4(a_coords.xyz, 1.0)`) into `v_coords`'s load: one `LD_VAR_BUF_IMM .v4` gives the colour
  (`FADD.f32 r1.h0, r2^` for z, the pair `r0` for x and y, `MKVEC .. u3.w0.h0` for the constant w)
  and the coordinate (`FMA.v2f16 r2, r0, 0x34000000.h11, 0x38000000.h11`). Then
  `DISCARD.f32.lt r2^.h0, u2.w1` follows the look-up.

## 4. Numbered lowering steps

Ranked by first-refusal cases, HOST-ONLY at `56b5878a`. "Moves" is the number of cases that leave
this text; "compiles" is the bypass probe's count, an upper bound for a pass. Every step runs
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table
--test mali_deqp_texcoord --test mali_deqp_fmaloc` with its references un-ignored, and moves each
of them in `REFUSED` to `LOWERED` or `DIFFERS` in the same commit.

| Step | What | Site | Moves | Compiles (bypass) | References | dEQP groups |
|---|---|---|--:|--:|---|---|
| **TC1** | F1, the cube face: fmaloc step 1 (`tex-cube-face`, fmaloc 3.1 mechanism 1) as designed | 8529 | 86 | 0 until the select builds the pair (then all 86 are a lone look-up of the texel) | `fmaloc_tc_cube_face`; `fstex_cubeface0..3` in `src/shaderc/online/mali/tests.rs` flip with it | `texture.specification.*_cube` |
| **TC2** | F2: accept a direction whose three lanes are constants or uniform words in `cube_dir_measured` (the implicit arm). No fold of the cube rows, and one sequence for equal directions. | 8529 | 69 | 9 (`fmaloc_tc_cube_const`, `rnd_swizzle_const` byte-equal) | `fmaloc_tc_cube_const`, `texcoord_uapi_*`, `texcoord_rnd_add_const`, `texcoord_rnd_swizzle_const` | `uniform_api.{value,info_query,random}.*samplerCube*` |
| **TC2b** | TC2's follow-ups, other texts: the harness over two or more look-ups (`gl_FragColor` lane 40: the fclane family) and a cube array element other than the first (`TexArrayElement` 20: an FAU descriptor word, section 3) | (other texts) | 0 of this text | the 60 uapi cases TC2 moves on | `texcoord_uapi_struct_cube` (lane), `_uapi_unused_cube_elem`, `_uapi_array_in_struct`, `_uapi_struct_in_array` (array element) | as TC2 |
| **TC3** | F5: widen `select`'s lone computed 2D pair (`frag_coord_tree` / `varying_coord_tree`) to a constant pair minus a converted pair (byte-equal with the check bypassed). Next, `x * 2.0` as `x + x` and a narrowed scalar varying `MKVEC`ed with a pushed half. | 12960 | 6 | 2 (`rnd_2d_const_sub` byte-equal, `rnd_2d_splat_mul` differs) | `texcoord_rnd_2d_*`, `fmaloc_tc_2d_scalar`, `texcoord_disc_texture` | `shaders.random.texture.fragment.{11,12,43,85}`, `shaders.discard.{basic,function}_texture` |
| **TC4** | F3: mixed lanes in the implicit arm: per-lane conversions (`S32_TO_F32`, `FROUND.rtz`, raw fp32 lanes) and sums in all three lanes (fmaloc 3.1 mechanism 3, step 4) | 8529 | 7 | 0 (4 meet the `gl_FragColor` lane, 1 operand form, 1 `TexKind`, 1 this text again) | `fmaloc_tc_cube_rnd_sub`, `texcoord_rnd_int_lane` | `shaders.random.texture.fragment.*` |
| **TC5** | F4: the same direction shapes in the bias arm | 8492 | 16 | 0: all 16 meet `TexKind` (a bias that is not a uniform lane) | `texcoord_rnd_bias_*` | `shaders.random.texture.fragment.*` |

If TC1-TC5 all land, the text goes from 184 cases to at most 1 (random 111, which meets it again
at a later site: measure it after TC4). After TC1 alone 98 remain (under the 100-case line); after
TC1 and TC2, 29.

Notes:

- **TC2 is the cheapest step.** It changes one predicate and adds no new Mesa shape, and the bypass
  probe is byte-equal on its lone forms. It should go before TC1, or with it, in the same commit
  series (both edit `cube_dir_measured`, the fmaloc note's "steps 1 and 4 share one `CubeDir`
  rewrite"). Keep the constant pass from folding `FRCP`/`FMA` of constants: Mesa does not fold
  them (`opt.rs`, `fau.rs`).
- **TC3 edits the predicate that `lower-train-2`'s TEX_COORD2 added** (`varying_coord_tree`,
  landed as `a27396fa`). At `56b5878a` all 15 `deqp_texcoord_*` and the four `deqp_fmaloc_tc_*`
  are still refused with this text (`refusals_pinned` of both files). The discard
  shape is a multiply-add of a vec4 varying's first two lanes by splats, which is TEX_COORD2's own
  form. It is still refused because the colour reads the same merged load (section 3), and
  TEX_COORD2 requires that nothing else reads the load. TC3 relaxes that rule for a load whose other
  lanes go to the colour. It can start now that `lower-train-2` has landed.
- **TC4 and TC5 move cases but pass none.** Do them after the fclane and `TexKind` work their next
  texts need (texmisc's bias step, `mali-texmisc-lowering.md`), or with that work.

## 5. Order relative to the schedule (sections 5.4 and 5.5a of `mali-lowering-schedule.md`)

- **Train 3 (5.4; 3b ready, 3c todo): none of these steps.** 3c's `varyload-lower-1` edits
  `walk_nodes`, the same function as the cube arms. 5.5a puts T1 after it for that reason.
- **Train 4 (5.5a), commit 12 "T1"** becomes, in this order: **TC2, TC1, TC4** (one `CubeDir`
  rewrite, cube arm and `cube_dir_measured`), then **TC3** as commit 12b (`lower-train-2`, its
  prerequisite, merged as `a27396fa`). All are byte-compared by `mali_deqp_fmaloc` and `mali_deqp_texcoord`. The 5.5a row's
  forecast (T1 104) becomes 86 + 69 + 7 = 162 cases moved off the text, plus TC3's 6.
- **TC2b** goes to the owners of the next texts: the `gl_FragColor`-lane harness to the fclane
  family, `TexArrayElement` to the texmisc design (it has no item yet). Neither belongs in train 4's
  T1 commit.
- **TC5** follows texmisc's `TexKind` bias step (no item yet), after TC4.

## 6. How the references were drawn

`tools/mali/deqp_ref.sh <pair> tools/mali/testdata/deqp_texcoord_<shape> --instrument shim`, with
the pairs taken verbatim from the host measure's `programs/` (the `.license` files name each case).
Ten pairs drew with the in-tree tools. `gl_ref.py` cannot draw the five `uapi_*` pairs: it does not
bind a sampler that is a struct member (`--uniform-values` stops, and `--zero-uniforms` puts a
`sampler2D` and a `samplerCube` on unit 0, which GL rejects at the draw). They were drawn with a
scratch copy of `tools/mali` whose `gl_ref.py` expands struct uniforms with `uniform_leaves()` when
it lists samplers (each leaf gets the next unit), and skips sampler leaves in
`uniform_settings()`. Drawn again with the unit order reversed, every `uapi_*` `expect.json` is
byte-identical, so the words do not depend on the units. The `.nir.sha` files come from
`tools/mali/nir_refs.py` with the same scratch copy (15 written, 0 errors). Making `gl_ref.py`
bind struct-member samplers in the tree is a small tool item. It is not this item's, and without
it `tools/mali/nir_refs.sh` and `g310_check.sh` cannot redraw the five `uapi_*` pairs.

## 7. Not in this note

- The `TexKind`, `TexArrayElement` and `gl_FragColor`-lane texts that TC2b, TC4 and TC5 lead
  to. Their designs are `mali-texmisc-lowering.md` and `mali-fclane-lowering.md`.
