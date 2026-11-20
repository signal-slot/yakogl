<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 7.1 cube maps and seamless filtering (the host half of U8)

Question U8 of `docs/notes/pre-study-rpi5.md` (section 3, lines 398-402;
the U8 row at line 577): "Does the 7.1 TMU filter cube maps seamlessly (the
4.2 defect `docs/v3d-noseamless-cube.md`)?" Its second half — "Mesa's cube
handling for 7.1 is not read", also the caveat on the cube row of
`docs/notes/v3d71-gl-table.md` section 3 — is read here. This note records
what Mesa's 7.1 code and the 7.1 packet layouts say, a prediction for the
hardware half, and the exact case a board session must run to answer it.
Nothing here is a board measurement. The U8 row and its `tools/cts/RESULTS.md`
row are written by the board item (board71-tex), not by this note.

Read first: `docs/v3d-noseamless-cube.md` (the 4.2 rule, the emulation and
its keying), `docs/notes/v3d71-gl-table.md` section 3 (the 7.1 texture
table), `docs/online-shaderc/v3d-backend.md` "Cube maps" (line 2365) and
"On the board with v3d-m5's cube look-up" (line 4083).

Citations. `X:` is `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml`
and `M:` is `third_party/mesa-26.2.1/src/broadcom/`, as in
`docs/notes/v3d71-gl-table.md:13-14`; `M:` also covers the vendored
`src/gallium/include/pipe/p_defines.h` (`third_party/mesa-26.2.1/UPSTREAM:171`).
`U:` is the whole Mesa tree at the pinned tag — `UPSTREAM` names tag
`mesa-26.2.1`, commit `da14d65e4499e66468094be52bff9ea0915a695e` — for the
files `third_party/` does not vendor (`gallium/drivers/v3d/v3d*.c` beyond
the format tables, `broadcom/{compiler,vulkan,ci}`, `compiler/nir/`,
`mesa/`, `.gitlab-ci/`), paths relative to the Mesa root. The `U:` reading
was done on a checkout of that exact tag; every vendored file that overlaps
it matched byte for byte (sha256 of `v3d_packet.xml` `b13f995b…` =
`UPSTREAM:140`, of `v3dx_format_table.c` `d7879d6b…` = `UPSTREAM:169`), so
the `U:` line numbers name the tree this repository pins.

## 1. What 7.1 changes in the cube records, and what it does not

| Piece | 4.2 | 7.1 | Citation |
|---|---|---|---|
| A cube field, a seamless field, a dimension field | none | **none**: `cube` and `seamless` do not appear anywhere in the packet XML; the only "type" field of the texture record is the **data format** (`Texture type` 7 bits, and Gallium writes it from the format table) | `X:` (grep); 4.2 `X:1616`, 7.1 `X:1681`; `U:src/gallium/drivers/v3d/v3dx_state.c:1036-1037` |
| Texture Shader State | `X:1592-1631` | re-laid out (`X:1633-1707`): `sRGB` 1 bit at 3 (`X:1627`) becomes **`Transfer func` 3 bits at 2** (none / sRGB / PQ / HLG / BT.1886 variants, `X:1697-1704`); `Reverse Standard Border Color` at 5 (`X:1625`) becomes **`Reverse`** at 5 (`X:1696`); new **`R/B swap` at 32** (`X:1692`); `Array Stride (64-byte aligned)` 26 bits at 32 (`X:1621`) becomes **24 bits at 33** (`X:1691`); new chroma pointers and offsets (`X:1654-1657`, the address-encoding comment `X:1635-1653` needs `base >> 6`); new UIF bits (`X:1659-1662`) | the split as such; nothing cube-specific |
| Revision split inside 7.1 | one layout | **7.1.2 has no R/B swap bit** and its stride starts at 32; Mesa uses the 7.1.5 definition for all 7.x (`X:1686-1690`); the 7.1 table refuses below 7.1.5 (`v3d71-gl-table.md` section 4; `driver/src/v3d/v3d_tex71.h:33`) | `X:1686-1690` |
| Fields both generations keep | Image Width/Height/Depth 14 bits each (`X:1617-1619`), Base/Max Level 4+4 (`X:1599-1600`), swizzle quartet (`X:1602-1613`) | the same at their own offsets: depth 14 bits at 86 (`X:1682`), Base/Max Level (`X:1664-1665`), swizzles (`X:1667-1678`) | `X:1592-1631`, `X:1633-1707` |
| Sampler State | one struct for every generation (`X:1709-1734`): wrap S/T/R 3 bits each (`X:1718-1720`), fixed bias s8.8 (`X:1722`), LOD clamps u4.8 (`X:1723-1724`) | identical struct, **no seamless or cube field**; `sRGB Disable` at 7 (`X:1726`) exists in no code path either generation writes — Mesa's own Vulkan code: `s.srgb_disable = 0; /* Not even set by v3d */` | `X:1709-1734`; `U:src/broadcom/vulkan/v3dvx_device.c:261` |
| TMU Config Parameter 0 / 1 | `X:1534-1537`, `X:1539-1543` | **the same structs** (no version attributes in the XML); Mesa says it outright: "TMU_CONFIG_PARAMETER_1 doesn't change between v42 and v71" | `X:1534-1543`; `U:src/broadcom/compiler/v3d_packing.c:31-46` |
| TMU Config Parameter 2 | `X:1560-1573`, `LOD Query` bit 24 (`X:1562`) | `X:1575-1590`: adds `DIM query` (25) and `Write conversion` (26) (`X:1577-1578`); **Mesa never sets either** — `dim_query` / `write_conversion` appear in no Mesa source outside the packet layout (`U:` grep; the only text hit is an unrelated PBO helper of the core) | `X:1560-1590`; see section 5 |

