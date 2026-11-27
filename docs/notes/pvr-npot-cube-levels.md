<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The src/gl change that keeps every level of a NPOT PowerVR cube texture

Plan #400, item k3-cube-design. This note names the change behind cause K3 of
`docs/es3/texture-failure-map.md` section 2.2.1: 56 AM62P
`texture.specification` fails, which are NPOT cube maps that keep only level 0
on the PowerVR path. This plan may not edit `src/gl/**` (plan #397 holds it),
so the note gives the exact patch, its host test and the cases it fixes. It
does not apply them. Line numbers are those of the tree this note was written
on (integration tip `3e788de9`; origin/main `2c4b32fa` has no `src/gl` change
beyond it).

## 1. Where the patch comes from

The fix exists as commit `2d215408` "pvr: a NPOT cube map holds its mip chain,
packed, for the native cube's copy". It sits on the unmerged branch
`connected/01a213c2-2083-76e8-91cd-41ba65308811` (remote-tracking ref in the
repository, no other branch contains it), based on `4f34ffb8`. Since then
`58b8fc8f` (GLES3 completeness starts at `GL_TEXTURE_BASE_LEVEL`) rewrote a
line beside one of its `src/gl/draw.rs` comment hunks, so the original diff no
longer applies. A three-way cherry-pick onto `3e788de9` merges without a
conflict. Section 4 is that result, the same change word for word with
moved offsets.

## 2. The cause, traced

The PowerVR back end table sets `native_layered_textures: false` and
`mip_level_align: 0` (`src/pvr/backend.rs:10857`, `:10862`). With those flags:

1. `npot_mip_chains` (`src/gl/texture.rs:1308-1310`) is
   `native_layered_textures() || (mip_level_align() == 0 && layers <= 1)`. It
   is false for the six stacked faces of a cube, so the GL core gives a NPOT
   cube one level of storage. The only stacked exception,
   `stacked_decoded_array` (`:1318-1324`), covers decoded ETC2 2D arrays, not
   cubes.
2. glTexImage2D at level >= 1 (`tex_image_2d`): the grow at `:3047` asks
   `npot_mip_chains` and is skipped. The early return at `:3065`
   (`npot_level && t.chain_levels == 1`) then records the level as defined
   and drops its texels.
3. glTexStorage2D (`:6725`) calls `texture_alloc_storage_ex(..., levels > 1)`.
   That function (`:1949-1961`) builds `NpotChain::from(npot_chain, false)`,
   so `storage_layout` lays out a single level for the stacked NPOT cube.
   glTexSubImage2D at level >= 1 then has nowhere to write.
4. glGenerateMipmap (`:4806`) skips `grow_npot_chain` for the same reason.
   The `t.chain_levels < 2` branch (`:4819`) marks the whole chain as
   present without storage.
5. `grow_npot_chain` (`:1359-1372`) returns at once unless
   `npot_mip_chains || stacked_decoded_array` holds.
6. The draw: `sampling_fields` (`src/gl/draw.rs:694-719`) hands the back end
   `t.levels`, which is 1. The native cube copy (`src/gl/native_cube.rs:181`
   `copy`; the per-level loop at `:255` runs over `t.chain_levels`) and its
   bind (`src/gl/draw.rs:5026-5046`, `GLES_TEXTYPE_CUBE_TWIDDLED_PROBE`) put
   one level in the twiddled copy. The PowerVR descriptor
   (`src/pvr/backend.rs:6306` `texture_descriptor`, `num_mip_levels` in
   IMAGE_WORD1, `:6459`) then describes one level, and every LOD reads level
   0. dEQP sees level 0 where it expects levels 1 to 5, so the cases end with
   "Image comparison failed".

The native cube's own layout (`gl/native_cube.rs` `physical_side`,
`level_texel_off`, twiddle over the power-of-two side) already follows Mesa's
layout for a NPOT side and does not change. Only the GL core's storage was
short. POT cubes are not affected: they always get their whole chain. That is
why every `cube_64x64_*` case passes on the board.

`PVR_NATIVE_CUBE_DEFAULT` is 1 (`src/shaderc/mod.rs:282`), so the native
copy is the default path. The online compile cache key already holds the cube
switch (`switches_now`, `src/shaderc/online/backend.rs:289-300`, bit 2), so
the "side finding" in `2d215408`'s message is settled on main.

## 3. The change in words

