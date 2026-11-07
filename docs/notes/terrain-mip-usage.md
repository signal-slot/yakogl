<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2 terrain: which mipmap chains it rebuilds each frame, and which levels it samples

Study, no code change (plan #329, item `terrain-mip-usage`; #1209 for the
Pi 4, #1199 for the AM62P). Written 2026-11-07. `src/`, `driver/` and
`docs/notes/` are the same at the main tip `e3b6f1e9` and at the tree this was
read in (`d83e25b5`): `git diff --stat d83e25b5 e3b6f1e9` lists only
`tests/gc_glmark2_mip_offer.rs` and `tools/cts/pi4-gles3-preflight*`,
`tools/cts/images-selftest.sh`. Every line number below is that tree's.
glmark2 is `22c527cb0556f3a1ac4445aaa52cc532760928d5`, the pin of
`driver/samples/zephyr/glmark2/CMakeLists.txt`. Nothing here was run on a
board; the board figures are the cited records', and the savings are
predictions marked as such.

## Answer in four lines

1. Terrain calls `glGenerateMipmap` **5 times a frame** on 4 textures: the
   800x600 terrain target twice (after its own render, and again after the
   bloom overlay blended into it), bloom H and bloom V (256x256) and tilt-shift
   H (800x600). **All five have new input every frame** (the scroll offset
   moves with time); none is rebuilt from unchanged contents. The textures
   whose chains never change (3 JPEG uploads, the specular map) are built once.
2. All four are set `GL_LINEAR_MIPMAP_LINEAR`, but **only one chain is read
   below level 1**: the terrain target's first build, by bloom H, at LOD 1.64
   (levels 1 and 2). The other four are read at LOD 0 (1:1 or magnified) up to
   a derivative skew of the blur taps that lifts the LOD by 0.0001 to 0.019:
   at most level 1, with a weight of at most 1.9 %.
3. ES 2.0 lets a driver build the chain **lazily** (an implementation filter is
   free, and a texture's levels above 0 can be observed only by sampling), and
   to skip levels no sample reaches, with one condition: the derived arrays
   are the contents *at the call*. Lazy-at-first-sample alone saves nothing in
   terrain (every chain is sampled by a mipmap-filtered draw). Building only
   the reachable levels saves **38 of the 43 chain kicks** a frame on the
   AM62P (52 -> 14 kicks) and 1 to 2 of the 5 TFU jobs on the Pi 4, plus most
   of the CPU box-filter work of the two chains the Pi 4 board refuses.
4. yakogl builds every level eagerly at the call and gives the backend no
   sampler LOD bound (`src/gl/texture.rs:4141`, `src/backend.h:652`).

## The harness command and the trace

```sh
# 1. the trace patch ("The trace patch", last section) on shim.c, kept out of the tree
# 2. terrain only, V3D and PowerVR hosts, mipmap/texture/draw trace + kick ids:
GM2_WRITE=0 GM2_TRACE=1 GM2_KICKS=1 GM2_FRAMES=5 GM2_ENTRIES='^terrain$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm2-trace v3d powervr
grep -a 'gm2: t ' build/gm2-trace/logs/v3d/0.log
grep -a -E 'gm2: (t |draw)' build/gm2-trace/logs/powervr/0.log
# 3. counts without the patch (GM2_PROFILE; GM2_PERF=1 adds perf):
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 GM2_ENTRIES='^terrain$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm2-prof v3d powervr
git checkout -- tools/hostharness/glmark2/shim.c    # drop the patch
```

`GLMARK2_SRC` was `$CONNECTED_DEPS_DIR/glmark2-src` (the pinned commit). The
harness has no per-call trace: `GM2_PROFILE` counts calls and times them,
`GM2_KICKS` logs a line after each draw (PowerVR only). The patch adds
`GM2_TRACE=1`: a `gm2: t ...` line per `glGenerateMipmap`, `glTexImage2D`,
`glTexSubImage2D`, `glTexParameteri`, `glFramebufferTexture2D`, `glClear`,
`eglSwapBuffers`, and per draw the framebuffer, the viewport, the program and
the textures bound to units 0..5. It changes no behaviour with the variable
unset and was **not committed** (the contract of this item is the note only);
the item that wants it as a tool takes the patch below to
`tools/hostharness/glmark2/shim.c` and `README.md`.

`GM2_PROFILE=1` (20 frames, `profile-v3d.md`, `profile-powervr.md`):
`glGenerateMipmap` **5.0 calls a frame** on both hosts and 14 calls in the
set-up and first frame; `glDrawArrays` 8.0 a frame. The V3D host model runs
**5.0 TFU jobs a frame** (all five chains made by the TFU; the Pi 4 board
makes 3, `v3d-terrain-copies.md`). The PowerVR host's kick ids advance by
**52 a frame** (trace, steady frames): 43 are the chains (see the table) and 9
the scene's draws.

### One steady frame (the V3D host trace, texture ids as created)

```
clear fb 0                                    (window)
clear fb1 256x256; draw prog2  tex 0         height map      <- tex1 (LINEAR, 1 level)
clear fb2 256x256; draw prog4  tex 1         normal map      <- tex2 (LINEAR, 1 level)
clear fb4 800x600; draw prog8  393216 verts  terrain         <- 4,5,6 (diffuse/detail), 2 normal, 3 specular, 1 height
genmip tex7                                                   [1] terrain, after its render
clear fb5 256x256; draw prog10 tex 7         bloom H         <- samples tex7
genmip tex8                                                   [2] bloom H
clear fb6 256x256; draw prog12 tex 8         bloom V         <- samples tex8
genmip tex9                                                   [3] bloom V
draw fb4 800x600 prog14 tex 9 (blend)        overlay into tex7 <- samples tex9
genmip tex7                                                   [4] terrain again, after the overlay
clear fb7 800x600; draw prog16 tex 7         tilt-shift H    <- samples tex7
genmip tex10                                                  [5] tilt-shift H
draw fb0 800x600 prog18 tex 10               tilt-shift V    <- samples tex10, to the window
swap
```

Set-up (not per frame): `setup_texture` calls `BaseRenderer::update_mipmap`
on the five FBO textures before anything is rendered into them (trace lines
25..93: tex 3, 7, 8, 9, 10), the three JPEG uploads each call
`glGenerateMipmap` (glmark2 `src/texture.cpp:94-95`), and the specular map is
rendered once and mipmapped (`SceneTerrain::setup`, trace line 94-96).

## Terrain's source: what it asks for

`src/scene-terrain.cpp` `init_renderers` (bloom and tilt-shift are `true` by
default, `SceneTerrain::SceneTerrain`; `default-benchmarks.h:66` runs the
scene with its defaults): the offscreen targets are made by
`BaseRenderer::create_texture` (`src/scene-terrain/base-renderer.cpp`,
RGBA8, `glTexImage2D` with no data) and given `GL_LINEAR_MIPMAP_LINEAR` by
`setup_texture` for the specular map, the terrain target, bloom H, bloom V and
tilt-shift H. The height and normal maps keep `GL_LINEAR`: no chain, and
`update_mipmap` (`base-renderer.cpp:84`) is a no-op for them.

`TextureRenderer::render` (`texture-renderer.cpp:53`: bloom H, bloom V,
tilt-shift H) and `TerrainRenderer::render` (`terrain-renderer.cpp:43`) end in
`update_mipmap()`; `OverlayRenderer::render` (`overlay-renderer.cpp:72`) draws
into the *terrain* target and ends in `target_renderer_.update_mipmap()`. The
tilt-shift H input is `RendererChain::append` giving it
`bloom_chain.texture()`, which for the overlay is
`target_renderer_.texture()` (`renderer.h:280`): the terrain target, so
tilt-shift H does not read the bloom output through any other texture. The
shaders (`data/shaders/terrain-*.frag`) call `texture2D` only: no bias, no
`texture2DLod`, no `dFdx`, no extension. `terrain.vert` fetches the height and
normal maps in the vertex shader (level 0 by definition, and they have no
chain).

## The chains: per-chain table

Sizes and kicks: a chain below an `WxH` level 0 has `floor(log2(max(W,H)))`
levels above it. 800x600 -> 400x300 .. 1x1 = **9 levels above 0** (10 in all);
256x256 -> **8** (9 in all). The PowerVR draws one kick a level
(`mip_chain_gpu`, `src/pvr/backend.rs:6241`, the loop at `:6268`,
`record_mip_level`), the Pi 4's TFU one job a chain
(`v3d_generate_mipmap`, `src/v3d/backend.rs:2319`). The kick counts match the
host trace's kick ids (294 -> 304 = 9 chain kicks + the next draw's kick, and
so on): 9 + 8 + 8 + 9 + 9 = 43 chain kicks of the 52 a frame. The board
note puts a frame at 53 kicks (`am62p-pvr-mipmap-kicks.md`).

"Levels reachable" is the range of LOD (ES 2.0 3.7.7, rho = the larger of the
two screen-axis derivative lengths in texels, lambda = log2 rho) over the draw
that samples the chain, for the 800x600 window of every board's glmark2 run
(`GM2_SIZE`, `verdin_am62p_am62p54_a53.conf` "the default 800x600"). With a
trilinear filter lambda in (k, k+1) reads levels k and k+1.

| # | texture (glmark2 object) | size, levels above 0 | built | rebuilt every frame | input unchanged vs the last frame | sampled by (program, trace) | lambda reachable | levels reachable | chain kicks (PowerVR) / TFU job |
|---|---|---|---|---|---|---|---|---|---|
| 1 | tex7, terrain target, **first build** | 800x600, 9 | after the terrain draw | yes | **no** (`uOffset`, `uvOffset` follow time) | bloom H (prog10), 256x256 viewport, uv 0..1 | **1.64** (rho = max(800/256, 600/256) = 3.125) | **1, 2** | 9 / 1 |
| 4 | tex7, terrain target, **second build** | 800x600, 9 | after the overlay blend | yes | **no** (the overlay changed level 0) | tilt-shift H (prog16), 800x600 viewport | 0 to 0.00013 (1:1; the tap offset adds 2k/600 = 0.0133 texel/px at k = 4 to the y derivative) | 0 (1 with weight <= 0.013 %) | 9 / 1 |
| 2 | tex8, bloom H | 256x256, 8 | after the bloom H draw | yes | **no** | bloom V (prog12), 256x256 viewport | 0 to 0.0113 (1:1; v tap = y(1 + k/256), k <= 2) | 0, 1 (weight <= 1.1 %) | 8 / 1 |
| 3 | tex9, bloom V | 256x256, 8 | after the bloom V draw | yes | **no** | overlay (prog14), 800x600 viewport, uv 0..1 | **-1.64** (256 texels over 800 px: 0.32 texel/px) = magnification | **0 only** | 8 / 1 |
| 5 | tex10, tilt-shift H | 800x600, 9 | after the tilt-shift H draw | yes | **no** | tilt-shift V (prog18), 800x600, window | 0 to 0.0191 (1:1; v tap = y + 2k(y-1/2)/600, k <= 4: rho up to 1.0133) | 0, 1 (weight <= 1.9 %) | 9 / 1 |
| - | tex3 specular map | 512x512, 9 | once, `SceneTerrain::setup` | no | (once) | terrain (prog8) fragment, `uvOverlay` = uv x 6 on an oblique 6000-unit plane | 0 to ~9 (not bounded by the viewport) | all | once |
| - | tex4, 5, 6 diffuse 1, 2, detail (JPEG) | 512x512, 9 | once, at load | no | (once) | terrain (prog8) fragment | as above | all | once |
| - | tex1 height, tex2 normal | 256x256 | none (`GL_LINEAR`) | no chain | - | terrain vertex + fragment, normal-from-height | - | 0 | - |

Rows are in the order of the trace's `genmip` calls ([1] .. [5] above; row 4
is the call [4]). Worked values: bloom H samples with
`u + k*stepX`, stepX = (1/800) y, so its coordinate moves 800/256 = 3.125
texels per pixel in u and 600/256 = 2.34 in v (the tap term adds k/800 per
unit y over 256 pixels, below 1e-5). The tilt-shift taps use
`TextureStepX * |0.5 - y| / 0.5` (`create_blur_shaders`, `blur-renderer.cpp`,
tilt 0.5; bloom passes tilt 0.0): the offset is linear in y with slope
2k/size texel per pixel, which is the only thing that lifts lambda above 0.
The hardware LOD unit works from quad differences and a fixed-point LOD
fraction (a weight under 1/256 is rounded to 0, to be confirmed per GPU),
so the three "up to level 1" rows probably read level 0 alone; the numbers are
the spec's bound, not the TPU's or TMU's output.

