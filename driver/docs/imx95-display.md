<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Verdin iMX95 on Mallow: the picture from Zephyr

Status: design (2026-11-20; the display fixed 2026-11-21), implemented
for route (b) and host-tested, not run on a board. The cold X4 path of
section 4(b) is `soc::imx95::{display_power, display, dpu, dsi_power,
combo_phy, dsi}` and `board::verdin_imx95_{dsi, display}` in
`driver/rust` (host tests against a mock System Manager, a display model, a
DSI model and the bridge model, the twelve steps in their order), behind
`yd_imx95_display_up` / `_flip` / `_down` (`ffi::imx95_display`); the
Verdin's Zephyr board layer shows `examples/triangle`'s window with it
(`driver/zephyr/lib/board/verdin_imx95/verdin_imx95_display.c`, the I2C and
the expander through Zephyr's drivers, U19), built by the gate's Verdin
iMX95 row. Nothing here was run on a board.
The board is fixed by the owner: a Toradex Verdin iMX95 module on a Toradex
Mallow carrier, with a **Toradex Capacitive Touch Display 10.1" DSI on the
Mallow's X4 (MIPI DSI) connector** and no LVDS panel on X12 (owner,
2026-11-21; U1). The goal is the
other boards' screen path on it: a Zephyr image puts a GPU-rendered picture
on a display, with the console beside it, in the product form (U-Boot starts
Zephyr, no Linux before it). This note names the display hardware, what the
Mallow puts behind it, three routes in the order the house method takes them
(adopt, cold, none), the cold DSI path in enough detail to implement it
(section 4(b)), the board checks, and what is still open.
The overall i.MX 95 pre-study is a separate note; this one is the display
only.

## Sources and rules

| Prefix | Tree or document | Version read |
|---|---|---|
| `ml:` | mainline Linux, `github.com/torvalds/linux` `master` | `65e4fd942f72` (2026-11-19) |
| `nxp:` | NXP's kernel, `github.com/nxp-imx/linux-imx` branch `lf-6.18.y` | `6e694eda40aa` (2026-09-08) |
| `tdx:` | Toradex's kernel, `git.toradex.com/linux-toradex.git` branch `toradex_6.18-2.1.x-imx` (it carries Toradex's overlays in the tree) | `851ac0f91f87` (2026-11-19) |
| `tub:` | Toradex's U-Boot, `git.toradex.com/u-boot-toradex.git` branch `toradex_imx_lf_v2026.04` | `f51b30894e05` (2026-10-28) |
| `nub:` | NXP's U-Boot, `github.com/nxp-imx/uboot-imx` branch `lf_v2026.04` | `bb41975aeb4c` (directory listing and one defconfig) |
| `mub:` | mainline U-Boot, the `github.com/u-boot/u-boot` mirror, `master` | `ece349ade297` (the mirror's last commit, 2026-07-06; directory listing only) |
| `hal:` | `github.com/zephyrproject-rtos/hal_nxp` `master` | `0f32f1caa5cf` (2026-11-19) |
| `z:` | `github.com/zephyrproject-rtos/zephyr` `main` | `4f2d5263e21e` (2026-11-20) |
| `hal:` (DSI) | hal_nxp's DW MIPI DSI driver `mcux/mcux-sdk-ng/drivers/mipi_dsi_imx/fsl_mipi_dsi.c` (the one Zephyr's `dsi_nxp_dwc.c` calls for the i.MX 95) and the camera-side headers `PERI_CAMERA_DSI_MASTER_CSR.h`, `PERI_CAMERA_DSI_OR_CSI_PHY_CSR.h`, `PERI_CAMERA_DSI_STREAM_CSR.h` (`devices/i.MX/i.MX95/periph/`) | `0f32f1caa5cf`, as `hal:` |
| `nxp:` (bridge) | `drivers/gpu/drm/bridge/ti-sn65dsi83.c` of the same tree | `6e694eda40aa` |
| `tsm:` (sources) | `boards/tdx-common/sm/brd_sm_tdx_common.c`, `devices/MIMX95/sm/dev_sm_perf.c` of the same tree | `3aa8a77a15d6` |
| `sm:` | NXP's System Manager, `github.com/nxp-imx/imx-sm` `master` (its EVK configuration, for comparison) | `eec232878ab7` (2026-09-03) |
| `tsm:` | Toradex's System Manager, `git.toradex.com/imx-sm-toradex.git` branch `main`: `configs/verdin-imx95.cfg`, the generated `configs/verdin-imx95/config_scmi.h`, and the SM sources it builds | `3aa8a77a15d6` (2026-11-16) |
| `V95:` | Toradex, Verdin iMX95 Datasheet, Rev. 0.7, 20-Jul-2026 (preliminary), `docs.toradex.com/200007-verdin_imx95_datasheet.pdf` | |
| `MAL:` | Toradex, Mallow Carrier Board V1.1 Datasheet, Rev. 1.9, 24-Sep-2025, `docs.toradex.com/116791-mallow_carrier_board_datasheet.pdf` | |
| `repo:` | this repository | `f7ab6fa0` |

Device-tree paths are under `arch/arm64/boot/dts/freescale/` unless they
say otherwise. `nxp:` and `tdx:` carry the same `imx95.dtsi` display nodes at
the same line numbers; the note cites `nxp:`.

Licences, and what each source may be used for:

| Source | Licence | Use here |
|---|---|---|
| Linux drivers (`nxp:` `dpu95/`, the `imx95-*` bridges, `ti-sn65dsi83.c`, `phy-fsl-imx8mp-lvds.c`; `ml:` `clk-imx95-blk-ctl.c`) | GPL-2.0 (`dpu95`, bridges, PHY: GPL-2.0+; the clock file GPL-2.0) | facts and `file:line` only, never code, comments or tables (`docs/source-form.md`) |
| U-Boot drivers (`tub:` `drivers/video/nxp/imx/imx95_dpu.c`, `imx95-ldb.c`, `imx95-pixel-*.c`, `dw_dsi_imx95.c`) | GPL-2.0+ | as Linux |
| Device trees (`ml:`/`nxp:` `imx95.dtsi`; `tdx:` overlays; `ml:` Verdin files) | `GPL-2.0-only OR MIT` (`imx95.dtsi`, the Toradex overlays), `GPL-2.0-or-later OR MIT` (the Verdin and Mallow files) | facts; the MIT side would allow more, not needed |
| `hal:` register headers (`PERI_DISPLAY_SEERIS.h`, `PERI_LDB.h`, `PERI_DISPLAY_*.h`, `PERI_MIPI_DSI.h`, `MIMX9596_ca55_COMMON.h`) and drivers (`fsl_dpu.c` in `drivers/dpu_1`, `fsl_ldb.c` in `drivers/ldb_1`, `fsl_mipi_dsi.c` in `drivers/mipi_dsi_imx`), the `PERI_CAMERA_DSI_*` headers | BSD-3-Clause (NXP) | **register map and sequence source**; may be generated from or vendored unmodified, as the i.MX8MP's `regs.rs` is (`tools/hal-nxp/gen_regs.py`) |
| `z:` `display_socionext_dpu.c`, `dsi_nxp_dwc.c` and the i.MX 95 M7 device tree | Apache-2.0 | sequence source; may be read and vendored |
| `sm:`, `tsm:` configuration and sources | BSD-3-Clause | facts |
| Datasheets | Toradex documents | facts by section and page |

Not read: the i.MX 95 Reference Manual (IMX95RM). Its address
`nxp.com/docs/en/reference-manual/IMX95RM.pdf` answers "Not found" without an
NXP account; an account was not used. Marek Vasut's i.MX95 display series
says "the DPU chapter is missing from the i.MX95 RM" (cover letter,
2025-10-11, read through its LWN copy, `lwn.net/Articles/1041688/`); that is
his statement, not checked here. lore.kernel.org and patchwork were not
searched directly; the two threads below were read through mirrors (LWN,
and the dri-devel archive at `ratatoskr.run`). Toradex's System Manager
configuration is `tsm:` (its repository is `imx-sm-toradex`, not `imx-sm`).
The Verdin iMX95 developer page was
read; the Mallow product page refused the fetch (HTTP 403), its datasheet was
read instead. Toradex's Dahlia and Development Board datasheets were not
read.

## 1. The display hardware

```
 DDR (linear XRGB8888; the DPU node has no IOMMU)
  |  AXI
  v
 DPU "Seeris" 0x4B400000 (4 MiB)                     display CSR 0x4B010000
  FetchLayer0 0x1D0000 --+                           +0x0 engine clock select (CCM | DSI PLL | LDB PLL/7)
  ConstFrame0 0x0F0000 --LayerBlend1 0x170000        +0x4 pixel interleaver control
                         -> ExtDst0 0x110000         +0x8 pixel link control
  display engine 0: DomainBlend0 0x2A0000 -> FrameGen0 0x2B0000 -> Signature0 0x2D0000
  display engine 1: DomainBlend1 0x320000 -> FrameGen1 0x330000
        | disp0, disp1
        v
 pixel interleaver 0x4B0D0000 (two channels, step 0x10000)
        |                                    \ (DSI only)
        v                                     pixel link -> camera/DSI CSRs 0x4AD00000.. -> DW MIPI DSI 0x4ACF0000
 LVDS CSR 0x4B0C0000: +0x0 PHY clock, +0x4 LDB (pixel mapper),     -> combo D-PHY (shared with CSI 1) -> DSI_1 pins
                      +0x8 LVDS0 PHY, +0xC LVDS1 PHY
        |
        v
 LVDS0 (ch 0), LVDS1 (ch 1) -> Verdin pins 88..144
 power: SCMI domain DISPLAY (13); the DSI side also CAMERA (3)
```

