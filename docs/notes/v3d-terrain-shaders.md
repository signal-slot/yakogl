<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 4.2: glmark2 terrain's shaders, our counts against Mesa's, and a per-job render model

Plan #329, item v3d-terrain-shader-study (#1209; ROADMAP.md stage 2, "Slow
scenes"). `v3d-terrain-gpu.md` ("What was not done") says that no V3D
instruction count of terrain's shaders exists. This note gives those counts
and a model of each job's `render` built from them. The model is set against
the `glmark2-jobs` entries at frame 10 (`render` 7930 and 9352 us,
`pi4-slow-terrain.md`, "What is left (new)"). Host only. No code changed,
no board run. Code is read at `537da3a8`. glmark2 is the pinned `22c527cb`
(`$CONNECTED_DEPS_DIR/glmark2-src`).

## Answer in short

- **Our compiler's counts.** The two tilt-shift blurs compile to 78
  (horizontal) and 80 (vertical) instructions at **4 threads**. Each does 9
  TMU look-ups with a 16-bit return, 18 `ldtmu`, a per-pixel P1 and no
  spill. That is at Mesa's level: Mesa's 7.1 count is 64 and 66, and the
  estimate for Mesa 4.2 is 79 to 82. The blurs are not long. Their ALU time
  is about 2.4 ms each and their TMU time at most 4.0 ms each (model below).
- **The large program is the noise pass.** `terrain-noise.frag` compiles to
  **1495 instructions at 2 threads** (256x256, no texture). At 500 MHz and
  32 lane-instructions a clock that is **6.1 ms** of QPU issue, not the
  "about 0.5 ms" `v3d-terrain-gpu.md` assumed. The terrain draw's vertex
  shader is 172 instructions at 2 threads and its fragment shader 160 at 4.
  Their QPU issue is at least **8.7 ms**.
- **So the 7930 and 9352 us entries are most likely the noise job and the
  terrain draw, not the two blurs.** The `glmark2-jobs` line lists the jobs
  that frame 10's swap completed. Its first entry can be the previous
  frame's last job, tilt-shift V
  (`driver/samples/zephyr/glmark2/src/main.cpp:1401-1403`). Then entry 2 is
  noise and entry 4 is the terrain draw. If both entries were blurs, the
  frame's render would be at least 17.3 + 6.1 + 8.7 + 1.2 = **33.2 ms**.
  That is more than the 23.49 to 27.26 ms render sum measured, at any V3D
  clock below about 850 MHz. Under the noise-and-terrain reading the model
  sums to **24.0 to 25.3 ms** against the measured 23.49 (section 4).
- **Ranked fixes.** (1) Better add/mul pairing in the V3D scheduler for the
  noise shader. It has 872 add-side and 830 mul-side operations in 1495
  words, so the floor is about 872 to 900 words: about **-2.4 to -2.6 ms a
  frame**, with no picture risk. (2) The same pairing for the terrain draw's
  VS and FS: about **-2 ms**. (3) Nothing in the blurs' instruction count,
  layout or TMU set-up: about **0 ms**. Every fix is in
  `src/shaderc/online/v3d/**` (plan #308).
- **Not run here:** Mesa's 4.2 `v3d_shc` (it needs a Mesa tree, and
  unpacking the archive was refused in this session), the glmark2 host
  harness corpus (`run.sh` was refused) and any board run. Each Mesa 4.2
  count below is an estimate and is marked as one.

## 1. How it was measured

### Our compiler

The scratch crate `$CONNECTED_DEPS_DIR/scratch-v3d-terrain` sits outside
the tree. It depends on this checkout with
`default-features = false, features = ["host-test", "gpu-v3d",
"shaderc-online", "no-shader-table"]`, and is run with:

```sh
cargo run --release --manifest-path "$CONNECTED_DEPS_DIR/scratch-v3d-terrain/Cargo.toml" -- <out dir>
cargo run --release --manifest-path "$CONNECTED_DEPS_DIR/scratch-v3d-terrain/Cargo.toml" -- corpus > corpus.tsv
```

Per program it does what `tests/v3d_varyings_compile.rs:171-185` does:
`lower::compile_to_ir` for each stage, then
`v3d::chain::compile_program(vs, fs, Prim::Triangles, Target { vpm_size:
65536 }, ..)`. That is the link-time compile of `codegen::compile_program`
(`src/shaderc/online/codegen.rs:1015-1048`) with the default sampler key,
so no binding is `ret32` and every look-up returns 16 bits. It writes each
stage's listing through `v3d::listing::write_instr` (`<prog>.<fs|vs|cs>.qpu`
in the out dir) and counts the words, threads and signals of the listing:

- `words`: `Code::words.len()`, one 64-bit instruction each.
- `threads`: `FsProg::threads` or `VsProg::threads` (`chain.rs:289-301`: 4
  when the values fit 4 threads' registers, else 2).
- `ldtmu`: the TMU result words read. `retire`: the TMU look-ups (the
  `tmus`/`tmuslod`/`tmuscm`/`tmusf` writes). `wrtmuc`: the config uniforms.
  `thrsw`: thread switches. `ldunif`: uniform loads.
- `add`, `mul`: words whose add or mul half is not `nop`.
- Spills: none can exist. The allocator has no spilling and falls back to 2
  threads instead (`alloc.rs:38-40`).

The texts are rebuilt from glmark2's sources the way `ShaderSource` builds
them. They are not copied from a harness run, because `run.sh` was refused.
Each text starts with the precision macros of
`libmatrix/shader-source.cc:493-505`. The fragment texts also get
`precision mediump float;` (an assumption: our front end refuses a
fragment text with no default float precision). The blur bodies come from
`scene-terrain/blur-renderer.cpp:76-163`, with the radius, sigma and
tilt-shift values of `scene-terrain.cpp:88-120`: tilt-shift radius 4,
sigma 2.7, tilt 0.5; bloom radius 2, sigma 4.0, tilt 1.0. On V3D the
precision does not change the code (every ALU operation is 32-bit). The
constants' digits do not change the counts either.

### Mesa

- **7.1:** `tools/shaders/glmark2_counts_71.tsv` is Mesa 26.2.1's
  `v3d_shc --device 71` over the glmark2 corpus (its header names the
  command and glmark2 `22c527cb`). It has terrain's pairs with `terrain.vert`
  (`terrain-18bca803...`) as the vertex shader. The pairs with
  `terrain-texture.vert` and the noise text failed its translator
  (`translate: cannot parse declarator 'vec2 uvScale'`), so **Mesa has no
  count of the noise shader, at 7.1 or at 4.2**. The fragment files map to
  roles as `gc-terrain-shaders.md:60-71` gives them: `8c866224` tilt V,
  `bbba1ac2` tilt H, `4d266cda` terrain, `bdfd8dd6` and `e043e1e8` bloom.
  The 14- and 15-instruction pair (`895a73bc`, `d697bb50`) is
  luminance/overlay, and `c549a79a` (28) is the normal map. The last two
  mappings are by lookup count and are inferred, not checked.
- **4.2: not measured.** `tools/v3d/README.md:10-29` builds `v3d_shc` in a
  Mesa 26.2.1 tree. `$CONNECTED_DEPS_DIR/downloads/mesa-26.2.1.tar.xz` is
  on this host, but this session's sandbox refused `tar`. As an estimate,
  the scratch `corpus` mode compiles every pair of
  `tools/v3d/qpu_counts_42.tsv` (Mesa 4.2) with ours and reads
  `qpu_counts_71.tsv` beside it. Of 1090 pairs, 874 compile in this
  scratch compile (most of the rest are `#version 450` texts that the ES
  front end refuses). Sums of fragment-shader words:

  | set | pairs | ours (4.2) | Mesa 4.2 | Mesa 7.1 | ours / Mesa 4.2 | Mesa 4.2 / 7.1 |
  |---|---:|---:|---:|---:|---:|---:|
  | every pair | 874 | 41,012 | 31,922 | 28,126 | 1.28 | 1.13 |
  | pairs with a texture look-up | 84 | 4,401 | 4,038 | 3,260 | **1.09** | **1.24** |

  The "Mesa 4.2 (est.)" column below is Mesa 7.1 x 1.24, the ratio of the
  texturing set. The nearest corpus program to a blur is
  `femtovg/filter_image.frag`, a Gaussian loop: ours 87, Mesa 4.2 93.

## 2. Per-shader table

Ours: the scratch run above. Mesa 7.1: `glmark2_counts_71.tsv` (vs, fs,
vs threads, fs threads). Mesa 4.2: estimate, not measured.

| program (job) | stage | ours words | ours threads | ours add / mul halves used | ours look-ups (`retire`) | ours `ldtmu` (16-bit words) | ours `thrsw` | ours `ldunif` / uniforms | spills | Mesa 7.1 words, threads | Mesa 4.2 (est.) |
|---|---|---:|---:|---|---:|---:|---:|---|---:|---|---:|
| noise (`terrain-noise.frag`) | fs | **1495** | **2** | 872 / 830 | 0 | 0 | 3 | 21 / 21 | 0 | not compiled (translator) | not measured |
| normal map | fs | 35 | 4 | 16 / 17 | 3 | 3 | 3 | 3 / 9 | 0 | 28, 4 | 35 |
| luminance (set-up only) | fs | 14 | 4 | - | 1 | 2 | 3 | 3 / 5 | 0 | 14 or 15, 4 | 17 to 19 |
| terrain (`terrain.frag`) | fs | 160 | 4 | 89 / 119 | 5 | 9 | 5 | 39 / 49 | 0 | 136, 4 | 169 |
| terrain (`terrain.vert`) | vs | 172 | **2** | 109 / 98 | 2 | 3 | 3 | 85 / 89 | 0 | 132, 4 | 164 |
| terrain (`terrain.vert`) | cs (bin) | 97 | 4 | 60 / 54 | 1 | 1 | 3 | 63 / 65 | 0 | not in the table | not measured |
| bloom H / V (radius 2) | fs | 46 / 46 | 4 | 20 / 27 | 5 | 10 | 5 | 7 / 17 | 0 | 39 and 40, 4 | 48 to 50 |
| overlay | fs | 14 | 4 | 4 / 8 | 1 | 2 | 3 | 1 / 3 | 0 | 14 or 15, 4 | 17 to 19 |
| tilt-shift H (radius 4) | fs | **78** | 4 | 38 / 56 | 9 | 18 | 7 | 10 / 28 | 0 | 64, 4 | 79 |
| tilt-shift V (radius 4) | fs | **80** | 4 | 38 / 56 | 9 | 18 | 7 | 10 / 28 | 0 | 66, 4 | 82 |
| `terrain-texture.vert` (every pass but terrain) | vs / cs | 26 / 21 | 4 / 4 | - | 0 | 0 | 1 | 19 / 19 | 0 | not compiled (translator) | not measured |
| gl_test case 220's program (`gl_test_shaders_es100.h:75-94`) | fs | 11 | 4 | 4 / 4 | 1 | 2 | 3 | 0 / 2 | 0 | (`uv.vert`+`texture.frag`: 9) | **11, measured** (`qpu_counts_42.tsv`) |

**TMU set-up, ours (`src/shaderc/online/v3d/select.rs:926-1053`).** Each
look-up writes P0 (`QUNIFORM_TMU_CONFIG_P0`: the unit and the mask of
result words) and P1 (the unit, bit 2 "per-pixel mask enable" as Mesa's
fragment look-ups carry it, and bit 0 "32-bit output" only for a `ret32`
binding) through `wrtmuc`. Then it writes T and S (`tmut`, `tmus`) and later
reads one `ldtmu` per result word. Terrain's textures are all RGBA8, so the
return is 16-bit: two f16 channels a word. An `.xyz` or `.xyzw` read is 2
words (18 `ldtmu` for the blurs' 9 look-ups), a `.x` read is 1. Look-ups
are batched until their result words would pass the 4-thread output FIFO
share, `16 / 4 = 4` words (`TMU_OUTPUT_WORDS`, `select.rs:1960-1969`, Mesa's
`ntq_tmu_fifo_overflow`, `nir_to_vir.c:251-258`). So a blur runs 5 batches
of at most 2 look-ups, each behind a `thrsw`. Its 7 `thrsw` are those 5
(the fifth is the doubled "last thread switch" at words 69-70 of
`tilt_v.fs.qpu`) plus the program end. Mesa's 4.2 code follows the same
rules (the cited Mesa lines are the ones our selection transcribes), so its
batching would be the same. Its listing was not produced here (not
measured).

**Where the noise shader's words go** (`noise.fs.qpu`). It has 690 `fmul`,
373 `fadd`, 245 `fsub`, 152 `ffloor` (the `mod289` and `floor` calls), 140
`mov`s of several kinds, and 28 each of `fcmp`/`fmax`/`not`. That is 872
add-half and 830 mul-half operations in 1495 words. Only 5 words have both
halves `nop`, but 623 words leave the add half empty and 665 the mul half.
A perfect pairing would be about 872 words. Data dependences make that a
floor that cannot quite be reached. The shader runs at 2 threads because
its values do not fit 4 threads' registers. With no TMU access the thread
count does not slow it.

## 3. Layouts of the sampled textures

Since `c6b16f71` and `31a4cf8d` every FBO target of terrain is a render
image. The TLB stores its level 0 in the TMU's layout (`image-store`, 7 a
frame, `pi4-slow-terrain.md` third run), and the TFU builds its chain
(`v3d-terrain-copies.md`, "`image-store`"). The layout rule is
`src/v3d/texture.rs:351-366`, a port of Mesa's `v3d_setup_slices`.
`rt_layout` (`:894-903`) drops level 0's UIF XOR for render images. RGBA8
has 4x4 utiles and 8x8 UIF blocks; a level is UIF_XOR when its padded UIF
block rows are a multiple of 32 (`PAGE_CACHE_UB_ROWS`, `:280`).

| job | texture sampled | size, format | level 0 layout (ours) | Mesa's level 0 for the same resource | filter |
|---|---|---|---|---|---|
| normal map | height map (noise target) | 256x256 RGBA8 render image | UIF_NO_XOR (XOR dropped) | UIF_XOR (32 UB rows) | LINEAR |
| terrain VS (and CS) | height map (`tDisplacement`), normal map (`tNormal`) | 256x256 RGBA8 render images | UIF_NO_XOR | UIF_XOR | LINEAR |
| terrain FS | `tDiffuse1`, `tDiffuse2`, `tDetail` (JPEG uploads, `GL_RGB` kept as RGBA8) | 512x512, mipmapped | UIF_XOR (64 UB rows); small levels UBLINEAR/LT | the same | mipmap-linear |
| terrain FS | `tSpecular` (luminance target, rendered at set-up) | 512x512 RGBA8 render image | UIF_NO_XOR | UIF_XOR | mipmap-linear |
| terrain FS | `tDisplacement` (height map) | 256x256 | UIF_NO_XOR | UIF_XOR | LINEAR |
| bloom H | terrain target | 800x600 RGBA8 render image, 10 levels (TFU chain A) | UIF_NO_XOR (75 UB rows) | **UIF_NO_XOR** (75 is not a multiple of 32; `ub_pad` 0) | mipmap-linear |
| bloom V | bloom H | 256x256, 9 levels | UIF_NO_XOR | UIF_XOR | mipmap-linear |
| overlay | bloom V | 256x256 | UIF_NO_XOR | UIF_XOR | mipmap-linear (magnified) |
| tilt-shift H | terrain target (after overlay, chain B) | 800x600 | UIF_NO_XOR | UIF_NO_XOR | mipmap-linear |
| tilt-shift V | tilt-shift H (chain C) | 800x600 | UIF_NO_XOR | UIF_NO_XOR | mipmap-linear |

No job samples a raster image; the window's raster surface is only stored.
The two blurs read UIF level 0 exactly as Mesa would lay out the same
800x600 resource, so **layout is not a cause for the blurs**. The 256x256
and 512x512 render images lose the XOR swizzle that Mesa's layout would
have. Those are small passes (section 4) and the cost of the missing XOR
is not measured.

## 4. Per-job render model, against the board

Assumptions, each marked:

- **QPU issue:** 8 QPUs (`driver/docs/rpi4-v3d.md:163`), each issuing one
  16-lane instruction every 4 clocks, so 32 lane-instructions a clock. That
  is the V3D 4.x QPU model; no document of this tree states it.
- **Clock:** 500 MHz (assumed, as in `v3d-terrain-gpu.md:207-212`). The
  board's `board: V3D clock` line is not in the tree.
- **TMU:** at most **0.92 ns a look-up**, from gl_test case 220. 100 draws
  of 512x512 with 1 look-up a pixel took 24 ms (`pi4-slow-terrain.md:591-601`):
  0.24 ms a draw for 262,144 look-ups. Its fragment shader is 11 words, so
  ALU issue is 262,144 x 11 / 32 = 90k clocks = 0.18 ms. The probe is
  therefore bound by the TMU or by the pixel rate, and 0.92 ns is an upper
  bound for a look-up that hits the cache (the probe's texture is one
  colour).
- **TLB bytes:** 4.47 MB/ms, the TFU rate of `v3d-terrain-gpu.md:161-168`,
  taken for the TLB as well (an assumption).
- **ALU and TMU overlap** (separate units, 2 to 4 threads), so a job is the
  larger of the two, not their sum.
- **Fragments:** 256x256 and 800x600 for the full-screen passes. For the
  terrain draw the fragment lanes are at least the 442k rasterized pixels
  of `terrain-overdraw.md:64-75`; 2x2-quad helper lanes are not counted
  (not measured). Its 393,216 vertices are shaded at least once in the
  render (vertices of triangles that span tiles are shaded again: not
  counted).

| job (current main, 8 a frame) | ALU: lanes x words / 32 / 500 MHz | TMU: look-ups x 0.92 ns | TLB | model `render` |
|---|---|---|---|---:|
| noise 256x256 | 65,536 x 1495 = 98.0 M: **6.12 ms** | 0 | 0.26 MB: 0.06 | **6.1** (ALU) |
| normal map 256x256 | 65,536 x 35: 0.14 | 196,608: 0.18 | 0.06 | 0.2 |
| terrain draw 800x600 | FS 442k x 160 = 70.7 M, VS 393,216 x 172 = 67.6 M: **8.65 ms** (lower bound) | FS 5 x 442k + VS 2 x 393k = 3.0 M: 2.8 | 3.84 MB: 0.86 | **>= 8.7** (ALU) |
| bloom H 256x256 | 65,536 x 46: 0.19 | 327,680: 0.30 | 0.06 | 0.3 |
| bloom V 256x256 | the same | the same | 0.06 | 0.3 |
| overlay 800x600 | 480,000 x 14: 0.42 | 480,000: 0.44 | 3.84 MB loaded and 3.84 MB stored, colour and depth: 1.7 | 0.4 to 1.7 |
| tilt-shift H 800x600 | 480,000 x 78: 2.34 | 4.32 M: **3.97** | 1.92 MB: 0.43 | **<= 4.0** (TMU) |
| tilt-shift V 800x600 | 480,000 x 80: 2.40 | 4.32 M: **3.97** | 1.92 MB raster: 0.43 | **<= 4.0** (TMU) |
| **sum** | | | | **24.0 to 25.3** |

Against the board (frame 10 of `e07d066a`, `pi4-slow-terrain.md:442`,
`:524-527`): 8 jobs, **render sum 23.49 ms**, two entries at 7930 and 9352
us.

- **The noise-and-terrain reading** (the line starts with the previous
  frame's tilt-shift V, then noise, normal, terrain, bloom H, bloom V,
  overlay, tilt-shift H; `main.cpp:1401-1403`). Noise is measured at 7.93
  against a 6.12 model (x 1.30). Terrain is 9.35 against at least 8.65
  (x 1.08 or less). The blurs would be about 3 to 4 ms each, and the sum
  fits. A slower clock cannot explain the noise's extra 30 %: the terrain
  draw caps the clock's error at about 8 % (500 x 8.65 / 9.35 = 463 MHz at
  least). What is left is specific to the noise program. One candidate is
  instruction-cache misses on a 12 KB program (1495 x 8 B; the cache size
  is not in the tree), which is not measured.