At other sizes only the first row moves: bloom H reads lambda = log2(W/256)
for a W x H canvas (800x600 1.64, 1024x600 and 1024x768 2.0, 1280x720 and
1280x800 2.32, 1920x720 2.91): levels 2..3 at the panel sizes of
`stage2-bench-gap.md`. The 1:1 passes stay at lambda ~ 0 (tilt-shift V's skew
is 2k/H: 0.0133 at H = 600, 0.01 at H = 800). A fullscreen run is not the
default and is not a board record's size.

### (a) Rebuilt every frame, unchanged input

All five builds are every frame. None has unchanged input: the height map
scrolls with the wall clock (`SceneTerrain::update`:
`uvOffset = (diff * 0.05, 0)`, `uOffset = scale * diff * 0.05`), the terrain
target follows, and each later chain follows that. Two of the five rebuild a
chain *under* a texture whose level 0 was just rewritten by a later pass of
the same frame (rows 1 and 4 are one texture, the second build replacing the
first after the overlay changed level 0). There is no cache to hit.

### (b) Sampled with a mipmap min filter

All four textures are `GL_LINEAR_MIPMAP_LINEAR` at every draw that samples them
(trace: `texparam ... pname 0x2801 value 0x2703`). Of the five builds, one
(row 1) has a consumer that reads deeper than level 1; one (row 3) has a
consumer in magnification, so no chain level above 0 is read; three (rows 4,
2, 5) are read at 1:1 where only the tap skew reaches level 1.