So: 7.1 re-lays the texture record and adds transfer-function, swap, reverse
and chroma content; **it changes nothing about how a cube map is described.**
A cube record is a 2D-array record in both generations, exactly as at 4.2
(`docs/v3d-noseamless-cube.md:50-61`), and the look-up instruction still
decides what it is (section 2).

## 2. Mesa's cube path is one code path for 4.2 and 7.1

- **Record setup.** `v3d_setup_texture_shader_state`
  (`U:src/gallium/drivers/v3d/v3dx_state.c:903-988`) has no version test in
  the cube lines: `image_depth` = last − first + 1 layer (`:936-938`, so 6
  for a plain cube); a *sampling* view of a cube array divides the layer
  count by 6 under the comment "Empirical testing with CTS shows that when
  we are sampling from cube arrays we want to set image depth to layers / 6,
  but not when doing image load/store or sampling from 2d image arrays"
  (`:941-948`); the base is `bo->offset + v3d_layer_offset(prsc, 0,
  first_layer)` (`:956-960`, `U:src/gallium/drivers/v3d/v3d_resource.c:771-779`);
  `array_stride_64_byte_aligned = cube_map_stride / 64` (`:961`, laid out by
  `U:src/gallium/drivers/v3d/v3d_resource.c:762-766`). The only
  `#if V3D_VERSION >= 71` block in it adds the chroma fields (`:963-969`). `sampling_cube_array` comes from the
  *view's* target (`:1015`), and the assert accepts a cube-array resource
  seen as a 2D array under ARB_texture_view (`:911-916`). Upload alignment
  32 bytes (`:1003-1005`).
- **Vulkan mirrors Gallium 1:1 for cubes** (`U:src/broadcom/vulkan/v3dvx_image.c`):
  depth = view layer count (`:82`), sampling views `/6` (`:85-92`), stride
  `cube_map_stride / 64` (`:107`), and a cube-array view is packed **twice**,
  a sampling pack and a storage pack (`:35-41`, `:168-175`).
