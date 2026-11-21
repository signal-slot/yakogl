<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: what the three-levels mip path would change in ES 2.0 answers and glmark2 pictures

Plan #329 item `pvr-three-levels-es2-impact`, issue #1199. Written 2026-11-21
at main `d2767897`; every `file:line` is that commit's. Study only: no code,
no board. The path is `pvr-mip-three-levels` (off by default behind a
`Backend` knob; **not on main yet**, so no knob exists at this commit: the
only chain knob in `src/pvr/backend.rs` is `mip_cpu_tail`, `:1697`). Its design
is `pvr-mip-three-levels.md`, its error `tools/hostharness/mip-multilevel/README.md`
(`mip-multilevel-error`). Nothing here was run: the verdicts are read from
the code and from the measured errors of those notes.

**dEQP source: read.** `VK-GL-CTS` at `93bca01` (the commit of
`tools/cts/deps.toml:259`) was fetched as a blobless partial clone into
`$CONNECTED_DEPS_DIR/VK-GL-CTS-es2part` (its own directory, so the sparse
`VK-GL-CTS-gles3` of `deps.toml:253-272` is untouched; only the files below
were read). `E` = `modules/gles2/functional/es2fTextureMipmapTests.cpp`,
`G` = `framework/opengl/gluTextureTestUtil.cpp`. One definition is not read:
`computeFixedPointError` (called by `computeFixedPointThreshold`,
`framework/common/tcuTexLookupVerifier.cpp:2199-2201`, defined elsewhere), so
the size of the threshold in LSBs below is marked unverified.

## Answer

1. **No API answer changes.** `tests/es2_report_guard.rs` runs a stand-in
   whose `generate_mipmap` is `None` (`tests/es2_report_guard.rs:132`), so it
   never reaches the chain. The only things at stake are texels.
2. **Texels that change** are the levels that are not "TPU, byte-identical by
   construction" (`pvr-mip-three-levels.md`, part (b)): levels 3, 4, 6, 7, 9
   of an 800x600 chain, 3, 4, 6, 7 of a 256x256 one, 3, 4, 6 of a 64x64 one.
   Levels 1, 2, 5, 8 cannot move. Measured size of the move (host model):
   max 1 LSB on every level of 256x256, 1 to 2 LSB on 800x600
   (`mip-multilevel/README.md`, result table).
3. **Verdict: default-on is safe for every ES 2.0 case and glmark2 scene
   this study could measure, on the evidence of the CTS source and the host
   error, with three things only a measurement closes** (the AM62P board run
   of the 20 cases, the error at 512x512 and 1600x1200, and refract's
   levels). dEQP's `generate` cases compare each stored level with the level
   above it in the *same* texture, not with a reference filter (part (a)),
   so a level that is 1-2 LSB from the iterated chain is judged against its
   own neighbour, with a threshold of several LSB. See the table in (c).

## (a) dEQP-GLES2 cases that reach the GPU chain on the AM62P

### The routing

`glGenerateMipmap` offers the chain to the backend only when all of these
hold (`src/gl/texture.rs:4854-4861`): `t.target == GL_TEXTURE_2D`,
`t.texfmt == GLES_TEXFMT_RGBA8`, `t.es3_fmt == 0`, `t.layers <= 1`,
`t.chain_levels >= 2`, `!t.bgra`, and no level above 0 attached to a
framebuffer. The PowerVR then takes it in `mip_chain_gpu`
(`src/pvr/backend.rs:7248`) unless it refuses: `format != RGBA8 || bgra`,
not 2D, an address outside the heap (`:7260`), a level one texel wide above a
taller tail (`:7280`, the PBE writes pixel pairs). It is registered as
`generate_mipmap: Some(pvr_generate_mipmap)` (`:10553`). The path is always
all-GPU: `mip_cpu_tail` is false by default, so `mip_gpu_last` returns every
level (`:7241-7243`) and the GL core sees `MipChain::Whole`
(`src/gl/texture.rs:4866-4875`).

