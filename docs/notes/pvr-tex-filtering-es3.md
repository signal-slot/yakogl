<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: the 244 AM62P `texture.filtering` fails (T-FILTER), split into sub-causes

Plan #400, item design-tfilter. Cause T-FILTER of
`docs/es3/am62p-gles3-notpass.tsv` is the 244 `Fail` lines that
`tools/cts/am62p-notpass-causes.py` matches with
`^texture\.filtering\.(?:2d|cube|2d_array|3d)\.`. This note splits them by
target, filter, wrap mode, format and size. For each sub-cause it says
whether origin/main fixes it, and by which commit. For each sub-cause that
is still open it gives a fix design with a host test. Docs only: no code
was changed and no board was run for this note.

- **Tree.** Code references are at origin/main `2340445f` (2026-11-28).
  Every file cited here is byte-identical there and at the integration tip
  `14e7a742` (`git diff 14e7a742 2340445f -- src/gl src/pvr src/shaderc/mod.rs
  src/shaderc/online/powervr driver/src/pvr` touches only
  `src/shaderc/online/mali/nir_fs.rs`).
- **The lines.** Every T-FILTER line is from the `ad2b5ed` build. The TSV
  header says rows 22-45 are "build ad2b5ed, run 2026-10-13/14". Row 21, the
  `2d_array` and `3d.combinations` row, is in the row range 11-21 of the
  header, but no T-FILTER line comes from `b25997e4`. That row's lines in the
  TSV are still the `ad2b5ed` ones (see "Caveats"). `ad2b5ed` is older than
  this repository's root commit (`381b0f09`, 2026-10-27), so it carries
  neither the native 2D array (`02c5c9af`, 2026-11-27) nor native 3D
  (`e179ad48`, 2026-11-05; `e90122b8`, 2026-11-17).
- **dEQP.** VK-GL-CTS `93bca01` (the `DEQP_GLES3_TEST_DIR` pin of
  `tools/cts/deps.toml`), `modules/gles3/functional/es3fTextureFilteringTests.cpp`:
  `Texture2DArrayFilteringCase` `:746-1037`, `Texture3DFilteringCase`
  `:1041-1299`, the groups `:1616-1798`.

Listing:

```sh
python3 -I tools/cts/am62p-notpass-causes.py --list docs/es3/am62p-gles3-notpass.tsv
grep -P '^Fail\tdEQP-GLES3\.functional\.texture\.filtering\.(2d|cube|2d_array|3d)\.' docs/es3/am62p-gles3-notpass.tsv
```

## 1. Where the 244 are

| group | mustpass | Fail (T-FILTER) | Pass | NotSupported | NoResult |
|---|---:|---:|---:|---:|---:|
| `2d.*` | 129 | **0** | 117 | 12 | 0 |
| `cube.*` | 124 | **0** | 112 | 12 | 0 |
| `2d_array.formats` | 72 | 60 | 0 | 12 (srgb_r8, srgb_rg8) | 0 |
| `2d_array.sizes` | 26 | 26 | 0 | 0 | 0 |
| `2d_array.combinations` | 36 | 29 | 7 | 0 | 0 |
| `3d.formats` | 71 | 54 | 0 | 10 (srgb_r8, srgb_rg8) | 7 (`*_linear_mipmap_linear`) |
| `3d.sizes` | 26 | 22 | 0 | 0 | 4 (`*_linear_mipmap_linear`) |
| `3d.combinations` | 56 | 53 | 3 | 0 | 0 |
| **T-FILTER** | | **244** | | | |

The 46 NotSupported lines are NS-SRGB-R8RG8: 12 each in `2d`, `cube` and
`2d_array`, and 10 in `3d`, where the 2 `3d.formats.srgb_*_linear_mipmap_linear`
cases have no result instead. The 11 NoResult lines are NR-3D-LML (#1285,
`docs/notes/pvr-3d-mipmap-hang.md`). Neither belongs to T-FILTER.

So **T-FILTER has no 2D and no cube line**: 115 are `2d_array`, 129 are
`3d`. The cube design of K3 (`docs/notes/pvr-npot-cube-levels.md`) moves
none of the 244.

What the dEQP cases draw (es3fTextureFilteringTests.cpp):

