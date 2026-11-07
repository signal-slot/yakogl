<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR path B's CPU tail: a cacheable view or wide loads

Status: a study from the code and the records, no board run. Line numbers are
at main `8990a5cc`. The board figures are those of `am62p-pvr-mipmap-kicks.md`
("Measured", 2026-10-29 and 2026-11-01) and `am62p-slow-terrain.md`
(candidate 3). Every time below is an estimate; the one record that decides
is at the end.

## The question

Path B (`Backend::mip_cpu_tail`, `src/pvr/backend.rs:1640`, off by default)
draws levels 1..=n of a chain on the GPU and lets the GL core filter the rest
on the CPU. On the AM62P it took terrain from 83 to 124 ms. Two causes: the
wait for the prefix's last kick (15.39 ms a call, about 4.4 ms expected;
`pvr-waited-kick`'s question) and the tail itself, 7.87 ms a call (mean of 80
calls, the measurement build of 2026-10-29). This note is about the second:
what makes the tail cost 7.87 ms and which change would bring it under 1 ms.

## What the tail does and what it reads

- The backend answers `2 + n` from `pvr_generate_mipmap` and the GL core runs
  `MipChain::Prefix(n)` (`src/gl/texture.rs:4293`, `first = n + 1`); with
  `first > 1` the filter is `mip_step_gpu_sample` (`texture.rs:4337`, loop at
  `:4352`; the function `:4112-4138`).
- `mip_gpu_last` (`src/pvr/backend.rs:6234-6236`) with
  `MIP_CPU_TAIL_TEXELS = 64 * 64` (`:6229`): the 800x600 chain keeps levels
  1..=3 on the GPU (400x300, 200x150, 100x75), the 256x256 chain level 1
  (128x128; level 2 is 64x64 = 4096, not over the limit).
- Before the loop the core invalidates the allocation's cache lines
  (`texture.rs:4343`, `osal::cache_invd(t.mem.cpu, t.mem.size)`; the osal does
  `sys_cache_data_invd_range`, `src/osal/zephyr.c:487-491`, `:503-506`). On a
  non-cacheable mapping that is a `dc ivac` loop with nothing to invalidate.
  It is over the **whole allocation** here (2.5 MB for an 800x600 chain, 40 k
  lines), not the tail; the record's "90,112 bytes of cache maintenance a
  frame" and "a few microseconds" are the measurement build's, which must have
  ranged it. Check this line before any board run (risk 6 below).
- `mip_step_gpu_sample` is per destination texel, per channel, four byte loads
  `texels[src + ..]` from the slice and one byte store: **16 byte loads a
  destination texel**, each source byte loaded about four times (the compiler
  cannot keep them: the stores go to the same slice).

### Byte counts (from the chain geometry; level sizes as in `pvr-mipmap-cpu-cases.md` and `terrain-mip-usage.md`)

| chain | tail source level | tail destination texels | distinct source bytes read | byte loads (16 a destination texel) | bytes written |
|---|---|---:|---:|---:|---:|
| 800x600 (3 a frame: terrain target twice, tilt-shift H) | level 3, 100x75 = 30,000 B | 50x37 + 25x18 + 12x9 + 6x4 + 3x2 + 1 = 2,439 | about 39.8 KB | 39 k | 9.8 KB |
| 256x256 (2 a frame: bloom H, V) | level 1, 128x128 = 65,536 B | 64x64 + 32x32 + ... + 1 = 5,461 | about 87.4 KB | 87 k | 21.8 KB |
| per frame (5 calls) | | 18,000 | about 294 KB (4,600 lines of 64 B) | 292 k | 51 KB (800 lines) |

Check on the 7.87 ms: the mean call does 58 k byte loads; 7.87 ms / 58 k is
**135 ns a load**, one uncached DRAM read each. The measured figure is the
loads' count times a DRAM latency: no compute term is needed to explain it.
(The same sum reproduces the 105.6 ms "tail filtered in a heap copy" row only
if that copy was bigger or narrower than the tail; the experiment's code is
not on main, so the copy's width and extent are unknown. A byte-wise or
whole-texture `memcpy` over the NC map would read 2.5 MB or 294 KB at 135 ns a
byte. That is why wide loads must be written as explicit 16 B loads, not as a
`memcpy`.)

