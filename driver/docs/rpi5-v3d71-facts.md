<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 5 (BCM2712, V3D 7.1): facts from Linux

The hardware facts the Pi 5 port needs, each with the Linux file and line it
was read from. It is the Pi 5 counterpart of `driver/docs/rpi4-v3d.md`, and it
replaces the page-summariser reading of `docs/notes/pre-study-rpi5.md`
(item H1). Nothing here was run on a board. Where the source does not say,
the item is marked **open**; nothing is guessed.

## Sources and rules

All line numbers were taken from real checkouts (`grep -n` or `cat -n`), not
from a summary. The trees are shallow clones (depth 1), kept in the project's
dependency cache, never in this repository:

| Prefix | Tree | Commit |
|---|---|---|
| `linux:` | `https://github.com/torvalds/linux.git`, tag `v6.18` | `7d0a66e4bb9081d75c82ec4957c50034cb0ea449` |
| `rpi:` | `https://github.com/raspberrypi/linux.git`, branch `rpi-6.12.y` | `5bd564aaa8dc4b1b8ed11733c19ec8ece63a72cc` |
| `docs:` | `https://github.com/raspberrypi/documentation.git`, branch `master`, under `documentation/asciidoc/computers/` | `7376533ab86ea07acc77ef6a9f3c3e57fcff833d` |
| `zephyr:` | `https://github.com/zephyrproject-rtos/zephyr.git`, the revision `west.yml` pins (the workspace this repository builds with) | `af17c0c8aa2a82a0b714a423915ad3610063ee68` |

The Linux and Raspberry Pi files are GPL-2.0: only facts and `file:line`
citations are kept here, never code. Zephyr is Apache-2.0; it is cited the
same way. Where `rpi:` and `linux:` hold the same
file, `linux:` is cited. `rpi:drivers/gpu/drm/v3d/v3d_regs.h` is byte for byte
the same as `linux:` (the two directories differ in other files, which the
`diff -rq` of the two trees lists). All `v3d_*` paths are under
`drivers/gpu/drm/v3d/`.

Mainline `v6.18` has **no V3D node** for the BCM2712 (`grep v3d` over
`arch/arm64/boot/dts/broadcom/bcm2712*` finds none; only the register-less
`vc4: gpu` at `linux:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:496`), and no
BCM2712 power-domain support (`grep 2712` over `drivers/pmdomain/bcm/bcm2835-power.c`
and `drivers/mfd/bcm2835-pm.c` finds none). The device-tree and power facts of
the Pi 5 therefore come from the Raspberry Pi fork (`rpi:`). The mainline
driver, binding and register header do know the chip (`linux:v3d_drv.c:282`).

## 1. Identity

