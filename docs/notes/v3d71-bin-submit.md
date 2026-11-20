<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 7.1: bin and render submission for the first triangle (M5)

Raspberry Pi 5 port (#7), milestone M5 of `pre-study-rpi5.md` section 6: the
M5 job is the first with a binning pass on the board (`rpi5_clear` cleared
without drawing). The submission facts it needs, each read for this note, with
the code it applies to: the 7.1 triangle frame in `driver/src/v3d` (the
`frame71-tri` item), the `rpi5_triangle` probe (the `probe-tri` item, under
`tools/probes/rpi_5/`) and the board session's log. It changes no code.

Sources, each read for this note:

- `L` = Linux **v6.18**, tag `v6.18` at commit `7d0a66e4bb9081d75c82ec4957c50034cb0ea449`
  (the `linux:` row of `driver/docs/rpi5-v3d71-facts.md`), sparse checkout of
  `drivers/gpu/drm/v3d/` and `include/uapi/drm/`. GPL: facts and `file:line`
  only. All `v3d_*` paths are under `drivers/gpu/drm/v3d/`.
- `S` = Mesa 26.2.1 `src/broadcom/simulator/v3dx_simulator.c` (a wrapper
  around the vendor simulator; the `*_BMFCT_SET` / `*_RMFCT_SET` field macros
  it uses come from the vendor's register header, which is not in the Mesa
  tree, but its sizes, orders and waits are in this file). MIT.
- `G` = Mesa 26.2.1 `src/broadcom/common/` and `src/gallium/drivers/v3d/`
  (not vendored; the gate's Mesa checkout, tag `mesa-26.2.1`).
- `X` = `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml`.
- `B` = the board's Linux references: `tools/v3d/pi5/v3d_regs.txt` (debugfs
  `regs`, idle) and `tools/v3d/pi5/triangle.clif` (one Gouraud triangle after
  a 64x64 clear on the board, `V3D_DEBUG=clif`); `C` = the same job on the
  host drm-shim, `tools/v3d/shim71/triangle.clif`.
- `F` = our `driver/src/v3d/v3d_hw.c`, `v3d.h`, `v3d_frame.c`, `v3d_regs.h`.
- `P` = `tools/probes/rpi_5/rpi5_clear/src/main.c` (the M1 to M4 probe).

## 1. The submit sequence itself does not move at 7.1

| Step | Linux v6.18 | Mesa simulator | Ours |
|---|---|---|---|
| bin: clear the previous job's overflow pool | `V3D_PTB_BPOS = 0`, "so we don't reuse the overflow attached to a previous job", `L:v3d_sched.c:246-249` | (each submit in the sim starts fresh) | `F:v3d_hw.c:412` |
| bin: invalidate the GPU caches | `L:v3d_sched.c:252` | `S:517` | `F:v3d_hw.c:413` |
| bin: tile allocation, tile state, list base, **list end last** (the write that starts the binner) | `CT0QMA`/`QMS` `:272-273`, `CT0QTS` `:276-277`, `CT0QBA` `:280`, `CT0QEA` `:281`, comment "Writing the end register is what starts the job" `:268-269` | `S:519-529` | `F:v3d_hw.c:414-418` |
| render: invalidate, then `CT1QBA`, `CT1QEA` | `L:v3d_sched.c:306,327-328` | `S:540-543` | `F:v3d_hw.c:421-430` |

Our `v3d_submit_bin` / `v3d_submit_render` (`F:v3d_hw.c:401-430`) already
follow this, plus clearing a late stage's done bit before starting
(`F:v3d_hw.c:403-406`). None of these registers is versioned in the header
(`L:v3d_regs.h:319-328,339-342`; `driver/docs/rpi5-v3d71-facts.md` section 2,
"What does **not** move"). Nothing to change for M5; the frame builder's
*contents* are the triangle's own item.

## 2. The tile-allocation size must be computed with draws = 1

- Our `v3d_frame_alloc_sizes` (`F:v3d_frame.c:117-127`) is Mesa's
  `v3d_tile_alloc_sizes` (`G:v3d_util.c:271-311`) term for term:
  `align(tiles*128, 4096) + 8192 + min(tiles*128*draws/2, 512 KiB)`, aligned
  to the page size (`G:v3d_util.c:285-309`). `tiles*128` is the 128 B initial
  block per tile (`STATIC_ASSERT` at `:307`).
- Gallium passes the job's queued draw calls (`job->draw_calls_queued`,
  `G:v3d_job.c:552-566`). The M5 job queues **one** draw, so the size must be
  computed with `draws = 1`. `rpi5_clear` passes 0
  (`P:228`, the clear queues none) and records 12288
  (`driver/docs/rpi5-v3d71-facts.md` section 14).