The consequence that corrects an earlier statement. `pvr-mip-three-levels.md`
(part (e)) says that only 4 of the 32 `generate.*` cases take the GPU chain
(plain `rgba8888`). The store decides, not the dEQP format name:
`sized_format` maps `GL_RGBA4`, `GL_RGB5_A1`, `GL_RGB8`, `GL_RGB565`, `GL_RGB`
and `GL_RGBA` all to `GLES_TEXFMT_RGBA8` (`src/gl/texture.rs:6390-6391`), and
`texfmt_of` narrows only luminance/alpha, and only where the backend samples
them narrow (`:6594-6603`; the PowerVR's `pvr_debug_narrow_la` is 1,
`src/pvr/backend.rs:2051`, `narrow_la_on` `:2079`). So on the AM62P:

| dEQP-GLES2 case group (`tools/cts/gles2-mustpass.txt`) | Lines | Cases | Store | Reaches the GPU chain? |
|---|---|---:|---|---|
| `texture.mipmap.2d.generate.rgba8888_{fastest,nicest}`, `_non_square_{fastest,nicest}` | 14907-14910 | 4 | RGBA8 | **yes** |
| `texture.mipmap.2d.generate.rgb888_*` (4) | 14895-14898 | 4 | RGBA8 (RGB padded) | **yes** |
| `texture.mipmap.2d.generate.rgb565_*` (4) | 14891-14894 | 4 | RGBA8 (565 expanded on upload) | **yes** |
| `texture.mipmap.2d.generate.rgba4444_*` (4) | 14899-14902 | 4 | RGBA8 | **yes** |
| `texture.mipmap.2d.generate.rgba5551_*` (4) | 14903-14906 | 4 | RGBA8 | **yes** |
| `texture.mipmap.2d.generate.{a8,l8,la88}_*` (12) | 14879-14890 | 12 | L8 / A8 / LA8 narrow | no (`format != RGBA8`, CPU filter) |
| `texture.mipmap.cube.generate.*` (16) | 14931-14946 | 16 | RGBA8 or narrow, but a cube | no (`target != GL_TEXTURE_2D`, `:4854`) |

That is **20 cases** (the table's first five rows), not 4. For the four
non-RGBA8-named formats the store is RGBA8 bytes already widened from the
narrow format, and the CPU filter averages those bytes
(`src/gl/texture.rs:4909-4912`), so the GPU's 2x2 average of the same bytes is
the same operation; the three-levels path changes which texels differ by an
LSB, not what is averaged. Whether dEQP then compares with the narrow
format's own precision is the threshold of the next section.

Other `glGenerateMipmap` users among the dEQP-GLES2 cases are **unverified**:
the mustpass list names no case by its calls. The list has
`negative_api.texture.generatemipmap_*` (4 cases, lines 3844-3847: error
answers, no texels, they stop before the offer: `incomplete_cube`,
`invalid_target`, `npot_wdt_hgt`, `zero_level_array_compressed`) and the
`state_query.integers.generate_mipmap_hint_*` (3, lines 14265-14267: hint
state, no chain). Which other group builds a chain with the call, if any
(`texture.filtering.*`, `texture.wrap.*`, `texture.size.*`, `fbo.render.*`),
needs one read of the CTS source for `generateMipmap` in
`modules/gles2/functional/` (and `modules/glshared/`), or the host harness
run with a call counter at `pvr_generate_mipmap`. Until then the table lists
only the 20 above, and the verdict below covers any other 2D RGBA8 chain by
the same rule.

### What each compares against, with its tolerance (CTS source)

The 20 cases are `Texture2DGenMipmapCase` (`E:770-880`), named
`<format>[_non_square]_<hint>` (`E:1170-1195`):

- **Content and sizes.** Level 0 is an 8-texel colour grid of two colours,
  (1, 0.5, 0, 0.5) and (0, 0, 1, 1) (`E:816-817`); the sizes are 64x64 and
  `non_square` 32x64 (`E:1121-1122`), so both are POT, 6 levels above 0
  (`numLevels = log2floor(max) + 1`, `E:808`). Formats and types
  `E:1101-1108`; hints `E:1110-1114` (FASTEST, NICEST; the backend ignores
  them, ES 2.0 asks for no filter).
- **How the levels are read.** `glGenerateMipmap` (`E:829-832`), then each
  level is drawn 1:1 with `GL_NEAREST_MIPMAP_NEAREST` (`E:801`, loop from
  `E:841`) and read back with `glReadPixels`. The comparison sees the
  texels the GPU wrote, after the sampler.
- **What it compares against.** `compareGenMipmapResult` (`G:3478-3555`):
  level 0 against dEQP's own level 0 (threshold as below); then **each
  level l against the result's own level l-1**, not a reference chain:
  (1) a bilinear check (`G:3348-3383`): the pixel must be a valid linear
  sample of the level above at the scaled centre, within `colorThreshold`
  and `uvwBits` = 4 fraction bits (`E:863`); (2) if that fails, a box check
  (`G:3385-`); a level that passes neither is a **QualityWarning**
  (`G:3527-3529`), not a failure; (3) only if a pixel then falls outside the
  min/max of its source neighbourhood (`G:3422-3476`, "very lenient", one
  texel margin) is the case failed (`G:3531-3542`).
- **Threshold.** `colorThreshold = computeFixedPointThreshold(cmpBits)`
  (`E:862`), with `cmpBits` = min(framebufferBits - 2, formatBits) per
  channel where the format has the channel (`E:855-861`). On an 8-bit
  framebuffer that is 6 bits for rgba8888/rgb888, 5 for rgb565's R and B,
  4 for rgba4444, 5 for rgba5551's colours. In LSBs of 8 bits that is about
  4 (rgba8888, rgb888) and 8 or more (the 16-bit formats):
  *unverified* (the definition of `computeFixedPointError` is not read);
  the repo's framebuffer is RGBA8 on the AM62P (assumed, not checked).

What this means for the three-levels path: a level that the shader makes
from level l-2 in one pass is checked against the stored level l-1, with
a bilinear-sample-of-the-level-above rule and a 4-LSB-order threshold. The
measured difference from the iterated chain is at most 1 LSB on POT levels
(`mip-multilevel/README.md`, 256x256), and the difference between a one-pass
box and the 2x2 box of the stored level above is of the same size, so the
bilinear check passes (the step is even, the weights exact, the grid's
colours uniform in 8-texel cells: levels 1-3 of the 64x64 grid are the cell
colours exactly, and only levels 4-6, where a texel straddles cells, can
differ by a rounding). Today's chain passes the 20 cases (the AM62P tally,
`docs/es2-cts-failures.md:20`: 17,140 pass, 0 fail, 5 QualityWarning
over the tree). Which of the five QualityWarnings, if any, are these cases
is not known here; a three-levels change must not add one.

