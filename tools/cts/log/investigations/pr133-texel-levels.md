<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PR #133: where the texels of 3D and 2D-array mip levels >= 1 go on the three GPUs

Written 2026-10-12, host only: nothing was built or run on a board, nothing was
posted to GitHub. It is for the people reading the PR #133 board results (plan
item `pr133-map`): it says which hardware path each GPU takes, what a
`gl_test.c` case that samples level 1 and level 2 should show on `origin/main`
(the control) and with the PR, and which older defects can colour a verdict.

## 0. What was read

| what | commit |
|---|---|
| PR #133 head (`refs/pull/133/head`) | `af9ecc535ac00d84fb3f0fa9af6db1918b8c9a2d` |
| `origin/work/int-pr49` | the same commit, `af9ecc5` |
| PR base (merge-base with `origin/main`) | `136659c206c637add0e71cf10e7801fd369f7d96` (merge of PR #100) |
| `origin/main`, fetched 2026-10-12 | `2fe4b8798b5faf2807f31dd486296f21045f4abf` (merge of PR #141) |
| the commits of the PR | `3e90710` (the change, from the ccfarm result b80ed8a82), `af9ecc5` (the test's `YakoglBackend` literal gets main's three `query_*` fields) |

Line numbers say which tree they are from: **main:** is `2fe4b87`, **PR:** is
`af9ecc5`. Backend files (`src/vivante`, `src/v3d`, `src/pvr`, `src/gl/draw.rs`)
are read at `2fe4b87`; the PR does not touch them. The workspace tip
(`db3545f`) is *older* than `2fe4b87` on some of these files (sRGB
sampling), so do not use it for the line numbers below.

The PR touches two files: `src/gl/texture.rs` (+490/-60) and the new
`tests/es3_tex_levels.rs` (686 lines). Main's `texture.rs` and `draw.rs`
differ from the PR base only by `#[cfg(not(feature = "report-es2"))]` on
tests (`git diff 136659c origin/main -- src/gl/texture.rs src/gl/draw.rs`), so
the PR's hunks do not overlap anything main changed. `git merge-tree
--write-tree origin/main pr133` (run 2026-10-12) merges them with no
conflict (result tree `b81e9847805d`).

## 1. The GPUs, in the PR's terms

`YakoglBackend::native_layered_textures` (`src/backend.rs:1475`) is the PR's
"layered" switch. `texture_alloc_storage_3d` and the draw both key on it.

| GPU | `native_layered_textures` | `mip_level_align` | how the sampler sees a 3D texture / 2D array |
|---|---|---|---|
| PowerVR (AM62P) | **false**: main:`src/pvr/backend.rs:7512` | 0 (`:7507`) | no hardware 3D or array path used. The shader rewrite (glsl2usc / the online compiler) samples a **2D image**: a 3D texture as a grid of slices, an array as one tall stack of layers. The backend never reads `type_` (`docs/es3-pvr-2darray.md` section 1) |
| GC7000 (i.MX8MP) | **true**: main:`src/vivante/backend.rs:2834` | 64 (`:2824`) | the texture unit walks the stack: descriptor TYPE 3D (an array is also TYPE 3D plus CONFIG1's TEXTURE_ARRAY bit), SIZE of one layer, SLICE the layer step, VOLUME / 3D_CONFIG the layer count |
| V3D 4.2 (Pi 4) | **true**: main:`src/v3d/backend.rs:4718` | 0 (`:4714`) | the TMU could walk a stack (a cube is one), but the backend **refuses** every 2D-array and 3D binding: main:`src/v3d/texture.rs:107`, "a 2D array or 3D texture is not sampled yet" |

So "layered GPUs" in the PR are the GC7000 and the V3D; the PowerVR is the
grid (3D) / tall-stack (array) one. Nothing here is an atlas of 2D tiles
except the PowerVR's 3D grid: `grid_cols_for(depth)` columns, as many rows as
needed (main:`src/gl/texture.rs:1064`).

## 2. The GL core, before and after (all GPUs)

What the application calls, and what the core keeps. "Draw is told" means the
`GlesTextureBind` of `src/gl/draw.rs` (main:`3707-3736`, `3808`):
`mip_levels` comes from `sampling_fields` (main:`draw.rs:602`) as
`min(levels, levels_written, max_level + 1)`, and `mip_filter` is non-zero only
when `levels > 1` and the min filter uses mipmaps.

### 2.1 3D texture, level >= 1

| call | main (before) | PR (after) |
|---|---|---|
| `glTexImage3D(GL_TEXTURE_3D, l >= 1)` | `return` at once, "one level: sampled without mipmaps" (main:`texture.rs:4075-4076`). The level is not recorded (`level_defined` unchanged), its texels are dropped, `levels` stays 1 | `tex_image_3d_level` (PR:`4251`, called from `4387`): records the level (`level_defined`), checks size and format against level `l` of the chain (3D depth halves: `level_depth_3d`, PR:`1064`), then uploads as a sub-image of the whole level if the storage holds it (`level_held`, PR:`1181`). A level of another size or format is recorded as bad (`level_bad`), not written. Stacked: also raises `levels` to `l + 1` (PR:`4297`) |
| `glTexSubImage3D(3D, l >= 1)` | the level is treated as "unheld": checked, then dropped (`recorded_unheld`, main:`1726`; `tex_sub_image_3d`, main:`4157`) | written. Grid: slice `k` at column `k % cols`, row `k / cols` of the level's grid, `level` passed on to `texture_write_rect` (it was a literal 0) (PR:`4483`..) |
| `glTexStorage3D(3D, n)` | `t.levels = 1` (main:`4920`), all `n` levels marked defined | `t.levels = n` capped at `chain_levels` when stacked, still 1 for a grid (PR:`5259`) |
| `glGenerateMipmap(3D)` | the generic path, which filtered **each slice as a 2D image** (no depth halving) | `generate_mipmap_3d` (PR:`3746`, called at `3586`): 2x2x2 box filter on the CPU, depth halved, every held level; `levels` = held levels when stacked |
| `glCopyTexSubImage3D(3D)` | refused with GL_INVALID_OPERATION for a 3D texture (it tiles slices in a grid) | accepted for stacked and grid; writes slice `k` of level `l`; marks the level written (PR:`4800`, `4918`) |

### 2.2 Storage layout of a 3D texture

* **Stacked (GC7000, V3D)**: `texture_alloc_storage_3d` returns
  `texture_alloc_storage_ex(.., false, levels > 1)` (PR:`1129-1131`); main calls
  `texture_alloc_storage`. In both, the power-of-two chain is laid out at
  once (`chain_layout_npot`, main:`1119`): level `l` is `max(w>>l,1) x
  max(h>>l,1)` with **all `depth` slices in every level** (as a 2D array's
  chain; a 3D level only uses the first `depth>>l` slices). The PR adds the NPOT case
  (`grow_npot_chain` accepts `GL_TEXTURE_3D`, PR diff hunk at `925`) and a "level
  past the stacked chain is only recorded" rule (the depth can outlast the sides:
  a 4x4x16 texture has 3 stacked levels but 5 GL levels).
  Offsets are in `GlTexture::level_off`; GC7000 packs them with the 64-byte
  alignment and one common pitch, V3D packs them tight at each level's own pitch.
* **Grid (PowerVR)**: main made one level, `cols*w x rows*h`, `chain_levels` 1.
  The PR (PR:`1129-1168`, `grid_chain_layout` `1082`) lays the levels >= 1 **after**
  level 0 in the same allocation, level `l` as its own grid of `max(depth>>l,1)`
  slices in the same column count, and sets `level_off[l]`; `width`/`height`/`chain_levels`/`levels`
  stay level 0's. The code comment says it plainly: "a draw samples level 0, as
  before" (PR:`texture_alloc_storage_3d` doc, `1117-1128`).

### 2.3 2D array, level >= 1

Unchanged in substance. On main a power-of-two array already has the whole
chain laid out (`texture_alloc_storage_ex`), `glTexImage3D(2D_ARRAY, l >= 1)`
checks size, raises `levels`/`levels_written` and uploads (main:`4093-4145`,
the `else` branches after `level == 0`); `glTexSubImage3D` and `glGenerateMipmap`
did the same. The PR's diff in that branch is a **comment** (the PowerVR's NPOT
array stays "recorded only", waiting for plan item `pvr-2darray`). What the PR
does change for arrays:

* `glCopyTexSubImage3D` now marks the level written (`note_level_written`, PR:`4918`),
  which is what lets a draw sample a level that was filled by copying (on
  main `levels_written` stays 1 after a copy and `sampling_fields` clips the
  draw to level 0).
* the shared helpers (`level_texel_offset`, `level_held`) that the new tests
  read through.

The PR title says "2D-array levels"; the host test
`every_level_of_a_2d_array_keeps_its_texels` checks the layout, and I expect
it to pass on main apart from the copy case. I did not run it: it calls
`texture::level_texel_offset`, which does not exist on main, so it cannot
compile there.

## 3. Per GPU: from `glTexImage3D` to the hardware, levels 1 and 2

### 3.1 PowerVR (AM62P)

**Draw side, common** (main:`src/gl/draw.rs:3707-3736`): not layered, so the
binding is *one 2D image*. 3D: `width = w * grid_cols`, `height = h * rows`
(`rows = ceil(depth / cols)`), `stride_texels = level_pitch(width)`,
`layers = depth`. Array: `width = w`, `height = h * layers`. `type_` is
GLES_TEXTYPE_3D / _2D_ARRAY but not read by the backend.

**Descriptor**: `build_tex_desc` (main:`src/pvr/backend.rs:4231`): image word
for a plain 2D texture (TEXTYPE_STRIDE, `stride_texels`, layer size `stride *
height * bpp` into word 4). `num_mip_levels` = `t.mip_levels` only when
`mip_filter != 0 && mip_levels > 1` (`:4255`), otherwise 1. Level `l` is
found by the TPU from level 0's address and the power-of-two chain (levels
consecutive, each a halved image of the *whole* tall/grid image). There is
no per-level base address or offset in the descriptor.

| | before the PR (main) | after the PR |
|---|---|---|
| **3D, level 1 / 2** | level texels dropped (`texture.rs:4075`); only the grid of level 0 exists, `levels` 1, `chain_levels` 1, `level_defined` has bit 0 only. A mipmapped min filter finds `level_defined & chain != chain` (main:`draw.rs:2623`) and the texture is **incomplete: it samples (0,0,0,1)**. NEAREST/LINEAR min filter: level 0, descriptor 1 level | texels of levels 1, 2 are **held** in the allocation after level 0 (`level_off[1]`, `[2]`), at that level's own grid pitch (`level_pitch(lw*cols)`); `level_defined` is complete. But `levels` and `chain_levels` stay 1, so `mip_filter` is 0 and `num_mip_levels` is 1: **the TPU is never told of them and the shader still samples level 0's grid** (a `textureLod`/bias on a 3D texture acts on the grid image's implicit level and is clamped to 0) |
| **2D array, level 1 / 2** | power-of-two w, h, and a layer count the stack height rounds to a power of two (`pot_stack`, `chain_layout_npot`): the chain is the tall image's chain; level `l` is a stack of `layers` images of `h>>l` rows; the descriptor has `num_mip_levels = levels`. Works when `layers` is a power of two (the shader's `v = (y + layer) / layers` is a fraction of the whole stack). A layer count that is not a power of two is read through rounded heights: **known to fail** (see section 5) | the same. Only `glCopyTexSubImage3D`-filled levels differ |
| NPOT array | one level; levels recorded, texels dropped (`npot_mip_chains` false for `layers > 1`, main:`texture.rs:903`) | the same, on purpose (PR:comment in `glTexImage3D`) |

The PR's consequence for the PowerVR is therefore a **completeness change,
not a pixel change**: a mipmap-filtered 3D texture that main treats as
incomplete (black) becomes a complete texture that samples level 0. No level
1 or level 2 colour can be seen through a draw on the PowerVR, before or
after. The only way to see the held texels is through `level_texel_offset` on
the host, or by reading the storage; `gl_test.c` has neither.

### 3.2 GC7000 (i.MX8MP)

**Draw side**: layered, so the binding is one layer plus the count:
`width = w`, `height = h`, `layers = depth` (main:`draw.rs:3712-3736`),
`stride_texels = level_pitch(w)` with the 64-byte-aligned chain pitch
(`mip_level_align` 64).

**Descriptor**: `build_descriptor` (main:`src/vivante/texture.rs:107`).
CONFIG0 TYPE 3D for both a 3D texture and an array (`:129`, `volume`); an array
adds CONFIG1 TEXTURE_ARRAY (`:206`); LINEAR_STRIDE `stride_texels * bpp`;
SLICE = stride * height (level 0's, one value; `:233`); VOLUME `log2(depth)`
and 3D_CONFIG depth `= layers` (`:228`, `:240`); BASELOD max = `mip_levels - 1`
(`:227`); one LOD address per level (`:270-280`), each 64-aligned, each level
advanced by `stride * level_h * bpp * layers` (**all `layers` per level**, which is what
the GL core stored). The file's own comment says where the hardware takes
a level's layer step from "is not measured".

**Tiled copy**: a layered texture with `mip_levels > 1`, or whose layer is not
tile-padded, is *not* sampled from the linear storage: `tiled_copy::wanted`
(main:`src/vivante/tiled_copy.rs:112`) builds a 4x4-tiled copy
(`tiled_layout`, `:176`) whose level `l` has `layers` layers of
`align4(w>>l) * align4(h>>l)` texels, read from the linear storage by
`linear_layout` (`:192`). The descriptor then points its LOD addresses into the
copy (`texture.rs:261-266`) and SLICE is the copy's.

| | before the PR (main) | after the PR |
|---|---|---|
| **3D, level 1 / 2** | dropped; `levels` 1, level 0 only in the descriptor. With a mipmap min filter: `levels (1) < min(chain_levels, needed)` (main:`draw.rs:2609`) so the texture is **incomplete, samples (0,0,0,1)**. With `glTexStorage3D` it is the same, `levels` is forced to 1 | levels held in the stacked chain at `level_off[l]`; `levels` = highest uploaded level + 1, `levels_written` follows; the binding's `mip_levels` is 3 or more, `mip_filter` set; the draw takes the tiled copy; the hardware walks levels 0, 1, 2. The sample at level 1 should read level 1's colour. Open point: the hardware's slice count at level `l` (`depth` or `depth >> l`) is unmeasured (see 4) |
| **2D array, level 1 / 2** | held and sampled, as the layered path always did | same. Only copy-filled levels differ |

### 3.3 V3D (Pi 4)

**Storage**: layered like the GC7000, but `mip_level_align` 0: the packed
chain, each level at its own pair-padded width (`gl_level_layered`, main:
`src/v3d/texture.rs:56`), a level holding its layers stacked
`max(h>>l,1)` rows each.

**Descriptor**: the TMU does not sample the linear storage; the backend tiles
a copy per job (`backend.rs`, `texture.rs` header). **Both 3D and 2D-array
bindings are refused before any of that** by `refusal_for`: the `match b.type_`
accepts only a 2D texture with one layer and a six-face cube
(main:`src/v3d/texture.rs:98-107`), called from `tex_state_for`
(main:`src/v3d/backend.rs:2854`). The draw is then refused and counted
(`count_refusal`, `yakogl/v3d: draw refused: <why>` once per cause).
Separately, the V3D online compiler "refuses every look-up other than 2D and
cube" (`docs/es3-shaderc-coverage.md`, "Texture look-ups"), so a program with
`sampler3D` or `sampler2DArray` may be refused at link and never reach a draw.
Which of the two stops the case I could not tell from the source alone.
(The array *layer index* write, `WADDR_TMUI`, in `src/shaderc/online/v3d/select.rs`
is for the cube's face; it is not an array path.)

| | before the PR (main) | after the PR |
|---|---|---|
| **3D, 2D array, level 1 / 2** | not sampled at all: refused. Whatever the GL core stored is irrelevant to the pixels | same. The GL core now stores the 3D levels (and `levels` > 1), but nothing reads them |

## 4. What each board should show

The case, as the plan describes it: upload a 3D texture and a 2D array with a
different colour per mip level, draw so that level 1 and level 2 are
sampled (a mipmap min filter, or `textureLod`), read the pixels back, assert
the level's colour. Notes for reading the result:

* **Use a mipmap min filter and give the texture a complete chain, or set
  `GL_TEXTURE_MAX_LEVEL`.** The draw's completeness rule needs every level up
  to `min(GL's last level of w x h, TEXTURE_MAX_LEVEL + 1)`
  (`mip_levels_needed`, main:`draw.rs:2580`); a hole is (0,0,0,1) on every
  GPU, with or without the PR (this is not the PR's bug). The chain is counted
  from the sides, not the depth.
* **Fill every slice of every level.** A stacked 3D level holds `depth`
  slice rows but only `depth >> l` are GL's; the rest stay cleared (zero).
  If the GC7000 hardware walks `depth` slices at level `l`, an `r`
  coordinate in the upper half reads zero. Probe `r` in the first slice's
  range first, then a second `r` as its own check, so a failure of the second
  reads as the "unmeasured slice count" and not as the PR.
* **Use a power-of-two layer count for the array** (2, 4, 8): the PowerVR's tall stack
  is exact only then.

| GPU | control (main, no PR) | with the PR |
|---|---|---|
| PowerVR 3D | black (0,0,0,1) if the levels come from `glTexImage3D(l >= 1)` with a mipmap filter (incomplete). Level-0 colour if the texture is made by `glTexStorage3D` with enough levels set to a NEAREST/LINEAR filter. Never level 1's or level 2's colour | level 0's colour, the texture complete. **A case that asserts level 1's colour fails on the PowerVR before and after the PR**: the case has to assert level 0's colour there (a documented limit, PR:`texture.rs` doc of `texture_alloc_storage_3d`), or skip the 3D draw check for that GPU |
| PowerVR 2D array (POT, `layers` 2^n) | level 1 / 2 colour (no difference from the PR), unless the levels are filled by `glCopyTexSubImage3D` (then level 0's colour) | same colours; copy-filled levels now show |
| GC7000 3D | black (0,0,0,1): `levels` 1 against a chain, incomplete. This is the control that should flip | level 1 / 2 colours at `r` in the first slice. If only the upper-`r` probe fails with black, that is the slice-step question, not the PR |
| GC7000 2D array | level colours (no difference), copy-filled aside | same |
| V3D 3D and array | the draw is refused (`draw refused: a 2D array or 3D texture is not sampled yet`) or the program is refused at link; the read-back is the clear colour; the case reports FAIL or whatever it does for a refused draw | identical to the control. **Not measurable on the Pi 4**: record it as "not applicable, refused by the backend" and look at the log line, do not read it as a PR failure |

So only the GC7000 shows a control failure that the PR is meant to fix in a
draw. The PowerVR shows black to level-0-colour (completeness) for 3D, and
the V3D shows nothing. The control "fails on main" is therefore a per-GPU
statement, and the host mock the case item uses must be the layered
mock (`native_layered_textures: true`, `src/testutil.rs:1390`) for the 3D
control. The two other mocks (`:1355`, `:1426`) are not layered.

The existing cases show how these paths behave today: `gl_test` 42 `es3array`
(an 8x8x3 array, level 0 only) and 48 `es3vol` (a 4x4x5 volume, level 0 only,
`texture`, `textureLod`, `texelFetch`) sample both targets without mipmaps
and are the nearest regression for the descriptor fields in section 3.

The default case lists differ per image: `gc_gl_test` runs the whole numbered
set; **`v3d_gl_test` defaults to 12 cases** (`5,58,59,66,67,102,3,9,11,12,13,4`,
`driver/samples/zephyr/v3d_gl_test/Kconfig`, `V3D_GL_TEST_CASES`, on main), which
include neither 42 nor 48 nor any new one. The Pi 4 board session has to name the new
case in that list, and "the whole `gl_test` set" there means those 12 plus the new
case. `gl_test` on the AM62P takes `GL_TEST_CASES`
(`driver/samples/zephyr/gl_test/Kconfig`).

## 5. Known defects that can confuse a verdict

From `docs/HANDOFF.md`, `tools/cts/RESULTS.md` and `docs/es3-cts-failures.md`
at `2fe4b87`. None is caused by the PR; each can look like it.

1. **AM62P, gl_test 48 `es3vol`** fails in the table image: "a blue channel
   off by 10 in a 3D-texture fetch", pre-existing since before the table
   (RESULTS.md around lines 4126 and 16442-16447 on main; the workspace tip's
   HANDOFF has a row for `60ffeec8`). The regression set there is 39 of 40 with 48 red, 0 resets.
   Treat a red 48 on the AM62P as the baseline, not a PR effect: compare with
   the baseline record (`am62p-pr133-baseline.md`, another item).
2. **AM62P, mipmapped 3D lookups hung the GPU** in the whole-tree dEQP run on
   `f2a13877` (RESULTS.md, "AM62P: the whole dEQP-GLES3 tree", images 20 and
   21: `texture.filtering.3d.formats.r11f_g11f_b10f_linear_mipmap_linear` and
   `texture.filtering.3d.sizes.3x7x5_linear_mipmap_linear`; recovery only by
   the RTI reset, verdict `no_reset`). Those are other formats and NPOT sizes
   and a different tree, but a `LINEAR_MIPMAP_*` 3D draw is the same kind. A reset
   during a PR run needs its own entry (the "recovery counts"), and
   `back_after` and `resets` must be read before calling a red.
3. **AM62P, 2D-array mip sampling is wrong in general**: the tall-image model
   lets the TPU filter and wrap across layers, and a mip level of the stacked
   image has rounded heights (`docs/es3-pvr-2darray.md` section 1;
   `docs/es3-cts-failures.md` ~269: `texture.filtering.2d_array` 210 of 210,
   shadow 2d_array). An array colour red on the PowerVR at level >= 1, especially
   with 3 layers or with LINEAR, is this. Plan item `pvr-2darray` (parked) is
   the fix; the PR does not touch it.
4. **A bias on a 3D texture** was a compile failure for four
   `texture.mipmap.3d` cases on the PowerVR (`es3-cts-failures.md` ~371); the
   workspace tip's HANDOFF says it now selects, but I did not find that line in
   main's HANDOFF. Use `textureLod`, not a bias, in the case.
5. **GC7000, `PE_COLOR_FORMAT` / RGBA16F** (gl_test 44 `es3hdr`, check 3,
   RESULTS.md "i.MX8MP: gl_test case 44 on `f2a13877` still fails") is a red in
   the whole-set run that is unrelated. So are the intermittent case 2 (#25),
   case 10 (#9) and case 16 (MSAA skip counted as failure, #8): the three
   2026-10-10 runs were 45, 46 and 46 of 48. A 46 / 48 on the i.MX8MP is
   the baseline; compare per-case. The i.MX8MP has **no console**: the
   verdict is the SNVS `CASE` record (four words), so a case's own pixel
   message does not reach the log; keep the case's failure in a word the record
   carries or run it alone (`CONFIG_GL_TEST_CASES`).
6. **V3D non-seamless cube LOD** (`docs/v3d-noseamless-cube.md`; HANDOFF row
   54; fix in `1cf946a9`; Pi 4 gl_test 12 of 12 and 934 of 934 cube rows pass
   there). It is about cube look-ups, which the V3D emulates as an
   array-style look-up of the selected face. It does not touch a 3D or
   array binding, which is refused. If a case also draws a cube on the Pi 4 and
   that goes red, read the cube note first.
7. **Capture loss and reachability**: the Pi 4 serial path drops bursts
   ("Lost in capture" on the AM62P, and the Pi 4 earlier); a missing verdict
   line is a capture problem, not a failed case. No UART path from the `nuc`
   to the Pi 4 on 2026-10-10 (RESULTS.md, "Pi 4: nothing run on 2026-10-10"):
   if the Pi 4 is unreachable the outcome is "nothing ran".
8. **The PR commit's message and trailer**: `af9ecc5`'s message carries a
   session trailer line. It is the PR's own history; do not copy it into
   records or comments.
9. **The `YakoglBackend` literal in `tests/es3_tex_levels.rs`**: main added
   `query_begin`, `query_end`, `query_result` (the PR's second commit already
   fills them); a later field added to the struct breaks the test file at
   compile time, which on a `check.sh` row looks like a PR failure.

## 6. What this means for the case and the boards

* The case cannot be one number with one expected colour for all three GPUs.
  Per GPU: GC7000 asserts level 1 / 2 colours (3D control: black), PowerVR
  asserts level-0 colour for 3D (control: black if the chain is uploaded
  level by level) and level colours for a POT array, V3D prints "not
  sampled" and passes without a check (or is left out of the V3D list).
* A host control that "fails on main" has to use the layered mock for 3D, and
  the grid mock for the PowerVR black-to-level-0 flip; the arrays need
  `glCopyTexSubImage3D` to differ at all.
* Board reading order for a red: (1) is it the baseline red for that board
  (items 1, 5 above); (2) was there a reset or a lost capture (2, 7);
  (3) is it the array stack on the PowerVR (3); (4) only then the PR.