- **The two-blurs reading** needs 7.93 + 9.35 for the blurs, plus at least
  6.12 + 8.65 of ALU for noise and terrain, plus at least 1.2 for the other
  four jobs. That is **at least 33.2 ms against 23.49** (or the 27.26 of the
  later frame v3d-jobs-readout studies). For it to fit even the larger sum,
  the clock would have to be about 850 MHz or more
  (14.77 x 500 / f <= 27.26 - 17.28 - 1.2). The Pi 4 does not run its V3D
  near that, so this reading contradicts our counts.
- **What decides it on the board.** The `tfu` marks place the entries.
  `tfu 1/806us` (chain A, the 800x600 terrain target) is on the first job
  that completes after the terrain draw, which is bloom H. So the terrain
  draw is the entry just before the one with the about-800 us TFU and a
  `render` of about 300 us, and noise is two entries before it. The record
  in the tree lists the five TFU times (806, 114, 114, 804, 804 us). The
  whole line with each entry's `render` is not in the tree
  (`records/pi4/20261121-0830-glmark2-e07d066a-4786beae/` is on the runner
  host), so the order is inferred. Item v3d-jobs-readout (in review) reads
  the same line.

## 5. Fix candidates, ranked

Board record values are the `glmark2-jobs: [terrain] ...: frame 10` line
(`main.cpp:1405-1425`). "The noise entry" is the job with no `tfu` two
entries before the terrain draw; "the terrain entry" is the one before the
`tfu 1/8xxus` entry (section 4).

