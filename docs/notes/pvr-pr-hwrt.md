<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR partial render: the HWRT/RTA facts for open items 4, 5 and 9 (`pr-hwrt-note`)

Status: host-read only; nothing here is measured on the board. Written
2026-11-21 for plan #301 (issue #3), item pr-hwrt-note. Lines are as they are
at the tip a85e3112. The three questions are items 4, 5 and 9 of
`docs/notes/pvr-pr-landed.md`, "What is open on parts 1 and 4". Marks:
**read** (a header or source line), **inferred** (a conclusion from it),
**unread** (a source this note could not read).

## What could and could not be read

- DDK headers, **read** (`third_party/ti-img-rogue-ddk-25.2`): the layout and
  field comments of `RGXFWIF_RTA_CTL`, `RGXFWIF_HWRTDATA`, the HWRT flags and
  states. The vendored tree holds headers only: no firmware or DDK host code
  that fills or reads the RTA words. A grep of the whole tree for `RTACtl`,
  `RTANum`, `MaxRTs`, `ui32RTACtlFlags` finds the struct
  (include/rogue/rgx_fwif_shared.h:1218-1230) and one trace string
  (include/rgx_fwif_sf.h:569). No `RTACtlFlags` bit is defined anywhere.
- Upstream Linux `pvr_hwrt.c` and `pvr_free_list.c` at v6.12: **unread**.
  The worker had no network (the fetch was refused) and no copy is on the
  machine. What this tree already records of `pvr_free_list_reconstruct`
  (docs/notes/pvr-render-pool.md:1158-1177, read there from Linux 7.3-rc5)
  is used: upstream's reconstruction puts every HWRT data of the list into
  `RGXFWIF_RTDATA_STATE_HWR`, clears `HAS_LAST_GEOM` (our
  `HWRTDATA_HAS_LAST_TA`), and writes the stack top and the two allocated
  counts. That record names no RTA word. Whether `pvr_hwrt.c` writes
  `sRTACtl` at creation is **unread**; no verdict below leans on it, and the
  last section says what reading it would settle. Nothing is copied.

## The structure, read

`RGXFWIF_RTA_CTL` is 32 bytes (rgx_fwif_shared.h:1218-1235, static_assert
:1232-1235), at offset 112 of `RGXFWIF_HWRTDATA` (member at :1277; pinned as
`HWRTDATA_RTA_CTL` = 112, fwif.rs:320):

| offset | member | header comment (rgx_fwif_shared.h) | writer |
|---|---|---|---|
| 0 | `ui32RenderTargetIndex` | "Render number" (:1219) | firmware (**inferred**) |
| 4 | `ui32CurrentRenderTarget` | "index in RTA" (:1220) | firmware |
| 8 | `ui32ActiveRenderTargets` | "total active RTs" (:1221) | firmware |
| 12 | `ui32CumulActiveRenderTargets` | "total active RTs from the first TA kick, for OOM" (:1222-1223) | firmware |
| 16 | `sValidRenderTargets` | "Array of valid RT indices" (:1225) | host |
| 20 | `sRTANumPartialRenders` | "Array of number of occurred partial renders per render target" (:1226-1227) | firmware counts, host allocates |
| 24 | `ui32MaxRTs` | "Number of render targets in the array" (:1228) | host |
| 28 | `ui32RTACtlFlags` | "Compatibility and other flags" (:1229) | unspecified |

What we build (driver/rust/src/gpu/pvr/render.rs): two arrays of
`RTA_MAX_RTS` dwords per HWRT slot, allocated once (`rta_valid_off`,
`rta_partial_off`, :247, :269), zeroed at that allocation only (:986-993),
the valid array rewritten 0, 1, 2 on every build (:995-1004), the two
addresses and `ui32MaxRTs` written at :1022-1025. Nothing writes offsets
0-12 or `ui32RTACtlFlags`: they hold what the allocation holds. `fw_alloc`
does not zero (:420-422); the HWRT data itself is zeroed only when
`hwrt_reinit` is set (:973-976), so a recycled slot keeps every word the
firmware left, RTA words included.

## Item 4: the count array is not re-zeroed when a slot is rebuilt

**Verdict: unknown until a board run. A defensive change is cheap and
harmless; recommended with the next render.rs edit.**

Evidence:
- The array is the firmware's tally of "occurred partial renders per render
  target" (rgx_fwif_shared.h:1226-1227); the host never writes it after
  allocation (render.rs:994-995: "the firmware counts them").
- Whether the firmware zeroes the counts when a render completes is in its
  code, not in the headers: **unread**. The only header statement about
  resetting state is the HWR note, that the state is not set to NONE so that
  every related TA is skipped (rgx_fwif_shared.h:1166-1168).
- A rebuild re-zeroing covers only a slot rebuilt for a new size or sample
  count (`fw_built` already 1, render.rs:963-966). A slot reused unchanged
  for the next frame is not rebuilt, so a count the firmware leaves stale
  between frames is not reached by this change at all (**inferred**); only a
  board run shows that case.
- Harm if stale, knob on: the firmware sees more partial renders on that
  target than happened. The count's plausible consumer is a per-target
  decision about partial renders, which a stale value could misdirect
  (**inferred**). With the knob off the arrays do not exist (render.rs:985).

Implied change (not made here; render.rs is held by another item): in the
`pr` block, write `RTA_MAX_RTS` zero dwords over `rta_partial_off` on every
build, then flush: the `write_bytes` and `k.flush` of render.rs:990-992,
moved out of the `if *off == 0`. No knob-off word can move; the cost is 12
bytes per rebuild. A host test next to
`with_the_knob_the_hwrt_data_carries_the_partial_render_accounting`
(render.rs:4102) seeds the array non-zero and rebuilds.

