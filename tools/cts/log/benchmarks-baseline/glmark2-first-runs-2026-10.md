<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2-es2 on Zephyr: the port's host checks and first board runs

Status: finished record, moved here from `driver/docs/glmark2.md` (its sections 3, 5, 5b, 5c, 5d and 6) on 2026-10-11. The first runs of 2026-10-03 and 10-04 on the AM62P and the Pi 4. The port is `driver/docs/glmark2.md`; current results are `tools/cts/STATUS.md` and the baselines next to this file. Superseded since: section 5's "the depth-only framebuffer is always UNSUPPORTED" (`src/gl/framebuffer.rs` makes it complete where the back end has depth-only targets), and the per-heap free lists it cites in `driver/src/pvr_render_pool.h` (now `driver/rust/src/gpu/pvr/pool.rs`).

## 3. Host checks (2026-10-03, `bench/glmark2` 5f2926b7 on main 5398fb14)

- The AM62P image builds from a pristine build directory with no error and no
  warning of ours: `zephyr.bin` 12,755,176 B, sha256
  `20f845dd914a8bc26ab3f2fd2b49f05ce737374721f74c3606b1c3207a3e78f9`
  (Zephyr af17c0c8aa2, SDK 1.0.1). The one CMake warning, "No SOURCES given to
  Zephyr library: drivers__display", is Zephyr's own; the slint and triangle
  AM62P builds print it too.
- It contains every scene's constructor (34 `Scene*::Scene*(Canvas&)`
  symbols), both image readers, the data-file table and the board layer.
- `RUN_BOUND=960 BOARD_HOST=<placeholder> tools/board.sh --dry-run am62p run <image>`:
  rc 0. The image checks pass (header, footprint, watchdog marker).
- The patched `util.cc` passed a host test (g++, `-D__ZEPHYR__`, a fake
  table): reads, `seekg` (cur, end, out of range), a missing file gives a
  failed stream, `list_files` with and without a trailing slash, NaN process
  times. The first version failed the trailing-slash case; the test found it.
- The i.MX8MP image also builds and links from a pristine build directory:
  `zephyr.bin` 12,256,088 B, sha256
  `c28033657cf95804dc40f07817ee83037ee5bd55bc4c0820a788f048e00aafae`.
- `cargo fmt --check`: clean (the change has no Rust).

## 5. First board run: AM62P, 2026-10-03 22:26 (image 20f845dd, U-Boot netboot)

Read from the run directory `build/glmark2-am62p-20261003-222609/20261003-2226-glmark2-20f845dd/console.txt`
of the main checkout (2500 lines; the runner was the lead's).

- `glmark2 Score: 260`; "20 scenes ran, 11 failed, 2 unsupported (33 list
  entries); window 800x600, shown yes, GPU resets 0"; `glmark2: FAIL rc=-5`;
  record `594b0201 00000002 00000000 00000000` (APP fail: the image predates
  kind GLMARK2).
- No vsync cap: pulsar 460, texture 438/434/436, build 408 fps, with
  `board_show` flipping every frame ("shown yes").
- The 2 unsupported are `buffer:...update-method=map` (no GL_OES_mapbuffer).
- **9 of the 11 failures have one cause (fixed in 92741d34): the online
  preprocessor did not predefine `GL_FRAGMENT_PRECISION_HIGH`** (`src/shaderc/online/pp.rs:53`
  predefines `__LINE__ __FILE__ __VERSION__ GL_ES` only), while
  `glGetShaderPrecisionFormat` reports fp32 for fragment `GL_HIGH_FLOAT`
  (`src/gl/shader.rs:516-523`). GLSL ES 1.00 §4.5.4 (revision 17, extracted
  text line 1751): "The built-in macro GL_FRAGMENT_PRECISION_HIGH is defined to
  one on systems supporting highp precision in the fragment language". Mesa
  defines it for every GLES shader (`glcpp-parse.y:2540-2541`).
  - jellyfish: its fragment shader asks for highp only under the macro
    (`libmatrix/shader-source.cc:442-452`), so it got mediump against the
    vertex default highp, and the link refused `uCurrentTime`. The refusal
    itself is right: §4.3.4 (line 1487) "types and precisions of uniforms with
    the same name must match", Issue 10.3 "RESOLUTION: Yes"; Mesa's linker
    refuses too when both stages use the uniform (`gl_nir_linker.c:1938-1949`,
    Mesa main 8fc4981d2d25). Our message cites "section 4.5.3" (Default
    Precision Qualifiers), which is the wrong section: it is §4.3.4.
  - conditionals (3), function (2), loop (3): the scene splices a second,
    untyped ShaderSource into the fragment source (`scene-conditionals.cpp:79-90`),
    so `HIGHP_OR_DEFAULT` is defined twice (`shader-source.cc:493-500`): empty in
    the fragment block without the macro, `highp` in the other. A redefinition
    with another body is refused, as in C++. With the macro both are `highp`.