1. **Noise: pair the add and mul halves better in the V3D scheduler.** 1495
   words for 872 add and 830 mul operations; a floor of about 872 to 900
   words. Expected: the noise job **6.1 to about 3.6 ms in the model, -2.4
   to -2.6 ms a frame**. If the measured 7.93 scales with the word count
   (instruction-cache misses would), about -3 ms. Picture risk: **none**,
   because the operations do not change, only the order in which words are
   packed. ES 2.0 risk: every V3D program's words move, so the words pins
   change (`tools/v3d` corpus pins, `tests/v3d_sim_mesa.rs`, `roundtrip.rs`
   counts). The answers of `tests/es2_report_guard.rs` must stay the same.
   The `sim` interpreter can show that values are unchanged. Owner:
   `src/shaderc/online/v3d/sched.rs` (and `select.rs` if the `mov`s are the
   cause), plan #308. Confirming record value: **the noise entry's `render`
   7930 to about 5000 us or less**, the other entries within run-to-run
   spread, render sum down by the same amount. A host pin first: the
   scratch count of `noise.fs` (1495) falls, and `sim` gives the same
   outputs on a pixel grid.
   - 1b. Fewer noise operations (front-end folding). This needs Mesa's 4.2
     count of the same text first. It is not measured: Mesa's translator
     fails on the text and its 4.2 `v3d_shc` was not built here. On the
     GC7000 the same text was 2.03x Mesa's (`gc-terrain-shaders.md`), but
     those causes (`rcp` of a constant, `mov` per negate) are partly
     GC-specific. Here `fsub` carries the negates, and the listing has no
     SFU `recip`. Expected: unknown until that count exists.