| group | texture | wrap | filters |
|---|---|---|---|
| `2d_array.formats` | 128x128x8, the 14 filterable formats (`:1622-1646`) | REPEAT, REPEAT | 6 min filters; mag LINEAR for the mipmapped ones, else min = mag |
| `2d_array.sizes` | RGBA8, 4x8x8, 32x64x16, 128x32x64, 3x7x5, 63x63x63 (`:1648-1671`) | REPEAT, REPEAT | as above (mustpass: all 6 filters for the first four sizes; `linear` and `nearest` for 63x63x63) |
| `2d_array.combinations` | RGBA8 123x107x7 (`:1673-1703`) | every S x T of clamp/repeat/mirror | min x mag; the mustpass keeps 36 |
| `3d.formats` | 64x64x64, the same formats (`:1711-1735`) | REPEAT on S, T and R | as `2d_array.formats` |
| `3d.sizes` | RGBA8, the same five sizes (`:1737-1761`) | REPEAT on S, T and R | as `2d_array.sizes` |
| `3d.combinations` | RGBA8 63x57x67 (`:1763-1797`) | every S x T x R | the mustpass keeps `linear_linear_*` and `nearest_nearest_*` (27 each), `linear_nearest_clamp_clamp_clamp` and `nearest_linear_clamp_clamp_clamp` |

Every texture has its whole mip chain uploaded (`numLevels = log2(max side)
+ 1`, `:838`, `:1131`). The coordinates are well outside [0, 1]: the
offsets go down to -3.7, and the scale is `2^lod * viewport / size`
(`:899-909`, `:962-976`, `:1165-1168`, `:1221-1237`). So every REPEAT or
MIRRORED_REPEAT axis is exercised over many periods. Each array case draws
3 quads, plus a fourth one, the layer-rounding quad (layer 1.50001 to
1.49999, `:908`). Each 3D case draws 4 quads.

## 2. The two sampling paths of the ad2b5ed build, and their faults

### 2.1 2D array: the stacked image (`pvr_native_array` 0 at ad2b5ed)

The array was one tall 2D image, `w x (h * layers)`, a STRIDE descriptor.
The layer was picked in the shader, and `v` was clamped into the layer
(`array_uv`, `src/shaderc/online/powervr/select.rs:4210-4268`:
`v = (clamp(c.y, 0.5/h, 1 - 0.5/h) + l) / layers`). Faults:

- **AW (wrap T lost).** The clamp of `v` into the layer comes before the
  TPU, so REPEAT and MIRRORED_REPEAT on T act as CLAMP_TO_EDGE. Wrap S
  survives, because `u` is not clamped and the TPU's `addrmode_u` applies.
  The board shows this exactly. The 7 `2d_array.combinations` passes are
  all clamp on T without a mipmap filter (`linear_linear_{clamp,mirror,repeat}_clamp`,
  `nearest_nearest_{clamp,mirror,repeat}_clamp`, `nearest_linear_clamp_clamp`). Every non-mipmapped
  fail has T = repeat or mirror, apart from the one AX case below.
- **AL (levels of a stack).** A level of the stacked image is a stack of
  smaller layers whose heights the TPU rounds (`docs/es3/pvr-2darray.md`
  section 1; `chain_layout`/`pot_stack` in `src/gl/texture.rs`). The TPU's
  level `l` does not hold layer `k`'s level `l` where the shader looks for
  it. For an NPOT array the GL core holds level 0 alone anyway (K4, below).
  Every mipmapped case fails, `clamp_clamp` included.
- **AX (one case, not traced).** `2d_array.combinations.linear_nearest_clamp_clamp`
  (minification LINEAR, magnification NEAREST, clamp on both axes) fails,
  while `nearest_linear_clamp_clamp` passes. On the stacked path its sampler
  words are those of a 2D texture: one level, `min_linear != linear_filter`,
  so `maxlod` is 64 (`tex_desc_sampler_lod`, `src/pvr/backend.rs:7237-7262`).
  That rule makes `2d.combinations` and `3d.combinations`
  `linear_nearest_*` pass on the same build. The most likely reason left is
  the stacked image itself, LINEAR taps of a minified footprint at the
  layer's clamped edge. It was not traced further, because the native array
  removes the stacked image (2.3).

### 2.2 3D: the grid of slices (`pvr_native_3d` 0 at ad2b5ed and at the tip)

