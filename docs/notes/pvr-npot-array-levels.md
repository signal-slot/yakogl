<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The src/gl change that keeps every level of a NPOT PowerVR 2D array

Plan #400, item k4-array-design. This note names the change behind cause K4 of
`docs/es3/texture-failure-map.md` section 2.2.1: 76 AM62P
`texture.specification` fails, which are NPOT 2D-array textures that keep only
level 0 on the PowerVR path. This plan may not edit `src/gl/**` (plan #397
holds it), so the note gives the exact patch, its host test and the cases it
fixes. It does not apply them. Line numbers are those of main at `472630df`
(integration tip, 2026-11-27).

## 1. Does the native array alone fix K4? No

`PVR_NATIVE_ARRAY_DEFAULT` has been 1 since `02c5c9af` "pvr: native 2d-array
textures by default" (`src/shaderc/mod.rs:314`). The failure map's "now 0"
predates that commit. So native arrays are already the default at the tip,
and K4 is still there:

- The native copy (`src/gl/native_array.rs:176` `copy`) copies
  `have = min(t.chain_levels, chain)` levels (`:230`). For a NPOT array the
  GL core holds one level of storage (`chain_levels` 1, section 2), so the
  copy holds level 0 alone. The bind's `mip_levels` is `t.levels` (1), so
  the descriptor says one level and every LOD reads level 0.
