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

## Summary as of 2026-11-21 (plan #301, issue #2 of signal-slot/yakogl)

The "What each landed part does" tables are the 2026-10-30 reading of
parts 1 and 4 (cites moved to the code of 2026-11-21 where a line shifted).
The test table, "What is open on parts 1 and 4" and "What a knob-on run can
reach today" were refreshed on 2026-11-21 against the tip (3cb260c2). The
current status is the paragraph "Status, 2026-11-21" in
docs/notes/pvr-render-pool.md; the facts:

- **Implemented**, behind `CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER` (default
  n, driver/zephyr/Kconfig): parts 1 and 4 (`f73eded7`) and the frame side:
  `src/pvr/spm.rs` (`14fbb630`), the PR EOT data slot (`5671bcf0`), the
  `frag_pr` block (`62b08d4b`), the load shader under `tools/pco_shc`
  (`cfb0a032`), `src/pvr/pr_zs.rs` (`6f550632`), the 3D_PR built from
  `frag_pr` (`f1b3f8d7`), and the backend filling `frag_pr` and
  `pds_pr_bgnd` (`1311a675`). Stated gaps: `pds_pr_bgnd` is not written into
  the 3D_PR command; the Z/S words of `pr_zs.rs` have no `frag_pr` field.
- **Measured on the board** (parts 1 and 4 only, before the frame side):
  `tools/cts/log/2026-10-31-pr-neg175/` (case 175 knob off TIMEOUT cause 4;
  knob on PASS, one partial render, HWR 0),
  `tools/cts/log/2026-10-31-pr-negative/` (case 217 knob off, 256 layers:
  NOT EXERCISED), `tools/cts/log/2026-11-02-pr-217probe/` (crossing between
  976 and 1220 layers; knob on at the 512 cap: stop cause 2, partial renders
  0; knob on without cap: one partial render, depth equal, colour mostly the
  clear).
- **Known blocker:** case 217 at the 512-page cap ends in the at-maximum stop
  (cause 2, `ccb.rs` `at_maximum`, not gated by the knob) with the knob on.
- **Nothing with the frame side has run on the board.** The three board
  record directories above are all from images built before the frame side
  (before `f1b3f8d7` and `1311a675`). Case 175/217 knob on with the frame
  side, and the regression set knob off and on, are not measured
  (board-regress, waiting for the board).

Every behaviour below exists only with the knob
(`CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER`, driver/zephyr/Kconfig:69, off by
default: driver/src/pvr_render.c:430-433). pvr_render.c:602 copies it to
`YdPvrRenderCfg.partial_render` (render.rs:229).

## What each landed part does

### Part 1: the PR commands, the fence words, the timeline

| behaviour | where |
|---|---|
| The geometry UFO block is one word longer than its 16 slots: the **geometry timeline** word at `gufo_off + 4 * GEOM_UFO_SLOTS` (allocated for both knob states; only used with the knob) | render.rs:565 (`fw_alloc(4 * GEOM_UFO_SLOTS + 4)`); address `timeline_fa` render.rs:1434 |
| The timeline starts 0 with the slots (below every seq, so the first kick's PR fence waits) | render.rs:612-616 |
| Geometry command: `dk[13]` = timeline address, `dk[14]` = `seq - 1` (wrapping). Knob off: `dk[13]` = the kick's own slot address, `dk[14]` = 0, as before | render.rs:1458-1459 (knob-off values render.rs:1448-1449) |
| Geometry CCB gains a second UPDATE, timeline = seq, after the existing slot UPDATE. The ring reservation grows by 2 x 16 bytes | render.rs:1462, :1465-1467 |
| Fragment CCB: `FENCE_PR {timeline, seq}`, `3D_PR`, `UPDATE {timeline, seq}`, then today's `FENCE` (kick's slot), `3D`, `UPDATE` (fragment UFO). The reservation doubles | render.rs:1565 (reserve), :1581-1583 (the trio), :1591 (the 3D) |
| The `3D_PR` body is 80 dwords, 320 bytes. With the knob and a zero `frag_pr` it is a byte copy of the 3D command (`dk`); with a filled `frag_pr` it is `dk_pr`: the PBE words and the EOT PDS data address (dword 15) from `frag_pr` (see open item 1) | render.rs:1531-1541 (`dk_pr`), :1582 (the 3D_PR put) |
| The PR UPDATE rewrites the timeline with `seq`, not `seq + 1`: it is a no-op while no partial render is pending, and the next kick's fence is an equality test | comment comment render.rs:1569-1580, UPDATE render.rs:1583 |
| The stall dump knows the two command types by name | render.rs:1970-1978 |
| The firmware's partial-render count (UPDATE_STATS element 1) is read into the counters gl_test prints | render.rs:2256, fwif.rs:131 |
| Command-type pins | fwif.rs:343 (`CCB_CMD_TYPE_3D_PR` 0x2ABC80CC), :346 (`CCB_CMD_TYPE_FENCE_PR` 0x2ABC00D7); driver/tools/offsetof/ddk_abi_pins.h:246, :249 |

