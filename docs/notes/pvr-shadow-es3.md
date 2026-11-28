<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: the 288 AM62P dEQP-GLES3 texture.shadow not-pass cases (T-SHADOW), by sub-cause

Plan #400 item `design-shadow`, written 2026-11-28. Desk work only: nothing
here was run on a board. The case list is the T-SHADOW row of
`python3 -I tools/cts/am62p-notpass-causes.py docs/es3/am62p-gles3-notpass.tsv`
(288 lines). Code is cited at origin/main `14e7a742` (`src/pvr/backend.rs`,
`src/shaderc/online/powervr/select.rs`, `src/gl/draw.rs` and
`src/shaderc/mod.rs` are the same at the integration tip `5f781c49`). dEQP
is VK-GL-CTS `93bca01`, `modules/gles3/functional/es3fTextureShadowTests.cpp`
(`ES3:<line>`). How the per-texel compare is built is in
`docs/notes/pvr-pcf.md`; the 2D-array layouts in `docs/es3/pvr-2darray.md`.

## 1. Where the 288 lines come from

All 288 are in tree row 39 (counted from 0; `texture.shadow.*` shares it
with `filtering.cube`, `format`, `mipmap` and `size`), whose TSV data is
still the `ad2b5ed` run of 2026-10-13/14 (the TSV header: "rows 22-45: build
ad2b5ed"). `ad2b5ed` predates the history squash `381b0f09` (2026-10-27) and
every commit below, so the TSV shows the shadow path before any of the
fixes in section 4. The board-d rerun of rows 39-45 at `472630df` had not
reached the TSV when this note was written.

dEQP's group (`ES3:1021-1150`): 3 targets x 6 filters x 8 compare functions
x 3 formats = 432 cases. The filters are `nearest` (NEAREST/NEAREST),
`linear` (LINEAR/LINEAR), and the four mipmap filters, **each with a LINEAR
magnification filter** (`ES3:1044-1049`). The formats are
DEPTH_COMPONENT16, DEPTH_COMPONENT32F and DEPTH24_STENCIL8. Textures: 2D
32x64 with a full chain (7 levels) and GL_REPEAT both ways; cube 32 (6
levels, verified as seamless clamp-to-edge); 2D array 32x64, 8 layers, 7
levels, REPEAT (`ES3:1140-1144`). Each case draws six quads, with
references of 0.5 (or 1.0 and 0.0 for EQUAL/NOTEQUAL) and the
out-of-range 1.1 and -0.1 (`ES3:275-294`). The array's quads start at
texture coordinates as far out as (-1.5, -2.7) and span the layer
coordinate from -0.5 to 8 (`ES3:883-909`). A case is verified first with
"high-quality PCF" bits (`pcfBits` 5). If that fails, it is verified again
without the PCF term, and a pass on that second try is a QualityWarning
"Low-quality result" (`docs/notes/v3d-shadow.md` section (f) has the
tolerances).

Not in the TSV, so Pass at `ad2b5ed`: every `always_*` and `never_*` case
(their result does not depend on the depth), and all 36 of the
`2d.nearest.*` and `cube.nearest.*` cases that compare a depth. So a single
TPU sample followed by the program's compare (`shadow_compare`, with the
reference clamp for D16/D24S8) was already exact. Every remaining line has
one of two reasons: a filter that blends depths before they are compared,
or (2D array only) the stacked layout.

## 2. Sub-causes