- **The look-up.** The compiler is a single shared file: `v3d_tex.c` has no
  version test anywhere in the cube path, and it always packs P0/P1/P2 with
  the **V3D42** packers even in a 7.1 build (`U:src/broadcom/compiler/v3d_tex.c:31`,
  `:285-292`), which is harmless because every field Mesa ever writes in P2
  sits at the same bit in the 4.2 and 7.1 layouts (`X:1560-1590`; the fields
  written: `op`, gather, gather component, coefficient, `disable_autolod`,
  `lod_query` — `U:src/broadcom/compiler/v3d_tex.c:255-262`). The TMU write
  addresses are version-neutral (`M:qpu/qpu_instr.h`): TMUI 36 (`:125`),
  **TMUSCM 40** (`:129`), TMUSF 41 (`:130`), TMUSLOD 42 (`:131`). A cube
  look-up retires with **TMUSCM** (`U:src/broadcom/compiler/v3d_tex.c:375-376`);
  texelFetch retires with TMUSF and *asserts the sampler is not a cube*
  (`:373-374`); an explicit level on a non-cube uses TMUSLOD (`:378`), but
  an explicit level **on a cube** writes the level to TMUB and sets P2
  `disable_autolod` instead, per the comment at `:141-145` and the branch at
  `:147-150`. The face index is never written to TMUI in the sampler path
  (TMUI only for `is_array`, `:122-128`); the face comes from the direction
  in the hardware. Cube *images* are the exception on record: load/store
  treats a cube as a 2D array whose third coordinate is the face (`:454`,
  `:470-477`), retiring TMUSF (`:505`).
- **Coordinates.** The NIR pass `nir_normalize_cubemap_coords` (max axis to
  ±1, projection first, array index untouched,
  `U:src/compiler/nir/nir_normalize_cubemap_coords.c:34-68`) runs in both
  7.1 front ends too: `U:src/gallium/drivers/v3d/v3d_program.c:374` and
  `U:src/broadcom/vulkan/v3dv_pipeline.c:334`.
- **Sizes.** `textureSize` is the uniform words as ever (44-47,
  `docs/notes/v3d-texture-size-words.md`): `U:src/gallium/drivers/v3d/v3d_uniforms.c:51-88`
  (cube-array answers `array_size / 6`, `:73-80`) fed from
  `U:src/broadcom/compiler/nir_to_vir.c:920-941`. `textureQueryLod` is the
  P2 `lod_query` bit, which is a 4.2 bit (`U:src/broadcom/compiler/v3d_tex.c:262`,
  `X:1562`). Neither uses the two new 7.1 P2 bits (section 5).

The repository's own 7.1 side already matches this, in
`src/shaderc/online/v3d/`: `select.rs:1034-1039` retires cube look-ups with
`WADDR_TMUSCM` (`select.rs:124`, = 40), `sched71.rs:99` lists TMUSCM as a
7.1 look-up terminator, and `sim.rs` models the cube look-up identically for
both generations (`sim.rs:145-203`, `:921`, `:2775-2836`).

## 3. Seamless control: Mesa has the plumbing and v3d never touches it

- The Gallium sampler state has the field `seamless_cube_map`
  (`U:src/gallium/include/pipe/p_state.h:470`) and the caps
  `PIPE_CAP_…_seamless_cube_map` / `_per_texture` exist
  (`M:src/gallium/include/pipe/p_defines.h:887-888`). Drivers with the
  hardware bit consume the field:
  `U:src/gallium/drivers/panfrost/pan_cmdstream.c:231`,
  `U:src/gallium/drivers/asahi/agx_state.c:528`. Mesa's core sets the flag per context
  (`U:src/mesa/main/version.c:690`: `CubeMapSeamless = _mesa_is_gles3(ctx)`),
  the state tracker puts it in the pipe sampler
  (`U:src/mesa/state_tracker/st_atom_sampler.c:65-90, 206`) and maps the caps
  to extensions (`U:src/mesa/state_tracker/st_extensions.c:1070, 1141`).
- **v3d, both generations, ignores all of it.** The word `seamless` appears
  in no file of `U:src/gallium/drivers/v3d/` or `U:src/broadcom/` (grep; the
  only broadcom hits are the CI expectation files of section 4); Gallium's
  sampler packer (`U:src/gallium/drivers/v3d/v3dx_state.c:585-767`) and the record setup
  (`:903-988`) carry it nowhere, and the records have no field for it to go
  to (section 1). `VK_EXT_non_seamless_cube_map` (`U:src/vulkan/registry/vk.xml:28277`)
  is implemented by the panfrost, asahi, anv, hasvk, zink, lavapipe, pvr,
  tu, radv and virtio drivers (grep of `src/`), and **by no file of
  `U:src/broadcom/vulkan/`**. v3d has no knob that could make a cube
  non-seamless, at any revision.
