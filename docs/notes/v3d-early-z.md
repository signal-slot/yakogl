<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D early Z: when it may be on, what yakogl does, what it would save

Plan #329, item `v3d-early-z-study` (Pi 4; #1209). A study: no code was
changed and no board was run. `v3d-terrain-gpu.md` ("Not asked, found: early
Z for the terrain draw") found that every V3D job sets `early_z_disable`, and
put the terrain draw's share at "about 1.7 ms" of the ~29 ms GPU floor. This
note gives the rules, today's state, the smallest change, the gain per scene
with the board value that confirms it, and the risks.

**Short answer.** Early Z is legal for glmark2 terrain's job 3 (depth test
LEQUAL, depth write on, no `discard`, no `gl_FragDepth`, no stencil). The
change is small and local to `driver/src/v3d/v3d_job.[ch]` and
`src/v3d/state.rs`. The gain is `4.3 ms x (overdraw - 1)` on job 3, **0 to
about 1.7 ms**, and nobody has measured the overdraw. The 1.7 ms of
`v3d-terrain-gpu.md` is the top of that range (overdraw 1.4); at the 1.3 that
note assumes, the arithmetic gives 1.3 ms (below). It is a small, low-risk
item, below candidates 1 to 3 of that note.

## What could not be read in the tree

`third_party/mesa-26.2.1` holds the packet XML (`src/broadcom/cle/`), the
compiler pieces and the format tables, **not** the Mesa v3d driver sources
(`src/gallium/drivers/v3d/` has only `v3d_format_table.h`,
`v3dx_format_table.[ch]`). `v3d_state.c`, `v3dx_draw.c`, `v3dx_rcl.c` and
`v3d_program.c` are therefore not in the tree. What follows about them comes
from yakogl's own citations of them (`v3d_frame.c:185-188`,
`v3d_job.c:545`, `src/shaderc/online/v3d/select.rs:2255-2282`) and from my
recollection of Mesa. **Every Mesa rule marked (recalled) must be checked
against the `mesa-26.2.1` tarball (`third_party/mesa-26.2.1/UPSTREAM`) before
the code is written.** What the XML fixes is cited with its line.

## (a) The rules

### The three places early Z is controlled (4.2)

All three are in `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml`:

| where | field | line | scope |
|---|---|---|---|
| `Tile Rendering Mode Cfg (Common)` (RCL) | `Early-Z disable`, `Early-Z Test and Update Direction` (0 = LT/LE, 1 = GT/GE) | 1002-1007 (and 1044-1049, the second variant) | **job**: one switch and one direction for every draw of the tile |
| `CFG_BITS` | `Early Z enable`, `Early Z updates enable` | 836-837 | **draw** |
| `GL Shader State Record` | `Turn off early-z test`, `Fragment shader does Z writes` | 1208-1209 (the 4.2 record, `max_ver="42"`) | **draw**, per fragment shader |