| Fact | Citation |
|---|---|
| Compatible `brcm,2712-v3d` selects generation 71 | `linux:v3d_drv.c:282`; list of all four at `:280-285` |
| Generation numbers are 33, 41, 42, 71 | `linux:v3d_drv.h:103-107` |
| Hub `IDENT0..3` at 0x08, 0x0c, 0x10, 0x14 | `linux:v3d_regs.h:35,37,51,56` |
| `HUB_IDENT1`: `TVER` 3:0, `REV` 7:4, `NCORES` 11:8; Linux computes `ver = TVER*10 + REV` (7.1 reads 7 and 1) and warns if it differs from the device tree | `linux:v3d_regs.h:42-49`, `linux:v3d_drv.c:382-388` |
| `NCORES` above 1 only warns ("multicore not yet implemented") | `linux:v3d_drv.c:390-391` |
| `HUB_IDENT3`: `IPREV` 15:8 (Linux's `rev`), `IPIDX` 7:0. Mesa also reads 23:16 as `compat_rev`; the Linux header has no name for those bits | `linux:v3d_regs.h:57-60`, `linux:v3d_drv.c:393-394`; Mesa `src/broadcom/common/v3d_device_info.c` ("devinfo->rev", "devinfo->compat_rev") |
| Core `IDENT0` 0x00 (`VER` 31:24); `IDENT1` 0x04: `VPM_SIZE` 31:28 (multiples of 1 KiB in the comment; Mesa uses 8192 per unit), `NSEM` 23:16, `NTMU` 15:12, `QUPS` 11:8, `NSLC` 7:4, `REV` 3:0 | `linux:v3d_regs.h:219-236` |
| `MMU_DEBUG_INFO` 0x1238: `PA_WIDTH` 11:8, `VA_WIDTH` 7:4, `VERSION` 3:0 | `linux:v3d_regs.h:209-215` |
| DMA mask is `30 + PA_WIDTH` bits; VA width is `30 + VA_WIDTH` | `linux:v3d_drv.c:374-380` |

None of these has a version condition in the header. The 7.1 values that
Mesa's drm-shim returns are Mesa's numbers, not a measurement
(`tools/v3d/README.md`, "Mesa's GL driver on the drm-shim").

## 2. Registers that move at 7.1

Linux writes the version in the macro (`(ver >= 71) ? new : old`). Each row is
`4.2 / 7.1`.

| Register | 4.2 | 7.1 | Citation |
|---|---|---|---|
| TFU `CS` | 0x400 | 0x700 | `linux:v3d_regs.h:99` |
| TFU `SU` | 0x404 | 0x704 | `:109` |
| TFU `ICFG` (`IOC` bit 0 = interrupt on completion) | 0x408 | 0x708 | `:120-122` |
| TFU `IIA`, `ICA`, `IIS`, `IUA` | 0x40c, 0x410, 0x414, 0x418 | 0x70c, 0x710, 0x714, 0x718 | `:125,127,129,131` |
| TFU `IOC` (image output config) | absent | 0x71c (new) | `:132-133` |
| TFU `IOA`, `IOS` | 0x41c, 0x420 | 0x720, 0x724 | `:135,137` |
| TFU `COEF0..3` | 0x424..0x430 | 0x728..0x734 | `:139,143,145,147` |
| `COEF0_USECOEF` (bit 31) and TFU `CRC` 0x434 | 4.x only | not on 7.1 | `:140-141`, `:149-150` |
| `MMUC_CONTROL` clear bit | `BIT(3)` | `BIT(11)` | `:155` |
| Core interrupt `CSDDONE` | bit 7 | bit 6 | `:283` |
| Core interrupt `PCTR` | bit 6 | bit 5 | `:284` |
| Core interrupt `GMPV` (bit 5) | used | not enabled | `:285`, `linux:v3d_irq.c:23-27,140-141` |
| Hub interrupt `GMPV` | absent | bit 6 | `linux:v3d_regs.h:68`, `linux:v3d_irq.c:29-33,238-241` |
| `GMP_STATUS`, `GMP_CFG`, `GMP_VIO_ADDR` | 0x800, 0x804, 0x808 | 0x600, 0x604, 0x608 | `linux:v3d_regs.h:381,394,400` |
| CSD queued `CFG0..6` | 0x904..0x91c | 0x930..0x948 | `:415,421,427,433,445,448,451` |
| CSD queued `CFG7` | absent | 0x94c | `:453-454` |
| CSD current `CFG0..6`, `CFG7` | 0x920..0x938 | 0x958..0x970, 0x974 | `:456-464` |
| CSD current `ID0`, `ID1` | 0x93c, 0x940 | 0x978, 0x97c | `:466,474` |
| Performance counter source fields | 7 bits | 8 bits | `:365-372` |
| Performance counter cycle-count index | 32 | 0 | `:374` |

What does **not** move: hub `AXICFG`/`UIFCFG` (`:31,34`), `INT_STS` and
the mask registers of the hub (0x50..0x64, `:62-67`) and core (`:275-280`),
`MMU_*` (0x1200..0x1238, `:160-215`), core `L2CACTL` 0x20 (`:246`), `SLCACTL`
0x24 (`:251`), `L2TCACTL` 0x30 (`:261`), `L2TFLSTA/END` (`:272-273`), the CLE
registers `CT0QBA` 0x160, `CT1QBA` 0x164, `CT0QEA` 0x168, `CT1QEA` 0x16c,
`CT0QTS` 0x15c, `CT0QMA/QMS` 0x170/0x174 (`:319-328`), the PTB registers
0x300..0x30c (`:339-342`), and `ERR_STAT` (`:500`).
The GCA block exists only below V3D 4.1 (`:76`, `linux:v3d_drv.c:414-418`).

Hub interrupt bits: `TFUF` 0, `TFUC` 1, `MSO` 2, `MMU_CAP` 3, `MMU_PTI` 4,
`MMU_WRV` 5 (`linux:v3d_regs.h:68-74`). Core interrupt bits: `FRDONE` 0,
`FLDONE` 1, `OUTOMEM` 2, `SPILLUSE` 3, `TRFB` 4 (`:286-290`).

## 3. The SMS block (7.1 only)

A third register region named `sms`. The driver maps it only for generation
71 or later (`linux:v3d_drv.c:356-360`).

| Fact | Citation |
|---|---|
| `REE_CS` at 0x000, `TEE_CS` at 0x400 | `linux:v3d_regs.h:518-519` |
| `INTERRUPT` 31, `POWER_OFF` 30, `CLEAR_POWER_OFF` 29, `LOCK` 28, `CLEAR_LOCK` 27, `SVP_MODE_EXIT` 26, `CLEAR_SVP_MODE_EXIT` 25, `SVP_MODE_ENTER` 24, `CLEAR_SVP_MODE_ENTER` 23, `THEIR_MODE_EXIT` 22, `THEIR_MODE_ENTER` 21, `OUR_MODE_EXIT` 20, `CLEAR_OUR_MODE_EXIT` 19 | `linux:v3d_regs.h:520-532` |
| `SEQ_PC` 16:10, `HUBCORE_STATUS` 9:8, `NEW_MODE` 7:6, `OLD_MODE` 5:4, `STATE` 3:0 | `linux:v3d_regs.h:533-542` |
| `STATE` values: `IDLE` 0x0, `ISOLATING_FOR_RESET` 0xa, `RESETTING` 0xb, `ISOLATING_FOR_POWER_OFF` 0xc, `POWER_OFF_STATE` 0xd | `linux:v3d_drv.h:287-291` |
| Power-up: write `CLEAR_POWER_OFF` to `TEE_CS`, poll `STATE == IDLE` for up to **100 ms** (`wait_for(COND, MS)`, not 100 iterations), log "Failed to power up SMS" on timeout without failing, then `v3d_reset_sms` | `linux:v3d_drv.c:289-303`; `wait_for` at `linux:v3d_drv.h:529` |
| `v3d_reset_sms`: write `STATE` field value 4 to `REE_CS`, poll until the state is neither `ISOLATING_FOR_RESET` nor `RESETTING`, 100 ms; no-op below 7.1 | `linux:v3d_gem.c:107-121` |
| Power-off (driver remove): write `POWER_OFF` to `TEE_CS`, poll for `POWER_OFF_STATE`, 100 ms | `linux:v3d_drv.c:305-317` |
| SMS register region in the device tree: size 0x700 at 0x10_0203_0800 | `rpi:arch/arm64/boot/dts/broadcom/bcm2712-ds.dtsi:414` |
| Binding: the 2712 node has three regions `hub`, `core0`, `sms` ("SMS state manager register") | `linux:Documentation/devicetree/bindings/gpu/brcm,bcm-v3d.yaml:69-85` |

The header names these fields (the `V3D_SMS_HUBCORE_STATUS`, `V3D_SMS_NEW_MODE`
and `V3D_SMS_OLD_MODE` mask and shift at `linux:v3d_regs.h:535-540`) but gives
no meaning for their values. What `NEW_MODE`/`OLD_MODE` mean is **open**.

## 4. Probe order

`linux:v3d_drv.c:326-457`, in order:

1. `of_device_get_match_data` gives the generation (`:345`).
2. Regions `hub` (`:348`), `core0` (`:352`) and, for 7.1, `sms` (`:356-360`).
3. The clock is `devm_clk_get_optional(dev, NULL)`, then enabled (`:362-370`).
   The name is `NULL`: the first `clocks` entry is used and `clock-names` is
   never read (see section 9).
4. `v3d_idle_sms` (`:372`).
5. `MMU_DEBUG_INFO` gives the DMA mask and VA width (`:374-380`).
6. `HUB_IDENT1` gives `ver`, checked against the device tree (`:382-388`);
   cores (`:390-391`); `HUB_IDENT3.IPREV` gives `rev` (`:393-394`).
7. Reset: `devm_reset_control_get_exclusive(dev, NULL)`; if absent (and not
   deferred) the `bridge` region is the fallback (`:398-412`). On the Pi 5
   the reset exists (section 9).
8. A 4 KiB zeroed write-combined scratch page for the MMU (`:420-426`).
9. `v3d_gem_init` (page table, `:428`), `v3d_irq_init` (`:432`).

The driver loads no firmware: `grep request_firmware` over `drivers/gpu/drm/v3d/*.c`
in v6.18 finds nothing. The GPU has no ring or firmware; the host writes
control lists and starts them by register (section 7).

## 5. MMU

| Fact | Citation |
|---|---|
| One level of page tables for a 4 GB address space; the table can need up to 4 MB of physically contiguous memory | `linux:v3d_mmu.c:7-10`; table size `4096 * 1024` at `linux:v3d_gem.c:265` |
| Address 0 is never allocated ("various bits of HW treat 0 as special") | `linux:v3d_gem.c:293-297` |
| Page shift is 12 (4 KiB MMU pages) | `linux:v3d_drv.h:22` |
| PTE bits: `VALID` 28, `WRITEABLE` 29, `BIGPAGE` 30, `SUPERPAGE` 31; a 64 KiB or 1 MiB page needs every PTE of the span filled with the big/super bit | `linux:v3d_mmu.c:24-30` |
| Page sizes 4 KiB, 64 KiB, 1 MiB, chosen by alignment of the virtual page and of the address | `linux:v3d_mmu.c:103-113` |
| PFN is checked below 2^24 (`BUG_ON(pfn + V3D_PAGE_FACTOR >= BIT(24))`): with 4 KiB pages that is a 36-bit physical address | `linux:v3d_mmu.c:101` (the 36-bit figure is arithmetic, not stated in the source) |
| Page table base: `MMU_PT_PA_BASE` 0x1204 = table physical address >> 12 | `linux:v3d_mmu.c:65`; register `linux:v3d_regs.h:180` |
| `MMU_CTL` 0x1200 = enable, PT-invalid enable/abort/int, write-violation abort/int, cap-exceeded abort/int | `linux:v3d_mmu.c:66-74`; bits `linux:v3d_regs.h:160-178` |
| `MMU_ILLEGAL_ADDR` 0x1230 = scratch page PFN, plus `ENABLE` (bit 31) | `linux:v3d_mmu.c:75-77`; register `linux:v3d_regs.h:203-204` |
| `MMUC_CONTROL` = `ENABLE`, then flush: write `FLUSH` and `ENABLE`, poll `FLUSHING` clear (100 ms), set `MMU_CTL.TLB_CLEAR`, poll `TLB_CLEARING` clear (100 ms) | `linux:v3d_mmu.c:38-61,78-80` |
| No 7.1 branch in `v3d_mmu.c`; the only 7.1 difference is the `MMUC_CONTROL` clear bit (not used by this file) | `linux:v3d_mmu.c` (whole file), `linux:v3d_regs.h:155` |

The real physical-address width is **open** until `MMU_DEBUG_INFO.PA_WIDTH` is
read on a board (`linux:v3d_drv.c:374-376` is where Linux reads it).

## 6. Caches and reset

All in `linux:v3d_gem.c`; none has a 7.1 branch except the SMS reset.

| Step | Citation |
|---|---|
| `v3d_invalidate_caches`: outside in: L3 flush (`v3d_flush_l3`, GCA on hardware below 4.1), L2C (`v3d_invalidate_l2c`, a no-op from 3.3), L2T (`v3d_flush_l2t`), slices (`v3d_invalidate_slices`) | `:247-259`; `:151-165`, `:170-179`, `:182-197`, `:237-245` |
| L2T flush: `L2TCACTL` = `L2TFLS` with `FLM_FLUSH` (0); `L2TFLSTA = 0`, `L2TFLEND = ~0` once at core init | `:193-195`; `:34-35` |
| Slice invalidate: `SLCACTL` with the four 4-bit fields (`TVCCS`, `TDCCS`, `UCC`, `ICC`) all 0xf | `:240-244` |
| `v3d_clean_caches` (after compute): `L2TCACTL = TMUWCF` and poll, then `L2TFLS` with `FLM_CLEAN` (2) and poll, 100 ms each | `:207-234` |
| `v3d_reset`: IRQs off, GCA idle, SMS reset, V3D reset, page table again, IRQs on | `:123-149` |
| V3D reset is the reset control if present, else the bridge; then `v3d_init_hw_state` | `:96-105` |
| `v3d_init_hw_state` only calls `v3d_init_core`, which does not enable interrupts | `:19-43` |

## 7. Starting a job

`linux:v3d_sched.c`. Each job type has no `V3D_GEN_71` branch except the TFU
and the compute dispatch.

| Job | Sequence | Citation |
|---|---|---|
| Bin | `PTB_BPOS = 0`; `v3d_invalidate_caches`; if the job has a tile-allocation area `CT0QMA`, `CT0QMS`; if it has a tile state `CT0QTS` = `ENABLE` (bit 1) plus the tile state address; then `CT0QBA = start`, last `CT0QEA = end` ("writing the end register is what starts the job") | `:249,252,268-281` |
| Render | `v3d_invalidate_caches`; then `CT1QBA`, `CT1QEA`. "XXX: Set the QCFG" is a comment: the queue configuration is not written | `:306,322,327-328` |
| TFU | `IIA`, `IIS`, `ICA`, `IUA`, `IOA`; on 7.1 `IOC` from `args.v71.ioc`; `IOS`; `COEF0`; `COEF1..3` (always on 7.1, on 4.x only if `COEF0_USECOEF`); last `ICFG` with its `IOC` bit set ("ICFG kicks off the job"). No cache invalidate | `:360-375`; `v71.ioc` in `linux:include/uapi/drm/v3d_drm.h:344-346` |
| Compute (CSD) | `v3d_invalidate_caches`; `CFG1..6`; on 7.1 `CFG7 = 0` ("XXX: Set the CFG7 register"); last `CFG0` | `:396,411-424` |
| Indirect CSD work-group count: `num_batches - 1` when `ver < 71` or `ver == 71 && rev < 6` | `:454-458` |

Out of memory (`linux:v3d_irq.c:39-84`): allocate 256 KiB, flush the MMU,
write `PTB_BPOA` (address) and `PTB_BPOS` (size) (`:44,77,79-80`). The
interrupt is edge signalled, and "as of V3D 4.1, FLDONE won't be reported
until any OOM state has been cleared" (`:112-118`).

## 8. Interrupts

| Fact | Citation |
|---|---|
| Core line: `platform_get_irq_optional(pdev, 1)`, name `v3d_core0`; hub line: `platform_get_irq(pdev, 0)`, name `v3d_hub`; both `IRQF_SHARED` | `linux:v3d_irq.c:260-281` |
| With no second interrupt the driver uses one line `v3d` for both (4.2) and the core handler falls through to the hub handler | `linux:v3d_irq.c:282-295,143-147` |
| Core handler reads `INT_STS`, writes it to `INT_CLR`, then handles `OUTOMEM`, `FLDONE`, `FRDONE`, `CSDDONE(ver)` | `linux:v3d_irq.c:107-135` |
| Hub handler reads `HUB_INT_STS`, writes `HUB_INT_CLR`, handles `TFUC`, the MMU faults (`WRV`, `PTI`, `CAP`: reads `MMU_VIO_ID` 0x122c and `MMU_VIO_ADDR` 0x1234 shifted by `va_width - 32`), and on 7.1 the hub `GMPV` | `linux:v3d_irq.c:159-241`; registers `linux:v3d_regs.h:200,207` |
| 7.1 AXI id table (mask 0x7f): L2T 0x00-0x2f, CLE 0x30-0x37, PTB 0x38, PSE 0x39, CSD 0x3a, TLB 0x40-0x5f, MMU 0x60-0x6f, TFU 0x7c-0x7d, GMP 0x7f. The 4.1 table (mask 0xff) differs | `linux:v3d_irq.c:175-225` |
| Enabled sets: core `OUTOMEM`, `FLDONE`, `FRDONE`, `CSDDONE(ver)` (and `GMPV` below 7.1); hub `MMU_WRV`, `MMU_PTI`, `MMU_CAP`, `TFUC` (and `GMPV` on 7.1). Enabled by writing `~set` to `MSK_SET` then `set` to `MSK_CLR` | `linux:v3d_irq.c:23-33,306-319` |
| The init clears pending bits of both sets first | `linux:v3d_irq.c:253-258` |
| GIC interrupt IDs: core SPI 249 is ID **281**, hub SPI 250 is ID **282** (SPI *n* is ID *n* + 32, the GIC architecture rule) | section 9, "The node" |
| Zephyr's stock `rpi_5` sets `NUM_IRQS` to **280** (IDs 0 to 279); the value comes from the SoC's Kconfig, not from the device tree | `zephyr:soc/brcm/bcm2712/Kconfig.defconfig:6-7` |
| With 280, an ID above `NUM_IRQS - 1` is handled as spurious by the arm64 IRQ entry, so a V3D interrupt would never reach its handler | `zephyr:arch/arm64/core/isr_wrapper.S:89-91` |
| With 280, connecting an ID above 279 fails the build: the ISR table generator rejects an index past the table | `zephyr:scripts/build/gen_isr_tables_parser_local.py:197-199` (direct), `:211-215` (software table) |
| Zephyr's GIC-v2 driver sizes the distributor from the hardware (`GICD_TYPER.ITLinesNumber`, (N + 1) x 32 lines, at most 1020), not from `NUM_IRQS` | `zephyr:drivers/interrupt_controller/intc_gic.c:242-246` |
| **Fix:** `CONFIG_NUM_IRQS=288` (any value of 283 or more) in the image's `prj.conf`; no overlay and no Zephyr patch is needed | the three rows above |

The GIC-400 of the BCM2712 is `arm,gic-400` at bus 0x7fff9000
(`rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:271-282`; Zephyr names the
same block `interrupt-controller@107fff9000`, `zephyr:dts/arm64/broadcom/bcm2712.dtsi:46-53`).
That its `GICD_TYPER` reports at least 288 lines (`ITLinesNumber` of 8 or
more) is **open** until a board image reads it. The first clear (M1 to M4)
polls the interrupt status registers and needs no interrupt; the first
triangle with the Pi 4's interrupt path (`src/osal/zephyr_v3d.c:263-264`,
`IRQ_CONNECT` and `irq_enable`) and `gl_test` (M5 on) need the fix. No
`tools/probes/rpi_5` image sets `NUM_IRQS` today.

