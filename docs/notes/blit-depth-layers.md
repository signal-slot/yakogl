<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glBlitFramebuffer depth: cube faces, array layers and stencil in a texture

`docs/notes/blit-depth-window.md` lists what `src/gl/blit.rs` still refuses.
This note covers two of those refusals: depth read from a cube face or a 2D
array layer of a depth texture (`REFUSED_DEPTH_SRC`), and stencil copied into
or out of a texture (`REFUSED_STENCIL`). The window is not covered here
(design-window item of plan #379). The answers come from reading the code at
`9ef7e79e`. Nothing was measured, and no code was changed.

## (a) One depth buffer per face/layer, or one per texture?

**One per texture, on every back end.** The id the GL core gives a back end
names only the object. No back end can tell which face or layer a draw
attaches, so `depth_resolve` and `ds_write` cannot reach a single face or
layer. They read and write the one plane kept under the texture's id. That
plane holds whatever face, layer or level was drawn last.

### The GL core: the id has no face, layer or level

- `context::ds_object_id` (`src/gl/context.rs:643-645`) builds the id from
  the serial alone, with bit 30 set for a texture. `draw.rs` `ds_identity`
  (`src/gl/draw.rs:2759-2770`) gives a texture attachment
  `(ds_object_id(t.serial, true), t.storage_gen, w, h)`. The size is that of
  `a.level`, and `a.layer` is not read. `ds_pair_id` (`:2749-2756`), the
  `GlesDraw::ds_id` of an FBO, is built from the two attachments' names
  only. `GlesDraw` (`src/backend.rs:1141-1162`) has no field for the layer
  of the depth attachment. `target_cpu` (`:1301`) names the colour buffer's
  face or layer, not the depth's.
- `framebuffer.rs` `attachment_storage` takes the depth-texture branch
  (`src/gl/framebuffer.rs:479-516`) without checking or applying `a.layer`.
  Its storage offset is `level_off[level]`, which is face/layer 0. The
  colour branch checks the layer (`:527-528`) and adds
  `layer * lh * pitch * bpp` (`:588-590`).
- `DepthTarget` (`src/gl/framebuffer.rs:99-109`) says so in its comment:
  "the backend keeps one depth plane per object, not per level". It records
  only the level drawn (`stamp_depth_target`, `:242-251`).
- `texture::resolve_depth` (`src/gl/texture.rs:699-744`) and `depth_seed`
  (`:972-991`) both read and write `level_off[level]`, that is, face/layer 0
  of the level. `depth_for_draw` (`:842-863`) samples the buffer in place
  only for `GL_TEXTURE_2D`. `draw.rs` `resolve_sampled_depth`
  (`src/gl/draw.rs:3662-3696`) skips cube and array samplers. A cube or
  array depth texture is laid out face after face (or layer after layer)
  inside each level: face `f` starts `f * lh` rows below `level_off[level]`
  (`src/gl/texture.rs:3003`, `:3283`, `:3369`).

### PowerVR (`src/pvr/backend.rs`)

- A buffer's key is `DsKey { kind, id, gen }` (`:1404-1423`). `zs_keys`
  (`:2078-2087`) works in one of two ways. With split on and depth on its
  own object, the key is `DS_DEPTH` under `depth_id` / `depth_gen`.
  Otherwise it is `DS_PACKED` under `ds_id`, for example a depth-stencil
  texture on both attachment points (`depth_id == stencil_id`). Neither key
  has a layer.
- `DsBuf::owners` (`:1442-1445`) holds the `depth_id` form of each plane.
  `ds_settle` (`:4727-4741`) writes it after each kick. `ds_plane_buf`
  (`:4961-4963`) finds a plane by `owners[pl] == id`.
  `pvr_backend_depth_resolve` (`:4910-4958`), `pvr_backend_stencil_resolve`
  (`:4983-5034`) and `pvr_backend_ds_write` (`:5059-`) all look up the
  buffer by owner id.
- A frame's identity is `frame_zs = [depth_id, depth_gen, stencil_id,
  stencil_gen]` (`:5311`). A new frame starts only when that differs
  (`:5782`). So drawing face 1 after face 0 continues in the same buffer,
  and that buffer still holds face 0's depth.
- These entries take any id, a texture's included. That is the path for
  stencil in a texture.

### V3D (`src/v3d/backend.rs`)

- `Target::ds` is `d.ds_id`, and `ds_owners` is `[d.depth_id,
  d.stencil_id]` (`:6308-6309`). `target_zs_bufs` (`:2782-2797`) keys the
  buffers through `zs_keys(ds_id, ..)` (`:2569-2580`). These are name
  halves, with no layer.
- `zs_buf` (`:1612-1636`) finds a buffer by `b.id == id`. The same id at
  another size frees the old buffer ("resized under the same id: the old
  buffer goes back", `:1631-1634`). Drawing another face at the same level
  reuses the buffer. Drawing another level drops the depth that has not
  been resolved yet.
- `ZsBuf::owners` (`:545-548`) holds the object ids.
  `v3d_depth_resolve` (`:4959-4964`) finds the plane by
  `owners[0] == id`. The table has `ds_write: None` and
  `stencil_resolve: None` (`:6745-6746`). The v3d-ds-write item adds them
  in `src/v3d/ds_plane.rs`.

### GC7000UL (`src/vivante`)

- `ds_cache::DsKey` (`src/vivante/ds_cache.rs:52-61`) is `(depth_id,
  depth_gen, stencil_id, stencil_gen, scale)`, built by `key_of` (`:226-243`)
  from the `GlesDraw` fields. A plane follows `PlaneKey { id, gen, scale,
  plane }` (`:63-79`). None of these has a layer.
- `ds_cache::resolve_depth` (`:681-704`) finds the plane by
  `k.id == id && k.scale == 1 && k.plane == DEPTH`. `gc_depth_resolve`
  (`src/vivante/backend.rs:3599-3607`) calls it. The table has
  `ds_write: None` and `stencil_resolve: None` (`:4013-4014`). That code
  belongs to plans #335/#355, so the entries are issue text, not this plan's
  code.

### Mali (`src/mali/backend.rs`, brief)

- A frame's key is `objects: [(depth_id, depth_gen), (stencil_id,
  stencil_gen)]` (`:2730`). `mali_depth_resolve` (`:3434-3454`) finds
  `planes` by `(id, aspect)` and then the zsbuf by `ds_id`. There is no
  layer, and the table has `ds_write` and `stencil_resolve` as None
  (`:3617-3618`).

### What this means today (pre-existing, not blit)

Every back end keeps one plane for a cube or array depth texture, and the GL
core writes that plane back into face/layer 0. So:

1. A draw into face B after face A loads face A's depth as its starting
   depth, unless the draw clears the whole plane.
2. A CPU read (`settle_texture`, a blit, a copy) writes the plane into face
   0 of the level, whichever face was drawn.
3. Sampling a cube or array depth texture after drawing into it does not
   resolve. `resolve_sampled_depth` skips those samplers, so the sampler
   reads texels that are out of date.
4. The same applies to levels on the V3D: drawing level 1 after level 0
   frees level 0's depth before it is resolved.

dEQP may or may not reach these (fbo.render, shadow cases that draw into
cube depth). The measure items count only the blit refusals. Each of these
is a candidate issue (see "For the planner" in the report).

## (b) The GL-core design

**Treat the back end's plane as a cache of one image.** The GL core records
which (level, layer) the plane of a depth texture holds. When a draw or a
blit wants another image, the GL core first resolves the plane into the old
image's texels with `backend::depth_resolve`. Then it marks the texels newer,
so the next draw hands the new image to the back end through
`GlesDraw::depth_texels` / `depth_seed`. All four back ends already write
seeded texels into their plane (`pvr` `ds_seed_depth` `:4466`, `v3d`
`seed_depth` `:2823`, the GC's `ds_cache::seed_depth` `:603`, `mali` `:2663`).
The design uses only existing entries and fields, so **`src/backend.rs` does
not change**. No `GlesDraw`, `GlesTextureBind` or `YakoglBackend` field is
added, so no literal needs updating.

Rejected: one id per face/layer, with the layer encoded in `depth_id` or
given as a new `GlesDraw::depth_layer`. The serial plus bits 30 and 31 leaves
no free bits in the id. A new field is a `src/backend.rs` change (plan
#368's file) and touches every back end's keying. A 6-face cube would also
take 6 of the 32 `ds_retention::DS_SLOTS`, which makes it more likely that
`ds_retention::refuse` refuses a draw. Also rejected: a different
`depth_gen` per layer. The V3D keys by `ds_id` without the gen, and every
`*_depth_resolve` looks up by owner id alone, so two gens of one id would be
ambiguous.

### Functions to change

`src/gl/framebuffer.rs` (plan #364's file: wait for #364, or let connected
order it):

- `DepthTarget` (`:104-108`): add `layer: u16` next to `level`. It is a
  Rust-only static, not part of the C view. `stamp_depth_target`
  (`:242-251`) records `a.layer` with the level. The reuse rules in
  `gl-depth-target-reuse.md` stay as they are. If this has to land before
  #364, use a side table `DEPTH_LAYER: Global<[[u16; MAX_TEXTURES];
  MAX_CONTEXTS]>` in `src/gl/texture.rs` with the same lifetime.

`src/gl/texture.rs`:

- `resolve_depth` (`:699-744`): write the plane at `image_off(t, level,
  layer)` with the recorded layer. Today it writes at `level_off[level]`.
  The byte bound check uses that offset.
- `depth_seed` (`:972-991`): seed from `a.layer`'s rows of `a.level`.
- New `pub(crate) fn depth_image_switch(c, a: &GlAttachment)`. It is called
  from `draw.rs` (`:2729-2737`, before `stamp_depth_target`) and from
  `blit.rs` before a texture plane is read or written. When the recorded
  (level, layer) differs from `a`'s, it calls `resolve_depth` (a no-op when
  `kick == resolved`) and then sets `depth_texels_newer` (through
  `note_depth_texels_written`). It does not set it when the draw clears the
  whole plane: `depth_seed` is already skipped for a `whole_depth_clear`.
  The resolve waits for the kick that drew the old image (`wait_kick`, or
  flush + finish), so the frame drawing into the old image is submitted
  before the seed. The tests have to pin that order.
- New `fn image_off(t, level, layer) -> Option<usize>`: `level_off[level]
  + layer * lh * level_pitch_at(level) * 4`, checked against `t.mem.size`.
  It is the colour formula of `attachment_storage` (`framebuffer.rs:588-590`)
  with 4-byte R32F texels, and it matches the stacked faces at
  `texture.rs:3003`. `blit.rs` uses it too.
- Not required for the blit: `depth_for_draw` stays 2D-only. Making
  `draw.rs` `resolve_sampled_depth` (`:3662`) resolve cube and array slots
  is fix (3) above. It is a separate item.

`src/gl/blit.rs` (shared with plan #339 through connected's ordering):

- `DepthTexels` (`:376-385`): add `layer: GLint`. `off` becomes
  `image_off(t, level, layer)`.
- `depth_texels` (`:388-409`): accept `GL_TEXTURE_CUBE_MAP` and
  `GL_TEXTURE_2D_ARRAY` as well as `GL_TEXTURE_2D`. Check `a.layer` against
  `t.layers.max(1)`, and take `off` from `image_off`. The source
  (`depth_source`, `:840-848`) and the destination (`PlaneDst::Texels`,
  `:1221-1224`) then both reach a face or layer. The `r.image == d.image`
  check (`:1183`) already compares the layer.
- `copy_depth` (`:962-`): before `resolve_depth(c, s.ti)` /
  `resolve_depth(c, d.ti)`, call `depth_image_switch` so that the
  resolve lands in the face that was drawn. Reword the SAFETY comment "a
  source and destination in one texture are different levels" to "different
  images (level or layer)". Two layers of one level are disjoint row ranges.
- Stencil into or out of a texture: allow a `PlaneTexture { id, ti, level,
  layer }` kind on both sides of the stencil match (`:1198-1212`). The
  `rb_src` / `rb_dst` closures (`:1188-1197`) become `plane_src` /
  `plane_dst`. They answer `ds_object_id(t.serial, true)` for a
  `GL_DEPTH_STENCIL` texture only when the texture's plane holds the
  attached image: its `DepthTarget` (level, layer) equals `a`'s, or the
  texture was never drawn into, in which case the `dst_writable` probe
  answers 1 and the blit is refused as `REFUSED_DS_UNHELD`. They also
  require `backend::can_stencil_resolve()` / `can_ds_write()`.
  `read_plane` and the `ds_write` write-back (`:931-945`, `:1068-1080`)
  work unchanged with a texture id: every back end looks up the plane by
  owner id. A plane that holds another face is refused as today
  (`REFUSED_STENCIL`). The GL core keeps no stencil texels
  (`texture.rs:207`, "The stencil byte is not kept"), so a switch would lose
  that face's stencil.
- Update the `REFUSED_DEPTH_SRC` / `REFUSED_DEPTH_DST` / `REFUSED_STENCIL`
  log texts (`:897-909`) to the causes that remain.

Limits of this design:

- **Stencil of a cube or array depth-stencil texture is right only for the
  image the plane holds.** A switch seeds depth only. `ds_seed_depth`
  keeps the stencil byte, so the new face starts with the old face's
  stencil. The fix is a GL-core stencil store per texture, resolved with
  `stencil_resolve` and seeded with `ds_write(DS_PLANE_STENCIL)` at the
  switch. It works only where both entries exist, and it is a follow-up.
- Where there is no `ds_write` (GC7000UL, Mali, and the V3D until
  v3d-ds-write lands), stencil to a texture stays refused, as stencil
  between renderbuffers is today.
- The cost is one resolve and one seed per image switch (a CPU detile on
  the V3D and the GC7000). glmark2 draws only 2D shadow maps, so it is not
  affected.

## (c) Host tests: `tests/es3_blit_ds_layers.rs`

The stand-in back end is copied from `tests/es3_blit_ds.rs`: the read-write
`BACKEND_RW` with `PLANES` keyed by `(id, plane)` and `b_draw` recording
`GlesDraw::depth_texels`. Header: `#![cfg(all(feature = "host-test",
not(feature = "report-es2")))]`. Run `cargo test --test es3_blit_ds_layers
--features host-test,gpu-powervr`. `host-test,gpu-vivante` is the same
stand-in. Like `es3_blit_ds.rs`, it runs in every non-es2 host-test row of
`tools/check.sh`, so no list in check.sh changes.

1. `a_cube_face_is_a_depth_source`: a `GL_DEPTH_COMPONENT32F` cube with a
   different ramp per face (texels written with `glTexImage2D` per face).
   Blit face 3, then face 0, into a 2D depth texture. Each result equals the
   model of `es3_blit_ds.rs` on that face's ramp.
2. `an_array_layer_is_a_depth_source`: the same with a
   `glTexStorage3D(GL_TEXTURE_2D_ARRAY)` depth array and
   `glFramebufferTextureLayer` layers 0 and 2.
3. `a_cube_face_and_a_layer_are_depth_destinations`: a 2D source into face
   5 and into layer 1. Every other face or layer stays byte-equal.
4. `a_drawn_face_resolves_into_its_own_face`: draw (stand-in) into face 2,
   with `HELD` answering a known plane for the texture's id. Blit from face 2:
   the plane lands at face 2's rows and face 0 is unchanged.
5. `switching_faces_resolves_then_seeds`: draw into face 1, then into face
   4. The second draw's `depth_texels` are face 4's texels. Exactly one
   `depth_resolve` ran before it, and it was written to face 1.
6. `a_whole_clear_needs_no_seed`: the same as 5 with `glClear(DEPTH)` on
   face 4. The second draw carries no texels.
7. `a_level_switch_resolves_first`: level 0, then level 1, of a 2D depth
   texture. Level 0's depth is resolved before the level 1 draw (the V3D
   frees the plane at the new size).
8. `stencil_into_and_out_of_a_depth_stencil_texture`: a 2D
   `GL_DEPTH24_STENCIL8` texture drawn once. Its plane is kept under
   `ds_object_id(serial, true)`. Stencil goes renderbuffer → texture and
   texture → renderbuffer through `ds_write` / `stencil_resolve` with the
   texture's id, scaled and scissored as in `stencil_between_renderbuffers_is_copied`.
9. `stencil_of_a_texture_not_drawn_is_refused_unheld`: the probe answers 1,
   so the result is `GL_INVALID_OPERATION` and nothing is written.
10. `stencil_of_another_face_is_refused`: the cube's plane holds face 0 and
    the blit names face 1. The result is `GL_INVALID_OPERATION`
    (`REFUSED_STENCIL`), and `WRITES` stays empty.
11. `same_texture_two_layers_is_not_the_same_image`: blit depth layer 0 →
    layer 1 of one array is allowed. The same layer is still
    `GL_INVALID_OPERATION`.
12. Without `ds_write` (the `BACKEND` stand-in), stencil to a texture is
    still refused, and depth from a face still works.

`tests/es3_blit_ds.rs` `stencil_in_a_texture_stays_refused_with_the_entries`
(`:746-769`) asserts that stencil into and out of a texture is refused (its
window cases moved to `tests/es3_blit_ds_window.rs` with gl-ds-window). The
gl-ds-layers item must reduce it to a texture not drawn (per test 9) or
remove it in the same commit.