| id | family (cases) | verdict | filters | count | code reason at `ad2b5ed` | state at `14e7a742` |
|---|---|---|---|---:|---|---|
| SH-1 | `texture.shadow.2d.*` | QualityWarning "Low-quality result" | linear, nearest_mipmap_nearest, linear_mipmap_nearest, nearest_mipmap_linear, linear_mipmap_linear | 90 | compare after filtering: the TPU blends depths, then the program compares the blend (Mesa pco's shape, `pvr-2darray.md` 7.1) | **fixed at `919db72b`** (2D PCF arm), host only |
| SH-2 | `texture.shadow.cube.*` | QualityWarning "Low-quality result" | the same five | 90 | the same, on the cube | **fixed at `43af85d7`** (seamless cube PCF arm), host only; one board question open (section 4.2) |
| SH-3 | `texture.shadow.2d_array.nearest.*` | Fail "Image verification failed" | nearest | 18 | stacked 2D array: `array_uv` clamps v to one layer, so GL_REPEAT on T is lost | **fixed at `02c5c9af`** (`PVR_NATIVE_ARRAY_DEFAULT` = 1), host only |
| SH-4 | `texture.shadow.2d_array.{linear,*_mipmap_*}.*` | Fail "Image verification failed" | the five blending filters | 90 | SH-3's layout (REPEAT lost, and taps reaching the next layer at levels >= 1), plus SH-1's compare after filtering | **fixed at `02c5c9af` + `919db72b`** (native layer address, PCF arm on it), host only |
| | | | | **288** | | |

Each row is 6 compare functions (less, less_or_equal, greater,
greater_or_equal, equal, not_equal) x 3 formats x the filters named. That
gives 18 cases per filter: 5 x 18 = 90 and 1 x 18 = 18. The counts by
family match the TSV: 2d 90 QW, cube 90 QW, 2d_array 108 Fail, which
adds up to the T-SHADOW row's 288.

Every commit named in the last column is in `472630df`, the build that
board-c/board-d ran (`git merge-base --is-ancestor` for each). The
commits that touch the PowerVR sampler after `472630df` do not change
these cases:

- `ba2cc7d3` ("a nearest_mipmap_nearest depth-compare sampler takes its
  own filter word") changes the word only when the magnification filter
  is NEAREST. With dEQP's LINEAR magnification, `shadow_filter_word`
  returned `1|4|8|(levels-1)<<8` before and after: `linear_filter = 1`
  skips the early return (`src/pvr/backend.rs:7312-7314` at the tip).
  Its commit message names `texture.shadow.*.nearest_mipmap_nearest`;
  for dEQP's cases that is not so.
- `349f989f` (BASE_LEVEL, MIN_LOD, MAX_LOD reach the sampler) packs
  GL's defaults, which this group keeps, as before.

So the board rerun of row 39 at `472630df` measures exactly the tip's
code for all four sub-causes.

## 3. Code reasons, with file:line at `14e7a742`

### 3.1 SH-1 and SH-2: compare after filtering

ES 3.0.6 3.8.15.1 compares each texel, then filters the 0/1 results. The
BXS-4-64 has no attested hardware compare that pco's operands reach
(`pvr-2darray.md` 7.1). So at `ad2b5ed` the program compared the reference
against the depth the TPU had already filtered. Under LINEAR, and under the
LINEAR magnification of every mipmap filter, that is one value where dEQP
expects a weighted mix of 0s and 1s. dEQP's high-quality PCF bound rejects
it and its second try accepts it: QualityWarning. NEAREST/NEAREST reads one
texel, so it passed.

At the tip:

- The shadow arm (`select.rs:1911-1983`) samples the depth as the plain
  look-up, then gathers the footprint for the compare per texel:
  `pcf_taps` for 2D (`select.rs:3511`) and `pcf_cube_taps` for the cube
  (`select.rs:1980`, `:3697`; four one-texel gathers per level, seamless
  across edges and corners).
- `shadow_compare` (`select.rs:3375`) clamps the reference for a
  fixed-point depth (`:3390-3394`, bit 3 of sampler meta word 12, which
  `compare_op` in `src/gl/draw.rs:785` and `texture_fixed_depth` at
  `:799` set for D16 and D24S8 but not D32F). It compares each tap and
  picks the PCF arm by sampler meta word 13 bit 0.
- Word 13 is `shadow_filter_word` (`src/pvr/backend.rs:7308`, written at
  `:7239`). For dEQP's filters over the 7-level 2D chain it is `1`
  (linear), `1|4|8|6<<8` (nearest_mipmap_nearest), `1|8|6<<8`,
  `1|4|16|6<<8` and `1|16|6<<8`. For the 6-level cube it is the same
  with `5<<8`.

### 3.2 SH-3 and SH-4: the stacked 2D array

With the native array off, YakoGL keeps a 2D array as one tall image of
stacked layers, and the program maps (u, v, layer) into it with `array_uv`
(`select.rs:4210`):
`v = clamp(c.y, 0.5/h, 1 - 0.5/h)` (`:4239-4240`), with `h` level 0's layer
height. Two effects:

- **REPEAT on T becomes clamp.** dEQP's array quads start as far out as
  `oY = -2.7` and `-1.1` with GL_REPEAT (`ES3:883-889`, `:1140-1141`). So
  even NEAREST reads the wrong texel over most of the quad. That alone is
  SH-3 (18 Fail; the 2D `nearest` cases, which sample a real 2D image,
  pass).
- **Taps reach the next layer.** The clamp margin is half a texel of
  level 0. At levels >= 1 a bilinear footprint, and the TPU's own filter,
  reach past the layer's edge into the next layer. `addrmode_w` is
  CLAMP_TO_EDGE for everything but a native volume (`backend.rs:7278`):
  the stacked image has no third axis to wrap. Together with SH-1's compare
  after filtering, this is SH-4 (90 Fail, not QualityWarning: the second
  try's any-outcome bound is broken by the wrong texels, not only by the
  blend).

At the tip `PVR_NATIVE_ARRAY_DEFAULT` is 1 (`src/shaderc/mod.rs:314`,
since `02c5c9af`, 2026-11-27):

- The shadow arm takes the native branch (`select.rs:1961-1965`). The
  layer is an address, `base + clamp(floor(z + 0.5), 0, depth) * layer_size`
  (`array_layer_addr`, `select.rs:4270`), which matches dEQP's -0.5..8
  layer sweep with its clamp at both ends.
- The image the TPU sees is one layer's 32x64 with its own 7-level chain
  (`backend.rs:7083-7128`), so its REPEAT on u and v wraps inside the
  layer and its filter never leaves it.
- The draw binds that copy whenever the program's record says so
  (`src/gl/draw.rs:5142-5156`, `native_array::copy`). Depth formats are
  not excluded (`native_array.rs` `copyable`).
- The PCF gathers take the same layer address (`pcf_taps(..., Some(addr),
  ...)`, `select.rs:1964`).

## 4. Fix design per sub-cause

The fixes for all four sub-causes are on main. What is left is measuring
them, one board question, and the off-arm.

### 4.1 SH-1 (90), SH-3 (18), SH-4 (90): fixed at `919db72b` / `02c5c9af`

No code change is designed. The expected board outcome is that the row-39
rerun at `472630df` (board-d) shows 0 not-pass lines in
`texture.shadow.2d.*` and `texture.shadow.2d_array.*`.

If lines remain, the next step depends on what remains:

- **SH-4 alone, NEAREST passing.** Compare the gather's level and the
  levels the TPU walks. On the native array the descriptor's chain is the
  copy's (`backend.rs:7106-7110`, `single` from `mip_filter` and
  `mip_levels`), and the gather replaces the level.
  `tests/es3_pvr_2darray.rs:700` (`array-tail-test`, 64x32x3, 7 levels) is the
  host analogue to extend to 32x64x8.
- **Only the D16 / D24S8 1.1 and -0.1 draws.** These are the reference
  clamp. It is pinned by `tests/es3_pvr_shadow.rs:323`
  (`the_filtered_depth_is_compared_by_the_samplers_op`, bit 3), so the fault
  would be in the bind.
- **With `DEQP_PVR_NATIVE_ARRAY=0`** (the A/B arm), SH-3 and SH-4 are
  expected to stay Fail. The stacked layout cannot wrap T per layer without
  doing REPEAT/MIRRORED_REPEAT in `array_uv` (a `fract` before the clamp,
  per wrap mode from the descriptor). It also cannot keep a level >= 1
  footprint in its layer without a per-level clamp margin. Neither is
  planned: the stacked image is the fallback, and K6 (`es3_pvr_2darray.rs`
  `the_stacked_path_cannot_address_the_smallest_levels_of_the_same_array`)
  already rules it out for the smallest levels.

### 4.2 SH-2 (90): fixed at `43af85d7`, with one open board question

`pvr-pcf.md` section 6 leaves open whether the TPU's cube gather returns
the texel of the direction's own face as its first tap. `pcf_cube_taps`
reads each texel by a gather a quarter texel past that texel's centre
((i + 0.75)/n, (j + 0.75)/n), so that the first tap is the texel even if
the TPU's footprint origin is (i, j). If the board still shows cube
QualityWarnings while 2D passes, that assumption is the suspect.

Design for that case: replace each one-texel gather in `pcf_cube_taps` by a
plain NEAREST-style point read of the texel centre. That is a `smp` with
`lodm` replace at the tap's level and the sampler's filter overridden to
POINT, through the gather sampler words 16..19 that `tex_desc_sampler_lod`
leaves unused (`pvr-2darray.md` 7.2; Mesa fills them at
`pvr_arch_sampler.c:220-232`). The back end packs words 16..19 as words
8..9 with min/mag POINT and `mipfilter` off. The selector reads the texel
value from lane 0 of a normal sample instead of `sbmode` rawdata. That
costs one extra sampler state of four dwords per bound texture, nothing in
the GL core, and changes only `src/pvr/backend.rs` (`tex_desc_sampler_lod`)
and `select.rs` (`pcf_cube_taps`).

## 5. Host tests to add

HOST-ONLY: a host draw reads back black, so these pin programs, words and
the byte model, as `tests/es3_pvr_pcf.rs` does. Feature set, as those
tests: `--no-default-features --features host-test,gpu-powervr,shaderc-online`.

| sub-cause | test (new file) | what it pins |
|---|---|---|
| SH-3, SH-4 | `tests/es3_pvr_shadow_deqp_array.rs` | dEQP's own 2D-array geometry: a 32x64x8, 7-level depth array, GL_REPEAT, the six `ES3:883-889` quads (texture coordinates from (oX, oY) to (oX + sX, oY + sY), the layer from -0.5 to 8). The compiled `texture(sampler2DArrayShadow, vec4)` program runs in `tests/usc_exec` over `texture_descriptor` of the `GLES_TEXTYPE_2D_ARRAY_TWIDDLED` bind, for all six dEQP filters, against texref's `sample_layer_compare`. With the native array off, the same draws differ from texref (pins the SH-3 reason: REPEAT on T lost) |
| SH-1 | the same file's 2D part, or a case in `tests/es3_pvr_pcf.rs` | dEQP's 32x64 7-level texture and its six `ES3:288-294` lod/offset rows (coordinates past one REPEAT period), for the five blending filters with LINEAR magnification. Today's tests use a 16x8, 5-level chain |
| SH-2 | a case in `tests/es3_pvr_pcf_cube.rs` | dEQP's 32-texel, 6-level cube at the six `ES3:570-591` rows (all six faces), against `sample_cube_compare`. If section 4.2's point-read design is taken, its shape test (one `smp` per tap without `sbmode` rawdata, sampler words 16..19 POINT) |
| all | `src/pvr/backend.rs` unit test next to `the_shadow_filter_is_sampler_meta_word_1` (`:12165`) | word 13 for dEQP's exact six (min, mag) pairs over 7 and 6 levels: `0`, `1`, `1|4|8|top`, `1|8|top`, `1|4|16|top`, `1|16|top`. This pins that `ba2cc7d3` leaves dEQP's nearest_mipmap_nearest word unchanged |

## 6. src/gl changes

None is needed for any T-SHADOW sub-cause. `compare_op` and
`texture_fixed_depth` (`src/gl/draw.rs:785`, `:799`) already give the
compare function, the compare mode and the fixed-point clamp. The native
array copy (`draw.rs:5142-5156`) already serves depth arrays.

Seen on the way, not a T-SHADOW cause (dEQP's shadow group sets the filters
on the texture and binds no sampler object): the native array copy and the
native cube copy decide `single` from the texture's own filters
(`draw.rs:5146` and `:5115`, `sampling_fields(t)`). A bound sampler object
replaces those filters only afterwards (`draw.rs:5166`). With a sampler
object whose mipmap filter differs from the texture's, the copy's chain and
the descriptor's `mip_filter` / `mip_levels` can disagree. That is for the
planner, not this note.