- `src/gl/texture.rs`: new `packed_npot_cube(target, w, h, layers)`. It is
  true for a `GL_TEXTURE_CUBE_MAP` of 6 layers that is not power of two in
  both sides and larger than 1x1, on a back end with no native layers and
  `mip_level_align() == 0` (only the PowerVR). New
  `pub(crate) packed_npot_cube_of(t)`: uncompressed and `chain_levels > 1`.
  New `packed_stack(t)`: `stacked_decoded_array(t)`, or an uncompressed
  `packed_npot_cube`. Such a cube's chain is laid out packed and level-major,
  level i being the six faces of max(w >> i, 1) x max(h >> i, 1) stacked, as
  a decoded array's is. `grow_npot_chain`, the glTexImage2D level path and
  glGenerateMipmap accept `packed_stack(t)` beside `npot_mip_chains`.
  `texture_alloc_storage_ex` passes `stacked = npot_chain &&
  packed_npot_cube(..)` to `NpotChain`, so glTexStorage2D lays out the whole
  packed chain.
- `src/gl/draw.rs` (`build_common`, after `sampling_fields`): a
  `GLES_TEXTYPE_CUBE` bind of a `packed_npot_cube_of` texture gets
  `(mip_filter, mip_levels) = (0, 1)`. The bind is still of type CUBE only
  when no native copy was made (a stacked-cube program, or the copy failed).
  A stacked-cube program steps (6h) >> l rows per level and cannot find the
  packed levels, so it samples level 0 alone, as it did before. The native
  copy, which is the default, re-lays every level from `level_off` and keeps
  the whole chain. This rule is deliberately not behind `cfg(feature =
  "gpu-powervr")`. With that gate, `packed_npot_cube_of` was unused in every
  other build and warned `dead_code`, and the gate counts a warning as a
  failed row. The back end flags already keep it to the PowerVR.
- Comment fixes in `src/gl/draw.rs`, `src/gl/state.rs` and
  `src/gl/texture.rs`. The one-level NPOT case on the PowerVR is now the
  stacked 2D array (K4), not the cube.
- The lib test `a_powervr_npot_cube_keeps_its_base_level` becomes
  `a_powervr_npot_cube_holds_its_chain_packed`. A 36x36-face cube with 3
  levels now has `chain_levels` 3, `levels` 3, `level_off[l]` right after
  level l-1's six faces (packed, not the power-of-two step), and level 1 face
  2 is the 2x2 box filter of level 0.

Other GPUs are not reached. The V3D and the GC7000 walk layers
(`native_layered_textures`), and the Mali aligns its levels
(`mip_level_align` != 0).

## 4. The patch

Against `3e788de9` (and origin/main `2c4b32fa`). `git apply --check` accepts
it. To apply it, extract the block and apply it, then add the test of
section 5:

~~~sh
sed -n '/^```diff$/,/^```$/{//!p}' docs/notes/pvr-npot-cube-levels.md | git apply
~~~