## (b) glmark2 scenes that call `glGenerateMipmap`

glmark2 calls it at `texture.cpp:95` (`Texture::load` of an image with a
mipmap min filter), `scene-terrain/base-renderer.cpp:89` (`update_mipmap`)
and `scene-refract.cpp:430`; those are all the call sites in
`$CONNECTED_DEPS_DIR/glmark2-src/src` (`grep GenerateMipmap`, pin `22c527cb`).
Three scenes reach them (`gc-mipmap-cpu-cases.md`, "The calls, per scene";
the 30 others never call it). The column `am62p` of
`tools/cts/glmark2-pictures.txt`, tolerance **8** levels per channel
(`:26`): `judge_picture` (`tools/cts/common-tally.py:351`) compares five
probes (c, tl, tr, bl, br) and the 40x30 mean of the **final picture**, each
within 8, and the size.

| Scene (picture line) | Call, texture | Size, levels above 0 | When | Levels the picture samples | Reaches GPU chain on AM62P | Tolerance |
|---|---|---|---|---|---|---|
| `[texture] texture-filter=mipmap` (`:99`) | `texture.cpp:95`, crate image upload | 512x512 (POT), 9 | once, at load | all levels the cube's minification reaches (not bounded; unmeasured) | yes | 8 |
| `[terrain]` (`:117`), the uploads | `texture.cpp:95`: 3 JPEG uploads; `base-renderer.cpp:89`: specular map | 512x512, 9 | once at load / setup (and the specular map once after its render) | all levels, LOD up to ~9 (`terrain-mip-usage.md`, specular row) | yes | 8 |
| `[terrain]`, the 5 per-frame chains | `base-renderer.cpp:89`: terrain target twice, tilt-shift H (800x600); bloom H, bloom V (256x256) | 800x600 NPOT, 9; 256x256, 8 | every frame, 43 kicks a frame | levels 1 and 2 of the first terrain build only; the other four at LOD 0, level 1 with weight at most 1.9% (`terrain-mip-usage.md`, per-chain table) | yes | 8 |
| `[refract]` (`:119`) | `scene-refract.cpp:430`: `tex_[COLOR]`, twice the canvas | 1600x1200 NPOT, 10 | every frame | not studied (`terrain-mip-usage.md` is terrain only) | yes (`am62p-pvr-mipmap-kicks.md:50`) | 8 |