- dEQP itself picks its reference per context and per group
  (`docs/online-shaderc/v3d-backend.md:4090-4095`): `texture.filtering.cube.*`
  is judged **seamlessly even under ES 2.0** (`es2fTextureFilteringTests.cpp:594`);
  `texture.size`/`texture.format` are not seamless under ES 2.0; `texture.mipmap`,
  `texture.vertex` and `shaders.texture_functions` use the seamless reference
  only on an ES 3-capable context. Mesa's own core always asks for seamless
  in ES 3 (above), so an ES 3 context on v3d gets exactly what the hardware
  gives.

## 4. Mesa expects 7.1 to behave like 4.2 — its CI says so line for line

- Mesa CI runs the dEQP GLES31/GLES3/GLES2 mustpass suites and the Khronos
  CTS runners on **both** boards, with the same file structure
  (`U:src/broadcom/ci/deqp-broadcom-rpi4-gl.toml` and
  `U:src/broadcom/ci/deqp-broadcom-rpi5-gl.toml`, each `:2-3, 15-16, 26-27,
  37-38, 56-57`; the dEQP-GLES module caselists are the CTS **AOSP** main
  files and the glcts caselists the **Khronos** ones,
  `U:.gitlab-ci/container/build-deqp.sh:294-303`), and **no dEQP-GL case
  appears in either board's fails or skips file** (grep `dEQP-GLES` over the
  four files: no hit) — every dEQP-GL cube case inside that coverage is
  *expected to pass* on both generations.
- The two *fails* files hold the **same 17 cube-named lines** — the entries
  are identical, only their line numbers differ — and all 17 are piglit:
  `arb_shader_texture_lod-texgradcube`
  (`U:src/broadcom/ci/broadcom-rpi4-fails.txt:84`,
  `U:src/broadcom/ci/broadcom-rpi5-fails.txt:68`), twelve
  `nv_image_formats` cube *declaration* lines (rpi4 `:169-210`, rpi5
  `:140-181`), and the shadow pair under the comment `# OpenGL 3.x applies
  non-seamless cubemap texturing, while our driver/GLES uses seamless cubemap
  texturing.` (rpi4 `:285-288`, rpi5 `:238-241`). Those shadow tests expect
  the *non-seamless* reading; Mesa expects them to fail on RPi5's 7.1 exactly
  as on RPi4's 4.2.
- The *flakes* files differ, and the difference sits on 4.2: rpi4 flakes
  `KHR-GLES31.core.texture_cube_map_array.image_op_*`
  (`U:src/broadcom/ci/broadcom-rpi4-flakes.txt:20-21`), three dEQP-GLES31
  `copy_image … cubemap_to_texture3d` (`:31-33`) and three piglit
  `tex-miplevel-selection … cube` lines (`:75, 91-92`) — cube *image storage,
  copy and LOD-choice* tests, not seam-filtering tests, and none of them
  appears in rpi5's flakes, whose cube names are all VK.
- The VK cube *filtering* cases with the `.seamless` suffix sit in the
  *flakes* lists of **both** boards — expected to pass:
  `U:src/broadcom/ci/broadcom-rpi4-flakes.txt:152`,
  `U:src/broadcom/ci/broadcom-rpi5-flakes.txt:131`, with the VK cube shadow
  cases at rpi5 `:133-135`.
- Mesa therefore expects 7.1 cube behavior to equal 4.2 cube behavior at
  every point where it states an expectation: identical cube *fails* on both
  boards, no dEQP-GL cube failure or skip anywhere, cube *flakes* only on
  4.2's storage tests. (Section 9 discusses which CTS lists these boards
  actually run; it does not change what the boards expect from each other.)

## 5. Mesa's choice per revision (7.1 vs 7.1.10)

