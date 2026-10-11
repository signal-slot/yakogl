<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The PowerVR kernel side in Rust: the host check

docs/source-form.md step 7 moved the PowerVR kernel side to Rust
(`driver/rust/src/gpu/pvr`: `mem`, `mmu`, `ccb`, `freelist`, `fwinit`,
`render`; driver/docs/pvr-kernel-rust.md has the inventory). This directory
proves, before any board, that the Rust does what the C it replaced did.

| file | what |
|---|---|
| `rec.h` | the recording platform: a 64 MiB carveout at 0xC0000000 in host memory, a register file, and the record both sides write (cleans `F`, invalidates `I`, barriers `B`, waits `D`, register writes `W` and reads `R`, log lines `L<level>`, results `=`, and at the end every non-zero word of the carveout `M`) |
| `scenario.h` | the scenarios, over a table of functions each side fills: `mem` (the allocators, the page tables), `ccb` (130 KCCB posts, the FWCCB service: grows from a pool and from a reserve, refusals, reconstructions, a context reset, an unknown command; the freelist audit), `fwinit` (the FWIF structures, the boot wait: started on the third tick, then a timeout with its trace and assert dump), `render` (a firmware model takes the kicks: renders of several sizes and sample counts, separate and combined kicks, the member-in-flight and UFO-ring waits, a dropped kick and the re-post, the MMUCACHE probes, a hardware recovery, two stalls with their dumps, HWRT recycles with and without the cleanup, a refused cleanup, the client CCBs round the ring) |
| `record_c.sh`, `record_c.c` | cut the C out of origin/main 674ea1cf (`pvr_fw_boot.c`: allocators, MMU, FWIF globals, CCB, freelists, FWIF setup, boot wait, accessors; `pvr_render.c` whole) and run the scenarios on it; `record_c_verbose` is the same with `PVR_RENDER_VERBOSE` |
| `km_check.c` | the same scenarios on the Rust (`libyakogl_drivers.a` for the host, feature `pvr`), and `km_check pins`, the Rust layout numbers as `PIN(value, expression)` lines |
| `pins_check.c` | compiles those lines into `_Static_assert`s against the vendored DDK headers (driver/tools/offsetof's way), compares the alignment-check table with `RGXFW_ALIGN_CHECKS_INIT_KM` and the MH tag names with `rgxmhdefs_km.h` |
| `rec_normalize.py` | sorts each run of consecutive `R`/`I` lines (see below) |
| `records.sha256` | the hashes the records must have (the records are not committed: about 2 MB of text) |
| `fwinit-step7.diff` | the object table of the FWIF structures before and after the switch to the DDK sizes |
| `hostshim/` | the Zephyr headers the cut-out C includes |

`tools/check.sh` (row "pvr km_check") builds the Rust side, runs the pins and
every scenario, and requires the hashes. To see a difference instead of a
hash mismatch, rebuild the C records (`tools/pvr/km/record_c.sh <dir>`) and diff
them against `build/pvr-check/km/`. `record_c.sh` reads the C with
`git show 674ea1cf:...`, so it needs that commit in the object store. It is
not an ancestor of main: main's history starts at the release commit
`afb16335`, and 674ea1cf is reachable only from older branches
(`origin/ccfarm/*` held it on 2026-10-11) or the pre-release history. The
gate needs only the hashes, not the commit.

## What "identical" means here

- `mem`: the Rust record is the C record, byte for byte (11,749 lines:
  every page-table entry, every allocation, every clean and barrier in
  order, the doorbell writes and readbacks, every log line).
- `ccb`: the same, byte for byte, up to three deliberate differences
  (26,313 lines; the C's are 26,261). After list 1 grew by 1024 pages from the
  shared grow pool, the C's reconstruction listed `backing + j` for every
  page. That gave the PM PFNs 0xc1430..0xc182f: 4 pages nobody owns, then
  1020 pages of list 2's backing (list 2 starts at 0xc1434). The C's
  freelist audit caught it: "= fl audit 1024", then "1026". The Rust
  reconstruction lists each grow's own pages (`freelist::page_phys`, the
  table `fl_grow_pfn`): the 1024 entries at 0x1432000..0x1432ffc are the
  pool's PFNs 0xc0830..0xc0c2f. Its audit accepts a grow's pages, so it
  reports only the scenario's two injected bad entries in list 3 ("=
  fl audit 0", then "2"); the C's log showed list 1's first four and hid
  them. The rest of the record is the C's, up to a second deliberate
  difference: the reconstruction now follows Linux drm/imagination's
  `pvr_free_list_reconstruct`. It invalidates each list's firmware
  structure before reading the firmware's current count (28 more `I` lines,
  e.g. `I 0xa7630 64`), and writes the stack top from that count. List 1's
  stack top (`0xa7640`) ends at 0x7ff (the firmware's 2048 current pages
  - 1), where the C wrote 0xbff (its own 3072 listed - 1). The current,
  ready and grow-pending fields are no longer written; in this scenario
  they held the values the C wrote anyway, so no other word moves. Each
  reconstruction then says what it did (18 lines, e.g. "freelist id=1
  reconstructed: 3072 pages listed; firmware current 2048 ready 0, stack
  top 2047; 0 HWRT datas reset to HWR").
  The third: the render service's stops apply without the grow knob too
  (tools/cts/log/investigations/pvr-render-pool.md, the common set's round 2).
  The scenario asks list 6, at its maximum of 16 pages with no room for a
  grow, to grow 22 times. Each "no grow (reserve 8, pool 1024/1024, 16/16
  pages)" is now followed by "render: stopped: the parameter manager out
  of memory with its freelist at the maximum (list 6, 0 KiB); ..." (22
  lines). The scenario waits for no fence, so nothing else moves.