A fragment is tested early only if the job's switch is on (RCL), the draw's
`Early Z enable` is set (CFG_BITS) and the shader record does not turn it
off. Otherwise it is tested late, after the shader (the TLB's Z/S test).
The hardware early-tests one direction a tile, which is why the RCL carries
a direction and why a job cannot mix `LESS/LEQUAL` with `GREATER/GEQUAL` draws.

### Mesa's decision (recalled; v3dx_draw.c `v3d_update_job_ez`, v3d_state.c)

yakogl cites `v3d_update_job_ez` at `v3dx_draw.c:941-945` and `emit_rcl` at
`v3dx_rcl.c:730-747` (`v3d_frame.c:185-187`) for the no-depth-buffer case.

1. **Per depth/stencil state** (`ez_state` of the zsa object, set when the
   state is created): depth func LESS, LEQUAL: `V3D_EZ_LT_LE`; GREATER,
   GEQUAL: `V3D_EZ_GT_GE`; NEVER, EQUAL: `V3D_EZ_UNDECIDED` (goes along with
   whatever direction the job has); ALWAYS, NOTEQUAL: `V3D_EZ_DISABLED`.
   Stencil enabled with an op that changes the buffer on depth fail
   (`zfail` not KEEP) turns it to `DISABLED`.
2. **Per draw, into the job** (`v3d_update_job_ez`): `UNDECIDED` keeps the
   job's state; a direction sets the job's if undecided, and if it differs
   from the job's, the job's becomes `DISABLED` for this and every later
   draw; `DISABLED` makes the job `DISABLED`. A fragment shader that writes Z
   itself (`prog_data->fs->writes_z`) makes it `DISABLED`.
3. `first_ez_state` is the state at the first draw that decided; the RCL
   uses it: `DISABLED` gives `Early-Z disable = 1`, otherwise the direction
   bit comes from it. A job with no depth buffer has `first_ez_state`
   `DISABLED` (this is the case yakogl's clear frame cites, `v3d_frame.c:185-188`).
4. **CFG_BITS** per draw: `Early Z updates enable` = job state not
   `DISABLED`; `Early Z enable` = the same, only if depth test is on.
5. **Shader record**: `Turn off early-z test` = `fs->disable_ez`, from the
   compiler (below).

### The shader's own flag, in Mesa and in yakogl

Mesa: `disable_ez` is set when the shader discards (unless
`can_earlyz_with_discard`, `v3d_program.c:627-631`: the draw writes neither
depth nor stencil), writes Z, or has no colour write (no early-tests
promotion without a colour write, `nir_to_vir.c:2067-2078`; writes Z and
flags a discard, `:2120-2126`). yakogl's compiler cites these and takes the
conservative half: `src/shaderc/online/v3d/select.rs:2281`,
`let disable_ez = p.discard.is_some() || p.rt_mask == 0;` and
`select.rs:2255-2268` (comment): the program does not know the draw, so a
discarding shader is always late, "which costs speed only". A shader that
reads `gl_FragCoord.z` writes Z through (`reads_z`, `:2282`) but keeps
`disable_ez` false (pinned by `chain.rs:2136`, `:2160`, `:2204-2212`).

### Per case (what decides legality)

| case | per draw | per job | evidence |
|---|---|---|---|
| depth test off | no early Z for the draw (CFG_BITS `Early Z enable` 0) | the draw does not break the job's direction; a job whose *first* decided draw has depth off and no depth buffer is `DISABLED` | `v3d_frame.c:185-188`; recalled |
| depth func changes direction inside the job | the later draw is tested late | job `DISABLED` from then on | recalled; the RCL has one direction field, xml:1004 |
| `LESS`/`LEQUAL` throughout (terrain) | early test and update | direction LT/LE | recalled |
| `discard` in the fragment shader | `Turn off early-z test`; the shader writes Z through, late test (`select.rs:2281`) | the other draws stay early | `select.rs:2255-2282`; `job_check.c:1568-1593` |
| depth write from the shader (`gl_FragDepth`; ES 2.0 has none, `gl_FragCoord.z` only reads) | late | job `DISABLED` (Mesa) | `select.rs:2282`; recalled |
| stencil | unchanged ops: early; any op that writes on depth fail: late | job `DISABLED` | recalled. yakogl's own bug history: with early tests a discarded fragment still updated stencil (femtovg, `select.rs:2257-2263`), so stencil+discard stays late |
| blending | no effect on early Z (fragments failing depth are dropped before the blend either way) | none | recalled; the order of blending is the same |
| depth buffer absent | none | `first_ez_state` `DISABLED`, `Early-Z disable = 1` | `v3d_frame.c:185-188` |
| no colour write (depth-only pass) | late (`rt_mask == 0` sets `disable_ez`) | other draws unaffected | `select.rs:2281` |

## (b) What yakogl does today

* Job level, always off: `driver/src/v3d/v3d_job.c:1248`,
  `common.early_z_disable = true;` in `v3d_job_end`'s
  `TILE_RENDERING_MODE_CFG_COMMON` (`:1240-1251`), with the comment "early Z
  off (see v3d_job.h)". The clear frame does the same,
  `v3d_frame.c:188`, for the stated Mesa reason (no depth buffer).
* Header: `driver/src/v3d/v3d_job.h:29-31`, "What this cut does not do,
  refused rather than drawn wrong: ... early Z (off: legal, only slower)".
* Draw level, never set: `emit_cfg_bits` (`v3d_job.c:545-567`) says "early Z
  stays off (the RCL disables it), so its two bits stay 0"; it writes only
  `z_updates_enable` and `depth_test_function` (`:564-565`). The change
  detector key `V3D_PACK_CFG` in `emit_state` (`v3d_job.c:658-661`) holds
  `depth_enabled` and `depth_writemask` bits and the function, no early-Z bit.
* Shader record: `r.turn_off_early_z_test = fs->disable_ez;`
  (`v3d_job.c:832`, `v3d_frame.c:624`), `fragment_shader_does_z_writes =
  fs->writes_z`. The flag arrives from the compiler
  (`src/shaderc/online/v3d/select.rs:2281`, `chain.rs:515`,
  `v3d_abi.rs:280`) through `V3dFsProg.disable_ez` (`src/v3d/ffi.rs:88`,
  filled at `src/v3d/backend.rs:3612`; C side `v3d_frame.h:169`). It is
  already correct per shader; nothing consumes it for a decision.
* The state that decides the direction already reaches the builder:
  `src/v3d/state.rs:549` `job_state` fills `depth_enabled`,
  `depth_writemask`, `depth_func` (`:600-605`), the stencil faces and the
  blend state into `V3dJobState` (`ffi.rs:337`, C `v3d_job.h:145`,
  `depth_func` at `:158-160`).
* Pinned as "off": `tools/v3d/job_check.c:685-688` expects `"Early-Z disable"`
  1 in the RCL; the shader-record checks are `:1568-1593`.

**Why it was turned off.** `git log -S early_z_disable -- driver src` gives
one commit, `381b0f09` (the first import). There is no earlier note and no
later one. The only reason on record is the header's "legal, only slower": the
first cut of the job builder left it out as a scope cut, along with
transform feedback and primitive restart, "refused rather than drawn wrong".
There is no known picture defect behind it. (The discard path's late-Z rule,
`select.rs:2255-2268`, has a reason: femtovg's stencil edge image
`998e5188`. It is a separate switch and stays.)

## (c) The smallest change

Mesa's decision made once per job, in the builder, from state the builder
already receives.

1. **`driver/src/v3d/v3d_job.h` / `v3d_job.c`** (held by #308, #321):
   * `struct v3d_job_state` gets `uint32_t ez_state` (UNDECIDED, LT_LE,
     GT_GE, DISABLED), computed by the caller (item 2), so the C builder
     stays free of GL;
   * `struct v3d_job` gets `ez_state` and `first_ez_state`; `v3d_job_draw`
     runs `v3d_update_job_ez` (rules 2 and 3 above, plus
     `fs->writes_z`);
   * `emit_cfg_bits` sets `early_z_updates_enable` and, with
     `depth_enabled`, `early_z_enable`; the `V3D_PACK_CFG` key gains the two
     bits (otherwise a draw whose only change is the job state is skipped as
     a repeat: the one trap of this change);
   * `v3d_job_end` writes `common.early_z_disable = j->first_ez_state ==
     DISABLED` and `early_z_test_and_update_direction` from it. The clear
     frame (`v3d_frame.c:188`) stays: it has no depth buffer.
   * `v3d_job.h:29-31` loses "early Z".
2. **`src/v3d/state.rs`** (`job_state`, `:549-640`; not `backend.rs`): map
   `depth_func` and the stencil ops to `ez_state` (rule 1) and fill the new
   field; `src/v3d/ffi.rs:337` mirrors the struct and its layout
   assertions (`:814` style). No change to `src/v3d/backend.rs`, which
   plan #308 and `v3d-wait-time` hold. `src/shaderc` needs **no** change:
   the conservative `disable_ez` stays.
3. **A switch.** Default off first, an `Backend` or env knob like
   `YAKOGL_V3D_TFU` (see `pi4-glmark2-cpu.md`), until the board run shows the
   pictures equal; then default on in the same pull request that carries the
   board record.

Not needed: any change to depth load/store, the TLB's Z/S format
(`v3d_job.c:1249` stays), or the early Z *clear* (`Early Depth/Stencil Clear`,
xml:999, a separate, optional optimisation).

### The host test that pins it

There is no rasteriser on the host, so the test pins **the bytes the builder
emits**, which is what the board then reads:

* `tools/v3d/job_check.c` (compares CLIF text with fields; built against the
  Mesa reference by `tools/check.sh`): change `:685-688` to a pair of jobs.
  A LEQUAL, depth-writing, non-discarding job: RCL `Early-Z disable` 0,
  direction 0, every draw's CFG_BITS `Early Z enable` and `Early Z updates
  enable` 1. A job that mixes LEQUAL and GEQUAL draws: the second draw has
  both bits 0 and the RCL keeps the first's direction. A discarding shader
  record (`:1568-1593`) keeps `Turn off early-z test` 1 and the job stays
  early for the other draws. A depth-less job: `Early-Z disable` 1.
* `tools/v3d/frame_check.c:788` already compares the shader-record bits; add
  the two CFG_BITS fields.
* `src/v3d/state.rs` unit tests: `ez_state` of the (depth_func, stencil) grid
  (LESS, LEQUAL, GREATER, GEQUAL, EQUAL, NEVER, ALWAYS, NOTEQUAL, stencil
  zfail KEEP/not).
* `tests/v3d_terrain_jobs.rs` (the terrain frame through the real backend and
  a stand-in builder): it cannot see the packets, so no change; its job
  counts must not move (a guard that this is a packet-level change only).
* The ES 2.0 guard (`tests/es2_report_guard.rs`) is unaffected (no GL answer
  changes); it is part of the gate regardless.

Gate: the pull request's CI reaches the v3d rows; the driver change also
reaches the local set (`driver/src/v3d`).

## (d) The gain per scene

Early Z saves the fragment shading of fragments that fail the depth test.
With late Z every fragment is shaded; with early Z only those that pass the
test at their time. Savings are `shaded - kept`, so with overdraw `o`
(shaded fragments a covered pixel under late Z) the fraction of fragment
work removed is at most `(o - 1) / o`, less when the draw order is back to
front. Nothing else moves: binning, vertex shading (it runs twice), the
depth test itself, the TLB loads and stores.

Figures are from `v3d-terrain-gpu.md` (model, not the board: its "per-job
times are not in the tree", `:50-53`, and `pi4-slow-terrain.md`):

| scene | job | model render now | fragment work | early Z removes | range | confirming board value |
|---|---|---:|---|---|---:|---|
| terrain | 3 (terrain draw, depth LEQUAL, write on) | ~7.2 ms | 480,000 px x 9 samples = 4.3 M at `o` = 1 | `4.3 M x (o - 1)` samples, 1 ns each | **0 at o = 1.0; 1.3 ms at o = 1.3 (that note's assumption: 0.14 M px x 9 = 1.3 M); 1.7 ms at o = 1.4** | `glmark2-jobs:` entry of job 3, `render`: from ~7 ms to 5.5-7 ms. All other entries, `draws`, the job count (9) and `tfu` unchanged. The frame `total` falls by the same amount. |
| shadow | its depth pass (depth-only shader: `rt_mask == 0`, so `disable_ez` stays 1; no change) and the lit pass | not in the tree | the horse and floor, trivial shader | lit pass only, overdraw ~1.0-1.2 (floor behind the horse) | 0 to ~0.2 ms of 7.34 ms; the frame is CPU/GL-bound (`pi4-glmark2-cpu.md`), so a GPU saving does not show | per-scene ms (`glmark2-frame:` ms) within noise of 7.34; `glmark2-jobs` render of the lit job falls by 0-0.2 ms |
| refract | the refracting draw (heavy shader, back faces first with `CullFace(FRONT)`, `scene-refract.cpp:255`) | ~17 ms frame, swap 15.4 ms of it, 3 jobs; GPU-bound | one heavy fetch pass over a 800x600 canvas, the depth texture 1600x1200 | the pass `o - 1` of the horse silhouette, 1.0-1.3 | 0 to ~1 ms of 17.27 ms (assumption: pass is ~ half the GPU time) | `glmark2-jobs` render of the main pass; frame ms 17.27 to 16.3-17.27 |

Notes on the table:

* The 1.7 ms of `v3d-terrain-gpu.md` is not what its own inputs give at
  overdraw 1.3 (`(0.624 - 0.480) M x 9 = 1.3 M` samples). Both are
  unmeasured; the **overdraw is the unknown**, and the sample cost (one
  bilinear a clock a slice) is the note's "assumption" (`:196-217`). If the
  TMU does more than one a clock the gain shrinks in proportion.
* The number that decides the work: the overdraw of the terrain draw. The
  host cannot count it (its rasteriser does not; `v3d-terrain-gpu.md`, "No
  overdraw of the terrain draw"). The board can: run once with the change on
  and read `render` of job 3 (value above); no separate counter is needed.
* The V3D already depth-sorts nothing: terrain's grid draws in index order, so
  hills hide what is behind them only when the view looks over them. The
  scene looks down at the terrain at a tilt (all four corner probes are
  terrain colour, `pi4-glmark2-cpu.md:371`); expect `o` near the low end.
* Shadow's number is a bound, not a measurement. The scene's frame is not
  GPU-bound on the Pi 4, so even a real GPU saving is hidden under the swap
  wait: the confirming value there is the jobs line, not the frame ms.

## (e) Risks

* **Pictures.** Early Z and late Z give the same result for a shader without
  `discard` and `gl_FragDepth`, if the direction rule holds. The failure
  modes: (1) a draw whose function changes direction in a job that the
  builder fails to notice (the `V3D_PACK_CFG` repeat-suppression trap above);
  (2) `Early Z updates enable` with a shader record that writes Z (Mesa
  disables the job); (3) `EQUAL`/`NEVER`/`NOTEQUAL` mapped to a direction.
  Mitigate by following the table exactly and by falling back to `DISABLED`
  for anything not listed (stencil with any op changing on zfail, `ALWAYS`).
* **Stencil.** Early Z/stencil with a discarding shader has an observed bug
  (femtovg, `select.rs:2257-2263`). That path keeps `disable_ez` and is
  unchanged; but a job with stencil enabled and a non-discarding shader
  would now test stencil early. Recommended first cut: stencil enabled makes
  the job `DISABLED` (conservative; terrain, shadow and refract use no
  stencil). The femtovg image `998e5188` is the board check.
* **Depth-only and discarding draws in the same job.** A job with an
  early-Z draw followed by a late one (a discarding fragment shader) is
  Mesa's normal case; the late draw still tests in the TLB. If the TLB's Z
  were updated early by a draw that a later late draw then discards, nothing
  is wrong (the late draw never discarded the earlier one). (recalled)
* **Hardware.** I know of no 4.2.14 erratum bearing on this that is in the
  tree; Mesa ships it on the Pi 4 unconditionally. Not verified on this
  board: the first board run is also the check.
* **ES 2.0 answers.** None: no query, enum or limit changes.
  `tests/es2_report_guard.rs` pins answers, not packets. The dEQP-GLES2 depth
  cases (`depth_stencil`, `depth_test`-style, `fragment_ops.depth_stencil`)
  are the cases to rerun on the Pi 4 board (`tools/cts`); a regression there
  would be a direction or a stencil bug, not an ES answer.
* **Gate/other plans.** The files are held by #308 (`src/v3d`, `driver/src/v3d`)
  and #321 (`driver/src/v3d/**`): the change waits for both to release them,
  is small, and rebases onto the current `origin/main`. It reaches
  `tools/v3d/job_check.c`, which `tools/check.sh` runs against the Mesa
  reference; any change in `v3d_job.c`'s bytes must also regenerate nothing
  (no generated file is touched).

## Handoff

* Value for a plan: low (0 to ~1.7 ms of ~29 on terrain, ~0 on shadow, <= 1
  ms on refract); do it after the candidates ranked above it in
  `v3d-terrain-gpu.md`, or never if the first board run of the jobs line
  shows job 3 under ~5.5 ms.
* First action when someone does it: fetch the `mesa-26.2.1` tarball, read
  `v3d_state.c` (`v3d_create_depth_stencil_alpha_state`), `v3dx_draw.c`
  (`v3d_update_job_ez`) and `v3dx_rcl.c` (`emit_rcl`), and correct every
  "recalled" row above.
* The 1.7 ms in `v3d-terrain-gpu.md` should read 1.3 ms at overdraw 1.3; that
  note was not edited here.
