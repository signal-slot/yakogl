<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The firmware's two windows: cached 0xC and uncached 0x4

Status: finding of 2026-10-25. It comes from a host-side study of the
firmware image, the vendored DDK header and our host code, and it backs
the open cause in the render-pool record (`pvr-render-pool.md`: a
firmware period of 5 and 8 ms delays the firmware's taking of our KCCB
post through the cached window, "Its cause is open"; the empty FWCCB
slot that fills 240 us after the write offset passed it). The knob is
`CONFIG_YAKOGL_DRV_GPU_PVR_FW_UNCACHED` (`driver/zephyr/Kconfig:121-138`).

Claims below are labelled: **shown** (code, header or binary), **read**
(a mechanism the shown facts support, not measured), **open**.

## The windows are part of the design

The firmware core reaches memory through 16 remap windows of 256 MiB:
a firmware address with region `r` remaps to the window's device
address (`rgx_riscv.h:51-57`). The 32 MiB firmware heap
(`RGX_FIRMWARE_RAW_HEAP_BASE/SIZE`, `fwif.rs:57-59`) is reachable
through four pairs of windows named in the header (`rgx_riscv.h:102-148`):

| region | header name |
|---|---|
| 0xC, 0xD, 0xE, 0xF | `RGXRISCVFW_SHARED_CACHED_DATA0..3` |
| 0x4, 0x5, 0x6, 0x7 | `RGXRISCVFW_SHARED_UNCACHED_DATA0..3` |

Shown: the names — the same heap has a cached half and an uncached
half of the core's address map, by design. Our knob hands the firmware
one or the other for every object the host shares with it:
`heap_fwaddr` ORs the heap offset with 0xC000_0000 or 0x4000_0000
(`mem.rs:108-110`, `fwif.rs:62-69`), and the tagged values are what the
firmware is given in `RGXFWIF_OSINIT`: `OSINIT_KCCB_CTL`, `OSINIT_KCCB`,
`OSINIT_KCCB_RTN_SLOTS`, `OSINIT_FWCCB_CTL`, `OSINIT_FWCCB` are written
with `heap_fwaddr` values (`fwinit.rs:73`, `:222-226`; the fields
`fwif.rs:212-216`). The same tagging covers the init blocks, the
freelists, the HWRT data and the client CCBs and UFOs. The boot code
that programs the window remaps is the firmware's own: nothing in the
vendored DDK host code touches the shared windows
(`rgxstartstop.c`: no `SHARED_CACHED`/`SHARED_UNCACHED` write), and the
comment at `pvr_fw_boot.c:264-277` records the boot code reprogramming
its data window itself. The host's own mapping of the carveout is
`K_MEM_ARM_NORMAL_NC` either way (`pvr_fw_boot.c:580-603`): the
host-side twin defect — the host CPU writing back its stale copy of the
shared CCB control line — is what Normal-NC fixed on silicon (the
comment at `pvr_fw_boot.c:584-592`).

## What the board records show (measured, `pvr-render-pool.md`)

- Cached window: 5 of 20 same-tree runs read an empty FWCCB slot whose
  words appeared 240 us after the write offset had passed the slot;
  uncached: 0 of 20 (Fisher one-sided p = 0.024; over all 22 cached
  runs p = 0.031). Every event is the GROW ack type 0x2abc0067, at
  fixed request numbers 7 and 15 of a deterministic ack schedule.
  (`pvr-render-pool.md:4175-4270`.)
- Grow acks take 5 and 8 ms cached — a fixed schedule, not noise —
  against 1 ms uncached. `ActivePMLatency` is ruled out as their
  cause: with PM latency 10 ms the acks stay 5 and 8
  (`pvr-render-pool.md:4271-4305`).
- Without the re-read instrument (`FWCCB_CHECK`, `ccb.rs:183-385`) the
  empty slot was skipped and its command lost, and a refused grow hung
  (`ccb.rs` test `without_the_check_the_empty_slot_is_skipped_and_its_command_lost`).
- Linux drm/imagination hands the firmware the uncached window
  (Kconfig help; `PVR_BO_FW_FLAGS_DEVICE_UNCACHED`).

## What the firmware image shows

Studied on the host from the gitignored blob `driver/src/fw/pvr_fw_blob.h`
(local copies and scripts: `build/fwprobe/`, gitignored scratch). The
image is TI and Imagination property: no byte of it, no disassembly of
it, is committed. The 32-bit decode below is exact; compressed forms
are decoded only where anchored, so parts of the listing show raw
halfwords. Fence, cbo and CSR counts come from a whole-image scan at
every 2-byte-aligned position, which cannot miss a 32-bit instruction
wherever it sits.

