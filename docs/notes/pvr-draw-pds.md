<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: the kick-USC and coefficient PDS programs once per frame and program

Fix F4 of [gl-draw-fixed-cost.md](gl-draw-fixed-cost.md) ("Ranked fixes"),
in `record_draw` (`src/pvr/backend.rs`). Before: 41e951696; after:
5f2a40938.

## What a draw did

Every draw ran three PDS generators and put each result into the open
frame's PDS arena with `pds_put`:

* the vertex-fetch program (`pvr_pds_render_vertex_program_ids`): per draw by
  design, its data segment carries the draw's attribute addresses;
* the fragment kick-USC program (`pvr_pds_render_kick_usc_program`);
* the coefficient (DOUTI) program for the fragment shader's varyings
  (`pvr_pds_render_coeff_program`).

The last two do not depend on the draw. `pds_put` already pointed a draw at
words the frame held (`pds_seen`, an FNV-1a hash and a word compare), so the
arena did not grow per draw; but every draw still ran both generators,
copied the coefficient program into one buffer, hashed both and compared
them with the arena.

## The key

Exactly what each generator reads, so a hit is the same words by
construction:

| program | key |
|---|---|
| kick-USC | the fragment program's USC heap offset (`d.fs_usc_gpu`), its temps (raised by `PVR_BACKEND_OPT_FS_TEMPS_MIN`), the sample rate (`fs_sample_rate`: the frame's samples, and blend, fbfetch, discard or the sample-store load quad) |
| coefficient | `uses_w`, `uses_z != 0`, `uses_pntc != 0`, and each iterated varying as `record_draw` builds it (`s.cv`: components, interpolation after the no-W fallback, VS output offset) |

The key is the generators' inputs, not a program pointer: the link step
writes a shader's varying offsets into its record (`src/shaderc/link.rs`),
so a record's address alone does not pin them, and a blend variant is
another USC offset. The coefficient entry also keeps the
generator's `PvrPdsCoeffInfo` (the PPP block's varying sizes).

## Where it lives and when it is forgotten

`Backend::kick_once` and `Backend::coeff_once`, 16 entries each, the oldest
replaced first; each entry keeps the program's PDS heap offset and the arena
offset its words end at.

* **Frame start** (`frame_begin_with`, where the PDS arena's bump goes back
  to 0 and the next frame set's arena becomes the active one): both tables
  are cleared. No offset outlives its frame; nothing is cached across frames.
* **A failed draw** (`record_draw_txn` gives the arena bytes back): entries
  whose words end past the restored bump are dropped, since the next put
  writes there. Entries below it stay, as `pds_put`'s own do.
* **Backend init** zeroes the whole backend state.
* The arena does not grow or move within a frame (a full arena ends the
  frame), so there is no other point.
* `PVR_BACKEND_OPT_DEBUG` with dump budget left generates every draw's
  programs, so the bring-up dump still prints their words.

`pvr_backend_set_pds_once(false)` (Rust only) turns the tables off: the
switch the tests compare the two paths with.

## Tests

* `tests/pvr_pds_once.rs`: glDrawElements through the GL core into the
  backend, on stand-in generators that count their calls and encode their
  inputs in their words. With the tables on and off every draw points at
  the same words; with them on, the generators run once per key and frame.
  One program x 64 draws, two programs alternating over three frames, and
  both sample rates in one 4x frame (blend on and off). With the tables off
  the per-frame generation counts are the draw counts, so the test fails.
* `src/pvr/backend_tests.rs`: one fragment program at both sample rates in
  one 4x frame (one generation per rate, each draw at its rate's words), and
  a draw that fails after its kick program was put (the next program lands
  there; the failed one's program is generated again, not pointed at).

## Host before and after

`tools/hostharness/glmark2/run.sh` with `GM2_PROFILE=1 GM2_FRAMES=20`
(19 counted frames), powervr build, the scenes `ideas`, the three `buffer`
entries, the four `shading` entries and `terrain`; 25 repeats a side,
alternating before and after. Host: 8 cores, shared with other jobs (load
average 3.6 to 7.1 during the runs). Figures are ms a frame in GL and EGL
calls (`compare.py`'s default metric), minimum and median of the 25, and
the CV of the before repeats (see
[FINDINGS-variance.md](../../tools/hostharness/glmark2/FINDINGS-variance.md):
report the minimum, call nothing under the spread resolved).

| scene | 41e951696 min | median | CV % | 5f2a40938 min | median | delta min | ratio min |
|---|---|---|---|---|---|---|---|
| `ideas:speed=duration` | 0.3897 | 0.5597 | 23 | 0.3843 | 0.4854 | -0.0054 | 0.99x |
| `buffer:...:update-method=map:interleave=false` | 0.0615 | 0.0985 | 25 | 0.0602 | 0.1033 | -0.0014 | 0.98x |
| `buffer:...:update-method=subdata:interleave=false` | 0.0927 | 0.1303 | 24 | 0.0923 | 0.1500 | -0.0003 | 1.00x |
| `buffer:...:update-method=map:interleave=true` | 0.0630 | 0.1017 | 23 | 0.0619 | 0.1097 | -0.0011 | 0.98x |
| `shading:shading=gouraud` | 0.0055 | 0.0075 | 22 | 0.0051 | 0.0069 | -0.0004 | 0.93x |
| `shading:shading=blinn-phong-inf` | 0.0047 | 0.0071 | 23 | 0.0054 | 0.0072 | +0.0006 | 1.13x |
| `shading:shading=phong` | 0.0055 | 0.0077 | 24 | 0.0055 | 0.0061 | +0.0001 | 1.01x |
| `shading:shading=cel` | 0.0053 | 0.0071 | 24 | 0.0055 | 0.0060 | +0.0002 | 1.04x |
| `terrain` | 0.1464 | 0.2202 | 22 | 0.1559 | 0.2030 | +0.0095 | 1.06x |

(`buffer:...` is `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:...`.)

Per call, `ideas`' 162 `glDrawElements` a frame: minimum 2.114 us before,
2.078 us after (median 3.021 and 2.584).

**No change is resolved on the host.** Every delta of the minima is inside
the before repeats' 22-25% spread; the largest draw scene, `ideas`, moved by
1.4% a frame and 1.7% a call. That fits the estimate of the study (0.2 to
0.6 us a draw on the board, unmeasured): on the host the two generators are
native x86 code of a few dozen stores each, and the hash and word compare
of `pds_put` that the hit also skips are of nine and about ten words. What
the change does remove, by construction and as the tests count, is two
generator calls, one copy, two hashes and two compares per draw after a
program's first draw of the frame. Whether that is visible on the AM62P's
Cortex-A53 is for the board session (board-am62p-final) to measure;
`terrain` and `shading` draw one or a few programs a frame and are expected
to stay where they are.

## What this does not cover

* The vertex-fetch program (per draw by design) and the common-store DMA
  program, whose data segment carries per-draw addresses.
* More than 16 (program, variant, rate) combinations in one frame: the
  oldest is generated again, which is correct and only slower.
