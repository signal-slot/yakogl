<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the corpus refusal families after plan #336's cases

Plan #336 item `ana-corpus`, preparing the roadmap stage after it (the
dEQP-GLES2 and glmark2 causes on the NanoPC-T6 Mali). State: tip `d512696b`,
`tools/mali/corpus-verdicts.txt` as pinned. Sites are `file:line` at that tip,
in `src/shaderc/online/mali/`; Mesa lines are 26.2.1 source
(`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1/src`).

**Evidence.** The counts are measured: the file has 328 lines, 4 header
lines, 240 `ok` and 84 others (66 Valhall refusals, 18 front-end rejections);
the groups below add to 84 and each non-ok line is in exactly one. No built
Mesa 26.2.1 drm-shim is under the deps directory (only the source tree), so no
words were captured in this item: every Mesa reference below is read from
source, and every claim about which refusal a source reaches, or about what
Mesa does to it, is **inferred** unless it says *measured*. A refusal site
named here is the one the message's `MaliError` variant is raised at; a
variant with several sites lists them, the one a given source reaches is
not run.

## Out of scope: the desktop-GLSL front-end rejections (18)

`front end: Rejected { line: 1, tok: "#version", msg: "desktop GLSL source;
this is an OpenGL ES compiler" }`. One group, no Mali work (the files are
desktop `#version 1xx` sources the ES front end refuses on every back end):
`armselect.frag`, `color_uv.{frag,vert}`, `compact.frag`,
`internal/{clear,fbload}.{frag,vert}`, `interp_color.frag`, `mvp.vert`,
`mvp_color.vert`, `passthrough.vert`, `points.vert`, `texture.frag`,
`texture2.frag`, `uniform_color.frag`, `uv.vert`, `zorder.vert`
(under `tools/shaders/`).

## The 66 Valhall refusals, by text

Counts are sources (paths under `tools/shaders/`), largest first. "Note" is
the existing note that carries the family's design; "—" is none.

| # | family (variant) | n | sources | note |
|---|---|---|---|---|
| 1 | arithmetic other than `+ - *` (`Arithmetic`) | 16 | see A below | `mali-alu-ops`, `mali-transcendentals`, `mali-derivatives`, `mali-trunc`, `mali-vs-alu-ops`, `mali-vs-fma`, `mali-int-and-size` |
| 2 | integer or boolean value (`IntValue`) | 6 | `es100/cond_complex.frag`, `loop_full.frag`, `ret_nested.vert`, `tri_discard.frag`, `tri_int.frag`, `varyings15.frag` | `mali-int-and-size`, `mali-int-loops` |
| 3 | kept run-time loop, mediump fragment (`LoopMediump`) | 6 | `es100/discard_dynloop.frag`, `loop_nested.dowhile.frag`, `loop_nested.frag`, `loop_nested.while.frag`, `loop_uniform.frag`, `ret_dynloop_never.frag` | `mali-int-loops`, `mali-loop-break`, `mali-loop-board-failures`, `mali-remaining-families` F7 |
| 4 | control flow (`ControlFlow`) | 4 | `es100/frontfacing.frag`, `partial_varying.vert`, `psiz_if.vert`, `gltest/predif_vs_uni.vert` | `mali-control-flow`, `mali-vs-abs-if` |
| 5 | look-up other than `texture2D(sampler2D, c)` (`TexKind`) | 4 | `es300/layers2.frag`, `texfetch.frag`, `gltest/texcube_fs.frag`, `usamp.frag` | `mali-texture-lookups`, `mali-texel-fetch`, `mali-cube-textures`, `mali-texture-refusals` |
| 6 | indexed local variable (`IndexedLocal`) | 3 | `es100/dynidx.frag`, `dynidx_loop.frag`, `matdyn.frag` | `mali-indexed-locals` |
| 7 | uniform at a run-time index (`UniformIndexed`) | 3 | `es100/dynvec4.frag`, `uniform_idx.frag`, `es300/uniidx.frag` | `mali-uniform-index`, `mali-uniform-arrays` |
| 8 | uniform other than float/vec/mat4 (`UniformType`) | 3 | `es100/light_amount_0_16.vert`, `light_amount_2_2.vert`, `sharedint.vert` | `mali-uniform-types`, `mali-int-uniforms` |
| 9 | `gl_FragColor` lane (`ColourForm`) | 3 | `es100/ifloop.frag`, `tri_if.frag`, `gltest/texcubenorm_fs.frag` | `mali-varying-constants` (shapes), `mali-remaining-families` F8 |
| 10 | constant product (`ArithConstant`) | 3 | `es100/loopcarry.frag`, `mvstruct.vert`, `xlink.vert` | `mali-constant-products`, `mali-vs-constant-products`, `mali-algebraic-folds` |
| 11 | two varying lanes of equal value (`VaryingDuplicate`) | 2 | `es100/discard_dynloop.vert`, `va_vec2.vert` | `mali-varying-constants` |
| 12 | varying lane from no attribute (`VaryingNotFromAttribute`) | 2 | `es100/vtex_struct_sampler.vert`, `vtex_struct_sampler_array.vert` | `mali-varying-constants`, `mali-vs-lookups` |
| 13 | more than 32 locals (`TooManyLocals`) | 2 | `es100/tmparr_const.frag`, `tmparr_const.vert` | — (only the `mali-remaining-families` F4 row) |
| 14 | loop in a vertex shader (`LoopVertex`) | 2 | `es100/loop_nested.vert`, `loop_uniform.dowhile.vert` | — (F6 row of `mali-remaining-families`; `mali-loop-board-failures` names it) |
| 15 | chain of 3+ adds/muls (`Rebalance`) | 1 | `es300/texoffset.frag` | `mali-rebalance`, `mali-remaining-families` F1 |
| 16 | vertex look-up with a constant lane (`TexVertexConstant`) | 1 | `es100/cubelod.vert` | `mali-vs-lookups` |
| 17 | texture coordinate not a whole vec2 varying (`TexCoord`) | 1 | `es100/xlink.frag` | — (F9 row only; `mali-texture-refusals` names the variant) |
| 18 | mixed highp and mediump (`MixedPrecision`) | 1 | `es100/ifelse_loop.frag` | `mali-mixed-precision` |
| 19 | operand form no reference draws (`FsOperandForm`) | 1 | `es100/cond_sequence.frag` | `mali-alu-ops`, `mali-remaining-families` |
| 20 | comparison other than `< <= > >= ==` (`CompareForm`) | 1 | `es100/sharedint.frag` | — (F11 row only) |
| 21 | `break`/`continue` behind a run-time condition in an unrolled mediump loop (`GuardMediump`) | 1 | `es100/contloops.frag` | `mali-loop-break` |