### Part 4: the HWRT side

| behaviour | where |
|---|---|
| Two dword arrays per HWRT slot, `RTA_MAX_RTS` (3) dwords each, allocated once (`off == 0`), zeroed and flushed | render.rs:71 (`RTA_MAX_RTS`, 3; `MAX_RT` is now 4 colour attachments, render.rs:61), :979-993; offsets kept in the slot, render.rs:247 |
| The valid render-target array is rewritten 0, 1, 2 on every build; the partial-render count array is left to the firmware | render.rs:995-1004 |
| The HWRT data's `sRTACtl` words: `sValidRenderTargets` (+16), `sRTANumPartialRenders` (+20), `ui32MaxRTs` (+24) = `RTA_MAX_RTS`; `ui32RTACtlFlags` (+28) and the counters stay 0 | render.rs:1023-1025; offsets fwif.rs:321-326 |
| `HWRTDATA_PARTIAL_RENDERED` (0x8) is never set by the host | render.rs:1026-1028 (comment), fwif.rs:316 |
| `freelist::reset_hwrts` clears `HWRTDATA_HAS_LAST_TA | HWRTDATA_PARTIAL_RENDERED` along with setting the state to HWR, for every HWRT data that uses the reconstructed list | freelist.rs:718-736 (mask at :733) |
| Pins: `sRTACtl` 112, `RGXFWIF_RTA_CTL` 32, its four member offsets, `HWRTDATA_PARTIAL_RENDERED`, `RTDATA_STATE_PARTIALRENDERFINISHED` | ddk_abi_pins.h:306, :320-330 |

### The gl_test cases

- Case 175 `refusehang` (driver/tools/pvrlab/gl_test.c:23354-23418, `case_refusehang`): one kick
  in flight, frames of growing quad count into 1024x1024 until a frame asks
  for a grow, built with `FL_PROBE_NO_RESERVE` so the grow is refused at
  A = 0. Knob off: the 10 s stop (cause 4), then the counters and FAIL. Pass
  with the knob needs "partial renders" >= 1.
- Case 217 `forcedpr` (gl_test.c:23523-23726, `case_forcedpr`): 256 rectangles in a
  1024x1024 target, depth LESS, `FL_PROBE_MAX_PAGES=512`, read back whole
  against a computed image. Not in the default list; run by number.
  Registered at gl_test.c:23876-23877.

## Design "Host tests, landing with each part": covered and open

Test names are in render.rs's test module (the rig is `rig(partial_render)`,
render.rs:3811) unless noted.

