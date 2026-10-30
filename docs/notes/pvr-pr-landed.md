<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The partial render, parts 1 and 4: what has landed

Host-only reading, no code, 2026-10-30 (plan #148 item pr-landed-note, issue
#3). It records what the code does at the integration tip (e9e1600a) so the
audit of parts 1 and 4 (pr-parts14-audit, whose report is not in the tree)
has a durable result. The design is "Design: the firmware's partial
render" in docs/notes/pvr-render-pool.md; this note is not a replacement of
it. Nothing here was run on hardware. Paths are relative to the repository
root; `render.rs` is driver/rust/src/gpu/pvr/render.rs.

Every behaviour below exists only with the knob
(`CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER`, driver/zephyr/Kconfig:69, off by
default: driver/src/pvr_render.c:430-433). pvr_render.c:602 copies it to
`YdPvrRenderCfg.partial_render` (render.rs:229).

## What each landed part does

### Part 1: the PR commands, the fence words, the timeline

| behaviour | where |
|---|---|
| The geometry UFO block is one word longer than its 16 slots: the **geometry timeline** word at `gufo_off + 4 * GEOM_UFO_SLOTS` (allocated for both knob states; only used with the knob) | render.rs:492 (`fw_alloc(4 * GEOM_UFO_SLOTS + 4)`); address `timeline_fa` render.rs:1362 |
| The timeline starts 0 with the slots (below every seq, so the first kick's PR fence waits) | render.rs:541-543 |
| Geometry command: `dk[13]` = timeline address, `dk[14]` = `seq - 1` (wrapping). Knob off: `dk[13]` = the kick's own slot address, `dk[14]` = 0, as before | render.rs:1384-1388 (knob-off values render.rs:1375-1376) |
| Geometry CCB gains a second UPDATE, timeline = seq, after the existing slot UPDATE. The ring reservation grows by 2 x 16 bytes | render.rs:1390, :1393-1395 |
| Fragment CCB: `FENCE_PR {timeline, seq}`, `3D_PR`, `UPDATE {timeline, seq}`, then today's `FENCE` (kick's slot), `3D`, `UPDATE` (fragment UFO). The reservation doubles | render.rs:1463 (reserve), :1478-1480 (the trio), :1488 (the 3D) |
| The `3D_PR` body is a byte copy of the 3D command (`dk`, 80 dwords, 320 bytes): same words, same EOT data, same Z/S and background words | render.rs:1479 (same `dk` as :1488) |
| The PR UPDATE rewrites the timeline with `seq`, not `seq + 1`: it is a no-op while no partial render is pending, and the next kick's fence is an equality test | comment render.rs:1470-1477, UPDATE render.rs:1480 |
| The stall dump knows the two command types by name | render.rs:1867-1875 |
| The firmware's partial-render count (UPDATE_STATS element 1) is read into the counters gl_test prints | render.rs:2141-2153, fwif.rs:131 |
| Command-type pins | fwif.rs:343 (`CCB_CMD_TYPE_3D_PR` 0x2ABC80CC), :346 (`CCB_CMD_TYPE_FENCE_PR` 0x2ABC00D7); driver/tools/offsetof/ddk_abi_pins.h:246, :249 |

### Part 4: the HWRT side

| behaviour | where |
|---|---|
| Two dword arrays per HWRT slot, `MAX_RT` (3) dwords each, allocated once (`off == 0`), zeroed and flushed | render.rs:61 (`MAX_RT`), :912-921; offsets kept in the slot, render.rs:175, :197 |
| The valid render-target array is rewritten 0, 1, 2 on every build; the partial-render count array is left to the firmware | render.rs:923-934 |
| The HWRT data's `sRTACtl` words: `sValidRenderTargets` (+16), `sRTANumPartialRenders` (+20), `ui32MaxRTs` (+24) = `MAX_RT`; `ui32RTACtlFlags` (+28) and the counters stay 0 | render.rs:951-953; offsets fwif.rs:320-326 |
| `HWRTDATA_PARTIAL_RENDERED` (0x8) is never set by the host | render.rs:954-956 (comment), fwif.rs:316 |
| `freelist::reset_hwrts` clears `HWRTDATA_HAS_LAST_TA | HWRTDATA_PARTIAL_RENDERED` along with setting the state to HWR, for every HWRT data that uses the reconstructed list | freelist.rs:718-735 (mask at :733) |
| Pins: `sRTACtl` 112, `RGXFWIF_RTA_CTL` 32, its four member offsets, `HWRTDATA_PARTIAL_RENDERED`, `RTDATA_STATE_PARTIALRENDERFINISHED` | ddk_abi_pins.h:306, :320-330 |

### The gl_test cases

- Case 175 `refusehang` (driver/tools/pvrlab/gl_test.c:22302-22380): one kick
  in flight, frames of growing quad count into 1024x1024 until a frame asks
  for a grow, built with `FL_PROBE_NO_RESERVE` so the grow is refused at
  A = 0. Knob off: the 10 s stop (cause 4), then the counters and FAIL. Pass
  with the knob needs "partial renders" >= 1.
- Case 217 `forcedpr` (gl_test.c:22386-22540): 256 rectangles in a
  1024x1024 target, depth LESS, `FL_PROBE_MAX_PAGES=512`, read back whole
  against a computed image. Not in the default list; run by number.
  Registered at gl_test.c:22685-22686.

## Design "Host tests, landing with each part": covered and open

Test names are in render.rs's test module (the rig is `rig(partial_render)`,
render.rs:3664) unless noted.