```diff
diff --git a/src/gl/draw.rs b/src/gl/draw.rs
index a28d10dc..4c3ef6f4 100644
--- a/src/gl/draw.rs
+++ b/src/gl/draw.rs
@@ -3629,7 +3629,7 @@ fn live_texture(textures: &mut [GlTexture; TEXTURE_SLOTS], h: usize) -> Option<&
  * every build that reports ES 2.0 claims since 2026-10-07 -- so it applies
  * only where texture::NPOT_RESTRICTED holds, which no build does any more.
  * Where the backend keeps one level of a NPOT image (the PowerVR's stacked
- * NPOT cube maps, texture.rs npot_mip_chains) such a texture samples its
+ * NPOT 2D arrays, texture.rs npot_mip_chains) such a texture samples its
  * base level under a mipmap filter.
  *
  * This replaces demoting the filter and sampling level 0, which showed an
@@ -3743,7 +3743,7 @@ fn incomplete_reason(t: &GlTexture) -> Option<&'static str> {
      * can. Levels below the base level are not looked at. */
     if mips {
         /* GL's chain, not the store's: where the backend keeps one level of
-         * a NPOT image (a PowerVR NPOT cube), GL still wants its whole chain, which
+         * a NPOT image (a PowerVR NPOT array), GL still wants its whole chain, which
          * glGenerateMipmap, glTexStorage2D or the application's own levels
          * record as present without holding them */
         let chain = (((1u32 << end) - 1) & !((1u32 << base) - 1)) as u16;
@@ -5083,6 +5083,18 @@ fn build_common(
                 (t.min_filter, t.mag_filter, t.wrap_s, t.wrap_t, t.wrap_r) = s;
             }
             (tb.linear_filter, tb.min_linear, tb.mip_filter, tb.mip_levels) = sampling_fields(t);
+            /* A PowerVR NPOT cube's chain is laid out packed
+             * (texture.rs packed_npot_cube), which only the native cube's
+             * copy re-lays for the TPU. Sampled as the stacked image (a
+             * stacked-cube program, or no copy could be made) the TPU would
+             * find level l at its power-of-two step of (6h) >> l rows, which
+             * is not where it is: level 0 alone, as before the chain was
+             * held. Not gated on the feature: packed_npot_cube is the back
+             * end's flags (no layers walked, packed levels), which only the
+             * PowerVR sets, as the layout in texture.rs is. */
+            if tb.type_ == GLES_TEXTYPE_CUBE && super::texture::packed_npot_cube_of(t) {
+                (tb.mip_filter, tb.mip_levels) = (0, 1);
+            }
             /* The Mali's 3D texture: its back end copies every level of the
              * grid the GL core holds after level 0 (`volume_of`,
              * `grid_level_at`; docs/notes/mali-3d-levels.md section 4), so it
diff --git a/src/gl/state.rs b/src/gl/state.rs
index 5a2aadb1..b8188f0a 100644
--- a/src/gl/state.rs
+++ b/src/gl/state.rs
@@ -145,10 +145,10 @@ const ALL_EXTENSION_NAMES: &[&[u8]] = &[
      * npot_mip_chains); the PowerVR's TPU finds level i+1 by its own
      * power-of-two arithmetic, so a 2D chain there is laid out as the chain
      * of its power-of-two extent, as Mesa's pvr driver lays one out
-     * (texture.rs pot_chain_layout). A stacked NPOT cube map on the PowerVR
-     * keeps its base level, complete, and is sampled from it: the one way
-     * the PowerVR falls short of the extension, right at level 0 and
-     * aliased where minified. */
+     * (texture.rs pot_chain_layout). A NPOT cube map on the PowerVR holds
+     * its chain packed (texture.rs packed_npot_cube), which the native
+     * cube's copy re-lays for the TPU; a stacked-cube program (the native
+     * cube off) samples its level 0 alone. */
     b"GL_OES_texture_npot\0",
     /* GL_OES_depth_texture and GL_OES_packed_depth_stencil.
      *
diff --git a/src/gl/texture.rs b/src/gl/texture.rs
index 127c4782..2a6a364b 100644
--- a/src/gl/texture.rs
+++ b/src/gl/texture.rs
@@ -1300,11 +1300,11 @@ pub(crate) const NPOT_RESTRICTED: bool = crate::REPORT_ES2 && !super::state::NPO
 /// layered texture themselves. The PowerVR's TPU is handed level 0 alone and
 /// finds level i+1 by its own power-of-two arithmetic: a 2D NPOT chain is
 /// laid out as the chain of its power-of-two extent, as Mesa's pvr driver
-/// lays a multi-level image out ([`chain_layout_npot`]). A stacked NPOT cube
-/// there keeps its base level: the emulated cube samples level l as one tall
-/// image of (6h) >> l rows, which is not six faces of h >> l rows once h is
-/// not a power of two. GL's chain is then recorded as present and the draw
-/// samples level 0.
+/// lays a multi-level image out ([`chain_layout_npot`]). Its six stacked
+/// faces of a NPOT cube are not walked by that arithmetic: the emulated cube
+/// samples level l as one tall image of (6h) >> l rows, which is not six
+/// faces of h >> l rows once h is not a power of two. Such a cube holds its
+/// chain packed for the native cube's copy instead ([`packed_npot_cube`]).
 fn npot_mip_chains(layers: u32) -> bool {
     backend::native_layered_textures() || (backend::mip_level_align() == 0 && layers <= 1)
 }
@@ -1323,6 +1323,41 @@ fn stacked_decoded_array(t: &GlTexture) -> bool {
         && backend::mip_level_align() == 0
 }
 
+/// A non-power-of-two cube map of `w x h` faces on a back end that does not
+/// walk layers itself and packs its levels ([`npot_mip_chains`] false for
+/// its six stacked faces: the PowerVR). Its chain is laid out packed, level
+/// i the six faces of max(w >> i, 1) x max(h >> i, 1) stacked, as for a
+/// [`stacked_decoded_array`]: the TPU never walks this storage's chain
+/// itself. The native cube's copy (`gl::native_cube::copy`) re-lays every
+/// level from `level_off` into the face-major twiddled chain of the
+/// power-of-two side the TPU's own cube walks (Mesa 26.2.1
+/// pvr_image_plane_setup_mip_levels), and a stacked-cube program samples
+/// level 0 alone (`gl::draw`, [`packed_npot_cube_of`]). Until this held the
+/// chain, such a cube kept one level, its levels >= 1 recorded and dropped,
+/// and every LOD read level 0 (dEQP-GLES3 texture.specification
+/// *cube_57x57*, AM62P; tests/pvr_npot_cube.rs).
+fn packed_npot_cube(target: GLenum, w: u32, h: u32, layers: u32) -> bool {
+    target == GL_TEXTURE_CUBE_MAP
+        && layers == 6
+        && !(is_pot(w) && is_pot(h))
+        && (w > 1 || h > 1)
+        && !backend::native_layered_textures()
+        && backend::mip_level_align() == 0
+}
+
+/// [`packed_npot_cube`] of texture `t`, uncompressed, as it is laid out now
+/// (a chain of more than one level).
+pub(crate) fn packed_npot_cube_of(t: &GlTexture) -> bool {
+    t.block_bytes == 0 && t.chain_levels > 1 && packed_npot_cube(t.target, t.width, t.height, u32::from(t.layers))
+}
+
+/// A NPOT texture whose chain is laid out packed though its back end does
+/// not walk layers: a [`stacked_decoded_array`] or a [`packed_npot_cube`].
+fn packed_stack(t: &GlTexture) -> bool {
+    stacked_decoded_array(t)
+        || (t.block_bytes == 0 && packed_npot_cube(t.target, t.width, t.height, u32::from(t.layers)))
+}
+
 /// Whether a layout asks for the whole chain of a non-power-of-two size
 /// (`chain`, [`chain_layout_npot`]), and whether it is a stacked decoded array
 /// that gets it though its back end does not walk layers ([`stacked_decoded_array`]).
@@ -1359,7 +1394,7 @@ fn texture_alloc_storage(t: &mut GlTexture, w: u32, h: u32, layers: u32, texfmt:
 fn grow_npot_chain(t: &mut GlTexture, oom: &mut bool) -> bool {
     *oom = false;
     let (w, h) = (t.width, t.height);
-    let stacked = stacked_decoded_array(t);
+    let stacked = packed_stack(t);
     if !(npot_mip_chains(t.layers as u32) || stacked)
         || !matches!(t.target, GL_TEXTURE_2D | GL_TEXTURE_CUBE_MAP | GL_TEXTURE_2D_ARRAY)
         || t.chain_levels != 1
@@ -1955,7 +1990,9 @@ fn texture_alloc_storage_ex(
     one_level: bool,
     npot_chain: bool,
 ) -> i32 {
-    let Some(layout) = storage_layout(w, h, layers, texfmt, one_level, NpotChain::from(npot_chain, false)) else {
+    /* glTexStorage2D of a NPOT cube with levels: packed (packed_npot_cube) */
+    let stacked = npot_chain && packed_npot_cube(t.target, w, h, layers);
+    let Some(layout) = storage_layout(w, h, layers, texfmt, one_level, NpotChain::from(npot_chain, stacked)) else {
         return -1;
     };
     texture_alloc_laid_out(t, w, h, layers, texfmt, layout)
@@ -3044,7 +3081,7 @@ unsafe fn tex_image_2d(
             && (level as u32) < u32::from(context::gl_mip_levels(t.width, t.height))
             && (width as u32, height as u32) == level_size(t, level as u32)
             && t.format == format;
-        if npot_level && npot_mip_chains(t.layers as u32) {
+        if npot_level && (npot_mip_chains(t.layers as u32) || packed_stack(t)) {
             if !settle_texture(c, ti) {
                 c.set_error(GL_OUT_OF_MEMORY, FUNC);
                 return false;
@@ -3059,7 +3096,7 @@ unsafe fn tex_image_2d(
         let t = &mut c.objs().textures[ti];
         let (lw, lh) = level_size(t, level as u32);
         /* GL's level, which a backend that keeps one level of a NPOT image
-         * (a PowerVR NPOT cube, npot_mip_chains) cannot hold: recorded as there, its
+         * (a compressed or external one, or storage that could not grow) cannot hold: recorded as there, its
          * texels dropped, so the chain is complete and samples its base, as
          * after glGenerateMipmap. */
         if npot_level && t.chain_levels == 1 {
@@ -4803,7 +4840,7 @@ pub unsafe extern "C" fn glGenerateMipmap(target: GLenum) {
      * the texture, whose texels are copied across. Elsewhere nothing is
      * waited for: glmark2's terrain asks again every frame for a texture it
      * has just rendered into. */
-    if npot && filtered && t.chain_levels < 2 && npot_mip_chains(t.layers as u32) {
+    if npot && filtered && t.chain_levels < 2 && (npot_mip_chains(t.layers as u32) || packed_stack(t)) {
         if !settle_texture(c, ti) {
             c.set_error(GL_OUT_OF_MEMORY, FUNC);
             return;
@@ -4818,7 +4855,7 @@ pub unsafe extern "C" fn glGenerateMipmap(target: GLenum) {
     let t = &mut c.objs().textures[ti];
     if t.chain_levels < 2 {
         /* No chain in the store: a NPOT image where the backend keeps one
-         * level (a PowerVR NPOT cube, npot_mip_chains), or a 1x1. GL's chain exists
+         * level (a PowerVR NPOT array, npot_mip_chains), or a 1x1. GL's chain exists
          * all the same (ES 2.0 3.7.11), so the texture is complete under a
          * mipmap filter and samples its base level. It was left incomplete,
          * and an ES 3.0 build sampled such a texture black. */
@@ -10741,12 +10778,14 @@ mod tests {
         assert!(t.mem.size >= size as usize);
     }
 
-    /// A NPOT cube map keeps its base level on the PowerVR's shape: the
-    /// emulated cube samples level l as one tall image (6h) >> l rows high,
-    /// which is not six faces of h >> l rows once h is not a power of two
-    /// (`npot_mip_chains`). GL's chain is recorded, the texture complete.
+    /// A NPOT cube map holds its chain on the PowerVR's shape, laid out
+    /// packed ([`packed_npot_cube`]): level l is the six faces of max(6 >> l,
+    /// 1) rows stacked, right after level l - 1 (not at the power-of-two
+    /// step of the stacked height, 64 rows for 36), which the native cube's
+    /// copy re-lays for the TPU. It kept its base level alone until the
+    /// texture.specification cube_57x57 cases (tests/pvr_npot_cube.rs).
     #[test]
-    fn a_powervr_npot_cube_keeps_its_base_level() {
+    fn a_powervr_npot_cube_holds_its_chain_packed() {
         let gl = TestGl::new(4, 4);
         let mut name: GLuint = 0;
         let face = ramp(36);
@@ -10771,8 +10810,25 @@ mod tests {
         }
         assert_eq!(gl.error(), GL_NO_ERROR);
         let t = &gl.ctx().objs().textures[name as usize];
-        assert_eq!((t.chain_levels, t.pot_chain), (1, false));
+        assert_eq!((t.chain_levels, t.levels, t.pot_chain), (3, 3, false));
         assert_eq!((t.level_defined, t.level_bad), (0b111, 0), "GL's chain recorded");
+        assert!(packed_npot_cube_of(t));
+        for l in 1..3u32 {
+            let (_, ph) = level_size(t, l - 1);
+            let prev = t.level_off[l as usize - 1] + t.level_pitch_at(l - 1) * ph * 6 * 4;
+            assert_eq!(t.level_off[l as usize], prev, "level {l} right after level {}", l - 1);
+        }
+        /* level 1 face 2 (3x3) is face 2's 2x2 boxes, as for a 2D chain */
+        // SAFETY: the storage's CPU view, which the generation wrote.
+        let s = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) };
+        let at = |l: u32, f: u32, x: u32, y: u32| {
+            (t.level_off[l as usize] + ((f * level_size(t, l).1 + y) * t.level_pitch_at(l) + x) * 4) as usize
+        };
+        for c in 0..4 {
+            let sum: u32 =
+                [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|&(x, y)| u32::from(s[at(0, 2, x, y) + c])).sum();
+            assert_eq!(u32::from(s[at(1, 2, 0, 0) + c]), (sum + 2) / 4, "level 1 face 2 (0, 0) channel {c}");
+        }
     }
 
     /// The level sizes of glmark2 terrain's 800x600 chain (GL_OES_texture_npot:
```

