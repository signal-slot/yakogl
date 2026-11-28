<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ETC2/EAC on the PowerVR: the 320 AM62P NotSupported cases (NS-ETC)

Design note, plan #400 item design-etc2 (roadmap stage 4). It covers the 320
dEQP-GLES3 cases that `tools/cts/am62p-notpass-causes.py` sorts into cause
`NS-ETC`, says where they are refused, and picks the design that makes them run
and pass on the AM62P's PowerVR BXS-4-64. Code addresses are of main
`ef87214a` (2026-11-28); `src/gl/texture.rs`, `src/backend.rs`, `src/pvr/**`
and `src/gl/state.rs` are the same at the integration tip `5f781c49`. Nothing
here was measured on a board for this note: the board numbers are the
not-pass TSV's (`docs/es3/am62p-gles3-notpass.tsv`), and the rest is reading.

**Short answer.** The refusal these 320 cases record is gone at main. Their
rows were run on build `ad2b5ed` (2026-10-13/14). At that build the PowerVR's
`GL_COMPRESSED_TEXTURE_FORMATS` named only ETC1, `RGB8_ETC2` and
`RGBA8_ETC2_EAC`. Since then the GL core has stored the sRGB ETC2 pair as
blocks on the PowerVR, and it decodes the six EAC and punch-through formats
on the CPU at upload (`4e539da8`, 2026-11-12, `docs/notes/etc2-eac.md`). An
ES 3.0 PowerVR build now lists all eleven names. The chosen design keeps that
path and changes no code. What is left is one PowerVR host test (section 6)
and the board run that measures the pictures. Native EAC/punch-through on the
TPU is the alternative (section 5). It would need a `src/gl` change, which
section 5.2 names exactly.

## 1. The cases

The `NS-ETC` rule (`tools/cts/am62p-notpass-causes.py:154`) matches
`texture.wrap.{etc2,eac}*` and `texture.format.compressed.*`. In the current
TSV that is **320 lines, all NotSupported**: 8 formats x 40 cases.

| format (dEQP name) | `texture.wrap` | `texture.format.compressed` |
|---|---:|---:|
| `etc2_srgb8` | 36 | 4 (`2d`/`cube` x `pot`/`npot`) |
| `etc2_eac_srgb8_alpha8` | 36 | 4 |
| `etc2_rgb8_punchthrough_alpha1` | 36 | 4 |
| `etc2_srgb8_punchthrough_alpha1` | 36 | 4 |
| `eac_r11` | 36 | 4 |
| `eac_signed_r11` | 36 | 4 |
| `eac_rg11` | 36 | 4 |
| `eac_signed_rg11` | 36 | 4 |
| **total** | **288** | **32** |

Each `texture.wrap` format has 36 cases: {clamp, repeat, mirror}² x {nearest,
linear} x {pot, npot}. There are two messages:

- 304 lines: `Compressed texture format not supported at gluTexture.cpp:142`.
  These are all 288 wrap cases and the 16 `*_2d_*` format cases, from
  `glu::Texture2D`.
- 16 lines: `Compressed texture format not supported: '' at
  gluTexture.cpp:345`. These are the `*_cube_*` format cases, from
  `glu::TextureCube`.

Both are dEQP checking the context's list
(`ContextInfo::isCompressedTextureFormatSupported`, i.e.
`GL_COMPRESSED_TEXTURE_FORMATS`) before it issues any GL call
(`docs/notes/etc2-eac.md` 7). The cases are in tree rows 39
(`texture.format.*` with `filtering.cube`, `mipmap`, `shadow`, `size`) and 41
(`texture.wrap.*`) of `tools/cts/am62p-gles3-tree.txt`, counted from 0. The
TSV header gives those rows as build `ad2b5ed`, run 2026-10-13/14.

The other three ETC formats in the mustpass list (`etc1`, `etc2_rgb8`,
`etc2_eac_rgba8`) have 120 cases. None of them is in the TSV, so all 120
passed on the AM62P at `ad2b5ed`. The TPU's native `ETC2_RGB` and
`ETC2A_RGBA` blocks, twiddled 2D and cube, NPOT and every wrap mode, are
therefore measured on silicon. In total the mustpass list has 440 ETC/EAC
cases (11 x 40). ASTC (`NS-ASTC`) is not part of this note: ES 3.0 does not
require it.

## 2. The refusal

### 2.1 At `ad2b5ed` (where the 320 come from)