16+6+6+4+4+3+3+3+3+3+2+2+2+2+1+1+1+1+1+1+1 = 66, and 66 + 18 = 84.

The user-text counts the plan expected (about 20 arithmetic) are 16; the
"5 each" for control flow and look-ups are 4 each; the kept fp16 loops and the
integer/boolean values are 6 each, not 7. The rows 19 to 21 are single
sources the plan's text lumped into "others".

### A. The 16 arithmetic sources, by the operation (inferred from the source text)

| operation | n | sources | note |
|---|---|---|---|
| `sin cos exp2 log2 inversesqrt` in a fragment shader | 3 | `es100/transc.frag`, `vtransc.frag`, `fmods.frag` | `mali-transcendentals` (case 77 is `transc`) |
| `rcp`, `inversesqrt` in a vertex shader | 1 | `es100/vrcp.vert` | `mali-vs-alu-ops`, `mali-transcendentals` |
| `dFdx dFdy` | 2 | `es100/deriv.frag`, `es300/derivs.frag` | `mali-derivatives` (case 138) |
| `trunc` family in a fragment shader | 1 | `gltest/es3trunc_fs.frag` | `mali-trunc` (case 199) |
| integer ALU: `/ % >> ^ ~ min max`, `int()` and `uint()` conversions, `bool()` of a float | 7 | `es100/int_div.frag`, `es300/int_div.frag`, `es300/uops.frag`, `es300/gcops.frag`, `es300/ifelse_uint.frag`, `es100/f2b.frag`, `gltest/f1_fs.frag` | `mali-int-and-size`, `mali-alu-ops` |
| `step` | 1 | `es300/layersunused.frag` | `mali-alu-ops` |
| vertex `ffma` shape (`floor(a*b*4.0)`, `u - abs(a*b)`) | 1 | `gltest/fma_vs_ctrl.vert` | `mali-vs-fma` |