| design bullet | covered by | state |
|---|---|---|
| render.rs: with the knob the fragment CCB holds FENCE_PR {timeline, seq}, 3D_PR (its size), UPDATE, FENCE, 3D, UPDATE; the geometry PR fence words are {timeline, seq - 1} | `with_the_knob_the_pr_trio_comes_first_on_both_ccbs` (render.rs:3809) | covered (also asserts the geometry timeline UPDATE, the 3D_PR size 320 and the copy equality) |
| render.rs: without the knob the bytes are today's | `without_the_knob_the_commands_are_todays` (render.rs:3897) and `the_knob_changes_only_the_pr_commands_and_the_geometry_fence_words` (render.rs:3919) | covered, in the test rig. The golden km render record is the other half: see open item 7 |
| Part 4: the RTA arrays and sRTACtl words in `hwrt_get` | `with_the_knob_the_hwrt_data_carries_the_partial_render_accounting` (render.rs:3954); knob off: `without_the_knob_the_partial_render_words_stay_zero` (render.rs:3977) | covered |
| Part 4: `reset_hwrts` clears PARTIAL_RENDERED | `reconstruction_resets_every_hwrt_that_uses_the_list` (freelist.rs:1398; asserts flags 0x1 after, and an unrelated HWRT left alone) | covered |
| Pins | driver/tools/offsetof against the DDK headers (ddk_abi_pins.h), run by the gate's offsetof rows | covered for the lines listed above |
| backend.rs: PR PBE words, PR EOT data address, `pds_pr_bgnd` only with the knob | src/pvr/backend_tests.rs: `pr_knob_off_frag_pr_zero_and_words_unchanged` (:7398), `pr_pds_pr_bgnd_only_with_the_option` (:7424), `pr_frag_pr_for_800x600_rgba8` (:7449), `pr_resources_are_owned_and_given_back` (:7548); the kick side in render.rs: `g1_a_zeroed_frag_pr_leaves_the_3d_command_words_as_todays` (:4177), `g3_with_the_knob_a_filled_frag_pr_builds_the_3d_pr_from_it` (:4220), `g4_the_pr_pbe_array_replaces_every_slot_of_a_four_target_frame` (:4267), `g2_the_frag_pr_block_is_the_last_field_and_fixed_width` (:4325) | covered (`1311a675`, `f1b3f8d7`, `62b08d4b`) |
| SPM scratch size per RT size (256x256, 800x600, 1600x1200, 4x MSAA) | src/pvr/spm.rs: `scratch_size_per_render_size` (:167), `scratch_size_holds_the_pr_store` (:234) | covered (`14fbb630`) |
| Background-load shader table entry decoded by our USC decoder | src/pvr/backend_tests.rs `spm_load_blob_matches_its_generator` (:7565): the blob, its entry offset, temps and constants against the generator's metadata (tools/pco_shc/spm_load/spm_load.meta.txt), and the entry read from `SPM_LOAD_ENTRY` | covered for the blob and its entry (`1311a675`). Not covered: a run of the shader on the GPU (never run on the board) |
| Depth/stencil words of the partial render | src/pvr/pr_zs.rs: `pr_zlsctl_swapped_stores_on` (:81), `pr_zlsctl_unswapped_equal` (:89), `pr_zlsctl_keeps_zonlyrender_and_loads` (:95), `pr_zs_words_copy_of_3d` (:106) | the builder is covered (`6f550632`); nothing calls it, so no kick carries its words (open item 1b) |
| The grow counters and the HWRT clear | freelist.rs: `a_refused_grow_counts_exactly_the_verdict_counters_and_asks_again` (:1424), `a_refused_grow_survives_with_room_and_stops_only_at_the_maximum` (:1449), `reset_hwrts_clears_the_partial_render_flags_and_leaves_the_rest` (:1466), `without_the_knob_partial_rendered_is_never_set` (:1505) | covered (`abe20b34`) |

The ring-wrap behaviour of the larger reservation has no PR-specific test; it
is the existing `ring_reserve` (render.rs:1037) with a larger `need`.

## What is open on parts 1 and 4

Status of each item of the 2026-10-30 list, checked against the tip
(3cb260c2) with git log, grep and the tests. A closed item names the commit
and the code or test that closes it; an open item names where the gap sits
today.