**For cube maps: none.** There is no `V3D_VERSION` / `devinfo->ver` test in
any cube line of the record setup, the compiler lowering, or the NIR
normalization; 7.1.10 handles a cube exactly as 7.1.2 — and as 4.2. The
revision-sensitive points that touch a cube at all are already recorded in
`docs/notes/v3d71-gl-table.md` section 3:

- the 7.1.2 vs 7.1.5 record layout split (`X:1686-1690`), which only moves
  the stride bits and the (never-set-by-GL) R/B swap bit; Mesa picks 7.1.5
  for all, and our table refuses below it;
- the per-generation *value* choices inside the same code: 4.2 writes
  `tex.srgb`, 7.1 writes `tex.transfer_func`
  (`U:src/gallium/drivers/v3d/v3dx_state.c:1023-1029`); 7.1 writes the
  chroma fields (`:963-969`); the 4.2 half-float border-color conversion is
  skipped at 7.x ("The TMU in V3D 7.x always takes 32-bit floats",
  `U:src/gallium/drivers/v3d/v3dx_state.c:749-760`); the draw-index shader
  record of 7.1.10 (`M:common/v3d_device_info.h:94-97`) is shader state, not
  texture state.
- The two new 7.1 P2 bits (`DIM query`, `Write conversion`, `X:1577-1578`)
  are **unused by Mesa**: the compiler sets only `op`, gather mode/component,
  coefficient mode, `disable_autolod` and `lod_query`
  (`U:src/broadcom/compiler/v3d_tex.c:255-262`), plus the offsets when they
  are constants (`:161-190`), and it packs P2 with the V3D42 packer in both
  builds (`:31`, `:290-292`); `textureSize` rides on the uniform words and
  `textureQueryLod` on `lod_query` (section 2). A 7.1 table may pack the two
  new fields for layout completeness (`driver/src/v3d/v3d_tex71.c:254-258`
  round-trips them) but no GL feature needs them set. (`v3d71-gl-table.md:72`
  says the opposite; see section 6.)

## 6. What the 7.1 table sets for a GL cube texture

Nothing cube-specific beyond the ordinary 2D-array record, and the in-tree
`v3d_tex71.*` packers already cover it. (In this section only, Mesa paths
are shortened: `v3dx_state.c` =
`U:src/gallium/drivers/v3d/v3dx_state.c`, `v3d_tex.c` =
`U:src/broadcom/compiler/v3d_tex.c`, `v3dvx_image.c` =
`U:src/broadcom/vulkan/v3dvx_image.c`.)

| Field | Value | Mesa | our packer |
|---|---|---|---|
| `width`, `height` | face extent (14 bits each) | `X:1683-1684`, `U:v3dx_state.c:921-933` | `v3d_tex71.h:85` |
| `depth` (Image Depth) | **6** (the layer count; 14 bits) | `X:1682`, `U:v3dx_state.c:936-938` | `v3d_tex71.h:85-86` |
| base pointer | start of layer 0 (`first_layer`) | `X:1694`, `U:v3dx_state.c:956-960` | the record's base, low bits reserved for the flags (`v3d_tex71.c:109-115, 141-143`) |
| `array_stride_64` | per-layer mip-tree stride / 64 (24 bits at 7.1) | `X:1691`, `U:v3dx_state.c:961` | `v3d_tex71.c:43` (layout stride, refusal if ≥ 2^24 at `:102`); `v3d_tex71.h:102, 107` |
| `transfer_func` | sRGB 1 / none 0 from the *view* format | `X:1697-1704`, `U:v3dx_state.c:1023-1028` | `v3d_tex71.h:92`, `v3d_tex71.c:97-98` (refuses the HDR values) |
| `reverse`, `r_b_swap` | **0 in the Gallium (GL) path** — only Vulkan sets them (`U:v3dvx_image.c:122-145`), and R/B swap needs 7.1.5 | `X:1692, 1696` | `v3d_tex71.h:95-98` (BGRA-family formats keep flowing through the composed swizzle, as at 4.2) |
| chroma | offsets 1 and 1, `Cb` = `Cr` = base >> 6 for an unconverted cube | `X:1635-1657`, `U:v3dx_state.c:963-969` | `v3d_tex71.h:82-83`, `v3d_tex71.c:134-135` |
| `Base Level`, `Max Level` | the view's levels (4 bits each; Mesa caps levels at `V3D_MAX_MIP_LEVELS` 13, `M:common/v3d_limits.h:69`) | `X:1664-1665` | `v3d_tex71.h` levels |
| sampler state | unchanged from 4.2 (one struct); the wrap modes are programmed but a TMUSCM look-up ignores them (measured at 4.2, `docs/v3d-noseamless-cube.md:30-32`); border colors as 32-bit floats at 7.x | `X:1709-1734`; `U:v3dx_state.c:749-760` | the shared sampler record |
| look-up | T, R, retire **TMUSCM**; an explicit level writes TMUB plus P2 `disable_autolod`, never TMUSLOD; P2 `DIM query` / `Write conversion` stay 0 | `U:v3d_tex.c:137-153, 255-262, 375-376` | `select.rs:1034-1039`, `chain.rs:2431-2464` (4.2 pins, same shape), `v3d_tex71.c:254-258` |