| design bullet | covered by | state |
|---|---|---|
| render.rs: with the knob the fragment CCB holds FENCE_PR {timeline, seq}, 3D_PR (its size), UPDATE, FENCE, 3D, UPDATE; the geometry PR fence words are {timeline, seq - 1} | `with_the_knob_the_pr_trio_comes_first_on_both_ccbs` (render.rs:3809) | covered (also asserts the geometry timeline UPDATE, the 3D_PR size 320 and the copy equality) |
| render.rs: without the knob the bytes are today's | `without_the_knob_the_commands_are_todays` (render.rs:3897) and `the_knob_changes_only_the_pr_commands_and_the_geometry_fence_words` (render.rs:3919) | covered, in the test rig. The golden km render record is the other half: see open item 7 |
| Part 4: the RTA arrays and sRTACtl words in `hwrt_get` | `with_the_knob_the_hwrt_data_carries_the_partial_render_accounting` (render.rs:3954); knob off: `without_the_knob_the_partial_render_words_stay_zero` (render.rs:3977) | covered |
| Part 4: `reset_hwrts` clears PARTIAL_RENDERED | `reconstruction_resets_every_hwrt_that_uses_the_list` (freelist.rs:1398; asserts flags 0x1 after, and an unrelated HWRT left alone) | covered |
| Pins | driver/tools/offsetof against the DDK headers (ddk_abi_pins.h), run by the gate's offsetof rows | covered for the lines listed above |
| backend.rs: PR PBE words, PR EOT data address, `pds_pr_bgnd` only with the knob | none yet | part 2/3, not landed |
| SPM scratch size per RT size (256x256, 800x600, 1600x1200, 4x MSAA) | none yet (spm-calc is in review) | not landed |
| Background-load shader table entry decoded by our USC decoder | none yet | part 3b, not landed |

The ring-wrap behaviour of the larger reservation has no PR-specific test; it
is the existing `ring_reserve` (render.rs:1037) with a larger `need`.

## What is open on parts 1 and 4

Each item is a behaviour the design expects that the code does not do, or a
fact the code assumes. Checked against render.rs, freelist.rs, fwif.rs at the
tip.