| Block | Base, size | Interrupt | Clocks (SCMI ids) | Power | Source |
|---|---|---|---|---|---|
| DPU (`nxp,imx95-dpu`; the M7 tree calls it `socionext,dpu`) | 0x4B400000, 0x400000 | through the display irqsteer, 86 lines (store9, extdst0/1/4/5, domainblend0, framegen0/1, sig0/2, ...) | `pix` DISP1PIX (80), `apb` DISPAPB (75), `axi` DISPAXI (76), `ocram` DISPOCRAM (78), `ldb` LDB PLL / 7, `ldb_vco` LDBPLL_VCO (37) | DISPLAY (13) | `nxp:imx95.dtsi:2584-2726` (clocks `:2696-2701`, domain `:2703`); `z:dts/arm/nxp/imx/nxp_imx95_m7.dtsi:1093-1099`; ids `ml:imx95-clock.h:46-47,87-92`, `ml:imx95-power.h:22` |
| display irqsteer | 0x4B0B0000, 0x1000 | GIC SPI 214-219 (two slots reserved) | DISPAPB | DISPLAY | `nxp:imx95.dtsi:2405-2424` |
| display CSR (`nxp,imx95-display-csr`, the blk-ctrl) | 0x4B010000, 0x10000 | | DISPAPB; sets DISPOCRAM 400 MHz, DISPAPB 133 MHz | DISPLAY | `nxp:imx95.dtsi:2331-2342`; registers `hal:mcux/mcux-sdk/devices/MIMX9596/periph/PERI_DISPLAY_BLK_CTRL_DISPLAYMIX.h:408-415` |
| pixel link (`nxp,imx95-dc-pixel-link`, a register of the display CSR) | display CSR + 0x8 | | | | `nxp:imx95.dtsi:2345-2403` |
| pixel interleaver | 0x4B0D0000, 0x20000 | irqsteer 62 | DISPAPB | | `nxp:imx95.dtsi:2526-2582`; `hal:.../PERI_DISPLAY_INTERLEAVER.h:411-421` |
| LVDS CSR (`nxp,imx95-lvds-csr`), LDB at +0x4, LVDS PHY 0 at +0x8, PHY 1 at +0xC | 0x4B0C0000, 0x10000 | | DISPAPB; LDBPLL (38), LDB PLL / 7 (a fixed /7 of LDBPLL) | DISPLAY | `nxp:imx95.dtsi:341-348,2426-2524`; `hal:.../PERI_LDB.h:408-412`; base `hal:mcux/mcux-sdk/devices/MIMX9596/MIMX9596_ca55_COMMON.h:1817` |
| MIPI DSI host (Synopsys DW MIPI DSI, NXP glue `nxp,imx95-mipi-dsi`) | 0x4ACF0000, 0x10000 | irqsteer 48 | MIPIPHYCFG (66), MIPIPHYPLLBYPASS (67), MIPIPHYPLLREF (68), CAMAPB (62), DISP1PIX (80) | **CAMERA (3)** | `nxp:imx95.dtsi:2104-2127` (domain `:2122`); `nxp:drivers/gpu/drm/bridge/imx/imx95-mipi-dsi.c:23` (the DW core); `hal:.../PERI_MIPI_DSI.h` |
| DSI stream CSR, DSI master CSR (pixel link select at +0x4), combo PHY CSR, MIPI TX PHY CSR | 0x4AD00000, 0x4AD10000, 0x4AD20000 (0x114), 0x4AD20100 (0x14) | | CAMAPB | CAMERA | `nxp:imx95.dtsi:2155-2225`; `hal:.../MIMX9596_ca55_COMMON.h:810-830` |

The DPU's units at their offsets from 0x4B400000, as both GPL drivers and
the BSD header give them: ConstFrame 0x0F0000/0x100000/0x130000/0x140000,
ExtDst 0x110000/0x120000/0x150000/0x160000, LayerBlend 0x170000..0x1C0000,
FetchLayer 0x1D0000/0x1E0000, FetchYUV 0x200000.., FetchEco 0x210000..,
DomainBlend 0x2A0000/0x320000, FrameGen 0x2B0000/0x330000, Signature 0x2D0000
and 0x2E0000 (`nxp:drivers/gpu/drm/imx/dpu95/dpu95-data.h:16-17,54,87-88,107-108,126,141,159-160,196-197`;
`tub:drivers/video/nxp/imx/imx95_dpu.c:669-688,1031-1052`;
`hal:mcux/mcux-sdk-ng/devices/i.MX/i.MX95/periph/PERI_DISPLAY_SEERIS.h:1063-1081,1700-1745,1833-1842`).

Three facts change the house method for this SoC:

- **A permissive register map exists for every block.** hal_nxp (BSD-3)
  has `PERI_DISPLAY_SEERIS.h` for the i.MX 95: 38,294 lines, rev. 4.0 of
  2026-02-28, every DPU register with its fields and a one-line meaning
  (FetchLayer0 `BASEADDRESS0` 0x1D0018 and `BASEADDRESSMSB0` 0x1D001C,
  `SOURCEBUFFERATTRIBUTES0` 0x1D0028, `LAYERPROPERTY0` 0x1D0048; FrameGen0
  `HTCFG1` 0x2B000C, `FGENABLE` 0x2B008C, `FGTIMESTAMP` 0x2B0098, `FGCHSTAT`
  0x2B009C with `PrimSyncStat` bit 8; `PERI_DISPLAY_SEERIS.h:1063-1081,1708-1745,23394`).
  The LVDS CSR, the display CSR, the interleaver, the pixel mapper and the
  DSI host have their own BSD headers. The i.MX8MP display was written from
  its RM and hal_nxp (`driver/docs/imx8mp-display-cold.md`); the i.MX 95 can
  be written from hal_nxp alone, with the GPL drivers only to cross-check.
- **A permissive driver exists for the DPU and the LDB.** hal_nxp's
  `drivers/dpu_1/fsl_dpu.c` (4,918 lines, BSD-3) drives `DISPLAY_SEERIS_Type`
  (`DPU_Init` `:1074`, `DPU_InitFetchUnit` `:1873`, `DPU_InitDisplayTiming`
  `:2303`, `DPU_StartDisplay` `:2466`); `drivers/ldb_1/fsl_ldb.c` (96 lines,
  BSD-3) is the i.MX 95 LVDS sequence (`:33-95`). Upstream Zephyr drives the
  i.MX 95 DPU from the M7 with them: `z:drivers/display/display_socionext_dpu.c`
  (387 lines, Apache-2.0, FetchYUV0, LayerBlend1, ExtDst0, FrameGen0,
  `:209-330`) and `z:drivers/mipi_dsi/dsi_nxp_dwc.c` (the DSI host, combo PHY
  and pixel link, `:109-126`), shown on a Raydium RM692C9 DSI panel
  (`z:boards/shields/nxp_mx9_dsi_oled/`). Zephyr has no LVDS path and no
  Verdin iMX95 board.
- **The DPU has a signature unit.** Signature0 at 0x2D0000 measures a CRC per
  colour channel over up to several windows of the output
  (`CRC_R/G/B_WINDOW0` at 0x2D0400-0x2D0408, reference values and a status
  register; `PERI_DISPLAY_SEERIS.h:1833-1842,1960-1965`). No driver read uses
  it. It is the nearest thing to a camera this display has (section 4).

The SCMI side: the i.MX 95 runs a System Manager on the M33. Linux and
U-Boot reach it through a messaging unit with shared memory in its SRAM
(`ml:imx95.dtsi:338-340,1590-1612`); power domains are SCMI protocol 0x11,
clocks 0x14, performance 0x13.

**Who owns the display on the Verdin iMX95 (Toradex's configuration,
`tsm:`).** The A55 logical machine "AP" (`configs/verdin-imx95.cfg:443`) has
two SCMI agents: "AP-S" (agent 1, secure, MU 1, `:496-499`) and "AP-NS"
(agent 2, non-secure, MU 3, `:570-573`). U-Boot, Linux and our image are
AP-NS. AP-NS holds, all with permission ALL (the highest,
`sm/rpc/scmi/rpc_scmi_config.h:83-89`):

| What | Entry |
|---|---|
| power domains DISPLAY (13) and CAMERA (3) | `config_scmi.h:458-459` (the configuration tool derives them; the `.cfg` has no line for them) |
| performance domains DISP (12) and CAM | `config_scmi.h:467-468` |
| clocks DISP1PIX (80), LDBPLL and LDBPLL_VCO (38, 37), VIDEOPLL1 and its VCO (19, 18), MIPIPHYCFG, MIPIPHYPLLBYPASS, MIPIPHYPLLREF, MIPITESTBYTE (66-69), LPI2C3 | `config_scmi.h:251,269-270,272,294-297,315-316`; `.cfg:584,596` |
| resources, owner with non-secure read/write (`.cfg:562`): DC and all its parts, BLK_CTRL_DISPLAYMIX, BLK_CTRL_CAMERAMIX, CAMERA1-8, LVDS, MIPI_CSI0/1, MIPI_DSI, MIPI_PHY, VIDEO_PLL1, LPI2C3, LPTPM5, LPTPM6, GPIO5 | `.cfg:608-643,755,776,794-795,804-808,892` |
| pins: GPIO_IO06 (TPM5, `PWM_2`, the X12 backlight), XSPI1_DATA4 (GPIO5_IO04, the backlight enable), GPIO_IO12, GPIO_IO19 (`PWM_3_DSI`), GPIO_IO28/29 (`I2C_2_DSI`); the LVDS and DSI pads are dedicated and have no pin entry | `.cfg:946,952,959,968-969,1021`; pads `ml:imx95-verdin.dtsi:885-887,1027-1028,1037-1038` |

**DISPAPB, DISPAXI and DISPOCRAM (75, 76, 78) belong to no agent.** They are
on the System Manager's own list "Modified via PERF protocol"
(`.cfg:149-170`); no agent has a clock permission for them, so an enable or
a rate request returns `DENIED` (permission below SET or EXCLUSIVE,
`sm/rpc/scmi/rpc_scmi_clock.c:1078-1086,1268-1276`); reading the rate is not
checked. The System Manager runs them with the DISPLAY domain
(`devices/MIMX95/sm/dev_sm_clock.c:1511-1519` puts them, with VIDEOPLL1,
LDBPLL and DISP1PIX, in that domain) and sets their rates from the DISP
performance level (`devices/MIMX95/sm/dev_sm_perf.c:1277-1283,2093,2302`).
CAMAPB is the same for the DSI side (`.cfg:164`). So the image does not
switch these clocks: it turns DISPLAY (and, for DSI, CAMERA) on. Toradex's
System Manager starts every performance domain without a local PLL at its
run level, overdrive (`BOARD_PERF_LEVEL` = `DEV_SM_PERF_LVL_ODV`,
`brd_sm_tdx_common.c:107,163`; the loop `dev_sm_perf.c:1374-1394` clamps only
the DRAM and A55 domains, `:1871-1909`), and the configuration sets no other
level (`.cfg` has only `PERF_A55` lines). DISP (12) and CAM (11) therefore
run at ODV, DISPAXI and CAMAXI from the 800 MHz parent (`dev_sm_perf.c:815-836,
921-942`), unless an agent lowered them: no perf request is needed, Z0 reads
the level.

**What turns them off.** The System Manager keeps one vote per agent per
power domain and clock. It clears an agent's votes, which turns a domain off
unless another agent voted for it, in one function
(`sm/rpc/scmi/rpc_scmi_base.c:1024-1130`, power `:1127` and
`sm/rpc/scmi/rpc_scmi_power.c:791-806`, clocks `:1120`), called in two cases
only: the logical machine is shut down or reset (`sm/lmm/lmm_sys.c:1055-1066`
into `sm/lmm/lmm.c:265-276` and `sm/rpc/scmi/rpc_scmi.c:456-470`), or an
agent sends BASE_RESET_AGENT_CONFIGURATION, which needs a base permission of
PRIV over the target (`rpc_scmi_base.c:913-952`) and no agent in this
configuration has any base permission (`config_scmi.h` has no `basePerms`).
The AP machine's stop list names only the A55 domains (`.cfg:452-492`), and
no other agent may vote for DISPLAY (the M7 agent holds only its own domain,
`config_scmi.h:93`). So:

- a U-Boot `go` or `booti`, a normal kexec and a crash kexec are invisible to
  the System Manager: the same agent on the same channel, no machine reset.
  The DISPLAY domain and the clock votes stay as the previous program left
  them. A normal kexec still loses the display, because Linux's own drivers
  power it down on the way out; a crash kexec skips them and keeps it.
- a reset of the AP machine (its watchdogs WDOG4 and the software fault give
  `lm_reset`, WDOG3 `sys_reset`, `.cfg:551-553`) turns the display off.
- our image can turn the display off itself (a DISPLAY off vote from AP-NS);
  the adopt probe must not send one.

## 2. The Mallow, and the other boards

What the Verdin iMX95 brings to the Verdin edge connector:

- **No HDMI.** The `HDMI_1_*` pins are "Not connected. HDMI not supported on
  Verdin i.MX 95" (`V95:` section 3.2, Table 8, page 17); "HDMI port is not
  natively supported ... can be enabled with a DSI-to-HDMI bridge solution on
  the carrier board" (`V95:` 5.4, page 40).
- **One 4-lane MIPI DSI** on `DSI_1` (pins 23-49), straight from the combo
  D-PHY that the DSI shares with one CSI-2 receiver; no multiplexer on the
  module, the mode chosen by `Tx_Rxz` in `COMBO_PHY_MODE_CONTROL`
  (`V95:` 5.4.1, Table 22, pages 40-41). Backlight `PWM_3_DSI` (pin 19,
  GPIO2_IO19), enable `GPIO_10_DSI` (pin 21) through the module's GPIO
  expander (Table 22, page 41).
- **Two native LVDS channels**: LVDS0 on pins 88-114, LVDS1 on pins 118-144
  (module-specific pins), single or dual channel, 18 or 24 bit; "both LVDS
  displays must use the same resolution and timing" (`V95:` 5.4, 5.4.2,
  Tables 23-24, page 42).
- The display I2C buses: `I2C_2_DSI` is LPI2C3 (pins 53/55), `I2C_3_HDMI` is
  I3C2 (unused here), `I2C_1` is LPI2C4 (pins 12/14)
  (`ml:imx95-verdin.dtsi:387-392,467-486`).

### The Mallow

| Mallow connector | Signals | What sits between the SoC and it | I2C | What the SoC must drive | Source |
|---|---|---|---|---|---|
| X12 LVDS, Hirose DF13 40-pin; X13 touch (FFC 10-pin). **Not this board's setup** (nothing on X12) | `LVDS_1_A` = Verdin pins 88-114 = **LVDS0**; `LVDS_1_B` = pins 118-144 = LVDS1; backlight PWM pin 35 (Verdin 16, `PWM_2`), enable pin 37 (Verdin 46); +12 V backlight through jumper JP1 | **nothing**: native LVDS, no bridge | touch only (the panel needs none) | DPU, interleaver, LDB, LVDS PHY 0, LDB PLL; TPM5 PWM and one GPIO for the backlight | `MAL:` 2.9.3, Tables 16-17, pages 24-25; `V95:` Table 23 |
| X4 MIPI DSI, Würth 687134149022 34-pin FFC (Toradex "Simple MIPI DSI"). **This board: the Toradex Capacitive Touch Display 10.1" DSI** | `DSI_1` clock and 4 lanes (pins 2-15), `GPIO_10_DSI` (pin 17, "display is OFF when the signal is Low", pull-up on the display), `PWM_3_DSI` (18, 10K to GND), `GPIO_9_DSI` (19, from the display), `I2C_DSI` (20/21 = Verdin 55/53, 1.8K to 1.8 V), `DSI_RESET_CON#` (22 = Verdin 42), 1.8/3.3/5 V | the display carries a **TI SN65DSI83** DSI-to-LVDS bridge and a Riverdi RVT101HVDNWC00-B LVDS panel (Toradex's 7" DSI display the same bridge with an RVT70HSLNWCA0) | SN65DSI83 at 0x2C and the touch controller (Ilitek ILI2132) at 0x41, both on `I2C_2_DSI` (LPI2C3) | DPU, interleaver, pixel link, DW MIPI DSI, combo PHY, then the bridge over I2C; `GPIO_10_DSI` and `GPIO_9_DSI` sit on the module's I/O expander (section 4(b)) | `MAL:` 2.9.1, Table 14, pages 22-23; `tdx:imx95-verdin-panel-cap-touch-10inch-dsi.dtso:32-60,86-125`, `tdx:imx95-verdin-panel-cap-touch-7inch-dsi.dtso:33` |
| X14 HDMI | fed from the Verdin `HDMI_1` pins | nothing, and the module leaves those pins unconnected | | **dead with this module** | `MAL:` 2.9.2, Table 15, pages 23-24; `V95:` Table 8 |

So the Mallow has no HDMI with this module, and the owner's board takes its
picture from **X4**: the Toradex 10.1" DSI display, whose chain Toradex's
overlay `imx95-verdin-panel-cap-touch-10inch-dsi.dtso` names (DPU, pixel
link, interleaver channel 0, DW MIPI DSI, SN65DSI83, panel-lvds; section
4(b) takes it apart). X4 is a Toradex "Simple MIPI DSI" FFC, so the display
plugs in directly; the "DSI Display Adapter" the overlay's header mentions is
for the Development Board's and Dahlia's Samtec connector (`:5-8`).

X12, for the record (not this board's setup): it would have been the
shortest path to a picture, with no bridge, no I2C, no DSI PHY and one power
domain. Toradex's overlay for it (`imx95-verdin-panel-cap-touch-10inch-lvds.dtso`,
"carrier boards with a Toradex standard LVDS display connector (e.g.,
Mallow)", `:3-6`) names the whole chain: panel 1280x800 at 71.1 MHz, VESA
24-bit (`:34-62`); DPU, pixel link, interleaver channel 0, LDB channel 0,
LVDS PHY 0 (`:65-106,126-137`); LDBPLL_VCO 2,986.2 MHz and LDBPLL 497.7 MHz
("LDBPLL_VCO = 6 * 7 * pixelclk, LDBPLL = 7 * pixelclk", `:80-83`);
backlight `PWM_2` on TPM5 and enable `I2S_2_D_OUT` (GPIO5_IO04) (`:22-32`).
The pixel link enabled there is questioned upstream: Liu Ying (NXP) says the
pixel link belongs only to the DSI path (dri-devel RFC thread, 2026-07-06);
U5 below. On the DSI path the pixel link is certainly used (section 4(b)).

Mainline's Mallow file enables `I3C2`, `LPI2C3` and `LPI2C4` and puts only a
temperature sensor (0x4F) and the carrier EEPROM (0x57) on `I2C_1`
(`ml:imx95-verdin-mallow.dtsi:107-131`); no display node is in mainline for
any Verdin carrier.

### The other boards, for comparison

| Board | Display connectors | Between SoC and connector | I2C, address | Source |
|---|---|---|---|---|
| Verdin Development Board | X48 `DSI_1` (Samtec LSS-130); J10 "Verdin Mezzanine LVDS" | DSI: Toradex adapters (DSI to HDMI: Lontium LT8912B; DSI to LVDS: TI SN65DSI84; DSI display adapter to the DSI displays above); LVDS: native | LT8912B 0x48 and SN65DSI84 0x2C on `I2C_1` (LPI2C4) | `tdx:imx95-verdin-dsi-to-hdmi.dtso:5-7,65-75`; `tdx:imx95-verdin-dsi-to-lvds-panel-cap-touch-10inch.dtso:95-101`; `tdx:imx95-verdin-mezzanine-panel-cap-touch-10inch-lvds.dtso` header |
| Dahlia | X17 `DSI_1` (LSS-130) | the same DSI adapters | as above | the same overlays' headers |
| Ivy, Yavia, Zinnia | not studied (Ivy and Zinnia enable neither `I2C_2_DSI` nor `I2C_3_HDMI` in mainline) | | | `ml:imx95-verdin-ivy.dtsi`, `-zinnia.dtsi` |
| NXP i.MX 95 EVK (19x19) | LVDS through NXP's LVDS-to-HDMI adapter; DSI through an ADV7535 adapter | ITE IT6263; ADV7535 | IT6263 0x4C on LPI2C3; ADV7535 0x3D on LPI2C2 | `nxp:imx95-evk-it6263-lvds0-common.dtsi:29-34`; `nxp:imx95-19x19-evk-adv7535.dtso:14-19` |
| NXP FRDM i.MX 95 (15x15) | HDMI on the board | IT6263 on LVDS channel 1 | 0x4C on LPI2C4 | `nxp:imx95-15x15-frdm.dts:343-357,497-526` |

The FRDM's on-board IT6263 is the only HDMI path among the candidates that
needs no adapter; it is also the one path validated by the upstream port
(below). For the Mallow it does not matter.

### What of our code carries over