## 5. The host test the applier must add

`tests/pvr_npot_cube.rs`, taken whole from `2d215408`:
`git show 2d215408:tests/pvr_npot_cube.rs > tests/pvr_npot_cube.rs` (blob
`4ad5bf934fe6`, 654 lines, SPDX header included). Feature set
`host-test,gpu-powervr,shaderc-online` (nothing under `report-es2`):
`cargo test --features host-test,gpu-powervr,shaderc-online --test
pvr_npot_cube`. It runs in the `tools/check.sh` test rows of those
configurations, which run every target of `tests/`.

The test uploads six faces with a distinct value for every (face, level, x,
y) and draws one quad through a `samplerCube` with NEAREST_MIPMAP_NEAREST on a
stand-in back end that has the PowerVR's flags. It takes the bind the draw
hands the back end and the descriptor that
`yakogl::pvr::backend::texture_descriptor` builds from it. It then reads the
bytes through a model of the TPU that knows only IMAGE_WORD0/1, the layer
stride and SAMPLER_WORD0, walking Mesa 26.2.1 pvr's twiddled layout. The
tests:

| test | asserts |
|---|---|
| `the_model_walks_mesas_twiddled_chain` | the TPU model's own layout |
| `pot_cube_every_level_lands_where_the_descriptor_points`, `pot_cube_each_level_samples_its_colour` | 64x64 control, 2 and 7 levels |
| `npot_cube_level_0_lands_where_the_descriptor_points` | 57x57, level 0 |
| `npot_cube_every_level_lands_where_the_descriptor_points` | 57x57 with 2 and 6 levels, through glTexImage2D, glTexStorage2D + glTexSubImage2D, and glTexSubImage2D in halves: every texel of every face and level is where the descriptor points |
| `npot_cube_each_level_samples_its_colour` | each LOD reads its own level |
| `a_stacked_cube_program_samples_level_0_alone` | with `pvr_native_cube` off, the bind gets level 0 alone (fails with the draw.rs clamp removed) |

