<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: the dEQP-GLES3 "GPU hang" on 3D `linear_mipmap_linear` (images 20 and 21)

Plan #144, item `pvr-3d-hang`. Investigation only: no file under `src/` or
`driver/` changed, and nothing ran on a board. Integration tip `cd738b1a`,
2026-10-14.

The defect as #1285 states it: on
`f2a13877` (2026-10-11) image 20 (`texture.filtering.3d.formats.*`) and
image 21 (`texture.filtering.3d.sizes.*`) of `tools/cts/am62p-gles3-tree.txt`
each gave 13 results, began a 14th case and printed nothing more. The 14th
cases are `3d.formats.r11f_g11f_b10f_linear_mipmap_linear` and
`3d.sizes.3x7x5_linear_mipmap_linear`. The runner's verdict was `no_reset`
at 602 s and 612 s. The board came back only through the RTI reset.

## Summary

The evidence says this is **probably not a GPU hang**. The most likely cause
is dEQP itself: it spends minutes on the A53 checking a wrong 3D
`LINEAR_MIPMAP_LINEAR` picture. The image then reaches its 600 s watchdog
bound before the case ends.

Measured on the host (section 5): with the PowerVR's picture, every 3D
`linear_mipmap_linear` case takes 8.8 to 18.8 s of dEQP checking, and
every other 3D case takes under 0.07 s. The board took 337 s for a case
that takes 18.8 s here, so the factor is about 18. At that factor, the two
`linear_mipmap_linear` cases before case 14 take about 550 s (image 20)
and 590 s (image 21) on the board. Case 14 then begins at about the 600 s
bound and cannot finish. That is exactly what the board recorded.

1. The PowerVR back end gets **bit-identical texture and sampler words** for
   case 14 (`linear_mipmap_linear`, "hung") and case 13 (`linear`, finished).
   This holds in both images (host repro below). On the PowerVR a 3D texture
   is a single-level 2D "slice grid", so the GL min filter never reaches the
   TPU. The shader is the same program too. Nothing the GPU receives can tell
   case 14 from case 13.
2. The board has already shown this cost: on `c4ccc966` a 3D
   `linear_mipmap_linear` case took **337 s** and then finished, and the next
   one ran 141 s until the watchdog (`tools/cts/README.md` 2e, old image
   19). No one called that a hang.
3. dEQP checks each pixel of a 3D `LINEAR_MIPMAP_LINEAR` result that differs
   from its reference. When the pixel's colour is inside the colour range of
   the candidate texels, the check is a six-deep search over two levels
   (`tcuTexLookupVerifier.cpp` `is3DTrilinearFilterResultValid`, inside six
   texel loops). The PowerVR samples level 0 only and clamps the coordinates
   instead of wrapping them. That makes a plausible but wrong picture, so
   almost every pixel takes the full search. A black picture (the host build)
   fails at the first bounds test and takes milliseconds. A correct picture
   (plain Mesa) passes in milliseconds.
4. A real GPU hang leaves a line on the console within about 20 s: the
   firmware recovery (`render: HWR during fence ...`, counted by
   `tools/board.sh` `pvr_resets`) or the back end's fence timeout
   (`pvr_backend: kick N did not complete`, `WAIT_MS` = 20000). Image 11 of
   the same tree printed its 4 recoveries and ended `done_with_resets`. For
   images 20 and 21, `tools/cts/RESULTS.md` records no result after the 14th
   case began and no recovery. (Their consoles were not read for this note;
   step 1 of the recipe reads them.)
5. "The 14th case in both images" follows from the case order, not from a
   counter. This CTS sorts a group's children by name (`tcuTestCase.cpp:105`).
   Each prefix has 6 filter cases in the order `linear`,
   `linear_mipmap_linear`, `linear_mipmap_nearest`, `nearest`,
   `nearest_mipmap_linear`, `nearest_mipmap_nearest`. So case 14 is always the
   `linear_mipmap_linear` case of the third prefix.
6. The `no_reset` verdict does not separate the two causes. The image feeds
   the RTI for 600 s after `rti_arm: armed`
   (`CONFIG_YAKOGL_AM62P_WDT_SECONDS=600`), and the RTI resets the SoC up to
   60 s after that. A capture that ends at 602 s or 612 s can end before
   that reset, whether the GPU or the CPU is busy.

The verification recipe (section 7) confirms or rejects this on the board:
a check of the existing console timestamps, then three single-case images.

## 1. The case sequence of each image

dEQP runs `--deqp-case=<pattern>` in hierarchy order, and this CTS keeps
every group's children sorted by name (`framework/common/tcuTestCase.cpp:105`:
"Sorted insert by child name"). The board filter of image 20 is
`texture.filtering.3d.formats.*`, which matches the whole dEQP tree, not only
the mustpass list (`am62p-gles3-tree.txt` header). So the order is:

| # | image 20 (`3d.formats`, 64x64x64, 7 levels) | image 21 (`3d.sizes`, RGBA8) |
|---:|---|---|
| 1-6 | `bgra8_*` (GL_BGRA8_EXT, not in the mustpass) | `128x32x64_*` (8 levels) |
| 7-12 | `bgra_*` (GL_BGRA, not in the mustpass) | `32x64x16_*` (7 levels) |
| 13 | `r11f_g11f_b10f_linear` | `3x7x5_linear` |
| **14** | **`r11f_g11f_b10f_linear_mipmap_linear`** | **`3x7x5_linear_mipmap_linear`** (3 levels) |

Within each prefix the six cases are `linear`, `linear_mipmap_linear`,
`linear_mipmap_nearest`, `nearest`, `nearest_mipmap_linear`,
`nearest_mipmap_nearest`. (`bgra8` sorts before `bgra` because `8` comes
before `_`; the host run of section 5 printed them in that order.) Before
case 14, each image ran two other
`linear_mipmap_linear` cases (#2 and #8). The BGRA cases are not skipped:
the library advertises `GL_EXT_texture_format_BGRA8888`, and the host repro
shows their 3D uploads succeed and draw.

Each case (`es3fTextureFilteringTests.cpp` `Texture3DFilteringCase`) creates
two textures (a gradient and a grid), uploads every level with
`glTexImage3D`, and draws 4 quads in a 64x64 random viewport. Each draw is
followed by `glReadPixels` and the CPU check `verifyTextureResult`. If the
high-precision check fails, it runs again at low precision (`lodBits` 4,
`uvwBits` 4). The wrap is REPEAT on all three axes. The magnification filter
is LINEAR for every mipmap case.

## 2. Host repro: what the back end is handed

### How

A scratch integration test (not committed, source in the appendix) runs the
GL core of this tip with a stand-in back end. The back-end flags are the
PowerVR's: no native layers, packed levels. It replays both sequences
above, cases 1 to 14, with the cases' own shaders (`glsTextureTestUtil.cpp`,
GLSL ES 3.00, `PROGRAM_3D_FLOAT`), uploads, texture parameters and draws. It
records `GlesDraw.textures[0]` and the fragment uniforms of every draw.