1. **The 3D_PR is not a PR command.** It is a byte copy of the 3D command
   (render.rs:1479). The design's part 2 replaces its PBE words and its EOT
   data address (`event_pixel_pds_data`, `dk[15]`) with the SPM scratch's and
   the PR EOT program's, and sets `pds_pr_bgnd`. None of that exists. The
   `PvrRenderKick` has no `frag_pr` block (kick-abi is building it). A
   partial render that executes this command would store the tiles to the
   frame's own buffers and nothing could load them back.
2. **The geometry PR-fence offsets are not pinned.** Design part 1 says
   `dk[13]`/`dk[14]` are "pinned through driver/tools/offsetof against the
   DDK's own CMDTA, never by count". The code takes them from the captured
   layout and says the DDK CMDTA is not in the vendored headers
   (render.rs:1378-1383); ddk_abi_pins.h has no `partial_render_ta_3d_fence`
   pin. The ABI rests on a count.
3. **The pins of part 4 stop at the offsets.** The `sRTACtl` word offsets are
   pinned. The `RGXFWIF_PRBUFFER` struct and `aui32FreeListHWRSnapshot` are
   not (pins-hwrt-rest).
4. **The valid-render-target and count arrays are not re-zeroed when a slot
   is rebuilt.** The count array is zeroed only at its first allocation
   (render.rs:915-921); a rebuild of a recycled slot keeps whatever the
   firmware counted. If the firmware's counts must start at 0 per
   dataset (the design says "allocated and zeroed there"), a rebuild after a
   knob-on partial render starts with stale counts. Unverified: no partial
   render has run.
5. **`reset_hwrts` clears the flag but not the RTA counts**, and not the
   sRTACtl `ui32RTACtlFlags`. Whether upstream's reconstruction resets them
   is not read in this tree.
6. **The fence arithmetic is unmeasured.** The PR fence waits for timeline ==
   `seq` while the geometry command waits for timeline == `seq - 1`
   (render.rs:1386-1387, :1478). That is upstream's arithmetic under one
   timeline (pvr_queue.c, per the design), but the firmware's test is not
   readable; the design's measurement (partial renders >= 1 on the forced
   case, no stall) is the only check. It also relies on the timeline equalling
   `seq` only for the one kick; a geometry phase that runs ahead of the
   awaiting PR fence moves the word past it (the code's own comment names
   the equality test).
7. **No golden proves the knob-off words unchanged.** The unit tests compare
   knob on against knob off in the rig only; acceptance c4 (the km render
   record) is the item c4-golden-read's finding, not a test of this part.
8. **The timeline is not in the stall dump.** The 10 s stop dump prints the
   last 3D command (render.rs:1826, :2340, :2362) but neither the timeline
   word, the 3D_PR, nor the HWRT's PR state (pr-diag-render).
9. **The firmware's refusal of a null/short RTA setup is untested.** The
   firmware names a null `ValidRenderTargets` an invalid setup
   (rgx_fwif_sf.h:569); with the knob off both arrays stay 0 and the
   behaviour is today's, with it on they are non-null. Whether the array
   length `MAX_RT` = 3 is what the firmware expects for one render target
   (it is the design's guess of "colour attachments one render can write")
   has not been checked.

## What a knob-on run can reach today

As the design note "After the decision (2026-10-27)" says: with parts 1 and 4
only, a refusal is survived only while a 3D is pending. With the knob on:

- Case 175 and case 217 enqueue the PR trio and the timeline words; the
  firmware can see a partial render command. A refusal at A = 0 with nothing
  to render is the case the partial render is for, and it is not yet sound:
  the 3D_PR it would run is a copy of the ordinary 3D (open item 1), with no
  scratch to store to and no background-load program to resume from.
- Whether the firmware accepts the commands at all (a queued FENCE_PR/3D_PR
  that is never executed) is the first thing the board session
  `neg175-board` measures; "partial renders" >= 1 would be a bonus, not
  expected, and a correct image from case 217 is not expected.
- With the knob off nothing changes: no new command, no new word, the HWRT
  data zero in the RTA words.

The knob stays off on the board until parts 2 and 3 land and the forced case
passes (the design's own condition).
