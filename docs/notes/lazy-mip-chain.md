<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2 terrain: which glGenerateMipmap chains a lazy (deferred) build skips exactly

Study, no code change (plan #329, item `lazy-mip-chain-study`; #1209 for the
Pi 4, #1199 for the AM62P). Written 2026-11-21 at `975a6792`. `src/` is the
same at `origin/main` `50570955`: `git diff --stat 975a6792 50570955` lists
docs, `tools/cts/x2-*`, `tools/probes/` and a FINDINGS file only. Every
line number below is that tree's. glmark2 is `22c527cb`, the harness pin.
Nothing here ran on a board. Board figures come from the cited notes, and
every gain in ms is a **prediction**.

## Answer

1. **Deferral alone skips none of terrain's 5 chains a frame.** Each chain
   is sampled with `GL_LINEAR_MIPMAP_LINEAR` by the very next draw that
   reads the texture, before any other use of it. So under the exact rule
   below, every chain is built, only a few calls later than now. Measured:
   a replay of the host trace (below) gives 5 built and 0 skipped a frame,
   on the V3D and the PowerVR hosts alike.
2. **Deferral plus a per-draw magnification proof skips exactly one chain
   a frame**: chain [3], bloom V's 256x256 `tex9`. Its only reader is the
   overlay draw, which magnifies it everywhere (rho at most 256/600 = 0.43
   texel/px, lambda at most -1.23; c = 0 for these filters). A magnified
   sample reads level 0 only. The next frame's `glGenerateMipmap(tex9)`
   then drops the unbuilt chain. Measured replay: 4 built and 1 skipped a
   frame.
   - **AM62P host: 51 to 43 kicks a frame** (-8, the 8 level kicks of a
     256x256 chain).
   - **Pi 4 host: 5.00 to 4.00 TFU jobs a frame**. On the board that is the
     bloom chain's 0.44 MB, about **0.10 ms** of the 2.64 ms of TFU
     (`v3d-terrain-gpu.md`).
3. **The other four chains are needed.** Chain [1] is read at LOD 1.64.
   Chains [2], [4] and [5] are 1:1 passes whose blur taps skew lambda above
   0, by 0.0001 to 0.019. Skipping them is the inexact candidate 1/1b of
   `terrain-mip-usage.md`, not a lazy build. `v3d-terrain-gpu.md`
   candidate 1 calls the terrain target's second build (chain [4], lambda
   up to 0.00013) an "exact skip". Under ES 2.0 it is not: its level-1
   weight is small but not 0, and only a per-GPU proof that the LOD unit
   rounds it to 0 would make it exact.
4. **The set-up has 5 exact skips, once.** These are the `glGenerateMipmap`
   calls on the five render targets made with no data. Deferral alone skips
   them **only if a render into level 0 forces the build when its job is
   submitted, not when it is recorded**. That is about 43 PowerVR level
   kicks (or 5 V3D CPU box filters) once per scene start. It is not a
   per-frame value.
