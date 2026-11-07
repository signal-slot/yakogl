<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: where glmark2 terrain's 83 ms frame goes (host and code breakdown)

Plan #329 item am62p-breakdown, issue #1199. Written 2026-11-07 from main
`4370107f` and checked at `aaf8e53e`, which changes none of the code cited
(line numbers are both commits'). Host runs and code reading
only: no library or driver code changed, no board run. The board figures are
the records the cited notes and commits give; each is marked with its commit.

**Result.** Terrain's frame is 52 kicks, and the CPU spends it waiting for
them. On the host (PowerVR build, every submit complete at once) the frame
costs 0.73 ms of CPU, 0.44 ms of it in GL calls; the board's 83.06 ms
(`6a7021ec`) has 81.9 ms in GL calls. Of the 52 kicks, 43 are mipmap levels
(three 800x600 chains of 9, two 256x256 chains of 8), 8 are the scene's
passes and 1 is a window clear that the scene overwrites before anyone
reads it. The board record of `ce3354ac` (the same mipmap path) splits the
83 ms into about 52.7 kicks at about 1.1 ms each (59 ms), one fence a frame
that does not move for 20 ms until the render service re-posts the kick,
and 3 to 4 ms of CPU work. A queued kick costs about 1.1 ms while the GPU
is busy for about 0.19 ms of it (gl_test case 216's TA|3D counters), and a
mip kick alone on an idle GPU would cost about 0.35 to 0.5 ms by the
window-size sweep's fit. Nothing the host or the CPU does explains the rest:
it is the firmware's take-up of a queued kick (post to geometry done about
1 ms). The two levers with the most milliseconds are the 20 ms fence (up
to -19 ms, driver only) and fewer renders a chain (-25 ms with three levels
a render); both are ranked at the end with the board record value that
confirms each.

## The host runs

glmark2 `22c527cb` (`$CONNECTED_DEPS_DIR/glmark2-src`), the harness of
`tools/hostharness/glmark2/` unchanged, through the wrapper
`tools/hostharness/am62p-terrain/run.sh`, which sets
`GM2_WRITE=0 GM2_ENTRIES='^terrain$' GM2_FRAMES=20` and runs:

```sh
tools/hostharness/am62p-terrain/run.sh "$GLMARK2_SRC" build/am62p-terrain profile
#  = GM2_PROFILE=1 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/am62p-terrain/prof powervr
tools/hostharness/am62p-terrain/run.sh "$GLMARK2_SRC" build/am62p-terrain perf
#  = GM2_PERF=1 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/am62p-terrain/perf powervr
tools/hostharness/am62p-terrain/run.sh "$GLMARK2_SRC" build/am62p-terrain kicks
#  = GM2_KICKS=1 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/am62p-terrain/kicks powervr
#    then tools/hostharness/am62p-terrain/kicks.py build/am62p-terrain/kicks/logs/powervr/0.log
```

`kicks.py` reads the `gm2: draw <n> fb <fb> kick <k> submitted <s>` line
shim.c writes after every draw with `GM2_KICKS=1`, cuts the frames at the
draw into fb 0 (the window), and prints the kicks submitted between two
draws.

### Profile (`GM2_PROFILE=1`): the CPU's GL calls

`profile-powervr.md`, 19 counted frames (frames 2 to 20):

> 19 counted frames: 0.729 ms wall, 0.729 ms cpu a frame; in GL and EGL
> calls 0.440 ms (60%), in glmark2 itself 0.289 ms.

and the raw lines:

```text
gm2: scene-end [terrain] <default>: ok frames=20 draws=161 refused=0 gl_errors=0 incomplete_draws=0
gm2: gl-frames swaps=20 counted=19 wall_ns=13860055 cpu_ns=13859869 gl_wall_ns=8368210 gpu_jobs=0 gpu_ns=0
gm2: draw-steps counted=19 scopes=152 untimed=0 ns=197716,157418,103308,247555,57727,0,0,0,0,0,0,0
```

| entry point | calls a frame | wall ms a frame | share of GL wall |
|---|---:|---:|---:|
| `glGenerateMipmap` | 5.0 | 0.221 | 50.1% |
| `glDrawArrays` | 8.0 | 0.154 | 35.0% |
| `glClear` | 8.0 | 0.010 | 2.2% |
| `glUseProgram` | 18.0 | 0.007 | 1.7% |
| `glBindTexture` | 18.0 | 0.007 | 1.6% |
| `glBindBuffer` | 11.0 | 0.007 | 1.5% |
| `glActiveTexture` | 13.0 | 0.004 | 0.9% |
| `glVertexAttribPointer`, `glEnable`/`glDisableVertexAttribArray` | 11.0 each | 0.011 | 2.5% |
| `glBindFramebuffer`, `glViewport`, `glDepthMask` | 8.0 each | 0.009 | 2.0% |
| `eglSwapBuffers` | 1.0 | 0.003 | 0.7% |
| the other 9 (`glUniform2fv`, `glDisable`, `glEnable`, `glClearColor`, ...) | 18.4 | 0.009 | 2.0% |

The draw steps (`src/draw_steps.rs`): 8.0 draws a frame, 5.02 us a draw
(`GlProgram` 1.30, `GlUniforms` 1.04, `GlAttribs` 0.68, `GlTextures` 1.63,
`GlState` 0.38). `glGenerateMipmap`'s 0.221 ms is the backend recording 43
level kicks (about 5 us a kick on the host: `mip_chain_gpu`,
`src/pvr/backend.rs:6241`, one `record_mip_level` a level, `:6268-6269`).

### perf (`GM2_PERF=1`): not run here

The machine that wrote this note has `kernel.perf_event_paranoid` 4, and the
run's process ended at once (`logs/powervr/0.rc` 255) with perf's
`Failure to open event 'cpu-clock:u' on PMU 'software'` and no counted frame.
The per-function split it would give is of the 0.44 ms above, under 4% of
the board's frame (below), so it ranks nothing; a host with paranoid 2 or
lower runs the same command line.

### Kicks (`GM2_KICKS=1`): 52 a frame, and what each is

`kicks.py`'s summary: `frames 1..18: 8.00 draws and 52.00 kicks submitted
a frame` (frame 0, the setup's, submits 61). Every steady frame is the same:

```text
frame 5: 8 draws, 52 kicks submitted; per draw (its fb, kicks since the draw before): fb1+2 fb2+1 fb4+1 fb5+10 fb6+9 fb4+9 fb7+10 fb0+10
```

Read with glmark2's terrain (`scene-terrain.cpp:332-339`, the renderers of
`:59-173`, `scene-terrain/base-renderer.cpp:84-91` `update_mipmap`):

| kicks | what | target |
|---:|---|---|
| 1 | the window's clear of glmark2's main loop (`canvas-generic.cpp:103`), sent as a kick of its own when the first pass draws into fb 1 (`src/gl/draw.rs:2523-2531`, `settle_pending_clear`): a frame with no draw, so it gets the covering object (`src/pvr/backend.rs:8508-8518`) and, not being a swap, stores its colour and depth/stencil | window 800x600, D24S8 |
| 1 + 1 | height map, normal map (no depth) | fb 1, fb 2, 256x256 |
| 1 + 9 | terrain (depth 16), then its 800x600 chain | fb 4; the levels at 400x300 |
| 1 + 8 | bloom h, then its chain | fb 5 256x256; the levels at 128x128 |
| 1 + 8 | bloom v, then its chain | fb 6 256x256; the levels at 128x128 |
| 1 + 9 | overlay into the terrain texture (loads its colour and depth), then the terrain chain again (`overlay-renderer.cpp:72-89`) | fb 4; the levels at 400x300 |
| 1 + 9 | tilt-shift h, then its chain | fb 7 800x600; the levels at 400x300 |
| 1 | tilt-shift v into the window, submitted by the swap | window 800x600 |
| **52** | 8 passes, 43 mip levels, 1 window clear | |

The 512x512 specular map's chain is made once in setup (`scene-terrain.cpp:300`;
the log's `the GPU made levels 1..=9 of the 10-level chain of a 512x512
texture`), not per frame. The board's 52.7 kicks a frame (`ce3354ac`: 5802
kicks over 110 frames) counts the setup's kicks in too.

The window clear is dead: tilt-shift v clears the window again before it
draws (`texture-renderer.cpp:57`), and nothing reads the window between the
two.

## The five parts of the frame

| part | ms a frame | from |
|---|---:|---|
| CPU time in GL calls | about 2 to 3 | host 0.440 ms; the board runs the draw steps about 6 times slower (0.25 ms a frame at `ce3354ac` against the host's 0.040), so 0.44 x 5 to 6 |
| glmark2's own CPU time | about 1.2 | the board's frame less its GL time (83.06 - 81.9 at `6a7021ec`); host 0.289 ms |
| logging | 0 | no line in a counted frame on the host; on the board the one line on the frame path is the re-post's, rate-limited to the first 8 and every 256th (`driver/rust/src/gpu/pvr/render.rs:2371`) |
| CPU waits on the GPU, from kick to kick | about 57 to 59 | 52 to 52.7 kicks at about 1.1 ms (`ce3354ac`: "kicks completing 1.11 ms apart and every wait about 1 ms"; after it, "the frame is still waits of about 1 ms a kick") |
| the once-a-frame 20 ms fence | about 20 | `ce3354ac`: "every frame now has one frame fence that does not move for 20 ms until the render service re-posts the kick" |
| **sum** | **about 80 to 83** | the board: 83.06 ms (`6a7021ec`), 83.08 ms (`ce3354ac`) |
| GPU time (TA or 3D busy) | about 10, under the waits | case 216: 7.75 ms a frame for 40 kicks, 0.19 ms a kick; 52 kicks x 0.19 |

The GPU time is not added: it runs while the CPU waits. The kicks are the
waits: the CPU is never more than four kicks ahead of the GPU, so the kick
rate is the frame rate.

### CPU waits on the GPU, by call site

Each backend wait is `Backend::wait_fence` (`src/pvr/backend.rs:2327`): it
returns at once when the fence is complete, else the OSAL's `z_wait`
(`src/osal/zephyr.c:636`) -> `pvr_render_wait` (`driver/src/pvr_render.c:778`,
named "frame fence" in the log) -> `wait_for`
(`driver/rust/src/gpu/pvr/render.rs:2327`). On the host every fence is
complete, so the host counts no wait (`Backend::waits` stays 0); the table
says where terrain's frame meets one on the board, from the code.

| call site | per frame | met on the board |
|---|---:|---|
| the frame set: kick N is recorded into set N mod 4 and waits for kick N - 4 (`src/pvr/backend.rs:5288-5291`, `PVR_BACKEND_FRAME_SETS` = 4 at `:271`) | 52 (one a kick) | nearly every kick: the GPU retires a kick every ~1.1 ms and the CPU records one in tens of us |
| the swap: `eglSwapBuffers` -> `finish_previous` (`src/gl/egl.rs:2413`) -> `wait_fence(last_fence - 1)` (`src/pvr/backend.rs:9174-9190`) | 1 | usually already complete (the frame-set waits drained the queue to 3) |
| the render service: "HWRT member in flight" (`render.rs:752`), "HWRT pair recycle" (`render.rs:790`), "geometry UFO ring throttle" (`render.rs:1397`) | 0 | not met: four members a size (`HWRT_MEMBERS`, `render.rs:85`) and the frame sets cap the kicks in flight at 4; terrain's 4 sizes take 8 of the 16 pairs; 16 UFO slots (`render.rs:54`). Before `ce3354ac` the first was met at every mip kick (999 us a submit) |
| `glGenerateMipmap` | 0 | the default chain (`MipChain::Whole`, `src/gl/texture.rs:4280-4289`) waits for nothing; with `Backend::mip_cpu_tail` the prefix waits for its last kick (`src/pvr/backend.rs:6290-6293`), 5 waits a frame |
| `settle_texture` (`src/gl/texture.rs:900-920`), `pvr_return_mipmap_chain` (`src/pvr/backend.rs:6364`), staging waits | 0 | terrain's frame neither uploads nor reads back a texture |

Which wait meets the 20 ms fence is not in any record. Every backend wait is
logged as "frame fence", and the re-post line prints the fence number only
(`render.rs:2373-2380`). A wait for the kick posted last is the case the
re-post exists for: "the firmware sits on a posted kick until another command
arrives" (`driver/src/pvr_render.c:333-345`, on by default since
2026-09-24). Path B's prefix waits are exactly such waits, and they cost
15.4 ms for 4 kicks (`am62p-pvr-mipmap-kicks.md`, "Measured"), where four
queued kicks cost 4.6 ms. In the default frame no backend wait names the
kick posted last (the frame set waits for N - 4, the swap for N - 1 after N
is posted), so the once-a-frame stall is not explained by the code; it
needs the counter below.

### CPU loops over carve-out memory

None in the default frame: no level is read or written by the CPU, and no
texture is uploaded per frame. With `mip_cpu_tail` on, the GL core filters
levels 4.. of each 800x600 chain and 2.. of each 256x256 chain from the
carve-out, mapped Normal non-cacheable (`driver/src/pvr_fw_boot.c:587-595`):
7.87 ms a prefix call on the board (`efa0170f`), and 90,112 bytes of cache
maintenance a frame.

## What a mip-level kick pays whatever its size

`record_mip_level` (`src/pvr/backend.rs:6138-6202`) draws one quad through
`draw_one`, so every level is a frame and a kick of its own.

| fixed cost | in code | the board figure it ties to (`pvr-fence-wait.md`) |
|---|---|---|
| a render target of level 1's size for every level: a 3x2 level walks the 25 x 19 = 475 tiles of 400x300 (the 256x256 chains: 64 tiles of 128x128); only the store is clipped to the level | `d.target_w/h = hwrt.max(dst)` (`:6151-6152`), `Backend::frame_clip` (`:5228`, `:8901-8916`); one HWRT set a size (`hwrt_get`, `render.rs:703`) | the sweep: fragment phase 121 us + 0.366 us a tile, geometry 196 us + 0.030 us a tile; the clear-only frame 0.26 us a tile. 475 tiles: about 0.12 to 0.17 ms of fragment and 14 us of geometry a level |
| the background clear | `d.do_clear = true` (`:6158`), the frame's `pds_bgnd` | the clear-only 800x600 frame: 495 us, mostly per tile |
| no depth/stencil load, store or init quad | `GlesDraw::ZERO` has `ds_persist` false, so `ds_resolve` answers `NoAttachment` (`:4258-4260`) | nothing to gain: the 1.9 MB ZLS load and store of the window kick (the fence-wait verdict) are not in a mip kick |
| PDS programs re-emitted into the frame set, arenas cleaned | `frame_begin_with` (`:5292-5296`), `cache_flush` of the PDS, general and VDM arenas (`:8987-8989`) | CPU only: about 5 us a kick on the host |
| one frame set of four: waits for kick N - 4 | `:5288-5291` | the kick period below |
| the firmware round trip: GEOM + UPDATE in the geometry CCB, FENCE + 3D + UPDATE in the fragment CCB, one COMBINED_TA_3D_KICK and the doorbell, one cleanup slot | `render.rs:1427-1459`, `:1579-1584`, `:1598-1628` | a 64x64 kick posted to an idle GPU: post -> geometry fence 120-140 us, post -> fragment fence 220-230 us |
| the MMU cache invalidate, only when a mapping changed | `mmu_cache_sync` (`render.rs:1156-1161`, `mmu_gate` on) | `syncs 0` in steady scenes; 4.10 ms median when posted |
| the 1 ms poll: a wait reads the fragment UFO once a millisecond | `WAIT_POLL_US` (`render.rs:2026-2028`), the loop (`:2333-2454`), `k_busy_wait` (`driver/src/pvr_fw_boot.c:109-112`) | a wait ends on the first whole ms; the 20 us poll saved ~100 us a light frame and 1.4 ms a terrain frame (83.08 -> 81.67, `ce3354ac`) |
| the re-post at 20 ms of a fence that does not move | `render.rs:2361-2382` | once a terrain frame (`ce3354ac`); path B's waits for the last kick |

Added up for a 400x300 level on an idle GPU: about 0.2 ms of geometry and
0.12 to 0.3 ms of fragment, 0.35 to 0.5 ms. Queued, as in terrain, a kick
costs about 1.1 ms (`ce3354ac`; case 216: 40 kicks in 46.15 ms), of which
the GPU is busy about 0.19 ms (case 216: TA|3D 7.75 ms a frame) and the host
about 5 us (`submit 5 us a kick`, `ce3354ac`). With the fence read every
20 us, post to geometry done is still about 1 ms a kick (`ce3354ac`), so
neither the poll nor the CPU sets the period. The ~0.9 ms left is between
the post and the geometry fence: the firmware's take-up of a queued kick.
The code does not show what it is; the firmware's own timestamps (HWPerf,
`pvr-fence-wait.md`) would.

Two readings of case 216 cannot be told apart from its record: 40 kicks at
1.15 ms each, or 40 kicks at about 0.65 ms plus one 20 ms fence at its
`glFinish` (the wait for the kick posted last). The re-post count over the
case's frames separates them (below).

## Fix candidates, ranked

Expected milliseconds are a frame of glmark2 terrain on the AM62P, from the
figures above; they are predictions. "Pictures" is the column `am62p` of
`tools/cts/glmark2-pictures.txt`; "ES 2.0" the dEQP-GLES2 answers and the
reported strings (`tests/es2_report_guard.rs`).

1. **The once-a-frame 20 ms fence.** Find the wait that meets it, then end
   it sooner: re-ring the doorbell every millisecond of a wait instead of
   every 8th (`wait_rering`), re-post earlier than 20 ms, or post a no-op
   command after a kick nothing follows.
   - Files: `driver/rust/src/gpu/pvr/render.rs` (`wait_for`, `:2356-2382`),
     `driver/src/pvr_render.c` (`pvr_render_wait_rering`,
     `pvr_render_wait_repost`, `:328-348`). Both are **#325's**.
   - Expected: up to -19 ms (83 -> about 64), if the fence is the stall the
     re-post ends. The board session can try it first with no new code but
     the knob: `pvr_render_wait_rering = true` (re-ring every 8 ms) would
     cut the stall to about 8 ms if a doorbell is enough.
   - Pictures and ES 2.0: none (the same commands, sooner).
   - Confirms it: the `render: fence N (frame fence) not moved at 20 ms,
     re-posted 1 kick(s) (M so far)` lines of the GM record (M over the
     frames run: about 1 a terrain frame now, 0 after); terrain's `FPS:`
     and the `glmark2-frame: [terrain]` line's gl and swap us; with the
     kick-times build, `pvr last completed kick per frame: ... wait`.
2. **Fewer renders a chain: three levels a render.** Level 1 from level 0,
   then levels l+1..l+3 from level l in one render with three colour
   targets (each the 2^j box of level l, as bilinear taps), so 4 renders an
   800x600 chain (1; 2-4; 5-7; 8-9) and 4 a 256x256 chain (1; 2-4; 5-7; 8).
   The draw already carries several targets (`GlesDraw::targets`,
   `frame_begin_with` `:5234-5239`), each with its own size and store clip
   (`:8901-8916`), and the end-of-tile programs exist for 1 to 3 targets
   (`PVR_RENDER_MAX_RT` = 3, `driver/src/pvr_render.h:57`). The rejection of
   path C in `am62p-pvr-mipmap-kicks.md` ("a new draw ABI") predates that.
   - Files: `src/pvr/backend.rs` (`mip_chain_gpu`, `record_mip_level`), a
     new internal fragment program (`tools/shaders/internal/es/`) and the
     generated `src/pvr/internal.rs` and `tests/reference/pvr_internal.rs`,
     all **#325's**.
   - Expected: 43 -> 20 mip kicks, -23 kicks x 1.1 ms = about -25 ms.
   - Pictures: at risk. A level made from one two or three levels up
     rounds once, not at each step, and on the NPOT chain's odd steps
     (75 -> 37 and after) the direct sample is not the iterated one. Levels
     4.. of terrain are 50x37 and smaller, so the change is expected within
     the column's tolerance, but only the board can say. ES 2.0: dEQP-GLES2's
     `texture.mipmap.2d.generate.*` take the GPU chain on the board for
     RGBA8; each is a threshold compare, and the whole set must be rerun.
   - Confirms it: gl_test case 216's `per frame: N kicks` (40 now: its
     four chains' 9 + 8 + 9 + 9 levels and 5 passes; 4 + 4 + 4 + 4 + 5 = 21
     after) and `mipchains: ... ms a frame` (46.15 now), its four quadrant
     probes; glmark2 terrain's `FPS:` and probes against the column.
3. **Path B again, once a waited kick costs what a queued one does.** The
   code is there (`Backend::mip_cpu_tail`, off). After (1), a prefix's wait
   should cost its 4 kicks (about 4.4 ms), not 15.4. The tail must also stop
   costing 7.9 ms: today it reads the carve-out, Normal non-cacheable, at
   DRAM latency per access; a cacheable CPU view of the texture's pages for
   the tail (invalidate, filter, clean), or wide loads of the few KB into a
   local buffer before filtering.
   - Files: `src/pvr/backend.rs` (`mip_cpu_tail`, `mip_chain_gpu`), the
     tail's reads in `src/gl/texture.rs` (`mip_step_gpu_sample`), the pool's
     mapping (`driver/src/pvr_fw_boot.c:587-595`,
     `driver/rust/src/gpu/pvr/mem.rs`): **#325's** (backend, mem.rs) and
     **#301's** (src/pvr).
   - Expected: 43 -> 11 mip kicks (the 800x600 chains' levels 1-3, the
     256x256 chains' level 1), -32 kicks x 1.1 ms = about -35 ms, less the
     five tails (under 10 KB each). Measured today: +40 ms (83 -> 124), the
     waits for the last kick and the uncached tail. So only after (1).
   - Pictures: the tail is the GPU's sample on the CPU (the probes and the
     mean were the same, `efa0170f`). ES 2.0: the dEQP-GLES2 generate
     cases go through the tail on the board; rerun them.
   - Confirms it: case 216 with the tail on: 15 kicks a frame and ms a
     frame under 46.15 (82.95 now); the measurement build's per-prefix time
     (15.39 ms now) near 4.4 and the tail's (7.87 ms now) under 1.
4. **One submit for a chain (experiment first).** If the ~0.9 ms is the
   firmware's take-up of each KCCB command rather than of each render,
   posting a chain's renders behind one command amortises it. It needs the
   renders of one size to reuse HWRT members in order (a geometry fence on
   the fragment UFO of the render that last used the member), a change in
   `render.rs` `submit` (**#325's**; **#301** for the partial-render words
   in it).
   - The experiment that decides it, no product code: a gl_test case that
     posts two renders back to back against one, with the kick times on;
     post -> geometry fence of the second against the first.
   - Expected: between 0 (the price is a render's) and about -40 ms (43
     level kicks down to one take-up a chain).
   - Pictures and ES 2.0: none (the same renders).
   - Confirms it: the `pvr last completed kick per frame` line's
     `kick->geom` and `frag->frag`, case 216's ms a frame.
5. **The window's dead clear kick.** Keep a framebuffer's full pending
   clear pending across a bind to another framebuffer (it is emitted only
   when the framebuffer is drawn into, read, copied from, or swapped), and
   drop it when a later full clear of the same buffers replaces it.
   - Files: `src/gl/draw.rs` (`settle_pending_clear`, `:2523-2540`, and its
     callers), **#325's** and **#310's**.
   - Expected: 52 -> 51 kicks, -1.1 to -1.8 ms (an 800x600 kick that also
     stores the window's depth/stencil); every glmark2 scene that clears the
     window and then renders into a framebuffer first gains the same.
   - Pictures: none if every reader still emits it first: the read paths do
     today (`src/gl/framebuffer.rs:1741`, `:2926`, `:3642`); the swap, and
     the sampling of a texture attached to a framebuffer whose clear is
     pending, would need the same. ES 2.0: the clear
     is deferred, not dropped, for any read; dEQP-GLES2's clear and
     read-back cases are the check.
   - Confirms it: the host's `kicks.py` (52 -> 51); the board's terrain
     `FPS:` and its probes.
6. **Each level's own render area.** Walk only the level's tiles (a TE/ISP
   screen region smaller than the HWRT's), keeping one HWRT set a chain:
   13,849 tiles a frame of mip kicks -> 2,158.
   - Files: `src/pvr/backend.rs` (`record_mip_level` `:6150-6152`, the
     frame's TE and ISP size in `flush`), **#325's**.
   - Expected: about -3 ms of GPU time a frame (11,691 tiles x 0.26 us);
     the frame gains only the part in series with the kick's take-up,
     0 to -3 ms. A HWRT set per level size instead would bring back the
     pair recycling (3.7 s a frame on 2026-10-16, `record_mip_level`'s doc).
   - Pictures: none (the tiles past the level store nothing today). ES 2.0:
     none.
   - Confirms it: case 216's `TA busy`, `3D busy` and `TA|3D` a frame (7.75
     ms now) down, then its ms a frame.
7. **The 20 us poll or an interrupt instead of the 1 ms poll.** Measured
   already for terrain: 83.08 -> 81.67 ms with the 20 us poll (`ce3354ac`);
   an interrupt gains at most that, and lets the CPU sleep.
   - Files: `render.rs` (`WAIT_POLL_US`, feature `pvr-fine-wait`),
     `driver/src/pvr_bringup.c` for the interrupt (`pvr-fence-wait.md`,
     "What (b) needs"), **#325's** (render.rs).
   - Expected: -1.4 ms. Pictures and ES 2.0: none.
   - Confirms it: terrain's ms a frame, `pvr last completed kick per frame:
     ... wait` bounded within 20 us.
8. **The transfer queue (path A).** One TQ job a level, or one submit for
   the chain. Unknown whether a transfer job costs less than a render's
   ~1.1 ms, and the most code: a transfer context, its CCB and programs, a
   fence each way (`am62p-pvr-mipmap-kicks.md`, "The paths").
   - Files: `driver/rust/src/gpu/pvr/**`, `driver/src/pvr_render.*`,
     `driver/src/pvr_usc_eot_blob.h`, `src/pvr/backend.rs`: all **#325's**.
   - Expected: unknown until one TQ blit is timed against a kick.
   - Confirms it: the TQ job's post -> fence time against `kick->frag`.

Not candidates:

- **No depth/stencil load or store for mip kicks**: already so (above).
- **Logging**: nothing on the frame path to cut.
- **The CPU's GL work**: 2 to 3 ms a frame on the board, under 4% of it.

## Counters a board run needs

- **The re-post rate.** The GM record already has the re-post lines with
  their running count (`render.rs:2371-2381`): the count at the end over
  the frames run gives re-posts a frame. To know which wait meets it, the
  line needs the backend's call site (frame set, swap, finish) and the last
  posted fence beside the waited one: a change to `wait_fence`'s callers
  or to the OSAL's wait name (`pvr_render.c:783`), library and driver code,
  so not built here.
- **gl_test case 216's re-posts**: the case prints its kicks a frame and the
  TA/3D counters; the delta of `pvr_render_fl_counters` word 6 (`reposts`,
  `render.rs:2237-2249`) over its 20 frames would tell the two readings of
  its 46 ms apart (`driver/tools/pvrlab/gl_test.c`, case 216).
- **Kicks a frame in the glmark2 image**: the board's 52.7 came from the
  kick-times build; a `glmark2-frame:` field with `pvr_backend_counters`'
  kicks and waits a scene would give kicks and waits a frame in every GM
  record (`driver/samples/zephyr/glmark2/src/main.cpp`).

## Open

- What the firmware does for ~0.9 ms between the post of a queued kick and
  its geometry fence (`ce3354ac`: post to geometry done ~1 ms a kick with
  the 20 us poll), when an idle 64x64 kick takes 120-140 us.
- Which wait meets the once-a-frame 20 ms fence, and whether it is still
  there at `6a7021ec` (the record of `ce3354ac` saw it; the `6a7021ec` row
  does not say).
