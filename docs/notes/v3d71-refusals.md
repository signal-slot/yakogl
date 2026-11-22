<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 7.1: every named refusal of the 7.1 path, and the gl_test cases that meet it

One list for the board session of P9 (`docs/notes/v3d71-gl-table.md`
section 5) to report "refused by name" against: the refusals of the 7.1
host modules `src/v3d/*71.rs`, of the C builders
`driver/src/v3d/v3d_job71.c`, `v3d_tex71.c` and `v3d_tfu71.c`, and of the
7.1 online compiler. The 4.2 table's own refusals are
`docs/notes/v3d-refusals.md` and are not repeated; the three the six cases
can meet through the shared GL core are in section 7.

All `file:line` were read on the tip `943f2d1f`. A line is the variant's
definition (or the condition's `if`); the message text is at the `message`
/ `Display` function named in the layer's header. "Case" is the gl_test
case that meets the refusal (5 scissor, 58 clear with scissor, 59 clear
with write masks, 66 clear not bounded by viewport, 67 FBO limits, 102
clear under a partial colour mask; texture cases 2 texture, 115 cube map,
118 mipmaps); "none" means no case of the six, nor of 2, 115, 118, reaches
it, and it is there for the GL core's other callers. Which case meets
which refusal was read from the conditions and from
`docs/notes/v3d71-gl-table.md` section 5; nothing here is a board result.

Each `src/v3d/*71.rs` enum is `Refusal`, `Refusal71` or (zskeep71)
`ZsRefusal71`.

## 1. Hub identity: ident71.rs, admit71.rs

Message: `ident71.rs:37` (`Display`). `admit71.rs` re-tags the module's
refusals as `Refusal { module, reason }` (`admit71.rs:25`), so a board
line reads `ident71: ...`, `rt71: ...`, `attr71: ...`, `record71: ...`,
`format71: <Debug of the variant>`, `admit71: ...` or `limits71: ...`.

| Refusal | Where | Condition | GL feature | Case |
|---|---|---|---|---|
| `Refusal::V3d42` "V3D 4.2: build gpu-v3d" | `ident71.rs:30` (fires `:61`) | HUB_IDENT1 version 42 | the other table serves it | none (a 4.2 board built with the 7.1 table) |
| `Refusal::OldIprev` "V3D 7.1.N: this table is 7.1.10 (BCM2712 D0) only" | `ident71.rs:32` (`:60`) | version 71 with IPREV < 10 | BCM2712 C0 and earlier | none (the Pi 5 is 7.1.10) |
| `Refusal::UnknownVersion` | `ident71.rs:34` (`:62`) | neither 42 nor 71 | | none |

## 2. Frame, targets, tile size: rt71, frame71, limits71, admit71, clearplan71

| Refusal | Where | Condition | GL feature | Case |
|---|---|---|---|---|
| rt71 `FormatNotRenderable` "no render-target row in Mesa's table" | `rt71.rs:144` (msg `:163`) | SNORM, RGB9_E5, sRGB8 without alpha, RGB8I and the other 3-channel integers, 16-bit normalised | colour attachment format | none (the six use RGBA8; 67's GL_RGB frame is stored as RGBA8 by the GL core, section 7) |
| rt71 `NotAColourFormat` | `rt71.rs:146` | a depth/stencil enum given as a colour target. `admit_frame` uses it as the *pass* signal for the depth/stencil format (`admit71.rs:137`) | | none |
| rt71 `UnknownFormat` | `rt71.rs:148` | a GL enum the table does not know | | none |
| rt71 `TooManyTargets` "more than 8 render targets" | `rt71.rs:150` (`check_targets` `:237`) | > 8 colour targets | `glDrawBuffers` / MRT | none (the six use 1) |
| rt71 `TargetTooLarge` "larger than 7680 px" | `rt71.rs:152` (`check_size` `:246`) | side > 7680 | framebuffer size | none |
| rt71 `EmptyTarget` | `rt71.rs:154` | side 0 | | none |
| rt71 `ClearKindMismatch` | `rt71.rs:157` | float clear colour on an integer target or the reverse | `glClearBuffer{f,i,ui}v` | none |
| rt71 `TargetNumberOutOfRange` | `rt71.rs:159` | render target number >= 8 | | none |
| admit71 "a colour format is not a depth or stencil format" | `admit71.rs:140-142` | the depth/stencil attachment has a colour format | | none |
| limits71 "no tile size fits N colour bytes per pixel" | `admit71.rs:147-150` (`choose_tile_size71_total`, `limits71.rs:101`) | total colour bytes per pixel beyond the 7.1 TLB's smallest (8x8) tile | 8 targets of 128 bpp | none |
| frame71 `EmptyFrame` | `frame71.rs:273` (msg `:295`) | frame width or height 0 | | none |
| frame71 `FrameTooLarge` | `frame71.rs:275` | a side over 7680 | | none |
| frame71 `TooManyRenderTargets` | `frame71.rs:277` | > 8 | | none |
| frame71 `TileSize` | `frame71.rs:279` | a tile side not 8, 16, 32, 64 | | none |
| frame71 `MsaaDoubleBuffer` | `frame71.rs:281` | double-buffer with 4x MSAA | | none |
| frame71 `DepthKind` | `frame71.rs:283` | depth type and the depth/stencil attachments disagree | | none (a defect of the caller if it fires) |
| frame71 `LoadRenderTarget` | `frame71.rs:285` | `Load Tile Buffer General` of a render target above 3 | | none |
| frame71 `FieldRange(name)` | `frame71.rs:287` (fires `:315`, `put`) | a packet field over its width | | none |
| frame71 `Unaligned` | `frame71.rs:289` | an address not 64-byte aligned | | none |
| frame71 `WrongPacketField` | `frame71.rs:291` | a Load asked for a dither mode, a Store for force-alpha | | none |
| clearplan71 `TooManyTargets{n}` | `clearplan71.rs:127` (msg `:145-150`) | > 8 targets | | none |
| clearplan71 `FramebufferSize{w,h}` | `clearplan71.rs:130` | 0 or > 7680 a side | | none |
| clearplan71 `TargetOutOfRange{index,n}` | `clearplan71.rs:135` | a clear of a target the framebuffer lacks | `glClearBufferfv(GL_COLOR, i)` | none |
| clearplan71 `NegativeScissor{w,h}` | `clearplan71.rs:139` | a negative scissor side | `glScissor` (GL gives an error first) | none for 5, 58 and 66 (their scissors are positive) |

## 3. Draw state, primitives, attributes, programs, depth/stencil

| Refusal | Where | Condition | GL feature | Case |
|---|---|---|---|---|
| draw71 `TargetSize` "a render target side outside 1 to 7680 pixels" | `draw71.rs:481` (msg `:515`, fires `:1289`) | | | none |
| draw71 `TooManyTargets` | `draw71.rs:482` (`:1292`) | > 8 colour targets | | none |
| draw71 `Multisample` "no 4x oversample state yet" | `draw71.rs:483` (`:1295`) | 4x MSAA | multisampled FBO | none |
| draw71 `UnknownCompare` | `draw71.rs:484` | not a GL compare enum | `glDepthFunc`, `glStencilFunc` | none |
| draw71 `UnknownBlendFactor` | `draw71.rs:485` | not an ES 3.0 factor | `glBlendFunc` | none |
| draw71 `UnknownBlendEquation` | `draw71.rs:486` | not an ES 3.0 equation | `glBlendEquation` | none |
| draw71 `UnknownStencilOp` | `draw71.rs:487` | | `glStencilOp` | none |
| draw71 `BlendReadsMissingAlpha` | `draw71.rs:488` | a destination-alpha factor on a target without alpha | blend into RGB | none |
| draw71 `ViewportNotFinite` | `draw71.rs:489` | non-finite or negative viewport | `glViewport` | none (case 66 uses a finite viewport) |
| draw71 `DepthScaleTooSmall` "Z scale under 0.01" | `draw71.rs:490` | |Z scale| < 0.01; needs the no-guardband clipper packet | `glDepthRangef(a, a)` | none |
| draw71 `ClipWindowRange` | `draw71.rs:491` | clip window outside 0..65535 | scissor | none (5, 58, 66 stay inside) |
| draw71 `ViewportOffsetRange` | `draw71.rs:492` | viewport offset outside the packet's range | | none |
| draw71 `OffsetNotFinite` | `draw71.rs:493` | polygon offset NaN/inf | `glPolygonOffset` | none |
| prim71 `Mode(m)` | `prim71.rs:141` (Display `:158`) | mode outside POINTS..TRIANGLE_FAN | `glDrawArrays` mode | none |
| prim71 `IndexType(t)` | `prim71.rs:143` | not UNSIGNED_BYTE/SHORT/INT | `glDrawElements` | none |
| prim71 `ZeroCount` | `prim71.rs:145` | count 0 (the caller skips the draw) | | none |
| prim71 `ZeroInstances` | `prim71.rs:147` | instance count 0 | | none |
| prim71 `Overflow{field,value,bits}` | `prim71.rs:149` | a value over its field | | none |
| prim71 `BaseVertex(v)` | `prim71.rs:151` | outside signed 32 bits | | none |
| prim71 `IndexRange{end,size}` | `prim71.rs:153` | indices run past the buffer | | none |
| prim71 `IndexAlign{offset,index_bytes}` | `prim71.rs:155` | misaligned index offset | | none |
| attr71 `ConstantAttribute` "no 16-byte slot ... (no default-attribute BO)" | `attr71.rs:112` (msg `:139`) | a disabled attribute the program reads and no `constant_addr` was given | `glVertexAttrib4f` generic values (P9: no default-attribute BO) | none: the six programs read `a_position`/`a_color`/`a_uv` as enabled arrays; fires only if a case leaves one disabled |
| attr71 `Type(t)` | `attr71.rs:114` | GL type with no 7.1 type | | none |
| attr71 `Components(n)` | `attr71.rs:116` | not 1..4 | | none |
| attr71 `PackedComponents` | `attr71.rs:118` | 2_10_10_10 with other than 4 | | none |
| attr71 `IntegerType` | `attr71.rs:121` | `glVertexAttribIPointer` with float/half/fixed, or integer with normalized | | none |
| attr71 `TooManyAttributes(n)` | `attr71.rs:123` | > 16 | | none |
| attr71 `Stride(s)` | `attr71.rs:125` | above 32 bits | | none |
| attr71 `Divisor(d)` | `attr71.rs:127` | above 16 bits | `glVertexAttribDivisor` | none |
| attr71 `Reads(n)` | `attr71.rs:129` | > 15 values of one attribute | | none |
| attr71 `ReadWithoutRecord(i)` | `attr71.rs:131` | a program reads an attribute index with no record | | none (the builder's twin is `v3d_job71.c:609-611`, section 5) |
| attr71 `Address` | `attr71.rs:133` | buffer address + offset past 32 bits | | none |
| attr71 `SeparateSegments` | `attr71.rs:135` | non-zero separate VPM input size | | none |
| record71 `NoDrawIndexRecord{ver,rev}` | `record71.rs:397` (msg `:413`) | not 7.1.10 or later: the D0 record does not exist | the 7.1.10 draw-index record | none (the Pi 5 is 7.1.10) |
| record71 `CodeAddressUnaligned(stage)` | `record71.rs:399` | code address not a multiple of 8 | | none |
| record71 `BadThreadCount(stage,n)` | `record71.rs:401` | threads not 1, 2, 4 | | none |
| record71 `VpmFieldRange(stage,name)` | `record71.rs:403` (fires `check_vertex`) | VPM sizes or in-play counts outside the 4-bit fields (inputs 1..=16) | | none |
| record71 `TooManyVaryings(n)` | `record71.rs:405` | > 255 | | none |
| record71 `BaseVertex(stage)` | `record71.rs:407` | a vertex-side program reads `gl_BaseVertex` | | none |
| record71 `DrawIndex(stage)` | `record71.rs:409` | a vertex-side program reads `gl_DrawID` | | none |
| zskeep71 `ReadLost(Depth)` "the draw reads depth an earlier job wrote, and the target has no depth/stencil buffer to keep it in" | `zskeep71.rs:463` (msg `:478`) | a draw reads depth an earlier job wrote into a tile-only target (the 4.2 string, word for word) | depth test after a job split | none of the six (no case tests depth) |
| zskeep71 `ReadLost(Stencil)` "the draw reads stencil an earlier job wrote, ..." | `zskeep71.rs:463` | the same for stencil | stencil test after a job split | **59** (and 67 if it reaches the same shape, see section 7) |
| zskeep71 `AbsentPlane(p)` | `zskeep71.rs:466` | a plane used or kept on a framebuffer without it | | none |
| zskeep71 `SplitNeedsBoth` | `zskeep71.rs:468` | split depth/stencil without both planes | | none |
| zskeep71 `EmptyBuffer` | `zskeep71.rs:470` | a combined buffer with neither plane | | none |
| zskeep71 `StoredWithoutBuffer(p)` | `zskeep71.rs:472` | a stored history on a tile-only target | | none |
| zskeep71 `WrittenWithBuffer(p)` | `zskeep71.rs:474` | an unstored history on a target with a buffer | | none |

## 4. Uniform stream: uniform71.rs

Message: `uniform71.rs:90` (`Display`).

| Refusal | Where | Condition | GL feature | Case |
|---|---|---|---|---|
| `UniformIndex{index,count}` | `uniform71.rs:77` | a `UNIFORM` word beyond the stage's block | `glUniform*` | none (the clear program reads dwords 0..3, `ours_clear.h:49`) |
| `TextureSize{contents}` "the 7.1 texture state is not written yet" | `uniform71.rs:79` (fires `word`) | textureSize words (width/height/depth/array size) | sampler programs | **2, 115, 118** until texture71's size words are wired (`texuni71`) |
| `TmuConfig{contents}` | `uniform71.rs:81` | TMU config words P0/P1 | sampler programs | **2, 115, 118**, same |
| `FragCoordY{contents}` | `uniform71.rs:83` | `gl_FragCoord.y` / `gl_PointCoord.y` window words | `gl_FragCoord` | none (neither `color_uv` nor `clear` reads it) |
| `Unknown{contents}` | `uniform71.rs:85` | any other contents | | none |
| `OutputTooSmall{need,have}` | `uniform71.rs:87` | the output slice is short | | none |

## 5. C builders

`driver/src/v3d/v3d_job71.c`. A refusal is `V3D_JOB71_REFUSED` (`v3d_job71.h:264`)
or `v3d_job71_begin` / `v3d_job71_end` returning false with `broken` or a
list's `bad_field` set; the draw leaves the lists as they were
(`v3d_job71.c:790-801`). The 18 rows below are the refusals
`tools/v3d/job71_check.c` `refused()` (`:1317`, `REFUSALS 18` `:1315`)
drives; `job71.sh` part (d) prints "refusals 18 of 18 refused".

| # | Check (job71_check `case`) | Where | Condition | GL feature | Case |
|---|---|---|---|---|---|
| 0 | width 0 | `v3d_job71.c:193` (`target_ok`) | width or height 0 | | none |
| 1, 2 | width / height 7681 | `:193` | > `V3D_JOB71_MAX_SIZE` | | none |
| 3 | 9 render targets | `:194` | rts > 8 | | none |
| 4 | depth format as a colour RT | `:201-203` (`rt71_internal` default `:175`) | an RT format with no 7.1 internal type | | none |
| 5 | S8 as a depth buffer | `:205-210` | `zs` not D24S8, D32F or D16 | | none |
| 6 | Z/S store without a Z/S buffer | `v3d_job71_end` `:976-983` | load/store of a buffer the target lacks | | none |
| 7 | a TMU configuration uniform | `write_uniforms` `:349-353` (default) | contents 40/41 | texture state | **2, 115, 118** |
| 8 | viewport Z scale 0 | `:480-483` | |Z scale| < 0.01 | | none |
| 9 | separate VPM segments | `vs_ok71` `:590-593` via `draw_is_supported` `:599` | | | none |
| 10 | a CS reading the base instance | `vs_ok71` (`uses_biid`) | | `gl_BaseInstance` | none |
| 11 | 3 FS threads | `:601` (`threads_ok`) | | | none |
| 12 | the VS reads attribute 2 with no record | `:609-611` | | | none |
| 13 | a `QUNIFORM_UNIFORM` with no storage | `:303-307` | | `glUniform*` | none |
| 14 | implicit point varyings on a triangle | `:630-633` | | `gl_PointCoord` on a non-point | none |
| 15 | lists of 64 bytes | `:289-292`, `:794-800` (FULL, not REFUSED) | list too small | | none |
| 16 | index type past 32-bit | `:615` | | `glDrawElements` | none |
| 17 | depth type past DEPTH_16 | `:196-197` | | | none |

Refusals of the builder the 18 do not drive (all in `v3d_job71.c`):
`draw_is_supported` `:596-617` (no state/programs `:599`, more than 64 FS
inputs `:603`, primitive past TRIANGLE_FAN or count 0 `:605`, more than
`V3D_MAX_ATTRS` attributes `:607`, components not 1..4 `:613`);
`to_pixel` `:66-70` and `:423-427` (a clip coordinate outside [0, 65536) or
NaN: scissors beyond 16 bits); the viewport offset `:510-512`;
`TEXTURE_*` size uniforms with the binding not given `:334-337`;
`FB_WPOS_*`/`FB_PNTC_*` words `v3d_wpos_uniform` false `:344-347`;
`v3d_job71_end`: an aspect the buffer does not hold `:984-991` (the 6
refusals of `job71_check g1`, `:1259-1306`) and a list overflow `:1112-1115`.
The header's own list is `v3d_job71.h:42-50` (4x multisampling, Z scale,
TMU config uniforms, separate segments, base instance, transform feedback,
primitive restart, a base vertex, early Z off).

`driver/src/v3d/v3d_tex71.c`: `v3d_tex71_pack_state` `:93-106` (false for an
unknown type, a base not 64-byte aligned, a transfer function not none/sRGB,
a size of 0 or above 16383, `max_level` > 15, `base_level` > `max_level`,
a pad > 15, an array stride of 24 bits or more, a swizzle past alpha);
`v3d_tex71_pack_sampler` `:168-184` (a wrap mode past mirror-once, a border
mode not known, compare function > 7, a NaN LOD limit or bias, a border
colour given with a mode that ignores it, a bias outside s8.8);
`v3d_tex71_p2` `:240-266` (an op > 15, a gather component or sample number
> 3, an offset outside -8..7, a failed pack round trip). Cases: **2, 115,
118** reach none of them in a normal shape; they are the checks that stop a
texture71/texfill71 value the Rust side let through.

`driver/src/v3d/v3d_tfu71.c`: `v3d_tfu71_supports` false `:74` (a texture
type that has no TFU form, and RGB9_E5/R32F/RG32F/RGBA32F only when not for
mipmaps `:66-71`); `v3d_tfu71_pack` `:122-153`: bytes per texel not a
utile size `:122`, width/height 0 or past `V3D_TFU71_MAX_DIM` `:126-128`,
unsupported type `:131`, a raster destination for mipmaps `:135-136`
("TFU requires UIF when generating mipmaps"), more levels than the larger
side halves `:144`, a stride that does not describe the image
`side_stride` `:99-116`, an IOC stride over 16 bits `:152`. Case **118**
(`glGenerateMipmap`) is the only one that reaches them.

## 6. Texture and TFU planning: format71, texstate71, texture71, sampler71, texfill71, tfuplan71

| Refusal | Where | Condition | GL feature | Case |
|---|---|---|---|---|
| format71 `UnknownFormat` | `format71.rs:239` (fires `:461`) | an enum no ES surface offers | | none |
| format71 `UnsizedTypeNotListed` | `format71.rs:242` (`:470`) | unsized format with a type that has no ES meaning | | none |
| format71 `NoThreeChannelRow` | `format71.rs:247` (`:459`) | RGB16F/RGB32F/RGB8I..: Mesa has no 3-channel row; the GL core stores them as 4 channels | | none |
| format71 `StencilTexturing` | `format71.rs:252` (`:460`) | STENCIL_INDEX8 as a texture | | none |
| texture71 `Format(r)` | `texture71.rs:194` (Display `:223-225`) | the format71 refusal | | none (the three cases use RGBA8) |
| texture71 `Compressed{pipe}` | `texture71.rs:197` | a compressed format | `glCompressedTexImage2D` | none |
| texture71 `SeparateStencil{pipe}` | `texture71.rs:200` | a format whose stencil is a separate buffer | | none |
| texture71 `TexelSize{pipe}` | `texture71.rs:202` | no texel size | | none |
| texture71 `SwizzleSource{value}` | `texture71.rs:204` | not a swizzle source | | none |
| texture71 `ZeroSize` | `texture71.rs:206` | width, height or level count 0 | | none |
| texture71 `LevelsBeyondChain{levels,chain}` | `texture71.rs:208` | more levels than the chain | `GL_TEXTURE_MAX_LEVEL` | none (118 stays in the chain) |
| texture71 `BaseLevelMissing` | `texture71.rs:210` | base level past the storage | `GL_TEXTURE_BASE_LEVEL` | none |
| texture71 `BaseAboveMax` | `texture71.rs:212` | base level above max level | | none |
| texture71 `LayoutSize` | `texture71.rs:214` | size past the layout's 14 bits | | none |
| texture71 `LayoutTooLarge` | `texture71.rs:216` | layout past 32 bits of bytes | | none |
| texture71 `AddressOverflow` | `texture71.rs:218` | address + layout past 32 bits | | none |
| texture71 `State(r)` | `texture71.rs:220` | texstate71's | | none |
| texstate71 `NotV3d71{ver}` | `texstate71.rs:38` (msg `:90`) | hub version not 7.1 | | none |
| texstate71 `RevisionBelow715{iprev}` | `texstate71.rs:42` | below 7.1.5 (no R/B swap bit, another stride layout) | | none (7.1.10) |
| texstate71 `ZeroSize` | `texstate71.rs:45` | | | none |
| texstate71 `DimensionTooLarge{dim}` | `texstate71.rs:46` | above 4096 | | none |
| texstate71 `LayersTooMany` | `texstate71.rs:49` | above 2048 layers/depth | | none |
| texstate71 `CubeNotSquare` | `texstate71.rs:53` | | cube map faces | none for 115 (square) |
| texstate71 `CubeFaces{depth}` | `texstate71.rs:55` | layer count not a multiple of 6 | | none |
| texstate71 `LevelOutOfRange` | `texstate71.rs:58` | base level or count out of range | | none |
| texstate71 `ZeroBytesPerTexel` | `texstate71.rs:62` | | | none |
| texstate71 `StrideNotAligned` | `texstate71.rs:64` | not a multiple of 64 bytes | | none |
| texstate71 `StrideTooLarge` | `texstate71.rs:68` | past 24 bits of 64-byte units | | none |
| texstate71 `StrideMissing` | `texstate71.rs:72` | a layered texture with no array stride | cube map: 6 layers | **115** if the caller omits the stride (a defect, not a GL case) |
| texstate71 `BaseNotAligned` | `texstate71.rs:74` | not 64-byte aligned | | none |
| texstate71 `TransferFunc{value}` | `texstate71.rs:79` | other than none or sRGB | | none |
| texstate71 `TmuField{name,value}` | `texstate71.rs:83` | a TMU config field out of range | | none |
| sampler71 `CubeNonSeamless` | `sampler71.rs:176` (msg `:193`, fires `:258`) | a cube map sampled non-seamlessly: 7.1 has no field to turn seamless off | GL ES 2.0 cube sampling the 4.2 emulation keys on the program | **115** only if the GL core asks for non-seamless; ES 3.0 is always seamless (`v3d71-cube-seamless.md`), so none is expected |
| sampler71 `CompareFuncOutOfRange` | `sampler71.rs:178` | compare function above 7 | | none |
| sampler71 `NanLod` / `NanBias` | `sampler71.rs:180`, `:182` | | | none |
| sampler71 `BiasOutOfRange` | `sampler71.rs:185` | outside -128 <= bias < 128 | `GL_TEXTURE_LOD_BIAS` | none |
| sampler71 `AnisotropyOutOfRange` | `sampler71.rs:187` | more than 16 taps | | none |
| sampler71 `NanBorder` | `sampler71.rs:189` | | | none |
| texfill71 `Texture(r)`, `Sampler(r)` | `texfill71.rs:46-47` (Display `:50-54`, built `:124-125`) | the module's refusal, passed through | | the two above |
| tfuplan71 `UnsupportedType` | `tfuplan71.rs:43` (msg `:61`) | `v3d_tfu71_supports(ttype, true)` false | `glGenerateMipmap` of RGB9_E5, R32F, RG32F, RGBA32F | none of 118 (RGBA8) |
| tfuplan71 `BadCpp` | `tfuplan71.rs:45` | bytes per texel not 1, 2, 4, 8, 16 | | none |
| tfuplan71 `Dimension` | `tfuplan71.rs:47` | side 0 or above 16384 | | none |
| tfuplan71 `LevelCount` | `tfuplan71.rs:50` | `last_level < base_level`, more levels than the side halves, or past the 4-bit NUMMM | `glGenerateMipmap` | none |
| tfuplan71 `NoLevels` | `tfuplan71.rs:52` | nothing after the base | a 1x1 texture | none |
| tfuplan71 `RasterDestination` | `tfuplan71.rs:54` | the destination is raster: "TFU requires UIF when generating mipmaps" | a small level kept as UBLINEAR or LINEARTILE (the open `slices71` question) | **118**: the first level that Mesa lays out non-UIF; to be read from the board run |
| tfuplan71 `SourceNotBase` | `tfuplan71.rs:57` | the source is not the base level in the destination's layout | | none |

## 7. Shared 4.2 refusals the six cases reach through the GL core

Not 7.1 code (the owner is `docs/notes/v3d-refusals.md`, not touched
here): the 7.1 table inherits the GL core's decisions, and these are the
two the Pi 4 run of the six cases named (`docs/notes/pi4-v3d-bringup.md`
`:322-335`).

| Case | Refusal | Where on the tip | State |
|---|---|---|---|
| 59 | "the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in" | `src/v3d/backend.rs:3806` (`record`, `Attempt::Unsupported`); the 7.1 twin is `zskeep71.rs:463`/`:478`, same wording | still there; `v3d71-gl-table.md` cites it as `backend.rs:3791`, which has moved |
| 67 | "a colour format with fewer than 8 alpha bits" (the GL_RGB frame) | **gone**: `src/v3d/backend_tests.rs:3359-3396` pins that such a target is drawn (`logs_with("alpha bits") == 0`), the GL core stores RGB/RGB565 as RGBA8 with the alpha masked; the `state.rs:969` cited by the table now holds no refusal (the only `alpha bits` text is a test comment, `state.rs:1071`) | `v3d71-gl-table.md` section 5 is stale for 67 |

## 8. The online compiler

`tools/v3d/gltest-shaders71/` pins two programs (`ours_clear`,
`ours_color_uv`); the compiler refused neither
(`tests/v3d71_gltest_shaders.rs:28-30`, `:63-65`: "there is none today").
So the six cases meet **no compiler refusal**, and Mesa's offline 7.1
binaries (`tools/v3d/gltest-mesa71`) are not needed for them. For other
programs the stage that refuses is, by name, `V3d71Error`
(`src/shaderc/online/v3d/chain71.rs:50-66`): `NotYet(&str)`, `Chain(V3dError)`
(the 4.2 stages' own refusals), `Select(Select71Error)` (`select71.rs:56`:
`NotYet`, `NotOn71`, `ViewportXyMissing`), `Alloc(Alloc71Error)`
(`alloc71.rs:130`: `NotYet`, `DoesNotFit` (no spilling), `Threads`,
`UseBeforeDef`, ...), `Sched(Sched71Error)` (`sched71.rs:107`: `NotYet`,
`ControlFlow`, `NotEncodable`, `CondWithoutFlags`, `LdvaryCWithoutLdvary`,
`Rf0Clobbered`, ...), `Encode(i, EncodeError)` (`codec71.rs`: `MesaRefuses`),
`Verify(VerifyError)`, `OutOfScratch`. Their corpus (glmark2's scene
shaders, the GLES2 corpus) is held by `glmark2shd71` and `gles2shd71`, not
here.

## 9. Which refusals the board run must show

Predicted verdicts for the six cases (the host reading; the board session
replaces them):

| Case | Predicted verdict | Refusal shown if not a pass |
|---|---|---|
| 5 scissor | pass | none expected; a `clip window` / `to_pixel` refusal would be a defect (`v3d_job71.c:423-427`) |
| 58 clear with scissor | pass | none; `clearplan71::NegativeScissor` only for a negative scissor |
| 59 clear with write masks | **refused by name**: "the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in" | `ZsRefusal71::ReadLost(Stencil)` / `backend.rs:3806` |
| 66 clear not bounded by viewport | pass | none |
| 67 FBO limits | **not refused at alpha bits any more** (section 7): pass if its stencil frame keeps stencil between jobs; otherwise refused by the same stencil string as 59 | if refused, `ReadLost(Stencil)`; the old alpha string is a regression |
| 102 clear under partial colour mask | pass | none |

That is 4 or 5 of 6 pass, 59 refused, 67 uncertain between the two. The
`v3d71-gl-table.md` prediction (4 of 6, 59 and 67 refused) was made before
the alpha refusal was removed; its section 5 should be re-read against
section 7 (that note is held by `cfg71audit`, not edited here).

The board's gl_test log must therefore show, by name, for the six cases:

1. for case 59: the line with `reads stencil an earlier job wrote`;
2. for case 67: either a pass or the same stencil line; never a
   `fewer than 8 alpha bits`, `V3D 7.1: ...` or `v3d 7.1: ...` line;
3. for 5, 58, 66, 102: no `V3D 7.1:`, `v3d 7.1:`, `7.1 draw:`,
   `texture state 7.1:`, `clear:`, `v3d71 tfu:` or `V3D 4.2: build gpu-v3d`
   line, and no `admit71`/`rt71`/`attr71`/`record71`/`format71`/`limits71`
   tagged reason.

For the texture cases 2, 115 and 118 (`board71-tex`): until `texuni71`'s
words are wired, **2, 115 and 118 are refused by `uniform71::TextureSize`
or `TmuConfig`** (`v3d_job71.c:349-353` in the C builder); case 118 adds
`tfuplan71::RasterDestination` if a small level is raster; case 115 shows
`sampler71::CubeNonSeamless` only if the GL core asks for it.

Any other refusal text in the log is a defect of the 7.1 path or of the
caller, and is a new cause for an issue, not an expected refusal.