For a **cube-array** view (if backend71 ever exposes `GL_TEXTURE_CUBE_MAP_ARRAY`;
Mesa has the cap on, `U:src/gallium/drivers/v3d/v3d_screen.c:267`):
a sampling view sets depth = layers / 6 (`U:v3dx_state.c:941-948`,
`U:v3dvx_image.c:85-92`), a storage/2D-array view keeps the full layer count
and lowers the face into TMUI (`U:v3d_tex.c:470-477`).

## 7. U8: the host half is answered; the hardware half gets a strong prediction

**Answered by reading (not the board):** 7.1 has *no* seamless control, in
the packet layouts (section 1), in Mesa's driver (section 3 — the pipe field
is ignored and no VK extension exists for broadcom), and therefore in our
table (section 6 — there is no bit to set or clear). The pre-study's line
"the 7.1 texture state has no seamless field either"
(`docs/notes/pre-study-rpi5.md:400-402`) is confirmed, with the citations
above. So a board session does **not** need any "can we turn seamless off"
probe; only the edge behaviour itself is open.

**Prediction for that open half:** *the 7.1 TMU filters cube maps seamlessly,
the way the 4.2 TMU does: a LINEAR tap past a face edge reads the
neighbouring face, and the sampler's wrap modes are ignored for a cube
look-up; unconditionally, with no mode to change it.* Confidence: high.

1. The mechanism is the look-up, not the record: TMUSCM is a QPU write
   address with no version variant (`M:qpu/qpu_instr.h:129`), Mesa retires
   cubes through it identically at 4.2 and 7.1 (section 2), and the record a
   cube gets is the same 6-layer record (sections 1, 6). Whatever the 4.2
   TMU does for TMUSCM at a face edge, nothing in the 7.1 path says it
   changed.
2. Mesa's expectations are identical per revision (section 4): the same 17
   cube-named piglit lines in each board's *fails* file (the shadow failures
   under the same "our driver/GLES uses seamless cubemap texturing" comment
   among them), the `.seamless` VK cube filtering cases flake-listed (expected
   to pass) on both boards, and no dEQP-GL case in either board's fails or
   skips file.
3. The 7.1 record adds many features and *not* one seamless/cube bit — if
   7.1 had changed the cube filtering behaviour, the natural place to expose
   it would have been such a bit (as Asahi's `seamful_cube_maps` and Panfrost's
   `seamless_cube_map` descriptor bits do,
   `U:src/gallium/drivers/asahi/agx_state.c:528`,
   `U:src/gallium/drivers/panfrost/pan_cmdstream.c:231`); Broadcom added neither.

**What follows for the code (no change needed):** the ES 2.0 emulation of
`docs/v3d-noseamless-cube.md` stays required for a 7.1 build that reports
ES 2.0 and stays skipped for an ES 3.0 build, exactly as at 4.2 — it changes
no record and no binding (`docs/v3d-noseamless-cube.md:50-61, 130-139`), and
the 7.1 compiler chain already threads the key: `src/shaderc/online/v3d/chain71.rs:372,
412-413` pass `cube_noseam` into the shared `prepare.rs` lowering. tex71,
mip71 and backend71 need nothing from U8 under this prediction.