## Item 5: `reset_hwrts` leaves the RTA counts and `ui32RTACtlFlags` alone

**Verdict: counts unknown until a board run; `ui32RTACtlFlags` harmless as
the host uses it. No change now.**

Evidence:
- `reset_hwrts` (driver/rust/src/gpu/pvr/freelist.rs:718-736) writes state
  `HWR` and clears `HAS_LAST_TA | PARTIAL_RENDERED` (:731-733). The flags
  are rgx_fwif_shared.h:1151-1152, the state :1166-1176.
- Upstream's reset as recorded here (pvr-render-pool.md:1161-1166): state,
  the has-last-geometry flag, the freelist counts; **no RTA word**. Ours adds
  `PARTIAL_RENDERED`, which upstream has no use for (an extra bit cleared,
  never one left set). `pvr_hwrt.c` being unread, whether upstream clears the
  counts elsewhere is **unread**.
- A reconstruction runs only after a context reset (FREELISTS_RECONSTRUCTION,
  pvr-render-pool.md:1152-1156). A render that was partial-rendering at the
  reset leaves stale counts exactly as item 4 describes. Whether the
  firmware resets them on leaving `HWR` is **unread**; the host cannot tell
  without a dump.
- `ui32RTACtlFlags` (:1229, offset 28): no bit is defined in the vendored
  tree, and the host never writes it (pinned `RTA_CTL_FLAGS` = 28,
  fwif.rs:326; the only reader is the stall-dump decoder,
  pr_diag.rs:105-107). Clearing it in `reset_hwrts` would invent a rule.

Implied change: none now. If the board run of item 4 shows the counts stale
after a reset, `reset_hwrts` zeroes `RTA_MAX_RTS` dwords per reset member.
The member's HWRT data is already read there (:722-726) and the array's
firmware address sits in it at `HWRTDATA_RTA_CTL + RTA_CTL_NUM_PARTIAL_RENDERS`,
so no render-service state is needed. A test seeds the array beside
`reset_hwrts_clears_the_partial_render_flags_and_leaves_the_rest`
(freelist.rs, tests near :1469-1524); its state and flag assertions stay.

## Item 9: `RTA_MAX_RTS` = 3

**Verdict: harmless (internally consistent). Whether 3 is the number the
firmware wants for a non-layered render is unknown until a board run.**

Evidence:
- `ui32MaxRTs` is the "Number of render targets in the array"
  (rgx_fwif_shared.h:1228); the array is the render-target array, meaning
  layers, not colour attachments (the sibling members count "Render number",
  "index in RTA", "total active RTs", :1219-1223). render.rs:65-71 says the
  same and keeps 3 while `MAX_RT` is 4 (:61); docs/notes/mrt4-v3d-pvr.md:286-287
  agrees.
- Consistency holds: both arrays are `4 * RTA_MAX_RTS` bytes (render.rs:989),
  the valid array holds 0, 1, 2 (:1000-1002), `ui32MaxRTs` is the same 3
  (:1025). A firmware loop bounded by `ui32MaxRTs` stays inside both arrays.
  The valid array is non-null, the one condition the DDK names as an error
  (rgx_fwif_sf.h:569).
- What the headers do not give: a default or bound for a non-array render.
  The host sets no layer count; `ui32ActiveRenderTargets` is whatever the
  allocation holds (0 on a fresh slot). A loop over the active count reads
  nothing; a loop over `ui32MaxRTs` reads three indices and three counts,
  all in the arrays. Neither loop is readable here (**inferred**).
- Upstream's `ui32MaxRTs` is **unread**. The upstream uAPI's HWRT-dataset
  create call carries a layer count, so a single-layer dataset plausibly
  yields 1 (**inferred** from the API's shape only, not from code).

Implied change: none now; keep 3. If the board dump shows
`ui32ActiveRenderTargets` or `ui32CurrentRenderTarget` non-zero on a
non-layered render, or the firmware trace names an RTA problem after a
refused grow, the change is `RTA_MAX_RTS` = 1: one constant (render.rs:71),
knob-on only because the arrays are written only under the knob, and the
test at render.rs:4102 names the count.

## What settles all three

One observation no host test gives: the eight `sRTACtl` words and the
three-dword count array on one HWRT slot at four points: after the first
build, after a refused-grow render (case 175 or 217, knob on), after the next
frame on the same slot, and after a context reset with a reconstruction. The
dump decoder prints the `sRTACtl` words raw (pr_diag.rs:105-129) and is wired
by item pr-diag-wire; the count array is not in it and would need adding
(read `HWRTDATA_RTA_CTL + RTA_CTL_NUM_PARTIAL_RENDERS`, then three dwords).
Reading `pvr_hwrt.c` at v6.12 on a machine with network (what it writes into
`sRTACtl` at creation, and whether the reconstruction touches it) would
answer upstream's `ui32MaxRTs` (item 9) and whether the host ever writes the
counts (items 4 and 5).

## Summary

| item | verdict | change it implies |
|---|---|---|
| 4 counts not re-zeroed on rebuild | unknown until a board run; the fix is harmless | zero the count array on every build inside the knob block (render.rs:986-1004) |
| 5 `reset_hwrts` leaves counts and flags | counts unknown until a board run; flags harmless | none now; zero the array in `reset_hwrts` if the board shows stale counts |
| 9 `RTA_MAX_RTS` = 3 | consistent, harmless; the right number unknown | none now; 1 if the board dump asks for it |
