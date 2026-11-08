<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: can the texture unit sample the D24S8 depth surface?

A read-only study. `ds_cache::resolve_depth` (`src/vivante/ds_cache.rs:562`)
turns the depth surface into R32F texels on the CPU, behind a `finish()`, once
a frame in glmark2 `shadow` and `refract` (`docs/notes/gc-depth-resolve.md`,
`docs/notes/imx8mp-glmark2-cpu.md` "Fix items, in order", item 2). The V3D
samples its depth buffer itself and says so through
`backend::samples_rendered_depth` (`src/backend.rs:1858`) and
`GlesTextureBind::depth_id`. This note asks whether the GC back end can do the
same. Nothing here was run on a board.

**History.** First written for YakoGL 0.1.0 (`381b0f09`) without the etnaviv
sources: its verdict was "unknown, leaning yes" by an 8888 view, and four
statements were marked *from memory, unchecked*. **Revised 2026-11-07** (plan
#328, item depth-tpu-mesa) against Mesa 26.2.1's etnaviv: the verdict, the
format statements, the sources section and the board experiment changed;
every place that changed says **[2026-11-07]**, and what the first version
said is kept beside it. In-tree line numbers are at main `98653bb5`.

## Verdict [2026-11-07]: yes on paper, by Mesa's own format; one board case decides

**Mesa samples a Z24S8 depth texture in place, through a real depth texture
format, `TEXTURE_FORMAT_D24X8` (0x11), with no copy and no shader rebuild**
(section (a), "How etnaviv samples depth"). The GC7000UL has every feature bit
that path needs, including linear texture addressing. What Mesa never does is
the combination this back end would need: a depth surface the PE wrote
**row-linear**, sampled with **LINEAR** addressing. Mesa always renders depth
tiled (supertiled on this part) and samples it that way. So the one open
question is whether the TPU's D24X8 reads a linear surface the way it reads a
supertiled one. The board case at the end of (a) answers it. The 8888 view
with the depth rebuilt in the shader (the first version's route) is still a
valid fallback, but it is not what Mesa does, and it is the costlier route.

*First version (2026-10-27), kept for the record:* "**Unknown.** No TPU format
that means "D24S8" is known on this part, and one board experiment decides the
rest. [...] There is a route that needs no new format: sample the words as an
8888 texel and rebuild the 24-bit depth from the channels. It is a yes only
after the board check at the end of (a)." The format statement was wrong
(below). The board check is replaced by the one at the end of (a).

### Sources, and what is missing

**[2026-11-07]** Mesa **26.2.1** (tag `mesa-26.2.1`, `VERSION` `26.2.1`). This
is the release `docs/notes/gc-shadow-compare.md` and
`gc-texture-swizzle-wrap.md` read, from the same tree in the deps cache
(`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`, outside the repository;
`third_party/` is unchanged and still has no etnaviv directory).
`src/vivante/texture.rs:625` names Mesa main `c8ce6c8bdfbb` for one
descriptor rule. The etnaviv files cited here were not compared against that
commit. Paths are relative to Mesa's `src/`, with these letters:

* `F` `gallium/drivers/etnaviv/etnaviv_format.c`
* `D` `gallium/drivers/etnaviv/etnaviv_texture_desc.c` (the descriptor path
  HALTI5 parts such as this one use)
* `T` `gallium/drivers/etnaviv/etnaviv_texture.c`
* `R` `gallium/drivers/etnaviv/etnaviv_resource.c`
* `S` `gallium/drivers/etnaviv/etnaviv_state.c`
* `Sc` `gallium/drivers/etnaviv/etnaviv_screen.c`
* `Tr` `gallium/drivers/etnaviv/etnaviv_translate.h`
* `RS` `gallium/drivers/etnaviv/etnaviv_rs.c`
* `X` `etnaviv/hw/common_3d.xml.h`
* `N` `etnaviv/hw/state_3d.xml.h`
* `Hw` `etnaviv/hwdb/etna_hwdb.c` (NXP database bit to Mesa feature)
* `Db` `etnaviv/hwdb/nxp/gc_feature_database.h`

The board's core is model `0x7000`, revision `0x6204`, product `0x70003`
(`src/vivante/VIVANTE_NOTES.md:13-14`). `Db` has two entries for it,
`gc7000ULN_v123` with EcoID 0 (`Db:18839-18844`) and EcoID 1
(`Db:19733-19738`). Every bit cited below has the same value in both, so it
does not matter which EcoID the board reports. `etnaviv_zsa.c` holds only the
depth/stencil test state and early-Z (`:66`), nothing about sampling. The
Linux UAPI (`include/drm-uapi/etnaviv_drm.h`) carries buffers, submits and
parameters and no texture or depth layout, so nothing here depends on it.

In-tree sources used:

- `src/vivante/regs.rs:876-899` and `:907-955`: the TEXDESC format values the
  back end knows (`common_3d.xml.h TEXTURE_FORMAT_*`): A8 0x01, L8 0x02, A8L8
  0x04, A8R8G8B8 0x07, X8R8G8B8 0x08, A8B8G8R8 0x09, X8B8G8R8 0x0a, R5G6B5
  0x0b, E5B9G9R9 0x1d, ETC1 0x1e. The extended set (CONFIG1, CONFIG0 format 0)
  has ETC2 0x01/0x02 and the EACs, G8R8 0x06, R16F 0x07, G16R16F 0x08,
  A16B16G16R16F 0x09, R32F 0x0a, G32R32F 0x0b, A2B10G10R10 0x0c, and the snorm
  formats 0x0e-0x11. **No D16, D24X8 or D24S8 value is among them**: the back
  end has never named Mesa's depth formats (`X:59-60`, `X:94`, below).
  - *First version, from memory, unchecked:* "etnaviv has no depth entry in
    its texture format table that samples a depth buffer in place; it samples
    a depth texture through a copy (an RS resolve into a colour-format
    texture) and compares in the shader." **[2026-11-07] Wrong on all three
    counts.** `F:113` maps `Z16_UNORM` to `D16`, and `F:179-180` map
    `X8Z24_UNORM` and `S8_UINT_Z24_UNORM` to `D24X8` (`X:59` 0x10, `X:60`
    0x11). Mesa samples in place on this part with no copy (section (a)). It
    compares in the texture engine on HALTI2+ and in the shader only for the
    emulated depth32f formats (`gc-shadow-compare.md` section 2, `D:112-113`).
- `src/vivante/ds_cache.rs:545-561` (doc of `resolve_depth`) and
  `src/vivante/backend.rs:1500-1512`: the layout, measured on the i.MX8MP
  2026-09-30 (gl_test 169).
- `src/vivante/texture.rs:306` (`build_descriptor`), the descriptor the back
  end writes today; `:288-295` the `depth_luminance` swizzle.

## (a) Can the TPU read the surface, and as what

### How etnaviv samples depth on a GC7000-class part [2026-11-07, new]

1. **Format.** A depth texture of gallium format `S8_UINT_Z24_UNORM` or
   `X8Z24_UNORM` (GL's DEPTH24_STENCIL8 and DEPTH_COMPONENT24) translates to
   `TEXTURE_FORMAT_D24X8` = 0x11 (`F:179-180`, `X:60`), a non-extended format.
   It goes in CONFIG0's `FORMAT` field, and CONFIG1's `FORMAT_EXT` stays 0
   (`D:179-185`: `COND(!ext && !astc, CONFIG0_FORMAT(format))`). `Z16_UNORM`
   is `D16` = 0x10 (`F:113`). The stencil of the same words is a separate
   view: `S8X24_UINT`/`X32_S8X24_UINT` are `EXT_D24S8` = 0x22 (`F:183-184`,
   `X:94`) with `SAMP_CTRL0.DEPTH_STENCIL_MODE_STENCIL` (`D:254-258`,
   `N:1973`). The same D24S8 words therefore serve as the depth view and the
   stencil view, chosen by format.
2. **The words are the PE's.** The PE renders those gallium formats as
   `PE_DEPTH_CONFIG_DEPTH_FORMAT_D24S8` (`Tr:226-229`, `N:927`). The note at
   `Tr:222` reads "Pipe format convention is LSB to MSB, VIVS is MSB to LSB",
   so `S8_UINT_Z24_UNORM` is S in bits 7..0 and Z in 31..8: the
   `depth << 8 | stencil` word of `ds_cache.rs:611` (`>> 8`). Mesa samples
   the very buffer the PE wrote, so the TPU's D24X8 reads the PE's D24S8 word
   as it is. No repacking exists anywhere in etnaviv.
3. **Layout: supertiled, sampled in place.** `etna_resource_create` gives a
   depth/stencil resource `ETNA_LAYOUT_TILED | BIT_SUPER` when
   `specs.can_supertile`, and `BIT_MULTI` only when the PE is multi-tiled
   (`R:572-576`). `can_supertile` is `ETNA_FEATURE_SUPER_TILED` (`Sc:878-879`),
   which comes from `REG_SuperTiled32x32` (`Hw:57`), and that bit is 1
   (`Db:18995`/`19889`). `pe_multitiled` needs `pixel_pipes > 1`
   (`Sc:978-979`), and there is 1 (`Db:18866`/`19760`). So Mesa's depth
   surface here is `SUPER_TILED`, 64x64-padded (`R:389-392`), and the PE is
   told so (`S:421`, `S:428-432`). Mesa never renders depth linear: `S:416`
   asserts `ETNA_LAYOUT_BIT_TILE` ("Cannot render to linear surfaces").
   Sampling: `etna_texture_handle_incompatible` (`T:242-271`) makes a tiled
   copy only when `etna_resource_sampler_compatible` fails. That function
   accepts `SUPER_TILED` when `ETNA_FEATURE_SUPERTILED_TEXTURE` is set
   (`T:219-220`; `Hw:69` `REG_SuperTiledTexture`, 1 at `Db:19050`/`19944`) and
   `LINEAR` when `ETNA_FEATURE_LINEAR_TEXTURE_SUPPORT` is set (`T:223-224`;
   `Hw:66` `REG_LinearTextureSupport`, 1 at `Db:19037`/`19931`). So on the
   GC7000UL the depth texture is its own render buffer, with no blit or RS
   copy between them. The descriptor points LOD 0 at that buffer
   (`D:205`), with `LINEAR_STRIDE` from its level (`D:190`) and CONFIG1
   `HALIGN` from its layout (`D:186`). `ADDRESSING_MODE_LINEAR` (3, `X:131`)
   is set exactly when the resource is `ETNA_LAYOUT_LINEAR` (`D:181-182`). The
   format is not a condition, so a linear D24X8 view is a descriptor Mesa's
   code would build. Mesa never meets one, because its depth is never linear
   (`S:416`).
4. **Tile status: yes in Mesa, and the sampler reads it.** Fast clear is on
   (`REG_FastClear` 1, `Db:18951`/`19845`, `Hw:43`; MMUv2 keeps it,
   `Sc:986-992`), so a depth resource gets a TS buffer (`R:531-535`,
   `R:148-170`). It is uncompressed: compression needs V4 compression or MSAA
   (`R:196-199`), and `V4Compression` is 0 (`Db:19191`/`20085`). A depth
   clear is a TS fast clear (`RS:437-474`). Before a draw samples it,
   `etna_update_sampler_source` (`T:162-206`) binds the TS to the sampler
   (`TS_SAMPLER_*`, `T:79-115`) when `etna_can_use_sampler_ts` allows
   (`T:122-159`: level 0 only, feature `TEXTURE_TILED_READ` = `Hw:74`
   `REG_TextureTileStatus`, 1 at `Db:19076`/`19970`). Otherwise it resolves
   the TS in place first (`T:192-198`). On this part, then, Mesa samples
   depth with sampler TS and no resolve. **This back end has no depth TS**
   (TS mode 0, `DISABLE_ZS`, clears through the PE: "Fast clear and tile
   status" below), so none of this applies and no resolve sits in the way.
5. **Filter.** D24X8 never takes `INT_FILTER`:
   `texture_use_int_filter` returns false for any depth/stencil format
   (`F:320`). A linear mip filter on a Z/S view is forced to NEAREST
   (`D:324-328`). MIN/MAG come from the sampler as usual (`D:105-114`).
6. **Swizzle and which channel holds depth.** The descriptor's swizzle is the
   view's, passed through (`F:347-383`, `PIPE_SWIZZLE_* == TEXTURE_SWIZZLE_*`
   at `F:373-378`, `X:117-122`). There is nothing depth-specific in it. The
   GL depth mode comes from Mesa core: `DEPTH_TEXTURE_MODE` defaults to
   `GL_LUMINANCE` outside desktop core (`mesa/main/texobj.c:395`), which is
   swizzle (X, X, X, 1) (`mesa/main/teximage.c:793-798`). So the D24X8 texel's
   **X (red) channel is the depth**, a float in [0, 1]. What the TPU puts in
   Y, Z and W for D24X8, and the precision of X (all 24 bits as fp32, or
   fewer), is **not found in Mesa**. The board case measures X.
7. **Compare.** The texture engine compares on this core: `SAMP_CTRL0`
   `COMPARE_ENABLE` (`N:1974`) and `COMPARE_FUNC` (`D:112-113`), the
   reference in `TEXLD` src2. In full, with the shader route Mesa keeps for
   depth32f: `docs/notes/gc-shadow-compare.md` section 2. That note's open
   point ("whether the TE's compare operates on a texel format that is not a
   depth format") goes away with D24X8, which is a depth format: the same
   TE compare Mesa uses would apply. **glmark2 needs no compare at all**:
   `shadow` samples `texture2D(ShadowMap, ...).x` and compares in GLSL
   (`data/shaders/shadow.frag:10-13`), and `refract` reads its `DistanceMap`
   the same way (`data/shaders/light-refract.frag:25`). Both make the
   texture NEAREST, CLAMP_TO_EDGE (`src/scene-shadow.cpp:103-106`,
   `src/scene-refract.cpp:195-198`, glmark2 2023.01).

### This back end's surface

**Layout.** The PE writes the D24S8 surface **row-linear** at `Surface::stride`
(`align(w, 64) * 4` bytes a row, `align(h, 64)` rows), row 0 at GL's y 0.
`PE_DEPTH_CONFIG` never sets `GC_PE_DEPTH_SUPER_TILED` (`regs.rs:423`,
`backend.rs:1500-1512`). It is not 4x4-tiled and not supertiled, and it has
no tile status: TS mode 0, so memory holds every pixel (`ds_cache.rs:553-555`).
Each word is `depth << 8 | stencil`: depth is bits 31..8, stencil bits 7..0
(`>> 8` in the resolve). That is the layout of a LINEAR texture
(`GC_TEX_ADDRESSING_LINEAR`, `regs.rs:801`, the mode `build_descriptor`
already uses for every untiled texture), so the addressing side needs nothing
a normal RGBA8 texture does not. **[2026-11-07]** The word is the one Mesa's
D24X8 reads (point 2 above). Only the addressing differs from Mesa's path
(point 3): LINEAR instead of SUPER_TILED. `LinearTextureSupport` is set on
this part, and Mesa's descriptor code would emit LINEAR for a linear
resource of any format, but no Mesa path samples a linear *depth* surface.
That combination is what the board case tests.

**Format.** Two readings of the words:

1. **ARGB8888 view, depth rebuilt in the shader (the first version's
   route; now the fallback).** A little-endian word `D2 D1 D0 S` (depth byte
   2, 1, 0 in bits 31..24, 23..16, 15..8; stencil in 7..0) read with
   `GC_TEX_FORMAT_A8R8G8B8` (0x07) gives B = stencil, G = depth bits 7..0,
   R = depth bits 15..8, A = depth bits 23..16. **[2026-11-07] Checked**: the
   name is MSB to LSB (`Tr:222`), and `F:164` maps gallium `B8G8R8A8_UNORM`
   (B in the lowest byte) to `A8R8G8B8`. So A is bits 31..24, R 23..16, G
   15..8, B 7..0, as stated. With the descriptor's swizzle
   (`GC_SWIZZLE_*`, `regs.rs:993-998`) the three depth channels can be routed
   to R, G, B and the stencil dropped (A = `GC_SWIZZLE_1`). The texel then
   arrives as three unorm8 values `c = k / 255`. The depth the CPU resolve
   builds is `((word >> 8) as f64 / 16_777_215.0) as f32`
   (`ds_retention::unorm24_depth`), i.e. `(A*65536 + R*256 + G) / 16777215`
   with the three bytes as integers. A shader rebuilds it as
   `(round(255*a)*65536 + round(255*r)*256 + round(255*g)) / 16777215`: every
   intermediate is an integer below 2^24, exact in fp32, so the result equals
   the CPU's value to the last bit if the TPU returns `k / 255` correctly
   rounded (not measured). The filter must be NEAREST on the depth channels:
   a bilinear filter of the three bytes is not a bilinear filter of the depth
   (carry between bytes), and a shadow compare wants the nearest or a
   per-texel compare anyway. `SAMP_CTRL0 INT_FILTER` is the 8-bit path, and
   Mesa's rule allows it for a unorm8 colour view (`F:297-338`). It stays on.
   This route is not what Mesa does. It costs the shader work in (b) 4, and
   the TE compare cannot be used with it.
2. **A depth format.** *First version, from memory, unchecked:* "some Vivante
   parts document a depth sampler format; etnaviv's table is where it would
   show. Unknown whether the GC7000UL has one." **[2026-11-07] Checked: it
   exists, and it is Mesa's route.** `TEXTURE_FORMAT_D24X8` = 0x11 (`X:60`,
   `F:179-180`), CONFIG0 `FORMAT`, `INT_FILTER` off (`F:320`). Mesa applies it
   on every core it drives, with no feature gate on the format
   (`translate_texture_format`, `F:276-293`). The GC7000UL has the addressing
   features it needs (point 3). A `.r` read returns the depth, so the GLSL
   sample needs no rewrite and the TE compare is available. **This is the
   route to try first.**

**Descriptor fields** **[2026-11-07: route 2 is the primary column]**
(against `build_descriptor`, `texture.rs:306`):

| field | route 2 (D24X8, Mesa's) | route 1 (8888 fallback) | today's code |
|---|---|---|---|
| CONFIG0 `TYPE` | `GC_TEX_TYPE_2D` | same | `build_descriptor` |
| CONFIG0 `FORMAT` | **0x11** `D24X8` (`X:60`; new constant `GC_TEX_FORMAT_D24X8` in `regs.rs`) | `GC_TEX_FORMAT_A8R8G8B8` (0x07) | `format_of` |
| CONFIG1 `FORMAT_EXT` | 0 (`D:183`) | 0 | |
| CONFIG0 `ADDRESSING_MODE` | `GC_TEX_ADDRESSING_LINEAR` (3, `X:131`) | same | as for an untiled texture |
| CONFIG1 `HALIGN` | as for an untiled texture today | same | |
| CONFIG1 swizzle | ES 2.0 (d, d, d, 1): R, R, R, 1 (`depth_luminance`, `texture.rs:288-295`); ES 3.0 (d, 0, 0, 1) | R<-A (depth 23..16), G<-R (15..8), B<-G (7..0), A<-1 | `texture.rs:288-295` |
| `LINEAR_STRIDE` | the surface's `stride` (a multiple of 256 bytes) | same | `texture.rs:375` |
| `SIZE` | the texture's width x height | same | |
| `LOG_SIZE_EXT`, `BASELOD` | one level: `levels = 1` | same | |
| `SLICE`, `3D_CONFIG` | stride * height, depth 1 | same | |
| LOD 0 address | surface GPU address **+ guard** (`GC_DEPTH_GUARD_BYTES`, `backend.rs:90`) | same | `texture.rs:431` takes `t.base_gpu` |
| `SAMP_CTRL0` | NEAREST (glmark2's), CLAMP_TO_EDGE as the texture asks, **`INT_FILTER` off** (`F:320`), MIP NEAREST (`D:324-328`) | NEAREST forced, `INT_FILTER` on | `texture.rs:585` |
| `SAMP_CTRL0` compare | `COMPARE_ENABLE`/`FUNC` when the GL asks (`gc-shadow-compare.md`) | not available | none today |

The surface's width in memory is `stride / 4` words, wider than the texture
(`align64`): `SIZE` is the texture's, `LINEAR_STRIDE` is the surface's, the
same relation `stride_texels` has to `width` for a padded colour texture.

**Fast clear and tile status: none to resolve.** The GC back end never clears
with the RS engine: a glClear is a quad through the PE, depth test always,
write on (`VIVANTE_NOTES.md:109-110`), and there is no BLT engine
(`VIVANTE_NOTES.md:1205`; **[2026-11-07] checked**: `REG_BltEngine` is 0 at
`Db:19174`/`20068`). `DISABLE_ZS` is set when nothing needs the depth tile
status (`backend.rs:1517-1518`). So no TS flush or resolve sits between the
depth pass and the sample. Mesa needs none on this part either, because it
samples through the sampler TS (point 4). One real exception: a plane that
holds **a whole clear only** has no memory behind it (the `fill` path: the
value is kept in `Plane::cleared_value`). That surface's words are not
written. It must answer "not sampleable" (the CPU resolve, which is cheap for
a constant, stays for it).

### The board experiment that decides the verdict [2026-11-07, replaces the first version's]

*First version:* "render the ramp of gl_test 169 into a depth texture, bind
the surface with the table above, and read it back through a trivial fragment
shader that writes the rebuilt depth to a colour target. Pass: every texel
equals `unorm24_depth` of the words, including the extremes `0x000000ff`,
`0xffffff00`, `0x80000000`. Also try the swizzle with the stencil kept
(A8R8G8B8 with an identity swizzle) once [...]." Replaced by one case that
tries Mesa's format first, keeps the 8888 view as its second arm, and compares
against the CPU resolve of the same words, so it does not depend on how the PE
rounds the depth.

**Case `dtpu`** (gl_test, number assigned by item gt-depth-tpu; GC7000 only,
SKIP elsewhere by name). It needs one test knob, in the pattern of
`gc_debug_set_narrow_la` (`backend.rs:430-441`, `test-knobs` feature only,
`VIVANTE_NOTES.md` "The debug switches and the test knobs"):
`gc_debug_set_depth_tpu(mode)`. `mode` 0 is today's CPU resolve. With 1 or 2,
`samples_rendered_depth` answers true for a Live, unscaled plane covering the
texture, and `build_descriptor` points the texture at the surface (table
above): 1 is route 2 (`D24X8`, swizzle R, R, R, 1, `INT_FILTER` off) and 2 is
the raw 8888 view (`A8R8G8B8`, **identity** swizzle, `INT_FILTER` on), both
LINEAR, NEAREST, CLAMP_TO_EDGE, LOD 0 = surface + guard,
`LINEAR_STRIDE` = surface stride. The knob is the only code; it is not in the
product archive (the `tools/check.sh` row "no test knob in the artefact").

**[2026-11-08, item gt-depth-tpu: built, not run.]** The case is **gl_test
222 `dtpu`** (`driver/tools/pvrlab/gl_test.c`, `driver/tools/pvrlab/README.md`
"Case 222"). The switch is `src/vivante/depth_sample.rs`: `gc_debug_set_depth_tpu`
and the getter `gc_debug_depth_tpu` (both in `tools/check.sh`'s
`KNOB_VIVANTE_SYMBOLS`), `gc_depth_sampled` as the GC7000 arm of
`backend::samples_rendered_depth`, and `descriptor`, which `texture.rs`
`build_descriptor` writes in place of the texture's own words when the
binding carries `depth_id`; a surface that is gone fails the draw with
`-ENOTSUP`, named. One departure from the paragraph above: route 2's swizzle
is R, R, R, 1 only under GL_OES_depth_texture (`depth_luminance`); on the ES
3.0 context the images run (`CONFIG_YAKOGL_REPORT_ES2=n`) it is **R, 0, 0, 1**,
the (d, 0, 0, 1) of point 6, so nothing reads what the TPU puts in Y, Z, W.
The host test `tests/gc_depth_sample.rs` is section (d)'s steps 2, 3 and 6
(the answer, the descriptor's every field against the table, no wait and no
CPU read of the surface under either view, the CPU read under mode 0); step 4
has no model (route 2) and step 5's recycled surface is tested at the
`descriptor` level (`-ENOTSUP`). Checks 1-7 below are the case's, plus a
check 8, no GL error after the passes. The X2 image is a sweep from case 177
(`IMX8MP_BOARD=x2 tools/cts/imx8mp-gltest-build.sh dtpu-<commit> 177`,
`SEL=45` runs 222 alone); a later board item runs it.

Steps, at the window size `TW` = `TH` = 64 (`gl_test.c:956-957`), so one
texel is one pixel:

1. A 64x64 `GL_DEPTH_COMPONENT`/`GL_UNSIGNED_INT` texture (case 177's,
   `depth_target_format`), NEAREST, CLAMP_TO_EDGE, the only attachment of an
   FBO. Check 1: the programs linked. Check 2: no GL error and the status is
   `GL_FRAMEBUFFER_COMPLETE`.
2. Clear depth to 1.0, depth test LESS, and draw case 177's ramp
   (`zonly_ramp`). Then `glFinish`. The measured words of this ramp
   (`ds_cache.rs:550-551`, gl_test 169) are depth `(x + y + 1) / 128` at GL
   pixel (x, y), i.e. `n = x + y + 1` in 1..127.
3. **P0, the reference.** Knob 0, then sample the texture over the window
   with the decoding shader below and read the window back (`read_topdown`,
   kept as an array). This is the current CPU resolve of the same words.
4. **P1, route 2.** Knob 1, the same draw and read-back.
5. **P2, the raw view.** Knob 2, sampled with an identity shader
   (`gl_FragColor = texture2D(u_tex, v_uv)`) and read back.
6. **P3, coherence.** Knob 1, clear depth to 1.0 and draw the ramp again
   with every vertex z raised by 0.5 (`-0.5, 0.5, 0.5, 0.5, 0.5, 1.5`), so
   the window depth is 0.25 higher. The part beyond the far plane is clipped
   and keeps the clear, 1.0. Sample with **no `glFinish`** before the
   sampling draw (glmark2's frame), then read back. This shows whether the depth cache's writes reach the texture
   cache without a wait (risk "Cache coherence" below).

The decoding shader (highp; the in-process compiler, as case 169 uses,
`prj_sweep.conf`):

```glsl
precision highp float;
uniform sampler2D u_tex;
varying vec2 v_uv;
void main() {
    float k = floor(texture2D(u_tex, v_uv).r * 16777215.0 + 0.5);
    float hi = floor(k / 65536.0);
    float mid = floor((k - hi * 65536.0) / 256.0);
    float lo = k - hi * 65536.0 - mid * 256.0;
    gl_FragColor = vec4(hi, mid, lo, 255.0) / 255.0;
}
```

**Expected pixels.** At GL pixel (x, y), with `n = x + y + 1`, the 24-bit
depth word is `k = n * 2^17` or `n * 2^17 - 1`. The exact value of
`n * 16777215 / 128` is `n * 131072 - n/128`, and the PE's rounding picks
one or the other. The words of 2026-09-30 were not kept per bit, so the case
does not assume which. P2 reads the word's bytes exactly, so it is one of the
two rows below. P0 and P1 go through the decoding shader, whose fp32
`f * 16777215` can miss `k` by up to 2 near 2^24 (two roundings of relative
2^-24 each, on top of the CPU's own f32). So P0's decoded
`kd = hi*65536 + mid*256 + lo` is **within 2 of** a row's `k`, not
necessarily equal to it:

| k | P0 pixel (R, G, B, A), if the decode is exact | P2 pixel (R, G, B, A) = (D1, D0, S, D2) |
|---|---|---|
| `n * 2^17` | (2n, 0, 0, 255) | (0, 0, S, 2n) |
| `n * 2^17 - 1` | (2n - 1, 255, 255, 255) | (255, 255, S, 2n - 1) |

For example, GL (0, 0) has n = 1: R = 2 or 1. GL (31, 32) has n = 64:
R = 128 or 127. GL (63, 63) has n = 127: R = 254 or 253. S is the stencil
byte the depth-only clear left (logged, not checked).

Checks, each a numbered CASE record (check, got, want) so a board with no
console reads the result:

- check 3: P0 at every pixel has `|kd - n * 2^17| <= 3` (either row, plus the
  decode's 2). This is the reference's sanity check, today's path.
- check 4 (**the verdict for route 2**): **P1 == P0 at all 4096 pixels**,
  bit for bit. The same shader decodes both, so equal inputs give equal
  pixels whatever its rounding: P1 == P0 means the TPU's D24X8 returned the
  CPU resolve's f32 at every texel. Got = the first differing pixel's index
  and value.
- check 5: if check 4 failed, P1's `kd` within 2^8 of P0's everywhere, so
  the top 16 bits agree ("D24X8 reads the surface but returns fewer than 24
  bits", which is still usable for glmark2's 0.1505 bias, not for bit-exact
  dEQP).
- check 6 (route 1's precondition): P2's bytes are one of the two rows above
  (`P2.A*65536 + P2.R*256 + P2.G` = `n * 2^17` or `n * 2^17 - 1`) and within
  2 of P0's `kd`. This also proves the byte order of route 1 and gives the
  exact words the PE wrote, recorded for the first differing pixel of
  check 4.
- check 7: P3 against the same ramp offset by 0.25: `n' = n + 32`, with
  `|kd - n' * 2^17| <= 3` where `n' < 128`, and depth 1.0 (`kd` = 0xffffff,
  within 3; pixel (255, 255, 255, 255)) where `n' >= 128`.

**Verdict table.**

| outcome | meaning | next step |
|---|---|---|
| check 4 passes | the TPU samples the row-linear D24S8 surface as Mesa's D24X8, exact | implement route 2 (section (b)); no shaderc change |
| check 4 fails, 5 passes | D24X8 works on the surface but with less than 24 bits | route 2 for glmark2; keep the CPU resolve where exact depth is read (dEQP) |
| checks 4 and 5 fail, 6 passes | D24X8 does not read a linear surface; the bytes are addressable | route 1, or render depth SUPER_TILED to match Mesa (`gc-msaa-addresses.md` row (E): not tried under SINGLE_BUFFER 1) |
| check 6 fails too | the LINEAR descriptor does not address the surface | verdict no; the CPU resolve stays |
| check 7 fails while 4 passes | stale texture cache | flush the texture cache after a depth pass before sampling (risk below) |

## (b) What would change in the GC back end

Mirror of the V3D: `v3d_depth_sampled`, `depth_buffer_state`/`depth_sample`
in `src/v3d/backend.rs`, `GlesTextureBind::depth_id`. **[2026-11-07]** With
route 2, item 4's shader work goes away. The list is otherwise unchanged and
keeps its first-version line numbers where they are not marked.

1. **The answer.** `backend::samples_rendered_depth` (`src/backend.rs:1858`)
   has V3D and PowerVR arms only. Add a `gpu-vivante` arm calling a new
   `vivante::backend::gc_depth_sampled(id, w, h)`. The GL core's caller
   (`gl/texture.rs`, `depth_for_draw`) needs no change: it already asks,
   submits the kick (`flush`, no wait) and puts `depth_id` into the binding.
   The comments that say "not tried on either board" are then updated.
2. **A lookup in the cache.** New `ds_cache::sampleable(id, w, h) ->
   Option<(gpu_addr, stride)>`, next to `resolve_depth` (`ds_cache.rs:562`),
   which finds the surface the same way (`mine(...)`) and answers
   `Some` only when: the depth plane is **Live** (not
   `cleared_value()`, not parked), `scale == 1` (the `mine` filter), the key's
   size covers `w` x `h`, and `stride >= w * 4`. It reports the surface's GPU
   address plus `GC_DEPTH_GUARD_BYTES` (resolve reads `all[guard..]`)
   and `Surface::stride`. It must not wait: pending kicks are fine,
   they are ordered before the draw in the same ring (the GL core flushed
   first); only the CPU read of `resolve_depth` needs `finish`.
3. **The descriptor.** `build_descriptor` (`vivante/texture.rs:306`) and
   `emit_slot` (`:480`): when `t.depth_id != 0`, take the surface from
   `ds_cache::sampleable` and write the table above instead of the texture's
   own `base_gpu` (`:431`), stride and format (`format_of`). **[2026-11-07]**
   For route 2 the `lum`/`depth_luminance` swizzle (`:288-295`) applies as it
   is (depth is in X, as Mesa core's (X, X, X, 1) assumes), and `INT_FILTER`
   must be off for the depth view (`F:320`), which `int_filter_wanted`
   (`texture.rs:464`) has to learn. *First version:* "must be reconciled: the OES_depth_texture
   luminance mapping becomes a second stage after the rebuild" (route 1
   only). A refusal (`sampleable` -> `None`) has to be visible to the draw:
   the GL core already left the texels stale on the strength of the answer,
   so a refusal at descriptor time must fail the draw with a named log (as
   `depth_sample` does for the V3D), not sample a stale copy.
4. **The shader.** **[2026-11-07] Route 2 needs none**: `texture2D(s, uv).r`
   is a plain sample of a D24X8 view, and a shadow sampler uses the TE
   compare (`gc-shadow-compare.md` section 4, "(c) The change", owns that). *Route 1 only*
   (first version's text): it needs the rebuild in the fragment (and vertex)
   program that samples the texture. The shader table (`src/shaderc`, the
   online compiler, `docs/online-shaderc/`) compiles the app's
   `texture2D(shadowmap, uv).r` as a plain sample. A variant key bit "this
   sampler is a packed depth view" must lower the sample to the
   three-channel sample plus the rebuild above. That is the largest part,
   and it is in `src/shaderc`. Alternative without shader work: **a GPU
   resolve pass**, a full-target quad (the mechanism of `vivante/mipmap.rs`,
   `GC_INTERNAL_MIPGEN_PS` in `vivante/internal.rs`) that samples the surface
   and writes the depth into the texture's R32F storage (needs R32F as a PE
   colour target: unmeasured). It keeps `depth_id == 0`, so shaderc and
   `build_descriptor` are untouched, drops the CPU loop and the `finish`, and
   costs one full-screen quad. **[2026-11-07]** With route 2 that resolve
   pass is also simpler (a D24X8 view, a plain `.r` sample), but direct
   sampling makes it unnecessary. It stays the fallback only if check 7
   (coherence) or a later refusal case needs a copy.
5. **Mirror of the V3D's bookkeeping.** `GlesTextureBind::depth_id` reaches the
   backend as `abi_layout.rs` (offset 52); `abi_layout` and `backend.h`
   need no edit. `depth_resolve: Some(gc_depth_resolve)` in the backend table
   stays: the CPU readers still resolve (below).
6. **Keep alive.** The depth surface must not be recycled (`victim` in
   `ds_cache.rs`) while a later draw samples it: `used_by` already
   records the latest kick that used it, and `victim` skips surfaces with
   live planes, so nothing new is needed for a live plane.

## (c) Risks

- **Fast clear / tile status.** None on this core (above); the exception is the
  cleared-only plane with no memory. If a later change adds RS clears or a
  tile status to depth, `sampleable` must refuse until a TS resolve ran, or
  bind the TS to the sampler as Mesa does (`T:79-159`, point 4 of (a)).
- **Linear depth, a path Mesa never takes [2026-11-07].** Mesa samples depth
  only SUPER_TILED (`R:572-576`, `S:416`). A TPU that reads D24X8 only from a
  tiled layout fails check 4 and passes check 6. The answer is then route 1 or
  a supertiled depth surface (verdict table).
- **Cache coherence.** The PE writes depth through the depth cache; the TPU
  reads through the texture cache. The back end flushes
  `COLOR|DEPTH|TEXTURE|SHADER_L1` before each draw (`VIVANTE_NOTES.md:1200`),
  which covers a depth pass followed by a sampling draw in one submit; a
  descriptor INVALIDATE follows the address writes.
  Not measured for depth specifically: check 7 of the board case measures it.
- **Precision.** **[2026-11-07]** Route 2: the precision of the D24X8 X
  channel is not found in Mesa. Check 4 compares it bit for bit with the CPU
  value, and check 5 grades a near miss. Route 1 (first version): exact in
  fp32 *if* the TPU returns `k/255` correctly rounded; a TPU that returns a
  truncated or approximate value breaks the integer recovery by one in the
  low byte. The fix is `round`. The CPU path's 24-bit unorm to f32
  (`k / 16777215` as f64 then f32) is the reference; a rebuild in fp32
  divides by a float, one rounding, equal to it for every k (checked
  exhaustively by the unorm24 item, not here).
- **Filtering.** LINEAR on route 1's three bytes is wrong (carry), so route 1
  forces NEAREST. Route 2 filters a real depth value, so LINEAR is allowed
  (Mesa does not restrict MIN/MAG for depth, only the mip filter,
  `D:324-328`). *First version:* "glmark2 shadow uses `GL_NEAREST`?
  unchecked". **[2026-11-07] Checked**: both `shadow` and `refract` set
  `GL_NEAREST` min and mag on the depth texture
  (`src/scene-shadow.cpp:103-104`, `src/scene-refract.cpp:195-196`, glmark2
  2023.01).
- **Stencil byte.** Part of the same word. Route 2 ignores it: D24X8 is the
  depth view, and the stencil view is the separate `EXT_D24S8` +
  `DEPTH_STENCIL_MODE_STENCIL` (`F:183-184`, `D:254-258`). Route 1's swizzle
  drops it. A stencil texture view is not this path.
- **The GL core's CPU readers of the texels.** With sampling in place the
  texels are *stale* until a CPU access. Every reader already goes through
  `settle_texture`/`resolve_depth` (as for the V3D): glReadPixels of the
  depth, glCopyTexSubImage, glBlit, glGenerateMipmap of the texture, a
  glTexSubImage that keeps the rendered rest, tf capture. They call
  `backend::depth_resolve` (`gc_depth_resolve`) and must keep working on the
  same surface, so the surface may not be reclaimed between the sampling
  draw and a later CPU read. `ds_cache::gone` frees a surface only when
  nothing holds a live plane, which a sampled one does until its object is
  deleted.
- **Size.** A surface is `align64` in both directions and one cache entry per
  (depth, stencil) pair; a texture whose key does not match (a stencil
  attachment that differs) is a different surface: `sampleable` answers none.
- **Multisample / scale != 1:** refused by the `mine` filter.
- **Shader cost.** Route 2: none (a plain sample). Route 1: 3 sample channels
  and a few ALU ops per depth lookup; shadow's fragment shader samples once.
  Against 2 ms of CPU resolve a frame on the host and a 7.68 MB write, not a
  regression.

## (d) Draft host test (for the fix item)

`tests/gc_depth_sample.rs`, with the stand-in OSAL and the submit model of
`tests/gc_mipmap_gpu.rs` (host memory per allocation, `submit` decoding each
draw's LOAD_STATEs). Gate:

```text
#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]
```

Steps:

1. Init the real back end (`gc_init`, `yakogl_backend_vivante`) over the
   stand-in OSAL. Draw once into a depth texture's pair through
   `GlesDraw { depth_id: id, ... }` with a depth attachment (as
   `pair_draw`/`ds_key` of `backend_tests.rs` do), and fill the surface's
   words through `ds_cache::memory_of(key)` with the pattern of
   `tests/gc_depth_resolve.rs`: xorshift words plus `0x000000ff`,
   `0xffffff00`, `0x80000000`, at the surface's own stride.
2. **The answer.** `gc_depth_sampled(id, w, h)` is true for a Live plane of
   that size and false for: another id, a larger size, an
   `id` whose plane is a whole clear, and a parked clear. With no GPU
   backend the shared `backend::samples_rendered_depth` stays false.
3. **The descriptor.** Draw with a binding `GlesTextureBind { depth_id: id,
   width: w, height: h, ..}` (as `gl::draw::texture_bind_2d` builds one) and,
   in the OSAL's `submit`, decode the texture descriptor
   (`GC_TEXDESC_CONFIG0`, `CONFIG1`, `LINEAR_STRIDE`, `SIZE`, LOD 0 address)
   and assert: **[2026-11-07]** format 0x11 (`D24X8`) with `FORMAT_EXT` 0
   for route 2 (first version: 0x07 with the route 1 swizzle), LINEAR
   addressing, the swizzle of the table above, `INT_FILTER` clear,
   `LINEAR_STRIDE == ds_cache::stride_of(key)`, `SIZE == (w, h)`, LOD 0
   address `== surface gpu + GC_DEPTH_GUARD_BYTES`, one level.
4. **The values.** Route 1 only: the model of the fragment program (as
   `gc_mipmap_gpu.rs` runs the mipgen quad) reads each texel of the surface
   through the descriptor and rebuilds `(a*65536 + r*256 + g) / 16777215`
   with the bytes from the model's unorm8 read (`c/255` rounded back); assert
   it equals `gc_depth_resolve`'s output for the same surface, **bit for
   bit** at every texel, for `w` x `h` of 64x64, 90x70 (padding), and 1x1.
   **[2026-11-07]** Route 2 has no model of the TPU's D24X8 output: the host
   test stops at the descriptor, and the board case is the value check.
5. **Refusals.** The draw after the surface is recycled (cycle
   `ds_cache::ENTRIES` other pairs, as `depth_resolve_reads_the_linear_depth_as_floats`
   does) fails with `-ENOTSUP`/logged, and samples nothing stale.
6. **No wait.** Count the OSAL's `finish`/wait calls across
   `samples_rendered_depth` + the draw: zero (the CPU resolve's `finish` is
   what this removes).

What it cannot show: whether the texture unit returns what the model returns.
That is the board case at the end of (a).