5. **Rank: below every open candidate for the AM62P except as an add-on.**
   It is worth -8 kicks, with a risk of rule changes in 4 files held by
   other plans. On the Pi 4 it is worth 0.1 ms, under the noise. The
   magnification proof it needs is the same compiler pass as
   `terrain-mip-usage.md` candidate 2 ("skip the chain of a texture only
   ever magnified"). This study finds that candidate 2 **has to be built as
   a lazy chain** (rule S below) to be exact. A skip decided at the
   `glGenerateMipmap` call cannot know the next draw's state.

## The host runs and their outputs

All outputs are under `$TMPDIR/lazy/` of this item (not committed).

```sh
# kicks a frame, PowerVR host (tools/hostharness/am62p-terrain, GM2_KICKS=1) and profile
bash tools/hostharness/am62p-terrain/run.sh "$GLMARK2_SRC" "$TMPDIR/lazy/am62p" kicks profile
# TFU jobs a frame, V3D host with the TFU model (tools/hostharness/pi4-terrain)
PI4T_RUNS=prof-tfu1 tools/hostharness/pi4-terrain/run.sh "$GLMARK2_SRC" "$TMPDIR/lazy/pi4"
# per-call trace: a COPY of the tree (git archive HEAD) with terrain-mip-usage.md's
# GM2_TRACE patch on its shim.c, extended with the kick id at each call,
# glCopyTex*, glDeleteTextures and glBindFramebuffer; 5 frames, both hosts
GM2_WRITE=0 GM2_TRACE=1 GM2_KICKS=1 GM2_FRAMES=5 GM2_ENTRIES='^terrain$' YAKOGL_V3D_TFU=1 \
  bash <copy>/tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" "$TMPDIR/lazy/trace" powervr v3d
# replay of the trace under the three rules (scratch script, $TMPDIR/lazy_sim.py)
python3 -I lazy_sim.py logs/<gpu>/0.log \
  --units "2:,4:0,6:0,8:0/1/2/3/4/5,10:0,12:0,14:0,16:0,18:0" --magnified "4:14"
```

`$GLMARK2_SRC` was `$CONNECTED_DEPS_DIR/glmark2-src`. The `--units` list
gives the units each program samples. It is from the glmark2 sources: the
blur, overlay and tilt-shift programs sample unit 0, and terrain samples
0..5. The GL core reads the same from the program's sampler uniforms
(`sh.sampler_bindings()`, `src/gl/draw.rs:4800`). It matters:
`BaseRenderer::update_mipmap` binds the target on the *active* unit, so
`tex7` stays bound to unit 5 through bloom H, bloom V, overlay and
tilt-shift, where no program samples it. A lazy build that forced on
"bound" rather than "sampled" would be correct but would count wrongly.
`--magnified 4:14` stands in for the proof: the overlay draw (fb 4,
program 14).

Results:

| run | figure | value |
|---|---|---|
| `am62p/kicks/kicks.txt` | kicks submitted a frame, frames 1..18 | **51.00** (8 draws; per draw `fb1+1 fb2+1 fb4+1 fb5+10 fb6+9 fb4+9 fb7+10 fb0+10`: chain kicks 9 + 8 + 8 + 9 + 9 = **43**, pass kicks 8). This is the pin of `tests/pvr_terrain_kicks.rs` (51 after the dead-clear fix) |
| `am62p/prof/profile-powervr.md` | `glGenerateMipmap` calls a frame; host wall | 5.0; 0.138 ms (host recording only, no GPU wait in it). 14 calls in set-up and the first frame |
| `pi4/prof-tfu1.summary.txt` | TFU model jobs a frame; V3D jobs; CPU waits | **5.00**; 8.00; 8.00. `glGenerateMipmap` 5.00 calls with 5.00 TFU jobs. Frame 1: 1 chain made by the TFU, `refused: unknown 13` (the set-up chains on targets not rendered yet go to the CPU filter) |
| `trace/logs/powervr/0.log` | kick ids across each `genmip` (steady frame 2) | `tex7` 188->198, `tex8` 198->207, `tex9` 207->216, `tex7` 216->226, `tex10` 226->236. A step is the chain's levels plus 1 when the open pass is submitted by the call. Set-up: `tex3` 2->12, `tex7` 39->48, `tex8` 48->56, `tex9` 56->64, `tex10` 64->73 on images with no data |
| replay, rule R | chains built / skipped a frame (frames 1..4) | **5 / 0** both hosts; set-up and frame 0: 14 / 0 |
| replay, rule S | same | **5 / 0** both hosts; set-up and frame 0: **9 / 5** (the five no-data targets) |
| replay, rule M | same | **4 / 1** both hosts (chain [3] `tex9`, every frame); set-up and frame 0: 8 / 6 |

## The per-chain table (one steady frame)

The order is the trace's `genmip` order. "Next use" is the first event on
the texture after the call, from the trace. Every texture is RGBA8, mag
`GL_LINEAR`, min `GL_LINEAR_MIPMAP_LINEAR` (`texparam 0x2801 0x2703`), so
c = 0.

| # | texture | size, kicks / TFU | `glGenerateMipmap` while | next use (trace) | lambda over that draw | verdict | rule that forces it |
|---|---|---|---|---|---|---|---|
| 1 | `tex7` terrain target, first build | 800x600, 9 kicks / 1 TFU (3.63 MB, 0.81 ms) | fb 4 bound, terrain pass open | bloom H draw (fb 5, prog 10) samples it | 1.64 (rho 3.125) | **needed** | a mipmap-filtered sample |
| 2 | `tex8` bloom H | 256x256, 8 / 1 (0.44 MB, 0.10 ms) | fb 5 bound | bloom V draw (fb 6, prog 12) | 0 to 0.0113 | **needed** (level-1 weight up to 1.1 %) | a mipmap-filtered sample |
| 3 | `tex9` bloom V | 256x256, 8 / 1 (0.44 MB, 0.10 ms) | fb 6 bound | overlay draw (fb 4, prog 14); then next frame's bloom V clear and draw into fb 6, then `genmip tex9` | **at most -1.23** (rho = max(256/800, 256/600) = 0.427; `terrain-mip-usage.md` row 3 gave -1.64 from the x axis alone) | **skipped exactly** with the magnification proof under rule S; **needed** without it | none (M); a mipmap-filtered sample (R, S) |
| 4 | `tex7` terrain target, second build | 800x600, 9 / 1 (0.81 ms) | fb 4 bound, overlay pass open | tilt-shift H draw (fb 7, prog 16) | 0 to 0.00013 | **needed** (not exact to skip: weight up to 0.013 %) | a mipmap-filtered sample |
| 5 | `tex10` tilt-shift H | 800x600, 9 / 1 (0.81 ms) | fb 7 bound | tilt-shift V draw (fb 0, prog 18) | 0 to 0.0191 | **needed** (weight up to 1.9 %) | a mipmap-filtered sample |
| s | set-up: `tex3`, `tex7`, `tex8`, `tex9`, `tex10` | 9, 9, 8, 8, 9 kicks | `create_texture` -> `update_texture_parameters` -> `update_mipmap`, with no data (`glTexImage2D(..., 0)`, `base-renderer.cpp:111-139`) | a clear and a draw into the target's fb, then that target's next `genmip`, before the fb is left | - | **skipped exactly under rule S** (once); **needed** under R | the render into level 0, if it forces at record time |
| - | `tex4`, `tex5`, `tex6` (JPEG), `tex3` after its render | 512x512 | after the upload / render | the specular render and the terrain draw sample them | up to ~9 | **needed** (once) | a mipmap-filtered sample |

Chain [3] is unsafe in one way of building it, and it shows why the rule
must be S. The overlay is the only reader, and it reads level 0. The next
event on `tex9` is a *write* to level 0: the next frame's bloom V clear and
draw into fb 6. If that write forced the build when recorded (rule R), the
chain would be built from the old level 0 on the chance a later draw reads
it. Only at the next `genmip tex9` is it known that nothing did. In yakogl
both backends submit the fb 6 job later than that call. The V3D submits
when a draw goes to another target (`draw_call`, `src/v3d/backend.rs:6863`),
here the overlay's first draw into fb 4. The PowerVR submits at the
framebuffer switch. So with rule S the stale chain is dropped before its
source is overwritten on the GPU.

## The exactness rule (ES 2.0)

A chain made by a call at time t0 holds levels 1.. as filtered from level 0
**as it was at t0** (ES 2.0.25 3.7.11; from memory, not checked against the
text here). A deferred build is exact iff it produces those bytes and
nothing observes levels 1.. before it is built. The texture's completeness
and level records stay what they are now: `level_defined`, `levels`,
`levels_written` are set at the call (`src/gl/texture.rs:4865-4873`), so
no query and no incomplete-texture decision moves. Errors are also decided
at the call (`:4727-4786`).

**Pending** from the call until built or dropped.

**Forced (built, from the t0 contents, before the event)** by:

1. A draw that **samples** the texture through a sampler of its program,
   in any stage, where the min filter in force is a mipmap filter (the
   sampler object's in ES 3.0, `src/gl/draw.rs:4976-4980`), the texture
   has levels > 1 (`sampling_fields`, `:694`; `tb.mip_filter != 0 &&
   tb.mip_levels > 1`), and lambda can exceed c somewhere in the draw.
   "Can exceed" is true unless proven otherwise. `texture2DLod`,
   `texture2DProjLod`, `textureCubeLod` (vertex shader) and a `bias`
   argument (fragment shader) count as able to exceed. Rule M's proof is
   the only exception.
2. The **submit** of a GPU job that writes the texture's level 0 (a draw or
   clear into a framebuffer it is attached to), when the chain was made
   before that write was recorded. This means every submit: the target
   switch, `glFlush`/`glFinish`/swap, `glReadPixels`, and also the
   **backend's own early submits** of the same target. Those are a job
   full (`draw_in_job`, `src/v3d/backend.rs:6153`), no memory (`:6173`), a
   TLB clear after draws (`:6900`), CPU depth texels (`:6874`), the
   PowerVR's kick ring and frame-set waits. This is why the deferral
   belongs **in the backend**: only the backend sees all of them.
3. Any **CPU read or write of any level**: `glTexImage2D`,
   `glTexSubImage2D`, `glCopyTexImage2D`, `glCopyTexSubImage2D`,
   `glCompressedTex(Sub)Image2D` of level 0 or of a level > 0; the GL
   core's own CPU readers (`settle_texture` -> `chain_back`,
   `src/gl/texture.rs:889-933`, the native cube/array/3D copies of the
   PowerVR, `src/gl/draw.rs:4920-5027`, the depth resolve).
   `return_mipmap_chain` (`src/backend.rs:2402`) is already the hook every
   one of these passes through: it must build and then wait.