The volume is level 0's slices tiled in a 2D grid, one STRIDE image. The
shader picks the two slices around `c.z * layers - 0.5`, samples each and
mixes them (the grid arm of `select.rs:1661-1798`, `vol_uv` `:4434`;
descriptor `addrmode_w` hard-wired to CLAMP_TO_EDGE, `src/pvr/backend.rs:7278-7282`).
Faults (also `docs/notes/pvr-3d-mipmap-hang.md` section 3):

- **GW (wrap lost on every axis).** `vol_uv` clamps `(u, v)` into the slice
  and the slice index into `0..layers-1`, so REPEAT and MIRRORED_REPEAT
  become CLAMP_TO_EDGE on S, T and R.
- **GN (NEAREST mixes two slices).** For a float texture the grid arm
  always samples two slices and blends them by the fraction of
  `c.z * layers - 0.5` (`select.rs:1759-1797`: `taps`, then
  `ffma(t, b - a, a)`). Only integer textures take the nearest slice
  (`integer`, `:1700-1704`). So a NEAREST filter is LINEAR across slices.
  `3d.combinations.nearest_nearest_clamp_clamp_clamp`, the only fail with
  no wrap fault, fails for this alone, while the three LINEAR
  `*_clamp_clamp_clamp` cases pass.
- **GL (level 0 only).** The descriptor has one level (`mipfilter` 0,
  `maxlod` 0), and an explicit level samples level 0 (`:1713-1719`). No
  mipmap filter applies.

### 2.3 What the tip changes

- **Native 2D arrays are the default**: `PVR_NATIVE_ARRAY_DEFAULT = 1`,
  `src/shaderc/mod.rs:314`, since `02c5c9af` "pvr: native 2d-array textures
  by default". A `sampler2DArray` look-up is one 2D `smp` over the layer
  that `tao` addresses (`select.rs:1386-1460`; `array_layer_addr`
  `:4270-4301` picks the layer as `clamp(floor(r + 0.5), 0, d)`, as ES 3.0
  3.8.8 says). The draw binds the twiddled layer-major copy
  (`GLES_TEXTYPE_2D_ARRAY_TWIDDLED`; descriptor arm `src/pvr/backend.rs:7083`).
  Each layer is a separate image with its own chain. The TPU wraps S and T
  by `addrmode_u`/`addrmode_v` (`tpu_addrmode`, `:1578-1584`: FLIP is GL's
  mirrored repeat), filters inside the layer and picks the level. This
  removes AW, AX and AL for every array whose chain the GL core holds.
- **The GL core holds the chain of a POT array only.** `npot_mip_chains`
  (`src/gl/texture.rs:1308-1310`) is false for any array of more than one
  layer on the PowerVR. An NPOT array keeps level 0 alone, and the native
  copy then holds one level (K4, `docs/notes/pvr-npot-array-levels.md`
  sections 1-2). So for an NPOT array with a mipmap filter, AL is still
  open at the tip.
- **Native 3D exists, but is off**: `PVR_NATIVE_3D_DEFAULT = 0`,
  `src/shaderc/mod.rs:342`. `e179ad48` (TEXTYPE_3D twiddled copy, NPOT level
  twiddle, `addrmode_w` from WRAP_R) and `e90122b8` (BASE_LEVEL, MAX_LEVEL,
  MIN_LOD, MAX_LOD) are on main. But a stock build still samples the grid
  (`select.rs:6584` `stacked_3d`). Only an image built with
  `CONFIG_DEQP_PVR_NATIVE_3D=1` (`driver/samples/zephyr/deqp/Kconfig:138`;
  `DEQP_PVR_NATIVE_3D=1` for `tools/cts/am62p-deqp-images.sh`) or a host run
  with `YAKOGL_PVR_NATIVE_3D=1` takes the native arm (`select.rs:1507-1615`:
  one `smp` with `dmn` 3d, `SampleZ { vol: true }`, no slice mix). So
  **GW, GN and GL are all still open on the default arm at the tip**. The
  native arm removes all three.
- K3 (`docs/notes/pvr-npot-cube-levels.md`) changes nothing here: T-FILTER
  has no cube line.

## 3. Sub-cause table (244)

"Faults" are the section 2 faults the ad2b5ed build had for those cases.
POT and NPOT are per dEQP texture size (for an array, width and height; for
3D, all three sides).