## 9. The GPU node, power, reset and clock on the Pi 5

### The node

`rpi:arch/arm64/boot/dts/broadcom/bcm2712-ds.dtsi:410-424` (included by
`bcm2712-rpi-5-b.dts:11`). The label `v3d` is the one the
`vc4-kms-v3d-pi5` overlay enables:
`rpi:arch/arm/boot/dts/overlays/vc4-kms-v3d-pi5-overlay.dts:89-94`.

| Property | Value | Line |
|---|---|---|
| `compatible` | `brcm,2712-v3d` | `:411` |
| `reg` hub | 0x10_0200_0000, size 0x4000 | `:412` |
| `reg` core0 | 0x10_0200_8000, size 0x6000 | `:413` |
| `reg` sms | 0x10_0203_0800, size 0x700 | `:414` |
| `reg-names` | `hub`, `core0`, `sms` | `:415` |
| `power-domains` | `<&pm BCM2835_POWER_DOMAIN_GRAFX_V3D>` (value 1: `rpi:include/dt-bindings/soc/bcm2835-pm.h:7`) | `:417` |
| `resets` | `<&pm BCM2835_RESET_V3D>` (value 0: `bcm2835-pm.h:22`) | `:418` |
| `clocks` | `<&firmware_clocks 5>`; 5 is `RPI_FIRMWARE_V3D_CLK_ID` (`rpi:include/soc/bcm2835/raspberrypi-firmware.h:166-170`: EMMC 1, UART 2, ARM 3, CORE 4, V3D 5) | `:419` |
| `clocks-names` | `"v3d"`. **The property name is misspelt in the source** (`clock-names` is the real one). It does not matter: Linux asks for the clock with a `NULL` name (`linux:v3d_drv.c:362`), which takes the first `clocks` entry. The Pi 4 node has the same typo and works (`rpi:arch/arm/boot/dts/broadcom/bcm2711-rpi-ds.dtsi:565`) | `:420` |
| `interrupts` | `GIC_SPI 250`, level high, then `GIC_SPI 249`, level high | `:421-422` |
| `status` | `"disabled"` until the overlay | `:423` |