1. **The 3D_PR is no longer always a byte copy: closed in part, two pieces
   open.** Closed: `PvrRenderKick` has the `frag_pr` block (`62b08d4b`:
   src/osal/render.rs:111-157, driver/src/pvr_render.h:147, render.rs:124
   with the offset asserts :221-229, test
   `g2_the_frag_pr_block_is_the_last_field_and_fixed_width`). With the knob
   and a filled block the 3D_PR takes the PR PBE words (the whole array) and
   the PR EOT PDS data address from it, and the 3D and 3D_PR carry the
   scratch flag (`f1b3f8d7`: render.rs:1515-1541, test
   `g3_with_the_knob_a_filled_frag_pr_builds_the_3d_pr_from_it`). The
   backend fills the block: SPM colour scratch, PR PBE, PR EOT data and
   `pds_pr_bgnd`, with the load shader's entry (`1311a675`: src/pvr/backend.rs
   `pr_fill` :3574, `pr_load_init` :3415, `pr_eot_data_write` :3272,
   `pr_bgnd_words` :3533, src/pvr/spm_load_blob.rs; tests
   `pr_frag_pr_for_800x600_rgba8`, `pr_pds_pr_bgnd_only_with_the_option`).
   **Still open:** (a) `pds_pr_bgnd` is in `frag_pr` but is not written into
   the 3D_PR command (comment render.rs:168; driver/src/pvr_render.h:153);
   (b) the Z/S words of src/pvr/pr_zs.rs (`6f550632`) are not wired: `frag_pr`
   has no field for them and the 3D_PR keeps the frame's own Z/S words
   (docs/notes/pvr-pr-zs.md has the decision).
2. **The geometry PR-fence offsets are still not pinned: open.**
   `dk[13]`/`dk[14]` are set by count (render.rs:1458-1459); the comment at
   render.rs:1452-1457 says the DDK CMDTA struct is not in the vendored
   headers, and ddk_abi_pins.h has no `partial_render_ta_3d_fence` line
   (grep finds none).
3. **The part 4 pins: closed.** `asPRBuffer` and `RGXFWIF_PRBUFFER_*`
   (ddk_abi_pins.h:238-242), `aui32FreeListHWRSnapshot` (:304), the `sRTACtl`
   members (:306, :320-330), `HWRTDATA_PARTIAL_RENDERED` and
   `RTDATA_STATE_PARTIALRENDERFINISHED` (:328-331) are pinned in `833e75f0`
   and `1311a675` (the six remaining pins); tools/cts/pr-pin-audit.py
   (`1d1ab93e`) reports 51 constants, 51 pinned, 0 unpinned.