- The power-of-two arrays pass on the native path
  (`tests/es3_pvr_2darray.rs`, the 64x32x3 7-level case) because a POT array
  always gets its whole chain in storage. Nothing in the copy or the
  descriptor depends on the size being a power of two. The copy already
  rounds each layer up to its power-of-two extent (Mesa's layout), and that
  holds for 57x44 too: level 0 of a 57x44x5 array lands where the descriptor
  points on every layer (`npot_array_level_0_lands_where_the_descriptor_points`
  below, which passes on main without the patch).
- Measured on `472630df` with native arrays on (the default) and without the
  `src/gl` part of section 4: `tests/pvr_npot_array_chain.rs` gives 5 passed,
  3 failed. The fails are `npot_array_every_level_lands_where_the_descriptor_points`,
  `npot_array_each_level_samples_its_colour` and
  `npot_array_generate_mipmap_holds_the_chain`.

So the stacked storage must change: the GL core has to hold the NPOT array's
chain. The native copy and the PowerVR back end need no change.

## 2. The cause, traced

The PowerVR back end table sets `mip_level_align: 0` and
`native_layered_textures: false` (`src/pvr/backend.rs:10857`, `:10862`).
With those flags:

1. `npot_mip_chains` (`src/gl/texture.rs:1308-1310`) is
   `native_layered_textures() || (mip_level_align() == 0 && layers <= 1)`. It
   is false for any array of more than one layer. The only stacked exception
   is `stacked_decoded_array` (`:1318-1324`), and it requires
   `t.decoded_from != 0`: a decoded ETC2/EAC array. An uncompressed NPOT
   array gets no chain.
2. glTexImage3D at level >= 1 (`tex_image_3d`, `:5791`): the grow at `:5906`
   asks `npot_mip_chains || stacked_decoded_array`, both false, so it is
   skipped. The early return at `:5927` (`npot_level && t.chain_levels == 1`)
   then records the level as defined and drops its texels.
   basic_teximage3d and texsubimage3d_depth take this path.
3. glTexStorage3D (`:6828`) calls `texture_alloc_storage_ex(..., levels > 1)`.
   That function (`:1949-1961`) builds `NpotChain::from(npot_chain, false)`,
   and `storage_layout` (`:1979`) drops `npot_chain` because
   `npot_mip_chains(layers)` is false. One level is laid out, and `t.levels`
   becomes `levels.min(t.chain_levels)` = 1 (`:6840`). glTexSubImage3D at
   level >= 1 is then `recorded_unheld` (`:2536-2545`, `:6036`) and dropped.
   texstorage3d.format and texstorage3d.size take this path.
4. glGenerateMipmap (`:4806`) asks only `npot_mip_chains` and skips the
   grow. The `t.chain_levels < 2` branch marks the chain present without
   storage. No K4 case does this, but it is the same rule.
5. `grow_npot_chain` (`:1359-1386`) returns at once unless
   `npot_mip_chains || stacked_decoded_array` holds (`:1362-1363`).
6. The draw: `sampling_fields` (`src/gl/draw.rs:711`) hands the back end
   `t.levels` (1). The native array copy (`src/gl/draw.rs:5088-5103`,
   `native_array::copy`) holds `have` = 1 level, and the twiddled descriptor
   (`src/pvr/backend.rs:6546-6590`, `GLES_TEXTYPE_2D_ARRAY_TWIDDLED`; `single`
   at `:6575`) describes one level. dEQP sees level 0 where it expects levels
   1 to 5, so the cases end with "Image comparison failed".

The packed layout the fix needs already exists. `storage_layout` with
`NpotChain { stacked: true }` (`:1975-1979`) turns `pot_stack` off and lays
level i out as the `layers` images of max(w >> i, 1) x max(h >> i, 1) right
after level i-1. That is what decoded ETC2 arrays use today, and it is what
`native_array::copy` reads (`level_off[l] + ((k * lh + y) * pitch + x) * bpp`).
The fix only lets an uncompressed NPOT array into that rule.

## 3. The change in words, and the draw rule

- `src/gl/texture.rs`: new `packed_npot_array(target, w, h, layers)`. It is
  true for a `GL_TEXTURE_2D_ARRAY` of more than one layer that is not power
  of two in both sides and is larger than 1x1, on a back end with no native
  layers and `mip_level_align() == 0` (only the PowerVR).
  `stacked_decoded_array(t)` becomes `t.block_bytes == 0 &&
  packed_npot_array(..)`. The `decoded_from != 0` condition goes: a decoded
  array's storage is uncompressed, so it is still covered, and so is every
  uncompressed NPOT array now. The name stays so that the callers, and
  `packed_stack` of the K3 patch, read as before. New `pub(crate)
  packed_npot_array_of(t)`: `chain_levels > 1 && stacked_decoded_array(t)`.
  `texture_alloc_storage_ex` passes `stacked = npot_chain &&
  packed_npot_array(..)` to `NpotChain`, so glTexStorage3D lays out the whole
  packed chain. glGenerateMipmap also grows the chain when
  `stacked_decoded_array(t)` holds. The glTexImage3D level path already asks
  `stacked_decoded_array`, so it now grows these arrays. Stale comments are
  fixed (glTexImage3D's two, `recorded_unheld`, glTexSubImage3D's `unheld`).
- **The draw rule**, `src/gl/draw.rs` (`build_common`, right after
  `sampling_fields`, after the sampler object's filters are in force): a
  `GLES_TEXTYPE_2D_ARRAY` bind of a `packed_npot_array_of` texture gets
  `(mip_filter, mip_levels) = (0, 1)`.
  - When the native copy was made, the bind's type is already
    `GLES_TEXTYPE_2D_ARRAY_TWIDDLED` and the rule does not touch it. The copy,
    the default, re-lays every level from `level_off` and keeps the whole
    chain.
  - The bind stays `GLES_TEXTYPE_2D_ARRAY` only when no copy was made: a
    stacked-array program (`pvr_native_array` off, `DEQP_PVR_NATIVE_ARRAY=0`),
    a render into the array in the open frame, or a copy that found no
    memory. A stacked program samples the tall image through `pvr_arrUV`,
    and the descriptor steps level l by the power-of-two-rounded stacked
    height. That walk cannot find the packed levels, so such a draw samples
    level 0 alone, as it did before.
  - The rule also fixes a latent fault: a decoded ETC2 NPOT array on the
    stacked path was handed its packed chain as mip levels.
  - The rule is not gated on `cfg(feature = "gpu-powervr")`, for the same
    reason as the K3 cube rule: the flags already confine it to the PowerVR,
    and a gate would make `packed_npot_array_of` dead code (a warning, which
    the gate counts as a failed row) in the other builds.

Other GPUs are not reached. The V3D and the GC7000 walk layers
(`native_layered_textures`), and the Mali aligns its levels
(`mip_level_align` != 0). Measured: `cargo check --no-default-features
--features host-test,gpu-vivante`, `host-test,gpu-mali,shaderc-online` and
`host-test,gpu-v3d,shaderc-online` give no warning with the patch.

Memory: a NPOT array with levels now holds its packed chain (about 4/3 of
level 0) beside the native copy. Before, it held level 0 only.

## 4. The patch, against main

Against `472630df`. `git apply --check` accepts it on that commit. It
includes the new host test (section 6). To apply both:

~~~sh
sed -n '/^```diff$/,/^```$/{//!p}' docs/notes/pvr-npot-array-levels.md | git apply
~~~

```diff
diff --git a/src/gl/draw.rs b/src/gl/draw.rs
index 95806e71..c5d2243c 100644
--- a/src/gl/draw.rs
+++ b/src/gl/draw.rs
@@ -5112,6 +5112,18 @@ fn build_common(
                 (t.min_filter, t.mag_filter, t.wrap_s, t.wrap_t, t.wrap_r) = s;
             }
             (tb.linear_filter, tb.min_linear, tb.mip_filter, tb.mip_levels) = sampling_fields(t);
+            /* A PowerVR NPOT 2D array's chain is laid out packed
+             * (texture.rs packed_npot_array), which only the native array's
+             * copy re-lays for the TPU. Sampled as the stacked image (a
+             * stacked-array program, or no copy could be made) the TPU would
+             * find level l at its power-of-two step of the stacked height,
+             * which is not where it is: level 0 alone, as before the chain
+             * was held. Not gated on the feature: packed_npot_array is the
+             * back end's flags (no layers walked, packed levels), which only
+             * the PowerVR sets, as the layout in texture.rs is. */
+            if tb.type_ == GLES_TEXTYPE_2D_ARRAY && super::texture::packed_npot_array_of(t) {
+                (tb.mip_filter, tb.mip_levels) = (0, 1);
+            }
             /* The Mali's 3D texture: its back end copies every level of the
              * grid the GL core holds after level 0 (`volume_of`,
              * `grid_level_at`; docs/notes/mali-3d-levels.md section 4), so it
diff --git a/src/gl/texture.rs b/src/gl/texture.rs
index 127c4782..06f6fe8e 100644
--- a/src/gl/texture.rs
+++ b/src/gl/texture.rs
@@ -1309,20 +1309,42 @@ fn npot_mip_chains(layers: u32) -> bool {
     backend::native_layered_textures() || (backend::mip_level_align() == 0 && layers <= 1)
 }
 
-/// A non-power-of-two 2D array of a decoded ETC2/EAC name on a back end that
-/// does not walk layers itself ([`npot_mip_chains`] false). Its GL levels all
-/// have texels (a decoded name's uploads must land somewhere): the chain is
-/// laid out packed, level i of `layers` images of max(w >> i, 1) x max(h >> i,
-/// 1), and the array copy (`native_array::copy`) puts it layer-major for the
-/// TPU. Uncompressed arrays keep one level there ([`npot_mip_chains`]).
-fn stacked_decoded_array(t: &GlTexture) -> bool {
-    t.target == GL_TEXTURE_2D_ARRAY
-        && t.decoded_from != 0
-        && t.layers > 1
+/// A non-power-of-two 2D array of `w x h` layers on a back end that does
+/// not walk layers itself and packs its levels ([`npot_mip_chains`] false
+/// for `layers` > 1: the PowerVR). Its chain is laid out packed, level i the
+/// `layers` images of max(w >> i, 1) x max(h >> i, 1) stacked: the TPU never
+/// walks this storage's chain itself. The array copy (`native_array::copy`)
+/// re-lays every level from `level_off` into the layer-major twiddled chain
+/// of the power-of-two extent the TPU walks (Mesa 26.2.1
+/// pvr_image_plane_setup_mip_levels), and a stacked-array program samples
+/// level 0 alone (`gl::draw`, [`packed_npot_array_of`]). Until this held
+/// the chain of an uncompressed array (it held a decoded ETC2/EAC one's
+/// only), such an array kept one level, its levels >= 1 recorded and
+/// dropped, and every LOD read level 0 (dEQP-GLES3 texture.specification
+/// *_2d_array 57x44x5 and 57x63x5, AM62P; tests/pvr_npot_array_chain.rs).
+fn packed_npot_array(target: GLenum, w: u32, h: u32, layers: u32) -> bool {
+    target == GL_TEXTURE_2D_ARRAY
+        && layers > 1
+        && !(is_pot(w) && is_pot(h))
+        && (w > 1 || h > 1)
         && !backend::native_layered_textures()
         && backend::mip_level_align() == 0
 }
 
+/// [`packed_npot_array`] of texture `t`, uncompressed (a decoded ETC2/EAC
+/// array included: its storage is the decoded texels). The name is the one
+/// the decoded array had alone, kept so the callers read as before.
+fn stacked_decoded_array(t: &GlTexture) -> bool {
+    t.block_bytes == 0 && packed_npot_array(t.target, t.width, t.height, u32::from(t.layers))
+}
+
+/// [`stacked_decoded_array`] of texture `t` as it is laid out now (a chain
+/// of more than one level): the draw keeps a stacked-array bind of it to
+/// level 0.
+pub(crate) fn packed_npot_array_of(t: &GlTexture) -> bool {
+    t.chain_levels > 1 && stacked_decoded_array(t)
+}
+
 /// Whether a layout asks for the whole chain of a non-power-of-two size
 /// (`chain`, [`chain_layout_npot`]), and whether it is a stacked decoded array
 /// that gets it though its back end does not walk layers ([`stacked_decoded_array`]).
@@ -1955,7 +1977,9 @@ fn texture_alloc_storage_ex(
     one_level: bool,
     npot_chain: bool,
 ) -> i32 {
-    let Some(layout) = storage_layout(w, h, layers, texfmt, one_level, NpotChain::from(npot_chain, false)) else {
+    /* glTexStorage3D of a NPOT 2D array with levels: packed (packed_npot_array) */
+    let stacked = npot_chain && packed_npot_array(t.target, w, h, layers);
+    let Some(layout) = storage_layout(w, h, layers, texfmt, one_level, NpotChain::from(npot_chain, stacked)) else {
         return -1;
     };
     texture_alloc_laid_out(t, w, h, layers, texfmt, layout)
@@ -2534,9 +2558,10 @@ pub(crate) fn level_size(t: &GlTexture, level: u32) -> (u32, u32) {
 }
 
 /// A level GL has (glTexStorage*, or a glTexImage* that fit its chain) but
-/// the backend is not told of: a chain the backend cannot hold (a NPOT array
-/// or cube on the PowerVR, `npot_mip_chains`), whose sub-images are checked
-/// and then dropped, as its images are; or a level above 0 of a 3D grid,
+/// the backend is not told of: a chain the backend cannot hold (a NPOT cube
+/// on the PowerVR, `npot_mip_chains`, or storage that could not grow), whose
+/// sub-images are checked and then dropped, as its images are; or a level
+/// above 0 of a 3D grid,
 /// which the store does hold ([`grid_level_held`]).
 fn recorded_unheld(t: &GlTexture, level: GLint) -> bool {
     level as u32 >= t.levels as u32
@@ -4803,7 +4828,7 @@ pub unsafe extern "C" fn glGenerateMipmap(target: GLenum) {
      * the texture, whose texels are copied across. Elsewhere nothing is
      * waited for: glmark2's terrain asks again every frame for a texture it
      * has just rendered into. */
-    if npot && filtered && t.chain_levels < 2 && npot_mip_chains(t.layers as u32) {
+    if npot && filtered && t.chain_levels < 2 && (npot_mip_chains(t.layers as u32) || stacked_decoded_array(t)) {
         if !settle_texture(c, ti) {
             c.set_error(GL_OUT_OF_MEMORY, FUNC);
             return;
@@ -5892,9 +5917,9 @@ unsafe fn tex_image_3d(
         /* A level of a non-power-of-two array's chain (ES 3.0 3.8.14: floor
          * sizes down to 1x1, the layer count the same at every level). The
          * store has one level of such an array until a backend that holds
-         * the chain is asked for it (grow_npot_chain); one that cannot (the
-         * PowerVR's stacked NPOT layers, npot_mip_chains) records the level
-         * as GL has it and keeps no texels, as for a NPOT cube. */
+         * the chain is asked for it (grow_npot_chain; the PowerVR's stacked
+         * NPOT layers packed, packed_npot_array). Storage that cannot grow
+         * records the level as GL has it and keeps no texels. */
         let npot_level = t.level_defined & 1 != 0
             && t.level_bad & 1 == 0
             && t.chain_levels == 1
@@ -5916,13 +5941,10 @@ unsafe fn tex_image_3d(
             }
         }
         let t = &mut c.objs().textures[ti];
-        /* Only the PowerVR's tall stacked storage gets here (npot_mip_chains
-         * false: layers > 1, no native layers): a stacked NPOT array cannot
-         * hold its chain, so the level is recorded and its texels dropped,
-         * and draws sample level 0. Left as it is on purpose: plan item
-         * pvr-2darray (docs/es3/pvr-2darray.md) lays PowerVR arrays out
-         * layer-major, and their levels >= 1 get storage there. Every other
-         * array keeps the texels of each level (grown above, or a
+        /* Storage that could not grow above (no CPU view, an external
+         * image): the level is recorded and its texels dropped, and draws
+         * sample level 0. Every other array keeps the texels of each level
+         * (grown above, packed on the PowerVR (packed_npot_array), or a
          * power-of-two chain). */
         if npot_level && t.chain_levels == 1 {
             t.level_defined |= 1u16 << (level as u32).min(15);
@@ -6033,8 +6055,8 @@ unsafe fn tex_sub_image_3d(
     }
     let t = &c.objs_ref().textures[ti];
     let grid_cols = t.grid_cols as u32;
-    /* a level the store keeps no texels for (a PowerVR NPOT array's above
-     * 0, see glTexImage3D); a 3D grid's levels above 0 are held after level
+    /* a level the store keeps no texels for (storage that could not grow,
+     * see glTexImage3D); a 3D grid's levels above 0 are held after level
      * 0's grid though the backend is told of level 0 alone */
     let unheld = recorded_unheld(t, level) && !grid_level_held(t, level as u32);
     let (lw, lh) = level_size(t, level as u32);
diff --git a/tests/pvr_npot_array_chain.rs b/tests/pvr_npot_array_chain.rs
new file mode 100644
index 00000000..cd84dd05
--- /dev/null
+++ b/tests/pvr_npot_array_chain.rs
@@ -0,0 +1,703 @@
+// SPDX-License-Identifier: Apache-2.0
+// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
+//! PowerVR (AM62P): the levels of a non-power-of-two 2D array sampled
+//! through the native array (`gl::native_array`, `pvr_native_array`, on by
+//! default since 02c5c9af), pinned layer by layer and level by level
+//! (dEQP-GLES3 `texture.specification.basic_teximage3d.*_2d_array` 57x44x5,
+//! `texstorage3d.format.*_2d_array` 57x44x5, `texstorage3d.size.
+//! 2d_array_57x63x5_2_levels`, `texsubimage3d_depth.*_2d_array`; cause K4 of
+//! docs/es3/texture-failure-map.md 2.2.1) beside a power-of-two control
+//! (64x32x3, 2 and 7 levels).
+//!
+//! Each case uploads the layers with a distinct value for every (layer,
+//! level, x, y), draws one quad through a `sampler2DArray` with
+//! NEAREST_MIPMAP_NEAREST on a stand-in back end with the PowerVR's flags,
+//! takes the texture bind the draw hands the back end and the descriptor
+//! the PowerVR back end builds from it
+//! (`yakogl::pvr::backend::texture_descriptor`), and then reads the bytes
+//! through a model of the TPU that knows only those words:
+//!
+//! - IMAGE_WORD0 (texstate.xml): textype at [0:2] (2D = 1), width - 1 at
+//!   [34:47], height - 1 at [48:61], one layer's API extent;
+//! - the twiddled IMAGE_WORD1: num_mip_levels at [0:3], depth (layers - 1)
+//!   at [4:14], mipmaps_present at [15], texaddr (>> 2) at [16:53];
+//! - image meta dword 4: one layer's bytes (the shader's layer stride,
+//!   PCO_IMAGE_META_LAYER_SIZE);
+//! - SAMPLER_WORD0: minlod at [13:22], maxlod at [23:32] (u4.6).
+//!
+//! The layout the model walks is Mesa 26.2.1 pvr's for a twiddled image
+//! (`pvr_image.c`, not vendored; the rules gl/native_array.rs cites): the
+//! physical extent is the API extent rounded up to a power of two per
+//! axis, level l is u_minify(physical, l) per axis in twiddled order (y the
+//! low bit, the longer axis' remaining bits linear above the interleave),
+//! the levels follow each other from the layer's start, and layer k starts
+//! k layer strides from texaddr. A NEAREST_MIPMAP_NEAREST look-up at an
+//! integer LOD L reads level min(L, maxlod, num_mip_levels - 1), or level
+//! 0 without mipmaps_present.
+//!
+//! What it pins (k4-array-design, plan #400; docs/notes/
+//! pvr-npot-array-levels.md). The native copy's layout is Mesa's for the
+//! NPOT extent: level 0 of the 57x44 array lands where the descriptor
+//! points on every layer, and gl/native_array.rs needs no change. The
+//! failures are upstream of the copy: the GL core kept ONE level of
+//! storage for an uncompressed NPOT array on the PowerVR (src/gl/texture.rs
+//! `npot_mip_chains` is false for stacked layers without native layers,
+//! and `stacked_decoded_array` held the chain of a decoded ETC2 array
+//! only), so a level above 0 was recorded and its texels dropped, the
+//! texture had `levels` 1, and every LOD read level 0. Such an array now
+//! holds its chain laid out packed (texture.rs `packed_npot_array`: grown
+//! on the glTexImage3D level path and by glGenerateMipmap, laid out whole
+//! by glTexStorage3D), which the native copy re-lays level by level; a
+//! stacked-array program (`pvr_native_array` off) samples level 0 alone
+//! (gl/draw.rs), as its power-of-two-stepped walk of the stacked image
+//! cannot find the packed levels (`a_stacked_array_program_samples_level_0_alone`).
+//!
+//! ```text
+//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_npot_array_chain
+//! ```
+//!
+//! Needs the in-process compiler (the quad's programs are not in the
+//! offline table). Compiles to nothing under report-es2: the cases are ES
+//! 3.0's (glTexImage3D, glTexStorage3D, sampler2DArray). The tools/check.sh
+//! rows that run it are the test rows of the host-test,gpu-powervr,
+//! shaderc-online configurations, which run every target of tests/.
+
+#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
+
+use core::ffi::{c_char, c_void};
+use std::ffi::CString;
+use std::sync::Mutex;
+use yakogl::backend::{
+    GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_TEXTYPE_2D_ARRAY, GLES_TEXTYPE_2D_ARRAY_TWIDDLED,
+};
+use yakogl::gl::consts::*;
+use yakogl::gl::draw::*;
+use yakogl::gl::program::*;
+use yakogl::gl::shader::*;
+use yakogl::gl::state::*;
+use yakogl::gl::texture::*;
+use yakogl::gl::types::*;
+use yakogl::gl::vertex::*;
+use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
+
+/* ---- the stand-in back end: it records the texture bind of each draw ------- */
+
+// SAFETY (Send): GlesTextureBind holds a raw `cpu` pointer; the tests read it
+// only on the thread that drew, while the context lives.
+struct SendBind(GlesTextureBind);
+unsafe impl Send for SendBind {}
+static BIND_SLOT: Mutex<Option<SendBind>> = Mutex::new(None);
+
+unsafe extern "C" fn b_init() -> i32 {
+    0
+}
+unsafe extern "C" fn b_heap_base(_h: GlesHeap) -> u64 {
+    0x1000_0000
+}
+unsafe extern "C" fn b_ok() -> i32 {
+    0
+}
+unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
+    // SAFETY: the core passes a live job.
+    let d = unsafe { &*d };
+    if d.num_textures > 0 {
+        *BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = Some(SendBind(d.textures[0]));
+    }
+    0
+}
+
+/// The PowerVR's flags (src/pvr/backend.rs `yakogl_backend_pvr`): packed mip
+/// levels (`mip_level_align` 0), the TPU walks no layers itself.
+static PVR_SHAPE: YakoglBackend = YakoglBackend {
+    name: c"pvr npot array stand-in".as_ptr() as *const c_char,
+    native_attrib_formats: 0,
+    rt_rgba8888_is_bgra: false,
+    mip_level_align: 0,
+    compressed_blocks_in_rows: false,
+    native_layered_textures: false,
+    front_facing_shader_variant: false,
+    etc1_decoded_at_upload: false,
+    init: Some(b_init),
+    shutdown: None,
+    draw: Some(b_draw),
+    flush: Some(b_ok),
+    finish: Some(b_ok),
+    finish_previous: None,
+    kick_id: None,
+    kick_pending: None,
+    heap_base: Some(b_heap_base),
+    counters: None,
+    texture_released: None,
+    surface_swapped: None,
+    texture_uploaded: None,
+    max_instance_divisor: 1,
+    wait_kick: None,
+    samples_texel_copies: false,
+    attachment_gone: None,
+    depth_resolve: None,
+    query_begin: None,
+    query_end: None,
+    query_result: None,
+    generate_mipmap: None,
+    return_mipmap_chain: None,
+    linear_row_align: 0,
+    uniforms_host_only: false,
+    ds_write: None,
+    stencil_resolve: None,
+    max_samples: 0,
+};
+
+/// Pointer-valued device addresses: a GPU address is its CPU pointer.
+unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
+    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
+    let cpu = mem.as_mut_ptr() as *mut c_void;
+    // SAFETY: `out` is the core's.
+    unsafe { *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu } };
+    0
+}
+unsafe extern "C" fn o_gpu_free(_m: *mut OsalGpuMem) -> i32 {
+    0
+}
+unsafe extern "C" fn o_cache(_cpu: *mut c_void, _n: usize) {}
+
+static OSAL: OsalOps = OsalOps {
+    init: None,
+    shutdown: None,
+    gpu_alloc: Some(o_gpu_alloc),
+    gpu_free: Some(o_gpu_free),
+    mmu_map: None,
+    map_phys: None,
+    gpu_write: None,
+    gpu_read: None,
+    cache_flush: Some(o_cache),
+    cache_invd: Some(o_cache),
+    submit: None,
+    wait: None,
+    completed: None,
+    reg_read: None,
+    reg_write: None,
+    query: None,
+    log: None,
+    wait_vblank: None,
+    present: None,
+    lock: None,
+    unlock: None,
+    thread_id: None,
+    display_busy: None,
+};
+
+static SERIAL: Mutex<()> = Mutex::new(());
+
+/// A fresh context on the PowerVR shape, for the length of `case`.
+fn on_powervr(case: impl FnOnce()) {
+    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
+    // SAFETY: the guard serialises the tests; the tables are statics.
+    let ctx = unsafe {
+        osal_set_ops(&OSAL);
+        yakogl::backend::g_backend = &PVR_SHAPE;
+        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
+        let c = yakogl::gl::context::gl_context_create(64, 64);
+        assert!(!c.is_null(), "context pool exhausted");
+        c
+    };
+    *BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = None;
+    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
+    // SAFETY: the context made above.
+    unsafe { yakogl::gl::context::gl_context_destroy(ctx) };
+    if let Err(e) = r {
+        std::panic::resume_unwind(e);
+    }
+}
+
+/* ---- GL plumbing ---------------------------------------------------------------- */
+
+fn no_error(what: &str) {
+    // SAFETY: a plain GL call.
+    let e = unsafe { glGetError() };
+    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
+}
+
+fn shader(ty: GLenum, text: &str) -> GLuint {
+    let text = CString::new(text).unwrap();
+    // SAFETY: plain GL calls; one NUL-terminated source.
+    unsafe {
+        let s = glCreateShader(ty);
+        let p: [*const c_char; 1] = [text.as_ptr()];
+        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
+        glCompileShader(s);
+        let mut ok: GLint = 0;
+        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
+        assert_eq!(ok, 1, "{text:?} did not compile");
+        s
+    }
+}
+
+const VS: &str = "#version 300 es\nin vec4 p;\nout vec2 v;\nvoid main() { v = p.xy * 0.5 + 0.5; gl_Position = p; }\n";
+const FS: &str = "#version 300 es\nprecision mediump float;\nuniform mediump sampler2DArray s;\nin vec2 v;\n\
+                  out vec4 o;\nvoid main() { o = texture(s, vec3(v, 2.0)); }\n";
+static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];
+
+/// One quad sampling the array on unit 0; the bind the back end was handed.
+fn draw_quad(fs: &str) -> GlesTextureBind {
+    *BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = None;
+    // SAFETY: plain GL calls; QUAD is a static.
+    unsafe {
+        glViewport(0, 0, 64, 64);
+        let p = glCreateProgram();
+        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
+        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
+        glBindAttribLocation(p, 0, c"p".as_ptr());
+        glLinkProgram(p);
+        let mut ok: GLint = 0;
+        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
+        assert_eq!(ok, 1, "link failed");
+        glUseProgram(p);
+        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
+        glEnableVertexAttribArray(0);
+        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
+        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
+    }
+    no_error("the quad");
+    BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()).take().expect("the quad was drawn with a texture").0
+}
+
+/* ---- the arrays ------------------------------------------------------------------ */
+
+/// How the case specifies its levels (the texture.specification groups).
+#[derive(Clone, Copy, Debug)]
+enum Spec {
+    /// glTexImage3D per level (basic_teximage3d).
+    Image,
+    /// glTexStorage3D, then glTexSubImage3D of each whole level
+    /// (texstorage3d.format, texstorage3d.size).
+    Storage,
+    /// glTexImage3D of every level with zeros, then glTexSubImage3D of each
+    /// level in two halves of its layers (basic_texsubimage3d).
+    SubImage,
+    /// glTexImage3D of level 0, then glGenerateMipmap (no dEQP group; the
+    /// same storage path). Its levels >= 1 are box-filtered, so only level
+    /// 0 and the level count are checked texel by texel.
+    Generate,
+}
+
+const SPECS: [Spec; 3] = [Spec::Image, Spec::Storage, Spec::SubImage];
+const BPP: usize = 4;
+
+/// GL's extent of level `l`: max(side >> l, 1) per axis; the layers stay.
+fn gl_size(w: u32, h: u32, l: u32) -> (u32, u32) {
+    ((w >> l).max(1), (h >> l).max(1))
+}
+
+/// The RGBA8 value uploaded for texel (x, y) of `layer`, level `l`: no two
+/// (layer, level) pairs share the first byte, and inside a level x and y
+/// tell the texel apart.
+fn value(layer: u32, l: u32, x: u32, y: u32) -> [u8; 4] {
+    [(layer * 16 + l + 1) as u8, x as u8, y as u8, 0xA5]
+}
+
+/// Layers `k0..k1` of level `l`, RGBA8, UNPACK_ALIGNMENT 1.
+fn texels(w: u32, h: u32, l: u32, k0: u32, k1: u32) -> Vec<u8> {
+    let (lw, lh) = gl_size(w, h, l);
+    (k0..k1).flat_map(|k| (0..lh).flat_map(move |y| (0..lw).flat_map(move |x| value(k, l, x, y)))).collect()
+}
+
+fn upload_array(w: u32, h: u32, layers: u32, levels: u32, spec: Spec) -> GLuint {
+    let mut name: GLuint = 0;
+    let sub = |l: u32, k0: u32, k1: u32| {
+        let px = texels(w, h, l, k0, k1);
+        let (lw, lh) = gl_size(w, h, l);
+        // SAFETY: `px` is the (k1 - k0) layers of lw x lh texels named.
+        unsafe {
+            glTexSubImage3D(
+                GL_TEXTURE_2D_ARRAY,
+                l as GLint,
+                0,
+                0,
+                k0 as GLint,
+                lw as GLsizei,
+                lh as GLsizei,
+                (k1 - k0) as GLsizei,
+                GL_RGBA,
+                GL_UNSIGNED_BYTE,
+                px.as_ptr().cast(),
+            )
+        };
+    };
+    let image = |l: u32, px: &[u8]| {
+        let (lw, lh) = gl_size(w, h, l);
+        // SAFETY: `px` is the level's layers x lw x lh texels.
+        unsafe {
+            glTexImage3D(
+                GL_TEXTURE_2D_ARRAY,
+                l as GLint,
+                GL_RGBA8 as GLint,
+                lw as GLsizei,
+                lh as GLsizei,
+                layers as GLsizei,
+                0,
+                GL_RGBA,
+                GL_UNSIGNED_BYTE,
+                px.as_ptr().cast(),
+            )
+        };
+    };
+    // SAFETY: plain GL calls.
+    unsafe {
+        glGenTextures(1, &mut name);
+        glBindTexture(GL_TEXTURE_2D_ARRAY, name);
+        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
+        glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
+        glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
+        glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
+        glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
+        /* a chain short of 1x1 is complete up to its last level (the
+         * 2-level cases) */
+        glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MAX_LEVEL, levels as GLint - 1);
+        if matches!(spec, Spec::Storage) {
+            glTexStorage3D(
+                GL_TEXTURE_2D_ARRAY,
+                levels as GLsizei,
+                GL_RGBA8,
+                w as GLsizei,
+                h as GLsizei,
+                layers as GLsizei,
+            );
+        }
+    }
+    for l in 0..levels {
+        let (lw, lh) = gl_size(w, h, l);
+        match spec {
+            Spec::Image => image(l, &texels(w, h, l, 0, layers)),
+            Spec::Storage => sub(l, 0, layers),
+            Spec::SubImage => {
+                image(l, &vec![0u8; (lw * lh * layers) as usize * BPP]);
+                sub(l, 0, layers / 2);
+                sub(l, layers / 2, layers);
+            }
+            Spec::Generate => {
+                if l == 0 {
+                    image(0, &texels(w, h, 0, 0, layers));
+                    // SAFETY: plain GL call.
+                    unsafe { glGenerateMipmap(GL_TEXTURE_2D_ARRAY) };
+                }
+            }
+        }
+        no_error(&format!("{spec:?} {w}x{h}x{layers}: level {l}"));
+    }
+    name
+}
+
+/* ---- the TPU's view, from the descriptor alone ----------------------------------- */
+
+/// The fields of the descriptor the model walks.
+#[derive(Debug)]
+struct Words {
+    textype: u64,
+    width: u32,
+    height: u32,
+    num_mip_levels: u32,
+    depth: u32,
+    mipmaps_present: bool,
+    texaddr: u64,
+    layer: u32,
+    minlod: u32,
+    maxlod: u32,
+}
+
+fn words(b: &GlesTextureBind) -> Words {
+    let w = yakogl::pvr::backend::texture_descriptor(b);
+    let w0 = u64::from(w[0]) | u64::from(w[1]) << 32;
+    let w1 = u64::from(w[2]) | u64::from(w[3]) << 32;
+    let s0 = u64::from(w[8]) | u64::from(w[9]) << 32;
+    Words {
+        textype: w0 & 7,
+        width: ((w0 >> 34) & 0x3fff) as u32 + 1,
+        height: ((w0 >> 48) & 0x3fff) as u32 + 1,
+        num_mip_levels: (w1 & 0xf) as u32,
+        depth: ((w1 >> 4) & 0x7ff) as u32,
+        mipmaps_present: (w1 >> 15) & 1 != 0,
+        texaddr: ((w1 >> 16) & ((1u64 << 38) - 1)) << 2,
+        layer: w[4],
+        minlod: ((s0 >> 13) & 0x3ff) as u32,
+        maxlod: ((s0 >> 23) & 0x3ff) as u32,
+    }
+}
+
+/// Twiddled index of (x, y) in a `pw x ph` level (powers of two): the low
+/// bits of x and y interleaved, y the low bit, up to the shorter side; the
+/// longer side's remaining bits linear above.
+fn twiddle(x: u32, y: u32, pw: u32, ph: u32) -> u32 {
+    let n = pw.min(ph).max(1).trailing_zeros();
+    let low = (0..n).fold(0, |o, b| o | ((y >> b) & 1) << (2 * b) | ((x >> b) & 1) << (2 * b + 1));
+    let rest = if pw > ph { x >> n } else { y >> n };
+    low | rest << (2 * n)
+}
+
+impl Words {
+    /// The physical extent: the API extent rounded up to a power of two.
+    fn phys(&self) -> (u32, u32) {
+        (self.width.next_power_of_two(), self.height.next_power_of_two())
+    }
+
+    /// The level a NEAREST_MIPMAP_NEAREST look-up at integer LOD `lod` reads.
+    fn level_at(&self, lod: u32) -> u32 {
+        if !self.mipmaps_present {
+            return 0;
+        }
+        ((lod * 64).clamp(self.minlod, self.maxlod) / 64).min(self.num_mip_levels.max(1) - 1)
+    }
+
+    /// The address of texel (x, y) of level `l` of `layer`.
+    fn texel_addr(&self, layer: u32, l: u32, x: u32, y: u32) -> u64 {
+        let (pw, ph) = self.phys();
+        let before: u32 = (0..l).map(|k| (pw >> k).max(1) * (ph >> k).max(1)).sum();
+        let at = before + twiddle(x, y, (pw >> l).max(1), (ph >> l).max(1));
+        self.texaddr + u64::from(layer) * u64::from(self.layer) + u64::from(at) * BPP as u64
+    }
+}
+
+/// The 40 bits of a device address texaddr holds (texstate.xml: [16:53],
+/// shift 2). The stand-in's addresses are host pointers, wider than that.
+const DEV_MASK: u64 = (1 << 40) - 1;
+
+/// The 4 bytes at device address `a` (40 bits, as the TPU sees it) of the
+/// copy bound in `b`: the stand-in OSAL's CPU pointer, its high bits put
+/// back from the bind.
+fn read(b: &GlesTextureBind, a: u64) -> [u8; 4] {
+    let host = (b.base_gpu & !DEV_MASK) | a;
+    assert!(host >= b.base_gpu && host + BPP as u64 <= b.base_gpu + u64::from(b.bytes), "0x{a:x} is outside the copy");
+    // SAFETY: inside the copy the draw bound, which lives in the native
+    // array's cache while the context does.
+    unsafe { *(host as *const [u8; 4]) }
+}
+
+/// The draw's bind of a `w x h x layers` array of `levels` levels and its
+/// words, with the checks every case shares: the native array's layers at
+/// the API extent, texaddr the copy.
+fn bound(w: u32, h: u32, layers: u32, levels: u32, spec: Spec) -> (GlesTextureBind, Words) {
+    upload_array(w, h, layers, levels, spec);
+    let b = draw_quad(FS);
+    let what = format!("{spec:?} {w}x{h}x{layers} x{levels}");
+    assert_eq!(b.type_, GLES_TEXTYPE_2D_ARRAY_TWIDDLED, "{what}: the native array's copy is bound");
+    assert_eq!(b.incomplete, 0, "{what}: complete");
+    let ws = words(&b);
+    assert_eq!((ws.textype, ws.width, ws.height, ws.depth), (1, w, h, layers - 1), "{what}: {ws:?}");
+    assert_eq!(u64::from(ws.layer) * u64::from(layers), u64::from(b.bytes), "{what}: layers of the layer stride");
+    assert_eq!(ws.layer % 4096, 0, "{what}: layers on 4 KiB boundaries");
+    assert_eq!(ws.texaddr, b.base_gpu & DEV_MASK, "{what}: texaddr is the copy");
+    (b, ws)
+}
+
+/// Every texel of levels `0..upto` of every layer read through the words:
+/// the mismatches, counted, with the first few named.
+fn misplaced(b: &GlesTextureBind, ws: &Words, w: u32, h: u32, layers: u32, upto: u32) -> (u32, Vec<String>) {
+    let (mut bad, mut first) = (0u32, Vec::new());
+    for k in 0..layers {
+        for l in 0..upto {
+            let (lw, lh) = gl_size(w, h, l);
+            for y in 0..lh {
+                for x in 0..lw {
+                    let (got, want) = (read(b, ws.texel_addr(k, l, x, y)), value(k, l, x, y));
+                    if got != want {
+                        bad += 1;
+                        if first.len() < 4 {
+                            first.push(format!("layer {k} level {l} ({x}, {y}): {got:?}, want {want:?}"));
+                        }
+                    }
+                }
+            }
+        }
+    }
+    (bad, first)
+}
+
+/// Every GL level of every layer lands where the descriptor points.
+fn every_level_lands(w: u32, h: u32, layers: u32, levels: u32, spec: Spec) {
+    on_powervr(|| {
+        let (b, ws) = bound(w, h, layers, levels, spec);
+        let what = format!("{spec:?} {w}x{h}x{layers} x{levels}");
+        assert_eq!((b.mip_filter, b.mip_levels as u32), (1, levels), "{what}: GL's chain reaches the back end");
+        assert!(ws.mipmaps_present && ws.num_mip_levels >= levels, "{what}: the image word holds the chain: {ws:?}");
+        let (bad, first) = misplaced(&b, &ws, w, h, layers, levels);
+        assert_eq!(bad, 0, "{what}: texels not where the descriptor points: {first:#?}");
+    });
+}
+
+/// A look-up at each LOD 0..levels reads that level's texels on every
+/// layer (the centre and the four corners of the level), and a LOD past the
+/// chain reads its last level.
+fn each_level_samples_its_colour(w: u32, h: u32, layers: u32, levels: u32, spec: Spec) {
+    on_powervr(|| {
+        let (b, ws) = bound(w, h, layers, levels, spec);
+        let what = format!("{spec:?} {w}x{h}x{layers} x{levels}");
+        for lod in 0..levels + 2 {
+            let want_l = lod.min(levels - 1);
+            let l = ws.level_at(lod);
+            assert_eq!(l, want_l, "{what}: LOD {lod} reads level {want_l}: {ws:?}");
+            let (lw, lh) = gl_size(w, h, l);
+            for k in 0..layers {
+                for (x, y) in [(lw / 2, lh / 2), (0, 0), (lw - 1, 0), (0, lh - 1), (lw - 1, lh - 1)] {
+                    let got = read(&b, ws.texel_addr(k, l, x, y));
+                    assert_eq!(got, value(k, want_l, x, y), "{what}: LOD {lod} layer {k} ({x}, {y})");
+                }
+            }
+        }
+    });
+}
+
+/* ---- the model itself ------------------------------------------------------------ */
+
+/// The walk on hand-written words: the rectangular twiddle, a 57x44
+/// layer's physical chain 64x64 .. 1x1 (5461 texels, 4 KiB aligned: 24576
+/// bytes), the LOD clamp.
+#[test]
+fn the_model_walks_mesas_twiddled_chain() {
+    assert_eq!(twiddle(1, 0, 2, 2), 2);
+    assert_eq!(twiddle(0, 1, 2, 2), 1);
+    assert_eq!(twiddle(3, 5, 8, 8), 0b01_10_11);
+    /* 8x2: one interleaved bit pair, then x's two remaining bits */
+    assert_eq!(twiddle(5, 1, 8, 2), 0b1011);
+    assert_eq!(twiddle(0, 3, 2, 4), 0b101);
+    let ws = Words {
+        textype: 1,
+        width: 57,
+        height: 44,
+        num_mip_levels: 7,
+        depth: 4,
+        mipmaps_present: true,
+        texaddr: 0x1000,
+        layer: (5461 * 4u32).next_multiple_of(4096),
+        minlod: 0,
+        maxlod: 5 * 64,
+    };
+    assert_eq!((ws.phys(), ws.layer), ((64, 64), 24576));
+    assert_eq!(ws.texel_addr(0, 1, 0, 0), 0x1000 + 64 * 64 * 4);
+    assert_eq!(ws.texel_addr(2, 5, 0, 0), 0x1000 + 2 * 24576 + (4096 + 1024 + 256 + 64 + 16) * 4);
+    assert_eq!((ws.level_at(3), ws.level_at(9)), (3, 5), "clamped to maxlod");
+    assert_eq!(Words { mipmaps_present: false, ..ws }.level_at(3), 0);
+}
+
+/* ---- the power-of-two control (64x32x3: passes on the native-array arm) ---------- */
+
+#[test]
+fn pot_array_every_level_lands_where_the_descriptor_points() {
+    for spec in SPECS {
+        for levels in [2, 7] {
+            every_level_lands(64, 32, 3, levels, spec);
+        }
+    }
+}
+
+#[test]
+fn pot_array_each_level_samples_its_colour() {
+    for spec in SPECS {
+        for levels in [2, 7] {
+            each_level_samples_its_colour(64, 32, 3, levels, spec);
+        }
+    }
+}
+
+/* ---- the non-power-of-two extents (57x44x5, 57x63x5) ----------------------------- */
+
+/// The K4 sizes: basic_teximage3d / basic_texsubimage3d / texstorage3d.format
+/// use 57x44x5 with the full chain (6 levels), texstorage3d.size
+/// 57x63x5 with 2.
+const NPOT: [(u32, u32, u32, u32); 3] = [(57, 44, 5, 6), (57, 44, 5, 2), (57, 63, 5, 2)];
+
+/// Level 0 of each NPOT array, every texel of every layer, where the
+/// descriptor points: the copy's NPOT placement (the 57x44 layer in the
+/// top-left of its 64x64 physical layer, twiddled over 64x64) is the TPU's.
+#[test]
+fn npot_array_level_0_lands_where_the_descriptor_points() {
+    for spec in SPECS {
+        for (w, h, layers, levels) in NPOT {
+            on_powervr(|| {
+                let (b, ws) = bound(w, h, layers, levels, spec);
+                let (bad, first) = misplaced(&b, &ws, w, h, layers, 1);
+                assert_eq!(bad, 0, "{spec:?} {w}x{h}x{layers} x{levels} level 0: {first:#?}");
+            });
+        }
+    }
+}
+
+/// Every GL level of every layer of the NPOT arrays lands where the
+/// descriptor points: the GL core holds the chain (texture.rs
+/// `packed_npot_array`) and the copy re-lays each level into the layer's
+/// physical chain. Until it held the chain, levels >= 1 had no texels.
+#[test]
+fn npot_array_every_level_lands_where_the_descriptor_points() {
+    for spec in SPECS {
+        for (w, h, layers, levels) in NPOT {
+            every_level_lands(w, h, layers, levels, spec);
+        }
+    }
+}
+
+/// A look-up at each LOD reads that level of the NPOT array (it read level
+/// 0 at every LOD while the GL core kept one level).
+#[test]
+fn npot_array_each_level_samples_its_colour() {
+    for spec in SPECS {
+        for (w, h, layers, levels) in NPOT {
+            each_level_samples_its_colour(w, h, layers, levels, spec);
+        }
+    }
+}
+
+/// glGenerateMipmap of a 57x44x5 array grows the packed chain: the bind
+/// carries all 6 levels and level 0 is where the descriptor points.
+#[test]
+fn npot_array_generate_mipmap_holds_the_chain() {
+    on_powervr(|| {
+        let (b, ws) = bound(57, 44, 5, 6, Spec::Generate);
+        assert_eq!((b.mip_filter, b.mip_levels), (1, 6), "the generated chain reaches the back end");
+        assert!(ws.mipmaps_present && ws.num_mip_levels >= 6, "{ws:?}");
+        let (bad, first) = misplaced(&b, &ws, 57, 44, 5, 1);
+        assert_eq!(bad, 0, "level 0: {first:#?}");
+        /* level 5 (1x1) of layer k: not level 0's texel (0, 0), which is what
+         * every LOD read while the chain was one level */
+        for k in 0..5 {
+            assert_ne!(read(&b, ws.texel_addr(k, 5, 0, 0)), value(k, 0, 0, 0), "layer {k} level 5 is its own");
+        }
+    });
+}
+
+/* ---- a stacked-array program over the held chain --------------------------------- */
+
+/// A program compiled with the native array off samples the stacked image
+/// (`pvr_arrUV`), whose descriptor walks level l at the power-of-two step
+/// of the stacked height, not where the packed chain keeps it: such a draw
+/// is handed the 57x44x5 array as one level (no mipmap filter), the
+/// storage itself, level 0's five layers stacked.
+#[test]
+fn a_stacked_array_program_samples_level_0_alone() {
+    const FS_STACKED: &str = "#version 300 es\nprecision mediump float;\nuniform mediump sampler2DArray s;\n\
+                              in vec2 v;\nout vec4 o;\nvoid main() { o = texture(s, vec3(v, 2.0)) * 1.0; }\n";
+    for spec in SPECS {
+        on_powervr(|| {
+            struct Default;
+            impl Drop for Default {
+                fn drop(&mut self) {
+                    yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
+                }
+            }
+            let _default = Default;
+            yakogl::shaderc::pvr_set_native_array(0);
+            upload_array(57, 44, 5, 6, spec);
+            let b = draw_quad(FS_STACKED);
+            let what = format!("{spec:?} 57x44x5 x6, stacked program");
+            assert_eq!(b.type_, GLES_TEXTYPE_2D_ARRAY, "{what}: the stacked image is bound");
+            assert_eq!(b.incomplete, 0, "{what}: complete");
+            assert_eq!((b.mip_filter, b.mip_levels), (0, 1), "{what}: level 0 alone");
+            for k in 0..5u32 {
+                for (x, y) in [(0u32, 0u32), (56, 0), (0, 43), (56, 43), (28, 22)] {
+                    let at = ((k * 44 + y) * b.stride_texels + x) as usize * BPP;
+                    // SAFETY: inside level 0 of the texture's storage, which
+                    // the bind names and the context keeps.
+                    let got = unsafe { *(b.cpu as *const u8).add(at).cast::<[u8; 4]>() };
+                    assert_eq!(got, value(k, 0, x, y), "{what}: layer {k} ({x}, {y})");
+                }
+            }
+        });
+    }
+}
```

## 5. The same change after the K3 cube patch

`docs/notes/pvr-npot-cube-levels.md` section 4 (cause K3) changes the same
functions. Section 4's diff does not apply on top of it: two hunks of
`src/gl/texture.rs` and one of `src/gl/draw.rs` conflict. If K3 lands first,
apply this variant instead. It is against `472630df` with the K3 diff
applied. `git apply --check` accepts it there. It holds only the `src/`
part; the test file is section 4's.

~~~sh
sed -n '/^```diff$/,/^```$/{//!p}' docs/notes/pvr-npot-cube-levels.md | git apply
sed -n '/^```patch$/,/^```$/{//!p}' docs/notes/pvr-npot-array-levels.md | git apply
sed -n '/^```diff$/,/^```$/{//!p}' docs/notes/pvr-npot-array-levels.md | git apply --include='tests/*'
git show 2d215408:tests/pvr_npot_cube.rs > tests/pvr_npot_cube.rs
~~~

In this variant:

- K3's `packed_stack` already calls `stacked_decoded_array`, so the
  glGenerateMipmap and grow arms need no edit.
- `texture_alloc_storage_ex` asks `packed_npot_cube || packed_npot_array`.
- The array draw rule goes right after K3's cube rule.
- The comments that K3 changed to "a PowerVR NPOT array" keeps one level
  now say "storage that could not grow".

```patch
diff --git a/src/gl/draw.rs b/src/gl/draw.rs
index 027b666c..1cc037e1 100644
--- a/src/gl/draw.rs
+++ b/src/gl/draw.rs
@@ -3645,9 +3645,9 @@ fn live_texture(textures: &mut [GlTexture; TEXTURE_SLOTS], h: usize) -> Option<&
  * mode and with mipmaps, and so does ES 2.0 with GL_OES_texture_npot, which
  * every build that reports ES 2.0 claims since 2026-10-07 -- so it applies
  * only where texture::NPOT_RESTRICTED holds, which no build does any more.