The node has no `interrupt-names`. The order is the binding's: hub first
("hub interrupt (required)"), then core
(`linux:Documentation/devicetree/bindings/gpu/brcm,bcm-v3d.yaml:32-37`), and
Linux asks for index 0 as the hub and index 1 as the core
(`linux:v3d_irq.c:260,272`). So **hub = SPI 250, core 0 = SPI 249**. SPI
numbers in the GIC are interrupt IDs 32 higher (the GIC architecture rule, not
from these files): 282 and 281.
The GIC is `arm,gic-400` at 0x7fff9000 (`rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:271-282`).

The register addresses use two address cells, like the other nodes at the root
of this dtsi. The same file defines `memory@0` as "filled by the bootloader"
(`rpi:arch/arm64/boot/dts/broadcom/bcm2712-rpi-5-b.dts:17-20`).

### The power domain and the reset: `pm`

`pm: watchdog@7d200000`, compatible `brcm,bcm2712-pm`, `reg-names = "pm"`
(0x7d200000, size 0x308), one cell for power domains and for resets
(`rpi:arch/arm64/boot/dts/broadcom/bcm2712-ds.dtsi:122-129`). The node has
**no `clocks` and no `asb` / `rpivid_asb` region** (the Pi 4 node has all of them:
`linux:arch/arm/boot/dts/broadcom/bcm2711.dtsi:109-123`).

What the fork's driver does for the V3D domain on a node without `asb`:

| Step | Citation |
|---|---|
| `bcm2835-pm` matches `brcm,bcm2712-pm` and sets `is_2712`; the power-domain device is created if `pm->asb` or `is_2712` | `rpi:drivers/mfd/bcm2835-pm.c:76,94,118-121` |
| Domain power-on for `GRAFX_V3D` without `asb`: `bcm2835_asb_power_on(pd, PM_GRAFX_2712, 0, 0, PM_V3DRSTN)` | `rpi:drivers/pmdomain/bcm/bcm2835-power.c:380-383` |
| `PM_GRAFX_2712` is register **0x304** of the `pm` region, `PM_V3DRSTN` is bit 6; every PM write carries the password 0x5a000000 | `:83,88,102,111-112` |
| The sequence: clock on, 1 us ("32 clocks for the reset to propagate"), clock off; set the reset bit (bit 6) in 0x304; clock on; ASB master and slave enable (register 0 means "do nothing", so on the Pi 5 these two are no-ops) | `:284-313,315-326`; the register-0 case at `:159-161` |
| The domain clock is `devm_clk_get(dev->parent, "v3d")`; the 2712 `pm` node has no clock, so it is `NULL` and the clock calls do nothing | `:512-523` |
| Power-off mirrors it: `bcm2835_asb_power_off(pd, PM_GRAFX_2712, 0, 0, PM_V3DRSTN)` clears the reset bit | `:449-452`; `:339-368` |
| The reset `BCM2835_RESET_V3D` is "power off then power on" of that domain; its status reads bit 6 of `PM_GRAFX` (0x10c, the **Pi 4** register, even on a 2712 node) | `:548-576,585-586` |
| The "already powered on by the firmware, leave it" shortcut (`PM_POWUP` set) is in `bcm2835_power_power_on`, which only the `GRAFX` and `IMAGE` domains call; the V3D domain goes through `bcm2835_asb_power_on` and has no such shortcut | `:214-229`; callers `:378`, `:389`, `:380-386` |

Mainline `v6.18` has none of this: its `bcm2835-power.c` has only `PM_GRAFX`
0x10c and the ASB registers (`linux:drivers/pmdomain/bcm/bcm2835-power.c:80,383-386,449-452`).

So the Pi 5 equivalent of the Pi 4 power-on is a **single register**: the
V3D reset bit in `pm` offset 0x304, plus the firmware clock. Whether the
firmware has already released that reset when it starts our image is **open**
(U5).