```text
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online \
    --test zz_pvr3d_scratch -- --nocapture
```

Result: 0 GL errors and 4 draws per case in all 28 cases, including the BGRA
ones.

### What each case hands over (every draw of a case is the same)

`fmt` 11 is `GLES_TEXFMT_R11G11B10F` and 0 is RGBA8. `type` 3 is
`GLES_TEXTYPE_3D`. `fsu[12]` is the hidden `pvr_layers_u_sampler` uniform
(64.0, 16.0, 5.0).

| case | grid w x h | stride | bytes | linear / min_linear | mip_filter | mip_levels | wrap s/t | pvr_layers |
|---|---|---:|---:|---|---:|---:|---|---:|
| img20 #1-#12 (bgra, bgra8) | 512 x 512 | 512 | 1048576 | per filter | 0 | 1 | REPEAT | 64 |
| img20 #13 r11f `linear` | 512 x 512 | 512 | 1048576 | 1 / 1 | 0 | 1 | REPEAT | 64 |
| img20 #14 r11f `linear_mipmap_linear` | 512 x 512 | 512 | 1048576 | 1 / 1 | **0** | **1** | REPEAT | 64 |
| img21 #1-#6 128x32x64 | 1024 x 256 | 1024 | 1048576 | per filter | 0 | 1 | REPEAT | 64 |
| img21 #7-#12 32x64x16 | 128 x 256 | 128 | 131072 | per filter | 0 | 1 | REPEAT | 16 |
| img21 #13 3x7x5 `linear` | 9 x 14 | 10 | 560 | 1 / 1 | 0 | 1 | REPEAT | 5 |
| img21 #14 3x7x5 `linear_mipmap_linear` | 9 x 14 | 10 | 560 | 1 / 1 | **0** | **1** | REPEAT | 5 |

Every draw in both images has `incomplete` 0, `ntex` 1 and the same
uniform block apart from `pvr_layers`.