The texel changes of the three-levels path are in levels the pictures
rarely read (the ALU levels 3+ only; levels 1, 2, 5, 8 are byte-identical),
and a level is sampled with a weight of at most 1, so an LSB-sized level
error moves a probe or the mean by at most that much (the bound
`mip-multilevel/README.md`, "Tolerance", states).

## (c) The verdict per case, with the measured error

Errors from `mip-multilevel/README.md` (worst max abs error over 16 images,
levels per channel; box taps against the CPU box): 256x256 levels 1-2: 0,
3-8: 1; 800x600 levels 1-2: 0, 3-5: 1, 6-9: 2. `mip-multilevel-error`
modelled `0` for the TPU levels and did not measure the ties of the
shader's `Q` on the board (README, "Not modelled"). Measured sizes:
800x600 and 256x256 only, so **512x512 and 1600x1200 are not measured**; POT
512 is the 256x256 case (every weight exactly 1/2), 1600x1200 is NPOT like
800x600 and has odd steps at levels 5, 6, 8 (75, 37, 9 high).

| Case | Levels that can move | Error (measured) | Tolerance | Keeps the answer? | Verdict |
|---|---|---|---|---|---|
| dEQP `mipmap.2d.generate.rgba8888_*` and `rgb888_*` (8 cases) | levels 3, 4, 6 of the 64x64 and 32x64 chains; on this grid content only 4-6 can differ | 1 LSB on POT levels 3-8 (256x256; 64x64 not measured, same weights) | each level against its own level above, bilinear then box then min/max, threshold about 4 LSB (unverified size) | **yes, by the source**: 1 LSB against about 4; a level passes the bilinear check, or at worst a QualityWarning | **default-on safe**, pending the board run |
| dEQP `generate.rgb565_*`, `rgba4444_*`, `rgba5551_*` (12 cases) | same | 1 LSB in the 8-bit store | the same with a 4 to 5 bit threshold (about 8 LSB or more, unverified) | yes, with more margin than the first group | **default-on safe** |
| dEQP `generate.{a8,l8,la88}_*` (12), `cube.generate.*` (16) | none | not on the GPU chain | - | yes, by routing (`texture.rs:4854`, `mip_chain_gpu` `:7249`) | unaffected |
| glmark2 terrain per-frame chains (5 a frame) | 3+ of 800x600, 3+ of 256x256 | 0 on levels 1-2; 1 to 2 on the ALU levels | 8 on probes and mean | **yes**: the picture reads level 1-2 at weight 1, levels 1 at weight <= 1.9%; those levels are exactly the TPU levels | **safe** (render target) |
| glmark2 terrain uploads (3 JPEG, specular) | 3+ of 512x512 | 1 LSB expected (POT, unmeasured at 512) | 8 | yes: <= 1 LSB through weight <= 1 | safe for the picture |
| glmark2 `texture-filter=mipmap` (crate, 512x512) | 3+ | 1 LSB expected | 8 | yes, the picture reads some level at weight <= 1; a probe moves <= 1 | safe for the picture |
| glmark2 refract (1600x1200) | 3+, NPOT odd steps | not measured (NPOT, like 800x600: 2) | 8 | probably (<= 2 against 8), **unmeasured**; the levels it reads are not studied | needs a measure and a read of its LOD before it is called safe |

**Default-on verdict: safe on the evidence read, three items open.**

- **dEQP's 20 cases.** The case never compares with a reference filter. It
  compares each stored level with the stored level above, with a bilinear
  rule and a threshold of several LSB (part (a)), and the path's error is
  1 LSB on POT levels. So it keeps "every ES 2.0 answer the same" at the
  case level. The risk that remains is a QualityWarning (a level that
  fails both the bilinear and the box check): `docs/es2-cts-failures.md:20`
  counts 5 on the AM62P today and the count must not grow. A board run of
  the 20 cases by name with the knob on is the proof (it is the board
  item's, not this note's).
- **glmark2.** Terrain's per-frame chains read levels 1 and 2, which are
  TPU levels and byte-identical; the other four per-frame chains are read at
  level 1 with weight at most 1.9%. The uploads and the crate image read
  every level but at an error of 1 LSB against a tolerance of 8. Safe.
