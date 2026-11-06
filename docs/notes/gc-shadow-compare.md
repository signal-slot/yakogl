<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: shadow (depth-compare) look-ups as Mesa's etnaviv does them

Design note of plan #310, item gc-sampler-note (part 2; part 1 is
`gc-texture-swizzle-wrap.md`). `GlesTextureBind::compare_op`
(`src/backend.rs:895`) carries a shadow look-up's compare function from the GL
core, and nothing in `src/vivante` reads it; the GC7000 compiler refuses every
`sampler*Shadow` look-up by name. This note answers where Mesa compares
(texture engine or shader), what `src/vivante` does today (with a local
experiment), the change that would make a shadow look-up work, the host tests
of that change, and a HOST-ONLY dEQP baseline. **No code changes with it, and
nothing here was run on a board.**

## 1. Sources

Mesa 26.2.1, tag `mesa-26.2.1`; the etnaviv driver is not in `third_party/`
(see `gc-texture-swizzle-wrap.md` section 1: it was read from the same
release's tree in the deps cache). Paths are relative to Mesa's `src/`.
Letters as in that note (`D` `etnaviv_texture_desc.c`, `F` `etnaviv_format.c`,
`X` `etnaviv/hw/common_3d.xml.h`, `N` `etnaviv/hw/state_3d.xml.h`, `T`
`etnaviv_translate.h`, all under `gallium/drivers/etnaviv/` except `X` and `N`)
and these:

* `C` `etnaviv_context.c`, `L` `etnaviv_nir_lower_texture.c`, `H`
  `etnaviv_shader.h`, `U` `etnaviv_texture_state.c`, `K`
  `etnaviv_compiler_nir.c`, `M` `etnaviv_compiler_nir_emit.c`, `S`
  `etnaviv_screen.c` (all `gallium/drivers/etnaviv/`).

Own code is cited at the baseline commit `1949cd0c`.

## 2. (a) Where Mesa compares

**In the texture engine on a HALTI2+ core such as the GC7000UL (HALTI5), and in
the shader only for the depth32f format and on older cores.** Both routes
exist, chosen per draw:

The TE route (the one that applies to `D16`, `D24X8` and `S8_Z24` depth):

* `D:106-113` `etna_create_sampler_state_desc` builds the sampler's
  `SAMP_CTRL0`: `COND(ss->compare_mode, ...SAMP_CTRL0_COMPARE_ENABLE) |
  ...SAMP_CTRL0_COMPARE_FUNC(translate_texture_compare(ss->compare_func))`
  (`D:112-113`), from the gallium sampler state's `compare_mode` and
  `compare_func`. It is written with the other sampler words per slot
  (`D:308`) and the companion slot of a 128-bit view (`D:506`).
* Bits: `N:1974` `COMPARE_ENABLE 0x00020000` (bit 17); `N:1975-1977`
  `COMPARE_FUNC` mask `0x001c0000`, shift 18 (bits 18-20). Bit 16 beside it is
  `DEPTH_STENCIL_MODE` (`N:1970-1973`, set to STENCIL for a stencil view,
  `D:254-257`), and bit 21 is `UNK21`, which `src/vivante` already sets.
* Values: `T:549-571` `translate_texture_compare` maps gallium's function to
  `X:132-139`: `LEQUAL 0, GEQUAL 1, LESS 2, GREATER 3, EQUAL 4, NOTEQUAL 5,
  ALWAYS 6, NEVER 7`. It maps by **name**, so gallium's `ref FUNC texel` reading
  (GL's) is the hardware's.
* Which opcode: `M:230` a shadow `texture()` (`nir_texop_tex`) is a plain
  `TEXLD`; `M:231` `TEXLDB` for a bias; `M:232` shadow gradients are
  `TEXLDGPCF` and `M:233` shadow explicit levels are `TEXLDLPCF`
  (`is_shadow ? ... PCF : ...`). The reference value is the NIR `comparator`
  source: `K:555-558` puts `nir_tex_src_comparator` (and `ddy`) in
  **src2** of the instruction (`K:538-565`, `emit_tex`), `M:206-225`
  `etna_emit_tex` copies it to `inst.src[2]`. For shadow gradients, `L:351-390`
  `legalize_txd_comparator` instead folds the comparator into the coordinate's
  `w`.
* The sampler is told nothing more: `D:324-328` (`emit_desc_sampler_ctrl0`)
  forces the MIP filter to NEAREST for any Z/S format with a linear mip filter
  (`ss->base.min_mip_filter == PIPE_TEX_MIPFILTER_LINEAR`), compare or not.
* The legacy (non-descriptor) sampler has the same bits in
  `NTE_SAMPLER_BASELOD` (`U:156-157`, `VIVS_NTE_SAMPLER_BASELOD_COMPARE_*`);
  the repo's `driver/src/vivante/hw/state_3d.xml.h:1865-1866` lists them.
* The core claims it: `S:281` `caps->texture_shadow_map = true`. The GC7000UL's
  row is HALTI0..HALTI5 (`docs/es3/gc-formats.md` section 1).

The shader route (compiler lowering), chosen per draw in `C:168-196`
(`etna_get_fs`):

```
for each fragment sampler with a view and compare_mode != NONE:
    emulated_z32f = format_is_emulated_z32f(view format)       // F/H: etnaviv_format.h:99-102
    if halti >= 2 and not emulated_z32f:  continue              // the TE compares
    if emulated_z32f: key->shadow_compare_no_clamp = 1
    key->has_sample_tex_compare = 1; key->tex_swizzle[i] = view swizzle; key->tex_compare_func[i] = func
```

with the comment `C:168-171`: "halti < 2 has no HW shadow compare. halti >= 2
has it, but depth32f is emulated as D24S8 and the float compare ref must not
be clamped to the D24 range, so it must compare in the shader (and not clamp
the ref)". The key fields are `H:59-62`, `H:70-72`; the pass is `L:463-467`
`nir_lower_tex_shadow(num_texture_states, tex_compare_func, tex_swizzle,
!shadow_compare_no_clamp)` (Mesa core's `nir_lower_tex_shadow`; it compares a
plain look-up's depth with the reference and applies the view swizzle); on
halti < 2 the legacy sampler also forces NEAREST for a compare sampler
(`U:159-168`), so those cores have no PCF at all.

So: **the TE compares (`COMPARE_ENABLE` + `COMPARE_FUNC` in `SAMP_CTRL0`, the
reference in `TEXLD` src2) on this core; the depth32f formats compare in the
shader without clamping, because their storage is emulated.** `Z32_FLOAT` and
`Z32_FLOAT_S8X24_UINT` are `EXT_R32F` "emulated format" rows (`F:142-143`);
`Z16_UNORM` is `D16` (`F:113`), `X8Z24`/`S8_Z24` are `D24X8` (`F:179-180`),
`X:59-60`. The TE clamps a compare reference to [0, 1] for fixed-point depth,
which is the thing Mesa avoids for float depth; `compare_op` bit 3 ("fixed-point
depth, reference clamped") is the GL core's name for the same distinction
(`src/gl/draw.rs:668-687`).

What is **not** in Mesa's text and decides a board check: whether the TE's
compare operates on a texel format that is not a depth format, which is how
this back end samples depth (below).

## 3. (b) What yakogl does today

**The GC7000 compiler refuses a shadow look-up by name; nothing is compared,
no reference is dropped.** Evidence, an uncommitted local experiment (a throw-away
`tests/zz_shadow_probe.rs`, since deleted, calling `SHC_ONLINE.compile` as
`tests/es3_wide128_gc.rs:53-62` does, `cargo test --features
host-test,gpu-vivante,shaderc-online --test zz_shadow_probe -- --nocapture`, tree at
`1949cd0c`). Fragment shaders `#version 300 es`, `out highp float o`:

| shader body | result |
|---|---|
| `uniform highp sampler2DShadow s; o = texture(s, vec3(uv, 0.5));` | `rc=-95` |
| `... sampler2DShadow ...; o = textureLod(s, vec3(uv, 0.5), 0.0);` | `rc=-95` |
| `... samplerCubeShadow ...; o = texture(s, vec4(uv, 0.3, 0.5));` | `rc=-95` |
| `... sampler2DArrayShadow ...; o = texture(s, vec4(uv, 0.0, 0.5));` | `rc=-95` |
| `uniform highp sampler2D s; o = texture(s, uv).x;` (control) | `rc=0` |

with the log `online shaderc: the code generator refused this fragment shader:
Select(Unsupported("an ES 3.00 texture look-up (a shadow sampler, an integer
result, a gradient, an offset or a cube fetch) is not selected yet"))` for each
of the four. The program therefore does not link; GL reports the log. The
refusal is `src/shaderc/online/select.rs:~2686-2691` (the `ir::Op::Tex` arm:
`op.es100_tex()` is `None` for a shadow look-up, `three_lane_lookup`
(`:4332`) and `fetch_level` (`:4352`) both require `shadow: false`). Beside it:
`src/shaderc/online/vivante/wide128.rs:159-160` refuses a shadow look-up on a
128-bit sampler by name, and `int_tex.rs:29,36` only rewrites non-shadow integer
look-ups. The machine side has the opcodes: `isa_tables.rs:288,290` carry
`texldgpcf` and `texldlpcf`, and `texld` has an unused SRC2 field
(`src/shaderc/online/vivante/builtins.rs` doc of `texture`: "SRC2_USE is 0 in
all 284 texlds"), but no instruction with the compare reference has ever been
emitted or measured, and `builtins.rs` states the corpus holds **zero**
`texldgpcf`/`texldlpcf`.

The sampler side does nothing either: `grep -rn compare_op src/vivante
src/shaderc/online/vivante` is empty; `emit_slot` writes `samp_ctrl0` from the
wraps, the filters and INT_FILTER only (`src/vivante/texture.rs:568-574`), no
COMPARE bits (`src/vivante/regs.rs` has no name for them; `GC_SAMP_CTRL0_UNK21`
is bit 21 and `INT_FILTER` bit 23). The GL core does its half: `compare_op`
(`src/gl/draw.rs:678`) is computed per draw, the sampler object's mode and
function replacing the texture's (`es3::sampler_compare`, set at `:4137`), and
the PowerVR back end consumes it (`src/pvr/backend.rs:5276`, dword 12).

How a depth texture reaches the TE here: the CPU resolve turns the D24S8
surface into **R32F texels** (`docs/notes/gc-depth-sample.md`,
`ds_cache::resolve_depth`), and a depth texture the application uploads is
R32F too (`GLES_TEXFMT_R32F`, `gc_texfmt` -> `EXT_R32F`). A shadow look-up that
compiled would sample an `EXT_R32F` texture, **not** a D16/D24X8 one as Mesa
does. It is unknown whether the TE's compare unit works on it.

## 4. (c) The change

Two independent halves; neither needs a per-draw shader key in the TE route.

### 4.1 Sampler half: `src/vivante/regs.rs`, `src/vivante/texture.rs`

* `regs.rs`, beside `samp_wwrap` (`:723-741`): `GC_SAMP_COMPARE_ENABLE =
  0x00020000` (`N:1974`) and `samp_compare_func(x) = (x & 7) << 18`
  (`N:1975-1977`), and the table from `compare_op`'s low three bits
  (`func - GL_NEVER`: 0 NEVER, 1 LESS, 2 EQUAL, 3 LEQUAL, 4 GREATER, 5 NOTEQUAL,
  6 GEQUAL, 7 ALWAYS) to the hardware's values (`X:132-139`):

  | `compare_op & 7` | GL | hw (`TEXTURE_COMPARE_FUNC_*`) |
  |---|---|---|
  | 0 | NEVER | 7 |
  | 1 | LESS | 2 |
  | 2 | EQUAL | 4 |
  | 3 | LEQUAL | 0 |
  | 4 | GREATER | 3 |
  | 5 | NOTEQUAL | 5 |
  | 6 | GEQUAL | 1 |
  | 7 | ALWAYS | 6 |

  as `const GC_COMPARE_FUNC: [u32; 8] = [7, 2, 4, 0, 3, 5, 1, 6]`.
* `texture.rs` `emit_slot`, the `samp_ctrl0` expression (`:568-574`): OR in
  `GC_SAMP_COMPARE_ENABLE | samp_compare_func(GC_COMPARE_FUNC[(t.compare_op & 7)])`
  when `(t.compare_op & 16) == 0`, and, as `D:324-328` does for a Z/S view, replace
  a LINEAR mip filter by NEAREST in that case (a compare texture is a depth
  texture, GL's only one that has a compare mode). The MIN/MAG filters stay
  as they are: with HALTI2+ the TE does the PCF (Mesa leaves them linear;
  forcing NEAREST is the pre-HALTI2 route, `U:159-168`). The sampler word is
  rewritten per draw, so a sampler object that changes the function between
  draws needs no extra work. The incomplete-texture stand-in (a 1x1 black
  texel built with `..GlesTextureBind::ZERO`) has `compare_op = 0`, which would
  turn the compare on: build it with `compare_op: 16`.
* Hand-built binds (`bind()` in `src/vivante/backend_tests.rs:1954` and the
  integration tests) leave `compare_op = 0` = "GL_NEVER with compare on"; the
  change enables the compare for every one of them. They compare an RGBA8
  texture the tests never draw, so no word they pin moves except SAMP_CTRL0's:
  every existing test that pins the whole word (`tests/gc_npot_repeat.rs`,
  `CTRL0_ALWAYS`, `:539`) must set `compare_op = 16`, or the GL core path
  (which does) must be what those tests use. List: `grep -rn 'GlesTextureBind {\|GlesTextureBind::ZERO' tests src/vivante`.
* Fixed-point vs float depth (`compare_op & 8`): Mesa's TE route is for the
  fixed formats; its float formats take the shader route without clamp. First
  cut: use the TE for both, accept that a reference outside [0, 1] against a
  `DEPTH_COMPONENT32F` is clamped by the TE (dEQP's 32f cases keep the
  reference in range, `es3fTextureShadowTests.cpp:275-276,563-564,870-871`), and
  record the divergence in `RESULTS.md`/the failures doc. The exact route
  (`has_sample_tex_compare`) needs the per-draw key below and is a second item.
* If the swizzle item has landed: the identity-swizzle clause for a compare-on
  binding (`gc-texture-swizzle-wrap.md` 5.1) is this item's line to add.

### 4.2 Compiler half: `src/shaderc/online/select.rs`, `vivante/builtins.rs`

* `select.rs`: a new arm in front of the "not selected yet" refusal for
  `ir::Op::Tex { shadow: true, proj: false, ret: F32, offset: None,
  lod: Implicit | Lod(_), .. }` on `D2`, `Cube` and `D2Array` (D3 has no
  shadow sampler in ES 3.0, so none is selected). The result is one lane.
* The operands (`ir.rs:414-423`, `eval.rs:699-700`): the coordinate `Src`
  holds the reference in lane 2 for D2 and lane 3 for Cube and D2Array
  (after the cube's three and the array's layer). `builtins::texture` gets a
  `shadow_ref: Option<SelSrc>`: src0 is the coordinate, src2 is
  `lane(coord, ref_lane)` (the helper at `builtins.rs:894`), as `K:555-558` /
  `M:206-225` do. Opcode: `texld` for `Implicit` (`M:230`); `texldlpcf` for
  `Lod` (`M:233`); `texldb` for a bias is out of scope (refuse a
  shadow look-up with a bias by name). `textureProj`,
  `textureOffset`, `textureGrad` and `textureLodOffset` of a shadow sampler stay
  refused by name (their text unchanged).
* What is **not known from Mesa's text** and needs a reference listing before
  the instruction is trusted: the destination write mask and `TEX_SWIZ` of the
  one-lane result (`dst_swiz` in `emit_tex`), and the SRC2 swizzle the
  hardware wants. Get the listing from the reference compiler (the `etna_shc`
  built in the deps cache, `$ETNA_SHC`; not present on every farm node) for the
  four shaders above, add them to the corpus the way the existing `.test`
  corpus is (`tests/es3_shaderc_corpus.rs`), and match them before the
  host-only assertions below are called right. Otherwise the instruction is
  "transcribed, not measured", as `builtins.rs` already says of `texldb`.
* `wide128.rs:159` keeps refusing a shadow look-up on a 128-bit sampler.

### 4.3 Files, generated outputs, and the second route

Edit: `src/vivante/regs.rs`, `src/vivante/texture.rs`,
`src/shaderc/online/select.rs`, `src/shaderc/online/vivante/builtins.rs`
(and `isa_tables.rs` only if `texldlpcf`'s fields are missing, they are not).
Regenerate in the same commit whatever moves: `src/vivante/internal.rs` and the
pinned shader fixtures move only if a fixture shader is added or an existing
listing changes; a shadow look-up compiles from nothing today, so no existing
fixture moves. `GlesTextureBind` gains no field. Docs:
`docs/es3-cts-failures.md` RC5 (`:144-154`) and its GC7000UL rows,
`docs/es3/backend-caps.md` (GC7000UL column of "Shadow samplers"/"Compare
sampler", `:113`, `:136`).

The second route (float depth without clamp, Mesa's `has_sample_tex_compare`)
needs a shader that compares in the compiler: a plain `texld` of the depth,
then `compare(ref, depth)` for the function, and the function in a per-draw key
beside `LodKey` (`src/shaderc/mod.rs:~564`), with the binding's swizzle (the
Mesa pass takes it). It gives non-PCF results (Mesa forces NEAREST for it). Not
needed to start; list it as a follow-up if the 32f cases show clamp errors.

## 5. Host tests for `tests/es3_shadow_gc.rs`

Style of `tests/es3_wide128_gc.rs` (compile and decode with
`decode_instr`) and `tests/es3_formats_gc.rs` / `es3_wide128_planes_gc.rs` (the
sampler words from a recorded stream, field positions pinned by number with the
Mesa line):

1. **The table.** For `compare_op` in `0..=7` (and each `| 8`): the compare
   bit is set and bits 18-20 hold `[7,2,4,0,3,5,1,6][op]`, checked against
   `X:132-139` by name (`rnndb("TEXTURE_COMPARE_FUNC_LESS")`, ...) and against
   GL's meaning with a model: for each function and a few `(ref, depth)` pairs,
   the hardware function applied to `(ref, depth)` equals GL's (`ref FUNC depth`
   under the name mapping).
2. **Compare off.** `compare_op = 16` and `16 | 8`: SAMP_CTRL0 equals the
   word before the change, bit for bit, for 2D, cube and 3D binds and for an
   RGBA8 texture.
3. **Other bits untouched.** With compare on, the wrap, MIN, MAG, UNK21 and
   INT_FILTER fields equal the compare-off word's; MIP is NEAREST when the bind
   asked for LINEAR (`D:324-328`) and unchanged for NEAREST and NONE.
4. **Bit 3 does not change the hardware word** in the first cut (fixed and float
   depth give the same SAMP_CTRL0).
5. **GL end to end.** A `DEPTH_COMPONENT16` 2D texture with `TEXTURE_COMPARE_MODE
   = COMPARE_REF_TO_TEXTURE`, `TEXTURE_COMPARE_FUNC = GL_GREATER`: a draw's
   stream holds enable + 3; after `glTexParameteri(..., GL_NONE)` the compare
   bits are gone; a sampler object bound to the unit with `GL_LESS` overrides
   the texture's (`es3::sampler_compare`).
6. **The stand-in.** An incomplete texture bound to a shadow sampler programs
   no compare (`compare_op: 16` in the stand-in).
7. **Compile, each shape.** `sampler2DShadow`, `samplerCubeShadow`,
   `sampler2DArrayShadow` with `texture()` compile (`rc = 0`); the code holds one
   `texld` (`decode_instr`, `units()` of `es3_wide128_gc.rs:78`) with SRC2 in
   use, SRC2 selecting the coordinate's lane 2 (2D) or 3 (cube, array), and
   `TEX_ID` the sampler's unit; `textureLod` gives `texldlpcf`. The 3D coordinate
   of the cube, and the layer of the array, stay in src0.
8. **Still refused, by name.** `textureProj`, `textureOffset`, `textureGrad` on a
   shadow sampler, a shadow look-up on a 128-bit sampler
   (`wide128_variant`, `wide128.rs:159`): the log text is pinned.
9. **No regression.** Every non-shadow shader of the existing corpus compiles to the same
   bytes (the pinned fixtures do not move); a shader with a shadow and a plain
   sampler binds each to its own unit.
10. **Sensitivity.** A shadow look-up on binding `b` does not make binding `b`
    wide128-sensitive (`wide128_sensitivity`).

Board checks (not host): (a) TE compare on an `EXT_R32F` texel (the open question
in 3); if it fails, the depth texels must be sampled as `D24X8`/`D16` (the road
`gc-depth-sample.md` leaves open) or the second route used; (b) the SRC2 form and
the one-lane write mask; (c) PCF with LINEAR on a 4x4 depth texture against the
dEQP `texture.shadow.2d.linear.*` cases; (d) cube and 2D-array shadow;
(e) the MIP-to-NEAREST rule for the `*_mipmap_linear` cases (dEQP prints a
QualityWarning for them on the PowerVR as well, RC5).

## 6. HOST-ONLY baseline

The host library cannot show a picture (the Linux stand-in OSAL keeps the
command stream; every `glReadPixels` reads black), so **only upload and
compile refusals mean anything here**; every "Fail" with an image message
below is a picture verdict and is *not* a result. Label: HOST-ONLY.

Run: `tools/cts/host-deqp-run.sh --gpu=vivante --module=gles3
--cts="$CONNECTED_DEPS_DIR/VK-GL-CTS" --out=<dir outside the tree> <filter>`
at commit **`1949cd0c`** (`git describe`: the tree was clean when the library
was built; the per-run label later reads `-dirty` because this note was
being written), library built with `YAKOGL_GPU=vivante`, `YAKOGL_SHADERC=online`,
`YAKOGL_REPORT_ES2=OFF`, 256x256 pbuffer; dEQP from VK-GL-CTS `93bca0186` (the
revision `tools/cts/README.md` pins) with the yakogl target applied
(`tools/cts/cts-yakogl-target.patch`, uncommitted in that checkout, which was
fetched into `$CONNECTED_DEPS_DIR/VK-GL-CTS` on this node because it was not
there). Logs were kept outside the working tree (the task's `$TMPDIR`,
`host/run-<filter>/`, with each chunk's `.qpa`). The case lists are the union
of `android/cts/main/gles3-main-*.txt` of that checkout. The main tip at the
time of writing, `68e7e129`, differs from `1949cd0c` in no file of
`src/vivante`, `src/shaderc`, `src/backend.rs`, `src/gl/draw.rs` or
`tests/gc_npot_repeat.rs` (`git diff --stat` empty), so every line cited here
holds there too.

| filter | cases | began | compile refusals | upload refusals (`Texture upload failed`) | NotSupported | picture verdicts (not results) |
|---|---|---|---|---|---|---|
| `functional.texture.swizzle.` | 83 | 83 | 0 | 0 | 0 | 70 Fail / 13 Pass |
| `functional.texture.shadow.` | 432 | 432 | **432** | 0 | 0 | 0 (all 432 stop at the compile) |
| `functional.texture.filtering.3d.` | 153 | 153 | 0 | 0 | 12 | 141 Fail |
| `functional.texture.vertex.` | 174 | 174 | 0 | 0 | 0 | 174 Fail |
| `functional.texture.wrap.` | 1,440 | 1,440 | 0 | 0 | 1,296 | 144 Fail |
| total | 2,282 | 2,282 | 432 | 0 | 1,308 | 529 |

Where the numbers come from:

* **Compile refusals, `shadow`: 432 of 432**, all one reason, at the same
  place the local experiment hit (section 3 above):
  `online shaderc: the code generator refused this fragment shader:
  Select(Unsupported("an ES 3.00 texture look-up (a shadow sampler, an integer
  result, a gradient, an offset or a cube fetch) is not selected yet"))`, source
  `src/shaderc/online/select.rs:2686-2691`; dEQP's message is `Failed to compile
  shader program at glsTextureTestUtil.cpp:411`. The group is 3 targets (2d,
  cube, 2d_array) x 6 filters x 8 compare functions x 3 formats
  (`depth_component16`, `depth_component32f`, `depth24_stencil8`,
  `es3fTextureShadowTests.cpp:1035-1057`).
* **`swizzle`, `filtering.3d`, `vertex`, `wrap`: no compile and no upload
  refusal.** The GC7000UL accepts the textures and compiles the shaders; what is
  wrong with swizzle and wrap R is a *picture* (the swizzle and the W wrap are
  descriptor words, see `gc-texture-swizzle-wrap.md` section 3), which the host
  cannot see. The whole baseline
  of those gaps is therefore **0 refusals**: a later item's host verdict for
  them is the unit tests of section 5, not this run, and the board's dEQP run
  (#28) is the only render verdict.
* **NotSupported, `filtering.3d` 12**: `GL_EXT_texture_sRGB_R8` (6) and
  `GL_EXT_texture_sRGB_RG8` (6) are not offered (`es3fTextureFilteringTests.cpp:75,78`).
  **`wrap` 1,296**: ASTC 1,008 (`ASTC not supported`) and the EAC/ETC2
  sRGB/punch-through/R11/RG11 formats 288 (`Compressed texture format not
  supported at gluTexture.cpp:142`); `etc1`, `etc2_rgb8`, `etc2_eac_rgba8`
  and `rgba8` run (36 cases each, `Image verification failed` is a picture).
* **What `texture.wrap` covers.** It is S and T of 2D textures only (36
  combinations per format, `texture.wrap.rgba8.<s>_<t>_<filter>_<pot|npot>`);
  **wrap R is exercised by `texture.filtering.3d.combinations.*`**
  (`<min>_<mag>_<s>_<t>_<r>`, 27 R/S/T combinations for `linear_linear`,
  `es3fTextureFilteringTests.cpp:1775-1792`), by `texture.vertex.3d` (57
  cases) and the 3D cases of `texture.mipmap` and `texture.specification`, which
  set `GL_TEXTURE_WRAP_R` too.
* **`vertex` 174** (2d 39, cube 39, 2d_array 39, 3d 57): picture verdicts only;
  vertex-stage look-ups compile.


## 7. Implementation items, as briefs

Three items, in this order; the first two are host work.

**gc-shadow-sampler** (`src/vivante/regs.rs`, `src/vivante/texture.rs`,
`tests/es3_shadow_gc.rs` cases 1-6). Words: SAMP_CTRL0 bit 17 `COMPARE_ENABLE`,
bits 18-20 `COMPARE_FUNC` (`N:1974-1977`), table `[7,2,4,0,3,5,1,6]` (`X:132-139`),
MIP LINEAR -> NEAREST when a compare is on (`D:324-328`); `compare_op: 16` in the
incomplete-texture stand-in; every hand-built `GlesTextureBind` that pins the
whole SAMP_CTRL0 (`tests/gc_npot_repeat.rs`, `src/vivante/backend_tests.rs`)
gets `compare_op = 16` (4.1). Adds the swizzle clause if gc-swizzle landed. No
table-struct field is added. Docs: `docs/es3/backend-caps.md` (GC7000UL, "Compare
sampler"), RC5 in `docs/es3-cts-failures.md`.

**gc-shadow-compiler** (`src/shaderc/online/select.rs`,
`src/shaderc/online/vivante/builtins.rs`, `tests/es3_shadow_gc.rs` cases 7-10).
Instructions: `texld` with src2 = the reference lane (`K:555-558`, `M:206-233`),
`texldlpcf` for an explicit level; one-lane result; shadow with a bias, a
projective form, an offset or gradients stays refused by name. **First step: a
reference listing** of the four probe shaders of section 3 from the reference
compiler (`$ETNA_SHC`, the `etna_shc` built from Mesa 26.2.1, a deps-cache tree on
the nodes that have it) to fix the destination mask, `TEX_SWIZ` and SRC2 swizzle,
and add them to the corpus; until then the arm is "transcribed, not measured".
Regenerate whatever moves in the same commit (`src/vivante/internal.rs`, pinned
fixtures: none expected to move). Host dEQP target: `texture.shadow.` 432
compile refusals -> 0 (the picture is the board's).

**gc-shadow-board** (board, #28 / #24 session): TE compare on `EXT_R32F` texels,
PCF with LINEAR, cube and 2D array, the SRC2 form; the 32f clamp divergence
(4.1). If (a) fails, the choices are the `D24X8`/`D16` sample of
`gc-depth-sample.md` or the shader route (4.3, a per-draw key beside `LodKey`).

Ordering against running work: `wide128-shader` (touches `src/gl/draw.rs`) and
`etc2-srgb-desc` (`src/vivante/texture.rs`) first; `gc-shadow-sampler` and
`gc-swizzle`/`gc-wrap-r` all edit `texture.rs` and should be merged one at a
time; `gc-shadow-compiler` does not touch `texture.rs` and can run beside any of
them.