| Ours | From | On the Mallow |
|---|---|---|
| `ip::sn65dsi8x` (`driver/rust/src/ip/sn65dsi8x.rs`) | the Verdin AM62P's SN65DSI83 and the Verdin iMX8MP's SN65DSI84 (`driver/docs/am62p-display-cold.md`, `driver/docs/imx8mp-display-cold.md`) | **yes, unchanged**: the i.MX8MP drives the same panel (Riverdi 10.1", 1280x800, 1404 x 840 total) through the same bridge family; only the pixel clock (70 MHz) and the lane rate (420 Mbit/s) differ (section 4(b)) |
| `ip::dw_mipi_dsi` (`driver/rust/src/ip/dw_mipi_dsi.rs`) | the STM32MP25's Synopsys DW MIPI DSI host | **the host sequence and plan**: the i.MX 95 DSI is the same DW core (`nxp:drivers/gpu/drm/bridge/imx/imx95-mipi-dsi.c:23`). Two changes: `probe` accepts ST's versions only (`:447-452`), and the `Phy` trait (`:265`) gets a new implementation for the NXP combo D-PHY (section 4(b)) |
| `firmware::scmi` (`driver/rust/src/firmware/scmi.rs`) | the STM32MP2 over OP-TEE | **the codec and client** (power, clock, perf); the `Transport` trait (`:127`) needs an i.MX MU-plus-SRAM transport, and the client a CLOCK_PARENT_SET (message 0xD, `tsm:sm/rpc/scmi/rpc_scmi_clock.c:61,524`) for DISP1PIX and the MIPI PHY clocks |
| `ip::adv7535` | the STM32MP257F-DK | no (NXP's EVK adapter only) |
| `ip::lcdifv3`, `ip::samsung_dsim`, `ip::cdns_dsi`, `ip::ti_dss7` | i.MX8MP, AM62P | no: different controller and DSI host |
| LT8912B | none in the tree | only for the DSI-to-HDMI adapter, which does not fit the Mallow's X4 |
| an LPI2C, a PCAL6416A and a TPM PWM driver | none in Rust (`ip::stm32_i2c`, `ip::imx_pwm` are other blocks) | LPI2C3 for the bridge, LPI2C2 for the module's I/O expander that holds the bridge enable, TPM6 for the backlight (section 4(b)). The bridge driver reaches I2C and GPIOs through `Env::i2c_write`, `i2c_write_read`, `gpio_set` (`driver/rust/src/env.rs:54-55`, served by the C side): Zephyr's `nxp,lpi2c` and PCA-series GPIO (`nxp,pcal6416`) drivers serve them on this A55 board (U19, answered by the build); the TPM PWM is ours (`ip::tpm_pwm`) |

The X12 LVDS path would have needed no bridge code; the X4 path reuses the
bridge and the DSI host and adds the combo D-PHY.

## 3. What exists upstream, and what U-Boot leaves

| Where | State | Source |
|---|---|---|
| mainline Linux | **no i.MX 95 display.** `imx95.dtsi` has no display node; the clock and power ids exist; the blk-ctrl clock driver knows `nxp,imx95-display-csr`, `-lvds-csr` and `-display-master-csr`; `drivers/gpu/drm/imx/dc/` matches only `fsl,imx8qxp-dc`; `fsl-ldb.c` has no i.MX 95 entry | `ml:imx95.dtsi` (grep); `ml:drivers/clk/imx/clk-imx95-blk-ctl.c:163-257,532-537`; `ml:drivers/gpu/drm/imx/dc/dc-drv.c:247`; `ml:drivers/gpu/drm/bridge/fsl-ldb.c:381-385` |
| posted series | Marek Vasut, "[PATCH 00/39] Add i.MX95 DPU/DSI/LVDS support", 2025-10-11: the DPU in the `dc` driver, tested on a "Toradex i.MX95 Verdin EVK v1.2" (DSI through LT8912, single LVDS panel); review asked for a separate module; not merged. Piyush Patle, "[RFC] drm/imx: upstream direction for i.MX95 display support", 2026-06-24: an LVDS-only port validated on the FRDM (IT6263, channel 1, 1080p60); open issues: a full modeset wedges the ExtDst shadow load (FrameGen must be disabled and re-enabled), the clock list is wrong, the two display engines are not modelled; no series posted by 2026-10-18 | `lwn.net/Articles/1041688/`; the dri-devel thread (read through `ratatoskr.run/dri-devel/2026/06/17170118/t`) |
| NXP's kernel | the full stack: `dpu95/` (11,743 lines; 3,383 of them the blitter, local dimming and other SoCs' data), `imx95-pixel-interleaver.c` (402), `imx95-pixel-link.c` (315), `imx95-ldb.c` (578), `imx95-mipi-dsi.c` (1,348), the LVDS PHY in `phy-fsl-imx8mp-lvds.c` (317, shared with i.MX8MP/93) | `nxp:drivers/gpu/drm/imx/dpu95/`, `nxp:drivers/gpu/drm/bridge/imx/`, `nxp:drivers/phy/freescale/phy-fsl-imx8mp-lvds.c:297-299` |
| Toradex's kernel | NXP's stack plus the Verdin overlays (section 2) | `tdx:` |
| mainline U-Boot | no i.MX 95 video: `drivers/video/imx/` holds IPU, LCDIF and LDB of older SoCs; no Verdin iMX95 defconfig | `mub:drivers/video/imx/`, `mub:configs/` |
| NXP's U-Boot | i.MX 95 video drivers (`imx95_dpu.c`, `imx95-ldb.c`, the interleaver and pixel link, `dw_dsi_imx95.c`) and EVK defconfigs that light a panel (`imx95_19x19_evk_lvds_defconfig`: `VIDEO_IMX95_DPU`, `_LVDS`, `PHY_IMX8MP_LVDS_PHY`, `SPLASH_SCREEN`) | `nub:drivers/video/nxp/imx/`, `nub:configs/imx95_19x19_evk_lvds_defconfig` |
| **Toradex's U-Boot** | carries the same drivers but **does not use them**: `verdin-imx95_defconfig` enables no video; its device tree includes the upstream `imx95.dtsi`, which has no display node; the only Verdin iMX95 device tree in it is `imx95-verdin-wifi-dev` | `tub:configs/verdin-imx95_defconfig:13,41`; `tub:arch/arm/dts/imx95-verdin.dtsi:12` |
| Zephyr | DPU and DSI from the M7 (section 1); no A55 display, no LVDS, no Verdin iMX95 | `z:` |

The U-Boot DPU driver is the shortest whole sequence read: 1,082 lines for
one plane (FetchLayer0 into LayerBlend1 over ConstFrame0, ExtDst, DomainBlend,
FrameGen; XRGB8888; the display CSR clock select `DSIP_CLK_SEL` = 2 for LVDS
PLL / 7) (`tub:drivers/video/nxp/imx/imx95_dpu.c:331-334,543-599,716-822`).
Its `remove` stops the FrameGen and the fetch (`:1017-1029`); the driver
sets only `DM_FLAG_PRE_RELOC` (`:1080`).

## 4. Routes

### (a) Adopt a display something else left running

What the scanout looks like from the CPU, from the BSD header: FrameGen0
`FGENABLE` (0x4B6B008C) bit 0 set, `FGCHSTAT` (0x4B6B009C) `PrimSyncStat`
set, `FGTIMESTAMP` (0x4B6B0098) moving; the plane is the fetch unit the
previous stage used: FetchLayer0 `BASEADDRESS0`/`MSB0` (0x4B5D0018/1C),
`SOURCEBUFFERATTRIBUTES0` (stride and bits per pixel, 0x4B5D0028),
`SOURCEBUFFERDIMENSION0` (0x4B5D002C), `LAYERPROPERTY0` (source buffer enable,
0x4B5D0048), or FetchYUV0 for Zephyr's own M7 driver. The active size is
FrameGen0 `HTCFG1`/`VTCFG1`. Writing pixels into that buffer and reading them
back is the whole adopt. Nothing may be read before the DISPLAY domain is
known to be on (U6).

Who could leave one on the Mallow:

| Previous stage | Leaves a display? | Note |
|---|---|---|
| Toradex's U-Boot as shipped | **no** (section 3) | |
| a Toradex U-Boot rebuilt with NXP's video drivers and a Mallow DSI device tree (with an SN65DSI83 driver, which the read U-Boot tree does not show) | yes, by NXP's EVK precedent | a boot loader of our own to carry, as the T6 note concluded for Rockchip's (`driver/docs/nanopc-t6-display.md`, 2(b)); whether `go` leaves the FrameGen running is open (U9). Not proposed |
| Linux (Toradex's 6.18 BSP with the overlay `imx95-verdin-panel-cap-touch-10inch-dsi.dtbo`), then a crash kexec into Zephyr | yes, if the board runs that kernel with that overlay applied | the T6's development route (`driver/docs/nanopc-t6-display.md`, 2(a)); the crash path skips the drivers' shutdown, so the DPU keeps scanning; the logical machine is not reset (section 1). Needs kexec and a crash reservation in that kernel (U8) |
| mainline Linux | no (no driver) | |

Needs: a SCMI transport only to read the DISPLAY domain state first; no
clock or power change. Size: small, as on the T6: a reader of a dozen
registers, a pattern, a read-back. Value: it gives the golden dump (the
register values of the working DSI mode on this board: DPU, display CSR,
camera-side CSRs, DW host, and the bridge's I2C registers) that the cold path
is checked against, and it settles the points where NXP's two sources
disagree (U13-U16). It is development only; it is not the product.

### (b) Cold: X4 DSI from Zephyr (the product form)

Everything below is for the owner's display: the Toradex Capacitive Touch
Display 10.1" DSI on X4, as Toradex's overlay
`tdx:imx95-verdin-panel-cap-touch-10inch-dsi.dtso` (cited `dsi.dtso:N`)
describes it. Register maps are hal_nxp's BSD headers; the sequences are
hal_nxp's `fsl_mipi_dsi.c` (BSD-3, cited `fsl_mipi_dsi.c:N`), Zephyr's
`dsi_nxp_dwc.c` (Apache-2.0, `dsi_nxp_dwc.c:N`), and, for facts only, NXP's
glue `nxp:drivers/gpu/drm/bridge/imx/imx95-mipi-dsi.c` (GPL, `glue:N`; Toradex's
copy is the same file) and `nxp:ti-sn65dsi83.c` (GPL, `sn65:N`). Where the
permissive and the GPL source give different values, both are listed and a
U-number names the board reading that decides.

**The display.**

| What | Value | Source |
|---|---|---|
| Panel | Riverdi RVT101HVDNWC00-B, 217 x 136 mm, LVDS `vesa-24`, behind the SN65DSI83 | `dsi.dtso:32-37` |
| Mode | 1280 x 800, pixel clock **70.000 MHz**; H: active 1280, front porch 36, sync 24, back porch 64, total **1404**; V: active 800, front porch 8, sync 8, back porch 24, total **840**; hsync and vsync active low, DE active high, data on the rising edge; refresh 70 MHz / (1404 x 840) = **59.36 Hz** | `dsi.dtso:39-53` |
| DSI link | 4 data lanes, RGB888 (24 bpp), video **burst** mode, no EoT packet, continuous clock (the SN65DSI83 asks for the mode flags `VIDEO`, `VIDEO_BURST`, `VIDEO_NO_HFP`, `VIDEO_NO_HBP`, `VIDEO_NO_HSA`, `NO_EOT_PACKET`, format RGB888) | `dsi.dtso:100`; `sn65:921-925` |
| Lane rate | 70 MHz x 24 / 4 = **420 Mbit/s** a lane (Linux takes `pixel clock x bpp / lanes` with no margin: `glue:887-911` through `phy_mipi_dphy_get_default_config`); DSI clock lane 210 MHz (DDR); lane byte clock 52.5 MHz | `glue:887-911,1093-1118` |
| Same as the i.MX8MP | the same panel and timing (1404 x 840) run on the Verdin iMX8MP at 69.3 MHz and 426.6 Mbit/s, 2.6 % above the pixel payload, because the Samsung DSIM broke with no margin (`driver/docs/imx8mp-display-cold.md`, "The chain"); here the DW host runs with none (risk below, U17) | |

**Clock plan (SCMI, agent AP-NS).** All rates are Toradex's overlay and NXP's
`imx95.dtsi`; the ids are `imx95-clock.h` (`IMX95_CCM_NUM_CLK_SRC` = 41).

| Clock (id) | Parent | Rate | Source |
|---|---|---|---|
| VIDEOPLL1_VCO (18) | | 3,360 MHz | `dsi.dtso:72-76` |
| VIDEOPLL1 (19) | VIDEOPLL1_VCO | 420 MHz (/8) | `dsi.dtso:72-76` |
| DISP1PIX (80) | **VIDEOPLL1** | 70 MHz (420 / 6, exact): the DPU's `pix` and the DSI host's `pix`; the DPU sets it to the mode's clock, `tub:imx95_dpu.c:592-593` | `dsi.dtso:72-75`; `nxp:imx95.dtsi:2109-2113` |
| MIPIPHYCFG (66) | 24M (2) | 24 MHz: `cfgclkfreqrange` = (24 - 17) x 4 = **0x1C** (`glue:665-672`; hal and Zephyr write 0x1C, `dsi_nxp_dwc.c:109-111`) | `nxp:imx95.dtsi:2115-2120` |
| MIPIPHYPLLREF (68) | 24M (2) | 24 MHz, the D-PHY PLL's input (Fin) | as above; `PERI_CAMERA_DSI_MASTER_CSR.h:489-495` (`Dphy_ref_clock_source` 0 = 24 MHz) |
| MIPIPHYPLLBYPASS (67) | VIDEOPLL1 | (Linux sets the parent; the PLL is in clock-generation mode, so it carries nothing) | `nxp:imx95.dtsi:2115-2120` |
| CAMAPB (62), DISPAPB/AXI/OCRAM (75, 76, 78) | | the System Manager's, from the CAM and DISP perf levels (ODV, section 1) | `tsm:` |
| LPI2C3 (129), LPI2C2 (48), TPM6 (155) | | the bridge bus (100 kHz), the expander bus (400 kHz), the backlight PWM | `ml:imx95-verdin.dtsi:395-399,468-476`; `tsm:config_scmi.h:271-272,310` |

AP-NS holds every one of these with ALL (section 1, and `config_scmi.h:271-272,310`
for the I2C and TPM clocks). DISP1PIX's parent and the PHY clocks' parents
need CLOCK_PARENT_SET.

**The NXP combo D-PHY PLL for 420 Mbit/s.** The PLL output is the DDR clock,
Fout = lane rate / 2 = **210 MHz**; Fout = Fin x M / (N x div), Fin 24 MHz,
2 MHz <= Fin / N <= 8 MHz, M 64-625, N 1-16, div 1/2/4/8 from
`vco_cntrl[5:4]` (`glue:155-170,549-651`); hal's form is
VCO = Fin x (M_field + 2) / (N_field + 1) with VCO = 2 x Fout for Fout in
160-320 MHz (`fsl_mipi_dsi.c:505-563,1153-1180`). Both searches land on the
same exact point:

| Parameter | Value | Linux (`glue:`) | hal (`fsl_mipi_dsi.c:`) |
|---|---|---|---|
| N | 4 (Fin / N = 6 MHz); field N - 1 = 3 | first exact hit, n = 4 (n = 3 gives M 53, below 64), `:594-631` | n_field = 3, `:530-560` |
| M | 70; field M - 2 = 68 = **0x44** | `:119-123,744-749` | m_field = 68, `:1290-1291` |
| VCO, div | 420 MHz, / 2 | `vco_cntrl` 0x1F (band 160-220 MHz) or 0x19 (210-330); both have div 2, Linux keeps the first: **0x1F**, `:226-238` | **0x19** (`{210, 25}`), `:1184-1205` |
| prop_cntrl | 0x0D | `:226-238` | 13, `:1109-1121` |
| hsfreqrange | | **0x16** (`{449, 0x16}`), `:241-310` | **0x05** (`{425.0, 5}`), `:1064-1100` |
| sr_osc_freq_target | | 0x7D0 (always), `:732-733` | **0x384** (Fout <= 500 MHz), `:1127-1150` |

The test-interface writes (8-bit registers reached through the DW host's
`PHY_TST_CTRL0/1` at 0xB4/0xB8, the 12-bit test code in two halves,
`fsl_mipi_dsi.c:1212-1240`, `glue:436-498`), in hal's order
(`fsl_mipi_dsi.c:1243-1317`), with the value for this mode:

| Test code | Register | Value | Differs? |
|---|---|---|---|
| 0x16A | PLL_13 `pll_mpll_prog_rw` | 0x03 | |
| 0x1AB, 0x1AA | CB_1, CB_0 (LP-RX reference) | 0x06, 0x53 | |
| 0x1AC | CB_2 `cb_clkdiv_clk_en_rw` (below 450 Mbit/s) | 0x10 | |
| 0x402 | CLK_TERMLOWCAP | 0x02 | |
| 0x272, 0x271, 0x270, 0x272 | SLEW_7 off, `sr_osc_freq_target` high, low, SLEW_7 on | 0x00, **0x03, 0x84** (hal) / **0x07, 0xD0** (Linux), 0x10 | U15 |
| 0x17A, 0x179 | PLL_29, PLL_28: M field | 0x00, 0x44 | |
| 0x178 | PLL_27: N override enable, N field | 0x98 | |
| 0x17B | PLL_30: VCO override enable, `vco_cntrl`, M override enable | **0xB3** (hal, 0x19) / **0xBF** (Linux, 0x1F) | U14 |
| 0x15E, 0x162 | PLL_1 `cpbias` 0x10; PLL_5 `gmp` 1, `int` 0 | 0x10, 0x01 | |
| 0x173-0x176 | PLL_22-25: lock thresholds | 0x02, 0x00, 0x60, 0x03 | |
| 0x166 | PLL_9 `pll_lock_sel_rw` | 0x04 | |
| 0x16E | PLL_17: `prop_cntrl`, power-on override | **0x0D** (hal: its comment says the power-on bits are set, its value does not) / **0xCD** (Linux sets bits 7:6, `glue:790-793`) | U15 |
| 0x17C | PLL_31: clock out left and right | 0x03 | |

Then hal waits 100 ms (`fsl_mipi_dsi.c:1316`). Linux writes the PLL_* codes
read-modify-write (`glue:513-531`); hal writes whole bytes. U-Boot's
`tub:dw_dsi_imx95.c` has Linux's values (`:249-250,575-581,624-641`).

**The camera-side CSRs** (CAMERA domain, CAMAPB; addresses
`hal:MIMX9596_ca55_COMMON.h:810-830,2017`, fields the `PERI_CAMERA_DSI_*`
headers):

| Register | Value | Meaning | Source |
|---|---|---|---|
| DSI master CSR 0x4AD10004 `DSI_PIXEL_LINK_CONTROL` | 0 | `Pixel_link_sel` = pixel link 0 (interleaver channel 0, display engine 0) | `MASTER_CSR.h:472-478`; `dsi_nxp_dwc.c:112-113`; Linux's mux `nxp:imx95.dtsi:2168-2173`, `glue:381-427,809` |
| DSI master CSR 0x4AD10008 `DSI_CLOCK_SETTING` | 0 | PLL reference 24 MHz, not the pixel-link clock | `MASTER_CSR.h:484-500`; `glue:823` |
| DSI master CSR 0x4AD10000 `DSI_CLOCK_GATING_CONTROL` | 0x2 | display 0 async FIFO clock on, display 1 gated, D-PHY PLL clkin/clkout/clkext on (1 = gated) | `MASTER_CSR.h:428-466`; `glue:861-862` |
| DSI stream CSR 0x4AD00000 `DSI_HOST_CONFIGURATION` | 0 | `Pixel_link_format` RGB 24-bit, `shutdown` 0 | `STREAM_CSR.h:429-458`; `glue:825-828` |
| PHY CSR 0x4AD20100 `COMBO_PHY_MODE_CONTROL` | **0xF3** (Linux: `PHY_enable_ext` lanes 0-3, `PLL_clksel` 01 clock generation, `Tx_Rxz` 1 DSI) / **0x03** (Zephyr: no lane enables) | | `OR_CSI_PHY_CSR.h:622-655`; `glue:848-851`; `dsi_nxp_dwc.c:117`; U16 |
| PHY CSR 0x4AD20104 `COMBO_PHY_FREQ_CONTROL` | `Phy_cfgclkfreqrange` 0x1C, `Phy_hsfreqrange` 0x16 or 0x05: **0x0016001C** / **0x0005001C** | | `OR_CSI_PHY_CSR.h:657-669`; `glue:706-708`; `dsi_nxp_dwc.c:109-111`; U13 |
| PHY CSR 0x4AD20108 `COMBO_PHY_TEST_MODE_CONTROL` | 0x2000 (`turndisable_0`) after the PLL (Linux only) | | `OR_CSI_PHY_CSR.h:778-784`; `glue:859` |
| PHY CSR 0x4AD2010C `COMBO_PHY_TEST_MODE_STATUS` | read: `phy_lock` bit 11 | | `OR_CSI_PHY_CSR.h:934-940` |
| CSI-2 receiver 1 (0x4AD40000) test interface +0x50/+0x54, RX test code 0x302 | bit 1 (`HSRXENTERM` bypass on the shared PHY's clock lane), written and read back (Linux only) | | `glue:143-148,840-846`; `nxp:imx95.dtsi:2252-2256`; U16 |

And in the display CSR: 0x4B010000 `DSIP_CLK0_SEL` = 0 (CCM video PLL clock,
i.e. DISP1PIX; `PERI_DISPLAY_BLK_CTRL_DISPLAYMIX.h:428-438`,
`tub:imx95_dpu.c:331-334,595-596`), 0x4B010008 `PIXEL_LINK_CTRL` = 0x3
(`PL0_valid`, `PL0_enable`; `PERI_DISPLAY_BLK_CTRL_DISPLAYMIX.h:471-488`,
`dsi_nxp_dwc.c:114-116`, Linux's pixel-link bridge sets valid then enable in
its `enable`, `nxp:imx95-pixel-link.c:80-96,276-282`).

**The DW MIPI DSI host at 0x4ACF0000** (`hal:PERI_MIPI_DSI.h`; the sequence
is `ip::dw_mipi_dsi::attach`, which follows `dw-mipi-dsi.c`; hal's
`DSI_Init`, `DSI_SetDpiConfig`, `DSI_InitDphy`, `DSI_PowerUp` are the
permissive equivalent, `fsl_mipi_dsi.c:271-603`). For this mode,
`ip::dw_mipi_dsi::plan` gives:

| Field | Value | Note |
|---|---|---|
| `DPI_COLOR_CODING` | 0x5 (24 bit) | |
| `DPI_CFG_POL` | 0x6 (hsync, vsync active low) | the glue writes 0x6 whatever the mode (`glue:821`); the same here |
| `VID_MODE_CFG` | burst, low power in every blanking region | |
| `PCKHDL_CFG` | no EoTp | the bridge's `NO_EOT_PACKET` |
| `CLKMGR_CFG` escape divider | 3 (52.5 MHz / 3 = 17.5 MHz <= 20) | `esc_division(420, 20)` |
| `VID_PKT_SIZE` | 1280 | |
| `VID_HLINE_TIME`, `HSA`, `HBP` | 1053, 18, 48 lane byte clocks (burst: x 420 / 8 / 70, exact) | 1053 x 4 lanes = 4212 bytes a line against 3840 of pixels |
| `VID_VSA/VBP/VFP/VACTIVE_LINES` | 8, 24, 8, 800 | |
| PHY timing | clock LP-to-HS 65, HS-to-LP 40; data LP-to-HS 49, HS-to-LP 23 (byte clocks, the 450 Mbit/s row) | `glue:1125-1201`; the same row in `fsl_mipi_dsi.c:33-58` |
| `PHY_IF_CFG` | 4 lanes; stop wait 0x20 (Linux) / 0x28 (hal) | `fsl_mipi_dsi.c:452` |
| two-frame wait | 34 ms | |
| lock | `PHY_STATUS` bit 0, as hal polls it (`fsl_mipi_dsi.c:565-580`): `lock_in_phy_status` true | |

`probe` must accept the i.MX 95's `VERSION` (U18). The glue's `phy_ops`
map one to one onto the `Phy` trait: `get_lane_mbps` (420), `get_timing`
(the row above), `init` (master and stream CSRs, the CSI-1 termination
bypass, `COMBO_PHY_MODE_CONTROL`, `COMBO_PHY_FREQ_CONTROL`, the test-code
writes, `COMBO_PHY_TEST_MODE_CONTROL`, the clock gating; `glue:803-865`),
`power_off` (gate everything, `glue:867-885`). New code: that `Phy`
(about 250 lines with its tests) and a test-code writer.

**The SN65DSI83** at 0x2C on LPI2C3 (0x42530000, 100 kHz, pads GPIO_IO28/29,
`ml:imx95-verdin.dtsi:468-476,921-924`; `dsi.dtso:86-113`), enable
`GPIO_10_DSI`. `ip::sn65dsi8x::plan` with the i.MX8MP's `Board` (4 lanes,
24 bpp, format 2 = VESA, 200 ohm termination, no reset line) and the mode
above at 420 Mbit/s gives, and NXP's driver computes the same
(`sn65:337-370`):

| CSR | Value | |
|---|---|---|
| 0x0A | 0x05: `LVDS_CLK_RANGE` 2 (62.5-87.5 MHz), `HS_CLK_SRC` (0x85 once `PLL_EN_STAT`) | |
| 0x0B | 0x10: `DSI_CLK_DIVIDER` 3 (210 / 3 = 70 MHz LVDS clock) | |
| 0x10 | `CHA_DSI_LANES` 0 (four lanes); 0x26 in the i.MX8MP golden, our driver writes only those two bits | |
| 0x12 | 0x2A: `CHA_DSI_CLK_RANGE` 42 (210 <= f < 215 MHz) | the lane rate sits on the range's lower edge |
| 0x18 | 0x18 by our plan and the i.MX8MP's Linux dump of the same panel; **0x78** by `sn65:563-568` (HS and VS negative from the panel's `hsync-active`/`vsync-active` 0) | U20 |
| 0x20-0x3A | 1280, 800, sync delay 33, HSA 24, VSA 8, HBP 64, VBP 24, HFP 36, VFP 8 | the i.MX8MP golden holds the same bytes, `driver/src/display/golden-linux-2026-09-21.txt:934-942` |

The order is SLLSEC1I's (the bridge CSRs with the DSI clock lane in HS and no
video, then PLL, soft reset, then video), which NXP's driver keeps with
`pre_enable_prev_first` (the DSI host comes up first, `sn65:995`) and its
`pre_enable`/`enable` split (`sn65:473-670`).

**GPIOs, backlight, touch.**

| Signal | Where | What to do | Source |
|---|---|---|---|
| `GPIO_10_DSI` (Verdin 21, X4 17; Toradex calls it `DSI_1_BKL_EN`) | the module's NXP **PCAL6416A** I/O expander at **0x20** on **LPI2C2** (0x44350000, on-module `CTRL_I2C`, 400 kHz), line **3** | the bridge's EN (Toradex's `enable-gpios`) and, on the display, "display OFF when Low": low >= 10 ms, then high before the bridge's CSRs | `dsi.dtso:88-90`; `ml:imx95-verdin.dtsi:395-420`; `V95:` 1.7.9 and Table 22, page 41; `MAL:` Table 14 |
| `GPIO_9_DSI` (Verdin 17, X4 19) | expander line **2**, input | the touch interrupt (falling edge); not needed for the picture | `dsi.dtso:119-122` |
| `DSI_RESET_CON#` (Verdin 42 = `I2S_2_BCLK`, X4 22) | **GPIO5_IO06** (0x43850000; pad XSPI1_DATA6, 0x51E), active low, 1.8K pull-up on the Mallow | Toradex's overlay makes it the touch controller's reset; left undriven it is released by the pull-up | `dsi.dtso:117-124`; `ml:imx95-verdin.dtsi:875-877`; `MAL:` Table 14 |
| `PWM_3_DSI` (Verdin 19, X4 18) | **TPM6 channel 2** (0x42510000), pad GPIO_IO19 mux TPM6_CH2 (0x11E) | period 6,666,667 ns (**150 Hz**), normal polarity; brightness steps 0, 45, 63, 88, 119, 158, 203, 255 of 255, default step 4 (119, 47 %); 10K to GND on the Mallow, so an undriven pin is dark | `dsi.dtso:24-30,158-161`; `ml:imx95-verdin.dtsi:687-690,1037-1039`; `MAL:` Table 14 |
| touch | **Ilitek ILI2132 at 0x41** on LPI2C3, the bridge's bus | not part of the picture | `dsi.dtso:115-125` |

AP-NS owns LPI2C2 and LPI2C3, LPTPM6, GPIO2 and GPIO5 and their pins
(`tsm:.cfg:752,755,775-776,795,959,968-969,978-979,1023`). The expander's own
registers (output port, configuration port) are from NXP's PCAL6416A data
sheet, not read here (U19).

**The order, cold** (the i.MX8MP's order, `driver/docs/imx8mp-display-cold.md`
"The sequence", which the bridge's data sheet fixes; hal's and Zephyr's DSI
steps inside it):

| Step | What | Registers | Permissive source |
|---|---|---|---|
| 1 | SCMI: power DISPLAY (13) and CAMERA (3) on; read perf DISP (12) and CAM (11) (ODV expected); VIDEOPLL1_VCO 3,360 MHz, VIDEOPLL1 420 MHz; DISP1PIX parent VIDEOPLL1, 70 MHz; MIPIPHYCFG and MIPIPHYPLLREF parent 24M, 24 MHz; MIPIPHYPLLBYPASS parent VIDEOPLL1; enable DISP1PIX, 66, 67, 68, LPI2C2/3, TPM6 | (SCMI) | `firmware::scmi` + MU transport + CLOCK_PARENT_SET |
| 2 | Expander: `GPIO_10_DSI` output low (bridge in reset, display off) for at least 10 ms; backlight PWM at 0 | PCAL6416A over LPI2C2; TPM6 | Zephyr's `gpio_pca_series.c` (U19) |
| 3 | Display CSR: `DSIP_CLK0_SEL` 0 (CCM); interleaver control `Mode` 0 (bypass) | 0x4B010000, 0x4B010004 | `PERI_DISPLAY_BLK_CTRL_DISPLAYMIX.h` |
| 4 | Pixel interleaver channel 0, as for LVDS | `PERI_DISPLAY_INTERLEAVER.h:411-421` | |
| 5 | DPU configured, not started: one plane, LayerBlend1 over ConstFrame0, ExtDst0, DomainBlend0, FrameGen0 1280 x 800 in 1404 x 840 | `PERI_DISPLAY_SEERIS.h` | `fsl_dpu.c`; `display_socionext_dpu.c:256-327` |
| 6 | Pixel link 0: master CSR select 0, clock setting 0; stream CSR RGB 24; display CSR `PIXEL_LINK_CTRL` 0x3 | the table above | `dsi_nxp_dwc.c:112-116` |
| 7 | DW host attach (`ip::dw_mipi_dsi::attach`): `PWR_UP` 0, escape divider, DPI, packet handler, burst video config, command mode, line and vertical timing, PHY held in reset, PHY timing, `PHY_IF_CFG`; then `Phy::init`: CSI-1 termination bypass, `COMBO_PHY_MODE_CONTROL`, `COMBO_PHY_FREQ_CONTROL`, the test-code table, `COMBO_PHY_TEST_MODE_CONTROL`, clock gating 0x2; `PHY_RSTZ` 0xF, `PWR_UP` 1, wait `PHY_STATUS` lock and clock-lane stop state; two frames; command mode with the clock lane requested in HS | `PERI_MIPI_DSI.h`, the CSR tables above | `fsl_mipi_dsi.c:271-603,1243-1317`; `dsi_nxp_dwc.c:44-133` |
| 8 | Bridge: `GPIO_10_DSI` high, ID (retried, as the AM62P display's power-up needed), CSRs, PLL, `PLL_EN_STAT`, soft reset, 10 ms (`ip::sn65dsi8x::Bridge::start`) | SN65DSI83 over LPI2C3 | SLLSEC1I Table 7-2 |
| 9 | DPU started: FrameGen0 enabled, shadow loads, wait `PrimSyncStat` | | as LVDS step 5 |
| 10 | DW host to video mode (`ip::dw_mipi_dsi::video_on`) | `MODE_CFG` | |
| 11 | Bridge link check: `IRQ_STAT` (0xE5) cleared, read after 1 ms (`check_link`; `sn65:646-668`) | | |
| 12 | Backlight: TPM6 channel 2, 150 Hz, the default duty | TPM | |

Expected size: the part shared with the LVDS path (the hal_nxp register
module, the DPU one-plane path, the interleaver, the MU transport and the
SCMI calls; sized under (b') below), plus the combo D-PHY `Phy` and its
test-code writer (about 250 lines), the camera-side CSR setup (about 80),
the board layer for the bridge, the expander and the PWM (about 150), and
host tests for the PLL search and the CSR values, with both sources' values
as cases. The DW host and the bridge are already ours.

### (b') Cold: X12 LVDS from Zephyr (not this board's setup)

Kept for a board with an LVDS panel on X12; nothing below is on the owner's
path except steps shared with the DSI path (SCMI, display CSR, interleaver,
DPU).

The list, in the order U-Boot's and hal_nxp's sequences agree on
(`tub:drivers/video/nxp/imx/imx95_dpu.c:880-1006`;
`hal:mcux/mcux-sdk-ng/drivers/ldb_1/fsl_ldb.c:33-95`;
`z:drivers/display/display_socionext_dpu.c:256-327`):

| Step | Block | Registers (BSD header) | Permissive sequence | GPL cross-check (lines) |
|---|---|---|---|---|
| 1 | SCMI as agent AP-NS: DISPLAY (13) on, which runs DISPAPB, DISPAXI and DISPOCRAM (not ours to switch, section 1; perf DISP at ODV); LDBPLL_VCO (37) 2,986.2 MHz, LDBPLL (38) 497.7 MHz for 71.1 MHz, both enabled | (SCMI) | `repo:driver/rust/src/firmware/scmi.rs` codec; a new MU transport | `nxp:imx95.dtsi:2696-2703`; `tdx:...10inch-lvds.dtso:80-83`; permissions `tsm:` (section 1) |
| 2 | display CSR: engine 0 clock select = LDB PLL / 7; interleaver control | `PERI_DISPLAY_BLK_CTRL_DISPLAYMIX.h:408-415,430-468` | | `ml:clk-imx95-blk-ctl.c:226-251`; `tub:imx95_dpu.c:595-598` |
| 3 | LVDS CSR: PHY clock divider, the four clock gates (U4), LDB control (channel 0 to DI0, 24 bit, SPWG or JEIDA, sync polarities, `VSYNC_ADJ_EN`), LVDS PHY 0 enable | `PERI_LDB.h:408-412` and its field masks | `fsl_ldb.c:33-95` (96 lines) | `nxp:imx95-ldb.c` (578), `phy-fsl-imx8mp-lvds.c` (317); `tub:imx95-ldb.c` (551) |
| 4 | pixel interleaver channel 0: reset, polarities, format | `PERI_DISPLAY_INTERLEAVER.h:411-421` | | `nxp:imx95-pixel-interleaver.c:24-43,84-147` (402); `tub:` (283) |
| 5 | DPU: one plane (FetchLayer0 or FetchYUV0), LayerBlend1 over ConstFrame0, ExtDst0, DomainBlend0, FrameGen0 timing and kick, shadow loads, FrameGen enable, wait for `PrimSyncStat` | `PERI_DISPLAY_SEERIS.h` | `fsl_dpu.c` (`dpu_1`, 4,918 lines for the whole DPU; the one-plane subset Zephyr uses is about 120 lines of calls, `display_socionext_dpu.c:256-327`) | `tub:imx95_dpu.c` (1,082); `nxp:dpu95/` (about 8,400 lines without the blitter, LD and other SoCs) |
| 6 | backlight: TPM5 PWM on `PWM_2`, GPIO5_IO04 high | (TPM, GPIO) | | `tdx:...10inch-lvds.dtso:22-32` |

Not needed for X12: the pixel link (U5), the DSI host and combo PHY, the
CAMERA domain, any I2C. Not needed for a first picture: the irqsteer (poll
`FGCHSTAT`), the signature unit.

Expected size, on the AM62P and i.MX8MP precedents, shared with the DSI
path: a register module generated from hal_nxp as `regs.rs` is today (new
header list in the generator), about 600 lines of Rust for the DPU one-plane
path, about 300 for the MU transport and the SCMI calls, a board layer, and
host tests against the golden dump; LVDS only: about 200 for LDB, PHY and
interleaver. One more open choice:
vendoring `fsl_dpu.c`/`fsl_ldb.c` unmodified under `third_party/` and calling
them, as the AM62P vendors TI's Zephyr DSI driver
(`driver/docs/am62p-display-cold.md`, "Documents"), against writing the
subset in Rust from the header. The house rule (`docs/source-form.md`)
allows either; the Rust subset is smaller than the 4,918-line driver.

The same vendoring choice applies to hal_nxp's `fsl_mipi_dsi.c` (1,317
lines) on the DSI path; there the Rust host already exists, so only the
D-PHY part would be new either way.

Risks (both variants unless marked):

| Risk | Note |
|---|---|
| The System Manager refuses a request | Toradex's configuration gives AP-NS everything the path needs (section 1); a request on DISPAPB/AXI/OCRAM returns `DENIED` by design, so the code must not send one. A board with another System Manager build is checked by Z0 |
| LVDS only: LDB PLL rate: the VCO must be 6 x 7 x the pixel clock; the PLL may not reach a mode's exact rate | the Toradex panel's rate is the overlay's; other panels are later |
| DSI: the link has no margin. 420 Mbit/s is exactly 70 MHz x 24 / 4; the i.MX8MP's Samsung DSIM broke the same panel's picture with no margin (issue #8) | the DW host in burst mode sends each line's 3840 bytes inside a 4212-byte line time (1053 byte clocks), so 372 byte times a line are left for the packet overhead and the LP-HS turnarounds (49 + 23 byte clocks); Toradex's BSP runs exactly this. If the picture breaks: raise the lane rate to a point the PLL makes exactly (e.g. Fout 216 MHz, M 72, N 4: 432 Mbit/s, 2.9 % margin); the line time follows from the rate, and the bridge's divider stays 3 (LVDS 72 MHz over a 70 MHz pixel clock, as the i.MX8MP runs 71.1 over 69.3); U17 |
| DSI: NXP's two sources disagree on four D-PHY values (hsfreqrange, `vco_cntrl`, `sr_osc_freq_target`, the PLL power-on override) and on the lane enables and the CSI-side termination bypass | start from Linux's values (the ones Toradex's BSP runs on this display) and keep hal's as the fallback; the M4 reads decide what can be read (U13, U16), a cold run the rest (U14, U15) |
| DSI: the bridge's EN sits behind the module's I/O expander on another I2C bus | two I2C buses and an expander before the first DSI byte; a wrong expander write can touch the module's other lines (the line names, among them USB recovery and the TPM on lines 10-13, `ml:imx95-verdin.dtsi:416-432`): write only line 3's bits, read-modify-write |
| A full mode change wedges the ExtDst shadow load unless the FrameGen is disabled and re-enabled (the upstream port's open defect) | set the mode once, at start; flips change only the fetch address |
| Reading DPU registers with DISPLAY off | U6; the probe reads the domain state first |

### (c) No display at first

The GPU work does not wait for a display. As on the NanoPC-T6 before its
display existed, `gl_test`, dEQP and glmark2 render into off-screen targets
and check by reading back (`driver/docs/nanopc-t6-mali.md`: pixel read-back
on every case). What it gives for sign-off: every GL result and every number
except "a picture on the panel". What it does not: the console screen, the
scan-out view, and the display item of the product form. Size: nothing new
for the display. It is not a route to a picture; it is what runs while the
display is open.

### Proof without a camera

From the BSD header, all readable by the image: `FGENABLE` and `FGENSTS`
(the FrameGen runs), `FGCHSTAT.PrimSyncStat` (the primary channel is in
sync), `FGTIMESTAMP` read twice (frames advance), FetchLayer0's address,
stride and size read back equal to our buffer, the control words equal to
the golden dump. On the DSI path also: `COMBO_PHY_TEST_MODE_STATUS.phy_lock`
and the DW host's `PHY_STATUS` (lock, lanes in stop state or HS), its
`INT_ST0/1` and `VID_PKT_STATUS` (no FIFO errors), and the bridge's
`PLL_EN_STAT` and `IRQ_STAT` (0xE5: no SoT, ECC, CRC or sync error once
cleared), as the i.MX8MP's health word reads them. The bridge can also draw
its own test pattern (`CHA_TEST_PATTERN`, 0x3C bit 4, with the vertical
fields that only the pattern uses), which separates "bridge and panel work"
from "the DSI stream is right" when a person looks at the panel. Beyond what
the other boards had: **Signature0**
gives per-window CRCs of what the display engine actually sends; a CRC that
is stable while the buffer is unchanged, changes when one pixel changes, and
equals the CRC measured under Linux for the same pattern proves the content
without a camera (U7). A person at the panel confirms; the farm has no
capture device for this panel.

## 5. Recommendation

1. **(c) from day one**: the GPU bring-up and its sign-off do not depend on
   the display.
2. **The day-one reads (section 6)**, which say whether the board's Linux
   drives the X4 display and read the values NXP's two sources disagree on.
3. **(a) by crash kexec, if M1-M2 show Toradex's kernel driving the DSI
   display**: a few days, and it yields the golden dump (DPU, CSRs, DW host,
   bridge) and a working reference link. Skip it otherwise; it is not worth
   building a Linux setup for. The M4 reads alone already give most of the
   dump.
4. **(b) cold on X4** from hal_nxp's headers and drivers, with
   `ip::dw_mipi_dsi`, a new combo D-PHY `Phy` and `ip::sn65dsi8x`, checked
   against the dump: the product form, signed off with U-Boot starting the
   image.

Why DSI on X4: it is the owner's display (2026-11-21); X12 is empty and X14
is dead on this module. The DSI path costs more than LVDS would have (a
second power domain, the combo D-PHY and its PLL, the bridge and the expander
over two I2C buses), but the DW host and the bridge driver are ours already
and the i.MX8MP runs the same panel through the same bridge family. Why cold
rather than a U-Boot that lights the panel: Toradex's U-Boot does not, and
carrying our own U-Boot build for it is the cost the T6 note refused.
Toradex's System Manager configuration (section 1) leaves the order as it
was: our agent holds every domain, clock and block the X4 path needs
(DISPLAY and CAMERA, the video PLL, DISP1PIX, the MIPI PHY clocks, MIPI_DSI,
MIPI_PHY, LPI2C2/3, LPTPM6, GPIO5 and their pins), and nothing between Linux
and our image (a crash kexec) or between U-Boot and our image (`go`) turns
the display off.

What would change it:

- **An LVDS panel on X12 instead** (not the owner's setup): (b') without the
  bridge, the expander, the D-PHY and the CAMERA domain.
- **Toradex's 7" DSI display instead of the 10.1"**: the same path with its
  mode (1024 x 600 at 60 MHz, 2 lanes, `tdx:...7inch-dsi.dtso:39-53,100`):
  720 Mbit/s a lane, a new PLL point and timing row.
- **An HDMI monitor is required**: the Mallow cannot give it with this
  module. It needs a DSI-to-HDMI device on X4 that the Mallow's FFC takes
  (Toradex's own adapter has the LSS-130 connector of the Development Board
  and Dahlia), or another carrier; then new bridge code (LT8912B or the
  device's own).
- **The board runs mainline Linux or no Linux**: no adopt; (b) without a
  golden dump, from the header and the permissive drivers alone, with the
  signature unit and the FrameGen status as the proof.
- **The board's System Manager is not Toradex's `main` configuration**
  (another build, or a configuration that gives the display to the M7):
  Z0 shows it; then the permissions are a firmware question for the owner.
  As read (section 1), they change nothing in this order.

## 6. Board checks for day one (reads only)

Under the board's Linux, over ssh, nothing written:

| Check | What is read | What it gives |
|---|---|---|
| M1 | `/proc/device-tree/model` and `compatible` (expected "Toradex Verdin iMX95 ... on Mallow Board"); `uname -r`; `/etc/os-release`; the applied overlays (Toradex's `overlays.txt`) | the carrier; whether the kernel is Toradex's BSP (has `dpu95`) or mainline (has no display) |
| M2 | `/sys/class/drm/*/status` and `modes` (expected 1280x800); `dmesg` lines of `dpu95`, `imx95-mipi-dsi` ("Using Pixel Link0"), `sn65dsi83`, `panel`; `i2cdetect -r -y` on the `I2C_2_DSI` bus (LPI2C3): 0x2C (SN65DSI83) and 0x41 (ILI2132 touch); `i2cdump -y -r 0x00-0xff <bus> 0x2c b` (reads only) | **does Linux drive the X4 display?**, and the bridge's registers (U20) |
| M3 | `dmesg` SCMI and System Manager lines (protocol versions, firmware build); `/sys/kernel/debug/clk/clk_summary` rows for `dispapb`, `dispaxi`, `dispocram`, `disp1pix`, `videopll1`, `videopll1_vco`, `mipiphycfg`, `mipiphypllref`, `mipiphypllbypass`, `camapb`; `/sys/kernel/debug/pm_genpd/pm_genpd_summary` for display and camera; `/proc/config.gz` for `KEXEC`, `CRASH_DUMP`; `/proc/iomem` for a crash reservation | what the System Manager lets the AP set; whether (a) can run |
| M4 | only if M2 shows a running panel and M3 the display domain on: one 32-bit read each of DPU `COMCTRL_IPIDENTIFIER` (0x4B400000); FetchLayer0 0x4B5D0008-0x4B5D004C and FetchYUV0's first block; ExtDst0 0x4B510008-0x4B510014; FrameGen0 0x4B6B0008-0x4B6B00A0; Signature0 0x4B6D0008-0x4B6D0028 and 0x4B6D0400-0x4B6D0408; display CSR 0x4B010000-0x4B010020; interleaver 0x4B0D0000, +0x20, +0x50; with CAMERA on: DSI stream CSR 0x4AD00000-0x4AD0000C, DSI master CSR 0x4AD10000-0x4AD10008, combo PHY CSR 0x4AD20100-0x4AD20110 (U13, U16), DW host 0x4ACF0000-0x4ACF0110 (`VERSION` for U18, the timing and PHY words) | the golden dump, with an allow list as the T6's and X2's (`driver/docs/nanopc-t6-display-allow.txt`) |

M1 to M4 are one script, `tools/probes/verdin_imx95/linux_day_one/day_one.sh`
(its README; M4's read list and gates are the `#@read` lines of
`driver/docs/imx95-display-allow.txt`).

Under U-Boot, at its prompt: `dm tree` (no `video` device expected),
`bdinfo` (DRAM banks, where a buffer may live). No register reads: U-Boot
has not powered the display domain.

Under Zephyr (the first probe image, later):

| Check | What the image does | Answers |
|---|---|---|
| Z0 | SCMI base discovery (agent id, expected 2 "AP-NS"; protocols); power state of domains 13 and 3; the perf DISP and CAM levels (expected 3, ODV); rate and state of clocks 75, 76, 78, 80, 18, 19, 62, 66, 67, 68; `CLOCK_GET_PERMISSIONS` on 18, 19, 66-68, 80 (expected all) and 75 (expected none) | does our image reach the System Manager, and is the display domain on? |
| Z1 | only if Z0 says DISPLAY is on: FrameGen0/1 enable and status, the fetch unit's address, stride and size | after a crash kexec: **a scanout to adopt?** |
| Z2 | the cold X4 path of section 4(b), step by step (a stop-after per step, as the i.MX8MP's `STOP_AFTER`), with the read-backs of "Proof without a camera"; the bridge's test pattern first, then our stream | **our own picture** |

Z0 is the probe `tools/probes/verdin_imx95/imx95_scmi` (read only, all
three domains and the clocks the port uses; `check.sh` compares it with
Toradex's configuration); the DSI side's identity before Z2 (U13, U16, U18,
U19, U21) is `imx95_dsi_ids`.

Which result picks which route:

| M2: the X4 display driven by Linux | M3: kexec and crash reservation | Route |
|---|---|---|
| yes | yes | (a) by crash kexec for the dump and the reference, then (b) |
| yes | no | the dump from M4, then (b) directly |
| no (mainline, or no overlay) | | (b) from the sources alone, Linux's values first |

## 7. Unknowns

| # | Question | What answers it |
|---|---|---|
| U1 | Answered by the owner (2026-11-21): the Toradex Capacitive Touch Display 10.1" DSI on X4; no LVDS panel on X12 | (M2 confirms it under Linux) |
| U2 | Answered from `tsm:` (section 1): AP-NS (agent 2) holds DISPLAY, CAMERA, perf DISP and CAM, DISP1PIX, the LDB and video PLLs and the MIPI PHY clocks with ALL, and owns the DPU, LVDS, DSI and the pins; DISPAPB/AXI/OCRAM are the System Manager's; only a reset of the AP machine clears the votes, so a `go` or a kexec keeps them. Still open: that the board runs this configuration | Z0 (agent id, permissions, domain state); the System Manager's build line in Linux's `dmesg` (M3) |
| U3 | Which Linux does the board run: Toradex's BSP (with `dpu95` and the 10.1" DSI overlay) or mainline? | M1 |
| U4 | LVDS only, not on this board's path. The LVDS CSR word at +0x0: Linux's clock driver has the PHY divider in bit 0 and four set-to-disable gates in bits 1-4 (`ml:clk-imx95-blk-ctl.c:163-218`); hal_nxp's `PERI_LDB.h` names only the divider bit. Which is right? | M4 value under a running panel |
| U5 | LVDS only (on the DSI path the pixel link is used). Is the pixel link part of the LVDS path? Toradex's LVDS overlay enables it; NXP's Liu Ying says it is DSI only | M4 read of display CSR + 0x8 under a running LVDS panel; a cold run without it |
| U6 | What does a read of a DPU register do with the DISPLAY domain off: a bus error, a stall, zero? | do not try it; Z0 reads the domain first |
| U7 | Is Signature0's CRC usable as a proof: stable over frames, sensitive to one pixel, equal to the value under Linux for the same pattern? Its CRC polynomial is not stated in the header | a board run of the signature unit in Z2, and the same windows under Linux |
| U8 | Does Toradex's kernel support kexec with a crash reservation on the i.MX 95 (as Armbian's does on the T6)? | M3 |
| U9 | If a video U-Boot were ever used: does `go` or `booti` leave the FrameGen running, or does pre-boot device removal run the driver's `remove`? | not on the recommended path; a U-Boot run only if (a) through U-Boot is ever wanted |
| U10 | For the X4 display, answered: the touch (ILI2132, 0x41) is on LPI2C3 with the bridge, X4's only I2C (`dsi.dtso:115-125`, `MAL:` Table 14). The open question was X12/X13's (LPI2C4 by the Mallow, LPI2C3 by Toradex's LVDS overlay) and no longer matters | M2 `i2cdetect` shows 0x41 |
| U11 | Does the IMX95RM, with an NXP account, have the DPU chapter, and is it needed now that hal_nxp's header exists? | someone with an account reads its table of contents; until then the header is the documentation |
| U12 | hal_nxp's `DPU_SetFetchUnitSrcBufferAddr` takes a 32-bit address (`fsl_dpu.c:2518`); the hardware has an MSB register. Is the A55's frame buffer below 4 GiB on this board? | the image's memory map (DRAM starts at 0x80000000; its first 16 MiB are reserved for the M7, `ml:imx95-verdin.dtsi:162-165`); `bdinfo` |
| U13 | `Phy_hsfreqrange` for 420 Mbit/s: 0x16 (Linux and U-Boot, `glue:262-263`) or 0x05 (hal and Zephyr, `fsl_mipi_dsi.c:1078`)? The two tables put the 420 Mbit/s point on opposite sides of a range edge | M4: `COMBO_PHY_FREQ_CONTROL` (0x4AD20104) under Linux, a plain CSR read |
| U14 | `vco_cntrl` 0x1F (Linux) or 0x19 (hal): the 210 MHz point lies in both bands of the databook table both copy | not a plain read (PLL_30 is behind the test interface, whose read needs TESTCLK/TESTEN writes): a cold run with each, PHY lock and the bridge's error status as the check |
| U15 | `sr_osc_freq_target` 0x7D0 (Linux, any rate) or 0x384 (hal, at this rate), and PLL_17 with or without the power-on override bits (Linux 0xCD, hal 0x0D) | as U14 |
| U16 | `COMBO_PHY_MODE_CONTROL` with the four lane enables (0xF3, Linux) or without (0x03, Zephyr), `COMBO_PHY_TEST_MODE_CONTROL.turndisable_0`, and the CSI-1 clock-lane termination bypass (Linux only): which are needed for TX? | M4 reads 0x4AD20100 and 0x4AD20108 under Linux; the CSI-1 test register is behind its test interface (as U14) |
| U17 | Does the zero-margin link (420 Mbit/s for 70 MHz x 24 / 4) give a stable picture through the DW host, where the i.MX8MP's DSIM needed 2.6 %? | M2 (Linux's picture is stable, a person looks); Z2's error counters over a minute |
| U18 | The i.MX 95 DW host's `VERSION` (`ip::dw_mipi_dsi::probe` accepts 1.30, 1.31, 1.41); hal writes `PHY_TMR_RD_CFG`, so 1.31 or later | M4 read of 0x4ACF0000 |
| U19 | Answered by the build at the workspace pin (zephyr af17c0c8aa2a): the A55's `nxp_mimx95_a55.dtsi` has LPI2C1-8 (`nxp,lpi2c`, clocks from the SCMI clock protocol) and the SCMI pin control (`nxp,imx95-pinctrl`), and `i2c_mcux_lpi2c.c`, `gpio_pca_series.c` (`nxp,pcal6416`, whose part table is the expander's register map) and `pinctrl_imx_scmi.c` build into the Verdin's image. Two gaps, handled in the board glue: the dtsi has no TPM node (the backlight is `ip::tpm_pwm`, only its pad is Zephyr's), and the LPI2C driver neither enables its SCMI clock nor waits for one, so the buses and the expander are `zephyr,deferred-init` and the first show turns LPI2C2/3 on over SCMI before it initialises them. Still open: that the SM leaves those clocks to AP-NS as read (`tsm:`), and the expander's answer on the board | Z2 (the first show's lines) |
| U20 | Bridge `LVDS_FMT` (0x18): 0x18 (our plan; the i.MX8MP's Linux dump of the same panel) or 0x78 (NXP's driver from the panel's active-low syncs)? An LVDS panel with DE likely ignores it | M2 `i2cdump` of 0x2C |
| U21 | The display's power-up after `GPIO_10_DSI` rises: how long until the bridge answers on I2C (the AM62P's adapter needed about 50 ms, `ip::sn65dsi8x` retries for a second) | Z2's ID retry count |