## How the carve-out is mapped

- One map, at boot: `pvr_fw_boot` calls `k_mem_map_phys_bare(&cpu,
  PVR_GPU_MEM_PHYS, PVR_GPU_MEM_SIZE, K_MEM_ARM_NORMAL_NC | K_MEM_PERM_RW)`
  (`driver/src/pvr_fw_boot.c:597-599`; the `K_MEM_CACHE_WB` fallback `:600-602`
  is for targets without the attribute, i.e. not the A53). The reason is in the
  comment above it (`:587-595`): the firmware and the CPU share cache lines
  in the CCB control blocks and the HWRT data, so a cached map with clean and
  invalidate by hand lost the firmware's updates on the second kick (a 3D BIF
  fault at VA 0, on silicon). Device memory is no option: it alignment-faults
  the struct writes. So **the whole 64 MiB is NC and that decision stays**;
  this note is about the texture pages only.
- Everything the CPU sees of the pool is `g_km.cpu` plus an offset
  (`driver/src/pvr_fw_boot.c:88`, `km_flush`/`km_invd` `:91-100`). The Rust
  side maps pages for the GPU only: `driver/rust/src/gpu/pvr/mem.rs` puts a
  block at carve-out offset `co` at heap offset `TOP_VA + co` (`:155-241`,
  `mmu::map(k, ..., k.phys + co, ...)`); it never maps CPU addresses. A
  texture over 256 KiB sits in the top region (`backend.rs:6248-6253`): every
  chain terrain tails (2.5 MB and 350 KB) is there.
- The Zephyr kernel VM is sized for one map: `KERNEL_VM_SIZE = 0x10000000`
  (256 MiB), `MAX_XLAT_TABLES = 256` (`driver/zephyr/Kconfig:218-224`). A
  second 64 MiB map fits the VM; the table cost is 32 level-3 tables more at
  4 KiB pages, to be checked against the image's table count.
- Precedent for a second, cached CPU view of GPU-visible pages already in the
  tree: `z_gpu_map_phys` maps a window surface's pages `K_MEM_CACHE_WB` for
  `glReadPixels` (`src/osal/zephyr.c:466-474`, comment "A cached CPU view for
  glReadPixels on a window surface"). Those pages are outside the carve-out
  (the caller owns them), so it is no alias of an NC map: it shows
  `k_mem_map_phys_bare` accepts such a request and the osal's flush and
  invalidate helpers (`:478-506`) are in use for it, nothing more.

## Option 1: wide loads into a local buffer (no mapping change)

Read each source row once, with 16-byte (or 32-byte `ldp q`) loads, into a
stack or heap row buffer in cacheable memory; filter from the buffer. Normal
NC memory allows unaligned and wide accesses (the boot comment says Device is
the type that alignment-faults), so a `u128` read volatile loop, or
`core::arch::aarch64` loads, works on the NC map. This is the shape of the
Pi 4 plan's `mip-filter-wide` item for `src/gl/texture.rs`; it applies to this
path as it is the same function.

Estimate, at 135 ns a load (the A53 does not coalesce NC loads; a wider load
is one bus read):

| | loads a frame | at 135 ns | a call (mean of 5) |
|---|---:|---:|---:|
| today, 16 byte loads a texel | 292 k | 39 ms | 7.9 ms |
| each source byte once, 1 B loads | 73 k | 9.9 ms | 2.0 ms |
| each source byte once, 16 B loads | 18.4 k | 2.5 ms | 0.5 ms (0.34 for 800x600, 0.74 for 256x256) |
| 32 B loads (`ldp q`), if the interconnect keeps them whole | 9.2 k | 1.2 ms | 0.25 ms |

The source of a row pair is read once and reused for the two destination rows
that need it (a row of the destination reads at most two source rows, and the
odd steps' neighbouring destination rows share one). Compute is then the
floor: about 18 k destination texels x 4 channels, 12 flops and 4 converts
each, about 40 cycles a channel on an in-order A53 at 1.4 GHz: 0.3 ms for an
800x600 call, 0.6 ms for a 256x256 one (0.4 ms mean). The even steps, which
are the CPU's integer 2x2 box byte for byte (the test at `texture.rs:9353`),
could skip the float path; they are most of the 256x256 chain's tail.
**Tail time: 0.5 to 0.9 ms a call, the edge of the 1 ms target; with the
integer path for even steps 0.4 to 0.7 ms.**

Risks: none of coherency (no new mapping); the GPU's writes are complete (the
backend waits the last kick, `backend.rs:6290-6293`) and the NC loads see
DRAM. The stores to the destination levels stay NC byte stores, which the
write buffer merges: 51 KB a frame, cheap. The picture must not move: the
filter's arithmetic is unchanged, only where its operands come from (pinned
by the test at `texture.rs:9353` and `mip-filter-pin`).

