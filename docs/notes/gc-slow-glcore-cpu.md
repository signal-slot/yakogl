<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: the GL core's CPU instructions in glmark2 terrain, shadow and refract (callgrind)

Plan #328 (issues #1204, #1106), item `glcore-cpu-profile`. Investigation only:
no code changed, no board used. `gc-slow-terrain.md`,
`gc-slow-shadow-refract.md` and `imx8mp-glmark2-cpu.md` estimated the CPU time
of the GL core (`src/gl`) in these three scenes from host wall times. This note
counts the instructions, per frame and per function, with callgrind on the
host harness, GC7000 build, at `d0f5813e`.

## Result

- **The GL core is not where the frames go.** `src/gl` itself (state
  validation, uniform upload, binding, the draw path above the back end) is
  **81 K instructions a frame in terrain (2.9% of its frame), 24 K in shadow
  (0.35%) and 27 K in refract (0.4%)**. The ranked list of what in it could
  still be shortened is at the end; none of it is worth more than a few
  percent of a millisecond on the A53.
- The warm frames are 2.84 M (terrain), 6.83 M (shadow) and 6.94 M (refract)
  instructions in total. At the factor the existing notes use (below) that is
  1 to 2 ms an A53 frame for terrain, and shadow's and refract's figure is not
  a time at all: 98% of it is one loop, the CPU depth resolve
  (`gc_depth_resolve`), whose board cost is its uncached loads, not its
  instructions (`gc-slow-shadow-refract.md`, "350 to 1000 ms").
- In terrain the instructions are in the back end: the two CPU tiled copies of
  a draw (`tiled_copy::tile`, 38%, with its `memcpy` 13%), the host OSAL's
  zeroing of the copies' memory twice (37% together: 18% is the host OSAL's, 18% is `tiled_copy.rs:427`) and the GPU mipmap chains
  (17%).