### The firmware clock and the property mailbox

`firmware_clocks: clocks` (`raspberrypi,firmware-clocks`, one cell) and `reset`
(`raspberrypi,firmware-reset`) are children of the `firmware` node
(`raspberrypi,bcm2835-firmware`, `mboxes = <&mailbox>`). `power`
(`raspberrypi,bcm2835-power`, which is the *firmware* power driver, not the
`pm` one above) is not a child: it sits beside `firmware` under `&soc` and
points at it (`firmware = <&firmware>`)
(`rpi:arch/arm64/boot/dts/broadcom/bcm2712-rpi.dtsi:8-29` firmware, `:31-35`
power, `:16-19` and `:21-24` the two children).

The firmware clock driver exports the V3D clock (`.export = true`,
`.minimize = true`; `rpi:drivers/clk/bcm/clk-raspberrypi.c:119-122`) and asks
the firmware with the property tags `GET_CLOCK_STATE` 0x00030001,
`GET_CLOCK_RATE` 0x00030002, `SET_CLOCK_RATE` 0x00038002,
`GET_MIN_CLOCK_RATE` 0x00030007, `GET_MAX_CLOCK_RATE` 0x00030004 and
`GET_CLOCKS` 0x00010007 (users at `rpi:drivers/clk/bcm/clk-raspberrypi.c:197,214,230,298,307,368`;
values at `rpi:include/soc/bcm2835/raspberrypi-firmware.h:51,55,56,58,61,82`).
`SET_CLOCK_STATE` 0x00038001 (`raspberrypi-firmware.h:81`) has no user in that
driver; `SET_POWER_STATE` 0x00028001 (`raspberrypi-firmware.h:54`) is used by
the firmware power driver (`rpi:drivers/pmdomain/bcm/raspberrypi-power.c:63`).
These are the same tags the Pi 4 uses.

The BCM2712 property mailbox, as an image without Linux reaches it. The
citations are `rpi:` (`rpi-6.12.y`, the commit of the sources table) unless
a row names another tree:

| Fact | Value | Citation |
|---|---|---|
| Node | `mailbox: mailbox@7c013880`, `brcm,bcm2835-mbox`, `reg <0x7c013880 0x40>`, GIC SPI 33 | `rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:229-234` |
| ARM address | **0x10_7c013880**: the node sits under `soc: soc@107c000000`, whose `ranges <0x00000000 0x10 0x00000000 0x80000000>` put child address *a* at 0x10_0000_0000 + *a* | `bcm2712.dtsi:189,191` |
| Zephyr's node for the same block | `mbox0: mbox@107c013880`, `brcm,bcm2711-mbox`, `reg <0x10 0x7c013880 0x40>`, `status = "disabled"` | `zephyr:dts/arm64/broadcom/bcm2712.dtsi:61-69` |
| Bus address of the message buffer | the ARM physical address, **no 0xc0000000 alias**: `soc` has no `dma-ranges` (`bcm2712.dtsi:189-193`), the `firmware` node has an empty `dma-ranges;` (`bcm2712-rpi.dtsi:14`), and the `axi` bus maps the first 64 GiB one to one (`bcm2712.dtsi:440`). The Pi 4's `soc` has `dma-ranges <0xc0000000 0x0 0x00000000 0x40000000>` | `bcm2712.dtsi:189-193,440`; `bcm2712-rpi.dtsi:14`; Pi 4: `rpi:arch/arm/boot/dts/broadcom/bcm2711.dtsi:45` |
| Registers | read `MAIL0_RD` 0x00, read status `MAIL0_STA` 0x18, write `MAIL1_WRT` 0x20, write status `MAIL1_STA` 0x38 (mailbox 1 at 0x20) | `rpi:drivers/mailbox/bcm2835-mailbox.c:32-33,41-46` |
| Status bits | `FULL` bit 31 (wait before a write), `EMPTY` bit 30 (wait before a read) | `bcm2835-mailbox.c:51-52`; used at `:75,118` |
| Message word | buffer address bits 31:4 with the channel in bits 3:0; the **property channel is 8** | `rpi:drivers/firmware/raspberrypi.c:21,24` |
| Buffer alignment and size | 16-byte aligned (the low 4 bits carry the channel; Linux warns otherwise), a whole number of 32-bit words, below 1 MiB; the address must fit 32 bits | `raspberrypi.c:53` (a 32-bit word), `:55,58` (alignment), `:100-101` (size), `:109` (1 MiB) |
| Buffer layout | word 0 total size, word 1 request/response code, the tags, a 0 end tag | `raspberrypi.c:111-114`; end tag `raspberrypi-firmware.h:37` |
| Response codes | request 0, success 0x80000000, error 0x80000001 | `rpi:include/soc/bcm2835/raspberrypi-firmware.h:15-17` |
| Clock ids | EMMC 1, UART 2, ARM 3, CORE 4, **V3D 5**, ... DISP 16 | `raspberrypi-firmware.h:165-182` (V3D at `:170`) |
| Clock tags | get state 0x00030001, get rate 0x00030002, get max 0x00030004, get min 0x00030007, get measured 0x00030047, set state 0x00038001, set rate 0x00038002 | `raspberrypi-firmware.h:55,56,58,61,79,81,82` |
| The V3D's clock in the device tree | `clocks = <&firmware_clocks 5>` | `rpi:arch/arm64/boot/dts/broadcom/bcm2712-ds.dtsi:419` (node `:410-424`) |
| The V3D's power domain in the device tree | `power-domains = <&pm BCM2835_POWER_DOMAIN_GRAFX_V3D>`, **1**, a domain of the `pm` block (section 9, "The power domain and the reset"), not of the firmware; `resets = <&pm BCM2835_RESET_V3D>`, 0 | `bcm2712-ds.dtsi:417-418`; `rpi:include/dt-bindings/soc/bcm2835-pm.h:7,22` |
| The firmware's V3D power domain | `RPI_POWER_DOMAIN_V3D` = **10** exists, with the tags get domain state 0x00030030 and set domain state 0x00038030, but **no** BCM2712 device tree uses it for the V3D (the only `<&power ...>` users are USB) | `rpi:include/dt-bindings/power/raspberrypi-power.h:22`; `raspberrypi-firmware.h:77,86`; `grep '<&power '` over `arch/arm64/boot/dts/broadcom/bcm2712*` |

What this means for an image without Linux: the clock is asked for through
the mailbox as on the Pi 4 (clock 5, the same tags); the power and reset are
the `pm` register (section 9), not a mailbox call. A **read-only** get domain
state for firmware domain 10 is a second thing to try on the board, after
the `pm` bit, to see what the firmware reports; whether the firmware answers
it on the BCM2712 is **open**.