## Option 2: a cacheable alias of the pool for the tail

Map the carve-out a second time, `K_MEM_CACHE_WB | K_MEM_PERM_RW` (inner
shareable, Normal write-back), as `g_km.cpu_wb`; the GL core's tail then reads
and writes the texture through it:

1. before reading: `dc civac` over the source level (clean and invalidate; not
   plain `ivac`, so no stale dirty line of a neighbour is dropped);
2. read and filter in the cache;
3. after writing: `dc cvac` over the destination range (and drop the lines).

Under Zephyr the calls exist: `sys_cache_data_flush_and_invd_range`,
`sys_cache_data_flush_range` (the A53's line is 64 B; Zephyr's aarch64 cache
code walks the range by VA with `dc civac`/`cvac`). Zephyr's
`k_mem_map_phys_bare` does not refuse a second virtual map of the same
physical pages. Mapping the whole pool needs 32 more level-3 tables out of
`MAX_XLAT_TABLES = 256`; mapping per texture needs an unmap at free (the pool
frees blocks: `mem.rs`) and still a table or more each, so the whole-pool
alias, read-only by convention except for the tail, is the simpler one.

Cost estimate. Line fills: the tail's 294 KB are 4,600 lines a frame; at a
sequentially prefetched 60 to 150 ns a line that is 0.3 to 0.7 ms a frame. The
maintenance: `dc civac` of the 4,600 source lines, absent from the cache, 30
to 100 ns each, 0.15 to 0.5 ms a frame; `dc cvac` of 800 destination lines
that are dirty and written back, about 100 ns each, 0.08 ms. Total memory side
**0.5 to 1.3 ms a frame, 0.1 to 0.26 ms a call**, plus the compute floor of
option 1 (0.3 to 0.6 ms a call): **0.4 to 0.9 ms a call**. That is no better
than option 1: with the loads cheap, the float filter is the cost, and both
options leave it. So the cacheable view saves nothing over the wide loads for
the tail's size, which is why it ranks second: more risk for the same time.

### Risks of option 2 (ARMv8 attribute aliasing, coherency with the GPU)

