<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR partial render: depth and stencil (`pr-zs`)

Status: decision, host-read only; nothing here is measured on the board.
Written 2026-10-30 for plan #148 (issue #3), core part 3c. Lines are as they
are at the tip d296b722. The reading it rests on is
`docs/notes/pvr-render-pool.md`, "The PR frame side, read (Mesa 26.2.1)"
(cited below as RP), parts (1) and (2). Marks as in RP: **read**,
**inferred**.

## The decision

- **Unswapped frame: no word changes.** Neither the 3D_PR nor the normal 3D
  command gets a new depth/stencil word. The PR stores into, and the
  resumed render reloads from, the frame's own depth/stencil buffer: the PR
  command keeps the normal `ZLOAD_BASE` / `STENCIL_LOAD_BASE` (RP (2), read;
  Mesa pvr_arch_job_render.c:1009-1027, copied at :1389). The base words
  and the extents are the ones we already write
  (`src/pvr/backend.rs:8301-8317`). The firmware forces the store and load
  bits itself (RP (1), **inferred** from the trace strings SPM 1/2 and
  SPM 17-20, MAIN 47/48).
- **Swapped frame: store bits on in the 3D_PR.** For a swapped frame
  `swap_drops_zs` (set at `src/pvr/backend.rs:8621`, cleared at :8574) makes
  the frame's `isp_zlsctl` carry the store bits off
  (`isp_zlsctl_stores(.., !b.swap_drops_zs)`, :8301), because nothing reads
  the depth after the swap. A partial render does read it: the resumed
  render loads what the PR stored. So the 3D_PR's `isp_zlsctl` is packed
  with `store = true` (`forcezstore`, `zstoreen`, `sstoreen`, packed by
  `isp_zlsctl_stores`, :691-709); the normal 3D keeps today's word (store
  off), so a frame that never partial-renders is byte for byte unchanged.
  The depth is then dropped at the last 3D as today (:8570-8578).
  RP (1) leaves open whether the firmware forces the store itself; if it
  does, the change is harmless, if it does not, it is required.

## `asPRBuffer` stays 0

`CMDTA3D_SHARED.asPRBuffer` (dk[2]/dk[3] in our block) is 0 in the geometry,
3D and 3D_PR commands. Neither Mesa nor Linux writes it (RP (1), read:
pvr_rogue_fwif_shared.h:151 is a declaration only; Linux pvr_job.c:195-198).
The type is the on-demand Z/S/MSAA buffer, `RGXFWIF_PRBUFFER`
(`third_party/ti-img-rogue-ddk-25.2/include/rogue/rgx_fwif_shared.h:122-146`:
`PRBUFFER_ZSBUFFER` 0, `bOnDemand`, state UNBACKED/BACKED/..._PENDING). A
non-zero slot lets the firmware ask the host to back and unback that
buffer: FWCCB `RGXFWIF_FWCCB_CMD_ZSBUFFER_BACKING` (101) and
`..._UNBACKING` (102) (`include/rogue/rgx_fwif_km.h:1178-1185`). Our
`fwccb_service_with` (`driver/rust/src/gpu/pvr/ccb.rs:183`) answers neither
(it handles FREELIST_GROW, FREELISTS_RECONSTRUCTION, CONTEXT_RESET and
UPDATE_STATS, among others), so the partial render would wait for an
answer that never comes: the hang this plan removes (pr-fwccb-read's
finding). A later item that sets the slot must first answer 101/102.

## The open measurement (board, not part of this item)

Settle on the AM62P with the forced-PR case (`gl_test` case 217) and the
firmware trace on (`fwinit-log-unmask`, `fw-trace-pr-lines`):
1. Which `ZLSCTL` bits the firmware forces for a PR: look for the SPM 1
   "Force Z-Load for partial render", SPM 2 "Force Z-Store ..." and SPM
   17-20 "ZStore/SStore/ZLoad/SLoad address" lines (DDK
   `include/rgx_fwif_sf.h:1499-1502, 1533-1540`; ids marked deprecated, so
   they may not print) and MAIN 47/48 (:208-213).
2. Depth read-back across the PR boundary (overlapping triangles) in an
   unswapped and in a swapped frame, with the store-bit change off and on.
   Expected: unswapped correct either way; swapped correct only with the
   store bits on, if the firmware forces only the load.

If (1) shows the firmware forces both, the swapped-frame change can be
dropped; keep it until then, it costs one store.

## The change list for `pr-zs`

Test names follow the "Host tests, landing with each part" list of
`docs/notes/pvr-render-pool.md` (backend.rs group); the names below are new.

1. **The one store-bit site.** `src/pvr/backend.rs:8301`: the `frag_pr`
   block's `isp_zlsctl` is `isp_zlsctl_stores(tx, ty, load_depth,
   load_stencil, true)` (ORed with `isp_zlsctl_zonlyrender()` as the 3D
   does), independent of `swap_drops_zs`. The normal 3D line stays.
   Tests: `pr_zlsctl_swapped_stores_on` (swapped: the 3D_PR has the three
   store bits, the 3D has none); `pr_zlsctl_unswapped_equal` (unswapped:
   the 3D_PR `isp_zlsctl` equals the 3D's).
2. **No other depth word.** The 3D_PR `ZLOAD_BASE`, `STENCIL_LOAD_BASE`,
   `isp_bgobjdepth` and `cmd_flags` (DEPTHBUFFER|STENCILBUFFER, :8318)
   equal the 3D's, in packed and `split_zs_on` frames. Test:
   `pr_zs_words_copy_of_3d`.
3. **`asPRBuffer` stays 0.** Geometry, 3D and 3D_PR dk[2]/dk[3] are 0.
   Test: `pr_prbuffer_slots_zero`.
4. **Knob off unchanged.** With the knob off no `frag_pr` is built and the
   kick bytes are today's. Test: `pr_zs_knob_off_unchanged` (the c4 goldens
   also cover it).

Out of scope here: the PR PBE and EOT words (`pr-frame-colour`), the load
shader and `pds_pr_bgnd` (`bgload-pds`), the kick ABI (`kick-abi`).