2. **Terrain draw: the same pairing for its VS (172 words for 109 add / 98
   mul) and FS (160 for 89 / 119).** Floors of about 115 and 125. Expected:
   ALU 8.65 to about 6.5 ms, **about -2 ms a frame**. The draw stays
   ALU-bound only while its 3.0 M look-ups (2.8 ms) stay under the ALU
   time. Risks and owner as in 1; candidates 1 and 2 are the same change.
   Confirming record value: **the terrain entry's `render` 9352 to about
   7000 us**. The `bin` of that entry (the coordinate shader, 97 words x
   393,216 vertices = 2.4 ms at the model's rate) falls with the CS pairing.
   2b. The terrain VS at 4 threads instead of 2 (register pressure) helps
   only the latency of its 2 look-ups. The gain is not measured and
   probably small, because the draw is ALU-bound.
3. **Blurs: nothing found to fix.** They are 78/80 words (Mesa about the
   same), 4 threads, 16-bit returns, the same TMU batching as Mesa, UIF
   level 0 as in Mesa, no raster sampling and no 32-bit return. Their model
   is at most 4.0 ms each, TMU-bound, and trilinear is ruled out
   (`pi4-slow-terrain.md:598-601`). Fewer look-ups would change glmark2's
   picture (its shader fixes 9 taps). Expected: **0 ms**. Confirming record
   value: the two blur entries read **3 to 4 ms of `render`**. Tilt-shift H
   is the last entry, carrying chain B's `tfu 1/80xus`. Tilt-shift V is the
   first entry, carrying chain C's (or the entry after tilt-shift H, if the
   line holds only frame 10's own jobs). If
   they read 7 to 9 ms, the TMU misses its cache on the 800x600 UIF source,
   section 4's reading is wrong, and a TMU study (cache footprint of the
   9-row vertical kernel) comes next.
4. **Keep UIF XOR on 256x256 and 512x512 render images** (`rt_layout`,
   `src/v3d/texture.rs:894-903`, plan #308). These passes are about 0.2 to
   0.3 ms each, so the gain is at most a fraction of that and not measured.
   Picture risk: none if the TLB, TMU and TFU are told the same layout.
   This is why the XOR was dropped, and the risk is real. Confirming
   values: the normal map, bloom H and bloom V entries' `render`. Rank last.

## What was not done

- **Mesa 4.2 counts**: not measured. The tree's route is a built
  `v3d_shc` (`tools/v3d/README.md:10-36`); this session could not unpack
  the Mesa archive. Mesa 7.1 has no noise count, because its translator
  refuses the `MEDIUMP_OR_DEFAULT vec2 uvScale` declarator.
- **The harness texts**: rebuilt by hand from glmark2's sources, not taken
  from `tools/hostharness/glmark2/run.sh` (refused here). The precision line
  is an assumption (section 1).
- **Quad helper lanes, per-tile vertex re-shading, instruction-cache size,
  the V3D clock**: not measured. Each one makes the terrain and noise
  models larger, never smaller.
- **No board run.** Every board figure is quoted from `pi4-slow-terrain.md`.
