<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: glmark2 terrain's TFU refusals and CPU copies on the Pi 4, by reason

Study, no code change. This note covers the per-reason detail behind
#1209 and `pi4-slow-terrain.md`. Read at main `4370107f`; `src/v3d`,
`src/gl/texture.rs`, `src/gl/framebuffer.rs` and `src/gl/draw.rs` are the
same at `8e81ea9e`, the main tip when this was written. Line numbers below
are that tree's. The counts come from the
board record of main `c4af4917` (`pi4-glmark2-cpu.md`, "On the board:
glmark2 from main `c4af4917`"; record `records/pi4/20261022-0308-glmark2-c4af4917/`).
They are the difference of two printed counter lines, from the line before
terrain to shadow's frame-1 line. That window is terrain's 62 frames plus shadow's
set-up and first frame, and whatever the scenes before terrain added after their
last line. The per-frame figures are the note's own division.
Nothing here was run on the board. The mechanism below is checked against
the code, against a model of `RtImage`'s table (the last section) that
reproduces every per-frame count of the record, and against one host run
("Host run" below).

| counter (arena line) | over the window | by terrain's frame 1 | a frame after |
|---|---:|---:|---:|
| TFU chains (`TFU mipmap ... chains`) | 184 | 1 | 3 |
| TFU refusal `unknown` | 135 | 13 | 2 |
| tiled copy `plain` | 24 | 24 | 0 |
| tiled copy `first-sample` | 201 | 18 | 3 |
| `image-store` | 245 | | about 4 |

## The finding: a stale "rendered into" mark overfills the render-image table

The host made **5 TFU chains a frame** in terrain (`pi4-glmark2-cpu.md`,
host before/after table: "the TFU made every chain (5 a frame in terrain)").
That run was terrain alone, `GM2_ENTRIES='^(terrain|refract)$'`. On the
board, where terrain runs after desktop, buffer, ideas and jellyfish, it
makes **3** and refuses 2 as `unknown`. The 3 `first-sample` copies a frame
come from the same cause:

1. The GL core keeps "the kick that last rendered into texture `ti`" by
   texture **slot**, in `TARGET_USE` (`src/gl/framebuffer.rs:89-96`; written
   by `set_target_use` `:203`, read by `texture_target_use` `:269`). The
   draw copies it into each binding's `target_kick` (`src/gl/draw.rs:3951-3952`,
   `:4015`; the comment says "the render-into stamp is kept by texture NAME
   (its slot)").
2. Deleting a texture does not clear it. `glDeleteTextures`
   (`src/gl/texture.rs:4958`) -> `free_texture` (`:5009-5019`) releases
   the storage and zeroes the record. `TARGET_USE[ctx][h]` is left as it
   was. It is reset only in unit tests (`framebuffer.rs:3762`) and moved by
   a rename (`:1387-1393`). `glGenTextures` hands out the lowest free slot
   (`src/gl/names.rs:23`).
3. Terrain loads its three 512x512 JPEG textures (`terrain-grasslight-512`,
   `terrain-backgrounddetailed6`, `terrain-grasslight-512-nm`;
   glmark2 `scene-terrain/terrain-renderer.cpp`, `init_textures`) into slots
   that an earlier scene's FBO textures freed. They are `GL_RGB`, which the
   GL core stores as RGBA8. Their bindings therefore carry a non-zero
   `target_kick` from a render of the earlier scene.
4. The V3D takes a binding for a render image when `rt_texture` holds
   (stamped 2D RGBA8 at the target pitch, `src/v3d/backend.rs:1788-1799`)
   and `target_kick != 0` (`rt_eligible`, `:1804-1806`). So those uploads
   are registered as render targets on their first sample (`rt_sample`
   -> `rt_register`, `:1905-1907`).
5. The table has `RT_IMAGES` = 8 slots (`backend.rs:286`). Terrain's real
   targets already take 7: height map, normal map, specular map, terrain,
   bloom H, bloom V, tilt-shift H. With two stale uploads that is 9 for 8 slots.
   `rt_register` evicts the least recently used slot (`:1858-1867`),
   every frame. A target evicted between its render and its
   `glGenerateMipmap` is refused `unknown` and filtered on the CPU. A target
   evicted before it is sampled is registered again and tiled on the CPU
   (`first-sample`).

The model of that table (last section) gives, with 2 of the 3 uploads
marked, a steady state of **3 chains, 2 `unknown`, 3 `first-sample`
copies made and 4 image stores a frame**, the record's figures. With 0 or 1
upload marked it gives 5 chains, 0, 0 and 7, the host's figures. With all 3
marked it gives 2, 3, 4 and 3, which is not the record. The record's GPU
side agrees too: frame 10 ran 8 render jobs and **4** TFU jobs, which are the 3
chains plus the TFU tile of the normal map below.

The refused targets are bloom H (256x256) and tilt-shift H (800x600). The
copies are made of the normal map, bloom H and tilt-shift H. Whether the
stale marks sit on the board's slots is what the first confirmation below
checks. Which scene left them depends on the image's scene list.

## Per reason: decision point, the terrain resource, bytes a frame, the cost

Terrain's resources (glmark2 `22c527cb`, `scene-terrain.cpp`
`init_renderers`, bloom and tilt-shift on by default). Each FBO texture is
RGBA8, created by `BaseRenderer::create_texture` (`glTexImage2D` with no
data). "Mip" means `GL_LINEAR_MIPMAP_LINEAR` with `glGenerateMipmap` after
each render (`BaseRenderer::update_mipmap`):