- **Refract** (1600x1200, NPOT, odd steps at levels 5, 6, 8) is the one
  case not covered: the error there is not measured and its sampled levels
  are not studied. If the error there is within 2 (as 800x600) the
  tolerance of 8 holds. Until measured, the verdict for refract is
  "likely safe, unmeasured".
- **"Safe only for render targets" is not needed** by the source, and it
  would cost a gate the backend offer does not have today (the GL core has
  a render-into mark per slot, `pi4-target-mark-clear`; passing it to the
  backend is unverified here). It stays the fallback if the board shows a
  QualityWarning on the uploads.
- **"Never"** would need a level out of its threshold. None is: every
  measured box-tap level is within 2 of a tolerance of 8, and dEQP's
  threshold is about 4 or more. NPOT chains differ from the CPU box on
  levels 4+ already today (`mip-multilevel/README.md`, finding), and the
  box taps are closer to it, so the path improves that.

## The host test that would pin it

Pin it on the source of the answer, not on a picture: the host harness draws
nothing (`tools/hostharness/glmark2/README.md`), so the host `host` column
(all-black) cannot show a texel move.

1. **Which textures reach the chain** (the part this note adds): in
   `src/pvr/backend_tests.rs` or `tests/mipmap_offer_guard.rs`'s style, with
   the knob on, offer a chain for each of `GL_RGBA`, `GL_RGB`, `GL_RGB565`,
   `GL_RGBA4`, `GL_RGB5_A1`, `GL_LUMINANCE`, `GL_ALPHA`, `GL_LUMINANCE_ALPHA`
   (2D, 64x64 and 64x32) and a cube, and assert the kicks and the answer
   (`MipChain::Whole` or not): five on the GPU, the narrow three and the
   cube not. This pins the 20-case count above.
2. **The level contents:** the level-content test of `pvr-mip-three-levels.md`
   (e), item 1: reference IR evaluator against today's chain at 64x64,
   64x32, 32x64, 128x128, 256x256, 800x600 and (new) 512x512 and 1600x1200,
   asserting levels 1, 2, 5, 8 byte-equal in every TPU model, and every
   level's max <= 1 under the `gl` and `w8` models (<= 2 at the NPOT sizes).
   The added sizes close the two unmeasured sizes in the table above.
   Add the dEQP check itself: for the dEQP grid at 64x64 and 32x64, each
   level l of the new chain must be a valid bilinear sample of the stored
   level l-1 within the 4-LSB-order threshold (`G:3348-3383`, the pass a
   QualityWarning needs to avoid).
3. **The render-target gate**, if the render-targets-only form is built: a
   rig test with a texture rendered into (kicks 4 a chain) and one uploaded
   (kicks 9 a chain, today's one kick a level) at the same size.
4. `tests/es2_report_guard.rs` and `tests/pvr_mipgen_program.rs` unchanged
   and green.
5. The board answer is a separate session: the 20 dEQP cases by name with
   the knob on, and the three glmark2 scenes' `[picture]` lines against the
   `am62p` column (`tools/cts/glmark2-pictures.txt`, tolerance 8).

## What is left, for the planner

- **Still unverified:** the size in LSBs of `computeFixedPointThreshold`
  (`computeFixedPointError`'s definition is not read) and which other
  dEQP-GLES2 cases call `glGenerateMipmap` besides the `generate` group
  (the CTS tree is not complete here; a `git grep` over a blobless clone
  fetches every blob and was stopped). A complete `VK-GL-CTS` checkout
  (`DEQP_WORK`) answers both in one grep.
- `pvr-mip-three-levels.md`, part (e), says 4 of the 32 `generate` cases
  reach the GPU chain; by the store the answer is 20 (the table in part
  (a)). That note belongs to `pvr-mip-three-levels`; the planner should pass
  this correction to it. The same note's "dEQP compares with a threshold
  that allows a filter's rounding" is right, but the case compares level
  against level, not against a reference: its host test should compare
  each level of the new chain with the box of the stored level above,
  as `compareGenMipmapBilinear` does, in addition to the iterated chain.
- Measure the box-tap error at 512x512 and 1600x1200
  (`mip-multilevel/compare.py --sizes`), and read refract's LOD.