**If the board contradicts it** (a LINEAR cube edge reads its own face on
7.1): the ES 2.0 side would still be correct (the emulation produces the
non-seamless values the ES 2.0 reference wants, and the 25 cases would pass
keyed or unkeyed), but the *ES 3.0* side would break — Mesa's own expectations
(section 4) say seamless is what 7.1 gives, and Mesa has no
non-seamless-to-seamless emulation anywhere, so such a result should first be checked as a
measurement fault (wrong image, keyed path active, wrong case list) before
it is accepted as behaviour. That is also why the case below is run on the
ES 3.0-reporting image, alone.

## 8. The case board71-tex runs

**Primary probe: gl_test case 115 `texcube`, run alone, on the Pi 5 gl_test
image with the 7.1 table.**

- The image: `driver/samples/zephyr/gl_test` built with the 7.1 selection of
  `docs/notes/v3d71-gl-table.md` section 4 (`CONFIG_YAKOGL_GPU_V3D71=y`,
  line 83) and the sample's ES 3.0 report (`prj.conf:27`
  `CONFIG_YAKOGL_REPORT_ES2=n`) — so no cube look-up is keyed and the
  hardware cube path runs (`docs/v3d-noseamless-cube.md:513-515`).
- The case: 115 (`driver/tools/pvrlab/gl_test.c:791-795`, body `:11148`,
  dispatch `:24013`), a V3D-only case. Six 4x4 faces — R a ramp along s, G
  along t, **B the face (16 + 40 · face)**; 12 draws, face × filter, wrap
  CLAMP_TO_EDGE (`tools/v3d/tex_cube_case.h:9-18`). NEAREST is judged exactly
  and LINEAR on the interiors; **the 4-pixel-wide LINEAR edge band is
  measured and printed, not judged: "there a seamless TMU blends in the next
  face while dEQP's CLAMP_TO_EDGE reference does not"**
  (`tools/v3d/tex_cube_case.h:20-24`; the band is compared against the
  per-face clamped reference, `:101-116`, and printed as `largest edge error
  … (not judged)`, `:172-179`). Because B encodes the face, reading the
  neighbour face moves B by a multiple of 40 in every pixel of the band.
- Running it alone: `CONFIG_V3D_GL_TEST_CASE=115` (the Kconfig option
  "Run this one case alone instead of the list", a fresh context,
  `driver/samples/zephyr/gl_test/Kconfig:468-473`; the pi4 hand-off in
  `tools/cts/common-images.sh:134-135` turns a bare `--cases 115` into
  exactly that define; the `common-images.sh` row list has no pi5 row yet —
  the P9 board item adds the same hand-off there).
- **The reading.** A large band error means the TMU read the neighbouring
  face: **seamless**. A band near zero means each face clamped at its own
  edge: non-seamless. The Pi 4 baseline for the same scene: **112 to 174 steps**
  measured across faces — "the V3D filters seamlessly across cube faces"
  (`docs/online-shaderc/v3d-backend.md:4083-4089`, 115 run alone on the
  4.2 board). Under the prediction, expect the same kind of numbers on the
  Pi 5 (the scene fixes the colour deltas between faces, so the exact values
  need not repeat; "small" is the discriminating word). The twelve judged
  lines (NEAREST every pixel, LINEAR the interiors) must PASS — they did on
  the 4.2 board, where 115 ran with 113, 118 and 61 all PASS at 84b2cf93
  (`docs/online-shaderc/v3d-backend.md:4083-4087`); a FAIL there is a cube
  *plumbing* fault of the 7.1 table, not a U8 answer.
- **Companions (same board session, not the U8 probe):** 133 `texcubemip`
  and 134 `texcubethin` judge the cube mip path and deliberately measure
  face *interiors* only, so they answer P10's chain, not the seam
  (`docs/online-shaderc/v3d-backend.md:2409-2476`,
  `tools/v3d/tex_cubemip_case.h`). The seamless-reference dEQP sets — the 19
  cases SEAM19 and the six LINEAR `texture.mipmap.cube.*` CUBELIN6, 19/19
  and 6/6 on Pi 4 images that report ES 3.0 at 758a3123
  (`docs/online-shaderc/v3d-backend.md:2502-2514`) — are the reference-side
  confirmation, for when a Pi 5 dEQP image exists (the U9 plan, not
  board71-tex): under the prediction they pass.