- The shader compiler does not run in warm frames (0 instructions in
  `yakogl::shaderc` after the first frame; 2.8 M once, in refract's setup).

## Commands

Valgrind 3.25.1 was not on the host and perf could not count instructions
(`kernel.perf_event_paranoid` 4). Valgrind was built from source
(`https://sourceware.org/pub/valgrind/valgrind-3.25.1.tar.bz2`,
`configure --prefix=$PWD/build/vg`, `make`, `make install`). The harness is
the stock one, built by its own script; nothing in it changed:

```sh
tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-prof vivante   # 3 frames a scene, builds glmark2-host-vivante
for s in terrain shadow refract; do
  mkdir -p build/cg/$s
  build/vg/bin/valgrind --tool=callgrind --dump-after=eglSwapBuffers \
    --callgrind-out-file=build/cg/$s/cg.out \
    build/glmark2-prof/glmark2-host-vivante --size 800x600 -b :nframes=8 -b $s
done
```

`--dump-after=eglSwapBuffers` writes one profile per frame and zeroes the
counters, so `cg.out.N` holds exactly the instructions between swap N-1 and
swap N. `cg.out.1` is the process start, glmark2's scene setup, the shader
compiles and the first frame (terrain 513 M, shadow 38 M, refract 1501 M).
`cg.out.2` to `cg.out.7` are the warm frames: they differ by under 0.01%
(terrain 2 835 832 to 2 835 944; shadow 6 831 794 to 6 832 007; refract
6 937 384 to 6 937 645). The tables use `cg.out.4`. `cg.out.8` is the last
frame plus glmark2's teardown, and `cg.out` is the exit; neither is used.

The tables come from `callgrind_annotate --inclusive=yes build/cg/<scene>/cg.out.4`,
or, as here (`callgrind_annotate` could not be run in this session), from the
dump itself with the script in the appendix:

```sh
python3 -I cgrep.py build/cg/terrain/cg.out.4 10 0x0000000000198400 0x0000000000197a00
```

Draws a frame: terrain 8 `glDrawArrays` (plus 5 `glGenerateMipmap`, each of
which the GPU chain turns into about 9 recorded draws: 43 in all), shadow 3,
refract 2 (+1 `glGenerateMipmap`).

Limits (as in `tools/hostharness/glmark2/README.md`): nothing renders, and
`osal/linux.c` completes every kick at once, so GPU time and GPU waits are
not here, and the depth resolve reads malloc'd, cached memory. Counts are x86-64
instructions (Ir), not A53 ones; they rank, they do not time. Two figures are
the host OSAL's, not the board's: `memset` in `l_gpu_alloc`
(`src/osal/linux.c:265` zeroes every allocation; `z_gpu_alloc` does not) and
`getenv` / `timing_query` in `l_submit` / `l_query` (`linux.c:353`, `:346`).

## Estimate on the A53 (an estimate, not a measurement)

The existing notes give no instruction-to-time factor for the A53, only a
host-to-board one: `imx8mp-glmark2-cpu.md:38` multiplies the host's wall
milliseconds by **4** ("an assumption, not measured") and says the A53 is
"several times" the host (`:21`, `:115`). The host milliseconds of the GL calls
are those of `gc-slow-terrain.md:32` (0.41 ms, terrain) and
`gc-slow-shadow-refract.md:151` (0.312 ms shadow, 0.297 ms refract), measured
by `GM2_PROFILE=1`. Times 4: **about 1.6 ms (terrain), 1.2 ms (shadow), 1.2 ms
(refract)** of CPU in GL calls on the A53, of which the GL core's own part,
from the shares below, is **0.05 ms, 0.004 ms and 0.005 ms**. The
instruction counts agree on the ranking, not on a scale: shadow's 6.8 M
instructions took 0.31 ms on the host (vector code at high IPC), so ms cannot be
read off the counts. The board's cost of the resolve loop is memory time, see
above. Nothing here was measured on a board.

## terrain (warm frame, `cg.out.4`: 2 835 944 instructions)

By GL entry point (inclusive, calls a frame):

| entry point | calls | instructions | share |
|---|---|---|---|
| `glDrawArrays` | 8 | 2 319 389 | 81.8% |
| `glGenerateMipmap` | 5 | 486 086 | 17.1% |
| `glClear` | 8 | 10 098 | 0.36% |
| `glBindTexture` | 18 | 3 626 | 0.13% |
| `glBindBuffer` | 11 | 2 035 | 0.07% |
| `eglSwapBuffers` | 1 | 1 974 | 0.07% |
| `glUseProgram` | 18 | 1 692 | 0.06% |
| `glVertexAttribPointer` | 11 | 1 551 | 0.05% |
| `glBindFramebuffer` | 8 | 1 190 | 0.04% |
| `glUniform2fv` | 2 | 954 | 0.03% |
| the other 9 entry points (`glActiveTexture`, `glEnable/DisableVertexAttribArray`, `glViewport`, `glDepthMask`, `glDisable`, `glEnable`, `glClearColor`, `glBlendFunc`, `glClearDepthf`) | 61 | 4 285 | 0.15% |
| glmark2's own code (mesh and matrix work outside GL calls) | | 13 461 | 0.5% |

By code (self instructions; they add up to the frame):

| code | instructions | share |
|---|---|---|
| libc `memset` / `memcpy` (the two unnamed libc functions `0x198400`, `0x197a00`) | 1 473 651 | 52.0% |
| &nbsp;&nbsp;of it `memset` called by `draw_one` directly: `core::ptr::write_bytes(dst, 0, total)` before each tiled copy (`src/vivante/tiled_copy.rs:427`, inlined; 2 calls; the board runs it too, on non-cacheable memory) | 524 326 | 18.5% |
| &nbsp;&nbsp;of it `memset` called from the frame `0x4038a90` (4 calls; `l_gpu_alloc`'s zeroing of the copies' memory, `src/osal/linux.c:265`, host only; callgrind does not name the object) | 524 350 | 18.5% |
| &nbsp;&nbsp;of it `memcpy` from `tiled_copy::tile` (32 768 calls) | 360 448 | 12.7% |
| `vivante::tiled_copy` (`tile`) | 710 228 | 25.0% |
| `vivante::backend` (`emit_draw`, `record`, `draw_one`, `frame_open`, `hw_targets`, ...) | 267 363 | 9.4% |
| `vivante::shader` | 67 657 | 2.4% |
| `vivante::texture` | 49 139 | 1.7% |
| host OSAL (`getenv` 56 888, `timing_query` 44 284, `clock_gettime` 8 008, `linux.c` 33 341) | 142 521 | 5.0% |
| **GL core** (`gl::draw` 38 310, `gl::framebuffer` 25 360, `gl::context` 4 056, `gl::names` 1 925, `gl::texture` 1 205, `gl::program` 848, `gl::tf` 308, `gl::state` 294, GL entry thunks 9 059) | 81 365 | 2.9% |
| `vivante::dedup` 9 788, `draw_steps` 7 800, `vivante::mipmap` 6 297, `vivante::ds_cache` 1 722, rest | 36 000 | 1.3% |

(The two 524 K `memset` lines are separate calls and add to the memset's
1 058 280 self instructions. `l_gpu_alloc` is 526 311 inclusive, which is the
second one; the first is the `write_bytes` that zeroes the destination.)

By back-end function (inclusive):

| function | calls | instructions | where |
|---|---|---|---|
| `backend::draw_one` | 9 + 43 | 2 706 381 | `src/vivante/backend.rs:2748` |
| `tiled_copy::tile` (CPU tiled copy, 2 a frame) | 2 | 1 067 988 | `src/vivante/tiled_copy.rs:251`, called `:428` from `prepare` `:346` |
| `l_gpu_alloc` (host zeroing, the copies' memory) | 2 | 526 311 | `src/osal/linux.c:265` |
| `mipmap::gc_generate_mipmap` (5 chains, 43 recorded draws) | 5 | 475 848 | `src/vivante/mipmap.rs:304` |
| `backend::record` | 52 | 405 981 | `src/vivante/backend.rs:3091` |
| `backend::emit_draw` | 52 | 348 410 | `src/vivante/backend.rs:2116` |
| `backend::frame_submit` + `l_submit` | | 70 580 | |
| `dedup::words` (3 closures) | | about 140 000, inside `emit_draw` | `src/vivante/dedup.rs` |
| `texture::emit_slot` / `build_descriptor` | | 50 483 / 39 227 | `src/vivante/texture.rs` |
| `shader::emit_uniforms` | | 44 895 | `src/vivante/shader.rs` |

Top 10 functions, self instructions:

| # | function | self | share |
|---|---|---|---|
| 1 | libc `memset` (`0x198400`) | 1 058 280 | 37.3% |
| 2 | `vivante::tiled_copy::tile` | 707 540 | 24.9% |
| 3 | libc `memcpy` (`0x197a00`) | 411 014 | 14.5% |
| 4 | `vivante::backend::emit_draw` | 94 782 | 3.3% |
| 5 | `getenv` (`l_submit`, 104 calls; host only) | 56 888 | 2.0% |
| 6 | `timing_query` (`l_query`; host only) | 44 284 | 1.6% |
| 7 | `emit_draw::{closure#8}` | 36 002 | 1.3% |
| 8 | `shader::emit_stage_uniforms` | 31 180 | 1.1% |
| 9 | `backend::record` | 29 697 | 1.0% |
| 10 | `backend::draw_one` | 26 973 | 1.0% |

Top 10 by inclusive instructions, library side: `draw_one` 2 706 381,
`glDrawArrays` 2 319 389, `gl::draw::draw_arrays` 2 318 941, `gl::draw::submit_draw`
2 260 627, `backend::gc_draw` 2 259 409, `tiled_copy::tile` 1 067 988,
`l_gpu_alloc` 526 311, `glGenerateMipmap` 486 086, `backend::generate_mipmap`
475 928, `backend::record` 405 981.

## shadow (warm frame: 6 831 802 instructions)

| entry point | calls | instructions | share |
|---|---|---|---|
| `glDrawArrays` | 3 | 6 803 266 | 99.6% |
| `glClear` | 2 | 8 901 | 0.13% |
| `glUniformMatrix4fv` | 5 | 3 355 | 0.05% |
| `eglSwapBuffers` | 1 | 1 974 | 0.03% |
| `glFramebufferTexture2D` | 1 | 1 745 | 0.03% |
| `glBindBuffer` | 6 | 1 028 | 0.02% |
| `glVertexAttribPointer` | 5 | 705 | 0.01% |
| the other 9 entry points | 31 | about 2 100 | 0.03% |

| code | instructions | share |
|---|---|---|
| `backend::gc_depth_resolve` (`src/vivante/backend.rs:2995`, into `ds_cache::resolve_depth` `src/vivante/ds_cache.rs:562`) | 6 748 975 | 98.8% |
| rest of `vivante::backend` (`emit_draw` 7 564, `draw_one` 3 644, ...) | 22 145 | 0.3% |
| **GL core** (`gl::draw` 10 471, `gl::framebuffer` 5 111, `gl::program` 3 421, `gl::context` 1 274, `gl::names` 485, `gl::texture` 187, `gl::tf` 140, thunks 2 592) | 23 681 | 0.35% |
| `vivante::shader` 7 194, libc 7 207, host OSAL 11 522, glmark2 8 065, rest | 28 000 | 0.4% |

`glDrawArrays` carries the resolve: `draw_arrays` -> `build_common`
(`src/gl/draw.rs:3359`, 6 765 522 inclusive) -> `texture::resolve_depth`
(`src/gl/texture.rs:698`) -> `gc_depth_resolve`, once a frame, 6 749 355. The
recording of the 3 draws is 42 K.

Top 10 functions, self: `gc_depth_resolve` 6 748 975 (98.8%), `emit_draw` 7 564,
`gl::draw::build_common` 6 826, libc `memcpy` 5 066, `timing_query` 4 365,
`draw_one` 3 644, `shader::emit_stage_uniforms` 3 508, `getenv` 3 282,
`shader::emit_shader_state` 2 831, `l_query` 2 349.

## refract (warm frame: 6 937 384 instructions)

| entry point | calls | instructions | share |
|---|---|---|---|
| `glDrawArrays` | 2 | 6 794 424 | 97.9% |
| `glGenerateMipmap` | 1 | 112 416 | 1.6% |
| `glClear` | 2 | 8 907 | 0.13% |
| `glFramebufferTexture2D` | 2 | 3 482 | 0.05% |
| `glUniformMatrix4fv` | 5 | 3 355 | 0.05% |
| `eglSwapBuffers` | 1 | 1 974 | 0.03% |
| `glUniform1i` | 3 | 1 143 | 0.02% |
| `glBindTexture` | 5 | 935 | 0.01% |
| the other 11 entry points | 29 | about 3 100 | 0.04% |

| code | instructions | share |
|---|---|---|
| `backend::gc_depth_resolve` | 6 748 957 | 97.3% |
| rest of `vivante::backend` (`emit_draw` 22 454, `record` 7 140, `draw_one` 7 408, ...) | 65 305 | 0.9% |
| **GL core** (`gl::draw` 9 649, `gl::framebuffer` 7 555, `gl::program` 4 093, `gl::context` 1 326, `gl::names` 695, `gl::texture` 357, `gl::tf` 112, thunks 2 855) | 26 642 | 0.4% |
| `vivante::shader` 16 823, `vivante::texture` 11 199, libc 16 488, host OSAL 35 000, glmark2 8 920, `mipmap` 1 411, rest | 79 000 | 1.1% |

The GPU mipmap chain (1600x1200, 11 levels) is 110 538 instructions in
`mipmap::gc_generate_mipmap` for 10 recorded draws, 1.6% of the frame.

Top 10 functions, self: `gc_depth_resolve` 6 748 957 (97.3%), `emit_draw` 22 454,
`getenv` 14 222, libc `memcpy` 12 582, `timing_query` 10 921, `emit_draw::{closure#8}`
8 522, `shader::emit_stage_uniforms` 7 850, `draw_one` 7 408, `record` 7 140,
`shader::emit_shader_state` 6 579.

## What the tables say

1. **Warm frames carry no GL-core cost that matters.** Per `glDrawArrays` the
   GL core is about 4 K instructions in terrain (8 draws), 5 K in shadow and
   7 K in refract, against 7 to 8 K for the back end's recording.
2. **Terrain's CPU instructions are the back end's copies.** The two CPU
   tiled copies (`tiled_copy.rs:251`) cost 1.07 M plus the `memcpy` and the
   zeroing under them. This is the item `tiled-copy-rs` (RS tiling in the
   stream, no CPU copy). On the A53 the copies read and write non-cacheable
   memory, so their time is above their instruction count
   (`v3d-frame-cpu-model.md`, `gc-terrain-tiled-copy.md`).
3. **Shadow's and refract's instructions are the depth resolve** (items
   `resolve-wide`, `depth-sample-gc`). Its instruction count says nothing
   about the board: it is the uncached loads.
4. The 5 mipmap chains of terrain cost 486 K instructions a frame (95 K a
   chain). That is CPU recording of about 9 draws a chain; item `mip-one-kick`
   changes the kicks, not these instructions.
5. The slow frames on the board (250 ms, 500 ms) are therefore not CPU
   instructions in the GL core, and not the shader compiler. They are GPU
   time, the waits, and the uncached memory passes, which an instruction
   count does not show.

## Ranked GL-core costs worth fixing (all small)

Instructions a frame in terrain, the scene where `src/gl` is largest (shadow
and refract show the same functions at a third to a tenth of the count).
None is a reason for a plan item of its own; they are listed because the
contract asked for the ranking and they are cheap to remove in passing.

| # | cost | where | terrain | note |
|---|---|---|---|---|
| 1 | `build_common`, the per-draw state gathering | `src/gl/draw.rs:3359` | 53 K inclusive (6.6 K a draw), 23.9 K self | includes the `memset`/`memcpy` of the draw record (400 and 5 056 instructions). The largest `src/gl` function in all three scenes (6.8 K self in shadow for 3 draws, 5.9 K in refract). |
| 2 | framebuffer queries repeated for every draw: `fb_target_pitched_n` (16 K inclusive), `attachment_loc` (13 K), `check_status` (8 K), `attachment_storage` (8.8 K), `level_above_zero_attached` (7 K), `attached_samples` (2 K) | `src/gl/framebuffer.rs:1186`, `:1094`, `:750`, `:451`, `:1831`, `:862` | 25.4 K self | the same answers each draw while the framebuffer, its attachments and their sizes do not change; a per-framebuffer cache invalidated by the attachment calls would remove most of it. |
| 3 | `draw_arrays` and `submit_draw` bodies | `src/gl/draw.rs:4286`, `:582` | 1.2 K, 1.2 K self | not worth touching. |
| 4 | `resolve_target` | `src/gl/draw.rs:2018` | 8.9 K inclusive | one call a draw. |
| 5 | `pending_clear_for` / `emit_pending_clear` | `src/gl/draw.rs:2655`, `:2435` | 9.4 K inclusive (`glClear` 10 K for 8 calls), 8.5 K in shadow | `glClear` is 1.2 K a call; shadow's `emit_pending_clear` copies 365 instructions of `memcpy`. |
| 6 | `context::is_foreign` | `src/gl/context.rs:1121` | 4.1 K self | called many times a draw; a field compare. |
| 7 | `names::holder` | `src/gl/names.rs:105` | 1.9 K | |
| 8 | `program::set_uniform` | `src/gl/program.rs:2490` | 2.3 K self in shadow | 5 `glUniformMatrix4fv` a frame, 3.4 K. |
| 9 | `draw_stamp` | `src/gl/draw.rs:458` | 1.9 K | |

All together the nine are about 70 K of the 2.84 M, so removing every one would
save under 3% of terrain's instructions and under 0.4% of shadow's and
refract's.

### Costs outside `src/gl` that the same profile ranks higher

Reported for the planner; not part of this item and not changed.

- `tiled_copy::tile` and its `memcpy` (`src/vivante/tiled_copy.rs:251`, `:428`):
  1.4 M of terrain's 2.84 M. Item `tiled-copy-rs`.
- `tiled_copy.rs:427` zeroes each copy's whole destination
  (`write_bytes(dst, 0, total)`, 524 K instructions a frame) and then `tile`
  writes every byte of it again; on the board that is a second pass over
  non-cacheable memory. The copies' memory is also freed and allocated again
  every frame (`l_gpu_alloc` is called 2 times a frame from `draw_one`,
  `tiled_copy.rs:380` to `:415`; two `gc_render_alloc` / `gc_render_free` on
  the board, not measured). `tile` writes the whole destination only if the
  copy has no padding the GPU reads; to be checked by `tiled-copy-rs`, which
  rewrites this code.
- Host-only costs that inflate the host harness and not the board:
  the `memset` of `l_gpu_alloc` (`src/osal/linux.c:265`, 524 K a frame), and `getenv`
  (`src/osal/linux.c:353`, in `l_submit`, 104 calls a frame, 58 K).
  `GM2_NOCLK`'s `getenv` runs on every submit: harmless on the board, which
  has its own OSAL.
- The depth resolve, 98% of shadow and refract (`src/vivante/backend.rs:2995`,
  `src/vivante/ds_cache.rs:562`). 6.75 M instructions for 1.92 M texels is 3.5
  instructions a texel; the cost on the board is the uncached loads.

## Appendix: `cgrep.py`

The parser used for the tables (`python3 -I cgrep.py <dump> <top-N> [callee...]`).
It reads the callgrind format (name compression, `calls=` lines); the cost
line after `calls=` is the inclusive cost of that call. Entry points are the
symbols matching `^(gl|egl)[A-Z]\w*$`; their instructions are the sum, over
their callers outside the GL entry points, of the cost of those calls.

```python
import sys, re, collections

def parse(path):
    names = {}
    selfc = collections.Counter(); incl = collections.Counter()
    calls = collections.Counter(); ccost = collections.Counter()
    cur = cfn = None; incall = False
    def nm(tok):
        m = re.match(r'\((\d+)\)(?: (.*))?$', tok)
        if not m: return tok
        i, n = m.group(1), m.group(2)
        if n: names[i] = n
        return names.get(i, i)
    for line in open(path, errors='replace'):
        line = line.rstrip('\n')
        if line.startswith('fn='): cur = nm(line[3:]); incall = False
        elif line.startswith('cfn='): cfn = nm(line[4:])
        elif line.startswith('calls='):
            calls[(cur, cfn)] += int(line.split('=')[1].split()[0]); incall = True
        elif line and (line[0].isdigit() or line[0] in '+-*'):
            p = line.split()
            if len(p) >= 2:
                c = int(p[1])
                if incall: incl[cur] += c; ccost[(cur, cfn)] += c; incall = False
                else: selfc[cur] += c
    return selfc, incl, calls, ccost
# group(): name prefix -> yakogl::vivante::<module>, yakogl::gl::<module>, l_* (osal),
# 0x... (libc without symbols), the rest. main(): prints the entry points
# (calls, inclusive Ir), the groups (self Ir), the top self and top inclusive
# lists and, for each extra argument, the callers of that function.
```

The two unnamed libc functions are `memset` (`0x198400`) and `memcpy`
(`0x197a00`) of glibc 2.43: the first is called by `draw_one` (the `write_bytes`), by the frame
`0x4038a90` (`l_gpu_alloc`) and by `emit_stage_uniforms`, the second 32 768 times by
`tiled_copy::tile` (11 instructions a call).