Why `mip_filter` is 0 and `mip_levels` is 1: the GL core keeps a 3D texture
as a grid of its slices, and tells the back end that the grid has one level
(`src/gl/texture.rs` `texture_alloc_storage_3d`: "the PowerVR samples level 0
until its shader back end learns 3D mip levels"). Levels 1 and up are held
after the grid but `t.levels` stays 1 (`texture.rs` ~4512, "a grid's levels
above 0 are not the backend's"). So `sampling_fields` (`src/gl/draw.rs:600`)
gives `mip_filter` 0 for every min filter. The only bind field the min
filter can change is `min_linear`, and it is 1 for both `LINEAR` and
`LINEAR_MIPMAP_LINEAR`.

The shader is the same source in every case. The 3D path of the online
compiler (`src/shaderc/online/powervr/select.rs:1012`, `vol_uv` :2774)
samples the grid twice, at slices `floor(z)` and `floor(z) + 1`, and mixes
the two. It clamps `s,t` into the slice (`clamp(uv, 0.5/sz, 1 - 0.5/sz)`)
and clamps the slice index `z` to `0..layers-1`. The look-up has an implicit
LOD and the filter state does not change it (`lodkey.rs` remaps explicit
levels only).

### The words: `build_tex_desc` on those binds

A scratch unit test in the `src/pvr/backend.rs` test module (removed again,
source in the appendix) ran the real `build_tex_desc` on the recorded
binds. The texture address is a stand-in: `HEAP_GENERAL_BASE + 0x100000`.

```text
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --lib zz_scratch_pvr3d_words -- --nocapture
```

```text
img20 #13 r11f linear   (as recorded)
  dwords 0..15: [c0000a84, 01ff07fe, 000001ff, 10200004, 00100000, 00000000, 00000000, 0000000c, 00000fff, 02000050, 0, 0, 0, 0, 0, 0]
  IMAGE_WORD0 01ff07fec0000a84: textype 4 width-1 511 height-1 511 texformat 0x58 swiz 0 1 2 4 gamma 0 minlod 0 smpcnt 0
  STRIDE_IMAGE_WORD1 10200004000001ff: stride-1 511 mipmaps_present 0 texaddr 0x8000100000 alpha_msb 0 num_mip_levels 1
  SAMPLER_WORD0 0200005000000fff: dadjust 0xfff minlod 0 maxlod 0 aniso 0 mag 1 min 1 mip 0 addr u 0 v 0 w 2 non_norm 0 cmp 0
  SAMPLER_WORD1 0000000000000000  image meta: layer size 1048576 pck c  sampler meta0 0
img20 #14 r11f lml      (as recorded)
  dwords 0..15: [c0000a84, 01ff07fe, 000001ff, 10200004, 00100000, 00000000, 00000000, 0000000c, 00000fff, 02000050, 0, 0, 0, 0, 0, 0]
  (every field as #13)
img21 #13 3x7x5 linear  (as recorded)
  dwords 0..15: [60000a64, 000d0020, 00000009, 18200004, 00000230, 00000000, 00000000, 00000140, 00000fff, 02000050, 0, 0, 0, 0, 0, 0]
  IMAGE_WORD0 000d002060000a64: textype 4 width-1 8 height-1 13 texformat 0xc swiz 0 1 2 3 gamma 0 minlod 0 smpcnt 0
  STRIDE_IMAGE_WORD1 1820000400000009: stride-1 9 mipmaps_present 0 texaddr 0x8000100000 alpha_msb 1 num_mip_levels 1
  SAMPLER_WORD0 0200005000000fff: dadjust 0xfff minlod 0 maxlod 0 aniso 0 mag 1 min 1 mip 0 addr u 0 v 0 w 2 non_norm 0 cmp 0
  SAMPLER_WORD1 0000000000000000  image meta: layer size 560 pck 140  sampler meta0 0
img21 #14 3x7x5 lml     (as recorded)
  dwords 0..15: [60000a64, 000d0020, 00000009, 18200004, 00000230, 00000000, 00000000, 00000140, 00000fff, 02000050, 0, 0, 0, 0, 0, 0]
  (every field as #13)
img21 #2 128x32x64 lml  (as recorded; this case finished on the board)
  dwords 0..15: [60000a64, 00ff0ffc, 000003ff, 18200004, 00100000, 00000000, 00000000, 00000140, 00000fff, 02000050, 0, 0, 0, 0, 0, 0]
  IMAGE_WORD0: textype 4 width-1 1023 height-1 255 texformat 0xc; STRIDE_IMAGE_WORD1: stride-1 1023 num_mip_levels 1; SAMPLER_WORD0 as above
```

Decoded with `third_party/mesa-26.2.1/.../csbgen/rogue/texstate.xml`:
textype 4 = STRIDE; texformat 0x58 = F10F11F11, 0xc = U8U8U8U8; swiz 4 =
SRC_ONE; FILTER 1 = LINEAR; ADDRMODE 0 = REPEAT, 2 = CLAMP_TO_EDGE; dadjust
0xfff = ZERO_UINT (no bias); CLAMP is u4.6 (64 per level).

For comparison, the words the back end **would** write if the GL core passed
the chain (`mip_filter` 2, `mip_levels` 3 or 7) to the same function:
`STRIDE_IMAGE_WORD1` gets `num_mip_levels` 3 or 7 and `mipmaps_present` 1,
and `SAMPLER_WORD0` gets `maxlod` 128 or 384 and `mipfilter` 1. No board run
has used those words for a 3D texture: the grid has no power-of-two chain
the TPU could walk.

**Finding:** in both images, case 14 and case 13 hand the GPU the same 16
descriptor dwords and the same program. Case 13 finished on the board with
these exact words. The words of case 14 cannot be what hangs, unless
something outside them changes between the cases.

## 3. Field by field with Mesa's pvr driver

Mesa: the farm's cached clone (`mesa`, `VERSION` 26.3.0-devel),
`src/imagination/vulkan/pvr_arch_tex_state.c` `pvr_arch_pack_tex_state`
(114-324), `pvr_image.c` (`pvr_image_init_memlayout` 49,
`pvr_image_plane_setup_mip_levels` 143-199), `pvr_arch_image.c` (image view,
104-117), `pvr_arch_sampler.c` (84-200). The repository pins 26.2.1; its
`texstate.xml` (vendored) is the one used for decoding. The device info is
`common/device_info/bxs-4-64.h` (36.53.104.796): it has
`tpu_image_state_v2` and `tpu_extended_integer_lookup`, and its quirks
(brn44079, 70165, 72168, 72463, 74056) do not include 51025, so Mesa does not
override the min/mag filters on this GPU.

The Mesa column is what Mesa would pack for the same GL state reached
through Vulkan: a `VK_IMAGE_TYPE_3D` optimal-tiled image of the same format
with the full chain, a 3D view of all levels, and a sampler with
min/mag LINEAR, `mipmapMode` LINEAR, REPEAT on U/V/W, `minLod` 0 and
`maxLod` at least the chain.

| field | yakogl, case 13 and 14 (r11f 64^3 / RGBA8 3x7x5) | Mesa pvr, same texture | same? |
|---|---|---|---|
| memory layout | linear grid of slices, `cols = ceil(sqrt(d))` (8x8 / 3x2), level 0 only visible | `PVR_MEMLAYOUT_3DTWIDDLED` (`pvr_image.c:59`), physical extent next power of two on each axis (`:83-92`), levels at 4-aligned extents (`:150-166`) | no |
| IMAGE_WORD0.textype | 4 STRIDE | 2 3D (`pvr_arch_tex_state.c:170-176`) | no |
| IMAGE_WORD0.width-1 / height-1 | grid: 511 / 511; 8 / 13 | extent: 63 / 63; 2 / 6 (`:230-233`) | no |
| depth | none (the STRIDE word has no depth field) | IMAGE_WORD1.depth = d-1: 63; 4 (`:277-282`) | no |
| IMAGE_WORD1 form | STRIDE_IMAGE_WORD1 | IMAGE_WORD1 (twiddled) (`:271-316`) | no |
| stride | 512-1; 10-1 (grid pitch, pair-padded) | none (twiddled) | n/a |
| num_mip_levels | 1 | view level count: 7; 3 (`pvr_arch_image.c:109`) | no |
| mipmaps_present | 0 | 1 (image has more than one level, `:116`) | no |
| baselevel | none (STRIDE word) | 0 | n/a |
| texformat | 0x58 F10F11F11; 0xc U8U8U8U8 | same formats (`pvr_formats.c`) | yes |
| swizzle | R G B ONE; R G B A | same channels | yes |
| alpha_msb | 0 (r11f); 1 (RGBA8) | `vk_format_is_alpha_on_msb` per format; the same per-format rule the 2D path uses (not 3D-specific, not re-checked here) | as 2D |
| gamma, smpcnt | 0, 0 | 0, 0 | yes |
| image meta LAYER_SIZE | 1048576; 560 (grid level 0) | `layer_size`: the whole chain, padded to 1x1x1 (`pvr_image.c:174-187`) | no |
| image meta PCK_INFO | 0xc; 0x140 | `setup_pck_info(format)` | as 2D |
| SAMPLER_WORD0.magfilter / minfilter | LINEAR / LINEAR | LINEAR / LINEAR (`pvr_arch_sampler.c:131-132`) | yes |
| mipfilter | 0 | 1 (`mipmapMode` LINEAR, `:134-135`) | **no** |
| minlod | 0 | `minLod` clamped, 0 | yes |
| maxlod | 0 | `maxLod` clamped to CLAMP_MAX (959) | **no** |
| dadjust | ZERO_UINT | ZERO_UINT + `mipLodBias` (0) | yes |
| addrmode_u / _v | REPEAT / REPEAT | REPEAT / REPEAT | yes (but see below) |
| addrmode_w | CLAMP_TO_EDGE (never used: the third axis is the shader's) | REPEAT | no |
| anisoctl, non_normalized, cmp_mode | 0 | 0 | yes |
| SAMPLER_WORD1 | 0 | 0 (no ycbcr, seamless) | yes |

What the table means:

- yakogl does not use the TPU's 3D texture type at all. Every field in which
  it differs from Mesa follows from that one design: a 3D texture is a
  single-level 2D image, and the shader does the third axis. These words
  are valid STRIDE words. Their kind, a single-level linear 2D image with
  LINEAR filters, is the same as every 2D texture the AM62P samples without
  trouble, and the same as case 13's.
- The fields that differ make the **picture** wrong, not the GPU stuck:
  - level 0 only: `mipfilter` 0, `num_mip_levels` 1, `maxlod` 0;
  - REPEAT on `s,t` is lost, because `vol_uv` clamps the coordinate into the
    slice before the TPU sees it, so the TPU's REPEAT never applies;
  - REPEAT on `r` is lost, because the slice index is clamped to
    `0..layers-1`.

  dEQP's 3D filtering cases use REPEAT and texture coordinates far outside
  `[0, 1]` (`Texture3DFilteringCase::iterate`: offsets down to -3.7 and
  scales of `2^lod * 64 / size`). So almost every pixel is wrong.
- A layout question that does not apply here: Mesa lays out a 3D image's
  chain from the next-power-of-two extent (3x7x5 is laid out as 4x8x8). The
  yakogl grid never hands the TPU more than level 0, so no TPU walk can leave
  the allocation. The same holds for both cases.

## 4. State carried from earlier cases

Every case creates its own two textures and deletes them in `deinit`. It
makes its own `TextureRenderer` (program) and frees it again. So no GL
object survives into case 14. What differs before case 14:

- the number of kicks: 4 draws per case, each followed by a read-back
  (so about one kick each). Before case 14 there were 13 x 4 = 52 draws in
  **both** images, the same count. But image 19 (`3d.combinations`
  `linear_linear_*`, `linear_nearest_*` and `nearest_*`, which includes
  `nearest_mipmap_*`; 63x57x67 RGBA8, also 4 draws per case) ran hundreds of
  cases past that count and ended `done`. So the count alone does not hang
  the GPU;
- the time used: cases 1 to 13 include two `linear_mipmap_linear` cases
  (#2 and #8) and four `*_mipmap_linear` or `*_mipmap_nearest` ones per
  prefix. Each of them is checked by the slow path when its picture is
  wrong. This is the state that matters under H1;
- the textures' GPU memory: about 2.4 MiB per case (two grids and their
  held levels), given back at `deinit`. Nothing grows with the case count
  in the GL core's view (the repro allocates and frees in the same pattern).

## 5. Why dEQP's check takes minutes here

`verifyTextureResult` (`glsTextureTestUtil.cpp`) renders the reference, then
checks every pixel that differs with `tcu::isLookupResultValid`. For a 3D
`LINEAR_MIPMAP_LINEAR` look-up this goes to
`isLinearMipmapLinearSampleResultValid` (3D form,
`tcuTexLookupVerifier.cpp:1172`). That function loops over every candidate
texel cube of level 0 (`k0,j0,i0`) and of level 1 (`k1,j1,i1`). For each
pair it calls `is3DTrilinearFilterResultValid` (`:461`). That function first
tests the colour bounds (`isInColorBounds`). If they pass, it searches six
nested float loops (`x0,y0,z0,x1,y1,z1`) at `searchStep` = `2^-uvwBits`
across the coordinate error bounds. When the high-precision check
(`uvwBits` 7) fails, the case runs it again at low precision (`uvwBits` 4).

- A pixel from the right level passes quickly.
- A black pixel fails at the colour-bounds test, so a host run with the
  PowerVR library (read-back is all black) takes 0.02 to 0.04 s a case
  (`host-deqp-run.sh` below).
- A level-0 or clamped colour from the same gradient or grid usually lies
  **inside** the colour bounds. So it takes the whole six-deep search for
  every texel pair, twice. This is the PowerVR's picture.

`linear_mipmap_nearest` and `nearest_mipmap_linear` search three float axes
of one level, or none, so they are much cheaper. `linear_mipmap_linear` is
the only filter with the six-deep search. On the board this cost was
measured once: 337 s for one `3d.combinations.linear_mipmap_linear_*` case
(`tools/cts/README.md` 2e). The A53 cores run dEQP single-threaded.

Measured here (host, PowerVR library, black read-back, so the fast path):

```text
tools/cts/host-deqp-run.sh --gpu=powervr --module=gles3 --cts=<VK-GL-CTS with the yakogl target> \
    --chunk=100000 --jobs=1 --out=build/host-deqp/pvr3d-sizes texture.filtering.3d.sizes
```

| case (mustpass, sorted) | duration |
|---|---:|
| 128x32x64_linear / _linear_mipmap_linear | 0.030 s / 0.041 s |
| 32x64x16_linear / _linear_mipmap_linear | 0.017 s / 0.030 s |
| 3x7x5_linear / **_linear_mipmap_linear** | 0.017 s / **0.027 s** |
| all 26 cases | 0.011 to 0.041 s each, all Fail (black) |

So the host build reproduces the sequence and does **not** hang. It cannot
reproduce the slow path, because its picture is black.

### The PowerVR's picture on the host (measured)

Section 7 step 3 (shim in the appendix) runs the host dEQP-GLES3 on the
host's Mesa (llvmpipe, surfaceless pbuffer, no window). Before every draw
it makes the bound 3D texture sample as the PowerVR does: level 0 only
(`GL_TEXTURE_MAX_LEVEL` 0), every wrap `CLAMP_TO_EDGE`, and the min filter
without its mipmap part. The shim reported every draw forced (`120 draws, 120
with a 3D texture forced to level 0` for image 21). With `PVR3D_SHIM=0` the
same run is plain Mesa. Filters as the board's (`--deqp-case=<pattern>`).

Image 21 (`3d.sizes.*`, 30 cases):

| case | PowerVR picture | plain Mesa |
|---|---:|---:|
| 128x32x64_linear / **_linear_mipmap_linear** / other 4 | 0.062 / **17.554** / 0.042-0.054 s, all Fail | 0.033-0.050 s, all Pass |
| 32x64x16_linear / **_linear_mipmap_linear** / other 4 | 0.025 / **15.479** / 0.016-0.027 s, all Fail | 0.009-0.014 s, all Pass |
| 3x7x5_linear / **_linear_mipmap_linear** / other 4 | 0.023 / **14.755** / 0.010-0.022 s, all Fail | 0.004-0.009 s, all Pass |
| 4x8x8_linear / **_linear_mipmap_linear** / other 4 | 0.020 / **16.243** / 0.011-0.022 s, all Fail | 0.005-0.010 s, all Pass |
| 63x63x63_linear / **_linear_mipmap_linear** / other 4 | 0.058 / **14.991** / 0.030-0.049 s, all Fail | 0.032-0.039 s, all Pass |

Image 20, its first 14 cases in board order (filter
`3d.formats.bgra*`, `r11f_g11f_b10f_linear`, `r11f_g11f_b10f_linear_mipmap_linear`):

| case | PowerVR picture |
|---|---:|
| bgra8_linear, _linear_mipmap_nearest, _nearest, _nearest_mipmap_linear, _nearest_mipmap_nearest | 0.038-0.063 s, Fail |
| **bgra8_linear_mipmap_linear** (#2) | **15.085 s**, Fail |
| bgra_linear, _linear_mipmap_nearest, _nearest, _nearest_mipmap_linear, _nearest_mipmap_nearest | 0.052-0.068 s, Fail |
| **bgra_linear_mipmap_linear** (#8) | **15.458 s**, Fail |
| r11f_g11f_b10f_linear (#13) | 0.058 s, Fail |
| **r11f_g11f_b10f_linear_mipmap_linear** (#14) | **8.760 s**, Fail |

Calibration against the board: `3d.combinations.linear_mipmap_linear_linear_clamp_clamp_clamp`,
which took 337 s on the AM62P (`c4ccc966`), takes **18.774 s** here
(`linear_linear_clamp_clamp_clamp`: 0.052 s, Pass). Factor about 18. The
host is an x86-64 desktop core and the board an A53 core. The two pictures
are not bit-identical (Mesa's level-0 filtering against the PowerVR's
two-slice emulation), so the factor is an estimate.

Board time this predicts (host time x 18, rounded):

| image | before case 14 | case 14 itself |
|---|---:|---:|
| 20 | #2 270 s + #8 280 s + 11 cases about 12 s = **about 560 s** | about 160 s |
| 21 | #2 315 s + #8 280 s + 11 cases about 7 s = **about 600 s** | about 265 s |

Both images reach case 14 at about the 600 s bound and neither can finish
it. This matches the board's record (13 results, the 14th begun, no result)
without any GPU hang. Only `linear_mipmap_linear` is slow; the other mipmap
filters cost no more than `linear`.

## 6. Hypotheses, ranked

**H1 (most likely). No GPU hang: dEQP's CPU check of a wrong 3D
`linear_mipmap_linear` picture outlasts the 600 s bound.**
The image keeps working, alone on one A53 core, until the RTI feeder stops
at 600 s and the RTI resets the SoC.
For:
- the board measured 337 s for one such case, and on the host with the
  PowerVR's picture every 3D `linear_mipmap_linear` case takes 9 to 19 s
  while every other case takes under 0.07 s; scaled to the board, both
  images reach case 14 at about 600 s (section 5);
- the GPU words of case 14 are those of case 13, which finished;
- no recovery was recorded for either image, while a GPU hang produces an
  HWR or fence-timeout line within about 20 s (to be confirmed from the
  consoles, step 1);
- "the 14th case" is a consequence of the sorted order, and each image had
  already run two other `linear_mipmap_linear` cases (#2, #8), which use up
  the time;
- old image 19 showed the same picture and was read as the watchdog, not a
  hang;
- `no_reset` at 602/612 s is what this cause produces when the capture ends
  before the RTI's reset, at up to 660 s after arming plus the boot.

Against, or not known: no per-case timestamps of the `f2a13877` console were
read for this note (the farm does not know where the records are). The board
times above are host times scaled by one calibration case, not board
measurements.

**H2. A GPU hang caused by state that builds up over the image** (the kick
count, frame-set reuse, a firmware ring, freelist growth).
For: both images reach case 14 after the same number of draws (52).
Against: image 19 ran far more kicks of the same kind (3D grid, 4 draws per
case) and ended `done`. A hang would print an HWR or fence-timeout line,
and none was recorded.
Nothing here depends on the min filter, so the "always
`linear_mipmap_linear`" pattern would be chance.

**H3. A GPU hang caused by the words of case 14 themselves** (texture or
sampler state, a TPU walk out of bounds).
Against: the words are bit-identical to case 13's, and case 13 finished
(section 2). The TPU is given one level, so it cannot walk past the grid
(section 3).

**H4. A CPU-side stall in the driver** (a wait loop with no timeout, the
compiler on a new shader).
Against: the program is the same source as case 13's. The online compiler
is deterministic, and its cache is per program. The fence wait has a 20 s
bound (`WAIT_MS`) and logs when it hits it. This has no evidence; it is listed
so that the board check below can rule it out.

What the hypotheses do not change: every 3D case with a mipmap filter, and
most other 3D cases, fail on the PowerVR because of level 0 only and the
lost REPEAT (section 3). That is the defect to fix. With a correct picture
the check is fast, and H1 disappears with it.

## 7. Verification recipe

Environment names only. `tools/board.sh`'s header and `tools/cts/README.md`
(2e and "AM62P") have the variables. `SW` is a detached checkout of the
commit under test.

### Step 1: no board needed, read the `f2a13877` consoles

On the machine that holds `records/am62p/deqp-gles3-f2a13877/`, take the
`console.txt` of `full/20261011-0232-img20-*` and `full2/20261011-0251-img21-*`.
Lines have the form `HH:MM:SS <line>`. List the time of each
`Test case '...'` line, the gap to the next one, and every line after the
14th:

```sh
grep -a "Test case '\|render: HWR\|did not complete\|rti_arm:\|U-Boot SPL" <console.txt>
```

Expected:

| | H1 | H2 / H3 / H4 |
|---|---|---|
| gaps of #2 and #8 (`*_linear_mipmap_linear`) | long: about 270 to 315 s each (section 5) | short, like their `linear` siblings (about 1 s or less) |
| time of the 14th `Test case` line after `rti_arm: armed` | late: about 560 s (image 20), about 600 s (image 21) | early: it begins well before 600 s, then silence |
| after the 14th line | nothing until `rti_arm: feeding bound of 600 s over after N feeds; the RTI resets the SoC` (if the capture reached it) | H2/H3: `render: HWR during fence ...` or `pvr_backend: kick N did not complete (...)` about 20 s after the 14th line; H4: silence |

If the gaps show H1, step 2 only measures the time. If they show "early
start, then silence", go to step 2 to tell H2/H3 from H4.

### Step 2: board, three one-case images

Filter file (`pvr3d-filters.txt`, three rows, labels as in
`am62p-deqp-images.sh`):

```text
# image P1:
dEQP-GLES3.functional.texture.filtering.3d.sizes.3x7x5_linear_mipmap_linear
# image P2:
dEQP-GLES3.functional.texture.filtering.3d.formats.r11f_g11f_b10f_linear_mipmap_linear
# image P3:
dEQP-GLES3.functional.texture.filtering.3d.sizes.3x7x5_linear
```

Build and run (the capture must outlast the RTI's reset, so `RUN_BOUND` is
900):

```sh
export ZEPHYR_WS=... WEST_VENV=... ZEPHYR_SDK_INSTALL_DIR=... DEQP_WORK=...
DEQP_MODULE=gles3 $SW/tools/cts/am62p-deqp-images.sh pvr3d-filters.txt <img> pvr3d > <img>/build.out
BOARD_HOST=... BOARD_UART=... BOARD_BOOT=uboot-mmc BOARD_MUSTPASS=$SW/tools/cts/gles3-mustpass.txt \
RUN_BOUND=900 RETURN_BOUND=240 TREE_IMAGE_DIR=<img> TREE_TAG=pvr3d \
    $SW/tools/board.sh am62p tree pvr3d-filters.txt <rec>/pvr3d
$SW/tools/board.sh am62p tally <rec>/pvr3d
grep -a "Test case '\|render: HWR\|did not complete\|rti_arm:\|deqp: summary" <rec>/pvr3d/*/console.txt
```

Expected:

| image | H1 | H2 (cumulative) | H3 (the case's words) | H4 (driver stall) |
|---|---|---|---|---|
| P3 (`3x7x5_linear`, control) | result in seconds, `done` | `done` | `done` | `done` |
| P1 (`3x7x5_linear_mipmap_linear` alone) | `Test case` line, then a silence of about 265 s (the check; host 14.8 s x 18), then `Fail (Image verification failed)`, `deqp: summary`, verdict `done`, resets 0, no HWR line, no `did not complete` | result in seconds, `done` (the case alone does not build up the state) | no result; `render: HWR during fence ...` or `pvr_backend: kick N did not complete` within about 20 s of the `Test case` line | no result, no HWR or timeout line, ever |
| P2 (r11f `linear_mipmap_linear` alone) | as P1, silence about 160 s (host 8.8 s x 18) | as P1 under H2 | as P1 under H3 | as P1 under H4 |

H1 and H4 both start with silence. They differ in the end: H1 ends with a
dEQP result after a few minutes, H4 never does.

### Step 3: host, the PowerVR's picture on Mesa (run, section 5)

This measures the cost of the check without a board. With
`EGL_KHR_get_all_proc_addresses` (Mesa has it), dEQP takes every GL function
from `eglGetProcAddress` (`framework/egl/egluGLContextFactory.cpp:425`), and
the yakogl target `dlopen`s `libEGL.so.1` by soname. So a shim built as
`libEGL.so.1` and put first on `LD_LIBRARY_PATH` sees every GL look-up. It
forwards every EGL entry point to the system libEGL (an ifunc per symbol)
and returns two wrappers from `eglGetProcAddress`: `glDrawElements` and
`glDrawArrays`. Before each draw the wrapper makes the bound 3D texture
sample as the PowerVR does: `GL_TEXTURE_MAX_LEVEL` 0, every wrap
`CLAMP_TO_EDGE`, and the min filter without its mipmap part. A first try
with a `libGLESv2.so.2` shim had no effect for that reason. Source in the
appendix (`egl_shim.c`, `run.sh`). The dEQP binary is the host build of
`tools/cts/host-deqp-run.sh --gpu=powervr --module=gles3`.

```sh
bash run.sh img21-pvr  'dEQP-GLES3.functional.texture.filtering.3d.sizes.*' 1   # PowerVR's picture
bash run.sh img21-mesa 'dEQP-GLES3.functional.texture.filtering.3d.sizes.*' 0   # plain Mesa, control
bash run.sh img20-pvr  'dEQP-GLES3.functional.texture.filtering.3d.formats.bgra*,dEQP-GLES3.functional.texture.filtering.3d.formats.r11f_g11f_b10f_linear,dEQP-GLES3.functional.texture.filtering.3d.formats.r11f_g11f_b10f_linear_mipmap_linear' 1
bash run.sh calib 'dEQP-GLES3.functional.texture.filtering.3d.combinations.linear_mipmap_linear_linear_clamp_clamp_clamp,dEQP-GLES3.functional.texture.filtering.3d.combinations.linear_linear_clamp_clamp_clamp' 1
```

Result (section 5 has the tables): with `1`, every `*_linear_mipmap_linear`
case takes 8.8 to 18.8 s and fails, every other case under 0.07 s; with `0`,
all 30 cases of image 21 pass in under 0.05 s.

## 8. What follows, for other items (not done here)

- **The cause to fix** (no plan item yet): 3D textures on the PowerVR sample
  level 0 only, and lose REPEAT on all three axes. Two ways: (a) the TPU's
  own 3D type, laid out as Mesa does (`3DTWIDDLED`, `TEXTYPE_3D`,
  IMAGE_WORD1 `depth`, the full chain, `addrmode_w` from `GL_TEXTURE_WRAP_R`),
  with the sample instruction given three coordinates; or (b) keep the grid
  and teach `vol_uv` the wrap modes (`fract` for REPEAT, mirror for
  MIRRORED_REPEAT) and the levels (a grid per level, the LOD computed in the
  shader, two levels mixed). (a) is Mesa's path and removes the shader
  emulation. Either fixes the picture, and with it the slow check.
- **The tree** (`tools/cts/gen-am62p-gles3-tree.py`, done: `ISOLATE`): image
  20 ran 12 BGRA cases and `rgba16f_linear_mipmap_linear` that are not in
  the mustpass, because its filter was `3d.formats.*`. The generator now
  names only the mustpass prefixes of `3d.formats` and `3d.sizes`, as it
  does for `3d.combinations`, and gives each of their 15
  `*_linear_mipmap_linear` cases an image of its own (46 images with the
  `builtin_functions.precision` split). The rest
  of the old images 20 and 21 (83 cases) can report while the cause is
  open. Board check: the 15 single-case images and the two prefix images
  must end inside the lifetime with 0 resets (`RUN_BOUND=900 RETURN_BOUND=240`).
- **The runner**: `RUN_BOUND` for an AM62P dEQP image should be at least the
  watchdog bound plus 60 s plus the boot (about 700 s). Then a run that
  reaches the bound shows the RTI reset in its own capture, and `no_reset`
  means a reset that really never came.

## Appendix: scratch sources (not committed)

### `tests/zz_pvr3d_scratch.rs` (the host repro of section 2)

The stand-in back end and OSAL are copied from
`tests/es2_pvr_vertex_texture.rs` (the same `YakoglBackend` fields, PowerVR
flags). Only the parts that differ are shown.

```rust
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
// use ... as tests/es2_pvr_vertex_texture.rs, plus yakogl::gl::framebuffer::glReadPixels

#[derive(Clone, Copy)]
struct Seen { tex: GlesTextureBind, ntex: u8, fsu: [u32; 16], fsu_n: u16, fs: usize }
static DRAWS: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn s_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 { return 0; }
    let mut fsu = [0u32; 16];
    let n = (d.fs_uniform_dwords as usize).min(16);
    if !d.fs_uniform_cpu.is_null() {
        for (i, v) in fsu.iter_mut().enumerate().take(n) { *v = *(d.fs_uniform_cpu as *const u32).add(i); }
    }
    DRAWS.lock().unwrap().push(Seen { tex: d.textures[0], ntex: d.num_textures, fsu, fsu_n: d.fs_uniform_dwords, fs: d.fs as usize });
    0
}
// STAND_IN: YakoglBackend { native_layered_textures: false, mip_level_align: 0, draw: Some(s_draw), .. }
// OSAL: host memory, gpu == cpu address (as es2_pvr_vertex_texture.rs)

/* glsTextureTestUtil.cpp ProgramLibrary, GLSL_VERSION_300_ES, PROGRAM_3D_FLOAT, highp */
const VS: &str = "#version 300 es\nin highp vec4 a_position;\nin highp vec3 a_texCoord;\nout highp vec3 v_texCoord;\n\nvoid main (void)\n{\n    gl_Position = a_position;\n    v_texCoord = a_texCoord;\n}\n";
const FS: &str = "#version 300 es\nlayout(location = 0) out mediump vec4 dEQP_FragColor;\nin highp vec3 v_texCoord;\nuniform highp float u_bias;\nuniform highp float u_ref;\nuniform highp vec4 u_colorScale;\nuniform highp vec4 u_colorBias;\nuniform highp sampler3D u_sampler;\n\nvoid main (void)\n{\n    dEQP_FragColor = texture(u_sampler, v_texCoord) * u_colorScale + u_colorBias;\n}\n";

/// One Texture3DFilteringCase: init (2 textures, every level), 4 draws + read-backs, deinit.
fn run_case(name: &str, ifmt: GLenum, w: i32, h: i32, d: i32, min: GLenum, mag: GLenum) {
    let levels = (32 - (w.max(h).max(d) as u32).leading_zeros()) as i32;
    let (fmt, ty, bpp) = transfer(ifmt); // RGBA8: RGBA/UNSIGNED_BYTE; R11F_G11F_B10F: RGB/UNSIGNED_INT_10F_11F_11F_REV; BGRA, BGRA8: BGRA/UNSIGNED_BYTE
    let mut texs = [0u32; 2];
    unsafe {
        glGenTextures(2, texs.as_mut_ptr());
        for &t in &texs {
            glBindTexture(GL_TEXTURE_3D, t);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            for l in 0..levels {
                let (lw, lh, ld) = ((w >> l).max(1), (h >> l).max(1), (d >> l).max(1));
                let data = vec![0x3cu8; (lw * lh * ld) as usize * bpp];
                glTexImage3D(GL_TEXTURE_3D, l, ifmt as GLint, lw, lh, ld, 0, fmt, ty, data.as_ptr() as *const c_void);
                // glGetError() collected and printed
            }
        }
        let prog = program(); // VS + FS above, link checked
        for sub in 0..4 {
            glBindTexture(GL_TEXTURE_3D, if sub < 2 { texs[0] } else { texs[1] });
            glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, min as GLint);
            glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAG_FILTER, mag as GLint);
            for p in [GL_TEXTURE_WRAP_S, GL_TEXTURE_WRAP_T, GL_TEXTURE_WRAP_R] { glTexParameteri(GL_TEXTURE_3D, p, GL_REPEAT as GLint); }
            glViewport(0, 0, 64, 64);
            glUseProgram(prog);
            // u_sampler 0, u_colorScale (1,1,1,1), u_colorBias (0,0,0,0); a_position vec4 x4, a_texCoord vec3 x4 from client arrays
            glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, [0u16, 1, 2, 2, 1, 3].as_ptr() as *const c_void);
            // glReadPixels 64x64 RGBA8
        }
        // print every Seen of this case: ntex, format, type_, layers, width x height, stride_texels, bytes,
        // linear_filter, min_linear, mip_filter, mip_levels, wrap_s/t, incomplete, bgra, no_alpha, fs, fsu
        glDeleteProgram(prog);
        glDeleteTextures(2, texs.as_ptr());
    }
}

#[test]
fn image20_and_21_first_14_cases() {
    // context 256x256 on the stand-in; then, in the board's (sorted) order:
    // image 20: for fmt in [bgra (GL_BGRA_EXT), bgra8 (GL_BGRA8_EXT), r11f_g11f_b10f] x 64x64x64
    //   (the board runs bgra8 first; the two give the same binds, so the order does not change the dump),
    // image 21: for size in [128x32x64, 32x64x16, 3x7x5] x GL_RGBA8,
    // each over [linear, linear_mipmap_linear, linear_mipmap_nearest, nearest, nearest_mipmap_linear,
    // nearest_mipmap_nearest] (mag = min for linear/nearest, else LINEAR), stopping after case 14.
}
```

### The word dump (temporarily at the end of `mod tests` in `src/pvr/backend.rs`)

```rust
#[test]
fn zz_scratch_pvr3d_words() {
    let b = |format, w, h, stride, minl, mipf: u8, miplv: u8| GlesTextureBind {
        format, type_: GLES_TEXTYPE_3D, base_gpu: HEAP_GENERAL_BASE + 0x0010_0000,
        width: w, height: h, stride_texels: stride, linear_filter: 1, min_linear: minl,
        mip_filter: mipf, mip_levels: miplv, wrap_s: GLES_WRAP_REPEAT, wrap_t: GLES_WRAP_REPEAT,
        no_alpha: (format == GLES_TEXFMT_R11G11B10F) as u8, ..Default::default()
    };
    // for each bind of section 2: let d = desc(&t); print d[0..16] and decode
    // IMAGE_WORD0 (d0,d1), STRIDE_IMAGE_WORD1 (d2,d3), SAMPLER_WORD0 (d8,d9), SAMPLER_WORD1 (d10,d11)
    // by the bit ranges of texstate.xml
}
```

### `egl_shim.c` and `run.sh` (step 3)

Both sit in one directory; `run.sh` takes the host dEQP from the checkout's
`build/host-deqp/` (made by `tools/cts/host-deqp-run.sh --gpu=powervr --module=gles3`).

```c
/* pvr-3d-hang investigation shim (gitignored build/, not part of the repo).
 *
 * Built as libEGL.so.1 and put first on LD_LIBRARY_PATH for the host
 * dEQP-GLES3 (yakogl target, which dlopen()s libEGL.so.1 by soname). With
 * EGL_KHR_get_all_proc_addresses (Mesa has it) dEQP takes EVERY GL function
 * from eglGetProcAddress (framework/egl/egluGLContextFactory.cpp:425), so the
 * interception is here: eglGetProcAddress returns the two draw wrappers below
 * and the real Mesa function for everything else. Every other EGL entry point
 * is an ifunc that resolves to the system libEGL's own symbol.
 *
 * Before each draw that samples a GL_TEXTURE_3D on unit 0, the texture is
 * made to sample the way the AM62P's PowerVR samples a 3D texture today
 * (yakogl texture_alloc_storage_3d: level 0 only; select.rs vol_uv: s,t
 * clamped into the slice, r clamped to the slices): GL_TEXTURE_MAX_LEVEL 0,
 * every wrap CLAMP_TO_EDGE, the min filter without its mipmap part.
 * PVR3D_SHIM=0 forwards everything unchanged (plain Mesa).
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <GLES3/gl3.h>

static void *real_lib(void)
{
	static void *h;
	static const char *const paths[] = { "/usr/lib64/libEGL.so.1", "/usr/lib/x86_64-linux-gnu/libEGL.so.1",
					     "/usr/lib/libEGL.so.1" };

	for (unsigned i = 0; !h && i < sizeof(paths) / sizeof(paths[0]); i++)
		h = dlopen(paths[i], RTLD_NOW | RTLD_GLOBAL);
	if (!h) {
		fprintf(stderr, "pvr3d-shim: no system libEGL.so.1\n");
		abort();
	}
	return h;
}

static void *real(const char *name)
{
	return dlsym(real_lib(), name);
}

#define FWD(n)                              \
	static void *r_##n(void)            \
	{                                   \
		return real(#n);            \
	}                                   \
	void n(void) __attribute__((ifunc("r_" #n)));

FWD(eglChooseConfig)
FWD(eglCopyBuffers)
FWD(eglCreateContext)
FWD(eglCreatePbufferSurface)
FWD(eglCreatePixmapSurface)
FWD(eglCreateWindowSurface)
FWD(eglDestroyContext)
FWD(eglDestroySurface)
FWD(eglGetConfigAttrib)
FWD(eglGetConfigs)
FWD(eglGetCurrentDisplay)
FWD(eglGetCurrentSurface)
FWD(eglGetDisplay)
FWD(eglGetError)
FWD(eglInitialize)
FWD(eglMakeCurrent)
FWD(eglQueryContext)
FWD(eglQueryString)
FWD(eglQuerySurface)
FWD(eglSwapBuffers)
FWD(eglTerminate)
FWD(eglWaitGL)
FWD(eglWaitNative)
FWD(eglBindTexImage)
FWD(eglReleaseTexImage)
FWD(eglSurfaceAttrib)
FWD(eglSwapInterval)
FWD(eglBindAPI)
FWD(eglQueryAPI)
FWD(eglCreatePbufferFromClientBuffer)
FWD(eglReleaseThread)
FWD(eglWaitClient)
FWD(eglGetCurrentContext)
FWD(eglCreateSync)
FWD(eglDestroySync)
FWD(eglClientWaitSync)
FWD(eglGetSyncAttrib)
FWD(eglCreateImage)
FWD(eglDestroyImage)
FWD(eglGetPlatformDisplay)
FWD(eglCreatePlatformWindowSurface)
FWD(eglCreatePlatformPixmapSurface)
FWD(eglWaitSync)

typedef void (*(*gpa_fn)(const char *))(void);

static void (*gpa(const char *name))(void)
{
	static gpa_fn f;

	if (!f)
		f = (gpa_fn)real("eglGetProcAddress");
	return f(name);
}

static unsigned long g_draws, g_forced;

static void report(void)
{
	fprintf(stderr, "pvr3d-shim: %lu draws, %lu with a 3D texture forced to level 0\n", g_draws, g_forced);
}

static void force_level0(void)
{
	static void (*gi)(GLenum, GLint *);
	static void (*tp)(GLenum, GLenum, GLint);
	static void (*gtp)(GLenum, GLenum, GLint *);
	static void (*at)(GLenum);
	GLint active = 0, tex = 0, min = 0;

	if (!gi) {
		gi = (void (*)(GLenum, GLint *))gpa("glGetIntegerv");
		tp = (void (*)(GLenum, GLenum, GLint))gpa("glTexParameteri");
		gtp = (void (*)(GLenum, GLenum, GLint *))gpa("glGetTexParameteriv");
		at = (void (*)(GLenum))gpa("glActiveTexture");
		atexit(report);
	}
	g_draws++;
	gi(GL_ACTIVE_TEXTURE, &active);
	at(GL_TEXTURE0);
	gi(GL_TEXTURE_BINDING_3D, &tex);
	if (tex) {
		g_forced++;
		gtp(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, &min);
		tp(GL_TEXTURE_3D, GL_TEXTURE_MAX_LEVEL, 0);
		tp(GL_TEXTURE_3D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		tp(GL_TEXTURE_3D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		tp(GL_TEXTURE_3D, GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE);
		if (min == GL_LINEAR_MIPMAP_LINEAR || min == GL_LINEAR_MIPMAP_NEAREST)
			tp(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
		else if (min == GL_NEAREST_MIPMAP_LINEAR || min == GL_NEAREST_MIPMAP_NEAREST)
			tp(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	}
	at((GLenum)active);
}

static void w_draw_elements(GLenum mode, GLsizei count, GLenum type, const void *indices)
{
	static void (*f)(GLenum, GLsizei, GLenum, const void *);

	if (!f)
		f = (void (*)(GLenum, GLsizei, GLenum, const void *))gpa("glDrawElements");
	force_level0();
	f(mode, count, type, indices);
}

static void w_draw_arrays(GLenum mode, GLint first, GLsizei count)
{
	static void (*f)(GLenum, GLint, GLsizei);

	if (!f)
		f = (void (*)(GLenum, GLint, GLsizei))gpa("glDrawArrays");
	force_level0();
	f(mode, first, count);
}

typedef void (*egl_fn)(void);

egl_fn eglGetProcAddress(const char *name)
{
	const char *on = getenv("PVR3D_SHIM");

	if (!(on && on[0] == '0')) {
		if (!strcmp(name, "glDrawElements"))
			return (egl_fn)w_draw_elements;
		if (!strcmp(name, "glDrawArrays"))
			return (egl_fn)w_draw_arrays;
	}
	return (egl_fn)gpa(name);
}
```

```sh
#!/bin/bash
# pvr-3d-hang scratch (gitignored build/): build the level-0 shim and run host
# dEQP-GLES3 on the host Mesa (llvmpipe, surfaceless pbuffer: no window) with
# one board image's case filter, as the board runs it (--deqp-case=<pattern>).
#   build/pvr3d-shim/run.sh <out-name> <deqp case pattern> [PVR3D_SHIM=0|1]
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
DEQP=$REPO/build/host-deqp/deqp-gles3/modules/gles3
rm -f "$HERE/libGLESv2.so.2"
gcc -shared -fPIC -O2 -Wall -o "$HERE/libEGL.so.1" "$HERE/egl_shim.c" -ldl
unset DISPLAY WAYLAND_DISPLAY
export EGL_PLATFORM=surfaceless LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
export PVR3D_SHIM=${3:-1}
cd "$DEQP"
LD_LIBRARY_PATH="$HERE" timeout "${PVR3D_TIMEOUT:-3000}" ./deqp-gles3 "--deqp-case=$2" \
    --deqp-log-filename="$HERE/$1.qpa" --deqp-log-images=disable --deqp-log-shader-sources=disable \
    --deqp-surface-type=pbuffer --deqp-surface-width=256 --deqp-surface-height=256 \
    > "$HERE/$1.out" 2>&1 || echo "deqp exit $?"
grep -o 'TestCaseResult CasePath="[^"]*"\|Unit="us">[0-9]*<\|StatusCode="[A-Za-z]*"' "$HERE/$1.qpa" |
    sed 's/TestCaseResult CasePath="dEQP-GLES3.functional.texture.filtering.//; s/"$//; s/Unit="us">//; s/<$//; s/StatusCode=//' |
    paste - - - | awk '{printf "%-60s %10.3f s  %s\n", $1, $2/1e6, $3}'
```