- Image structure (ELF, machine 243): code sections link through the
  windows — `.uncached_code` at 0x3000_0000 (64 KiB; region 0x3, the
  boot loader code, `rgx_riscv.h:74-75`), `.coremem_code` at
  0xb000_0000 (16 KiB) and `.cached_code` at 0xb000_4000 (112 KiB) —
  the main code lives in window 0xB, a region the header does not name.
  Data links through region 0xA (`rgx_riscv.h:76`): `.boot_data` at
  0xa000_0000, `.shdata` at 0xa000_1040, `.shdatabss` at 0xa000_1180,
  `.rodata_noncritical` at 0xa002_0030.
- Declared ISA (`.riscv.attributes`): `rv32i2p1_m2p0_c2p0_zicsr2p0_zifencei2p0_zmmul1p0`,
  built with GCC 13.2.0 (`.comment`). So: compressed code yes, cache
  management instructions no.
- Instructions found: 202 `fence iorw,iorw` (0x0ff0000f), one
  `fence.i` (0x0000100f, Zifencei: opcode MISC-MEM, funct3 001) at
  0x3000_0034, which syncs the instruction stream and touches no data
  cache line, **zero** `fence.tso`, **zero** `cbo.*` (matched on the
  exact Zicbom encoding: MISC-MEM, funct3 010).
  The firmware orders its accesses with fences. A fence cannot pull a
  dirty cache line out of the core's cache, and cannot drop a stale
  one; only the CSRs below can move lines.
- The core has its own data cache and bus faults around it: the
  vendor's own MCAUSE table lists "Machine D-bus store error NMI",
  "Machine D-bus non-blocking load error NMI" and "dCache
  unrecoverable NMI" (`rgx_riscv.h:216-231`), and its debug-dump
  register list names `micect` (0x7f0), `mdcect` (0x7f3), `mdcrfct`
  (0x7f4) (`rgx_riscv.h:190-205`). The header gives only these short
  names, without comments; what the registers hold is **open**.
- Boot setup (exact): at 0x3000_0010 the boot loader loads 0x1000_0000
  — the base of region 0x1, `RGXRISCVFW_CUSTOM_HEAP_DATA_BASE`
  (`rgx_riscv.h:150-152`, the firmware's own heap window) — and writes
  it to `micect`, `mdcect` and `mdcrfct`; then writes 2 to 0x7cc and
  2 to 0x7cf (0x3000_002c-30), 0x1500_002a to 0x7c0, and `csrrwi` 1 to
  0x7c6 (0x3000_0060; also at 0x3000_100c, 0xb000_09f8, 0xb000_4032,
  0xb001_7226). At 0xb000_0a02 (in `.coremem_code`) the main image
  rewrites 0x1000_0000 to the same three configuration tables. What
  the table bits mean per region is **open** (core internals).
- Line maintenance through CSRs (exact, 49 sites each for 0x7cc and
  0x7cf). The clearest one, 0xb000_a5a6-0xb000_a5d4: round a range to
  64-byte lines (`addi +223` then `andi -64` up, `andi -64` down), a
  `fence`, then per line write `line_address | 1` to CSR 0x7cc and 6
  to CSR 0x7cf, step 64, loop to the end, `fence`. These loops sit
  inside the command handlers, on addresses that come from the host:
  a handler bounds-checks a received pointer with
  `addr & (0x4000_0000 - 1) < 0x0200_0000` (mask built at
  0xb000_a4e8-0xa4f8, bound compared at 0xb000_a590; the bound equals
  `RGX_FIRMWARE_RAW_HEAP_SIZE`) — it strips the window tag and asks
  "is this inside my 32 MiB heap" — and command type words are
  compared against constants in the 0x2abc0_000 band (0xb000_a4e4,
  0xb000_a5ea-0xb000_a75e), which is the band the GROW ack 0x2abc0067
  sits in (`ccb.rs`, record). SOCIF access goes through region 0x2
  (`rgx_riscv.h:157-159`): references at 0x2000_1000 and 0x2000_3000
  appear in the handlers (0xb000_0a26, 0xb000_a654-0xa662).
- What the firmware is given: the CCB addresses arrive as windowed
  values, they are masked and bounded as above, and the A/B outcome
  changes with the window tag alone (host mapping identical either
  way) — **shown**, from the board record: whatever the firmware does
  with them, the tag it was given changes its behaviour.

## The mechanism (a reading, not a measurement)

On today's build the host side is clean: the carveout is mapped
Normal-NC, so a host read is a memory read and a host write is not
held in a host cache. The only cache left between the host's view of
memory and the firmware is the firmware core's own cache, and the
window decides whether the firmware's accesses pass through it.

- **KCCB direction (our post, its taking).** We write the command
  words and the KCCB write offset (Normal-NC: they go to memory).
  The firmware's reads of that control dword, through the cached
  window, can hit a line it already holds: nothing in the image
  invalidates the ring's control lines (no cbo; the CSR loops are on
  payload ranges), and a fence does not refresh a line. The firmware
  then sees our post only when that line is next brought in — at its
  own fixed times (the 5 and 8 ms bands, keyed to request numbers 7
  and 15 of a fixed schedule: the schedule's fixed cache state, not
  noise). Through the uncached window the reads are memory reads:
  taken at once, 1 ms. **Read**, supported by: the bands are a fixed
  firmware schedule (`pvr-render-pool.md:4271-4305` shows PM latency
  does not move them); the only thing the knob changes is the window
  bit of the address the firmware is given; uncached takes every post
  at once. The image does not yet show the exact polling loop, and
  the host logs cannot split "its read was stale" from "its answer
  sat in its cache": either way it is the same line-turnover
  mechanism, and which one it is stays **open**.
