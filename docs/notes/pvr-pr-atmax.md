<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The partial render at a list's maximum: the finding, the driver change, the fallback

Host reading plus two board records, 2026-11-21 (plan #301 item
pr-atmax-note, issue #3). It says what the firmware is expected to do after
an out-of-memory notice at a free list that is at its maximum, what the
driver's at-maximum stop must do with the knob
(`CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER`) on, and what to do if the board
shows otherwise. Each statement is marked **measured** (a board log in the
tree), **read** (a header or source line) or **inferred** (neither).

## (a) The finding

1. **The host has no command that starts a partial render. Read.** The
   kernel-CCB command list
   (`third_party/ti-img-rogue-ddk-25.2/include/rogue/rgx_fwif_km.h:906-1002`)
   has kick, MMU cache, cleanup, ZS-buffer backing updates, freelist grow
   update, reconstruction update, `DISABLE_ZSSTORE` and `CANCEL_WORK`; none
   is a "render partially now". The `DISABLE_ZSSTORE` entry
   (`rgx_fwif_km.h:944-946`) is the only render-related one and it
   forbids a store, it does not start one. So a partial render is the
   firmware's own decision.
2. **At a full list the firmware has no grow to ask for. Read.** The list
   record carries `ui32MaxPages`, `ui32GrowPages`, `ui32CurrentPages`,
   `bGrowPending` and `ui32ReadyPages`
   (`.../rogue/rgx_fwif_shared.h:951-965`). The firmware's SPM states and
   messages (`wait for grow`, `wait for pending grow`, `received grow
   update`, `OOM TA/3D PR check`, `PR deadlock unblocked`, ...,
   `third_party/ti-img-rogue-ddk-25.2/include/rgx_fwif_sf.h:1569-1630`) are
   the ones a grow-bounded list goes through. A list whose current pages
   equal its maximum has nothing to grow by, so the firmware's request for
   one cannot be made. **Inferred** from the fields and message names: the
   firmware's source is not in the tree.
3. **Measured: the firmware sends the notice alone at the maximum.** Case 217
   with the list capped at 512 pages and the knob off or on: one
   `UPDATE_STATS element=2 (out of memory)`, no `FREELIST_GROW`, then the
   driver's stop (`tools/cts/log/2026-11-02-pr-217probe/README.md`, runs A, B,
   S, T: `grow requests 0`, `notices 1`, `stop 2`, `partial renders 0`).
4. **Measured: the knob-on firmware rendered partially after a refused grow.**
   Case 175 (list not at its maximum), knob on: the grow was refused, one
   `UPDATE_STATS element=1 (partial renders)` followed, the kick completed,
   0 HWR, 0 resets; knob off: the same refusal ended in the 10 s stop, cause
   4, `partial renders 0`
   (`tools/cts/log/2026-10-31-pr-neg175/README.md`, runs A1-B2). Case 217 in
   run C's configuration (no reserve, no cap) did the same
   (`.../2026-11-02-pr-217probe/README.md`, run C).
5. **Expected, not measured: with the knob on, the firmware renders
   partially by itself after an out-of-memory notice at a list at its
   maximum.** What links 3 and 4: the firmware's SPM logic reacts to the
   out-of-memory state, with or without a grow request (item 4 shows it does
   so after a refused one), and nothing in item 1's command list says the
   host must start it. Nothing measured shows it for the at-maximum case,
   because the driver's at-maximum stop (`ccb.rs`, `at_maximum`, line 402,
   not gated by the knob) ends the render service in the same FWCCB batch
   and the host stops reading the FWCCB. Whether the firmware would have
   rendered partially after the notice at the maximum is unknown until a run
   lets it.

## (b) The driver change

With the knob on, `at_maximum`'s stop
(`driver/rust/src/gpu/pvr/ccb.rs:402`, `freelist::STOP_AT_MAXIMUM`,
`freelist.rs:230`) becomes a logged wait, bounded by the existing
`STOP_WAIT_MS` (10 s) stop of `render.rs:2308`, which ends a run whose fence
did not move as cause 4. With the knob off the rule is unchanged.

The warning text, exactly:

```
render: at maximum (list <id>, <pages> pages), awaiting the firmware's partial render for up to 10 s
```

Outcomes the board tells apart:

- **A partial render follows** (`UPDATE_STATS element=1`, the kick completes,
  `stop 0`): item 5 holds and the knob-on run at the cap passes.
- **No partial render, cause 4 at 10 s** (`partial renders 0`, `stop 4`): item
  5 is wrong; use (c).
- Cause 2 must no longer occur with the knob on; one is a defect in the change.

The knob-off run is the known negative in both cases (case 217, runs A and S
above: cause 2).

## (c) The fallback

If the board shows cause 4 instead, the at-maximum list cannot be rescued by
waiting. Case 217 is then run in **run C's configuration**: no grow reserve
(`FL_PROBE_NO_RESERVE=y`), no cap (`FL_PROBE_MAX_PAGES=0`), 1220 layers, list
at 448 + 64 of 4096 pages (`.../2026-11-02-pr-217probe/README.md`, image C).
There the firmware asks for a grow, is refused, and renders partially
(**measured**, run C), the case that case 175 already proves.

The 512-page cap cannot work as the case's configuration there: the list
starts full, so the firmware has no grow to ask for (item 2). Acceptance
c2's wording names the cap, so using run C's configuration **needs the
owner's amendment** of c2; this note does not make it.

## What is measured, what is inferred

| statement | status |
| --- | --- |
| No host command starts a partial render | read (`rgx_fwif_km.h:906-1002`) |
| Notice alone at a capped list, then the driver's stop | measured (217 probe A, B, S, T) |
| Knob-on partial render after a refused grow, 0 resets | measured (neg175 B1, B2; 217 run C) |
| Knob-on partial render after a notice at the maximum | **inferred, not measured** |
| The firmware has no grow to request at a full list | read as fields, inferred as behaviour |
| The colour of case 217 after the partial render | measured wrong in run C (frame side not landed then; see the probe record) |