4. Attaching **a level > 0** of the texture to a framebuffer.
   `glFramebufferTexture2D` takes any level in yakogl, also in an ES 2.0
   context (`src/gl/framebuffer.rs:2073`; OES_fbo_render_mipmap
   behaviour). Rendering into, `glReadPixels` from or `glCopyTex*` from
   such a level reads or writes levels 1... Today
   `level_above_zero_attached` (`src/gl/framebuffer.rs:2102`) keeps such a
   chain in the GL core. The lazy path must force at the attach and keep
   that refusal.

**Dropped (skipped exactly)** by:

5. The texture's **next `glGenerateMipmap`** with nothing of 1-4 in
   between. Writes to level 0 recorded but not yet submitted are allowed;
   the new chain is then made from the level 0 those writes leave.
6. `glDeleteTextures`, or storage made anew (`storage_made_anew`,
   `src/gl/texture.rs:2033`; its use at `:2084` already clears
   `chain_in_backend`).

**Not a drop:** `glTexImage2D` of level 0 at the same size and format.
The old levels 1.. stay, and the texture is complete again with them, so
it forces (case 3). Nor does changing the min filter to a non-mipmap one
and back drop anything. The chain stays pending, and its source must still
be the t0 level 0: case 2 or 3 forces it before a write.