- * Where the backend keeps one level of a NPOT image (the PowerVR's stacked
- * NPOT 2D arrays, texture.rs npot_mip_chains) such a texture samples its
- * base level under a mipmap filter.
+ * Where the backend keeps one level of a NPOT image (storage that could
+ * not grow, texture.rs grow_npot_chain) such a texture samples its base
+ * level under a mipmap filter.
  *
  * This replaces demoting the filter and sampling level 0, which showed an
  * application's mistake as a picture rather than as black. */
@@ -3760,7 +3760,7 @@ fn incomplete_reason(t: &GlTexture) -> Option<&'static str> {
      * can. Levels below the base level are not looked at. */
     if mips {
         /* GL's chain, not the store's: where the backend keeps one level of
-         * a NPOT image (a PowerVR NPOT array), GL still wants its whole chain, which
+         * a NPOT image (storage that could not grow), GL still wants its whole chain, which
          * glGenerateMipmap, glTexStorage2D or the application's own levels
          * record as present without holding them */
         let chain = (((1u32 << end) - 1) & !((1u32 << base) - 1)) as u16;
@@ -5124,6 +5124,13 @@ fn build_common(
             if tb.type_ == GLES_TEXTYPE_CUBE && super::texture::packed_npot_cube_of(t) {
                 (tb.mip_filter, tb.mip_levels) = (0, 1);
             }
+            /* The same for a PowerVR NPOT 2D array (texture.rs
+             * packed_npot_array), whose packed chain only the native array's
+             * copy re-lays: sampled as the stacked image (a stacked-array
+             * program, or no copy could be made), level 0 alone. */
+            if tb.type_ == GLES_TEXTYPE_2D_ARRAY && super::texture::packed_npot_array_of(t) {
+                (tb.mip_filter, tb.mip_levels) = (0, 1);
+            }
             /* The Mali's 3D texture: its back end copies every level of the
              * grid the GL core holds after level 0 (`volume_of`,
              * `grid_level_at`; docs/notes/mali-3d-levels.md section 4), so it
diff --git a/src/gl/texture.rs b/src/gl/texture.rs
index 2a6a364b..2443ae11 100644
--- a/src/gl/texture.rs
+++ b/src/gl/texture.rs
@@ -1309,20 +1309,42 @@ fn npot_mip_chains(layers: u32) -> bool {
     backend::native_layered_textures() || (backend::mip_level_align() == 0 && layers <= 1)
 }
 
-/// A non-power-of-two 2D array of a decoded ETC2/EAC name on a back end that
-/// does not walk layers itself ([`npot_mip_chains`] false). Its GL levels all
-/// have texels (a decoded name's uploads must land somewhere): the chain is
-/// laid out packed, level i of `layers` images of max(w >> i, 1) x max(h >> i,
-/// 1), and the array copy (`native_array::copy`) puts it layer-major for the
-/// TPU. Uncompressed arrays keep one level there ([`npot_mip_chains`]).
-fn stacked_decoded_array(t: &GlTexture) -> bool {
-    t.target == GL_TEXTURE_2D_ARRAY
-        && t.decoded_from != 0
-        && t.layers > 1
+/// A non-power-of-two 2D array of `w x h` layers on a back end that does
+/// not walk layers itself and packs its levels ([`npot_mip_chains`] false
+/// for `layers` > 1: the PowerVR). Its chain is laid out packed, level i the
+/// `layers` images of max(w >> i, 1) x max(h >> i, 1) stacked: the TPU never
+/// walks this storage's chain itself. The array copy (`native_array::copy`)
+/// re-lays every level from `level_off` into the layer-major twiddled chain
+/// of the power-of-two extent the TPU walks (Mesa 26.2.1
+/// pvr_image_plane_setup_mip_levels), and a stacked-array program samples
+/// level 0 alone (`gl::draw`, [`packed_npot_array_of`]). Until this held
+/// the chain of an uncompressed array (it held a decoded ETC2/EAC one's
+/// only), such an array kept one level, its levels >= 1 recorded and
+/// dropped, and every LOD read level 0 (dEQP-GLES3 texture.specification
+/// *_2d_array 57x44x5 and 57x63x5, AM62P; tests/pvr_npot_array_chain.rs).
+fn packed_npot_array(target: GLenum, w: u32, h: u32, layers: u32) -> bool {
+    target == GL_TEXTURE_2D_ARRAY
+        && layers > 1
+        && !(is_pot(w) && is_pot(h))
+        && (w > 1 || h > 1)
         && !backend::native_layered_textures()
         && backend::mip_level_align() == 0
 }
 
+/// [`packed_npot_array`] of texture `t`, uncompressed (a decoded ETC2/EAC
+/// array included: its storage is the decoded texels). The name is the one
+/// the decoded array had alone, kept so the callers read as before.
+fn stacked_decoded_array(t: &GlTexture) -> bool {
+    t.block_bytes == 0 && packed_npot_array(t.target, t.width, t.height, u32::from(t.layers))
+}
+
+/// [`stacked_decoded_array`] of texture `t` as it is laid out now (a chain
+/// of more than one level): the draw keeps a stacked-array bind of it to
+/// level 0.
+pub(crate) fn packed_npot_array_of(t: &GlTexture) -> bool {
+    t.chain_levels > 1 && stacked_decoded_array(t)
+}
+
 /// A non-power-of-two cube map of `w x h` faces on a back end that does not
 /// walk layers itself and packs its levels ([`npot_mip_chains`] false for
 /// its six stacked faces: the PowerVR). Its chain is laid out packed, level
@@ -1990,8 +2012,9 @@ fn texture_alloc_storage_ex(
     one_level: bool,
     npot_chain: bool,
 ) -> i32 {
-    /* glTexStorage2D of a NPOT cube with levels: packed (packed_npot_cube) */
-    let stacked = npot_chain && packed_npot_cube(t.target, w, h, layers);
+    /* glTexStorage2D of a NPOT cube or glTexStorage3D of a NPOT 2D array
+     * with levels: packed (packed_npot_cube, packed_npot_array) */
+    let stacked = npot_chain && (packed_npot_cube(t.target, w, h, layers) || packed_npot_array(t.target, w, h, layers));
     let Some(layout) = storage_layout(w, h, layers, texfmt, one_level, NpotChain::from(npot_chain, stacked)) else {
         return -1;
     };
@@ -2571,8 +2594,8 @@ pub(crate) fn level_size(t: &GlTexture, level: u32) -> (u32, u32) {
 }
 
 /// A level GL has (glTexStorage*, or a glTexImage* that fit its chain) but
-/// the backend is not told of: a chain the backend cannot hold (a NPOT array
-/// or cube on the PowerVR, `npot_mip_chains`), whose sub-images are checked
+/// the backend is not told of: a chain the backend cannot hold (storage
+/// that could not grow, `grow_npot_chain`), whose sub-images are checked
 /// and then dropped, as its images are; or a level above 0 of a 3D grid,
 /// which the store does hold ([`grid_level_held`]).
 fn recorded_unheld(t: &GlTexture, level: GLint) -> bool {
@@ -4855,7 +4878,7 @@ pub unsafe extern "C" fn glGenerateMipmap(target: GLenum) {
     let t = &mut c.objs().textures[ti];
     if t.chain_levels < 2 {
         /* No chain in the store: a NPOT image where the backend keeps one
-         * level (a PowerVR NPOT array, npot_mip_chains), or a 1x1. GL's chain exists
+         * level (storage that could not grow), or a 1x1. GL's chain exists
          * all the same (ES 2.0 3.7.11), so the texture is complete under a
          * mipmap filter and samples its base level. It was left incomplete,
          * and an ES 3.0 build sampled such a texture black. */
@@ -5929,9 +5952,9 @@ unsafe fn tex_image_3d(
         /* A level of a non-power-of-two array's chain (ES 3.0 3.8.14: floor
          * sizes down to 1x1, the layer count the same at every level). The
          * store has one level of such an array until a backend that holds
-         * the chain is asked for it (grow_npot_chain); one that cannot (the
-         * PowerVR's stacked NPOT layers, npot_mip_chains) records the level
-         * as GL has it and keeps no texels, as for a NPOT cube. */
+         * the chain is asked for it (grow_npot_chain; the PowerVR's stacked
+         * NPOT layers packed, packed_npot_array). Storage that cannot grow
+         * records the level as GL has it and keeps no texels. */
         let npot_level = t.level_defined & 1 != 0
             && t.level_bad & 1 == 0
             && t.chain_levels == 1
@@ -5953,13 +5976,10 @@ unsafe fn tex_image_3d(
             }
         }
         let t = &mut c.objs().textures[ti];
-        /* Only the PowerVR's tall stacked storage gets here (npot_mip_chains
-         * false: layers > 1, no native layers): a stacked NPOT array cannot
-         * hold its chain, so the level is recorded and its texels dropped,
-         * and draws sample level 0. Left as it is on purpose: plan item
-         * pvr-2darray (docs/es3/pvr-2darray.md) lays PowerVR arrays out
-         * layer-major, and their levels >= 1 get storage there. Every other
-         * array keeps the texels of each level (grown above, or a
+        /* Storage that could not grow above (no CPU view, an external
+         * image): the level is recorded and its texels dropped, and draws
+         * sample level 0. Every other array keeps the texels of each level
+         * (grown above, packed on the PowerVR (packed_npot_array), or a
          * power-of-two chain). */
         if npot_level && t.chain_levels == 1 {
             t.level_defined |= 1u16 << (level as u32).min(15);
@@ -6070,8 +6090,8 @@ unsafe fn tex_sub_image_3d(
     }
     let t = &c.objs_ref().textures[ti];
     let grid_cols = t.grid_cols as u32;
-    /* a level the store keeps no texels for (a PowerVR NPOT array's above
-     * 0, see glTexImage3D); a 3D grid's levels above 0 are held after level
+    /* a level the store keeps no texels for (storage that could not grow,
+     * see glTexImage3D); a 3D grid's levels above 0 are held after level
      * 0's grid though the backend is told of level 0 alone */
     let unheld = recorded_unheld(t, level) && !grid_level_held(t, level as u32);
     let (lw, lh) = level_size(t, level as u32);
```

Measured on `472630df` + K3 + this variant + both test files
(`host-test,gpu-powervr,shaderc-online`): `pvr_npot_array_chain` 8 passed,
`pvr_npot_cube` 7 passed, `es3_pvr_2darray` 15, `es3_pvr_etc_array` 6,
`pvr_array_native_cases` 7, `--lib gl::` 1927 passed (1 ignored, as on
main). `rustfmt --check` is clean.

## 6. The host test to add

`tests/pvr_npot_array_chain.rs` (8 tests, in section 4's diff). Feature set
`host-test,gpu-powervr,shaderc-online` (nothing under `report-es2`):

~~~sh
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_npot_array_chain
~~~

It runs in the `tools/check.sh` test rows of those configurations, which run
every target of `tests/`. It follows `tests/pvr_npot_cube.rs` (K3). A
stand-in back end with the PowerVR's flags records the bind each draw hands
it. One quad samples a `sampler2DArray` with NEAREST_MIPMAP_NEAREST.
`yakogl::pvr::backend::texture_descriptor` turns the bind into the
descriptor. A TPU model then reads the bytes knowing only:

- IMAGE_WORD0 (textype 2D, one layer's extent);
- the twiddled IMAGE_WORD1 (levels, depth = layers - 1, mipmaps_present,
  texaddr);
- image meta dword 4 (the layer stride);
- SAMPLER_WORD0's LOD range;
- Mesa 26.2.1 pvr's layout: the power-of-two physical extent, a rectangular
  twiddle and packed levels per layer.

Each (layer, level, x, y) has its own uploaded value. The specs are
glTexImage3D per level (basic_teximage3d), glTexStorage3D + glTexSubImage3D
(texstorage3d), and glTexImage3D of zeros + glTexSubImage3D in two halves of
the layers (basic_texsubimage3d form).

| test | asserts | without the src/gl part |
|---|---|---|
| `the_model_walks_mesas_twiddled_chain` | the model's own layout (rectangular twiddle, 57x44 layer = 24576 bytes, LOD clamp) | pass |
| `pot_array_every_level_lands_where_the_descriptor_points`, `pot_array_each_level_samples_its_colour` | 64x32x3 control, 2 and 7 levels | pass |
| `npot_array_level_0_lands_where_the_descriptor_points` | 57x44x5 (6 and 2 levels), 57x63x5 (2 levels): level 0 | pass |
| `npot_array_every_level_lands_where_the_descriptor_points` | the same sizes, every texel of every layer and level is where the descriptor points | **fail** |
| `npot_array_each_level_samples_its_colour` | each LOD reads its own level; a LOD past the chain reads the last | **fail** |
| `npot_array_generate_mipmap_holds_the_chain` | glGenerateMipmap of 57x44x5: 6 levels bound, level 5 is not level 0 | **fail** |
| `a_stacked_array_program_samples_level_0_alone` | with `pvr_native_array` off, the bind is `GLES_TEXTYPE_2D_ARRAY` with `(mip_filter, mip_levels) = (0, 1)` and level 0's texels | pass; **fails** with `texture.rs` patched and the `draw.rs` rule left out (7 passed, 1 failed) |

Measured on `472630df` with section 4 applied (the same feature set):

- `pvr_npot_array_chain` 8 passed.
- Without the `src/gl` part: 5 passed, 3 failed (the table's **fail**
  rows), so the test pins the fix.
- `cargo clippy --no-default-features --features
  host-test,gpu-powervr,shaderc-online --lib --test pvr_npot_array_chain`
  gives no warning.
- `rustfmt --check` is clean on the three files.

The `precompiled-shaders` configuration was not run: it needs
`YAKOGL_PRECOMPILED_TABLE`. The quad's programs come from the online
compiler, as in `pvr_npot_cube`.

## 7. Tests and references that move

- **None change.** No existing test asserted that a NPOT array keeps one
  level. Measured with section 4 applied (`host-test,gpu-powervr,shaderc-online`),
  all green:
  - `--lib gl::`: 1927 passed, 1 ignored (as on main);
  - `es3_pvr_2darray` (15), `es3_pvr_etc_array` (6; the decoded arrays that
    lost their `decoded_from` condition), `pvr_array_native_cases` (7),
    `es2_npot` (18), `pvr_npot_levels` (5), `pvr_native_switch_cache` (1),
    `pvr_mip_chain_pin` (1), `pvr_mipgen_program` (2), `pvr_mip_shapes` (3);
  - `es3_pvr_texsize` (9), `es3_pvr_texfn` (3), `es3_pvr_fragout_array` (5),
    `es3_blit_ds_layers_pvr` (5), `es3_depth_image_switch_pvr` (5, 5
    ignored), `es3_mrt4_copytex_pvr` (5, 1 ignored).
- `tests/reference/*.rs`, `tests/fixtures/`, `src/pvr/internal.rs`: none. The
  patch changes storage layout and draw state, not a compiled shader or a
  pinned descriptor.
- Documentation that goes stale when it lands, for the applier to update in
  the same commit:
  - `docs/es3/texture-failure-map.md` 2.2.1: the K4 row "Pass with a src/gl
    change", the K4 paragraph (it names a test "that does not exist yet" and
    `PVR_NATIVE_ARRAY_DEFAULT` "now 0"), and the "tip, native array on" row of
    the outcome table.
  - `docs/es3/shader-failure-map.md:703` ("Holding the levels of an NPOT
    array is a separate change").
  - The `texfn-gl-design` patch quoted there (`:597`, a NPOT array's levels
    above 0 "get the extent alone"). After K4 those levels are held, packed
    exactly as that design's base-level restate assumes ("level b holds the
    `nlayers` images of max(w >> b, 1) x max(h >> b, 1)").

## 8. The dEQP cases it fixes (K4)

76 AM62P `Fail` lines of `docs/es3/am62p-gles3-notpass.tsv` (counted from the
TSV):

| group (`dEQP-GLES3.functional.texture.specification.`) | cases | TSV message at `ad2b5ed` |
|---|---:|---|
| `basic_teximage3d.*_2d_array` (all of the group, 57x44x5, full chain) | 49 | 24 "Image comparison failed", 25 "Compile failed" (the integer formats) |
| `texstorage3d.format.*_2d_array` (all 21 of the group) | 21 | 10 "Image comparison failed", 11 "Compile failed" (integer) |
| `texstorage3d.size.2d_array_57x63x5_2_levels` | 1 | "Image comparison failed" |
| `texsubimage3d_depth.{depth_component16,depth_component24,depth_component32f,depth24_stencil8,depth32f_stencil8}_2d_array` | 5 | "Image comparison failed" |

Prerequisites:

- The 36 integer cases were "Compile failed" at `ad2b5ed`. The K1 compiler
  fix (before `31cdaefc`) lets them compile, and then they meet K4. This is
  36 of the 119 cases that section 2.2.1 says it has not reconciled between
  its causes and the TSV's message split.
- `basic_teximage3d.rgba32f_2d_array` also needs K2 (`ecd9743e`, on the tip).

Expected after the patch: Pass, on a build with the native array on (the
default since `02c5c9af`). The board run is still to do. The 31cdaefc board
images of plan #400 carry neither this fix nor `02c5c9af`.

Not claimed:

- The depth cases are not pinned on the host. The test uses RGBA8, and the
  depth texels take the same storage path. Whether a depth 2D array is
  sampled from its texels or from a depth buffer on the board is for the
  board run to show.
- `texstorage3d.size.2d_array_64x32x3_7_levels` is K6. It is power of two
  and not reached by this patch, and it passes with the native array on.
- `texture.filtering.2d_array.*` and `texture.shadow.2d_array.*` are not
  attributed to this cause here.