| id | cases | target, size | filter | wrap | faults at ad2b5ed | state at the tip (`2340445f`) |
|---|---:|---|---|---|---|---|
| TF-A1 | 78 | 2d_array POT: `formats` 128x128x8 (all 10 formats, 6 filters: 60), `sizes` 4x8x8, 32x64x16, 128x32x64 (6 filters each: 18) | all 6 | REPEAT, REPEAT | AW, and AL for the 52 mipmapped | **fixed at `02c5c9af`** (native array; the POT chain is held) |
| TF-A2 | 17 | 2d_array NPOT, no mipmap filter: `sizes` 3x7x5 and 63x63x63 `{linear,nearest}` (4), `combinations` 123x107x7 (13) | `linear`/`nearest` min and mag | 12 combos with T repeat/mirror, the 4 sizes REPEAT, 1 `linear_nearest_clamp_clamp` | AW (16), AX (1) | **fixed at `02c5c9af`** (native array; one level needed) |
| TF-A3 | 20 | 2d_array NPOT with a mipmap filter: `sizes.3x7x5_*mipmap*` (4), `combinations.*_mipmap_*` 123x107x7 (16) | the 4 mipmap filters | sizes REPEAT x2; combos: 8 clamp x2, 4 with S mirror, 4 with T mirror | AL; AW for the 8 whose T wraps | **needs src/gl (named): K4**, `packed_npot_array` and its draw rule (`docs/notes/pvr-npot-array-levels.md` sections 3-4), on top of `02c5c9af` |
| TF-V1 | 26 | 3d POT, no mipmap filter: `formats` 64^3 `{linear,nearest}` x 10 formats (20), `sizes` 4x8x8, 32x64x16, 128x32x64 `{linear,nearest}` (6) | `linear`/`nearest` | REPEAT x3 | GW; GN for the 13 `nearest` | **open**: native 3D is not the default (fix design 4.1) |
| TF-V2 | 43 | 3d POT with a mipmap filter: `formats` 64^3 `{linear_mipmap_nearest, nearest_mipmap_linear, nearest_mipmap_nearest}` x 10 (30) and the 4 `linear_mipmap_linear` that failed rather than hung (r11f_g11f_b10f, rgb10_a2, rgb565, rgb5_a1); `sizes` 4x8x8, 32x64x16, 128x32x64 x the same 3 filters (9) | mipmap filters | REPEAT x3 | GW, GL; GN for the `nearest_mipmap_*` | **open**: native 3D is not the default (4.1) |
| TF-V3 | 56 | 3d NPOT, no mipmap filter, some axis REPEAT or MIRRORED_REPEAT: `combinations` 63x57x67 (52: 26 `linear_linear_*`, 26 `nearest_nearest_*`), `sizes` 3x7x5 and 63x63x63 `{linear,nearest}` (4) | `linear`/`nearest` | every S x T x R but clamp x3 (combos), REPEAT x3 (sizes) | GW; GN for the 28 `nearest` | **open**: native 3D is not the default (4.1); NPOT wrap on a twiddled volume not yet seen on a board (4.3) |
| TF-V4 | 1 | `3d.combinations.nearest_nearest_clamp_clamp_clamp` (63x57x67) | NEAREST | clamp x3 | GN only | **open**: native 3D is not the default (4.1) |
| TF-V5 | 3 | 3d NPOT with a mipmap filter: `sizes.3x7x5_{linear_mipmap_nearest, nearest_mipmap_linear, nearest_mipmap_nearest}` | mipmap filters | REPEAT x3 | GW, GL; GN for 2 | **open**: native 3D is not the default (4.1); NPOT level twiddle not yet seen on a board (4.3) |
| **total** | **244** | 115 2d_array + 129 3d | | | | 95 fixed at `02c5c9af`; 20 need the K4 src/gl change; 129 need native 3D on |

Check: 78 + 17 + 20 = 115; 26 + 43 + 56 + 1 + 3 = 129; 115 + 129 = 244.

By format: the 10 formats of the `formats` groups (rgba16f, r11f_g11f_b10f,
rgb9_e5, rgba8, rgba8_snorm, rgb565, rgba4, rgb5_a1, srgb8_alpha8, rgb10_a2)
fail the same way, 6 of 6 cases each on `2d_array`, and 5 or 6 on `3d` (the
sixth is NoResult or not in the mustpass). The same formats pass all of
`2d.formats` and `cube.formats` on the same build. So **no T-FILTER line is
format-bound**: the texel format words are right, and the fault is the
target's addressing. `sizes` and `combinations` are RGBA8 only.