Measured on `3e788de9` with section 4 applied plus this file:
`pvr_npot_cube` 7 passed. Without the `src/gl` part,
`npot_cube_every_level_lands_where_the_descriptor_points` and
`npot_cube_each_level_samples_its_colour` fail (5 passed, 2 failed), so the
test pins the fix. `rustfmt --check` is clean on the four files.

## 6. Tests and references that change

- Changed in the patch: the lib test in `src/gl/texture.rs`, renamed and
  rewritten (section 3). Measured: `--lib gl::texture::` 329 passed under
  `host-test,gpu-powervr,shaderc-online`.
- Unchanged and green with the patch (same feature set, measured):
  `es2_npot` (18), `pvr_npot_levels` (4), `es3_pvr_pcf_cube` (4),
  `pvr_int_cube` (1), `pvr_mip_chain_pin` (2), `pvr_mipgen_program` (1),
  `pvr_native_switch_cache` (5). `cargo build --features host-test,gpu-vivante`
  builds without a warning (the `dead_code` check of section 3).
- `tests/reference/*.rs`, `tests/fixtures/`, `src/pvr/internal.rs`: none. The
  patch changes storage layout and draw state, not a compiled shader or a
  pinned descriptor of a POT texture.
- Documentation that goes stale when it lands, for the applier to update in
  the same commit or to name: `docs/es3/texture-failure-map.md` 2.2.1 (K3
  row "Pass with the src/gl fix" and the K3 paragraph, which call the test
  "not in this tree"), and `docs/notes/pvr-mipmap-cpu-cases.md:16` ("the
  PowerVR NPOT cube keeps one level").
- Memory: a NPOT cube with levels now holds its packed chain (about 4/3 of
  level 0) beside the native copy, where before it held level 0 only.

## 7. The dEQP cases it fixes (K3)

56 AM62P `Fail` lines of `docs/es3/am62p-gles3-notpass.tsv`, all "Image
comparison failed":

- `dEQP-GLES3.functional.texture.specification.texstorage2d.format.*_cube`:
  all 54 of the group in the mustpass list (57x57, full chain), every format
  from `depth24_stencil8_cube` to `srgb8_cube`. `rgba32f_cube` also needs K2
  (`ecd9743e`, on the tip).
- `dEQP-GLES3.functional.texture.specification.texstorage2d.size.cube_57x57_2_levels`
  and `.cube_57x57_6_levels`. (`cube_57x57_1_levels` already passes.)

Expected after the patch: Pass, on a build with the native cube on (the
default). A board run of these 56 is still to do. The 31cdaefc board images
of plan #400 do not carry the fix.

Not claimed: `texture.format.sized.cube.*` (56) fails as much for `_pot` as
for `_npot` (integer formats, another cause). The remaining
`texture.mipmap.cube.*` fails (9) and `texture.filtering.cube.formats.*` (12)
are not attributed to this cause here.