3+1+2+1+7+1+1 = 16. Which `Arithmetic` site each reaches is not run. The
variant has the catch-all arms `nir_fs.rs:4216` (every fragment ALU op the
walk does not name), `nir_fs.rs:2142` (`Alu` arm of the vector-builtin
lowering), `nir_fs.rs:1032`, `nir.rs:2613` (vertex ALU not named) and
`nir.rs:2304` (any/all compare not named), plus the specific `nir.rs:2213`,
`2260`, `2288`, `2299`, `nir_fs.rs:2373`, `2377`, `4125`, `7116` and
`vec16.rs:630`.

## Covered families: one line each

Each has a note with the design; this note adds only the count.

- 1 Arithmetic (16): `mali-alu-ops` is "the ops behind the 16 arithmetic
  refusals"; the splits above are in the notes named. Transcendentals,
  derivatives and trunc have implementation items in plan #336
  (`fs-transc`, `fs-deriv`, `fs-trunc`, `isa-transc`, `isa-deriv`); the
  integer group has no item.
- 2 IntValue (6): sites `nir.rs:2351`, `2491`, `2587`, `2612`,
  `nir_fs.rs:1413`, `1504` (**inferred** which). `mali-int-and-size`.
- 3 LoopMediump (6): `nir_fs.rs:3526`. `mali-int-loops`, `mali-loop-break`.
- 4 ControlFlow (4): `nir_fs.rs:2720`, `3032`, `3044`, `3060`, `7013`,
  `nir.rs:2341`, `vec16.rs:451`. `mali-control-flow`; the vertex `if`
  cases (`psiz_if`, `predif_vs_uni`, `partial_varying`) are plan item
  `vs-abs-if`'s family.
- 5 TexKind (4): `nir_fs.rs:2610`, `4343`, `4383`, `4412`, `4519`, `4525`,
  `7517`. The four sources are a 2D array (`layers2`), `texelFetch`
  (`texfetch`), a cube map (`texcube_fs`) and an integer sampler (`usamp`).
- 6 IndexedLocal (3): `glsl.rs:129`; `mali-indexed-locals`, item
  `fs-indexlocal`.
- 7 UniformIndexed (3): `nir.rs:1054`, `nir_fs.rs:4273`, `4294`, `7759`,
  `7761`, `8515`, `8532`; `mali-uniform-index`.
- 8 UniformType (3): `nir.rs:2369`; `mali-uniform-types`.
- 9 ColourForm (3): `nir_fs.rs:984`, `1319`, `4094`, `4139`, `4160`,
  `4179`, `5212`, `5743`, `6282`, `6789`, `8053`. `ifloop` and `tri_if` are
  an integer loop or `if` in front of the colour, `texcubenorm_fs` a cube
  look-up whose coordinate is a quotient: they follow rows 2, 4 and 5, not a
  colour rule of their own (**inferred**).
- 10 ArithConstant (3): `nir.rs:2018`, `2054`, `2461`, `nir_fs.rs:2321`,
  `2339`, `5052`, `5064`, `7111`, `glsl.rs:1463`; `mali-constant-products`
  (fragment, case 70 `loopcarry`), `mali-vs-constant-products` (vertex:
  `mvstruct`, `xlink`).
- 11, 12 varying lanes (4): `nir.rs:2786` (equal), `mod.rs:1593`,
  `nir.rs:2780`, `2800`, `nir_fs.rs:5428` (no attribute); `mali-varying-constants`.
- 15 Rebalance (1): `glsl.rs:1226` to `1354`; `mali-rebalance`. `texoffset`
  also needs a `TexKind` look-up (`textureOffset`, `mali-texel-fetch`)
  behind it.
- 16 TexVertexConstant (1): `nir.rs:2643`; `mali-vs-lookups` (case 101).
- 18 MixedPrecision (1): `mod.rs:1295`, `1386`, `nir_fs.rs:2460`, `4032`;
  `mali-mixed-precision`. 19 FsOperandForm (1): `nir_fs.rs:644`'s
  comment, raised at `5667`, `5857`, `5890`, `5895`, `6188`.
- 21 GuardMediump (1): `nir_fs.rs:3023`; `mali-loop-break`.

## Uncovered families (rows 13, 14, 17, 20): site, example, Mesa

All four are **inferred**: no drm-shim run, and the sources have not been
compiled for this note beyond what the pinned verdict says.

### 13. More than 32 locals (2 sources)

- **Site.** `nir.rs:2325` raises `TooManyLocals` in the vertex walk and
  `nir_fs.rs:2661` in the fragment walk, against `MAX_LOCALS = 32`
  (`nir.rs:65`). The check counts front-end locals before any array is
  split, so it does not look at what Mesa would keep.