## 4. Fix designs for the open sub-causes

### 4.1 TF-V1 to TF-V5 (129): native 3D as the default

**Change.** `src/shaderc/mod.rs:342`: `PVR_NATIVE_3D_DEFAULT = 1`. Nothing
in `src/gl/**` changes: the native copy and its view (`src/gl/native_vol.rs`)
and the draw's bind (`src/gl/draw.rs:5217`) are on main and behind the
switch. Nothing in `src/pvr/**` changes either:

- the `GLES_TEXTYPE_3D_TWIDDLED` descriptor arm, `src/pvr/backend.rs:7131`;
- `addrmode_w` from WRAP_R, `:7278-7282`;
- the LOD range, `tex_desc_sampler_lod` `:7237-7262`;
- the shader arms, `select.rs:1507-1615`.

Each fault goes away on that path:

- GW: the TPU wraps all three axes by `addrmode_u/v/w` (`tpu_addrmode`).
- GN: one `smp` with `dmn` 3d, so the TPU's own NEAREST picks one texel in
  `w` as well; there is no shader mix.
- GL: the copy holds the defined chain (`native_vol::sampled_levels`,
  `view`) and the descriptor walks it (`mipfilter` for LINEAR between
  levels, `maxlod` over the chain).

What moves with the flip, in the same commit:

- `src/shaderc/precompiled.rs:161-169` `switches_at_boot` compares against
  the constant, so the precompiled table is made at the new default, and
  `tests/fixtures/precompile_counts.txt` moves for any program with a
  `sampler3D`.
- `tests/pvr_native3d_sampler_words.rs:361-363`
  (`the_default_is_the_grid_of_slices_without_wrap_r_or_a_level_range`)
  asserts `PVR_NATIVE_3D_DEFAULT == 0`. It becomes the test of the grid arm
  under `pvr_set_native_3d(0)`.
- The other tests that name `PVR_NATIVE_3D_DEFAULT` only restore the
  default after a run (`es3_pvr_3d.rs`, `es3_pvr_3d_fix.rs`,
  `es3_pvr_texgrad.rs`, `es3_pvr_texsize.rs`, `es3_pvr_texoffset.rs`,
  `es3_pvr_texfn.rs`, `pvr_texfn_compile.rs`, `pvr_texfetch_fixed.rs`,
  `pvr_native_switch_cache.rs`, `pvr_int_targets_deqp.rs`,
  `pvr_rg_int_targets.rs`, `es3_pvr_fragdepth.rs`). Each still has to be
  rerun, because a default-arm assertion in it may now see the native arm.
  The full list of pins that move is item n3d-flip-impact's.
- `driver/samples/zephyr/deqp/Kconfig:138-150`: the help text of
  `DEQP_PVR_NATIVE_3D` says -1 is "off until a board run says otherwise";
  after the flip -1 means on.