- For one 64-pixel tile (the board target, 64x64) and `draws = 1`:
  `12288 + 64 = 12352`, and aligned to any page size we or the board use
  (4 KiB ours, 16 KiB the board's kernel) that is **16384** — the value in
  both triangle references (`B:triangle.clif:329` and `C:triangle.clif:329`,
  `@format blank 16384` on `tile_alloc`); two tiles (a 128x64 target like
  `rpi5_clear`) give 12416, which aligns to the same 16384. The kernel
  writes the driver's size straight to `CT0QMS` (`L:v3d_sched.c:272-273`),
  so Mesa's number *is* the board's register value: with `draws = 1` our
  `CT0QMS` equals it.
- `tools/v3d/pi5/README.md` ("Differences", 2) records 16384 against 12288
  for the **clear**; for the triangle the host shim gives 16384 too, so
  `frame_check.sh` (d)'s "ours rounded up to 16 KiB" rule and an equal raw
  16384 coincide only at this size. `probe-tri`: pass 1, and print the size in
  the `frame %ux%u:` line; `12288` in that line is the old value.

## 3. Viewport X/Y scale is clipper granularity 64, not 256

- Mesa emits the VS/FS `VIEWPORT_X_SCALE`/`VIEWPORT_Y_SCALE` uniforms as
  `viewport.scale[i] * devinfo->clipper_xy_granularity`
  (`G:v3d_uniforms.c:254-260`), and the granularity is 256.0f at 4.2, 64.0f
  at 7.1 (`G:v3d_device_info.c:86-91`, the `case 42` / `case 71` rows).
- The `CLIPPER_XY_SCALING` packet follows the same rule: the 7.1 branch
  writes `viewport_half_width_in_1_64th_of_pixel = scale * 64.0f`
  (`G:v3dx_emit.c:454-459`; the 4.2 branch at `:445-452` uses 256.0f),
  matching the packet definition (`X:919-927`: `max_ver="42"` fields "in
  1/256th of pixel", `min_ver="71"` fields "in 1/64th of pixel").
- Board evidence: `B:triangle.clif:51-53` — `CLIPPER_XY`, "Viewport
  Half-Width in 1/64th of pixel: 2048.000000" for a 64x64 target: 32 px
  times 64.
- Our code hard-codes 256 in both places: `F:v3d_frame.c:462-467` (the
  uniforms, comment "256.0f on 4.2") and `F:v3d_frame.c:548-549` (the packet
  fields). Both must branch at 7.1 (the `frame71-tri` item); leaving 256
  there scales the viewport four times too large — the clipper reads the
  value in 1/64ths — so all but a central quarter of the geometry is clipped
  away and the raster coordinates are wrong everywhere: a missing or partial
  triangle, not a near-miss colour.

## 4. Waiting for the bin: `v3d_wait_bin`, and one pre-mapped 256 KiB overflow block

- Wait with `v3d_wait_bin` (`F:v3d.h:202-209`, `F:v3d_hw.c:515-519`): poll
  `FLDONE` and, on `OUTOMEM`, hand the binner the next block of
  `struct v3d_bin_overflow` (`F:v3d.h:189-199`: "mapped already", `used`
  counts the blocks handed out; the caller zeroes it before submitting).
  `rpi5_clear` used the plain `v3d_wait` with no pool (`P:261`), for which an
  `OUTOMEM` is the verdict failure "M3 bin: out of memory" (`P:265`) — fine
  for a clear, wrong for the first bin with content.
- The overflow block is **256 KiB**, from both references: Linux allocates
  `256 * 1024` and writes `PTB_BPOA` (address) then `PTB_BPOS` (size)
  (`L:v3d_irq.c:44,79-80`); the simulator does the same size in its
  `OUTOMEM` branch (`S:348-356`). Linux allocates at fault time from a work
  item; a poll-only image cannot, so `probe-tri` maps one 256 KiB block in
  the page table (`P:102-116`'s `gpu_map`) before the bin and passes it as
  `blocks[0]`; `wait_bits` writes it to `BPOA`/`BPOS` when the binner asks
  (`F:v3d_hw.c:481-484`). Keep `PTB_BPOS = 0` at submit
  (`F:v3d_hw.c:412` = `L:v3d_sched.c:246-249`): it is what stops the binner
  reusing a previous job's pool.
- `OUTOMEM` is edge-signalled and, from V3D 4.1 on, `FLDONE` is **not**
  reported while the binner is out of memory: the wait must give the block
  and keep polling (`L:v3d_irq.c:112-120`; `S:348-356` returns from the ISR
  and only the next pass sees `FLDONE`). `wait_bits` does this
  (`F:v3d_hw.c:478-487`); the pool must be non-empty before the first spill,
  because the spill has already happened when the bit appears.
- `V3D_CLE_BFC` 0x134 and `V3D_CLE_RFC` 0x138 (`L:v3d_regs.h:309-310`) are
  the bin/render **flush counts**: the simulator reads them before a submit
  and spins until each value changes as its "wait for bin to complete before
  firing render" (`S:508-512,531-537,545-547`; the comment names the kernel
  scheduler's fence as the real mechanism). We poll the `FLDONE` interrupt
  bit instead and never read BFC/RFC — that is the right choice for us
  (interrupt semantics, not counters), and it is why they do not appear in
  the debugfs dump below or in any log we take.

## 5. Every buffer the GPU reads or writes is cleaned from the CPU caches before the submit

The rule `rpi5_clear` states and implements
(`P:24-28`): each static buffer is cleaned and invalidated
(`sys_cache_data_flush_and_invd_range`, `P:118-127`) before the GPU sees it —
page table and scratch before `v3d_mmu_init` (`P:203`), the three lists, the
tile allocation, the tile state and the target before the bin (`P:253-254`).
Flush-**and-invalidate**, not flush: a clean line left behind would later be
written back over what the GPU wrote (`P:118-119`). A cache call that fails
ends the run, it does not continue (`P:26-28`): with
`CONFIG_CACHE_MANAGEMENT` off every call is `-ENOTSUP` and the GPU would
read stale memory. After the render: `v3d_caches_clean` first (GPU side,
`P:278`, mirroring `L:v3d_gem.c:207-234`), then `sys_cache_data_invd_range`
of the target before reading it (`P:286`).

M5 adds buffers — the shader binaries, the 7.1 shader state record, the
uniform buffer(s) and the attribute data — and they all take the same step:
they are written by the CPU and read by the GPU, so each is cleaned before
`v3d_submit_bin`. This is the A76 cache-coherence question U10, answered for
the first clear's buffers (`driver/docs/rpi5-v3d71-facts.md` section 11 and
14: 0 wrong pixels with exactly this pattern).

## 6. If it faults: the 7.1 address, the 7.1 client table, the GMP pair

- An MMU fault is a hub interrupt (`MMU_WRV` 5, `MMU_PTI` 4, `MMU_CAP` 3,
  `L:v3d_regs.h:68-74`). Read `MMU_VIO_ID` and `MMU_VIO_ADDR`
  (`L:v3d_irq.c:172-173`). The reported address holds only the top bits:
  Linux prints `VIO_ADDR << (va_width - 32)` (`L:v3d_irq.c:173-174`) and the
  simulator computes the same (`S:407-408`). This board has `VA_WIDTH` 36
  (`B:v3d_regs.txt:11`, `MMU_DEBUG_INFO` 0x20804664; `P:180` prints
  `va_width`), so on the board the print is **`vio_addr << 4`**. Our stall
  snapshot stores the raw register (`F:v3d_hw.c:445`); the probe prints it
  shifted. Linux then writes `MMU_CTL` back to itself to re-arm (`:201`).
- Decode the client as `axi_id & 0x7F` against the 7.1 table (table
  `L:v3d_irq.c:188-200`, mask and decode `:206-213`, `begin <= id < end`):
  L2T 0x00-0x2f, CLE 0x30-0x37,
  PTB 0x38, PSE 0x39, CSD 0x3a, TLB 0x40-0x5f, MMU 0x60-0x6f, TFU 0x7c-0x7d,
  GMP 0x7f. (The 4.1 table at `:179-187` has a 0xff mask and different
  ranges — do not reuse it.)
- At 7.1 the GMP violation bit moved to the **hub** (`V3D_V7_HUB_INT_GMPV`
  bit 6, `L:v3d_regs.h:68`; enabled on the hub only, `L:v3d_irq.c:27,33`);
  `wait_bits` already treats it as fatal on the BCM2712
  (`F:v3d_hw.c:455-456`). Linux prints only "GMP Violation" there
  (`L:v3d_irq.c:238-240`); for the board session print the pair next to it:
  `GMP_STATUS` 0x600 and `GMP_VIO_ADDR` 0x608 at 7.1 (`L:v3d_regs.h:381,400`;
  0x800/0x808 at 4.2), whose bits `VIO` 0, `INVPROT` 1, `CNTOVF` 2 and the
  RD/WR counters (`:382-392`) say which violation and where. Mesa's simulator
  prints `GMP_VIO_ADDR` raw (`S:444-446`); whether it is 16-byte granular is
  not documented in any source we hold — print it raw and uninterpreted.
  Board idle values: STATUS 0x00000030, CFG 0, VIO_ADDR 0
  (`B:v3d_regs.txt:12-14`).

## 7. What the board session logs around the submit

The debugfs `regs` file (`B:v3d_regs.txt`) is the model set: `L:v3d_debugfs.c`
REGDEFs UIFCFG (`:25`), MISCCFG (`:52`), CT0CA (`:56`), CT1CA (`:59`),
BPCA/BPCS (`:62-63`), FDBGS (`:71`), ERR_STAT (`:72`). The M5 log adds what
the idle dump cannot show, one line per stage:

- **Before the bin:** `MISCCFG` (core 0x018, `L:v3d_regs.h:241`) — expected
  as on the board under Linux: 0x00000006 with bit 0 clear
  (`B:v3d_regs.txt:18`), because Linux writes OVRTMUOUT only below V3D 4.1
  (`L:v3d_gem.c:27-28`) and our driver never writes MISCCFG; `UIFCFG` (hub
  0x004, `L:v3d_regs.h:34`) — the board reads 0x45 idle (`B:v3d_regs.txt:1`)
  and Linux never writes it (only a param read, `L:v3d_drv.c:53`), so it is
  the firmware's/reset value and the reference for our UIF stores;
  `ERR_STAT` 0x0f20 and `FDBGS` 0x0f10 (core, `L:v3d_regs.h:500,484`) —
  idle 0x00001000 and 0x7 (`B:v3d_regs.txt:31-32`); Linux reads ERR_STAT
  before a hang reset (`L:v3d_gem.c:129-130`) and decodes no bits; and both
  `INT_STS` at offset 0x050 — core and hub share the offset, so name the
  block in the print (`L:v3d_regs.h:275,62`).
- **After `FLDONE`:** `CT0CA` (`L:v3d_regs.h:298`) against `CT0EA`
  (0x0108, `:295`): equal means the CLE ran the bin list to its end;
  `BPCA`/`BPCS` (0x0300/0x0304, `L:v3d_regs.h:339-340`, what the binner's
  pool allocation is right now, 0 at idle, `B:v3d_regs.txt:27-28`) and the
  number of overflow blocks used, which is our `ov.used`
  (`F:v3d.h:190-192`) — BPCA/BPCS say where, `used` says how often.
- **After `FRDONE`:** `CT1CA` (0x0114, `L:v3d_regs.h:299`) against the
  render list's end address (the `CT1QEA` we wrote).

`print_stall` (`P:94-100`) already dumps core/hub `INT_STS`, CT0CA/CT0RA,
CT1CA/CT1RA, `ERR_STAT`, `MMU_CTL`, `VIO_ADDR`, `VIO_ID` — but only on a
failed stage. M5 prints the three stage lines above on the **pass** path too
(faults are rare and silent otherwise); `v3d_stall_snapshot` (`F:v3d.h:174`)
reads the same registers for the failure path.

## 8. The three register facts against Linux v6.18

| Claim | Verdict | Citation |
|---|---|---|
| `BFC`/`RFC`, the flush counts, live at 0x134/0x138 | **Confirmed** | `L:v3d_regs.h:309-310` (`V3D_CLE_BFC 0x00134`, `V3D_CLE_RFC 0x00138`, between `PCS` 0x130 and `TFBC` 0x13c). "Flush count" behaviour: the simulator's completion reads `S:508-512,535-547`. Not in the debugfs dump list, so no BFC line in `B:v3d_regs.txt` |
| MISCCFG bit 0 is OVRTMUOUT | **Confirmed** | `L:v3d_regs.h:241` (`V3D_CTL_MISCCFG 0x00018`), `:244` (`V3D_MISCCFG_OVRTMUOUT BIT(0)`). Meaning: the texture sampler uniform config's TMU output-type field is used instead of the type-derived default (`L:v3d_gem.c:22-26`). Linux sets it only below 4.1 (`L:v3d_gem.c:27-28`) and debugfs prints "Override TMU" only there too (`L:v3d_debugfs.c:184,203-205`); the board's idle MISCCFG 0x6 with bit 0 clear matches (`B:v3d_regs.txt:18`) |
| ERR_STAT bit 12 means "VCD idle" | **Confirmed as the header's name; the idle reading is consistent** | `L:v3d_regs.h:504` (`V3D_ERR_VCDI BIT(12)`, siblings L2CARE 15, VCMBE 14, VCMRE 13, VCDE 11 at `:501-505`). The header names the bit but expands nothing and no Linux code decodes it (raw dump only, `L:v3d_gem.c:129-130`); "VCD idle" is the expansion of the name — and the board's idle ERR_STAT is exactly 0x00001000, this bit alone set with the GPU quiet (`B:v3d_regs.txt:32`), which an error bit would not be. No stronger confirmation exists in the sources we hold |

## Open points handed on

- Whether `GMP_VIO_ADDR` is 16-byte granular (print raw; no source says).
- The M5 frame's own bytes and the D0 shader record: the `frame71-tri` and
  `ours71-record` items; the inside/outside/edge, tolerance-3 pass rule of
  the Pi 4 `v3d_triangle` is `pi4-v3d-bringup.md:247,300` and
  `pre-study-rpi5.md:556` — the readback classifier belongs to `tri-check`.
- MISCCFG and FDBGS have no names in `F:v3d_regs.h` yet (MISCCFG is core
  0x018, FDBGS core 0x0f10); `probe-tri` adds the two defines if it logs
  them itself.