- **Smallest example.** `es100/tmparr_const.vert`: a `vec4 arr[40]` written
  with 40 constants, read at four int-uniform indices and summed (62 lines);
  `tmparr_const.frag` is the same on the fragment side (dEQP
  `indexing.tmp_array.vec4_const_write_dynamic_read`).
- **Mesa.** Not a limit of Mesa's: the array is a `function_temp` variable.
  `bifrost_nir.c:209` splits array variables (`nir_split_array_vars`) and
  `:210` shrinks them; `:981-982` lowers an indirect access on an array of
  at most 8 elements to an if/else tree (`nir_lower_indirect_derefs_to_if_else_trees`,
  threshold 8) and the larger array (40 elements here) stays a variable that
  `:998` (`nir_lower_vars_to_explicit_types`) and `:1005`
  (`nir_lower_explicit_io`) turn into scratch (TLS) loads and stores, the
  `load_scratch`/`store_scratch` that `bifrost_compile.c:1030` handles.
  So the shader needs **scratch**, the same dependency that
  `mali-indexed-locals` names for the run-time-indexed local (that note's
  binary-search ladder is for arrays within the 8-element threshold). **The
  shape needs a decision, not a rule**: a 40-element array is either
  lowered to 160 scalar slots (an RA problem past the 32 work registers,
  `MaliError::Spill`) or to scratch, which this back end has no stage for
  (`ra.rs:21`, "the full-file retry and spilling are not" written).
- **Item.** Raise the 32 cap only together with the scratch stage; as a
  stand-alone change the two sources would meet `Spill` or `IndexedLocal`
  next. Corpus-only (neither source is a sweep case).

### 14. A loop in a vertex shader (2 sources)

- **Site.** `nir.rs:2340`, the `IrNode::Loop` arm of the vertex walk, comment
  "intloop_loop_nested_vs: Mesa keeps the loops as blocks".
- **Smallest example.** `es100/loop_uniform.dowhile.vert` (17 lines):
  `do { res = res.yzwx; } while (++i < ui_three);` with a mediump int
  uniform bound. `loop_nested.vert` is two nested loops.