### (c) Spec latitude, and what yakogl does

- **Lazy build.** ES 2.0.25 3.7.11 ("Mipmap Generation", read from memory, not
  re-checked against the text here) defines the derived levels as a filtered
  reduction of level 0 with no required filter ("a box filter is
  recommended"). In ES 2.0 the only way to observe a level above 0 is to
  sample it (an FBO attachment is level 0 only), so a driver may build the
  levels at the first draw that samples them, or build only those a draw can
  reach, if the result is the same as at the call. The condition: the derived
  arrays are the contents of level 0 *at the call*; a render into or an upload
  to level 0 after the call does not change them. A lazy build therefore has
  to run before the next write to level 0, or take a copy of level 0; or the
  driver accepts that an app which writes level 0 and then samples deeper
  levels *without calling glGenerateMipmap again* sees the new level 0's chain
  (terrain never does: every chain sampled is regenerated after its last
  write, see the trace). That is a deviation to gate (candidate 1 below),
  not a free move. ES 3.0 adds observers (`glFramebufferTexture2D` of a level
  above 0, `glReadPixels`, `glCopyTexSubImage2D`, `GL_TEXTURE_BASE_LEVEL`/
  `MAX_LEVEL`), which bring the chain back to full and eager
  (`level_above_zero_attached`, `src/gl/texture.rs:4276`, already refuses the
  backend's chain then).
- **What yakogl does.** `glGenerateMipmap` (`src/gl/texture.rs:4141`) validates
  and, for an RGBA8 2D chain, offers it to the backend at once
  (`:4268-4296`, `backend::generate_mipmap`, `src/backend.rs:2169`,
  `YakoglBackend::generate_mipmap`, `src/backend.h:652`, `src/backend.rs:1540`):
  `Whole` marks `chain_in_backend` and returns; `Prefix(n)` and `None` filter on
  the CPU in place (`:4342-4429`, after `settle_texture`, `:900`, which waits
  for the kicks using the texture). The backend is told the chain length and
  the base image only: **no sampler LOD range**, nothing about which draw will
  read it (`GlesTextureBind`, `src/backend.h`). The chain comes back to the
  CPU only when the CPU reads or writes it (`chain_back`, `:880`).
  - **PowerVR** (`pvr_generate_mipmap`, `src/pvr/backend.rs:6315`): one
    `record_mip_level` kick per level, all queued, nothing waited for
    (`mip_cpu_tail` off, `:1640`; path B is excluded here).
  - **V3D** (`v3d_generate_mipmap`, `src/v3d/backend.rs:2319`): one TFU job for
    a render image registered in the 8-slot table (`RT_IMAGES`); otherwise a
    refusal (`unknown`, 2 a frame on the board) and the GL core's box filter
    on the CPU after a `finish()`, `v3d-terrain-copies.md`.
  - **GC7000** (`gc_generate_mipmap`, `src/vivante/mipmap.rs:304`): the GPU
    path of #1106, out of this plan's boards.
- **Where the lazy build would sit.** The sample sites are the draw's texture
  binding (`texture_bind_2d`, `src/gl/draw.rs`) and the CPU access sites
  already funnelled through `settle_texture` / `chain_back`. Draw-time
  marking of a pending chain and a flush before the next write to level 0
  (the clear and the draw of a render pass into the texture) are the same two
  points that `flush_clear_into` (`src/gl/texture.rs:865`) and
  `settle_texture` use for the eager path today.

## Fix candidates, ranked

Savings are per frame, for the 800x600 default, against the 52 kick ids a
frame (43 chain kicks) of the PowerVR host trace and the 5-job V3D host
model (3 TFU jobs and 2 CPU box filters on the Pi 4 board). Time
predictions use the board's per-kick price and are **predictions, not
measured**: the AM62P frame is 83.06 ms for ~53 kicks (1.57 ms a kick,
`6a7021ec`, RESULTS.md Common set), and path B showed that a kick *waited
for* costs far more than a queued one (`am62p-pvr-mipmap-kicks.md`,
"Measured"), so a saving that adds a wait loses.

| rank | candidate | AM62P kicks a frame | Pi 4 TFU jobs / CPU filters a frame | risk |
|---|---|---|---|---|
| 1 | **Build only the reachable levels, at the first sampling draw** (a sampler LOD bound per draw, lazy chain, stale-chain rule gated by a backend flag like `mip_cpu_tail`) | 43 -> 5 chain kicks (**-38**: 52 -> 14) | TFU jobs 5 -> 4 (tex9 skipped); the two CPU filters of tex8 and tex10 each cut to level 1 (reads 349,524 -> 262,144 B and 87,380 -> 65,536 B written for tex8) | high: needs the LOD bound; output moves by <= 4.9 LSB at the extremes (1.9 % x 255) |
| 1b | the same, dropping a level whose weight rounds to 0 (lambda < 1/256: row 4) | -39 (52 -> 13) | -2 jobs (tex9, tex7 second build) | as 1, plus a per-GPU rounding rule |
| 2 | **Skip the chain of a texture only ever magnified** (row 3, tex9) | **-8** | **-1 job** (tex9 is one of the 3 the board's TFU makes) | medium: exact (lambda < 0 proven), but still needs the per-draw bound |
| 3 | **Lazy at first sample, drop on overwrite** (no LOD bound) | **0** in terrain (every chain is sampled by a mipmap-filtered draw) | 0 | low; helps only the set-up's 14 builds, 5 on never-rendered textures; do not do it for terrain's sake |
| 4 | Level-limited from history ("the last K draws that sampled this texture reached at most level L") | like 1, unproven | like 1 | unsound: a draw that reaches deeper later reads unbuilt levels. Rejected |

### Candidate 1, the bound and the order of work

The LOD bound is the hard part. A draw's fetch coordinate is an arbitrary
shader expression; the fixed cases in terrain are:

- a **screen-aligned quad** (`terrain-texture.vert`: `vUv = uvScale *
  (position.xy * 0.5 + 0.5) + uvOffset`, `gl_Position = vec4(position, 1)`),
  which is bloom H/V, overlay and tilt-shift H/V. The derivative is
  `uvScale * size_texels / viewport_pixels`, exact for the whole draw, plus
  the blur taps' slope term; and
- the terrain draw (a 256x256 grid under a perspective projection): an
  unbounded bound, so every level is built. It samples only the once-built
  textures, so nothing per frame depends on it.

A compiler pass (`src/shaderc`) classifying "texture2D coordinate = affine in
a varying whose vertex output is an affine function of an attribute and
uniforms, `gl_Position` an attribute pass-through" and the draw-time
evaluation of the quad's vertex data would give the bound; anything it does
not prove keeps the full chain. That puts the work in `src/shaderc`,
`src/gl/draw.rs` and `src/gl/texture.rs`, which #325, #310 and #308 are
changing, so it is ordered after their items release them (the plan's
rule). Row 2 of the table needs the same pass but only the sign of lambda
(`rho < 1`), the smallest first step.

### The board record value that confirms each

- **Candidate 1 / 1b, AM62P:** the kicks a frame, from the `kicks a frame`
  line of gl_test case 216 `mipchains` (terrain's four chains a frame, 20
  frames, glFinish each; 40 at `61ad38a1`, 15 with path B on) and, for the
  scene, the delta of `pvr_backend_counters` kicks over the scene's frames
  (52 to 53 a frame now, 14 predicted). Then the scene's ms a frame, the
  Common set row of `tools/cts/RESULTS.md` (**terrain 83.06 ms at
  `6a7021ec`**; ~1.6 ms a kick predicts about 28 to 30 ms if the 38 kicks
  are fully additive) and the picture against `tools/cts/glmark2-pictures.txt`
  column `am62p` within `tolerance 8` (the levels moved by candidate 1 change
  the pixels by at most 4.9 per channel; terrain's mean there is 120 129 44).
  `pvr_backend_mip_chains` stays at 5 a frame (calls made); it is the kicks
  that fall.
- **Candidate 1 / 1b, Pi 4:** the printed line `yakogl/v3d: TFU mipmap at
  frame N: C chains (...) refused: ...` (`src/v3d/backend.rs:4913`): C grows
  3 a frame now (the record `records/pi4/20261022-0308-glmark2-c4af4917/`),
  predicted **2** (tex9 skipped) or **1** (1b); `refused: unknown` 2 a frame
  now; the CPU filter bytes are in the `v3d-terrain-copies.md` model (2.9 M
  one-byte loads a frame), cut by a quarter for the two refused chains. The
  scene's ms a frame (**terrain 161.79 ms at `c4af4917`**) will move by the
  CPU-filter and wait share only: the frame's 130 ms of unexplained CPU time
  is #1209's other items, and `pi4-slow-terrain.md` (the breakdown item)
  decides how much of it these chains are.
- **Candidate 2:** AM62P kicks a frame 52 -> 44; Pi 4 `chains` 3 -> 2.
- **Candidate 3:** none per frame; the set-up's `genmip` count would fall
  from 14 to 9 (host `GM2_PROFILE` `glGenerateMipmap` calls in "Setup and
  first frame", 14 now); not a board value.

### What this study does not decide

- Whether the hardware's LOD for the 1:1 passes is exactly 0 on the TPU and
  the TMU (then rows 4, 2 and 5 need no level above 0 at all and the kicks
  fall to 2 + 0 = **2**, i.e. **-41**). A GL test that samples a 1:1 quad
  with a second chain whose level 1 differs from level 0 by a known amount
  would settle it on each GPU.
- The sampled-area bound for other scenes (refract's 1600x1200 target is
  mipmapped once a frame, `pi4-glmark2-cpu.md`; shadow has none).
- The CPU tail (path B) and the transfer queue (path A): excluded by the
  item. This note changes how many chains/levels are built, not the cost of
  a kick.

## The trace patch

Apply to `tools/hostharness/glmark2/shim.c` for the `GM2_TRACE` runs above;
`git checkout -- tools/hostharness/glmark2/shim.c` drops it. It was run at
`d83e25b5` and the trace in this note is its output.

```diff
@@ static void log_kick(void)
+/* ---- GM2_TRACE=1 (terrain-mip-usage study): the texture and mipmap calls -- */
+
+static int tr_on(void)
+{
+	static int on = -1;
+
+	if (on < 0) {
+		const char *e = getenv("GM2_TRACE");
+		on = e && !strcmp(e, "1");
+	}
+	return on;
+}
+static GLuint tr_unit, tr_tex[8], tr_fb, tr_prog, tr_vw, tr_vh;
+static void w_glActiveTexture(GLenum t) { tr_unit = (t - GL_TEXTURE0) & 7; glActiveTexture(t); }
+static void w_glBindTexture(GLenum t, GLuint id) { if (t == GL_TEXTURE_2D) tr_tex[tr_unit] = id; glBindTexture(t, id); }
+static void w_glUseProgram(GLuint p) { tr_prog = p; glUseProgram(p); }
+static void w_glViewport(GLint x, GLint y, GLsizei w, GLsizei h) { tr_vw = w; tr_vh = h; glViewport(x, y, w, h); }
+static void w_glBindFramebuffer(GLenum t, GLuint fb)
+{
+	tr_fb = fb;
+	glBindFramebuffer(t, fb);
+}
+static void w_glFramebufferTexture2D(GLenum t, GLenum a, GLenum tt, GLuint tex, GLint lvl)
+{
+	if (tr_on()) fprintf(stderr, "gm2: t fb-attach fb %u tex %u level %d\n", tr_fb, tex, lvl);
+	glFramebufferTexture2D(t, a, tt, tex, lvl);
+}
+static void w_glTexImage2D(GLenum t, GLint lvl, GLint ifmt, GLsizei w, GLsizei h, GLint b, GLenum f, GLenum ty, const void *p)
+{
+	if (tr_on()) fprintf(stderr, "gm2: t teximage tex %u level %d %dx%d ifmt 0x%x fmt 0x%x type 0x%x data %d\n",
+			     tr_tex[tr_unit], lvl, (int)w, (int)h, (unsigned)ifmt, f, ty, p != NULL);
+	glTexImage2D(t, lvl, ifmt, w, h, b, f, ty, p);
+}
+static void w_glTexSubImage2D(GLenum t, GLint lvl, GLint x, GLint y, GLsizei w, GLsizei h, GLenum f, GLenum ty, const void *p)
+{
+	if (tr_on()) fprintf(stderr, "gm2: t texsubimage tex %u level %d %dx%d\n", tr_tex[tr_unit], lvl, (int)w, (int)h);
+	glTexSubImage2D(t, lvl, x, y, w, h, f, ty, p);
+}
+static void w_glTexParameteri(GLenum t, GLenum pn, GLint v)
+{
+	if (tr_on()) fprintf(stderr, "gm2: t texparam tex %u pname 0x%x value 0x%x\n", tr_tex[tr_unit], pn, v);
+	glTexParameteri(t, pn, v);
+}
+static void w_glGenerateMipmap(GLenum t)
+{
+	if (tr_on()) fprintf(stderr, "gm2: t genmip tex %u (fb %u)\n", tr_tex[tr_unit], tr_fb);
+	glGenerateMipmap(t);
+}
+static void w_glClear(GLbitfield m)
+{
+	if (tr_on()) fprintf(stderr, "gm2: t clear mask 0x%x fb %u vp %ux%u\n", m, tr_fb, tr_vw, tr_vh);
+	glClear(m);
+}
+static EGLBoolean w_eglSwapBuffers(EGLDisplay d, EGLSurface sf)
+{
+	if (tr_on()) fprintf(stderr, "gm2: t swap\n");
+	return eglSwapBuffers(d, sf);
+}
+static void tr_draw(const char *what, GLsizei count)
+{
+	if (!tr_on())
+		return;
+	fprintf(stderr, "gm2: t %s fb %u vp %ux%u prog %u count %d tex", what, tr_fb, tr_vw, tr_vh, tr_prog, (int)count);
+	for (int u = 0; u < 6; u++)
+		fprintf(stderr, " %u", tr_tex[u]);
+	fprintf(stderr, "\n");
+}
@@ static void w_glDrawArrays(GLenum mode, GLint first, GLsizei count)
 	n_draws++;
+	tr_draw("draw", count);
 	glDrawArrays(mode, first, count);
@@ static void w_glDrawElements(...)
 	n_draws++;
+	tr_draw("draw", count);
 	glDrawElements(mode, count, type, indices);
@@ static const struct { const char *name; void *fn; } W[] = {  (before glDrawArrays)
+	{ "glActiveTexture", (void *)w_glActiveTexture },
+	{ "glBindTexture", (void *)w_glBindTexture },
+	{ "glUseProgram", (void *)w_glUseProgram },
+	{ "glViewport", (void *)w_glViewport },
+	{ "glBindFramebuffer", (void *)w_glBindFramebuffer },
+	{ "glFramebufferTexture2D", (void *)w_glFramebufferTexture2D },
+	{ "glTexImage2D", (void *)w_glTexImage2D },
+	{ "glTexSubImage2D", (void *)w_glTexSubImage2D },
+	{ "glTexParameteri", (void *)w_glTexParameteri },
+	{ "glGenerateMipmap", (void *)w_glGenerateMipmap },
+	{ "glClear", (void *)w_glClear },
+	{ "eglSwapBuffers", (void *)w_eglSwapBuffers },
```

(The hunk markers are abbreviated; the added lines are exactly the 83 of
the patch that was run. Insert the block after `log_kick`, the two
`tr_draw` lines before the real draw call in each draw wrapper, and the
entries in `W[]`.)