1. **Mismatched attributes.** The same physical page would be mapped Normal
   NC and Normal WB. The Arm ARM (Armv8-A, B2.8 "Mismatched memory
   attributes") makes the result of mixed accesses unpredictable unless
   software cleans and invalidates between uses: a cacheable line may be
   allocated at any time (hardware prefetch, speculative load) while a
   cacheable mapping is valid, and a later NC access to the same address may
   or may not hit it. With the whole pool aliased cacheable, speculative
   fills of the *CCB and HWRT control blocks* are possible, the very lines
   whose cached mapping lost firmware updates (`pvr_fw_boot.c:587-595`).
   They would only be read through the NC alias, but an NC read that hit a
   stale line is allowed by the architecture. To be safe the alias must cover
   **only the texture pages** (per-texture, with unmap), not the pool, which
   costs the page table and VM bookkeeping above. This is the main reason
   against option 2.
2. **Dirty lines written back late.** After the tail's stores through the WB
   alias a dirty line can be evicted any time; the next frame's GPU writes to
   the same level would then be overwritten. Clean (`cvac`) right after the
   last store, and never write the texture through the alias elsewhere.
3. **Line sharing.** The 64 B line must hold only this texture's bytes:
   texture blocks are page-aligned in the pool, so only the levels of one
   chain share lines, and the tail runs after the GPU's last kick completed.
4. **The GPU is not coherent** with the A53's caches on this SoC (the driver
   flushes every write for it: `src/osal/zephyr.c:478-506`, `pvr_fw_boot.c`),
   so the invalidate before the read is mandatory, and the GPU reads of the
   next frame (the TPU sampling the new level 4..) need the `cvac` and a
   `dsb` before the render is posted. The backend's existing flush covers
   the PDS and constants arenas, not the texture.
5. **A single clean hole costs a wrong picture** (a stale level 4+ sampled
   by the next draw), the failure mode of the 2026-10-29 "cache maintenance of
   the tail alone" experiment's row (81.00 ms, same time, the maintenance
   itself is cheap).
6. **The whole-allocation invalidate at `texture.rs:4343`** is `t.mem.size`
   long (2.5 MB: about 40 k `dc` instructions, about 1 to 4 ms on a cacheable
   view and a smaller cost on the NC one). It must become the tail's range
   (the source level and the destinations) with either option, or option 1
   gains less than its estimate. The `:1044`, `:1227`, `:1517` and `:4479`
   uses are other paths and not terrain's.

## Option 3 (not recommended): the whole carve-out cacheable

The boot comment (`pvr_fw_boot.c:587-595`) records the silicon failure: do
not.

## Ranked recommendation

1. **Wide loads into a local row buffer, with the invalidate cut to the tail's
   range** (option 1, plus the integer 2x2 step on even sizes). No mapping
   change, no aliasing, no coherency work; the byte counts predict 0.4 to 0.9
   ms a call against 7.87. It is a change in `src/gl/texture.rs`
   (`mip_step_gpu_sample` and the loop at `:4337-4352`; #310's, and the Pi 4's
   `mip-filter-wide` item, which should add the 16 B load helper this reuses)
   and none in `src/pvr` or the driver, so it does not touch #325's or #301's
   files.
2. **Texture-page cacheable alias with civac / cvac** (option 2), only if 1
   misses the target. Per texture, with unmap, to avoid risk 1. The compute
   floor is the same, so expect no gain over option 1; its value would be on a
   larger tail (the 64x64 limit raised), where the lines do the work. Touches
   `driver/src/pvr_fw_boot.c`, `src/osal/zephyr.c`, `mem.rs` (#325's).
3. **Cut the compute**: the float filter on the even steps (byte-exact with the
   integer box) and the 4x reload. It is part of option 1's estimate; on its
   own it leaves the 135 ns loads.

Neither fixes the tail's other half, the prefix's wait (15.39 ms for 4
kicks); path B stays off until that costs 4.4 ms (`pvr-waited-kick`).

## The record that confirms it

gl_test case 216 `mipchains` with the tail on (`mip_cpu_tail`; `pr-session.sh
--knob ... --cases 216`) and the measurement build's per-prefix lines: **the
tail's time under 1 ms** (7.87 ms now), the cache maintenance bytes at about
51 KB the frame at most, the kicks 15 a frame, the four quadrant probes PASS
(the filter's bytes are unchanged), then glmark2 terrain's ms a frame with the
tail on against 123.8 (path B now) and 83.06 (default). A tail near 0.5 ms
confirms option 1; if it stays above 3 ms, the loads are not the DRAM latency
and the filter's compute (or the whole-allocation invalidate, risk 6) is the
cost: read the invalidate's time first.