- **Mesa.** `bifrost_nir.c:268` runs `nir_opt_loop_unroll` in the
  optimisation loop; the options set `max_unroll_iterations = 32`
  (`bifrost_compile.h:140`). A run-time (uniform) bound cannot be unrolled,
  so Mesa keeps the loop as NIR blocks and the backend emits branches
  (`BRANCHZ`/`BRANCH` flow, the same machinery as the kept fragment
  loops of row 3). `force_indirect_unrolling` (`bifrost_compile.h:141`) is for
  indirect array access, not this. In the vertex stage there is no fp16
  carried value (Mesa's vertex shader runs fp32), so the kept-loop work
  of row 3 minus its fp16 vector handling is what applies.
- **Item.** Reuse the fragment kept-loop lowering (`nir_fs.rs:3526`'s
  neighbours) for the vertex walk. Corpus-only; glmark2 has no vertex loop
  in its 33 scenes' refusals according to `mali-glmark2-frametime.md`
  (**inferred**: not re-read here).

### 17. A texture coordinate that is not a whole vec2 varying (1 source)

- **Site.** `TexCoord` is raised at `nir_fs.rs:4325`, `4380`, `4416`,
  `4453`, `4497`, `4532`, `5456`, `6213`; the 2D look-up arm is the one at
  `4453` (`let Val::Lane(n, _) = v else { return Err(MaliError::TexCoord) }`)
  and its neighbours, which accept a vec2 varying read whole or two
  constants.
- **Smallest example.** `es100/xlink.frag`: two look-ups at
  `v_uv2.xy` and `v_uv2.zw` of one vec4 varying (second look-up on the
  upper half), result `mix(a, b, v_mix) * u_fade.opacity` with a struct
  uniform.
- **Mesa.** The coordinate is the two interpolated varyings
  `LD_VAR_IMM` results; Mesa loads a vec4 varying as vec4 or splits it per
  the link (`pan_nir_resize_varying_io.c`, `pan_nir_lower_fs_inputs.c`)
  and the `TEXS`/`TEX` coordinate pair comes from the registers those load
  into, so a `.zw` pair is a lane pair 2,3 of one load. Our walk takes a
  coordinate only as the whole varying's own pair. The note
  `mali-texture-refusals` lists the variant; `mali-texture-lookups`
  covers the second look-up (an earlier row), not the lane choice.
- **Item.** Accept a coordinate pair at lanes 2,3 of a vec4 varying (and
  the case of the second look-up in the same shader, behind `TexCount`).
  The source also meets `ConstantFold` (`mali-constant-products`,
  `xlink.vert` is the vertex half) and the struct uniform; so the three
  `xlink` refusals come in one item. Corpus-only.

### 20. A `!=` comparison (1 source)

- **Site.** `nir_fs.rs:2141`: `AluOp::Fne | AluOp::Ine => Err(CompareForm)`,
  with the comment that no reference that compiles draws it.
- **Smallest example.** `es100/sharedint.frag`: `if (u_p.mode != 0)
  c.rgb *= 0.5 + 0.5 * sin(v_xy.x + v_xy.y);` (an int struct member
  uniform, so `Ine`). The shader also needs `sin` (row 1) and an `if`
  around it, and its vertex half `sharedint.vert` stops at `UniformType`
  (row 8): the comment names exactly that wait.
- **Mesa.** `nir_op_ine` is lowered to `ine_pan` by the Bifrost
  compile (`bifrost_compile.c:2590` and `:3254`: `bi_icmp_to(... BI_CMPF_NE)`,
  the same `ICMP.s32` the `==` and `<` forms use with the compare changed).
  The float form is `fneu_pan` (`:2591`), `FCMP.f32.ne`-class. So the
  rule is a compare-function change in two already-lowered ops, not a
  new instruction.
- **Item.** Add `Ine`/`Fne` to the compare arm once a measured word exists
  (the shim run is the item's step 0); it moves no sweep case by itself.

## Ranked implementation items (largest family first)

Count is sources in `corpus-verdicts.txt`; "plan" is the #336 item that
already covers part of it.

1. **Integer ALU and conversions** (7 of row 1, plus rows 2 and 3's integer
   values, ~19 sources together): `idiv/imod/ishr/ixor/inot/imin/imax`,
   `f2i/f2u/i2f/u2f`, `f2b`, bool values. The largest uncovered group; plan
   #336 has no item for it. Needs the Mesa reference first
   (`mali-int-and-size`, `mali-alu-ops` have the inferred op list).
2. **Kept mediump fragment loops** (row 3, 6; `GuardMediump` +1, vertex
   loops row 14, +2): fp16 vectors carried across the back edge, and the
   same walk in the vertex stage. `mali-int-loops`, `mali-loop-break`.
3. **Transcendentals and reciprocals** (4 of row 1: `transc`, `vtransc`,
   `fmods`, `vrcp`): plan items `fs-transc` and `isa-transc`; `vrcp.vert`
   needs the vertex side (`vs-lookups`-like, a separate walk arm at
   `nir.rs:2613`).
4. **Control flow** (row 4, 4; and the `if` in rows 9 and 21): the vertex
   `if`s are plan item `vs-abs-if`; `frontfacing.frag` is a built-in input
   (`mali-builtin-inputs`).
5. **Texture look-ups other than `texture2D`** (row 5, 4; `texoffset` in
   row 15, `xlink` in row 17, `cubelod` in row 16): 2D array, `texelFetch`,
   cube map, integer sampler: four separate descriptors, each with a
   note (`mali-texel-fetch`, `mali-cube-textures`).
6. **Indexed locals** (row 6, 3): plan item `fs-indexlocal`;
   `mali-indexed-locals`.
7. **Uniform at a run-time index** (row 7, 3) and **uniform types** (row 8,
   3): `mali-uniform-index`, `mali-uniform-types`; independent of each other.
8. **Constant products** (row 10, 3): plan item `fs-fold` and the vertex
   half `mali-vs-constant-products`.
9. **Derivatives** (row 1, 2) and **trunc** (row 1, 1): plan items
   `fs-deriv`, `fs-trunc`.
10. **Varying lanes** (rows 11, 12, 4 sources) and **colour lanes** (row 9,
    3): `mali-varying-constants`; the colour lanes mostly follow items 1, 2
    and 4.
11. **Scratch and the 32-local cap** (row 13, 2): the only item with no
    note of its own; blocked on a scratch (TLS) stage. Corpus-only.
12. **One-source rows**: `!=` (row 20, after `sin`), a `.zw` coordinate pair
    (row 17), `MixedPrecision` (row 18), `FsOperandForm` (row 19),
    `Rebalance` (row 15), `TexVertexConstant` (row 16, plan `vs-lookups`).

Nothing in this note changes code or `corpus-verdicts.txt`.