Cross-check with the board layer's client,
`driver/zephyr/lib/board/rpi4/rpi_mbox.c`: its BCM2712 base 0x10_7c013880
(`:22-27`), the offsets 0x00, 0x18, 0x20, 0x38 and bits 31 and 30
(`:31-36`), channel 8 (`:37`), the codes 0 and 0x80000000 (`:38-39`), no
bus alias on the BCM2712 against 0xc0000000 on the BCM2711 (`:41-51`), and
the tags 0x00038001, 0x00030002, 0x00030004, 0x00038002, 0x00030001,
0x00030047 (`:367-407`) all agree with the table. Its `bcm2835-mailbox.c`
and `raspberrypi.c` line numbers are mainline `linux:` v6.18's (`:41-50`,
`:23`); the `rpi:` lines above are 2 and 1 higher, the values are the same.
Its buffer is 64-byte aligned (`:54`), which meets the 16-byte rule. Its
comment on the bus alias names the `axi` `dma-ranges`; the mailbox is a
child of `soc`, which has none (the result, no alias, is the same).

## 10. Display: what Linux does for HVS, pixel valve and HDMI

All `rpi:` (the display nodes are not in mainline `v6.18` for the BCM2712 except
`vc4: gpu`). The whole chain is `status = "disabled"` in the base device tree
and is enabled by the `vc4-kms-v3d-pi5` overlay
(`rpi:arch/arm64/boot/dts/broadcom/bcm2712-ds.dtsi:436-487`;
`bcm2712-rpi-5-b.dts:276-311`; the overlay targets `aon_intr`, `ddc0/1`,
`hdmi0/1`, `hvs`, `mop`, `moplet`, `pixelvalve0/1`, `v3d`, `disp_intr`, `vc4`
at `vc4-kms-v3d-pi5-overlay.dts:13-125`). A firmware device tree without the
overlay shows these nodes as disabled.

| Fact | Citation |
|---|---|
| `vc4: gpu`, `brcm,bcm2712-vc6`, no registers | `rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:446-448` |
| HVS: `brcm,bcm2712-hvs`, 0x10_7c580000, size 0x1a000 (the `reg` has 3 cells under `#ifndef FIRMWARE_UPDATED`, 4 cells otherwise); interrupts 2, 9, 16 on `disp_intr` named `ch0-eof`, `ch1-eof`, `ch2-eof`; `iommus = <&iommu4>` | `bcm2712.dtsi:612-623` |
| HVS clocks: firmware clock 4 (`core`, `RPI_FIRMWARE_CORE_CLK_ID`) and 16 (`disp`, `RPI_FIRMWARE_DISP_CLK_ID`) | `bcm2712-rpi-5-b.dts:288-291`; ids at `raspberrypi-firmware.h:166-182` |
| `disp_intr`: `brcm,bcm2711-l2-intc` at 0x7c502000, to GIC SPI 97 | `bcm2712.dtsi:318-324` |
| `pixelvalve0` 0x7c410000, SPI 101; `pixelvalve1` 0x7c411000, SPI 110 | `bcm2712.dtsi:292-302` |
| `aon_intr` at 0x7d510600, to GIC SPI 239 | `bcm2712.dtsi:284-290` |
| `hdmi0`: nine regions (`hdmi`, `dvp`, `phy`, `rm`, `packet`, `metadata`, `csc`, `cec`, `hd`), reset `dvp 1`, interrupts on `aon_intr` 1-3 (CEC) and 7-8 (hot-plug connected/removed); `hdmi1`: same names, `dvp 2`, interrupts 11-15 | `bcm2712.dtsi:370-397`, `:399-426` |
| Board clocks for both HDMI blocks: firmware clock 13 (`hdmi`; `RPI_FIRMWARE_M2MC_CLK_ID`), 14 (`bvb`; `RPI_FIRMWARE_PIXEL_BVB_CLK_ID`), the `dvp` audio clock, the 27 MHz crystal (`cec`) | `bcm2712-rpi-5-b.dts:276-286`; ids `raspberrypi-firmware.h:178-179` |
| Output-to-FIFO map for the 6.x HVS (BCM2712): output 0 to FIFO 0, output 2 to FIFO 2, outputs 1, 3 and 4 to FIFO 1 | `rpi:drivers/gpu/drm/vc4/vc4_hvs.c:916-932` (function at `:853`) |
| The driver binds `brcm,bcm2712-hvs` | `vc4_hvs.c:2263-2265` |
| Display-list allocator starts after `HVS_BOOTLOADER_DLIST_END` (32), "never overwrite the setup from the bootloader" | `vc4_hvs.c:1570-1583`; constant `rpi:drivers/gpu/drm/vc4/vc4_regs.h:1056` |
| `vc4_hvs_bind` gets clocks `core` and `disp`, asks the firmware for the maximum core clock; at least 550 MHz enables HDMI 2.0, at least 600 MHz 4096x2160 | `vc4_hvs.c:2112-2135` |
| `vc4_drm_bind` removes conflicting framebuffers and, when the firmware's own display driver is not the one in use (`firmware_kms()` false), sends `RPI_FIRMWARE_NOTIFY_DISPLAY_DONE` to stop it | `rpi:drivers/gpu/drm/vc4/vc4_drv.c:306-312,390,397-400` |
| `vc4_hdmi_bind` enables the pixel, HSM and BVB clocks if `HDMI_VID_CTL` already has `VC4_HD_VID_CTL_ENABLE` (bit 31): the firmware may leave HDMI running | `rpi:drivers/gpu/drm/vc4/vc4_hdmi.c:3391-3399`; bit `vc4_regs.h:1007` |
| HDMI clock requests: `hdmi` (HSM) minimum is the larger of 120 MHz and 101% of the TMDS character rate; `bvb` is 300 MHz above 297 MHz, 150 MHz above 148.5 MHz, else 75 MHz | `vc4_hdmi.c:1523-1526,1546-1553`; `HSM_MIN_CLOCK_FREQ` at `:112` |
| BCM2712 HDMI variants: `max_pixel_clock` 600 MHz, `vc5_hdmi_init_resources`, `vc6_hdmi_phy_init` / `vc6_hdmi_phy_disable`, lane map 0, 1, 2, clock | `vc4_hdmi.c:3538-3562` (hdmi1 at `:3566-3588`), match table `:3598-3599` |

### BCM2712 HDMI PHY initialisation (read here for the first time)

`vc6_hdmi_phy_init` in `rpi:drivers/gpu/drm/vc4/vc4_hdmi_phy.c:954-1198`
(register fields and constants for it begin near `:137-169`). It runs with
the HDMI spin lock held, in this order:

1. `vc6_hdmi_reset_phy` (`:945-952`): `PHY_RESET_CTL = 0`, `PHY_POWERUP_CTL = 0`,
   `PLL_POST_KDIV = BYPASS_EN`.
2. Nine writes of fixed constants to `PHY_PLL_MISC_0..8` (`:974-982`).
3. `PLL_REFCLK` = CMOS reference select with a reference frequency field of 54
   (`:984-986`). The reference is 54 MHz.
4. `PHY_RESET_CTL = 0x7f` (`:988`).
5. `HDMI_RM_OFFSET` from the VCO frequency (`:990-993`; `phy_get_rm_offset`
   at `:250`).
6. `PLL_VCOCLK_DIV` = enable with the VCO divider (`:995-998`; the VCO
   frequency and divider come from the TMDS character rate in
   `vc6_phy_get_vco_freq`, `:609`).
7. `PLL_CFG` with `PDIV` 0; `PLL_POST_KDIV` with `CLK0_SEL` 2 and `KDIV` 1
   (`:1000-1005`).
