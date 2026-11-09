<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: sampling RGBA32F/UI/I as two G32R32 planes

The design behind gc-formats.md item 4 (`docs/es3/gc-formats.md` section 3),
read from Mesa 26.2.1 etnaviv (`third_party/mesa-26.2.1/UPSTREAM`; the
etnaviv sources are not vendored. Citations are to the release tarball,
`src/gallium/drivers/etnaviv/` unless a path says otherwise). Three parts:
the back end (planes, descriptor pair, companion slot), the compiler (two
look-ups joined) and the mask flip. The first two are in the tree: the back
end (`67b27e26`) and the compiler join (`f455cc1c`, `8c8fc3b6`, `2091d22d`,
`1f37cd60`). The path stays unreachable: `GC_SAMPLED_TEXFMTS` holds no
128-bit class, so the GL core never stores one for this back end, and
`emit_slot` (`src/vivante/texture.rs`) still samples the black texel, by
name, for a binding of one. What is left is the flip: gc-wide128-flip adds
RGBA32F/UI/I to `GC_SAMPLED_TEXFMTS`.

## Mesa's design

**No 128-bit TE format.** `format_is_128bit` (`etnaviv_format.h:86-96`)
names RGBA32F, RGBA32I and RGBA32UI. Their table rows are the G32R32
plane formats: `EXT_G32R32I` for both integer classes and `EXT_G32R32F`
for the float one, marked "emulated format" (`etnaviv_format.c:229-233`).
In common_3d.xml.h these are `TEXTURE_FORMAT_EXT_G32R32I` 0x24 and
`TEXTURE_FORMAT_EXT_G32R32F` 0x0b.

**Two stacked planes per level.** `setup_miptree` lays the resource out as
its 64-bit twin (`translate_format_128bit_to_64bit`, `etnaviv_format.h:71`):
stride, layer stride and level size are those of 8-byte texels, and the
level is twice that size (`etnaviv_resource.c:310-338`). Plane 0 holds every
layer's R,G halves. Plane 1 holds the B,A halves and starts half-way into
the level: `etna_resource_level_second_plane_offset` is `size * depth / 2`
(`etnaviv_resource.h:91-94`). A 128-bit resource is always TILED
(`etnaviv_resource.c:592-594`) and has no tile status
(`etnaviv_resource.c:165-167`). The CPU tiles and untiles it in two halves,
`etna_texture_tile_rgba32f` / `etna_texture_untile_rgba32f`
(`etnaviv_tiling.c:137-158`, called from `etnaviv_transfer.c:235-243` on
unmap and `:489-496` on a read map).

**The descriptor pair.** A view of such a format composes two descriptors in
one allocation. The second is a byte copy of the first with each level
address moved to that level's plane 1, and it gets its own
`DESC_ADDR_COMPANION` (`etnaviv_texture_desc.c:374-415`). The emit loop
writes the view's sampler state and descriptor address at its slot. Right
after that it writes the same state and the companion address at the
companion slot (`etnaviv_texture_desc.c:479-513`). Every slot written is
invalidated after all the addresses (`:525-535`). The legacy state path
does the same with registers (`etnaviv_texture_state.c:286-296, 516-557`),
but HALTI5 uses descriptors.

**The companion slot.** On the i.MX8MP the core is GC7000 rev 0x6204.
Mesa's NXP hwdb row for it is `gc7000UL_6200`
(`src/etnaviv/hwdb/nxp/gc_feature_database.h:15263`, matched on
`chipVersion & 0xfff0`, `:25131-25132`), with `REG_UnifiedSamplers` 1 and
`REG_Halti5` 1 (`:15514`, `:15596`). So `specs.unified_samplers` holds
(`etnaviv_screen.c:837-845`), and the stage's views are packed by
`etna_pack_unified_companions` (`etnaviv_texture.c:301-322`). The k-th
128-bit view of a stage, in view order, takes slot `nr + k`, where `nr` is
the number of views the stage binds. The vertex stage's base then moves
past the fragment stage's companions:
`num_fragment_sampler_views + bitcount(tex_is_128bit[FS])`
(`etnaviv_texture.c:428-439`). `companion_slot` adds that base for a vertex
view (`:441-453`). With no companion (`~0U`) the pair is skipped
(`etnaviv_texture_desc.c:505`). The non-unified split (`:340-354`,
`:373-384`) does not apply to this core.

**The shader join.** `etna_nir_lower_128bit` (`etnaviv_nir_lower_128bit.c:
89-170`), keyed per draw on `tex_is_128bit` and `sampler_companion`
(`etnaviv_context.c:146-150, 198-202`; `etnaviv_shader.h:74-75`), clones
each look-up of a 128-bit sampler onto the companion. It joins the results
as `vec4(a.x, a.y, b.x, b.y)`. A gather (`tg4`) becomes two 16-bit-half
gathers of one plane, joined with `(hi << 16) | lo` (`:115-151`).

## What this back end does (the inert half)

- **The gate.** `vivante::texture::wide128_sampled(t)` is true for a
  complete binding of a 128-bit class (`is_wide128`) when the class is in
  `backend::sampled_texfmts()`. `GC_SAMPLED_TEXFMTS` holds none, so this is
  false outside a host test that adds the classes through
  `TEST_EXTRA_SAMPLED`. Everything below happens only when it is true.
  Otherwise the binding samples the black texel with the log line
  "has no texture descriptor format", as before. No slot moves.