- **The trap not to fall into.** On our driver the *discriminating* dEQP
  cases on an ES 2.0-reporting image (`texture.vertex.cube.filtering.*` 10,
  `texture.vertex.cube.wrap.*` 8, `texture.mipmap.cube.{basic,bias,projected}.{linear_linear,linear_nearest}`
  6, `shaders.texture_functions.vertex.texturecubelod` 1, the table at
  `docs/v3d-noseamless-cube.md:22-29`) are answered by the **keyed
  emulation** on 7.1 too (`chain71.rs:372, 412-413`) — their pass says
  nothing about the 7.1 TMU. And `texture.filtering.cube.*` is judged
  seamlessly even under ES 2.0 (`docs/online-shaderc/v3d-backend.md:4090-4091`),
  so it never discriminates either. The unkeyed **measurement** of 115 is
  the only direct hardware reading.

## 9. For the planner

- `docs/notes/v3d71-gl-table.md:72` states that `textureSize` and
  `textureQueryLod` "use the new bits" (`DIM query`, `Write conversion`).
  Mesa uses neither, at either revision: sizes ride on the uniform words
  (`U:src/gallium/drivers/v3d/v3d_uniforms.c:51-88`,
  `U:src/broadcom/compiler/nir_to_vir.c:920-941`) and `textureQueryLod` on
  the 4.2 `lod_query` bit (`U:src/broadcom/compiler/v3d_tex.c:262`); the two
  7.1 bits appear in no Mesa source (section 2). tex71 may keep packing them
  (`v3d_tex71.c:254-258`); the P10 table needs no feature keyed on them.
- The U8 row of `docs/notes/pre-study-rpi5.md:577` ("Mesa's cube handling
  for 7.1 is not read") is now read, and its "first cube-map case on the
  board" has a name: gl_test 115. This note cites those files but does not
  edit them; board71-tex records the answer there and in `tools/cts/RESULTS.md`.
- The coverage question is closed at the revision this repository pins.
  Mesa CI states no dEQP-GL cube *failure or skip* on either board while
  running the GLES31/GLES3/GLES2 mustpass suites (section 4), and the 25
  cases this repository measured failing on the same 4.2 hardware live only
  in the **AOSP** GLES2 list: at VK-GL-CTS@93bca01 (`tools/cts/deps.toml:327`)
  the Khronos main list `gles2-khr-main.txt` (473 lines) names none of
  `texture.vertex.cube.*`, `texture.mipmap.cube.*` or
  `shaders.texture_functions.vertex.texturecubelod`, while the AOSP
  `aosp_mustpass/main/gles2-main.txt` (17165 lines) holds them all
  (`:14118`, `:14923-14930`, `:14947-14950`, `:15262-15297`, `:15298-15306`)
  — and `tools/cts/gles2-mustpass.txt` is a byte-identical copy of exactly
  that file (`cmp` against a checkout of 93bca01; the provenance
  `tools/cts/README.md:991-993` claims). Mesa's runner copies *both*
  families (`U:.gitlab-ci/container/build-deqp.sh:294-303`): the toml
  dEQP-GLES caselists are AOSP filenames (both boards' tomls `:3, 16, 27`),
  the glcts caselists Khronos ones (`:39-41`), no name collision between
  them. One half stays open: Mesa builds dEQP from its own pinned GLES CTS
  release (3.2.14.0, `build-deqp.sh:29`), and whether *that* revision's
  AOSP list still carries the 25 was not readable on this machine. What it
  cannot change: the question bears on *Mesa's coverage*, not on what 7.1
  does — what Mesa states about 7.1, where it states it, matches 4.2
  everywhere it states it — and board71-tex must keep treating the keyed
  dEQP cube cases as non-probes (section 8).