4. **The count array is not re-zeroed when a slot is rebuilt: open.** The two
   RTA arrays are zeroed only at first allocation (render.rs:986-993); a
   rebuild rewrites the valid array (render.rs:995-1004) and the HWRT data
   (render.rs:1023-1025) but not the count array. Nothing has run to show
   whether stale counts matter (plan #301 item hwrt-open-read).
5. **`reset_hwrts` and the RTA words: open.** It clears
   `HWRTDATA_HAS_LAST_TA | HWRTDATA_PARTIAL_RENDERED` and sets the state
   (freelist.rs:718-736; test
   `reset_hwrts_clears_the_partial_render_flags_and_leaves_the_rest`,
   `abe20b34`); it leaves the RTA counts and `ui32RTACtlFlags` alone. Whether
   upstream resets them is not answered in this tree (hwrt-open-read).
6. **The fence arithmetic is unmeasured: open.** The PR fence waits for
   timeline == `seq`, the geometry command for `seq - 1` (render.rs:1459,
   :1581, comment :1569-1580). The firmware's test is not readable; the only
   check is a board run, and none has run with the frame side.
7. **No golden proves the knob-off words unchanged: open.** The host tests
   compare knob on against knob off in the test rig
   (`without_the_knob_the_commands_are_todays` render.rs:4045,
   `the_knob_changes_only_the_pr_commands_and_the_geometry_fence_words`
   :4067, `g1_a_zeroed_frag_pr_leaves_the_3d_command_words_as_todays` :4177,
   `pr_knob_off_frag_pr_zero_and_words_unchanged` backend_tests.rs:7398).
   That is the rig, not the km render record of acceptance c4
   (c4-golden-read's finding, unchanged).
8. **The stall dump: open, decoders exist but are not wired.** The 10 s stop
   dump prints the last 3D command (`dump_last_frag_cmd`, render.rs:1929,
   called at :2453 and :2475) and names FENCE_PR in the pending client-CCB
   listing (render.rs:1970-1978), but not the timeline word, the 3D_PR body
   or the HWRT's PR state. The decoders are in
   driver/rust/src/gpu/pvr/pr_diag.rs (`ab60b8c5`; `pub mod pr_diag`,
   mod.rs:30), host-tested, and nothing outside the module calls them
   (plan #301 item pr-diag-wire). Closed on the log side: the firmware's debug
   trace groups are unmasked when the knob is on (`ec6b1302`: fwinit.rs:60,
   :71, :400), so the firmware's own partial-render lines reach the log.
9. **The RTA array length: open.** It is `RTA_MAX_RTS` = 3 (render.rs:71;
   comment :65-70: it counts render targets of a render-target array, not
   colour attachments, so it stayed 3 when `MAX_RT` became 4, render.rs:61).
   A null `ValidRenderTargets` is the firmware's invalid setup
   (rgx_fwif_sf.h:569); with the knob on it is non-null (test
   `with_the_knob_the_hwrt_data_carries_the_partial_render_accounting`,
   render.rs:4102). That 3 is the length the firmware expects for one render
   target is not checked (hwrt-open-read).

After this refresh: closed 3; closed in part 1 (frame colour side, the
fields and the PR PBE/EOT, `62b08d4b`, `f1b3f8d7`, `1311a675`). Open: 1a, 1b,
2, 4, 5, 6, 7, 8 (the wiring), 9.

## What a knob-on run can reach today

With `CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=y`:

- Every kick carries the FENCE_PR / 3D_PR / UPDATE trio and the geometry
  timeline words (render.rs:1458-1466, :1581-1583). The firmware's debug
  trace groups are unmasked (`ec6b1302`).
- When the backend ran with `PVR_BACKEND_OPT_PARTIAL_RENDER` and could set the
  load up (`pr_fill`, backend.rs:3574), the kick's `frag_pr` is filled and the
  3D_PR is `dk_pr`: the PR PBE words (the SPM scratch as the render target)
  and the PR EOT PDS data, with the scratch flag in the 3D and the 3D_PR. A
  partial render stores the tiles to the colour scratch. Without a filled
  block (a frame the load shader cannot reload, or a backend without the
  option) the 3D_PR is the byte copy of the 3D, as before.
- What a partial render still lacks: `pds_pr_bgnd` is not in the 3D_PR
  command, so nothing in the command loads the stored tiles back when the
  render resumes (item 1a); the depth/stencil words are the frame's own
  (item 1b). A depth-tested frame such as case 217 is therefore not expected
  to read back correctly from a resumed render.
- The at-maximum stop (`ccb.rs` `at_maximum`) is not gated by the knob: a
  refusal at the capped list still stops with cause 2 (record
  tools/cts/log/2026-11-02-pr-217probe/); plan #301 item pr-atmax-gate is the
  work on it.
- With the knob off nothing changes: no new command, no new word, the HWRT
  RTA words zero, `frag_pr` zero.

**Nothing with the frame side has run on the board.** The board records are
tools/cts/log/2026-10-31-pr-neg175/, tools/cts/log/2026-10-31-pr-negative/
and tools/cts/log/2026-11-02-pr-217probe/, all from images built before the
frame side (`f1b3f8d7`, `1311a675`). They show the parts 1 and 4 behaviour
only (case 175 knob on passed with one firmware partial render and no HWR);
no partial render's pixels were read. Whether the firmware accepts the new
3D_PR, runs the scratch store, and what a resumed render shows is unmeasured.
board-regress measures it and waits for the AM62P. The knob stays off by
default until the forced case passes on the board (the design's own
condition).