| resource | size | levels | rendered | sampled by |
|---|---|---:|---|---|
| height map | 256x256 | 1 | every frame | normal map, terrain (VS) |
| normal map | 256x256 | 1 | every frame | terrain |
| specular map | 512x512, mip | 10 | once, at set-up | terrain |
| diffuse 1, 2, detail (uploads) | 512x512, mip | 10 | never | terrain |
| terrain | 800x600 + D16, mip | 10 | twice (terrain, bloom overlay) | bloom H, tilt-shift H |
| bloom H, bloom V | 256x256, mip | 9 | every frame | bloom V, overlay |
| tilt-shift H | 800x600, mip | 10 | every frame | tilt-shift V (window) |

`glGenerateMipmap` runs 5 times a frame: terrain twice, bloom H, bloom V and
tilt-shift H. Bytes below are the GL core's linear storage at the pair-padded
pitch: a 256x256 chain is 349,524 B, its levels 1 and up 87,380 B; an
800x600 chain is about 2.56 MB, level 0 1,920,000 B, levels 1 and up about
0.64 MB. The GL storage, the tiled copies and the images are all
non-cacheable on the Pi 4.

### TFU refusal `unknown` (2 a frame; 13 at set-up)

- **Decision:** `generate_mipmap`, `src/v3d/backend.rs:2343`
  (`rt_find(...).ok_or(NotFound)`). `MipRefusal::NotFound` is printed as
  `unknown` (`:2272-2275`). It means "no render image registered for this
  texture", nothing else.
- **Not a hardware limit.** The TFU job is never built. Every TFU check
  (`Layout`, `Pack`, `NoQuery`, `TfuFailed`, `:2365-2422`) comes after it, and
  the same 256x256 and 800x600 chains are made by the TFU in the frames where
  the slot survives (5 a frame on the host).
- **Terrain:** a frame: bloom H and tilt-shift H, evicted between their render
  and their `glGenerateMipmap`. At set-up, 13: `setup_texture` calls
  `update_mipmap` on 5 FBO textures before any render, the 3 JPEG loads
  call `glGenerateMipmap` (glmark2 `texture.cpp:94-95`), the specular map
  is rendered once and mipmapped, and the first frame asks before the
  first sample of 4 more. Those 13 happen once.
- **What it costs, a frame.** `v3d_generate_mipmap` answers 0, and the
  GL core (`src/gl/texture.rs:4280-4296`) goes on to:
  - `settle_texture` (`:900`, `:4298`). The texture's render kick is pending
    (`texture_target_use`), so `backend::flush()` and `backend::finish()`
    run (`:915-920`). That is **a CPU wait for all the GPU work, twice a
    frame**.
  - the CPU box filter (`:4342-4429`), byte by byte through a `&mut [u8]`,
    with a `cache_invd` and a `cache_flush` of the whole allocation around
    it. Bloom H reads 349,520 B and writes 87,380 B. Tilt-shift H reads
    about 2.56 MB and writes about 0.64 MB. That is **about 2.9 M one-byte
    non-cacheable loads and 0.73 M one-byte stores a frame**, plus
    `stamp_texels`.