- `fwinit`: at the former allocation sizes (`PVR_LEGACY_SIZE_*`: the CCB
  controls at 24 bytes, the trace buffer at 864, the GPU utilisation at 10280)
  and the former object base (0xa1000) the Rust record is the C record, byte
  for byte (11,311 lines). The driver builds them at the DDK's sizes from the
  DDK's host main heap (0xc0000); that record is `fwinit-step7`, and its
  difference from the former is `fwinit-step7.diff`: the objects start
  0x1f000 higher, the CCB controls are 8 bytes smaller, the trace buffer 424,
  the GPU utilisation 24; the fault page is 4 KiB aligned, so what follows it
  keeps its distance from the base and the heap ends at 0xd8d28 (was
  0xb9d28).
- `render`, `render-verbose`: identical once each run of consecutive register
  reads and invalidates is sorted (33,256 and 21,363 lines). The render
  service's diagnostic log lines read registers and invalidate buffers inside
  their argument lists; C leaves the order of argument evaluation unspecified
  (the host compiler evaluates right to left) and Rust evaluates left to
  right. Every write, barrier, clean, wait, log line and result keeps its
  order. One deliberate addition to `render`: when the FWCCB batch that
  holds the scenario's grow request is first read, two invalidates (the
  fragment UFO and the geometry UFO ring); after the batch, the line "render:
  at grow request 1 (sampled when the batch was first read): last submitted
  34, last fragment done 33; 1 kicks in flight: 0 with TA done and 3D
  pending, 1 with TA pending" (`render::InFlight`). Nothing else moves.

## Not covered

- The render pool's free path from a recycled HWRT that grows (`free_bo`):
  the scenarios keep every target at most 1024 wide, where the buffers never
  grow. The callback is one line (pvr_render.c `rr_free_bo`).
- The firmware model is ours: it follows the protocol (the kick sets the
  client CCB write offset, UPDATE writes, FENCE compares), not the firmware's
  timing. The board runs are the proof against the real firmware.