- **The planes: a tiled copy at a page.** The GL core keeps the texels
  linear and interleaved, 16 bytes each. `tiled_copy::wide_texels` takes a
  128-bit binding that is sampled as planes, as it takes RGBA16F: a 4x4-tiled
  copy whose base is on a page. Mesa only hands this GPU page-aligned GEM
  buffers, and a linear 64-bit layout misread on the board
  (`src/vivante/VIVANTE_NOTES.md`, "RGBA16F targets"). `tile` calls
  `tile_planes` for 16-byte texels. It de-interleaves each layer: bytes 0-7
  of a texel go into plane 0 and bytes 8-15 into plane 1, both tiled at
  8 bytes a texel. A level's chain layout is the one `tiled_layout` gives for
  16-byte texels, which is twice the 8-byte level, as Mesa sizes it.
  `tiled_copy::second_plane_offset(t, l)` is `layers * align4(w_l) *
  align4(h_l) * 8`, Mesa's half-way point.
- **The descriptor pair.** `build_descriptor(f, t, plane)` takes
  `wide128_plane_format`: CONFIG0 format 0 and CONFIG1 `EXT_G32R32F` /
  `EXT_G32R32I`. There is no CONFIG2 sign bit, because
  `etnaviv_texture_desc.c:187-189` sign-extends only 8- and 16-bit
  channels, and no INT_FILTER. The addressing is TILED, `LINEAR_STRIDE` is
  `align4(w) * 8` and `SLICE` is `align4(w) * align4(h) * 8` (the plane's
  layer). The binding's own swizzle goes into both planes. Plane 1's
  descriptor is the same words except each `LOD_ADDR`, which is moved by
  `second_plane_offset`. `emit_slot` builds plane 1 straight after plane 0.
  It writes the slot's sampler registers, then the same values at the
  companion (`write_sampler`). The deferred invalidates cover both slots.
- **The companion-slot policy.** Mesa's unified packing. For each stage,
  `count` is the slots the stage's units take: the largest bound or
  declared unit + 1, the count `VS_SAMPLER_BASE` already used. The k-th
  128-bit binding in unit order gets `companion_slot(base, count, k) =
  base + count + k`. `VS_SAMPLER_BASE` is the fragment count plus the
  fragment stage's 128-bit bindings.
  - **Collision rule.** None can happen: a companion lies past every unit
    its stage binds or declares, so no GL unit is displaced. The GL core
    excludes nothing (`src/gl/texture.rs` is unchanged).
  - **Out of slots.** A companion past slot 31, the end of the 32-entry
    array the stages share, is `None`. The binding then samples the black
    texel in its own slot, by name ("no companion sampler slot").
  - **No copy.** A reserved companion whose binding has no copy (no CPU
    view) gets the black texel too, so it never keeps a stale descriptor.
- **The CPU untile on retire.** Sampling never writes the planes, so
  nothing comes back from them. The copy is derived and rebuilt from the
  linear storage whenever the key changes (`content_gen` for a CPU write,
  `target_kick` for a render). A render into the texture still in flight is
  finished first (`tiled_copy::refresh`). The copy itself is rebuilt from
  the storage, not untiled. Untiling on retire is what a 128-bit render
  target will need: two planes written by the PE, untiled and re-interleaved
  into the linear storage when the kick retires, the way
  `wide_target.rs` does for 64-bit targets with Mesa's
  `etna_texture_untile_rgba32f` as the model. That is not part of this work
  (render targets are a separate item).

## What is left

1. **Done since the back end: the compiler join and the per-draw key**
   (wide128-shader). All of it is inert: `GC_SAMPLED_TEXFMTS` still holds no
   128-bit class, so the key is always empty on a shipped draw.
   - `f455cc1c`, the compiler half: `wide128::split_planes`
     (`src/shaderc/online/vivante/wide128.rs`) turns each look-up on a keyed
     unit into two, the second at the companion slot, joined as
     `(a.x, a.y, b.x, b.y)`. The key is `Planes` (`set_compile_planes` /
     `compile_planes`); `compile_vivante` runs the pass only for a non-empty
     key. It refuses shadow, gradient and offset look-ups on a keyed unit,
     and a companion that is no unit of its own or that the shader samples.
   - `8c8fc3b6`, `2091d22d`: `wide128_sensitivity` names the bindings a
     program's look-ups read, `wide128_planes` builds the stage's key from
     the per-binding companion slots, and `wide128_variant` compiles a stage
     again under a key.
   - `1f37cd60`, the per-draw key from the GL core: `wide128_select`
     (`src/gl/draw.rs`) takes `wide128_companions` (the slots
     `emit_textures_inner` computes, over `wide128_sampled`), builds the key
     per stage and points the draw at the cached variant. It refuses a draw
     that needs a front-facing twin and a plane key at once. A `usampler2D`
     names an RGBA8UI and an RGBA32UI texture alike, so the key comes from
     the draw, not the program.
   - Host test `tests/es3_wide128_gc.rs`.
   Left of the join: a gather (it needs Mesa's 16-bit halves; no
   `textureGather` split exists) and the no-alpha 1 (item 3).
2. **The flip** (gc-wide128-flip, still left): the three classes into
   `GC_SAMPLED_TEXFMTS`, the gc-formats.md rows (`none` becomes the plane
   format) and the host dEQP measurement.
3. **Caveats for those items.**
   - A binding with `no_alpha` (an RGB32F texture stored as RGBA32F) gets
     swizzle A = ONE in both descriptors. The join reads channel 1 of
     plane 1, which that swizzle does not reach, so the shader side must
     supply the 1 itself. Mesa has the same shape.
   - With sixteen fragment units in use, the companions plus the vertex
     stage can run past slot 31. Vertex slots past it are not bounded
     today, as before.
   - Nothing here is measured on the board.