- **Avoidance:** design 1 (below) removes the per-frame cases. Design 3
  covers the 13 at set-up.

### Tiled copy `first-sample` (3 made a frame; 18 by frame 1)

- **Decision:** `rt_sample`, `src/v3d/backend.rs:1905-1907`: `rt_find`
  misses, so the texture is registered (`rt_register` `:1856`, evicting at
  `:1861-1866`) and `copy_why = FirstSample`. The copy is made in
  `copy_image_for` (`:4278`) and counted only when a fresh fill ran
  (`made`, `:4328-4332`). A kept copy of the same stamp is reused uncounted,
  so the stale uploads and the specular map cost nothing here after their
  first frame.
- **Terrain, a frame:** the normal map, bloom H and tilt-shift H. Each was
  rendered since its last copy (`begin_job` drops the kept copies of the memory
  it renders into, `:2571-2573`), so each re-registration fills a new copy:
  - **normal map:** 256x256, one level, 262,144 B. That is 64 KiB or more and a
    single level, so `tfu_tile` (`:4010-4053`, rule `:4017`) gives it to the
    TFU through the **synchronous** `OSAL_QUERY_V3D_TFU_RUN`. The OSAL first
    finishes the render in flight, then the CPU spins on the TFU
    (`v3d-tfu-order.md`, section 1). No CPU bytes; one CPU wait and one TFU job.
  - **bloom H:** 9 levels, so `tfu_tile` refuses (`layout.levels != 1`) and
    `texture::tile` (`:4324`) tiles it on the CPU. It reads 349,524 B and
    writes a UIF/UBLINEAR/LT image of about the same size. The UIF walker
    covers levels 0 to 3 (`v3d-tiling-path.md`).
  - **tilt-shift H:** 10 levels, CPU tile. It reads about 2.56 MB and writes about 2.6
    MB or more (layout padding, `texture::layout`). The walker covers level 0
    only (75 % of the bytes). Mesa's utile loop does the rest.
  - Before each fill, `copy_image_for` waits for the render of the texture
    when it is still pending (`:4298-4306`). Bloom H and tilt-shift H
    are sampled by the job right after their own, so this is **up to 3 more
    CPU waits a frame** (`tex_waits`).
  - Eviction itself: `rt_free(…, true)` -> `rt_free_image` (`:1822-1826`)
    untiles a stale level 0 (`rt_level0_back`, `:2069-2102`, with a
    `wait_in_flight`) and a chain (`rt_chain_back`) into the storage. In the
    model about 1 evicted slot a frame holds a stored image: the height map,
    262,144 B untiled. The record's `level0-back` and `chain-back` counters
    show it.
- **In all, a frame:** about **2.9 MB read and 2.9 MB written by the CPU
  tiler**, one synchronous TFU job and up to 4 CPU waits. A healthy table
  has 0 of each, because all three are sampled from their render images
  (`rt_sample` `:1938-1949`).
- **Avoidance:** design 1. Design 2 makes it robust.

### Tiled copy `plain` (24, all by frame 1)

- **Decision:** `rt_sample`, `backend.rs:1899`. A binding that is not
  `rt_eligible` (`:1900`) keeps `Plain` (or `R32f`). Its copy is counted when it
  is made in `copy_image_for`.
- **Terrain:** the uploads that are not marked (at least the third JPEG
  texture, 512x512 x 10 levels, 1,398,100 B of storage, CPU-tiled once).
  Some of the 24 may belong to the scenes before terrain in the
  counter window. A kept copy is reused in later frames: **0 a frame**.
- **Avoidance:** none needed for the frame time. Uploading straight into the
  TMU layout (`eager_tile`, counted `upload`) is the set-up's business.

### `image-store` (about 4 a frame)

- **Decision:** `flush_job`, `backend.rs:2945-2958`, once per submitted job
  whose target has a render image (`uif_address`, set by `begin_job` from
  `rt_for_render`, `:2576`, `:2641-2651`).
- **This is not a CPU copy.** It is the TLB storing RT 0 into the render
  image in the TMU's layout, on the GPU. Since lazy raster (`361f4cbf`,
  `uif_only`, `:2650`) it replaces the raster store; it is not added to it
  (`v3d-lazy-raster.md`). It costs no CPU time and no bytes beyond the
  raster store it replaces.
