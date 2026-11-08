<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: E1, the tail route to fewer PowerVR mipmap kicks, as a host-run patch and a board procedure

Written 2026-11-07 at main `ea371491` (line numbers are that commit's). This
is experiment E1 of `pvr-mip-fewer-kicks.md`: its route 1, the "tail route"
(rank 1 there). That note's own "E1" is the board read-back of the ALU
bilinear against the TPU's. Here that read-back is step 3 of the board
procedure below, because the patch's tail levels depend on its answer.

The patch is **for an experiment branch, not main**. This commit holds this
note alone. The patch touches `src/pvr/backend.rs`, `src/pvr/internal.rs`,
`tools/shaders/**` and `src/pvr/backend_tests.rs`, which belong to plans
#325 and #301 while they run. It was applied in a local tree only. No board
was run.

**Result on the host.**

- **Kicks a frame.** glmark2 terrain on the PowerVR stand-in goes from
  **52 to 32** (`tools/hostharness/am62p-terrain/run.sh <GLMARK2_SRC> <out>
  kicks`, frames 1..18, 8 draws a frame both ways). Each 800x600 chain falls
  from 9 kicks to 5, and each 256x256 chain from 8 to 4.
- **Tail levels against today's chain.** The tail program's module was run
  by the reference IR evaluator over every pixel of the tail surface. With
  the TPU's sample modelled as the program's own ALU arithmetic, the tail
  levels match today's chain texel for texel: 0 texels differ at both sizes,
  for random and for smooth level-0 content, and the padding is 0. With the
  sample modelled as ES 3.0's four-weight sum, or with 8-bit sub-texel
  weights, up to 8 texels of a level move, by at most 1 LSB. Only on smooth
  content, and only in 800x600 levels 6 to 8 (12x9, 6x4, 3x2). That spread
  is E1's board question.
- **Deviation from the study.** The study planned s = 3 for the 800x600
  chain, a tail of 6 levels and 4 kicks a chain. A 6-level tail program
  needs 255 or 256 temporaries, which our allocator refuses (it took 247,
  `OutOfRegisters` at 255). So the tail holds
  5 levels (219 temporaries), the 800x600 chain keeps level 4 as a kick of
  its own, and it ends at 5 kicks.

`tests/pvr_mip_chain_pin.rs` (item mip-pin-pvr) is not at the tip. The
level contents were therefore compared with a scratch test made for this
note (appendix B), not with that pin.

## What the patch does

`mip_chain_gpu` (`src/pvr/backend.rs:6241`) draws one `record_mip_level` a
level, in the loop at `:6268`. With the patch, a chain whose whole length is
drawn on the GPU (`mip_gpu_last` = n - 1, which is the case while
`mip_cpu_tail` is off, the default) is split in two by `mip_tail_plan`:

- Levels 1 ..= s stay today's kicks, unchanged. s = max(3, n - 1 - 5).
- One more kick, `record_mip_tail`, draws levels s+1 .. n-1. Those levels
  are one contiguous byte range of the storage (`mip_chain_layout`,
  `:6095`). The kick views that range as one linear RGBA8 surface of P
  texels a row and R rows:
  - P is even (the PBE's pixel pairs) and divides the range's texel count.
  - P is the narrowest such pitch at or past the square root.
  - P x R lies inside level 1's grid: the frame is level 1's size, as every
    level kick's is (`record_mip_level` `:6151-6152`), so the chain still
    uses one HWRT size.
  - The store is clipped to P x R through `mip_clip` (`:5228`, `:8903`).
- If the range's texel count is odd, its last texel goes out through a
  second PBE emit. That texel is the last level, a 1x1 level at pitch 1.
  The emit is the MRT path's slot 1: a 1x1 `GlesDrawTarget` at that level's
  address, clipped 1x1 by its own size (`:8904-8918`). No byte outside the
  storage is written.
- The new fragment program `mipgen_tail.frag` gets a 44-dword block:
  - `u_lv[k]` = (1/w, 1/h, w, h) for each tail level. The first two floats
    are exactly the `u_scale` that `record_mip_level` computes (`:6143`,
    `1.0 / dst.w as f32`), passed as they are.
  - `u_ly[k]` = (texel offset in the range, pitch, 1/pitch, 0).
  - `u_sf` = (P, 1 if there is a last texel, the tail's level count, 0).
- The program, for each pixel:
  - It finds its texel's level k and its (x, y) in that level.
  - It walks the 4^k leaves under that texel in one loop that may run a
    different number of times in each pixel. Such a loop runs under the
    execution mask (`powervr/unroll.rs` `Trip::divergent`), so a pixel
    leaves the loop when its own leaves are done.
  - A leaf is level s+1's texel, computed exactly as the level kick
    computes it: `textureLod` of level s, which is bound alone with linear
    filters and CLAMP_TO_EDGE as `record_mip_level` binds it (`:6170-6187`),
    at `(x + 0.5) * u_scale`, packed to 8 bits. `gl_FragCoord.xy` is x + 0.5,
    so the coordinate's bits are the level kick's.
  - Each deeper level is the bilinear filter of four texels of the level
    above, computed in the ALU at that level's own `u_scale` point:
    CLAMP_TO_EDGE, then rounded to 8 bits, as the per-level kick rounds.
    One partial row (t, r) a level holds the reduction.

The kick counts, from `mip_tail_plan`:

| chain (GL size, levels) | s | tail levels | surface | kicks today | kicks with the patch |
|---|---|---|---|---|---|
| 800x600, 10 (terrain, tilt-shift, case 216's A and C) | 4 | 5..9 (25x18 .. 1x1), 1364 texels | 44 x 31 | 9 | **5** |
| 256x256, 9 (bloom, case 216's B) | 3 | 4..8 (16x16 .. 1x1), 341 texels | 20 x 17 + the last texel | 8 | **4** |
| 512x512, 10 (a texture terrain builds once at setup) | 4 | 5..9, 341 texels | 20 x 17 + the last texel | 9 | 5 |
| 1600x1200, 11 (refract) | 5 | 6..10 | 44 x 31 | 10 | 6 |
| 64x64, 7 (64x64 chains such as `a_64x64_chain...`) | 3 | 4..6, 21 texels | 10 x 2 + the last texel | 6 | 4 |
| 16x16 and smaller | - | none (s + 2 >= n) | - | as today | as today |

The tail's work, with the sizes worked out:

- The 800x600 tail kick makes 5120 leaf look-ups:
  32x32 + 16x16 x 4 + 8x8 x 16 + 4x4 x 64 + 2x2 x 256 = 5 x 1024.
- The worst pixel is level 9's and walks 256 leaves.
- In the 256x256 tail, pixel (0, 0) walks 1 + 256 leaves (its own texel,
  then the last one).
- The compiled program is 3898 dwords and 219 temporaries (`src/pvr/
  internal.rs`, row `PVR_INTERNAL_MIPGEN_TAIL`). `mipgen.frag` is 100 dwords
  and 26 temporaries.

## (1) The patch, against `ea371491`

Apply the diff below, then regenerate the generated file with its generator,
in this order:

```text
git apply e1.diff        # the diff below
# yakogl-precompile links the library, whose backend.rs now names
# PVR_INTERNAL_MIPGEN_TAIL: make the row from a tree that builds first.
# Either build the table with the backend hunks reverted, then re-apply them:
git apply -R --include=src/pvr/backend.rs e1.diff
cargo run -q -p yakogl-precompile --features gpu-powervr -- --internal -o src/pvr/internal.rs
git apply --include=src/pvr/backend.rs e1.diff
cargo run -q -p yakogl-precompile --features gpu-powervr -- --internal --verify src/pvr/internal.rs
#   -> "src/pvr/internal.rs is what the list's 47 lines make"
```

- **`src/pvr/internal.rs`.** The regenerated file adds row 37,
  `PVR_INTERNAL_MIPGEN_TAIL`: 612 lines, the program's words and its record.
  Its sha256 at `ea371491` plus this patch is `27f95bb63597c6858c873c589bc16bfca059575e93ee80ebcaa4f91d9f4c848b`. It is not
  copied here: the generator makes it byte for byte (`tools/check.sh`'s
  `gen src/pvr/internal.rs` row).
- **`tests/reference/pvr_internal.rs` (Mesa's programs).** The patch does
  not change it. `tools/glsl2usc.sh` writes into it only the `usc-blob`
  blocks of `fbload.frag` (`tools/glsl2usc.sh:54-62`). An `internal` line is
  our compiler's alone, and `mipgen.frag` has no Mesa twin either
  (`tests/pvr_mipgen_program.rs:18-20`).
  - The generator could not be rerun here. It refused this host's toolchain
    (`pco_shc=78f49122d7fb glslang=bfcc2479c85b` against the recorded
    `01aa62a069bb` / `2e8c31daf6b2`) rather than write a new reference.
  - A host with the recorded toolchain shows it unchanged (`tools/check.sh`
    row `gen tests/reference/blobs.rs == tools/glsl2usc.sh`).

```diff
diff --git a/src/pvr/backend.rs b/src/pvr/backend.rs
index 103ceb20..83623d81 100644
--- a/src/pvr/backend.rs
+++ b/src/pvr/backend.rs
@@ -1623,6 +1623,9 @@ struct Backend {
     /// draws level n of a chain by sampling level n-1 at the centre of each
     /// of its texels ([`pvr_generate_mipmap`]).
     mipgen_fs: OsalGpuMem,
+    /// The tail route's fragment program ([`PVR_INTERNAL_MIPGEN_TAIL`]): a
+    /// chain's small levels in one kick ([`record_mip_tail`]).
+    mipgen_tail_fs: OsalGpuMem,
     /// The fence of the last kick of the last chain made on the GPU (0:
     /// none): what [`pvr_return_mipmap_chain`] waits for before the GL core
     /// reads or writes such a chain's levels on the CPU.
@@ -1901,6 +1904,10 @@ struct FbLoad {
     /// ([`record_mip_level`]; its draw is [`MIP_DRAW`]).
     mip_fs_sh: ShcShader,
     mip_scale: [f32; 2],
+    /// The tail route's program as our compiler recorded it
+    /// (`mipgen_tail.frag`) and its block ([`record_mip_tail`]).
+    mip_tail_fs_sh: ShcShader,
+    mip_tail_u: [f32; MIP_TAIL_UNIFORM_DWORDS],
 }
 
 /// Keep a framebuffer's depth and stencil apart when they are two
@@ -3228,6 +3235,9 @@ fn static_objects_init(b: &mut Backend, fl: &mut FbLoad) -> i32 {
     if rc == 0 {
         rc = upload_words(&mut b.mipgen_fs, internal_words(internal(PVR_INTERNAL_MIPGEN)));
     }
+    if rc == 0 {
+        rc = upload_words(&mut b.mipgen_tail_fs, internal_words(internal(PVR_INTERNAL_MIPGEN_TAIL)));
+    }
     if rc == 0 {
         rc = alloc_mapped(HEAP_GENERAL_BASE, QUAD_BYTES, &mut b.fbload_quad);
     }
@@ -3276,6 +3286,7 @@ fn static_objects_init(b: &mut Backend, fl: &mut FbLoad) -> i32 {
     /* the GPU mipmap quad's FS: the same shape (one texture at sh0..19, a
      * two-float block after it, gl_FragCoord.y's pair after that) */
     fl.mip_fs_sh = *internal(PVR_INTERNAL_MIPGEN);
+    fl.mip_tail_fs_sh = *internal(PVR_INTERNAL_MIPGEN_TAIL);
     /* EOT (pixel event) DATA segment slot; rewritten per frame with the
      * framebuffer address. The CODE lives at the static PDS offset the
      * firmware expects (PVR_RENDER_EOT_PDS_CODE_OFF, placed by the driver). */
@@ -3471,6 +3482,7 @@ pub extern "C" fn pvr_backend_shutdown() {
         &mut b.fbload_fs,
         &mut b.fbload_vs,
         &mut b.mipgen_fs,
+        &mut b.mipgen_tail_fs,
         &mut b.fbload_quad,
         &mut b.black_tex,
         &mut b.seq_idx,
@@ -6205,6 +6217,160 @@ fn record_mip_level(p: &mut Pvr, base: u64, src: &MipLevel, dst: &MipLevel, hwrt
 /// floats, bools, null pointers), as `PVR` is.
 static MIP_DRAW: Global<GlesDraw> = Global::zeroed();
 
+/// The most levels the tail kick draws ([`record_mip_tail`]): what
+/// `mipgen_tail.frag`'s arrays hold. Six levels need 255 temporaries, which
+/// our register allocator refuses; five need 219.
+const MIP_TAIL_LEVELS: usize = 5;
+
+/// The last level drawn by a kick of its own, at the least, before the tail
+/// kick: levels 1 ..= 3 of terrain's chains stay today's kicks, so the
+/// large levels it samples keep today's bytes by construction.
+const MIP_TAIL_MIN_S: usize = 3;
+
+/// `mipgen_tail.frag`'s block: `u_lv[5]`, `u_ly[5]`, `u_sf`.
+const MIP_TAIL_UNIFORM_DWORDS: usize = 4 * (2 * MIP_TAIL_LEVELS + 1);
+
+/// The tail kick of a chain ([`mip_tail_plan`]): levels `s` + 1 .. `n` - 1
+/// drawn from level `s` as one `pitch` x `rows` surface over their bytes,
+/// and, when the range is odd (`last`), its last texel through a second
+/// 1x1 surface.
+#[derive(Clone, Copy, PartialEq, Eq, Debug)]
+struct MipTail {
+    s: usize,
+    pitch: u32,
+    rows: u32,
+    last: bool,
+}
+
+/// Whether the chain `lv` (`n` levels) has a tail kick, and its surface:
+/// the tail is the last [`MIP_TAIL_LEVELS`] levels or fewer, after level
+/// [`MIP_TAIL_MIN_S`] at the least, of at least two levels. Every level of
+/// it a power-of-two pitch (the program finds a texel's row by a shift).
+/// The range's texels, less a last one when their count is odd, are the
+/// surface: the narrowest even pitch at or past the square root that
+/// divides them, inside level 1's grid (`lv[1]`, the chain's one
+/// render-target size). `None`: the chain keeps a kick a level.
+fn mip_tail_plan(lv: &[MipLevel], n: usize) -> Option<MipTail> {
+    if n < 2 {
+        return None;
+    }
+    let s = MIP_TAIL_MIN_S.max((n - 1).saturating_sub(MIP_TAIL_LEVELS));
+    if s + 2 >= n {
+        return None;
+    }
+    let tail = &lv[s + 1..n];
+    if tail.iter().any(|l| !l.pitch.is_power_of_two()) {
+        return None;
+    }
+    let texels: u32 = tail.iter().map(|l| l.pitch * l.rows).sum();
+    let last = texels % 2 != 0;
+    if last && (tail[tail.len() - 1].pitch, tail[tail.len() - 1].rows) != (1, 1) {
+        return None;
+    }
+    let body = texels - u32::from(last);
+    let mut pitch = 2;
+    while pitch * pitch < body {
+        pitch += 2;
+    }
+    while pitch <= lv[1].w {
+        if body % pitch == 0 && body / pitch <= lv[1].h {
+            return Some(MipTail { s, pitch, rows: body / pitch, last });
+        }
+        pitch += 2;
+    }
+    None
+}
+
+/// The tail kick of the chain `lv` at `base` ([`mip_tail_plan`]): a frame of
+/// its own, as [`record_mip_level`]'s, into the surface over levels
+/// `t.s` + 1 .. `n` - 1 -- level s+1's address, `t.pitch` texels a row,
+/// `t.rows` rows, the store clipped to it on level 1's grid -- with one
+/// full-screen quad drawn by [`PVR_INTERNAL_MIPGEN_TAIL`] sampling level
+/// `t.s` as the level kicks sample theirs. Its block holds each tail level's
+/// u_scale, exactly the floats [`record_mip_level`] would give it, its size,
+/// offset and pitch. An odd range's last texel goes out through a second
+/// 1x1 surface at the last level's address. 0, or the draw's error.
+fn record_mip_tail(p: &mut Pvr, base: u64, lv: &[MipLevel], n: usize, t: &MipTail) -> i32 {
+    // SAFETY: as record_mip_level's.
+    let d = unsafe { MIP_DRAW.get() };
+    let fl = &mut p.fbload;
+    let (src, top) = (&lv[t.s], &lv[t.s + 1]);
+    let levels = n - t.s - 1;
+    fl.mip_tail_u = [0.0; MIP_TAIL_UNIFORM_DWORDS];
+    for (k, l) in lv[t.s + 1..n].iter().enumerate() {
+        let lv4 = &mut fl.mip_tail_u[4 * k..4 * k + 4];
+        lv4.copy_from_slice(&[1.0 / l.w as f32, 1.0 / l.h as f32, l.w as f32, l.h as f32]);
+        let ly = &mut fl.mip_tail_u[4 * (MIP_TAIL_LEVELS + k)..4 * (MIP_TAIL_LEVELS + k) + 4];
+        let off = (l.off - top.off) / RGBA8_BYTES;
+        ly.copy_from_slice(&[off as f32, l.pitch as f32, 1.0 / l.pitch as f32, 0.0]);
+    }
+    let sf = 4 * 2 * MIP_TAIL_LEVELS;
+    fl.mip_tail_u[sf..sf + 4].copy_from_slice(&[t.pitch as f32, f32::from(u8::from(t.last)), levels as f32, 0.0]);
+    let block: *const f32 = fl.mip_tail_u.as_ptr();
+    let (vs_sh, fs_sh): (*const ShcShader, *const ShcShader) = (&fl.vs_sh, &fl.mip_tail_fs_sh);
+
+    p.b.mip_clip = (t.pitch, t.rows);
+    *d = GlesDraw::ZERO;
+    d.target_gpu = base.wrapping_add(top.off as u64);
+    d.target_w = lv[1].w.max(t.pitch);
+    d.target_h = lv[1].h.max(t.rows);
+    d.target_pitch = t.pitch;
+    d.target_format = GLES_SURF_RGBA8888;
+    d.target_alpha_bits = 8;
+    if t.last {
+        d.num_targets = 2;
+        d.targets[1] = GlesDrawTarget {
+            gpu: base.wrapping_add(lv[n - 1].off as u64),
+            w: 1,
+            h: 1,
+            format: GLES_SURF_RGBA8888,
+            out_shift: 0,
+        };
+    }
+    d.do_clear = true;
+    d.clear_color_mask = 0xf;
+    d.prim = GLES_PRIM_TRIANGLES;
+    d.index_count = 6;
+    d.vs = vs_sh;
+    d.fs = fs_sh;
+    d.vs_usc_gpu = p.b.fbload_vs.gpu;
+    d.fs_usc_gpu = p.b.mipgen_tail_fs.gpu;
+    d.attribs[0] =
+        GlesAttrib { vi_reg: 0, components: 4, stride: 16, buffer_gpu: p.b.fbload_quad.gpu, ..GlesAttrib::ZERO };
+    d.num_attribs = 1;
+    d.vs_output_size_dwords = 4;
+    d.textures[0] = GlesTextureBind {
+        unit: 0,
+        format: GLES_TEXFMT_RGBA8,
+        type_: GLES_TEXTYPE_2D,
+        layers: 1,
+        base_gpu: base.wrapping_add(src.off as u64),
+        width: src.w,
+        height: src.h,
+        stride_texels: src.pitch,
+        bytes: src.pitch.wrapping_mul(src.rows).wrapping_mul(RGBA8_BYTES),
+        linear_filter: 1,
+        min_linear: 1,
+        mip_filter: 0,
+        mip_levels: 1,
+        wrap_s: GLES_WRAP_CLAMP_TO_EDGE,
+        wrap_t: GLES_WRAP_CLAMP_TO_EDGE,
+        ..GlesTextureBind::ZERO
+    };
+    d.num_textures = 1;
+    d.viewport = [0, 0, t.pitch as i32, t.rows as i32];
+    d.depth_range = [0.0, 1.0];
+    d.depth_func = GLES_CMP_ALWAYS;
+    d.fs_uniform_cpu = block as *const c_void;
+    d.fs_uniform_dwords = MIP_TAIL_UNIFORM_DWORDS as u16;
+    d.color_write = true;
+    d.color_mask = 0xf;
+    d.line_width = 1.0;
+    let rc = draw_one(p, d);
+    p.b.mip_clip = (0, 0);
+    rc
+}
+
 /// The levels of a chain the GPU draws are those over this many texels, and
 /// level 1 always; the GL core filters the rest on the CPU with the GPU's
 /// filter (gl/texture.rs `mip_step_gpu_sample`) once they completed.
@@ -6265,8 +6431,14 @@ fn mip_chain_gpu(p: &mut Pvr, t: &GlesTextureBind, levels: u32) -> Result<Option
         return Err("a level one texel wide above a taller tail: the PBE writes pixel pairs");
     }
     let last = mip_gpu_last(&lv, n, p.b.mip_cpu_tail);
-    for l in 1..=last {
-        let rc = record_mip_level(p, t.base_gpu, &lv[l - 1], &lv[l], (lv[1].w, lv[1].h));
+    /* the tail route: the small levels in one kick after the large ones */
+    let tail = if last + 1 == n { mip_tail_plan(&lv, n) } else { None };
+    let kicked = tail.map_or(last, |tl| tl.s);
+    for l in 1..=kicked + usize::from(tail.is_some()) {
+        let rc = match tail {
+            Some(tl) if l > kicked => record_mip_tail(p, t.base_gpu, &lv, n, &tl),
+            _ => record_mip_level(p, t.base_gpu, &lv[l - 1], &lv[l], (lv[1].w, lv[1].h)),
+        };
         if rc != 0 {
             /* the levels recorded so far may be in flight, and the CPU
              * filter is about to rewrite them: wait them out first */
@@ -9387,6 +9559,7 @@ pub extern "C" fn pvr_backend_pool_owners() -> u64 {
         &b.fbload_fs,
         &b.fbload_vs,
         &b.mipgen_fs,
+        &b.mipgen_tail_fs,
         &b.fbload_quad,
         &b.black_tex,
         &b.seq_idx,
diff --git a/tools/shaders/precompiled.list b/tools/shaders/precompiled.list
index 392a80c0..8a7cf5f8 100644
--- a/tools/shaders/precompiled.list
+++ b/tools/shaders/precompiled.list
@@ -179,6 +179,9 @@ internal powervr fbload_u32x4   internal/es/fbload_int.frag replace@rgba32ui
 # (src/pvr/backend.rs pvr_generate_mipmap; the chain is otherwise the GL
 # core's CPU box filter).
 internal powervr mipgen         internal/es/mipgen.frag
+# ... and the tail route's: the chain's small levels in one kick, as one
+# linear surface over their bytes (docs/notes/pvr-mip-tail-e1.md).
+internal powervr mipgen_tail    internal/es/mipgen_tail.frag
 
 # The GC7000's: its clear quad (blending and colour masks are pixel-engine
 # state there, so one fragment shader clears every mask), and the GPU
diff --git a/tools/shaders/internal/es/mipgen_tail.frag b/tools/shaders/internal/es/mipgen_tail.frag
new file mode 100644
index 00000000..df783459
--- /dev/null
+++ b/tools/shaders/internal/es/mipgen_tail.frag
@@ -0,0 +1,118 @@
+#version 300 es
+/* glGenerateMipmap on the GPU, the tail of a chain in one kick (the PowerVR's
+ * tail route, docs/notes/pvr-mip-tail-e1.md; src/pvr/backend.rs
+ * record_mip_tail): levels s+1 .. s+n (n <= 5: a sixth level needs more
+ * temporaries than our allocator has) of a 2D RGBA8 chain follow each other
+ * in the storage, so their bytes are one range, drawn as one linear surface
+ * of u_sf.x texels a row. Pixel (X, Y) is texel i = Y * u_sf.x + X of the
+ * range; the level it falls in is found from the levels' offsets (u_ly[k].x,
+ * in texels from level s+1) and its (x, y) from that level's pitch
+ * (u_ly[k].y, a power of two: u_ly[k].z is 1 / it).
+ *
+ * Level s+1 is what mipgen.frag draws: the source level s, bound alone,
+ * sampled at the centre of the texel at that level's u_scale (u_lv[0].xy),
+ * packed to 8 bits -- a leaf. Every deeper level k is the bilinear filter,
+ * in the ALU, of four texels of level k-1 at the point (x + 0.5) *
+ * u_lv[k].xy (the u_scale the per-level kick gets) times level k-1's size
+ * (u_lv[k-1].zw), CLAMP_TO_EDGE, packed to 8 bits again: what the per-level
+ * kick computes with the TPU. The 4^k leaves under a texel are walked in one
+ * loop, digit m of the leaf's number choosing the corner at level m, with
+ * one partial row (t, r) a level kept for the reduction. Values are kept as
+ * 0..255.
+ *
+ * A range of odd length leaves its last texel -- the last level's (0, 0),
+ * a 1x1 level at pitch 1 -- past the even surface: pixel (0, 0) computes it
+ * too, after its own texel (u_sf.y = 1), and writes it to output 1, which
+ * the backend stores through a second 1x1 surface at that level's address.
+ * Padding (x >= w or y >= h of a level) is 0. */
+precision highp float;
+precision highp int;
+uniform highp sampler2D u_src;
+/* level s+1+k: (1/w, 1/h) -- the u_scale mipgen.frag gets -- then (w, h) */
+uniform vec4 u_lv[5];
+/* level s+1+k: (first texel from level s+1, pitch, 1 / pitch, 0) */
+uniform vec4 u_ly[5];
+/* (surface pitch, 1: an odd range's last texel goes to output 1, levels in
+ * the tail, 0) */
+uniform vec4 u_sf;
+layout(location = 0) out vec4 o_color;
+layout(location = 1) out vec4 o_last;
+
+void main() {
+    vec2 px = floor(gl_FragCoord.xy);
+    float i = px.y * u_sf.x + px.x;
+    /* the level k this texel is in, and the texel (x, y) of it */
+    float k = 0.0;
+    vec2 xy = vec2(0.0);
+    for (int l = 0; l < 5; l++) {
+        if (float(l) < u_sf.z && i >= u_ly[l].x) {
+            float j = i - u_ly[l].x;
+            float y = floor((j + 0.5) * u_ly[l].z);
+            k = float(l);
+            xy = vec2(j - y * u_ly[l].y, y);
+        }
+    }
+    bool pad = false;
+    for (int l = 0; l < 5; l++) {
+        if (k == float(l)) {
+            pad = xy.x >= u_lv[l].z || xy.y >= u_lv[l].w;
+        }
+    }
+    /* the 4^k leaves under texel xy of level k, reduced level by level; then,
+     * at pixel (0, 0) of an odd range, the 4^(n-1) under the last level's */
+    float own = exp2(2.0 * k);
+    float kl = u_sf.z - 1.0;
+    float leaves = own + (u_sf.y > 0.5 && i == 0.0 ? exp2(2.0 * kl) : 0.0);
+    vec2 f1 = vec2(0.0), f2 = vec2(0.0), f3 = vec2(0.0), f4 = vec2(0.0);
+    vec4 t1 = vec4(0.0), t2 = vec4(0.0), t3 = vec4(0.0), t4 = vec4(0.0);
+    vec4 r1 = vec4(0.0), r2 = vec4(0.0), r3 = vec4(0.0), r4 = vec4(0.0);
+    vec4 v = vec4(0.0);
+    vec4 mine = vec4(0.0);
+    for (int n = 0; float(n) < leaves; n++) {
+        bool last = float(n) >= own;
+        float nf = last ? float(n) - own : float(n);
+        float kk = last ? kl : k;
+        /* the corner digit m of n (DIGIT, S = 4^-(m-1)): which of the four
+         * texels of level m-1 that level m's texel filters */
+#define DIGIT(S) (floor(nf * (S)) - 4.0 * floor(nf * (S) * 0.25))
+        /* down */
+        vec2 p = last ? vec2(0.0) : xy;
+#define DOWN(M, L, F, S) \
+        if (kk >= float(M)) { \
+            float D = DIGIT(S); \
+            vec2 u = (p + 0.5) * u_lv[M].xy * u_lv[L].zw - 0.5; \
+            vec2 i0 = floor(u); \
+            F = u - i0; \
+            p = clamp(i0 + vec2(mod(D, 2.0), floor(D * 0.5)), vec2(0.0), u_lv[L].zw - 1.0); \
+        }
+        DOWN(4, 3, f4, 0.015625)
+        DOWN(3, 2, f3, 0.0625)
+        DOWN(2, 1, f2, 0.25)
+        DOWN(1, 0, f1, 1.0)
+        v = floor(textureLod(u_src, (p + 0.5) * u_lv[0].xy, 0.0) * 255.0 + 0.5);
+        /* up: the leaf completes a row, a texel, or nothing yet */
+        bool carry = true;
+#define UP(M, F, T, R, S) \
+        if (carry && kk >= float(M)) { \
+            float D = DIGIT(S); \
+            if (D == 0.0 || D == 2.0) { \
+                T = v; \
+                carry = false; \
+            } else if (D == 1.0) { \
+                R = mix(T, v, F.x); \
+                carry = false; \
+            } else { \
+                v = floor(mix(R, mix(T, v, F.x), F.y) + 0.5); \
+            } \
+        }
+        UP(1, f1, t1, r1, 1.0)
+        UP(2, f2, t2, r2, 0.25)
+        UP(3, f3, t3, r3, 0.0625)
+        UP(4, f4, t4, r4, 0.015625)
+        if (!last) {
+            mine = v;
+        }
+    }
+    o_color = pad ? vec4(0.0) : mine * (1.0 / 255.0);
+    o_last = v * (1.0 / 255.0);
+}
```

### What the patch breaks on purpose: tests that pin a kick a level

On an experiment branch these fail, because they pin today's one kick a
level. Update them in the same experiment branch if it is ever taken
further:

- `src/pvr/backend_tests.rs`:
  - `a_64x64_chain_is_drawn_level_by_level_into_the_storage`
  - `an_800x600_chain_is_drawn_as_the_power_of_two_chain_it_is_laid_out_in`
  - `a_chain_asked_for_during_a_frame_into_level_0_follows_that_frames_kick`
  - `asking_the_chain_back_waits_for_its_last_kick`
  - `chains_the_gpu_cannot_make_are_left_to_the_cpu_by_name`
  - Their helper `level_draws` expects every chain kick to carry the
    two-float block.
- `tests/mipmap_offer_guard.rs` `pvr::*_gets_its_chain_from_the_gpu` (3
  tests): `call.kicks` is one a level.

Every other test run here passed with the patch applied:

- the other 205 `pvr::` unit tests;
- `tests/pvr_mipgen_program.rs` (7);
- `tests/es2_report_guard.rs` (16, `report-es2`).

## (2) Result on the host

### Kicks a terrain frame

`tools/hostharness/am62p-terrain/run.sh $CONNECTED_DEPS_DIR/glmark2-src
build/e1-<x> kicks` (glmark2 at `22c527cb`, `GM2_FRAMES=20`, `kicks.py`).
Each draw is listed as its framebuffer and the kicks since the draw before
it.

| tree | kicks a frame (frames 1..18) | per draw |
|---|---|---|
| main `ea371491` | **52.00** | fb1+2 fb2+1 fb4+1 fb5+10 fb6+9 fb4+9 fb7+10 fb0+10 |
| `ea371491` + patch | **32.00** | fb1+2 fb2+1 fb4+1 fb5+6 fb6+5 fb4+5 fb7+6 fb0+6 |

- The five chains a frame (`terrain-mip-usage.md`: 9 + 8 + 8 + 9 + 9 = 43
  kicks) become 5 + 4 + 4 + 5 + 5 = 23.
- Both runs: `scene-end [terrain]: ok frames=20 draws=161 refused=0
  gl_errors=0 incomplete_draws=0`, and no backend refusal line.
- The run was repeated after the last change to the program's comment; the
  program's words were the same, and so was the count.

### What the tail kick records (backend words, appendix A)

The rig test `e1_the_tail_kick_of_terrains_chains` (appendix A, passed)
reads the recorded kicks of both chains:

- levels 1..=s are one kick each, then one tail kick;
- the tail kick declares level 1's size;
- emit 0 is at level s+1's address, with
  `pbe_reg_words(RGBA8888, P, R, pitch P)`;
- for 256x256, `num_rt` is 2 and emit 1 is at level 8's address with 1x1
  words; for 800x600, `num_rt` is 1;
- the fragment program is `mipgen_tail_fs`;
- the 44-dword block holds each level's `1/w, 1/h` exactly as `record_mip_level`
  makes them, the pitches, and `u_sf`.

### Level contents against today's chain (appendix B)

`tests/pvr_mip_tail_e1.rs` (appendix B, passed; `cargo test
--no-default-features --features host-test,gpu-powervr,shaderc-online
--release --test pvr_mip_tail_e1 -- --nocapture`) does the following:

- It compiles `mipgen_tail.frag` with the online front end
  (`compile_to_ir`) and checks that the block's offsets are the ones
  `record_mip_tail` writes (u_lv 0, u_ly 20, u_sf 40).
- It runs the module with the reference IR evaluator
  (`src/shaderc/online/eval.rs`, `run_in_with_textures`) at every pixel of
  the P x R surface, with a `TexelSource` answering the look-ups.
- It compares each tail level's GL extent, texel by texel, against today's
  chain: a kick a level, each level the same model's sample of the level
  above, packed to 8 bits by round-half-up. The padding must be 0.

Level 0 is 800x600 or 256x256, and its content is either `random` (xorshift
bytes) or `smooth` (gradients plus noise of 0..6). Each line is one level:
texels that differ of the level's texels / bytes / max |d|.

| TPU model | content | 800x600: levels 5 (25x18), 6 (12x9), 7 (6x4), 8 (3x2), 9 (1x1) | 256x256: levels 4..8 |
|---|---|---|---|
| `alu`: the program's own arithmetic, `mix(mix(t00, t10, a), mix(t01, t11, a), b)` over 0..255 | random | 0, 0, 0, 0, 0 | 0 at every level |
| `alu` | smooth | 0, 0, 0, 0, 0 | 0 |
| `gl`: ES 3.0.6 3.8.10's four-weight sum over value/255 in f32 | random | 0, 0, 0, 0, 0 | 0 |
| `gl` | smooth | 0, 0, **7 of 24 (7 bytes, max 1)**, **2 of 6 (3 bytes, max 1)**, 0 | 0 |
| `w8`: `gl` with the fractions cut to 8 bits | random | 0, 0, 0, 0, 0 | 0 |
| `w8` | smooth | 0, **2 of 108 (2 bytes, max 1)**, **8 of 24 (8 bytes, max 1)**, **3 of 6 (4 bytes, max 1)**, 0 | 0 |

How to read it:

- **`alu` reads 0 everywhere.** The program's logic is what today's chain
  computes when the TPU computes its sample as the ALU does: the level
  search, the texel position, the corner digits, the clamp, the reduction
  order, the leftover texel and the zero padding.
  - The test catches a fault. With the corner's x and y swapped in the
    program, 800x600 level 6 read 95 of 108 texels differing, max 5.
- **The leaves are the same in every model.** Level s+1 (800x600's 5,
  256x256's 4) is the same sample of the same level at the same coordinate
  bits, so it reads 0 in all three.
- **`gl` and `w8` are where the bytes can move.** Whatever the TPU does
  differently from the ALU (precision, rounding, sub-texel bits) shows up
  only in the ALU levels. In these models that is up to 1 LSB, in levels of
  108 texels or fewer, on the 800x600 chain's odd steps (37 -> 18, 25 -> 12,
  9 -> 4, 3 -> 1). The power-of-two 256x256 chain read 0 in every model:
  its weights are exactly 1/2, so the four-weight sum and the two `mix`es
  agree there.
- **What the host cannot see.**
  - The board's TPU precision.
  - The tie behaviour of the pack (`pck.u8888` with `rtz = 0`,
    `src/shaderc/online/powervr/assemble.rs:1189`, rounds to nearest; this
    model rounds ties up).
  - Whether the back end compiles the module correctly. The byte model
    `tests/usc_exec` answers a look-up with a marker, not a filtered texel,
    so it cannot run this comparison on the USC words.

## (3) The board procedure, with the expected numbers

Use `target:am62p` with `target:am62p-console`. The images are built from
**two** commits:

- **before**: an origin/main commit. am62p-board-before's baseline may serve
  if it is the same commit.
- **after**: that commit plus this patch, with `src/pvr/internal.rs`
  regenerated. It is an experiment branch, not on origin/main, so it needs
  `--allow-local`.

1. **gl_test cases 216 and 219, both commits.** Each commit gets one image
   and one session, run with `tools/cts/pr-session.sh` (its build is
   `common-images.sh`'s, the partial-render knob off, as main):

   ```text
   tools/cts/pr-session.sh e1-before --knob off --cases '216 219'              # on the main commit
   tools/cts/pr-session.sh e1-tail   --knob off --cases '216 219' --allow-local # on the experiment commit
   ```

   Record values from the console (`driver/tools/pvrlab/gl_test.c:20179-20196`,
   `:20303-20324`), with the expected numbers:

   | line | before (main) | after (E1), expected |
   |---|---|---|
   | `gl_test:   per frame: N kicks (M over 20 frames)` | 40.00 (`RESULTS.md:300-302`, `61ad38a1`) | **24.00**: 3 chains of 9 -> 5, 1 chain of 8 -> 4, plus 5 passes |
   | `gl_test: mipchains: 20 frames, X ms a frame` | 46.15 at `61ad38a1` | about **27.8 + the 4 tail kicks' shading**, if a queued kick costs 1.15 ms (16 fewer kicks); about **35.7 + shading** if a kick is 0.65 ms plus one 20 ms fence a frame (`am62p-slow-terrain.md:212-215`) |
   | `gl_test:   per frame: TA busy, 3D busy, TA\|3D` | TA\|3D 7.75 ms | about the same, plus the tail's ALU: the levels drawn are the same pixels, and the deep levels' texels are fewer than one level-4 kick's tiles |
   | `gl_test: mipchains: reposts N over 20 frames` (new) | read it: about 20 means one 20 ms fence a frame (the second reading above) | not more than before. Fewer only if the fence followed the last kick of a chain |
   | `gl_test: case 216 mipchains: PASS` (the four quadrant probes) | PASS | **PASS**. The probes check quadrant centres, which are one colour at every level (`gl_test.c:20052-20054`), so they catch a hang or a broken picture, not a 1-LSB move |
   | `gl_test: kickpair: two: ... ms a rep` and `one: ... ms a rep`, `kicks a rep` 2.00 / 1.00 | read them | the same as before: case 219 does not mip. (two - one) is the marginal cost c of one queued render. It turns the kick count into ms below |

   - **E1's tail is confirmed** when case 216 reads 24.00 kicks, PASS, and
     its ms a frame falls by at least 16 x c - 2 ms. The 2 ms is slack for
     4 tail kicks' shading. With c = 1.15 that is a fall of at least 16 ms,
     to about 30 ms or less.
   - **If the kicks fall and the ms do not**, the frame is not kick-bound.
     It is the waits (`HWRT_MEMBERS` = 4, `pvr-waited-kick`), as path B
     showed.
   - **If the tail kick's own time is large**, look at its last-kick times.
     `pvr_render_last_kick_us` with `CONFIG_YAKOGL_DRV_GPU_PVR_KICK_TIMES`
     gives post->frag for the frame's last kick, which is a tail kick when
     the frame ends in a chain.

2. **glmark2 terrain, both commits.** Run `--glmark2` on the same two
   sessions, or `tools/cts/board-stage.sh am62p common` on the main commit
   and the same build of the experiment commit. Record values:

   | record value | before | after (E1), expected |
   |---|---|---|
   | terrain ms a frame (per-scene list; `FPS:`) | 83.06 ms at `6a7021ec` (`RESULTS.md:83`), am62p-board-before's at the current main | 83.06 - 20 x c: about **60 ms** at c = 1.15, about **70 ms** at c = 0.65. 20 fewer kicks a frame: host 52 -> 32, board about 53 -> 33 |
   | `glmark2-frame: [terrain]` gl and swap us | read | gl time falls with the kicks, if the waits are in GL calls (`am62p-slow-terrain.md:133`) |
   | `render: fence N (frame fence) not moved at 20 ms, re-posted ...` lines (M so far) | read | not more |
   | GL errors, incomplete draws, `verdict=done`, resets | 0, 0, done, 0 | **the same** |
   | pictures, column `am62p` of `tools/cts/glmark2-pictures.txt`, tolerance 8 | 28 match / 5 mismatch (animated probes) at `6a7021ec` | **the same split**; terrain's mean `119 129 44` within 1. The five per-frame chains are sampled at levels 0-2 only (`terrain-mip-usage.md`, its table). The tail's ALU levels reach the picture only through the 512x512 chain built once at setup (`the GPU made levels 1..=9 of the 10-level chain of a 512x512 texture`), which terrain's oblique plane samples at every level |
   | the other mipmapping scenes (refract: its 1600x1200 chain 10 -> 6 kicks) | 35.09 ms | lower by about 4 x c |

3. **E1's read-back: the tail's levels against today's on the GPU.** No
   gl_test case reads back levels 5..9 today. Case 216's probes see levels
   1-2 only. Two ways, cheapest first:
   - **dEQP-GLES2 `functional.texture.mipmap.2d.*`, both commits**, from
     `tools/cts/board-stage.sh am62p deqp` (the whole tree) or a case-list
     image. `generate.rgba8888_*` takes the GPU chain on the board for RGBA8
     (`am62p-slow-terrain.md:259-261`). With the patch, a 64-texel chain's
     levels 4..6 come from the tail kick: level 4 is a leaf, so it is
     identical, and levels 5 and 6 come from the ALU (see the 64x64 row
     above). Expected: the same pass/fail per case as before. A new
     failure is a texel the ALU puts out of dEQP's threshold.
   - **A new gl_test read-back case.** This is new work, not in this item.
     Level 0 is RGBA8 random bytes at 800x600 and 256x256. Generate the
     chain twice, once with the tail and once with the tail off (a debug
     switch on `mip_tail_plan`). Attach each level l >= s+1 to an FBO; the
     gl_test image reports ES 3.0, which allows a non-zero level. Read it
     back with `glReadPixels`, and print `level l: differing bytes N, max |d|
     M` (the study's E1 lines).
     - **N = 0** at every level: the TPU equals the ALU, and the route is
       bit-identical.
     - **M <= 1**: within the pictures' tolerance; dEQP decides.
     - **M > 1**: the ALU emulation or the coordinate is wrong. Fix it
       before any route is built.

Put the records in `docs/notes/` (am62p-board-before and am62p-board-after,
or a note of the session's own) with:

- each image's commit;
- the `verdict=` line;
- the records path (`records/am62p/...`, `tools/cts/log/<date>-pr-e1-*`).

## (4) Risk to pictures and ES 2.0 answers

- **Levels 1..s**: the same kicks, byte for byte. That is levels 1..4 of
  800x600 and 1..3 of 256x256: every level terrain samples (levels 1-2,
  `terrain-mip-usage.md`) and the large ones.
- **Level s+1**: the same `Q(T(L_s, c))`. The same sampler state (linear,
  one level, CLAMP_TO_EDGE) and the same coordinate bits. `textureLod(.., 0)`
  replaces `texture()`; with a one-level sampler whose min and mag filters
  are both linear, both select the same filter. The 8-bit value then goes
  out as n/255, which the pack returns as n.
- **Levels s+2..**: the ALU's bilinear in place of the TPU's.
  - On the host the difference is 0 against the ALU model. It is at most
    1 LSB, in levels of 108 texels or fewer, against the two TPU models
    tried.
  - On the board it is unknown until step 3.
  - Ties on even steps: the ALU rounds half up (`floor(v + 0.5)`). If the
    TPU's average or `pck`'s rounding goes to even, power-of-two levels can
    move by 1 LSB at ties. That covers 64x64's levels 5-6 and 256x256's
    levels 5-8.
- **glmark2 pictures** (column `am62p`, tolerance 8). Terrain's per-frame
  chains are sampled at levels 0-2 (`terrain-mip-usage.md`), all of them
  per-level kicks here. The 512x512 chain built once at setup is sampled at
  every level by the distant plane, and its levels 6..9 (8x8 .. 1x1) are
  ALU levels. A 1-LSB move there cannot move a probe by 8. Expected: no
  change.
- **dEQP-GLES2**:
  - **In scope:** `texture.mipmap.2d.generate.*`, the RGBA8 ones (the GPU
    chain takes RGBA8 2D only, `mip_chain_gpu` `:6242-6247`). Also any
    `texture.mipmap.2d.*` or `texture.filtering.*` case that calls
    glGenerateMipmap on an RGBA8 2D texture of 32 texels or more a side
    (n >= 6, so that s + 2 < n).
  - **Not in scope:** cube maps, other formats, and chains below that size,
    which keep their per-level kicks.
  - dEQP compares mipmaps with a threshold that allows a filter's
    rounding, so 1 LSB is expected to pass. A failure means M > 1.
  - The whole `texture.mipmap` group should be rerun on both commits.
- **ES 2.0 API answers** (`tests/es2_report_guard.rs`): no change. The guard
  runs a stand-in with `generate_mipmap: None`
  (`tests/es2_report_guard.rs:132`), and its 16 tests pass with the patch.
- **Other risks, not pictures.**
  - The program is large (3898 dwords, 219 temporaries). Few instances fit
    in the USC at once, so the tail kick's shading time (estimated under a
    millisecond) is a board figure. Read it from case 216's ms and the
    kick-time build.
  - It is the first internal program with a loop that runs a per-pixel
    number of times, and the first internal draw with two PBE emits of
    different sizes.
  - A loop or EOT fault would show as a hang or a wrong picture in case 216.
    That is why case 216 runs first and alone.

## Appendix A: the rig test (scratch, `src/pvr/backend_tests.rs`, appended)

```diff
diff --git a/src/pvr/backend_tests.rs b/src/pvr/backend_tests.rs
index aea7d38e..3183d7f1 100644
--- a/src/pvr/backend_tests.rs
+++ b/src/pvr/backend_tests.rs
@@ -6489,3 +6489,53 @@ fn spm_load_blob_matches_its_generator() {
         assert_eq!(off, 200, "{groups} groups end at the last byte");
     }
 }
+
+/// E1 scratch (docs/notes/pvr-mip-tail-e1.md, never committed): the tail
+/// kick of terrain's two chain sizes -- the levels before it a kick each,
+/// then one frame on level 1's grid into level s+1's address at the
+/// surface's pitch, clipped to it, drawn by the tail program with its
+/// 44-dword block; an odd range's last texel a second 1x1 emit at the last
+/// level's address.
+#[test]
+fn e1_the_tail_kick_of_terrains_chains() {
+    for (w, h, stride, n, s, p, r, last) in
+        [(800u32, 600u32, 1024u32, 10u8, 4usize, 44u32, 31u32, false), (256, 256, 256, 9, 3, 20, 17, true)]
+    {
+        let rig = Rig::new();
+        with_platform(|pl| pl.kick_usc_rates.clear());
+        let base = HEAP_GENERAL_BASE + 0x2000_0000;
+        let b = mip_bind(base, w, h, stride, n);
+        assert_eq!(generate(&b, n as u32), 1);
+        let mut lv = [MipLevel::default(); CAP_MIP_LEVELS];
+        mip_chain_layout(w, h, stride, n as u32, &mut lv).unwrap();
+        assert_eq!(mip_tail_plan(&lv, n as usize), Some(MipTail { s, pitch: p, rows: r, last }));
+        let kicks = rig.kicks();
+        let emits = rig.eot_emits();
+        assert_eq!(kicks.len(), s + 1, "{w}x{h}: levels 1..={s} a kick each, then the tail");
+        let (k, _) = &kicks[s];
+        assert_eq!((k.width, k.height), (lv[1].w, lv[1].h), "level 1's grid");
+        let e = &emits[s];
+        assert_eq!(emit_address(e[0], e[1]), base + u64::from(lv[s + 1].off), "level s+1's address");
+        assert_eq!(k.frag.pbe_word[0], pbe_reg_words(GLES_SURF_RGBA8888, false, p, r, p, false, false));
+        if last {
+            assert_eq!(k.frag.num_rt, 2);
+            assert_eq!(emit_address(e[2], e[3]), base + u64::from(lv[n as usize - 1].off), "the last level's");
+            assert_eq!(k.frag.pbe_word[1], pbe_reg_words(GLES_SURF_RGBA8888, false, 1, 1, 2, false, false));
+        } else {
+            assert_eq!(k.frag.num_rt, 1);
+        }
+        let tail = heap_off_of(rig.b().mipgen_tail_fs.gpu) as u64;
+        let fs = with_platform(|pl| pl.kick_usc_rates.iter().map(|x| x.0).collect::<Vec<_>>());
+        assert_eq!(fs[s], tail, "the tail kick runs the tail program");
+        let dk = &rig.dma_kicks()[s];
+        let words = &with_gen(|g| g.dma_words.clone())[s];
+        let uni = dk.iter().position(|x| x.2 == TEX_DESC_DWORDS && x.1 == 44).expect("the 44-dword block");
+        let u: Vec<f32> = words[uni].iter().map(|&x| f32::from_bits(x)).collect();
+        for k in 0..(n as usize - s - 1) {
+            let l = &lv[s + 1 + k];
+            assert_eq!(u[4 * k..4 * k + 4], [1.0 / l.w as f32, 1.0 / l.h as f32, l.w as f32, l.h as f32]);
+            assert_eq!(u[20 + 4 * k + 1], l.pitch as f32);
+        }
+        assert_eq!(u[40..43], [p as f32, if last { 1.0 } else { 0.0 }, (n as usize - s - 1) as f32]);
+    }
+}
```

## Appendix B: the level-content test (scratch, `tests/pvr_mip_tail_e1.rs`)

```diff
diff --git a/tests/pvr_mip_tail_e1.rs b/tests/pvr_mip_tail_e1.rs
new file mode 100644
index 00000000..e8be8d4b
--- /dev/null
+++ b/tests/pvr_mip_tail_e1.rs
@@ -0,0 +1,296 @@
+// SPDX-License-Identifier: Apache-2.0
+// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
+//! E1 of docs/notes/pvr-mip-fewer-kicks.md, the tail route, on the host
+//! (docs/notes/pvr-mip-tail-e1.md): `mipgen_tail.frag`'s module, run by the
+//! reference IR evaluator over every pixel of the tail kick's surface, its
+//! texture look-ups answered by a model of the TPU's bilinear sample, against
+//! today's chain -- a kick a level, each level the same model's sample of the
+//! one above, packed to 8 bits -- at glmark2 terrain's two chain sizes. It
+//! prints each tail level's differing texels and bytes and the largest
+//! difference, per TPU model:
+//!
+//! - `alu`: the sample computed as the program's ALU computes it (two
+//!   `mix`es over 0..255 values): differences here are the program's logic
+//!   (level, texel, corner, clamp), and must be none.
+//! - `gl`: ES 3.0.6 3.8.10's four-weight sum over value / 255 in f32.
+//! - `w8`: `gl` with the fractions cut to 8 bits, as a texture unit with
+//!   8 sub-texel bits would.
+//!
+//! ```text
+//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_mip_tail_e1 -- --nocapture
+//! ```
+
+#![cfg(all(feature = "host-test", feature = "shaderc-online"))]
+
+use yakogl::shaderc::online::arena::Arena;
+use yakogl::shaderc::online::eval::{self, Fault, Frame, Inputs, Sink, TexLookup, TexelSource};
+use yakogl::shaderc::online::ir::{IoSlot, TexDim};
+use yakogl::shaderc::online::lower::compile_to_ir;
+use yakogl::shaderc::online::sema::Stage;
+
+const SRC: &str = include_str!("../tools/shaders/internal/es/mipgen_tail.frag");
+
+#[derive(Clone, Copy, PartialEq, Debug)]
+enum Tpu {
+    Alu,
+    Gl,
+    W8,
+}
+
+/// One RGBA8 level, its GL extent, tightly packed.
+#[derive(Clone)]
+struct Level {
+    w: u32,
+    h: u32,
+    px: Vec<[u8; 4]>,
+}
+
+impl Level {
+    fn at(&self, x: i64, y: i64) -> [u8; 4] {
+        let x = x.clamp(0, self.w as i64 - 1) as u32;
+        let y = y.clamp(0, self.h as i64 - 1) as u32;
+        self.px[(y * self.w + x) as usize]
+    }
+}
+
+fn q(v: f32) -> u8 {
+    (v * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8
+}
+
+/// The model's sample of `l` at the normalised point `c`, as floats.
+fn sample(l: &Level, c: [f32; 2], tpu: Tpu) -> [f32; 4] {
+    let u = c[0] * l.w as f32 - 0.5;
+    let v = c[1] * l.h as f32 - 0.5;
+    let (i0, j0) = (u.floor(), v.floor());
+    let (mut a, mut b) = (u - i0, v - j0);
+    if tpu == Tpu::W8 {
+        a = (a * 256.0).floor() / 256.0;
+        b = (b * 256.0).floor() / 256.0;
+    }
+    let (i0, j0) = (i0 as i64, j0 as i64);
+    let t = [l.at(i0, j0), l.at(i0 + 1, j0), l.at(i0, j0 + 1), l.at(i0 + 1, j0 + 1)];
+    let mut out = [0.0f32; 4];
+    for ch in 0..4 {
+        let f = |k: usize| t[k][ch] as f32;
+        out[ch] = match tpu {
+            /* the program's: mix(mix(t00, t10, a), mix(t01, t11, a), b) over
+             * 0..255, as the reduction computes a deeper level */
+            Tpu::Alu => {
+                let mix = |x: f32, y: f32, w: f32| x * (1.0 - w) + y * w;
+                mix(mix(f(0), f(1), a), mix(f(2), f(3), a), b) / 255.0
+            }
+            Tpu::Gl | Tpu::W8 => {
+                let g = |k: usize| f(k) / 255.0;
+                (1.0 - a) * (1.0 - b) * g(0) + a * (1.0 - b) * g(1) + (1.0 - a) * b * g(2) + a * b * g(3)
+            }
+        };
+    }
+    out
+}
+
+/// Today's level below `l`: a kick drawing `w` x `h` with the u_scale the
+/// backend computes (`record_mip_level`).
+fn next(l: &Level, w: u32, h: u32, tpu: Tpu) -> Level {
+    let s = [1.0 / w as f32, 1.0 / h as f32];
+    let mut px = Vec::with_capacity((w * h) as usize);
+    for y in 0..h {
+        for x in 0..w {
+            let c = [(x as f32 + 0.5) * s[0], (y as f32 + 0.5) * s[1]];
+            px.push(sample(l, c, tpu).map(q));
+        }
+    }
+    Level { w, h, px }
+}
+
+struct Src<'a> {
+    l: &'a Level,
+    tpu: Tpu,
+    lookups: u64,
+}
+
+impl TexelSource for Src<'_> {
+    fn sample(&mut self, q: &TexLookup) -> Result<[u32; 4], Fault> {
+        if q.sampler != 0 || q.dim != TexDim::D2 {
+            return Err(Fault::new(format_args!("a look-up the tail program does not make")));
+        }
+        self.lookups += 1;
+        let c = [f32::from_bits(q.coord[0]), f32::from_bits(q.coord[1])];
+        Ok(eval::f32x4(sample(self.l, c, self.tpu)))
+    }
+    fn size(&mut self, _: u8, _: TexDim, _: i32) -> Result<[u32; 4], Fault> {
+        Ok([self.l.w, self.l.h, 1, 0])
+    }
+}
+
+#[derive(Default)]
+struct Outs {
+    o: [[u32; 4]; 2],
+}
+
+impl Sink for Outs {
+    fn store(&mut self, slot: IoSlot, value: [u32; 4], _: [bool; 4]) {
+        match slot {
+            IoSlot::FragColor(k) if k < 2 => self.o[k as usize] = value,
+            _ => {}
+        }
+    }
+}
+
+/// The level sides of a `w` x `h` chain (GL), `n` levels.
+fn sides(w: u32, h: u32, n: usize) -> Vec<(u32, u32)> {
+    (0..n).map(|l| ((w >> l).max(1), (h >> l).max(1))).collect()
+}
+
+/// The physical pitch and rows of level `l` of a `w` x `h` chain
+/// (src/pvr/backend.rs `mip_chain_layout`: power-of-two sides, max 1).
+fn phys(w: u32, h: u32, l: usize) -> (u32, u32) {
+    ((w.next_power_of_two() >> l).max(1), (h.next_power_of_two() >> l).max(1))
+}
+
+fn level0(w: u32, h: u32, kind: &str) -> Level {
+    let mut s: u32 = 0x1234_5678;
+    let mut rnd = || {
+        s ^= s << 13;
+        s ^= s >> 17;
+        s ^= s << 5;
+        s
+    };
+    let mut px = Vec::with_capacity((w * h) as usize);
+    for y in 0..h {
+        for x in 0..w {
+            let r = rnd().to_le_bytes();
+            px.push(match kind {
+                "random" => r,
+                /* a smooth picture with a little noise, like a render */
+                _ => {
+                    let g = |a: u32, b: u32, n: u8| (((a * 255) / b.max(1)) as u8).wrapping_add(n % 7);
+                    [g(x, w, r[0]), g(y, h, r[1]), g(x + y, w + h, r[2]), 255]
+                }
+            });
+        }
+    }
+    Level { w, h, px }
+}
+
+/// One chain: today's levels, then the tail kick's surface run through the
+/// evaluator, then the comparison of every tail level.
+fn chain(w: u32, h: u32, n: usize, s: usize, sf_pitch: u32, sf_rows: u32, kind: &str, tpu: Tpu) -> Vec<String> {
+    let sz = sides(w, h, n);
+    let mut today = vec![level0(w, h, kind)];
+    for l in 1..n {
+        let lv = next(&today[l - 1], sz[l].0, sz[l].1, tpu);
+        today.push(lv);
+    }
+    /* the block, as record_mip_tail writes it */
+    let tail: Vec<usize> = (s + 1..n).collect();
+    let mut offs = Vec::new();
+    let mut texels = 0u32;
+    for &l in &tail {
+        offs.push(texels);
+        let (p, r) = phys(w, h, l);
+        texels += p * r;
+    }
+    let last = texels % 2 != 0;
+    assert_eq!(sf_pitch * sf_rows + u32::from(last), texels, "the surface covers the range");
+    let mut block = vec![0u32; 4 * (2 * 5 + 1)];
+    for (k, &l) in tail.iter().enumerate() {
+        let (lw, lh) = sz[l];
+        let (p, _) = phys(w, h, l);
+        let f = [1.0 / lw as f32, 1.0 / lh as f32, lw as f32, lh as f32];
+        let g = [offs[k] as f32, p as f32, 1.0 / p as f32, 0.0];
+        for c in 0..4 {
+            block[4 * k + c] = f[c].to_bits();
+            block[4 * (5 + k) + c] = g[c].to_bits();
+        }
+    }
+    let sfv = [sf_pitch as f32, if last { 1.0 } else { 0.0 }, tail.len() as f32, 0.0];
+    for c in 0..4 {
+        block[40 + c] = sfv[c].to_bits();
+    }
+
+    let mut buf = vec![0u8; 8 << 20];
+    let a = Arena::new(&mut buf);
+    let lowered = compile_to_ir(&a, Stage::Fragment, SRC).unwrap_or_else(|e| panic!("{}: {}", e.line, e.msg));
+    let m = &lowered.module;
+    for (name, off) in [("u_lv", 0u16), ("u_ly", 20), ("u_sf", 40)] {
+        let d = m.uniforms.iter().find(|u| u.name == name).expect("the block's uniform");
+        assert_eq!(d.dword_off, off, "{name}: where record_mip_tail writes it");
+    }
+    let mut values = vec![[0u32; 4]; m.instrs.len()];
+    let mut def = vec![[false; 4]; m.instrs.len()];
+    let mut locals = vec![[0u32; 4]; m.locals.len()];
+    let mut ldef = vec![[false; 4]; m.locals.len()];
+    let mut src = Src { l: &today[s], tpu, lookups: 0 };
+    let mut surface = vec![[0u8; 4]; texels as usize];
+    for y in 0..sf_rows {
+        for x in 0..sf_pitch {
+            let io = [(IoSlot::FragCoord, eval::f32x4([x as f32 + 0.5, y as f32 + 0.5, 0.5, 1.0]))];
+            let mut out = Outs::default();
+            let frame = Frame { values: &mut values, def: &mut def, locals: &mut locals, ldef: &mut ldef };
+            eval::run_in_with_textures(m, &Inputs { io: &io, uniforms: &block }, frame, &mut out, &mut src)
+                .unwrap_or_else(|f| panic!("pixel ({x}, {y}): {}", f.as_str()));
+            let px = out.o.map(|o| o.map(|b| q(f32::from_bits(b))));
+            surface[(y * sf_pitch + x) as usize] = px[0];
+            if last && x == 0 && y == 0 {
+                surface[texels as usize - 1] = px[1];
+            }
+        }
+    }
+    let mut lines = vec![format!(
+        "{w}x{h} {kind:6} tpu {tpu:?}: tail levels {}..{} from level {s}, surface {sf_pitch}x{sf_rows}{}, {} look-ups",
+        s + 1,
+        n - 1,
+        if last { " + the last texel" } else { "" },
+        src.lookups
+    )];
+    for (k, &l) in tail.iter().enumerate() {
+        let (lw, lh) = sz[l];
+        let (p, r) = phys(w, h, l);
+        let (mut texd, mut byted, mut maxd, mut padnz) = (0, 0, 0u8, 0);
+        for yy in 0..r {
+            for xx in 0..p {
+                let got = surface[(offs[k] + yy * p + xx) as usize];
+                if xx >= lw || yy >= lh {
+                    padnz += usize::from(got != [0; 4]);
+                    continue;
+                }
+                let want = today[l].px[(yy * lw + xx) as usize];
+                let d = (0..4).map(|c| got[c].abs_diff(want[c])).collect::<Vec<_>>();
+                if d.iter().any(|&x| x != 0) {
+                    texd += 1;
+                }
+                byted += d.iter().filter(|&&x| x != 0).count();
+                maxd = maxd.max(*d.iter().max().unwrap());
+            }
+        }
+        lines.push(format!(
+            "  level {l} {lw}x{lh}: {texd} of {} texels differ, {byted} bytes, max |d| {maxd}; padding not zero: {padnz}",
+            lw * lh
+        ));
+    }
+    lines
+}
+
+#[test]
+fn the_tail_kick_against_the_chain_at_terrains_sizes() {
+    /* src/pvr/backend.rs mip_tail_plan at terrain's sizes: 800x600 (10
+     * levels) draws levels 5..9 from 4 as 44x31; 256x256 (9 levels) levels
+     * 4..8 from 3 as 20x17 and the last texel */
+    let mut alu_diffs = 0;
+    for kind in ["random", "smooth"] {
+        for tpu in [Tpu::Alu, Tpu::Gl, Tpu::W8] {
+            for (w, h, n, s, p, r) in [(800, 600, 10, 4, 44, 31), (256, 256, 9, 3, 20, 17)] {
+                let lines = chain(w, h, n, s, p, r, kind, tpu);
+                for l in &lines {
+                    println!("{l}");
+                }
+                if tpu == Tpu::Alu {
+                    alu_diffs += lines.iter().filter(|l| l.contains("texels differ") && !l.contains(": 0 of")).count();
+                    alu_diffs +=
+                        lines.iter().filter(|l| !l.ends_with("padding not zero: 0") && l.contains("padding")).count();
+                }
+            }
+        }
+    }
+    assert_eq!(alu_diffs, 0, "the program's logic: identical to the chain when the TPU's sample is its ALU's");
+}
```