`ad2b5ed` is older than the squashed history (`381b0f09`, 0.1.0) and is not
in this clone, so the line cannot be cited at that commit. The refusal is
inferred from its effect. The three names that ran are exactly the three that
`stored_compressed_format` stores on every build: ETC1, `RGB8_ETC2` and
`RGBA8_ETC2_EAC`. The eight that did not run are the sRGB pair and the six
decoded formats. The decoder landed on 2026-11-12 (`4e539da8`) and the ten ES
3.0 names on 2026-11-13 (`455c3bb5`). At 0.1.0 the PowerVR mask already had
the sRGB ETC2 classes (`git show 381b0f09:src/backend.rs`, line 190 onward),
so the pair was added between `ad2b5ed` and 0.1.0.

### 2.2 At main `ef87214a`

The GL core makes the list and the upload refusal from one test:

- `src/gl/texture.rs:3637-3650` `stored_compressed_formats` builds
  `GL_COMPRESSED_TEXTURE_FORMATS` and `GL_NUM_COMPRESSED_TEXTURE_FORMATS`
  (`src/gl/state.rs:1442-1443`). It lists every name in `COMPRESSED_FORMATS`
  (`texture.rs:3602-3615`, 11 names) that passes one of two checks.
- `src/gl/texture.rs:3626-3633` `stored_compressed_format` covers the block
  classes. The sRGB pair is refused there when `crate::REPORT_ES2` is set or
  when `backend::sampled_texfmts()` (`src/backend.rs:335`) lacks the class
  bit.
- `src/gl/texture.rs:3855-3861` `stored_decoded_format` covers the six decoded
  names. It applies the same test to the class the decode lands in
  (`decoded_format`, `texture.rs:3834-3851`: R16F, RGBA16F, RGBA8,
  SRGB8_ALPHA8).