- **Terrain:** in the churned table, about 4 a frame: the height map, terrain
  twice (render and overlay) and bloom V. That is about 4.4 MB of image
  writes by the TLB. The 3 targets re-registered after their render have no
  image in that job, so they store raster instead (the same TLB bytes) and are
  tiled again by the CPU (above). Healthy: 7 a frame, one per render job
  into a sampled target, about 6.8 MB, with no raster store left.
- **Avoidance:** none. A level nobody reads is not stored: every image
  terrain stores is sampled in the same frame. The count rises to 7 with design 1;
  that is correct.

## Avoidance designs, ranked

1. **Clear the render-into mark when a texture slot is freed.** In
   `free_texture` (`src/gl/texture.rs:5009`), or in a `framebuffer`
   helper it calls, set `TARGET_USE[ctx][h]` to 0 and reset
   `DEPTH_TARGET[ctx][h]`, as the rename path does for the old name
   (`framebuffer.rs:1387-1398`). The orphan path (`glDeleteTextures`
   `:4979-4982`) keeps the mark: the texture still lives in a framebuffer.
   - Files: `src/gl/texture.rs` (plan #310 owns it) and `src/gl/framebuffer.rs`
     (in plan #325's list).
   - Effect: the uploads are not `rt_eligible` again, terrain needs 7 of the 8
     slots, and the board should match the host. That is 5 chains, 0
     `unknown`, 0 `first-sample` and 7 `image-store` a frame, and no CPU filter,
     tiler bytes or the waits above. The same stale mark also makes
     `settle_texture` wait for an unrelated kick (`texture.rs:916`) and the
     GC7000's tiled copy refresh for one (`draw.rs:4047-4049`). It does that
     on every backend, with no wrong picture, only waits.
   - Risk to pictures: low. The mark only selects a path (render image
     or tiled copy) and a wait. A texture newly made in the slot has no render
     to wait for.
   - Risk to ES 2.0 answers: none expected. No GL state or error depends on
     the mark; `tests/es2_report_guard.rs` is the check.
   - Host test: a new file under `tests/`. Render into texture A, delete it,
     upload texture B (same slot, `glGenTextures`), draw with B, and expect
     `target_kick == 0` in the backend's binding. Then a V3D stand-in
     sequence shaped like terrain after an FBO scene, expecting 5 chains a
     frame (`v3d_mip_counters`, `v3d_copy_counters`).
   - Host repro before the fix: not with run.sh as it is, which runs one
     process per list entry ("Host run"). It needs a GL sequence in one
     process, an FBO scene's textures deleted and then terrain's set-up and
     frames, on the V3D stand-in. That is the host test above with the
     terrain-shaped frame. Expect the 3/2/3 pattern before the fix and 5
     chains after.
   - Board record value that confirms it: on the arena lines, terrain's
     `first-sample` and `unknown` stop growing after frame 1, `TFU mipmap
     ... chains` grows by 5 a frame, and frame 10 has 5 TFU jobs. Terrain's
     `gl ms` (156.06 at `c4af4917`) falls by the part of it these copies
     were.
2. **Make the render-image table robust to more sampled targets**
   (`src/v3d/backend.rs`, plan #308).
   - Either `RT_IMAGES` 8 -> 16 (`:286`): a slot is metadata, and its
     image is allocated only when a job renders into it (`rt_for_render`), so
     `rt_bytes_peak` does not move for a scene that fits.
   - Or `rt_register` prefers to evict a slot with no image and no render since
     its registration (the specular map, a stale upload) over one a job stores
     every frame.
   - Or `rt_eligible` requires a render newer than the texture's upload: the
     binding's `target_kick` against the kick of its `content_gen`.
   - **Done:** the first option, `RT_IMAGES` = 16. `rt_bytes_peak` is
     unchanged for a scene that fits (images are made in `rt_for_render`); the
     host test is `nine_sampled_targets_keep_their_render_images`
     (`src/v3d/backend_tests.rs`). Board record values are not measured.
   - Risk: low. A missed registration falls back to today's tiled copy.
   - Test: `tests/v3d_copy_reasons.rs` style, 9 sampled targets in one
     frame, 0 `first-sample` after the first frame.
   - Without design 1 this alone hides the stale mark rather than fixing it.
3. **Register a rendered target at `glGenerateMipmap`**
   (`generate_mipmap`, `backend.rs:2343`). For a `rt_texture` binding the GL
   core knows was rendered into (it would have to fill `target_kick` in
   `draw::texture_bind_2d`, `src/gl/draw.rs:690-697`, today 0), register
   it with its levels. The next render makes the image and later calls get the
   TFU.
   - It saves the CPU filters of terrain's first frame, about 4 of the 13. The 5
     `setup_texture` calls before any render and the 3 JPEG uploads would
     need a TFU chain from the raster level 0. Whether the TFU takes a raster
     source with NUMMM > 0 is in Mesa's `v3dx_tfu.c`, which is not vendored
     (`MESA_V3D_TREE` was not available here).
   - Set-up time only; **no frame-time gain**. Last.

The per-frame CPU work above (about 2.9 M one-byte non-cacheable loads in
the filter, about 5.8 MB through the tiler, up to 6 CPU waits on the GPU) is
the largest CPU item found for terrain's unexplained 130 ms a frame. How
many of those milliseconds it accounts for is a board measurement, design
1's before and after. The host's malloc'd memory cannot show it.

## Host run

`tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/x v3d` at
`8e81ea9e` plus this note: the whole list, 3 frames a scene, with the
TFU on (run.sh's Pi 4 default). The run writes `FINDINGS-v3d.md`, which was
restored afterwards. The harness runs **one process per list entry**
(run.sh, "one process per benchmark-list entry"), so terrain runs after
no other scene. It is the "0 uploads marked" case and cannot show the
board's churn. Terrain's frame-1 lines (`logs/v3d/22.log`; the counters are
printed at frame 1 only, since no arena grew later):

```
TFU mipmap at frame 1: 1 chains (0 queued behind a job, 0 waited), refused: ... unknown 13, ...
tiled copies for draws at frame 1, by reason: plain 3, r32f 0, first-sample 7, cpu-write 0, ...
other copies at frame 1: upload 0, arena 0, stamp-replaced 0, stale-given-back 0, image-store 1, level0-back 0, chain-back 0
```

This is the set-up accounting above: 13 `unknown` before the first sample,
the 3 JPEG uploads as `plain` (not eligible: no render mark), and the 7
real targets registered once (`first-sample` 7). With every upload unmarked
the 7 targets fit in 8 slots. On the board the window's `plain` 24 and
`first-sample` 18 also hold the scenes before terrain, so they cannot be
split per scene from the record. The board's per-frame 3/2/3 pattern
needs terrain to run after an FBO scene in one process. That is the
board's image, or a harness mode that runs several entries in one process,
which run.sh does not have.

## The model

The model is a script outside the tree. It replays one terrain frame on
`RtImage`'s bookkeeping: `rt_for_render` touches a found slot, `rt_sample`
registers on a miss and evicts the least recently used slot that the open job
does not store into, and `generate_mipmap` refuses on a miss. A copy is
counted when a re-registered texture was rendered since its kept copy. The
frame, in glmark2's order:

```
render height; render normal, sample height; render terrain, sample
diffuse1 diffuse2 detail normal specular height; mip terrain; render bloom_h,
sample terrain; mip bloom_h; render bloom_v, sample bloom_h; mip bloom_v;
render terrain (overlay), sample bloom_v; mip terrain; render tilt_h,
sample terrain; mip tilt_h; render window, sample tilt_h
```

The diffuse textures count as eligible as marked. The specular map is
rendered once before the first frame. Steady state, a frame:

| uploads marked | chains | `unknown` | `first-sample` made | `image-store` | slots evicted |
|---:|---:|---:|---:|---:|---:|
| 0 or 1 | 5 | 0 | 0 | 7 | 0 |
| 2 | 3 | 2 (bloom H, tilt-shift H) | 3 (normal, bloom H, tilt-shift H) | 4 | 6 |
| 3 | 2 | 3 | 4 | 3 | 8 |

The result is the same with the vertex shader's height map sampled before or
after the fragment shader's units, and for any choice of the 2 marked uploads.
The model leaves out the set-up and the kept-copy budget
(`CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES`, 24 MiB in `rpi_4b.conf`).