**Gate before the flip: the board.** board-n3d (plan #400: rows 22-40 and
`texture.mipmap.3d` with `DEQP_PVR_NATIVE_3D=1`) is the A/B. Expected:
TF-V1, TF-V2 and TF-V4 Pass (POT, or clamp-only). For TF-V3 and TF-V5 see
4.3. Rows 22-38 hold the 3D `formats` and `sizes` (TF-V1, TF-V2, TF-V5 and
the 4 `sizes` of TF-V3). The 53 `3d.combinations` cases (TF-V3's 52 and
TF-V4) are in row 21 of `tools/cts/am62p-gles3-tree.txt`, next to
`2d.*` and `2d_array.*`. The A/B covers them only if that row's
`3d.combinations` part is added to it. On a native-3D-on board run, `texture.mipmap.3d` affine, basic and bias
were already right before `e90122b8` (`docs/notes/pvr-3d-native.md`, "Base
level, ..."). That is board evidence that the TPU walks the POT twiddled
chain as `native_vol` lays it out. If the A/B fails a sub-cause, the default
stays 0, and that sub-cause gets its own item. `pvr_set_native_3d(0)`
returns to the grid unchanged.

**Host test to add: `tests/pvr_filtering_3d_cases.rs`.** Feature set
`host-test,gpu-powervr,shaderc-online`, not under `report-es2`:

```sh
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_filtering_3d_cases
```

It follows `tests/pvr_native3d_sampler_words.rs`: the stand-in back end
records each draw's bind, and `yakogl::pvr::backend::texture_descriptor`
packs it. The OSAL renders nothing, so the test checks the words, the
record and the copy, not pictures. Each test builds one dEQP case's GL state
as the case does: the whole chain uploaded by `glTexImage3D` per level, the
filters and wraps by `glTexParameteri`, one quad with dEQP's
`sampler3D` fragment program. It runs **with the switch as it boots**, so
it fails while the default is 0 and pins the flip:

1. `filtering_3d_formats_shape`: 64^3, each of the 10 formats x 6 filters,
   REPEAT x3. Assert:
   - the record has `native_cube & 4` (the native arm, no slice mix);
   - the bind is `GLES_TEXTYPE_3D_TWIDDLED`;
   - IMAGE_WORD0 textype is 3D (2), with the texformat of the same format's
     2D descriptor;
   - IMAGE_WORD1 depth is 63, and `num_mip_levels` is 7 with a mipmap
     filter, else 1;
   - `mipfilter` is set exactly for `*_mipmap_linear`;
   - `maxlod` is `6 * 64` with a mipmap filter;
   - `addrmode_u/v/w` are REPEAT, and the min and mag filters are the
     case's.
2. `filtering_3d_sizes_shape`: the five sizes x 6 filters, RGBA8. The same
   assertions, with the API size in word 0 (`w - 1`, `h - 1`) and in the
   depth field; the levels are `log2(max side) + 1`.
3. `filtering_3d_combinations_shape`: 63x57x67, the 56 mustpass names. The
   wrap triple reaches `addrmode_u/v/w` (CLAMP_TO_EDGE / REPEAT / FLIP).
   `nearest_nearest_*` has both filters POINT (TF-V4's fault: on the grid
   arm the same draw compiles to two slice samples and a mix, which is what
   the test's control asserts with `pvr_set_native_3d(0)`).
4. `npot_volume_copy_places_each_level`: 3x7x5 and 63x57x67 with their
   chains. Every texel of every level lands at
   `level offset + usclib_twiddle3d index` over the level's API extent, in
   the power-of-two physical extent (`native_vol::twiddle3d`,
   `physical_extent`, `level_extent`). The copy is read back through the
   descriptor's `texaddr` and level count, as K4's TPU model does for
   arrays. This pins TF-V3 and TF-V5's layout on the host. Whether the TPU
   reads it that way is 4.3.

### 4.2 TF-A3 (20): K4, the src/gl change that keeps an NPOT array's chain

**Needs src/gl (named).** The change is K4 of
`docs/notes/pvr-npot-array-levels.md` (sections 3-5, the patch at
`472630df`): `packed_npot_array(target, w, h, layers)` in
`src/gl/texture.rs`; `stacked_decoded_array` widened to every uncompressed
NPOT array on a back end without native layers and with
`mip_level_align() == 0`; `texture_alloc_storage_ex` and `glGenerateMipmap`
growing the packed chain; and the draw rule in `src/gl/draw.rs`
`build_common` that gives a stacked-array program `(mip_filter, mip_levels)
= (0, 1)`. Nothing in `src/pvr/**` or `src/shaderc/online/powervr/**`
changes. The native copy (`native_array::copy`) and the
`GLES_TEXTYPE_2D_ARRAY_TWIDDLED` descriptor (`src/pvr/backend.rs:7083`)
already describe any number of held levels. That note's section 8 does not
count these 20 cases ("`texture.filtering.2d_array.*` ... not attributed to
this cause here"); this note attributes them.

Why K4 is the whole fix for these 20: with the native array (the default),
AW and AX are gone (TF-A1 and TF-A2 are the same draws with POT sizes or
without a mipmap filter). The only thing left is that the bind has
`mip_levels` 1 for an NPOT array, and the TPU then reads level 0 at every
LOD. dEQP uploads all levels (`:854-896`). 123x107 has 7 levels and 3x7
has 3, and K4 holds them.

**Host test.** `tests/pvr_npot_array_chain.rs` of K4 (its section 6). Add
the two dEQP filtering shapes to its `npot_array_every_level_lands_where_the_descriptor_points`
and `npot_array_each_level_samples_its_colour` sizes: 123x107x7 (7 levels)
and 3x7x5 (3 levels), each uploaded by `glTexImage3D` per level as
`glu::Texture2DArray::upload` does. Without the src/gl part both fail
(level 1 reads level 0's value). With it, each LOD reads its own level on
every layer.

### 4.3 What only a board can say (TF-V3, TF-V5; also TF-A2, TF-A3)

- **NPOT wrap on a twiddled image.** The native copies pad each axis to a
  power of two and give the descriptor the API size. That is Mesa's layout
  (`pvr_image.c:83-92`). Vulkan requires REPEAT and MIRRORED_REPEAT on an
  image of any size, and Mesa's pvr gives such an image the same API-size
  words (`docs/notes/pvr-3d-native.md`, "The layout"). So the TPU is
  expected to wrap at the API extent, not at the padded one. Mesa's sampler
  code was not re-read for this note. This holds for the native array as
  much as for 3D: TF-A2 has NPOT arrays with REPEAT. No board has run a
  native NPOT texture with REPEAT yet. The `2d.sizes` 31x55/127x99 passes
  are STRIDE images, so they do not answer it. Row 21 of board-b (at
  `472630df`, native arrays on: `2d_array.*`) answers it for the array.
  board-n3d (`3d.combinations` is in row 21 too, so board-n3d needs that
  row's 3D part as well as rows 22-40) answers it for 3D.
- **NPOT level twiddle.** `docs/notes/pvr-3d-native.md` "What is not
  verified", item 1: whether the TPU twiddles an NPOT volume's level by the
  physical or the halved API extent. `3d.sizes.3x7x5_*mipmap*` (TF-V5) is
  the case that tells. If it is the other one, the fix is in
  `native_vol::twiddle_extent` (src/gl) and is named by that board result.
- **Cost.** A 64^3 RGBA8 copy is 262,144 texels per level-0 write. A dEQP
  case uploads its two textures (gradient and grid) once, then draws 4
  quads, so each texture is copied once per case. It is not expected to approach the 600 s watchdog.

## 5. Expected T-FILTER at the tip, by arm

| arm | TF-A1 | TF-A2 | TF-A3 | TF-V1..V5 | T-FILTER Fail |
|---|---|---|---|---|---:|
| `ad2b5ed` (the TSV) | Fail | Fail | Fail | Fail | 244 |
| tip, default (`472630df`, `2340445f`): native array on, native 3D off | Pass | Pass (4.3) | Fail (K4) | Fail | 149 |
| tip + `DEQP_PVR_NATIVE_3D=1` (board-n3d) | Pass | Pass (4.3) | Fail (K4) | Pass (V3, V5: 4.3) | 20 |
| + K4 applied (src/gl) | Pass | Pass | Pass | Pass | 0 |

The 149 is 20 + 129. These are expectations from the code, not
measurements. The NR-3D-LML cases (11) are outside T-FILTER: they were
`linear_mipmap_linear` cases checked slowly against a wrong grid picture.
On the native 3D arm they are TF-V2-like draws, and
`docs/notes/pvr-3d-mipmap-hang.md` owns their outcome.

## 6. Caveats

- The TSV's row-21 lines (`2d_array.*`, `3d.combinations.*`) are the
  `ad2b5ed` ones, although the header puts row 21 in the `b25997e4` range.
  Either row 21 gave the same names at `b25997e4` (it carries neither
  `02c5c9af` nor native 3D on) or the splice kept the old lines. That is
  for docs-1285 / board-b to settle. The sub-cause split does not depend on
  it.
- AX (1 case) is not traced on the stacked path. It is counted in TF-A2 as
  fixed at `02c5c9af` because the native array's words for it are the 2D
  ones that pass `2d`/`3d` `linear_nearest_*`. The board run decides.
- "Fixed at `02c5c9af`" (95 cases) has not yet been seen on a board. The
  host pins are `tests/pvr_array_native_cases.rs` (the middle layer at
  levels 0, 1 and 1.5, native and stacked), `tests/es3_pvr_2darray.rs` (the
  64x32x3 7-level array) and the descriptor test
  `an_array_descriptor_is_a_twiddled_textype_2d_with_the_layer_count`
  (`src/pvr/backend.rs:12429`). None of these pins `addrmode_v` (wrap T) on
  a twiddled array. A one-line assertion with `wrap_t:
  GLES_WRAP_MIRRORED_REPEAT` there would pin AW's fix (see the report; it
  is not part of this note).