8. Four channel registers `PHY_CTL_0`, `_1`, `_2`, `_CK` (`:1010-1169`), each
   built from a per-rate lane table (`vc6_phy_get_channel_settings`) and the
   variant's lane map.
9. `TMDS_CLK_WORD_SEL` = 3 when the TMDS rate is 340 MHz or more, else 0
   (`:1171-1175`).
10. `PHY_POWERUP_CTL` = bandgap, LDO, bias and the four transmitters
    (`:1177-1184`); `PLL_POWERUP_CTL` = PLL power-up (`:1186-1187`).
11. `PLL_RESET_CTL`: clear `PLL_RESETB`, then set it (`:1189-1195`).

The per-rate lane-table values and the nine fixed constants are in the source
and are not copied here. Disable is the reset step alone (`:1200-1207`). That
this is all the PHY needs on a board that the firmware has not already set up
is **open**.

## 11. What the generic arm64 boot protocol says (U6, in part)

`linux:Documentation/arch/arm64/booting.rst` is the contract between a loader
and an arm64 kernel image; it says nothing about the Raspberry Pi firmware.

| Fact | Citation |
|---|---|
| Image header: `text_offset`, `image_size`, `flags`, magic `0x644d5241` ("ARM\x64"); from v3.17 `text_offset` is little endian | `:82-88,103-107` |
| The image goes `text_offset` bytes above a 2 MB aligned base | `:138` |
| On entry `x0` = physical address of the DTB; `x1`, `x2`, `x3` = 0 | `:165-168` |
| CPU in non-secure state at EL2 (recommended) or EL1; MMU off | `:174-179` |
| `CNTHCTL_EL2.EL1PCTEN` must be set when entering at EL1 | `:197` |

What the Pi 5 firmware actually passes, and at which address and exception level
it starts a `kernel=` image, is **open** (needs the board, D1). The Raspberry
Pi documentation states (docs commit above): the firmware loads
`kernel_2712.img` and falls back to `kernel8.img` (`docs:config_txt/boot.adoc:24`);
on the Pi 5 "instead of loading `start.elf`, the firmware loads the Linux
kernel" (`docs:raspberry-pi/bootflow-eeprom.adoc:77`); and a partition counts as
bootable on a Pi 5 when it has `config.txt` and a suitable device tree
(`docs:raspberry-pi/eeprom-bootloader.adoc:601`). `kernel_address` is not in
`config_txt/boot.adoc` (`grep` finds none); the legacy page documents it:
32-bit images load at `0x8000`, 64-bit images at `0x200000`, and with
`kernel_old` at `0x0` (`docs:legacy_config_txt/boot.adoc:38-40`).

### What Zephyr's `rpi_5` image does with that start state

Read in the Zephyr tree the workspace pins (`zephyr:` in the sources table):

| Fact | Citation |
|---|---|
| The image starts with the Linux arm64 Image header: `b __start`, `text_offset` 0, the image size, flags, magic `ARM\x64` | `zephyr:arch/arm64/core/header.S:25-41` (`:26` the branch, `:28` the offset) |
| The image is linked at `sram0`, a three-cell `reg <0x0 0x200000 SIZE>` (512 KiB in the SoC file; the `tools/probes/rpi_5` overlays make it 32 MiB, `reg = <0x0 0x200000 0x2000000>`) | `zephyr:dts/arm64/broadcom/bcm2712.dtsi:40-44`; e.g. `tools/probes/rpi_5/rpi5_clear/boards/rpi_5.overlay:14-16` |
| Nothing in the start code compares the address it runs at with the link address; an image loaded anywhere but 0x200000 is not detected by Zephyr | `zephyr:arch/arm64/core/reset.S` (whole file) |
| **x0 is lost before any hook runs**: `__reset_prep_c` uses x0 as a scratch register in its first instruction after saving the link register | `zephyr:arch/arm64/core/reset.S:45` (`switch_el x0, ...`) |
| So the DTB pointer must be taken before Zephyr's `__start`: the probes link with `-Wl,--wrap=__start`, and `__wrap___start` stores x0 to x3, `CurrentEL`, its own address (the load address, U6) and `MPIDR_EL1` in a `.noinit` record, then branches to `__real___start` with the registers unchanged | `tools/probes/rpi_5/common/boot/boot_x0.S:33-61`, `rpi5_boot.h:13,16-22`, `rpi5_boot.cmake:13`, `README.md` |
| Zephyr accepts entry at EL3, EL2 or EL1 and runs at EL1: from EL3 it `eret`s to the next level, from EL2 it `eret`s to EL1 with all exceptions masked | `zephyr:arch/arm64/core/reset.S:220-248` |
| The `rpi_5` defconfig makes the MMU's virtual and physical address spaces 40 bits, so `device_map()` reaches the V3D hub, core and SMS (0x10_02xx_xxxx), `pm` (0x10_7d20_0000) and the mailbox (0x10_7c01_3880) | `zephyr:boards/raspberrypi/rpi_5/rpi_5_bcm2712_defconfig:3-4` |
| The defconfig sets `CONFIG_PCIE=y`, the board picks `pcie1` as `zephyr,pcie-controller` and enables `gpio0_0` (the RP1 GPIO behind `pcie2`); `pcie1` and `pcie2` are `okay` in the SoC file. The PCIe controller driver starts at `PRE_KERNEL_1` with `CONFIG_PCIE_INIT_PRIORITY` (default 41); the RP1 GPIO driver at `POST_KERNEL` | `rpi_5_bcm2712_defconfig:12`; `zephyr:boards/raspberrypi/rpi_5/rpi_5_bcm2712.dts:26,49-51`; `bcm2712.dtsi:111-123,125-136,148-155`; `zephyr:drivers/pcie/controller/pcie_brcmstb.c:631-632`; `zephyr:drivers/pcie/host/Kconfig:17-19`; `zephyr:drivers/gpio/gpio_rp1.c:219-220` |
| The probes under `tools/probes/rpi_5` keep that: none sets `CONFIG_PCIE=n` or disables `&pcie1`, `&pcie2` or `&gpio0_0`. The board layer's BCM2712 watchdog is armed at `PRE_KERNEL_1` priority 0, before the PCIe driver, so a hang in the PCIe start is reset by it | `tools/probes/rpi_5/*/prj.conf`, `*/boards/rpi_5.overlay`; `driver/zephyr/lib/board/rpi4/rpi_wdog.c:131-142` |

If the PCIe start ever hangs or faults on the board, the way out is
`CONFIG_PCIE=n` in the probe's `prj.conf` and `status = "disabled"` for
`&pcie1`, `&pcie2` and `&gpio0_0` in its overlay; nothing of the GPU path
needs PCIe or RP1. No image does this today.