- **FWCCB direction (its commands, our reads).** Its writes to a slot
  and to the FWCCB write offset sit in a write-back cache; each line
  is written back on its own schedule, in an order the firmware code
  cannot control (fences order the core's accesses, not the
  cache-to-memory writeback). When the offset line leaves the cache
  before the slot's lines, our Normal-NC read sees the offset past a
  slot that still reads empty in memory; the slot's words land later
  (exactly 240 us in both events, with the re-read instrument
  recovering them). Without the re-read the slot is skipped and its
  command is lost — the lost FWCCB command of the record, and the
  refused-grow hang that followed it. Through the uncached window
  every write goes to memory in program order: 0 events in 20 runs.
  **Read**, supported by: the 240 us shape of the events
  (`pvr-render-pool.md:4175-4270`), the zero cbo instructions, the
  5-of-20 against 0-of-20 split at p 0.024, and the host-side twin:
  the same class of fault (one shared 64-byte CCB control line, one
  side caching) already happened and was fixed on the host side by
  Normal-NC (`pvr_fw_boot.c:580-603`).

## What this does not settle

- What bits 0x7cc/0x7cf write (line clean? invalidate? a refill
  trigger?) and what the `micect`/`mdcect`/`mdcrfct` value
  0x1000_0000 selects: core internals, not in any open document we
  have.
- The firmware's KCCB polling loop itself: it reads the rings through
  values copied from the init block (no window constant is needed),
  so it hides in the compressed code our disassembler cannot fully
  decode yet.
- Whether some firmware paths re-tag a host pointer into its own
  heap window 0x1 before using it (the mask-and-bound pattern shows
  at least one normalisation).
- `RGX_CR_AXI_ACE_LITE_CONFIGURATION` (0x38C0; AWDOMAIN/ARDOMAIN
  shifts pinned at `ddk_abi_pins.h:429-439`): the core supports
  AXI ACE-Lite (`pvr_rgx_core.h:14-17`, `rgxlayer.rs:982` — "yes" for
  36.V.104.796) and our driver never writes the register. Whether
  the chip's default makes the firmware's accesses snooped or not is
  untested; our observations fit "not snooped for the host's NC
  reads", and a write to that register is the cheap test.
- Why the bands are 5 and 8 ms and not, say, 3 and 6, and why only
  requests 7 and 15 carry events: the firmware's own working set
  (proprietary code), open.

## What would settle it next

1. **Swap windows per object.** `heap_fwaddr` is called per object
   (`fwinit.rs:73`): give the two CCB *control* blocks the uncached
   window and keep the ring payload cached, and the reverse, in two
   builds. If the empty-slot events follow the payload window and the
   5-8 ms acks follow the control window, the two halves of the
   mechanism above are separated on silicon, with the existing
   gl_test 174 harness and `FWCCB_CHECK` tally.
2. **Program the ACE-Lite control** to the coherent domain on the
   cached arm and rerun the A/B: if the events move, the default
   domain is part of the story.
3. **Finish the disassembly**: fix the compressed decoder, then the
   rings' loops — find where the firmware copies the five CCB fields
   of the init block into its `.shdatabss` globals (the block sits at
   a fixed heap offset, `fwif.rs:212-216`) and read the loops from
   there.