- `src/gl/texture.rs:4148-4153` is the one place an upload of a name outside
  the list is refused (`GL_INVALID_ENUM`, "a format the storage does not
  hold"). `glCompressedTexImage2D` takes the decode branch first
  (`texture.rs:4450-4453`), and `glCompressedTexImage3D` does the same for 2D
  arrays (`texture.rs:6213`).

On the PowerVR, `sampled_texfmts()` is `PVR_SAMPLED_TEXFMTS`
(`src/backend.rs:215`). That mask holds `GLES_TEXFMT_ETC2_SRGB_RGB`,
`GLES_TEXFMT_ETC2_SRGB_RGBA` and `GLES_TEXFMT_SRGB8_A8` (`:217-219`, lines of the `texfmts` list), and
through `BASE_SAMPLED_TEXFMTS` (`:191-197`) also `RGBA8`, `RGBA16F` and
`R16F`. With `report-es2` off, every check above passes. The AM62P GLES3
images build with `CONFIG_YAKOGL_REPORT_ES2=n` (`tools/cts/am62p-deqp-images.sh:153`),
so all eleven names are listed and accepted. The unit test
`compressed_format_list_has_the_advertised_length`
(`src/gl/state.rs:3577-3610`) pins this list for `gpu-powervr`.

The PowerVR back end itself has no refusal. `texture_descriptor`
(`src/pvr/backend.rs:6840`) maps the four block classes in its last `else`
arm (`:6985-6996`). The six decoded formats arrive as ordinary
R16F/RGBA16F/RGBA8/SRGB8_A8 binds (`:6926-6959`). One hazard for any later
change: that last arm sends **every** class that no earlier arm claimed to
`TEXF_ETC2_PT`. A new compressed class that is not given its own arm there
would be sampled as punch-through ETC2, with no error.

## 3. What the BXS texture state can do

The TPU has native formats for all ten:

- `third_party/mesa-26.2.1/src/imagination/csbgen/rogue/texstate.xml:224-230`:
  `FORMAT_COMPRESSED` `ETC2_RGB` 68, `ETC2A_RGBA` 69, `ETC2_PUNCHTHROUGHA`
  70, `EAC_R11_UNSIGNED` 71, `EAC_R11_SIGNED` 72, `EAC_RG11_UNSIGNED` 73,
  `EAC_RG11_SIGNED` 74. These are generated into `src/pvr/csbgen.rs:976-982`.
  The field is IMAGE_WORD0 `texformat_compressed` [27:33] (`texstate.xml:281`).
- Mesa 26.2.1 `src/imagination/vulkan/pvr_arch_formats.c:158-167` maps all ten
  VK ETC2/EAC formats onto these values, with the sRGB variants on the same
  value plus gamma. This file is not vendored in `third_party/`; it was read
  in the project's Mesa 26.2.1 dependency tree. Its table is the one
  `docs/es3/pvr-formats.md` 2.4 reproduces. Mesa sets `textureCompressionETC2`
  for every Rogue core without a feature test (`pvr_physical_device.c:253`),
  so it treats ETC2/EAC as part of the TPU, not as a per-core option.
- The TI DDK headers (`third_party/ti-img-rogue-ddk-25.2`) are the kernel
  side only. They have no texture-state format table and no ETC/EAC entry,
  so they neither add to Mesa's answer nor contradict it.

Of these, only `ETC2_RGB` and `ETC2A_RGBA`, without and with gamma, are used
by yakogl. `ETC2_PUNCHTHROUGHA` has a constant (`TEXF_ETC2_PT`,
`src/pvr/backend.rs:1374`), but no GL name reaches class 3. The EAC values
are unused.

## 4. Chosen design: the GL core's CPU decode for the six, native blocks for the four (as on main)

Keep what main does on the PowerVR:

| GL name | how it is stored | PowerVR descriptor |
|---|---|---|
| `RGB8_ETC2`, `ETC1_RGB8_OES` | blocks, twiddled grid | `ETC2_RGB`, alpha `SRC_ONE` |
| `RGBA8_ETC2_EAC` | blocks | `ETC2A_RGBA` |
| `SRGB8_ETC2` | blocks | `ETC2_RGB` + IMAGE_WORD0 `gamma` |
| `SRGB8_ALPHA8_ETC2_EAC` | blocks | `ETC2A_RGBA` + `gamma` |
| `R11_EAC`, `SIGNED_R11_EAC` | decoded to R16F | `F16` (`TEXF_R16F`) |
| `RG11_EAC`, `SIGNED_RG11_EAC` | decoded to RGBA16F (r, g, 0, 1) | `F16F16F16F16` |
| `RGB8_PUNCHTHROUGH_ALPHA1_ETC2` | decoded to RGBA8 | `U8U8U8U8` |
| `SRGB8_PUNCHTHROUGH_ALPHA1_ETC2` | decoded to SRGB8_ALPHA8 | `U8U8U8U8` + `gamma` |

The values, the rounding (`es3_formats::f32_to_half`) and the margin against
dEQP's thresholds are in `docs/notes/etc2-eac.md` 1-3. The worst case is the
half-float rounding of an 11-bit value, about 3 % of the tightest wrap
threshold.

**Code changes: none.** Neither the 320 cases running nor, as far as can be
judged here, their passing depends on a change in `src/pvr/**`, `driver/` or
`src/gl/**`. The new parts are the host test in section 6 and the board run.

Why this path and not native:

1. **Each piece it relies on already passes on the AM62P, except the sRGB
   blocks.** The board rows of the TSV contain no `texture.format.sized`
   (2d, cube, 2d_array, 3d; pot and npot) or `texture.filtering.{2d,cube}.formats`
   case of R16F, RGBA16F or SRGB8_ALPHA8. These are 48 mustpass cases, so all
   of them pass, cube and NPOT included. `texture.wrap.rgba8` (36 cases,
   every wrap mode and NPOT, same STRIDE RGBA8 descriptor as decoded
   punch-through) has no line either. The linear ETC2 blocks pass in all 120
   cases (section 1). The sRGB block pair is the one untested piece: the
   `ETC2_RGB`/`ETC2A_RGBA` words that pass, plus the `gamma` bit that passes
   on SRGB8_ALPHA8.
2. **It needs no edit in files other plans hold.** `src/pvr/**` belongs to
   plan 377, and `src/gl/**` is held by plan 397 and needs a named change
   (section 5.2). The decode is format-blind toward the back end.
3. **The cost is memory and upload time, not correctness.** R11 takes 4x the
   bytes, and RG11 and punch-through 8x (`etc2-eac.md` 1). dEQP's
   textures are small, and no AM62P UI ships EAC today.

Risks the board has to settle, all within the 320:

- **R16F and RGBA16F under REPEAT/MIRRORED_REPEAT on an NPOT STRIDE image**
  (64 of the 144 `wrap.eac_*` cases: NPOT with REPEAT or MIRRORED on S or T). The STRIDE wrap path is
  measured only with RGBA8 (`wrap.rgba8`). `format.sized` and
  `filtering.formats` for the F16 classes do not use every wrap mode.
- **The sRGB ETC2 blocks on silicon** (80 cases: `etc2_srgb8`,
  `etc2_eac_srgb8_alpha8`). These are `TEXF_ETC2_SRGB_*`
  (`src/pvr/backend.rs:1361-1362`), not measured yet.
- **The decoded NPOT cube** (`format.compressed.eac_*_cube_npot` and
  `*punchthrough*_cube_npot`: 6 cases) goes through the GL core's packed NPOT
  cube chain (`texture.rs` `packed_npot_cube`, `stacked_decoded_array`
  above it at `:1312`). The uncompressed F16 NPOT cubes, which pass, take the
  same path.

If one of these fails on the board, the fix goes into the PowerVR back end or
the decode classes. It is a picture fault, no longer NotSupported, and gets
its own item.

## 5. The alternative: native EAC and punch-through on the TPU

### 5.1 What it would take

It would replace the CPU decode of the six on the PowerVR with TPU formats
70-74. The memory cost of section 4 goes away, and `glCompressedTexSubImage*`
writes blocks instead of decoded texels.

- `src/backend.rs`: five new `GLES_TEXFMT_*` classes (R11, signed R11, RG11,
  signed RG11, sRGB punch-through), with `gles_texfmt_bytes`/block sizes. Their
  bits, plus class 3 `GLES_TEXFMT_ETC2_RGB_PT`, go into `PVR_SAMPLED_TEXFMTS`.
  The mask is `u64`, so there are free bits.
- `src/pvr/backend.rs` (plan 377's files):
  - `TEXF_EAC_R11`, `TEXF_EAC_R11_SIGNED` and `TEXF_EAC_RG11*` via `tex_etc`,
    with swizzles (R, 0, 0, 1) and (R, G, 0, 1).
  - `TEXF_ETC2_SRGB_PT` = `TEXF_ETC2_PT` + `gamma`.
  - Their arms in `is_etc2` (`:1364-1373`, which decides the twiddled block
    layout and the cube/array twiddled words).
  - Explicit arms in `texture_descriptor`'s compressed branch (`:6985-6996`)
    in place of the fall-through to `TEXF_ETC2_PT`.
  - The pack info. Mesa's `setup_pck_info` gives a compressed format
    `PVR_PCK_FORMAT_INVALID` (0x1f) with the UNORM bit only when
    `util_format_is_unorm`, so the signed EAC formats would be 0x1f, not
    yakogl's `TEX_PCK_INFO_COMPRESSED_UNORM` 0x11f. As the comment at
    `:6949-6950` notes, pco reads this word only for shadow compares and
    formatless stores, so it matters little for sampling. The host test should
    still pin it.
- Not measured anywhere: the TPU's EAC decode precision against dEQP's 7-bit
  nearest threshold (0.0078585), and how SNORM blocks are clamped (`-1024` →
  -1.0).

### 5.2 The `src/gl` change it needs, named exactly

1. `src/gl/texture.rs` `compressed_format_info` (`:3588-3598`): six arms.
   - `GL_COMPRESSED_R11_EAC` → (R11 class, 8)
   - `GL_COMPRESSED_SIGNED_R11_EAC` → (signed R11, 8)
   - `GL_COMPRESSED_RG11_EAC` → (RG11, 16)
   - `GL_COMPRESSED_SIGNED_RG11_EAC` → (signed RG11, 16)
   - `GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2` → (`GLES_TEXFMT_ETC2_RGB_PT`, 8)
   - `GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2` → (sRGB PT, 8)
2. `stored_compressed_format` (`:3626-3633`): the mask test applies to every
   new class, not only the two sRGB ones. A back end without the bit then
   falls back to the decode.
3. The decode branches give way to native storage. In `glCompressedTexImage2D`
   (`:4450`) and in the 2D-array decode in `glCompressedTexImage3D`
   (`:6213`), take `stored_decoded_format` only when
   `stored_compressed_format(internalformat)` is `None`. The same order is
   needed in the sub-image path (`texture_write_decoded`, `:3919`). Otherwise
   a back end with both would keep decoding.
4. `stored_compressed_formats` and `COMPRESSED_FORMATS` stay as they are: the
   list is the same eleven names.

The GC (`docs/notes/etc2-eac.md` 6) and the V3D would take the same `src/gl`
change. Their masks decide whether they use it.

### 5.3 Why not now

It adds a `src/gl` change and new back-end classes, plus a TPU decode
that no board has run for EAC. None of that is needed for the 320 cases to
run. The decode path already samples on silicon through classes that pass.
Native is a follow-up worth an issue (memory). If the board shows section 4's
F16 wrap risk as a real fault, native is also the way around it, because EAC
blocks then sample in the twiddled layout the linear ETC2 blocks already pass
in.

## 6. The host test to add

`tests/pvr_etc2_eac_words.rs`, feature set
`--no-default-features --features host-test,gpu-powervr,shaderc-online`,
`#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]`.
It follows `tests/pvr_native3d_sampler_words.rs`: a stand-in back end
captures the `GlesTextureBind` of one draw, and the PowerVR's own
`pvr::backend::texture_descriptor` packs it. Nothing renders: the test checks
the words.

For each of the ten ES 3.0 names (and ETC1):

1. The name is in `GL_COMPRESSED_TEXTURE_FORMATS`, and the list has 11
   entries. This repeats the `state.rs` unit test through the public API, as
   dEQP's `gluTexture.cpp:142/345` asks it.
2. `glCompressedTexImage2D` of a POT (8x8) and an NPOT (12x20) level. Then
   one triangle sampling it through `sampler2D`, with `GL_TEXTURE_WRAP_S/T` =
   REPEAT / MIRRORED_REPEAT and a LINEAR filter. Assert:
   - The bind's class is the table's in section 4.
   - IMAGE_WORD0 `texformat`/`texformat_compressed` [27:33] is 68/69 for the
     block classes and `F16` (11) / `F16F16F16F16` (28) / `U8U8U8U8` (12) for
     the decoded ones.
   - `gamma` [3] is set exactly for the three sRGB names.
   - `textype` [0:2] is 2D (twiddled) for blocks and STRIDE for the decoded
     ones.
   - The swizzle `swiz0..3` [5:16] gives alpha `SRC_ONE` for RGB8/SRGB8.
   - For the blocks, the twiddled IMAGE_WORD1
     `tpu_image_state_v2_compression_mode` [54:56] is TPU. The pack info
     `w[7]` is 0x11f for blocks, `TEX_PCK_INFO_F16` for R16F/RGBA16F,
     `TEX_PCK_INFO_RGBA8_UNORM` and `TEX_PCK_INFO_SRGB8_A8` for the
     punch-through pair.
   - SAMPLER_WORD0 `addrmode_u` [41:43] and `addrmode_v` [44:46] are the wrap
     modes asked for.
3. The same name as a cube (`GL_TEXTURE_CUBE_MAP`, six faces, POT 8 and NPOT
   12): the cube bind's format words are those of 2. The texture's six faces
   are uploaded, so the draw is not refused.
4. A guard for section 2.2's fall-through: a class that the PowerVR mask does
   not hold never reaches `texture_descriptor`. Equivalently, the bind of every
   one of the eleven names has an explicit format, never `TEXF_ETC2_PT`, until
   class 3 has a GL name.

The decoded texel values are already pinned by `tests/es3_etc2_eac.rs`
(`gpu-powervr` arm, against Mesa's outputs) and the upload forms by
`tests/es3_etc2_upload_forms.rs`. The new test covers only the PowerVR words
between those and the TPU. Section 5's native path would change the expected
words for the six decoded names in step 2, so the test is also that change's
pin.

## 7. Expected results

At a build from main `ef87214a` or later, on the AM62P GLES3 image (rows 39
and 41):

- **NotSupported → ran: all 320.** No `Compressed texture format not
  supported` line should remain for an ETC2/EAC name. If one does, the image
  was built with `REPORT_ES2`, which section 2.2's check rules out.
- **Expected to pass: 320**, in three confidence groups:
  - 176 on classes and wrap paths already measured on the board: the 80
    punch-through cases (RGBA8 / SRGB8_ALPHA8 STRIDE, the `wrap.rgba8` and
    `srgb8_alpha8` paths), and 96 EAC cases (the 4 formats' 72 `pot` wrap
    cases, their 16 `format.compressed` cases, and their 8 `npot`
    clamp/clamp cases).
  - 64 with section 4's F16 STRIDE risk: the 4 EAC formats' `npot` wrap
    cases with REPEAT or MIRRORED_REPEAT on S or T (16 per format).
  - 80 with section 4's sRGB-block risk: `etc2_srgb8` and
    `etc2_eac_srgb8_alpha8`.

  The expectation is reading plus the 48 + 36 + 120 board passes of section 4,
  not a measurement. The host dEQP cannot tell: its read-back is black, and
  every after-arm case is `Image verification failed` there
  (`etc2-eac.md` 7).
- `NS-ETC` in `am62p-notpass-causes.py` then matches nothing. Any remaining
  ETC/EAC line is a Fail with its own cause.

## What is not measured

No board has sampled a decoded EAC/punch-through texture or an sRGB ETC2
block. The TPU's native EAC formats (70-74) have never been programmed by
yakogl. The case counts are those of the TSV at `ad2b5ed`. A later board run
of rows 39 and 41 replaces them.