- **Item for the PowerVR driver: the render pool runs dry in glmark2 and never recovers.**
  1. Pool: 57.8 MiB (64 MiB carve-out - 4 MiB - 192 KiB firmware heap - 2 MiB page tables, `driver/src/pvr_internal.h:57-67`).
  2. First exhaustion: terrain's set-up (entry 22, after 20 scenes ran): "3072 KiB requested, 2748 KiB left", so the bump stood at ~55 MiB.
  3. After it, 0 KiB for the rest of the run: 808 exhaustion lines, 268 "no pool memory for the ... HWRT", 268 submit failures (-12), 179 dropped texture updates, 445 incomplete framebuffers.
  4. Peaks seen: terrain asks 3,072 KiB (a buffer), 1,368 KiB (512x512 mip chain), 2 x 1,876 KiB (800x600); shadow and refract each ask 7,500 KiB (a 1600x1200 texture).
  5. Unknown: whether the 20 earlier scenes' objects went back to the per-heap free lists (`driver/src/pvr_render_pool.h`) or leaked; the pool prints no high-water or free-list summary, which the item includes.
  6. Not the sample's: it allocates no GPU memory; glmark2 deletes its objects at each scene's teardown.
- **Item for the GL core and every backend: a framebuffer with only a depth
  attachment is refused as GL_FRAMEBUFFER_UNSUPPORTED** (`src/gl/framebuffer.rs:600-605`:
  "Every kick renders into a colour buffer; a depth/stencil-only framebuffer has
  nothing for the PBE to write"). glmark2's shadow scene asks for exactly that
  (a 1600x1200 depth texture, no colour; `scene-shadow.cpp:107-114`), as does any
  shadow-map pass.
  1. Legal: ES 2.0.25 section 4.4.5 lets an implementation refuse "an
     implementation-dependent set of restrictions" on the combination of attached
     formats (extracted text lines 5606-5609).
  2. Per GPU: every backend already draws with colour writes off into a target
     (the PowerVR's `color_write_disable`, ISPCTL tagwritedisable,
     `src/pvr/ffi.rs:161`; the GL core's no-draw-buffer draw, test at
     `src/gl/draw.rs:6060-6067`, "on every backend"). A target with no colour
     buffer at all never reaches a backend (the status refuses it first), so on
     the PowerVR, the GC7000 and the V3D it is unmeasured.
  3. What it costs the suite: nothing in dEQP-GLES2.functional.fbo.completeness.
     dEQP passes a GL_FRAMEBUFFER_UNSUPPORTED status
     (`modules/glshared/glsFboCompletenessTests.cpp:576-584`, `pass()`), and
     the group is 370 / 370 on the AM62P (a3856a06) and the i.MX8MP (f83a7361)
     (`tools/cts/RESULTS.md:111,120`). On the Pi 4 its 27 stencil-texture cases
     were NotSupported for another reason, the depth-texture claim the V3D
     withheld until v3d/depth-texture (`tools/cts/log/status-2026-09-25-to-10-07.md:99`). The cost is in applications: glmark2's
     shadow, and shadow maps in general.
  4. The direction: a depth-only framebuffer renders with an internal colour
     target and colour writes off, which every backend handles today; a
     gl_test case with depth read-back lands with it.

## 5b. Pi 4 run, 2026-10-03 23:09 (image ddd58c27 = b973c4d0, tryboot)

Read from `build/glmark2-pi4-20261003-230914/20261003-2309-glmark2-ddd58c27/console.txt`
of the main checkout (720 lines; the runner was the lead's).

- `glmark2 Score: 201` (189 before the macro fix); "29 scenes ran, 0 failed,
  4 unsupported (33 list entries); window 800x600, shown yes, GPU resets 0";
  record `594b1201 000000c9 021d0004 03200258` (GLMARK2: score 201, status 2,
  ran 29, failed 0, unsupported 4, 800x600). The 4 unsupported: buffer map x2
  (no GL_OES_mapbuffer), shadow and refract ("We do not have the depth texture
  extension!!!").

1. **The floor is `board_show`'s copy, measured.** In every GPU-light scene the
   frame is the copy:
   ```text
   glmark2-frame: [build] use-vbo=true: 2764 frames; us per frame: gl 19, swap 16, show 3582, other 2, frame 3619
   glmark2-frame: [texture] texture-filter=nearest: 2690 frames; us per frame: gl 28, swap 25, show 3665, other 0, frame 3718
   glmark2-frame: [shading] shading=phong: 2262 frames; us per frame: gl 47, swap 38, show 4336, other 1, frame 4422
   glmark2-frame: [conditionals] fragment-steps=0:vertex-steps=0: 2300 frames; us per frame: gl 41, swap 28, show 4279, other 1, frame 4349
   glmark2-frame: [jellyfish] <default>: 2094 frames; us per frame: gl 404, swap 28, show 4344, other 2, frame 4778
   ```
   `show` is 3,577-4,603 us in all 29 scenes; it is 91-99% of the frame in the 23
   scenes whose GL time is under 0.5 ms. `board_show` on the Pi 4 copies the
   800x600 window (1,920,000 B) row by row from a Normal non-cacheable window to
   the Normal non-cacheable firmware framebuffer (`driver/zephyr/lib/board/rpi4/board_rpi4.c:195-198`;
   mappings `:155-156`, `rpi_mbox.c:193`): about 0.42-0.54 GB/s. `swap` is
   16-60 us there: the V3D finishes the frame's job while the CPU copies (a swap
   submits and returns, `src/v3d/backend.rs:4231-4236`), so the GPU is not
   on the critical path. The fix is the board layer's scan-out view
   (`CONFIG_YAKOGL_BOARD_SCANOUT_VIEW`, branch unify/stride-view): a window
   smaller than the panel is a view into the framebuffer at its stride, with
   no copy. With it the copy floor is gone (5d). Without it, the Pi 4's
   GPU-light scenes measure that copy, not the V3D.
2. **The V3D job times: two instrument defects, the second fixed only in part.**
   - First (fixed in fc4f96f6): the line printed "prep 13, bin 0, render 0,
     total 0" in every scene. The OSAL finishes job N inside job N+1's submit
     and then zeroes the fields for N+1, which writes only its prep, so a read
     after eglSwapBuffers always saw N+1's prep and four zeros. The OSAL now
     keeps a copy when a job completes (`osal_v3d_last_completed_job_us`).
   - Second: with that copy, the run of a8a6d591 (d3cdefd0;
     `build/glmark2-pi4-20261003-232340/.../console.txt` of the main checkout)
     printed numbers that cannot be GPU time:
     ```text
     glmark2-frame: [texture] texture-filter=nearest: 2690 frames; ... show 3665, other 2, frame 3719; v3d last completed job per frame: prep 9, bin 0, render 3664, total 3709 (jobs 2690 over 2690 frames, ...)
     glmark2-frame: [conditionals] fragment-steps=0:vertex-steps=0: 2300 frames; ... show 4278, other 1, frame 4348; v3d last completed job per frame: prep 13, bin 0, render 4240, total 4318 (...)
     glmark2-frame: [conditionals] fragment-steps=5:vertex-steps=0: 2299 frames; ... show 4279, other 1, frame 4349; v3d last completed job per frame: prep 13, bin 0, render 4255, total 4332 (...)
     glmark2-frame: [desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4: 106 frames; ... v3d last completed job per frame: prep 13, bin 22, render 918, total 960 (jobs 1178 over 106 frames, ...)
     ```
     In the one-job scenes "render" tracks "show" within 0.1-0.3 ms and does
     not move with the fragment cost; a heavier, multi-job scene reads a
     shorter render.
   - Why, from the code (`src/osal/zephyr_v3d.c`): the render's START is
     stamped in the ISR, at the FLDONE interrupt, just before
     `v3d_submit_render` (`v3d_isr`, the FLDONE branch). Its END has no
     interrupt: the ISR takes only FLDONE and OUTOMEM (`V3D_BIN_IRQS`), and the
     end stamp is taken by the thread in `finish_inflight`, after `v3d_wait`
     polls FRDONE. `finish_inflight` runs at the next submit, which in a
     one-job scene is the next frame's eglSwapBuffers, after `board_show`'s
     copy. So "render" there is the render's start to the end of the copy: the
     copy, not the GPU. In desktop and terrain the next submit comes at the
     next target switch inside the frame, so most ends are seen promptly, and
     ~0.9 ms per blur pass is plausible (the known case).
   - "bin 0": bin was written only in `bin_finish`, which runs only when the
     thread finds the binner still busy; in a one-job scene binning ended long
     before the next submit, so the field kept the 0 the submit wrote: not
     measured, printed as 0.
   - The fix on this branch (not run on the board yet): bin is now computed
     always, from the bin's submit stamp to the FLDONE interrupt's; and the
     OSAL records `render_late` (`zephyr_v3d.h`), 1 when FRDONE was already up
     at the OSAL's first look. The glmark2 line takes render and total only from
     jobs whose end the OSAL waited for, and prints "render absent, total
     absent (the end seen late in all N)" when there were none. Prediction for
     the next Pi 4 run: one-job scenes print render absent (the copy outlasts
     the GPU job every frame) and a real bin of tens of us; desktop keeps
     render ~0.9 ms "end waited for" in most frames.
   - How to read the line: "v3d last completed job per frame" is, for each
     frame in which a job completed, the job that completed LAST before the
     read after eglSwapBuffers. Since a swap submits its frame's job and returns
     with it running, that is normally the previous frame's job (or the last of
     several when a frame has more than one). prep and bin are means over those
     frames; render and total over the frames whose job end was waited for ("end
     waited for in E of F"). "absent" means no job completed in the scene's
     window, or none had a waited-for end; a field the OSAL did not measure is
     never printed as 0.
   - The planned measurement, done (5d): the scan-out view removes the copy,
     and the run with it reads the V3D's own render time under the
     `render_late` rule above, with no second instrument. The FRDONE interrupt
     was not needed.
3. **terrain: ten jobs a frame and INVALID_OPERATION from glGenerateMipmap** (an
   item, no fix without a gl_test case):
   `glmark2-frame: [terrain] <default>: 57 frames; us per frame: gl 169902, swap 3069, show 4263, other 2, frame 177236; ... (jobs 572, frames with more than one 57)`.
   - The jobs: terrain is a chain of render-to-texture passes (height map,
     normal map, specular, the terrain at 800x600, bloom h/v at 256x256, the
     overlay, tilt h/v at 800x600; `scene-terrain.cpp:62-120`). Each target
     change submits a job (`src/v3d/backend.rs:2843-2844`), so ~10 per frame.
   - The GL time (170 ms): a draw that samples a texture an earlier job
     rendered waits for that job and has the texture re-tiled into the TMU
     layout by the CPU (or the TFU) from the GL core's storage
     (`src/v3d/backend.rs:2835-2931`), every frame, for 800x600 targets.
     desktop shows the same shape (`gl 89742` per frame, 1178 jobs in 106
     frames).
   - glGenerateMipmap: `BaseRenderer::update_mipmap` (`scene-terrain/base-renderer.cpp:84-90`)
     asks a chain for the 800x600 off-screen textures, which use
     `GL_LINEAR_MIPMAP_LINEAR`. ES 2.0.25 section 3.7.11 (extracted text line
     4119): "If either the width or height of the level zero array are not a
     power or two, the error INVALID_OPERATION is generated." YakoGL reported
     ES 2.0 without GL_OES_texture_npot and applied exactly that, so the error
     was right for what it advertised, and those textures were
     mipmap-incomplete for sampling. Mesa's v3d reports ES 3.1, where the rule
     is gone. Since 2026-10-07 (`2673197d`) every ES 2.0 build claims
     GL_OES_texture_npot: the call succeeds and the texture is complete. The
     V3D and the GC7000 hold the whole chain (box-filtered on the CPU after
     the frame's render into level 0, every frame); the PowerVR holds it too,
     laid out at the power-of-two extent as Mesa's pvr driver does (`cc788590`;
     `src/gl/texture.rs` `npot_mip_chains`, `pot_chain_layout`).
4. **`program 2 has no uniform 'ModelViewMatrix'` (gouraud, blinn-phong-inf) is
   correct**: neither shader declares it (`data/shaders/light-basic.vert:5-6` and
   `light-advanced.vert:4-5` declare only `ModelViewProjectionMatrix` and
   `NormalMatrix`), and glmark2 sets it for every shading mode
   (`scene-shading.cpp:302`), so the location is -1. It is not an optimisation
   and not a link-table gap.

## 5c. AM62P run, 2026-10-03 23:09 (image 9d4f63c8 = b973c4d0, U-Boot netboot)

Read from `build/glmark2-am62p-20261003-230937/20261003-2309-glmark2-9d4f63c8/console.raw`
of the main checkout (864 lines once CRs are stripped; the runner was the lead's).

- `glmark2 Score: 303` (260 before the macro fix); "29 scenes ran, 2 failed, 2
  unsupported (33 list entries); window 800x600, shown yes, GPU resets 0";
  record `594b1201 0000012f 021d0202 03200258` (score 303, status 2, ran 29,
  failed 2, unsupported 2). Against TI's published 306.4 (SDK 12.01,
  "Glmark2-DRM", size and command not stated, section 4): ours had 2 failed and
  2 unsupported scenes, TI's states none; the two numbers are not equal
  measurements, and TI's size is unknown.
- **The frame is the swap.** `show` is 3-5 us (a DSS flip), `gl` 25-116 us in
  the GPU-bound scenes:
  ```text
  glmark2-frame: [conditionals] fragment-steps=0:vertex-steps=0: 4879 frames; us per frame: gl 27, swap 2017, show 3, other 2, frame 2049; gpu job times: absent
  glmark2-frame: [loop] fragment-loop=false:fragment-steps=5:vertex-steps=5: 4879 frames; us per frame: gl 28, swap 2016, show 3, other 2, frame 2049; gpu job times: absent
  glmark2-frame: [effect2d] kernel=0,1,0;1,-4,1;0,1,0;: 1420 frames; us per frame: gl 25, swap 7016, show 3, other 1, frame 7045; gpu job times: absent
  glmark2-frame: [effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;: 915 frames; us per frame: gl 26, swap 10910, show 3, other 1, frame 10940; gpu job times: absent
  ```
  Six scenes of different fragment cost (conditionals with 0 steps, function
  low, loop without a fragment loop, ...) sit at exactly 2,016-2,017 us: a floor
  that is not the GPU's. The PowerVR fence wait polls the FRAG UFO once per
  millisecond, with `busy_wait_us(1000)` between polls
  (`driver/rust/src/gpu/pvr/render.rs:1734-1827`), so a swap waits whole
  milliseconds: 2,016 and 7,016 us are that granularity. Scenes heavier than
  the floor (effect2d, cel, jellyfish at 7,072) are GPU-bound above it. The
  kick's GEOM+FRAG time is not measured on the AM62P (the line prints "gpu job
  times: absent"), so GPU time against poll time cannot be split from this run;
  an item for the PowerVR driver: a wait on the fence interrupt, or a finer
  poll, and a kick time the OSAL keeps as the V3D's does.
- **terrain** hits the render pool (section 5's item): "render pool exhausted:
  4608 KiB requested, 2736 KiB left" x3 and 3072 KiB (glBufferData, `GL error
  0x505`), then 1876 KiB x2 (800x600 textures, `glTexImage2D` 0x505), and its
  framebuffers 4 and 7 are incomplete (0x8cd6) every frame after; it still runs
  (62 frames, `gl 162807`).
- **shadow: `glCheckFramebufferStatus failed (0x8cdd)`, a real unsupported
  combination of ours, not the pool.** Its framebuffer has only a depth texture
  (1600x1200, `scene-shadow.cpp:107-114`), and `check_status` returns
  GL_FRAMEBUFFER_UNSUPPORTED for any framebuffer without a colour attachment
  ("Every kick renders into a colour buffer; a depth/stencil-only framebuffer has
  nothing for the PBE to write", `src/gl/framebuffer.rs:600-605`). A depth
  texture the pool had refused would have given 0x8cd6 first (its storage is
  checked earlier, `framebuffer.rs:363-398`), and no exhaustion line precedes
  shadow's. ES 2.0.25 section 4.4.5 lets an implementation refuse "an
  implementation-dependent set of restrictions" on "the combination of internal
  formats of the attached images" as FRAMEBUFFER_UNSUPPORTED (extracted text
  lines 5606-5609), so the answer is legal; whether the PowerVR backend could
  render with no colour target is not measured here. An item for the GL core
  and the PowerVR backend: a depth-only target (glmark2's shadow, a common
  shadow-map pass).
- **refract: 0x8cd6 is the pool.** "render pool exhausted: 7500 KiB requested,
  356 KiB left" twice (its 1600x1200 depth and colour textures), then the
  incomplete attachment.
- **The verdict folded unsupported scenes into FAIL** (both boards printed
  `glmark2: FAIL rc=-5`; the Pi 4 had 0 failed). Fixed on this branch: a run
  passes when no scene failed and the score is not 0; the unsupported scenes
  stay their own count (record w2's low byte, `record.rs` kind 18), the sample
  prints how many, and `tools/board.sh` puts `unsupported=N` on its verdict
  line (`yakogl-tools record --line` has the field). The AM62P's run stays a
  FAIL: 2 failed (the pool item).

## 5d. Pi 4 pair, the scan-out view off and on, 2026-10-04 00:27 (images from 1a46b33c, tryboot)

Read from the two consoles under `build/sv175-pi4-gm2-20261004-002724/{off,on}/`
of the main checkout (the runner was the lead's; `RUN_BOUND=960`). One image per
arm, the same tree; only `CONFIG_YAKOGL_BOARD_SCANOUT_VIEW` differs.

| arm | image sha256 | window | Score | scenes | resets | record |
|---|---|---|---|---|---|---|
| view off | `c82a2c3329ee8be6...` | 800x600, stride 3200 (copied) | 201 | 29 ran, 0 failed, 4 unsupported | 0 | `594b1201 000000c9 011d0004 03200258` |
| view on | `19acce90c5968c59...` | 800x600, stride 5120 ("a view into the framebuffer, pitch 5120 (no copy)") | 842 | 29 ran, 0 failed, 4 unsupported | 0 | `594b1201 0000034a 011d0004 03200258` |

The off arm repeats 5b (Score 201; show 3,591-4,602 us). In the on arm `show`
is 0 in all 29 scenes, and the frame becomes the V3D's job: `swap` now waits
for the previous frame's job, so the render's end is waited for and measured.
The predictions written before the run (show about 0; the light scenes' frame
falls to the V3D time; render no longer absent; Score above 201) all held.

Per scene (the `glmark2-frame:` lines, generated from the two consoles; "render's
end waited for" is E of F frames, section 5b point 2):

| scene | off: show / frame us | on: show / frame us | on: bin / render us (render's end waited for) |
|---|---|---|---|
| `[build] use-vbo=false` | 3591 / 6379 | 0 / 2845 | 142 / 715 (1/3514) |
| `[build] use-vbo=true` | 3595 / 3631 | 0 / 620 | 133 / 471 (16117/16118) |
| `[texture] texture-filter=nearest` | 3679 / 3732 | 0 / 699 | 25 / 656 (14296/14298) |
| `[texture] texture-filter=linear` | 3674 / 3717 | 0 / 708 | 25 / 667 (14104/14105) |
| `[texture] texture-filter=mipmap` | 3644 / 3688 | 0 / 716 | 25 / 675 (13946/13947) |
| `[shading] shading=gouraud` | 3625 / 3671 | 0 / 762 | 197 / 548 (13109/13111) |
| `[shading] shading=blinn-phong-inf` | 4282 / 4353 | 0 / 1064 | 264 / 778 (9386/9387) |
| `[shading] shading=phong` | 4336 / 4420 | 0 / 1350 | 260 / 1065 (7401/7403) |
| `[shading] shading=cel` | 4336 / 4410 | 0 / 1495 | 263 / 1211 (6683/6684) |
| `[bump] bump-render=high-poly` | 4467 / 4572 | 0 / 1876 | 836 / 1006 (5323/5327) |
| `[bump] bump-render=normals` | 4316 / 4414 | 0 / 733 | 29 / 680 (13625/13626) |
| `[bump] bump-render=height` | 4315 / 4411 | 0 / 790 | 28 / 739 (12640/12642) |
| `[effect2d] kernel=0,1,0;1,-4,1;0,1,0;` | 4375 / 4452 | 0 / 2900 | 36 / 2839 (3446/3447) |
| `[effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | 4376 / 4455 | 0 / 4247 | 37 / 4184 (2354/2355) |
| `[pulsar] light=false:quads=5:texture=false` | 4276 / 4446 | 0 / 725 | 31 / 671 (13770/13772) |
| `[desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4` | 4597 / 94658 | 0 / 91278 | 23 / 917 (110/110) |
| `[desktop] effect=shadow:windows=4` | 4602 / 21586 | 0 / 18063 | 36 / 967 (554/554) |
| `[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata` | 4458 / 10060 | 0 / 6077 | 127 / 713 (1/1645) |
| `[ideas] speed=duration` | 4300 / 10065 | 0 / 5658 | 186 / 712 (1/1768) |
| `[jellyfish] <default>` | 4344 / 4777 | 0 / 2339 | 109 / 2207 (4117/4274) |
| `[terrain] <default>` | 4276 / 177227 | 0 / 164185 | 35 / 2610 (61/61) |
| `[conditionals] fragment-steps=0:vertex-steps=0` | 4279 / 4347 | 0 / 611 | 53 / 537 (16328/16328) |
| `[conditionals] fragment-steps=5:vertex-steps=0` | 4279 / 4348 | 0 / 1189 | 55 / 1112 (8403/8404) |
| `[conditionals] fragment-steps=0:vertex-steps=5` | 4277 / 4347 | 0 / 622 | 64 / 536 (16046/16047) |
| `[function] fragment-complexity=low:fragment-steps=5` | 4278 / 4347 | 0 / 864 | 52 / 790 (11555/11556) |
| `[function] fragment-complexity=medium:fragment-steps=5` | 4278 / 4347 | 0 / 1493 | 51 / 1421 (6689/6690) |
| `[loop] fragment-loop=false:fragment-steps=5:vertex-steps=5` | 4279 / 4350 | 0 / 905 | 87 / 796 (11034/11035) |
| `[loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5` | 4279 / 4350 | 0 / 905 | 87 / 796 (11034/11035) |
| `[loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5` | 4278 / 4350 | 0 / 1931 | 88 / 1822 (5173/5174) |

Reading it:
- The light scenes' frame is now the V3D's bin plus render plus 16-25 us:
  for example `use-vbo=true` 3,631 -> 620 us (bin 133, render 471),
  `texture nearest` 3,732 -> 699 (25, 656), `gouraud` 3,671 -> 762 (197, 548),
  `conditionals 0:0` 4,347 -> 611 (53, 537).
- Fragment cost now shows: `shading` gouraud 548, blinn-phong 778, phong 1,065,
  cel 1,211 us of render; `effect2d` 3x3 2,839 and 5x5 4,184; `loop` with a
  uniform bound 1,822 against 796 with a constant one.
- `[loop] fragment-loop=false` and `fragment-uniform=false` read exactly the same
  (11,034 frames, render 796). Not measured why; one explanation to check is that
  a loop with a constant bound compiles to the same program as no loop.
- `use-vbo=false`, `buffer` and `ideas` are CPU-bound (gl time dominates) and
  their render's end was waited for in 1 frame only: their render column is one
  frame, not a scene's measurement.
- `desktop` and `terrain` are multi-job scenes; their render (917-967, 2,610)
  is the same as with the copy, so the copy was never their floor.
- 842 is above the public Mesa figure of 697 in section 4. Not the same
  conditions: that run is Mesa on Linux in a window, its flags are not stated,
  and the score leaves out unsupported scenes (ours leaves out 4; section 4).

## 6. What only a board can settle

- Whether the image boots and the canvas comes up (EGL config choice,
  `eglCreateWindowSurface` on the board window) on each board.
- The run time, the heap high-water mark (96 MiB is a guess), the stack, and
  whether every scene sets up (shader compiles by the online compiler, the
  texture and FBO formats).
- The scores themselves, the GPU reset count, and whether `board_show` caps
  the frame rate at the panel's refresh.
- The kernel's virtual address space. Both Verdin images keep the default
  `CONFIG_KERNEL_VM_SIZE=0x10000000` (256 MiB). The image takes about 153 MB
  (AM62P) and 156 MB (i.MX8MP) of it, which leaves about 100 MiB for the
  64 MiB GPU carve-out map, the window's alias and the device blocks. dEQP
  on the i.MX8MP failed that map with 56 MiB left and moved to 512 MiB
  (`deqp/boards/verdin_imx8mp_mimx8ml8_a53.conf`). Not changed here, because
  nothing shows it is needed. A boot that panics in `k_mem_map_phys_bare` is
  this.
- The i.MX8MP has no UART: its numbers need the record of section 2 first.