## The cases that make it unsafe, and what dEQP-GLES2 covers

| case | why it breaks a naive lazy build | where yakogl stands | dEQP-GLES2 cover (`tools/cts/gles2-mustpass.txt`) |
|---|---|---|---|
| Sampling with a mipmap filter right after the call (the normal case) | must build before the draw's job runs | the eager path today | `functional.texture.mipmap.2d.generate.*` (32 cases), `.cube.generate.*` (16), `functional.texture.mipmap.2d.{basic,affine,projected,bias}`-style groups (84 in `mipmap.2d`, 28 in `mipmap.cube`), `functional.texture.filtering.2d/cube` (136 / 112), `functional.texture.vertex.2d/cube.filtering.*_mipmap_*` (vertex fetch, 45 / 45) |
| Explicit LOD in a shader (`texture2DLod` in the vertex shader, `bias` in the fragment shader) | lambda is not the derivative's: a magnification proof from the coordinate alone is wrong | none today (no proof exists) | `functional.shaders.texture_functions.vertex.texture2dlod`, `.texture2dprojlod_vec3/4`, `.texturecubelod`, `.fragment.texture2d_bias`, `.texture2dproj_vec3/4_bias`, `.texturecube_bias` (17) |
| Render into level 0 between the call and the first sample (terrain's overlay pattern, sampled deeper later without a new call) | the chain must come from the level 0 *before* the render | eager today. A lazy build must force at that job's submit (rule 2), including the backend's early submits | **not covered**. `functional.fbo.render.*` renders into textures but calls no `glGenerateMipmap` (`fbo.render.texsubimage.*`, 4 cases, is the nearest: TexSubImage between renders) |
| `glTexSubImage2D` / `glCopyTexSubImage2D` on a level > 0 between the call and the use | the write lands on unbuilt levels, and the later build would overwrite it | `settle_texture` -> `chain_back` comes first today | **not covered** with a generate in between. `functional.texture.specification.basic_texsubimage2d.*` (26), `basic_copytexsubimage2d.*` (10), `texsubimage2d_align.*` (40), `texsubimage2d_empty_tex.*` (26) cover the write paths alone |
| `glTexImage2D` of level 0 at the same size after the call | old levels stay valid. Treating it as a drop would sample a wrong chain | storage kept, levels kept | **not covered** with a generate. `functional.texture.specification.basic_teximage2d.*` (26), `random_teximage2d.*` (20), `teximage2d_align.*` (34), `functional.texture.completeness.2d.*` (19: `extra_level`, `missing_level_1`, ...), `.cube.*` (21) cover level records |
| A level > 0 attached to a framebuffer (render into it, `glReadPixels`, `glCopyTex*` from it) | the attachment reads and writes levels 1.. directly | `level_above_zero_attached` keeps the chain in the GL core | none in GLES2 (the core spec rejects level != 0; yakogl accepts it). `functional.fbo.completeness.*` attaches level 0 only |
| Share group: a second context renders into, or samples, the texture | textures are per group (`GlObjects`), framebuffers per context (`src/gl/context.rs:905-911`). `flush_clear_into` (`src/gl/texture.rs:874`) and `level_above_zero_attached` look at the **current** context's framebuffers only | yakogl is single-threaded by contract, so GL call order is the order. But the forced-at-submit rule must see another context's jobs: a backend-side pending chain sees them, a GL-core one does not | **not in dEQP-GLES2**. dEQP-EGL `functional.sharing.gles2.*` is the cover, and this tree does not run the EGL module (no `dEQP-EGL` list in `tools/cts/`) |
| An EGLImage made from the texture | another client reads level 0 only (`EGL_GL_TEXTURE_LEVEL` 0 is the behaviour, `src/gl/egl.rs:2763`) | `t.external` textures are not filtered (`src/gl/texture.rs:4786`) | dEQP-EGL `functional.image.*`, not run here |
| Debug dumps of a bound texture (`dump_bound_texture`, `src/gl/draw.rs:5060`) | a CPU read of levels | must force (case 3) or read level 0 only | none |

Summary for the gate: the ES 2.0 answers guarded by
`tests/es2_report_guard.rs` and the dEQP-GLES2 tree exercise **only the
first two rows**. The order-dependent rows (render, sub-image, same-size
re-spec between generate and sample) are not covered by any dEQP-GLES2
group. A fix needs its own host test per row: one GL call sequence each,
with the levels' bytes compared against the eager path on the V3D and
PowerVR doubles of `tests/v3d_mip_chain_pin.rs` and
`tests/pvr_mip_chain_pin.rs`.

## The gain, counted

| board | baseline (host, this tip) | rule R or S | rule M (magnification proof + S) | board time (prediction) |
|---|---|---|---|---|
| AM62P, kicks a frame | **51** (`kicks.py`; pin `tests/pvr_terrain_kicks.rs`) | 51 (0) | **43** (-8) | -8 queued kicks. At 0.65 to 1.15 ms a queued render (`kickpair` range of `terrain-board-readout.md`) that is **-5.2 to -9.2 ms**; at the 1.1 ms a kick of `am62p-slow-terrain.md`, -8.8 ms. A 256x256 level kick is small (each level kick walks its own tiles, `pvr-mip-level-area`), so the fixed cost a kick is the whole saving |
| AM62P, set-up | 14 `genmip` calls | S: 9 chains built, 5 dropped (about 43 kicks once) | 8 built | one-off, not in FPS |
| Pi 4, TFU jobs a frame | **5.00** (host TFU model) | 5.00 | **4.00** | -0.44 MB of TFU traffic, **-0.10 ms** of the 2.64 ms (4.47 MB/ms, `v3d-terrain-gpu.md`). V3D render jobs (8) and CPU waits (8) do not change |
| Pi 4, set-up | 1 TFU chain and 13 CPU-filter refusals at frame 1 | S: the 5 no-data targets' CPU box filters go | same | one-off |

Side effect, not counted above. If the GL core binds a proven-magnified
draw with `mip_filter = 0`, then the overlay (V3D job 6) no longer asks the
TMU for a second level. That is exact under c = 0: magnified,
`LINEAR_MIPMAP_LINEAR` and `LINEAR` read the same level-0 texels.
`v3d-terrain-gpu.md` gives that 0.5 ms or less, unproven. It is also the
natural way to tell the backend about the proof: the backend's lazy rule
reads only `tb.mip_filter`, and needs no new field.

## Rank against the existing candidates

AM62P (`am62p-slow-terrain.md`, `pvr-mip-fewer-kicks.md`,
`terrain-board-readout.md`):

| rank | candidate | kicks a frame (from 51) | exact | state |
|---|---|---|---|---|
| 1 | three levels a render (`pvr-mip-three-levels`) | about 21-24 | no (picture error measured in `mip-multilevel-error`) | implementation item ready, held by plan 388's review |
| 2 | E1 tail route (`pvr-mip-tail-e1.md`) | about 32 | yes (the GPU's filter on the CPU) | board item waiting |
| 3 | only the reachable levels (`terrain-mip-usage.md` 1/1b) | 13-14 | no (up to 4.9 LSB) | needs the same LOD proof plus more |
| 4 | one submit a chain (candidate 4, `pvr-chain-one-submit-study`) | kicks the same, submits fewer | yes | study running in parallel |
| **5** | **this: lazy chain + magnification proof (rule M)** | **43 (-8)** | **yes** | adds to 1-4: it removes a whole chain, which the others make cheaper |

Pi 4 (`v3d-terrain-gpu.md`): below candidates 1 (blur passes read one
level, up to 8.6 ms of samples), 3 (dead window clear, done) and the early-Z
finding. It is the exact part (0.10 ms) of candidate 2 (TFU bytes,
0.1 to 1.8 ms). Not worth a code path on the Pi 4 alone.

## What a fix would touch, and who holds it

The design that keeps the rule exact is a **pending chain in the backend**.
`generate_mipmap` answers `Whole` (1) and records the chain without
building it. The backend builds it before (a) binding the image in a draw
with `tb.mip_filter != 0 && tb.mip_levels > 1`, (b) submitting any job
or kick that writes the image while the chain predates the write, or (c)
`return_mipmap_chain`. It drops it at the next `generate_mipmap` of the
same image. The GL core only supplies the magnification proof.

| file | change | held by |
|---|---|---|
| `src/v3d/backend.rs` | pending TFU chain on the `RtImage` (`generate_mipmap`, `:2426`; build at the bind of a sampled image and before `flush_job`, `:2935`, of a job whose `rt` is the image; `rt_chain_back`, `:2123`; `v3d_return_mipmap_chain`, `:2559`) | #308 (`src/v3d/**`) |
| `src/pvr/backend.rs` | pending chain (`pvr_generate_mipmap`, `:7356`; `mip_chain_gpu`, `:7282`, run at the force point; `pvr_return_mipmap_chain`, `:7405`; force before the kick of a render into the target) | #325, #301 |
| `src/gl/draw.rs` | the per-draw proof: `tb.mip_filter = 0` for a sampler whose coordinate is proven to have rho < 1 over the draw (`:4980`) | #325, #310 |
| `src/shaderc` (reflection, e.g. `src/shaderc/online/reflect.rs`, `link.rs`) | classify "`texture2D` coordinate = an affine function of a varying that is an affine function of an attribute, `gl_Position` a pass-through, no `Lod`/`bias`" (`terrain-mip-usage.md`, "Candidate 1, the bound") | #325 |
| `src/gl/texture.rs` | none for rule S in the backend. A GL-core-side variant would change `glGenerateMipmap` (`:4722`) and every CPU path, and cannot see the backend's early submits, so it is not recommended | #310 |
| `src/backend.rs`, `src/backend.h` | the `generate_mipmap` doc comment: "Whole" may now mean "pending". The ABI is the same | #325, #310 |
| `tests/pvr_terrain_kicks.rs` | pin 51 -> 43 in the same commit | - |
| `tests/v3d_terrain_jobs.rs` | TFU chains a frame 5 -> 4 | - |
| new `tests/lazy_mip_*.rs` | one host test per row of the unsafe-case table, on both doubles | - |
| `tests/es2_report_guard.rs` | read only: every ES 2.0 answer must stay the same | plan-c3-recheck2 (running) |

Order: after #308, #325, #310 and #301 release these files, small and
rebased (the plan's rule). The proof (draw.rs plus shaderc) comes first,
because it is useful alone for the V3D's TMU fetch. The backend pending
chain comes second.

## The board values that would confirm it

From images built from an `origin/main` commit with the fix, against the
same commit without it:

- **AM62P**: the `glmark2-counters: terrain...: frames N ... kicks K`
  line (`driver/samples/zephyr/glmark2/src/main.cpp:1336`,
  `glmark2-scene-counters.md`), with K/N going from about 51.7 to about
  **43.7** (the board counts about +0.7 of set-up). Also the `pvr last
  completed kick per frame ... kicks K over N frames` line where the
  kick-times build prints it. Terrain's `FrameTime` should fall by 5 to
  9 ms. The `glmark2-picture:` probes must be **unchanged** against column
  `am62p` of `tools/cts/glmark2-pictures.txt`: the rule is exact, so the
  picture must not move by even 1 LSB beyond the animation, unlike
  candidates 1 and 3. 0 GL errors, `verdict=done`, 0 resets.
- **Pi 4**: the same line's `tfu.jobs` / `rt.chains` over `frames`,
  **5 to 4** a frame. In `yakogl/v3d: TFU mipmap at frame N: C chains`
  (`src/v3d/backend.rs:5543`), C grows 4 a frame, not 5. The `glmark2-jobs`
  entry of job 5 (bloom V) loses its `tfu 1/100us`. `jobs` and `waits`
  stay 8 a frame. `FrameTime` should move by about 0.1 ms (expected under
  the spread). The picture is unchanged.
- **Both, set-up**: the scene's first-frame counters (or the host's
  `GM2_PROFILE` "Setup and first frame") show the 5 no-data chains gone.
  This is not a board verdict.

## What this study does not decide

- Whether the TMU and the TPU round the 1:1 passes' LOD (lambda < 0.02) to
  exactly 0. If they do, chains [2], [4] and [5] are also exact skips on
  that GPU (`terrain-mip-usage.md`, "What this study does not decide"). The
  gl_test case of `gltest-lod-time` times LINEAR against
  LINEAR_MIPMAP_LINEAR. It does not compare the bytes.
- The cost of a forced build *at a draw* instead of at the call on the
  PowerVR. The chain's kicks are then recorded while the sampling pass's
  clear is parked. `pvr_generate_mipmap` flushes (`mip_chain_gpu`, the
  `flush(p)` after the levels), so the parked clear must go into the
  sampling pass's kick, not a kick of its own. A host test with
  `tests/pvr_terrain_kicks.rs` would show a +1 if it did not.
- Other scenes. Refract mipmaps its 1600x1200 target once a frame and
  samples it with a refraction offset, so no magnification proof applies.