Still **open** for the board (D1's record, `rpi5_dtb`): the exception level
at entry, the value of x0, and the load address (U6); and the GPU and DMA
side of U10 (whether the V3D sees the image's RAM coherently).

## 12. Differences from the Pi 4 (BCM2711, V3D 4.2)

| Item | Pi 4 (`bcm2711`) | Pi 5 (`bcm2712`) | Citation |
|---|---|---|---|
| Compatible, generation | `brcm,2711-v3d`, 42 | `brcm,2712-v3d`, 71 | `linux:v3d_drv.c:281-282` |
| Register regions | `hub` 0x7ec00000, `core0` 0x7ec04000 (bus), 0x4000 each | `hub` 0x10_0200_0000 (0x4000), `core0` 0x10_0200_8000 (0x6000), `sms` 0x10_0203_0800 (0x700) | `linux:arch/arm/boot/dts/broadcom/bcm2711.dtsi:622-624`; `rpi:bcm2712-ds.dtsi:412-415` |
| Interrupts | one, SPI 74, for hub and core | two: hub SPI 250, core SPI 249 | `bcm2711.dtsi:628`; `bcm2712-ds.dtsi:421-422`; `linux:v3d_irq.c:260-295` |
| Clock | firmware clock 5 in the fork's tree (`bcm2711-rpi-ds.dtsi:564`); `pm` has its own CPRMAN clock `v3d` | firmware clock 5 | `rpi:bcm2711-rpi-ds.dtsi:564`; `bcm2711.dtsi:117-121`; `bcm2712-ds.dtsi:419` |
| Power domain `GRAFX_V3D` | `PM_GRAFX` 0x10c reset bit, plus ASB master 0x0c and slave 0x08 in the `asb`/`rpivid_asb` regions; clock on/off around the reset | `PM_GRAFX_2712` 0x304 reset bit; no ASB; no domain clock | `linux:drivers/pmdomain/bcm/bcm2835-power.c:80,383-386`; `rpi:drivers/pmdomain/bcm/bcm2835-power.c:380-386` |
| `pm` node | `0x7e100000` (+ `asb`, `rpivid_asb`), clocks | `0x7d200000`, `pm` region only | `bcm2711.dtsi:109-123`; `bcm2712-ds.dtsi:122-129` |
| SMS | none | `sms` region, power-up before the first register read | section 3 |
| Reset | reset control, or the `bridge` region as fallback | reset control (`BCM2835_RESET_V3D` on `pm`); `bridge` is not in the node | `linux:v3d_drv.c:398-412`; `bcm2712-ds.dtsi:418` |
| GCA | exists below 4.1 only (so not on 4.2) | not present | `linux:v3d_drv.c:414-418` |
| TFU, GMP, CSD registers, interrupt bits, `MMUC_CONTROL` clear bit | 4.x layout | 7.x layout | section 2 |
| MMU format and page table | same | same | section 5 |
| Cache and bin/render start registers | same | same | sections 6 and 7 |
| Revision rule for the indirect CSD count | n/a | rev below 6 subtracts 1 | `linux:v3d_sched.c:454-458` |

## 13. Open items of the pre-study (U1, U3 to U7)

| Item | Status after this reading |
|---|---|
| **U1** BCM2712 register manual | **Open.** A `grep -rl BCM2712` over the documentation repository at the commit above finds 16 files: product pages (Compute Module, keyboard computers, Industrial, single-board computers), revision codes and OTP bits, the security architecture and the secure-boot provisioner, boot pages (EEPROM, network boot), `configuration/reference.adoc`, `legacy_config_txt/memory.adoc`, `linux_kernel/building.adoc`, `io-controllers/rp1.adoc`, and the processor page `computers/processors/bcm2712.adoc` — an overview with a "Vulnerabilities and mitigations" section. None of the 16 is a register manual. Whether one is published elsewhere is not answered. |
| **U3** Display | **Answered in part.** The path (HVS, pixel valve, HDMI) and every address, interrupt and clock Linux uses are in section 10; the PHY sequence is in section 10 as well. The firmware can leave a scanout running and Linux has code to adopt or stop it (`vc4_drv.c:397-400`, `vc4_hdmi.c:3391-3399`). **Open:** whether a Zephyr image started with `kernel=` finds a running scanout, its address, and whether the property mailbox still hands out a framebuffer. All display nodes are `disabled` in the base device tree. |
| **U4** Real PA width; line-exact citations | **Answered in part.** Line-exact citations: this file. The PA width is read from `MMU_DEBUG_INFO.PA_WIDTH` at run time (`linux:v3d_drv.c:374-376`) and stays **open** until a board is read. The PFN limit gives at most 36 physical bits through the page table (`linux:v3d_mmu.c:101`). |
| **U5** Clock, power, reset | **Answered in part.** The clock is firmware clock 5 through the property mailbox; the power-domain and reset is one bit in `pm` 0x304 (section 9). The misspelt `clocks-names` has no effect because Linux requests the clock without a name (`linux:v3d_drv.c:362`). **Open:** whether the GPU is already clocked and out of reset when the image starts. |
| **U6** Start state of the image | **Answered in part, from the generic protocol only** (section 11). What the firmware does for this board — exception level and register state — is **open**. The default load addresses are documented (legacy page): 32-bit images at `0x8000`, 64-bit at `0x200000` (`docs:legacy_config_txt/boot.adoc:38-40`); `config_txt/boot.adoc` does not name `kernel_address`. |
| **U7** `tryboot` on the Pi 5 | **Answered in part.** The documentation says "all Raspberry Pi models support `tryboot`", with a caveat only for Pi 4 Model B revisions 1.0 and 1.1 (`docs:raspberry-pi/bootflow-eeprom.adoc:91`); the flag is one-shot and loads `tryboot.txt` (`:82`); with secure boot it loads `tryboot.img` (`:93`); the `[tryboot]` filter is in `config_txt/conditional.adoc:255-261`. The "`TRYBOOT` property table" the note named is **not** in `raspberry-pi/eeprom-bootloader.adoc` at this commit (`grep TRYBOOT` finds none). Whether it works with the EEPROM a given board ships is **open** (a board trial). |

### New findings

- **GIC interrupt numbers.** SPI 250 and 249 are interrupt IDs 282 and 281.
  Zephyr's stock `rpi_5` sets `NUM_IRQS` to 280
  (`zephyr:soc/brcm/bcm2712/Kconfig.defconfig:6-7`; the count is a Kconfig
  default, not a device-tree property), so both IDs are outside its table:
  the IRQ entry treats them as spurious (`zephyr:arch/arm64/core/isr_wrapper.S:89-91`)
  and `IRQ_CONNECT` of either fails the build
  (`zephyr:scripts/build/gen_isr_tables_parser_local.py:211-215`). The GIC
  driver itself sizes the distributor from `GICD_TYPER`
  (`zephyr:drivers/interrupt_controller/intc_gic.c:242-246`). The fix is
  `CONFIG_NUM_IRQS=288` (283 or more) in the image's `prj.conf`, no overlay
  or patch. Needed from M5 on (the first clear polls); that the GIC-400
  reports at least 288 lines is open until a board image reads it (section 8).
- **Mailbox without Linux.** The BCM2712 property mailbox is the BCM2835
  block at ARM 0x10_7c013880 with no bus alias for the buffer; the V3D clock
  is firmware clock 5; the V3D power domain in the device tree is the `pm`
  block's domain 1, not the firmware's domain 10 (section 9, the mailbox
  table).
- **Zephyr's start.** Zephyr overwrites x0 before any hook; the probes keep
  it with `--wrap=__start` (section 11).
- **Two firmware-DTB facts for day one.** The V3D node needs the
  `vc4-kms-v3d` overlay to be `okay` in a Linux DTB; the DTB the firmware
  hands to `kernel=` has the node `disabled`, but its `reg`, `interrupts`
  and `clocks` are still readable (D1).
