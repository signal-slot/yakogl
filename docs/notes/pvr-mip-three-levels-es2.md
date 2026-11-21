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

**dEQP source: unverified.** `VK-GL-CTS` is not in the dependency cache (only
`VK-GL-CTS-gles3/data`, the shader corpus; `tools/cts/deps.toml:253-272`),
and a sparse fetch of `modules/gles2` was refused here. So the reference
generator and threshold of dEQP's mipmap cases are **not read from the CTS
source** in this note. Where a claim about them is needed it is marked
*unverified* and the section says what to read. The repo's own citations of
that file are `es2fTextureMipmapTests.cpp:330-340` (lod precision,
`docs/notes/v3d-qpu-grad.md:204`) and `:658`, `:669`
(`docs/online-shaderc/v3d-backend.md:2414-2418`), commit `93bca01` of
`deps.toml`; they are second-hand here.

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
3. **Verdict: never default-on for every texture; safe for render targets
   (terrain).** Details in the table. A render target is what glmark2's
   per-frame chains are; the dEQP cases are uploads. The deciding fact the
   notes do not have is dEQP's threshold (unverified), and with it a board
   run of the 20 cases below.

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
format's own precision is dEQP's side (unverified, below).

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

### What each compares against, with its tolerance

**Unverified.** From the repo: the 20 cases sit in the pass tally of the
AM62P (`docs/es2-cts-failures.md:20`, 17,140 pass, 0 fail over the tree), so
today's chain (one kick a level, TPU bilinear, 8-bit) passes them. That
chain is not the CPU box on NPOT levels 4 and up of 800x600
(`mip-multilevel/README.md`, finding), and the case sizes are POT (a dEQP
chain of the kind `pvr-mip-three-levels.md` lists as 64x64, 6 levels; also
unverified, read it in the `generate` group's init: the sizes of
square and non-square) where `iter` is byte-identical to the CPU.

To fill in from `es2fTextureMipmapTests.cpp` at `93bca01` (what to read, so
nobody has to guess): (1) does the generate case build its reference from
the driver's own levels (read back), or by dEQP's box filter of level 0,
and with which rounding; (2) the pass threshold of the sampled-image
comparison (`lodThreshold`/`colorThreshold`, from the format's bit depths)
and the `lodBits` (the repo cites 6 for ES 3 and 4 for projected,
`v3d-backend.md:2414-2418`); (3) the case sizes. The repo's own recollection,
**not verified here**: dEQP compares the rendered image of the textured quad
with a reference sampled through its own `TextureRenderer`, with a
per-format threshold that grows as the bit depth falls. If that holds, the
four rgba8888 cases have the tightest threshold (about 1-2 LSB at 8 bits)
and the rgb565, rgba4444, rgba5551 cases are loose (a 4 to 6 bit step).
Under that reading a 1 LSB level error passes the loose cases and is on the
edge for the rgba8888 and rgb888 ones.

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
| dEQP `mipmap.2d.generate.rgba8888_*` and `rgb888_*` (8 cases) | 3, 4, 6 of a 64x64 chain (or the dEQP sizes) | not measured at the dEQP sizes; 1 LSB at 256x256 levels 3-8 | dEQP's threshold, unverified, plausibly 1-2 LSB at 8 bits | **unknown**; at the edge if the threshold is 1 LSB | **not default-on** until the 8 cases are run |
| dEQP `generate.rgb565_*`, `rgba4444_*`, `rgba5551_*` (12 cases) | same | 1 LSB at 8-bit store | a 4 to 6 bit threshold, unverified | likely (one 8-bit LSB is a fraction of a 4-bit step), unverified | **not default-on** with the first group (same code path); no extra risk |
| dEQP `generate.{a8,l8,la88}_*` (12), `cube.generate.*` (16) | none | not on the GPU chain | - | yes, by routing (`texture.rs:4854`, `mip_chain_gpu` `:7249`) | unaffected |
| glmark2 terrain per-frame chains (5 a frame) | 3+ of 800x600, 3+ of 256x256 | 0 on levels 1-2; 1 to 2 on the ALU levels | 8 on probes and mean | **yes**: the picture reads level 1-2 at weight 1, levels 1 at weight <= 1.9%; those levels are exactly the TPU levels | **safe** (render target) |
| glmark2 terrain uploads (3 JPEG, specular) | 3+ of 512x512 | 1 LSB expected (POT, unmeasured at 512) | 8 | yes: <= 1 LSB through weight <= 1 | safe for the picture |
| glmark2 `texture-filter=mipmap` (crate, 512x512) | 3+ | 1 LSB expected | 8 | yes, the picture reads some level at weight <= 1; a probe moves <= 1 | safe for the picture |
| glmark2 refract (1600x1200) | 3+, NPOT odd steps | not measured (NPOT, like 800x600: 2) | 8 | probably (<= 2 against 8), **unmeasured**; the levels it reads are not studied | needs a measure and a read of its LOD before it is called safe |

**Default-on verdict: not safe, with one exception.**

- The 20 dEQP uploads cannot be covered by the measurements, and nothing in
  the repo says dEQP's threshold. A path that moves 1-2 LSB of a level the
  case samples is a risk to a CTS row that today passes (c3's "every ES 2.0
  answer the same", plan goal). Default-on is **not** safe until those 20
  cases run on the AM62P with the knob on and come back Pass
  (`tools/cts/RESULTS.md` row, a board session), and the CTS reference and
  threshold above are read from the source.
- **Safe only for render targets (terrain's per-frame chains):** the picture
  tolerance (8) is four times the largest measured error (2), and the
  levels the frames sample are the byte-identical TPU levels. That is an
  argument for turning it on for a texture that a draw rendered into (the GL
  core keeps a render-into mark per texture slot, item `pi4-target-mark-clear`;
  whether the offer can pass it to the backend is unverified here and is
  the implementer's question), and leaving uploads on one kick a level.
  Without such a gate the knob is **all or nothing**.
- **Never:** nothing is "never" on the evidence: no measured level is out of
  tolerance (every box-tap level is within 2 of 8). The case that would be
  "never" is a dEQP threshold of 0 LSB on rgba8888, which the CTS source
  would show; if so the 8 rgba8888/rgb888 cases make default-on "never" and
  render-targets-only the only form.

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
3. **The render-target gate**, if the render-targets-only form is built: a
   rig test with a texture rendered into (kicks 4 a chain) and one uploaded
   (kicks 9 a chain, today's one kick a level) at the same size.
4. `tests/es2_report_guard.rs` and `tests/pvr_mipgen_program.rs` unchanged
   and green.
5. The board answer is a separate session: the 20 dEQP cases by name with
   the knob on, and the three glmark2 scenes' `[picture]` lines against the
   `am62p` column (`tools/cts/glmark2-pictures.txt`, tolerance 8).

## What is left, for the planner

- Read `es2fTextureMipmapTests.cpp` at `93bca01` for the reference
  generator, threshold and sizes of the `generate` cases, and
  `grep generateMipmap` over `modules/gles2` and `modules/glshared` for the
  other users. This note marks both unverified; every verdict above that
  hangs on them is marked.
- `pvr-mip-three-levels.md`, part (e), says 4 of the 32 `generate` cases
  reach the GPU chain; by the store the answer is 20 (the table above). That
  note belongs to `pvr-mip-three-levels`; the planner should pass this
  correction to it.
- Measure the box-tap error at 512x512 and 1600x1200
  (`mip-multilevel/compare.py --sizes`), and read refract's LOD.
