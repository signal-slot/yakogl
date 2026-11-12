<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# NanoPC-T6 (RK3588): the picture on HDMI from Zephyr

Status: design. No RK3588 display code exists yet. The T6 leaves "work in
progress" only when Zephyr and YakoGL put the rendered picture on a display
in the product form: U-Boot starts Zephyr from the eMMC, with no Linux before
it. This page is the plan the implementation follows: the chain, the routes,
the mode, the hand-over from the GPU, the proofs, the dump the lead takes and
the steps.

Sources, as in `driver/docs/nanopc-t6-mali.md`:

- `linux:` Linux v6.18. Every file used here is GPL-2.0 only
  (`drivers/gpu/drm/rockchip/rockchip_drm_vop2.[ch]`, `rockchip_vop2_reg.c`,
  `dw_hdmi_qp-rockchip.c`, `drivers/gpu/drm/bridge/synopsys/dw-hdmi-qp.[ch]`,
  `drivers/phy/rockchip/phy-rockchip-samsung-hdptx.c`, `drivers/clk/rockchip/`,
  `drivers/pmdomain/rockchip/pm-domains.c`, `drivers/iommu/rockchip-iommu.c`,
  the device trees). Facts and file:line only (`docs/source-form.md`, group
  E): an address, a bit, a value, an order. No code and no comment is copied.
  There is no public RK3588 reference manual we use; where Linux is silent the
  value is measured on the board (the dump below) or listed under "Unknowns".
- `uboot:` mainline U-Boot v2026.07, v2026.10, v2027.01-rc2 and its `main`
  branch of 2026-11-11.
- `board:` the T6 under Armbian (Linux 6.18), read only.

## 1. The chain

```
 DDR (below 4 GiB, linear XRGB8888)
  |  AXI0
  v
 VOP2 0xfdd90000 -- Esmart0 window --> overlay --> VP0 --(DSP_IF: HDMI0 mux = VP0)--+
  (PD_VOP; vop_mmu 0xfdd97e00/0xfdd97f00 in bypass)                                 |
                                                                                    v
 HDPTX PHY0 0xfed60000 (no power domain) <--TMDS data-- DW HDMI QP TX0 0xfde80000 (PD_VO1)
  ROPLL = TMDS clock, also "clk_hdmiphy_pixel0" --> DCLK_VOP0 mux --> VP0 pixel clock
  |
  v
 HDMI connector "hdmi0-con" (type A)
```

The T6 has two HDMI outputs. Its device tree links VP0 to HDMI0 and VP1 to
HDMI1; VP2 and VP3 are not linked (linux:`rk3588-nanopc-t6.dtsi:1189-1201`,
`:358-362`, `:378-382`; connectors `:44-64`). The design uses **HDMI0 on VP0**
only. HDMI1 is the same code with the second instance's numbers, listed for
completeness.

### VOP2 (video output processor)

| Fact | Value | Source |
|---|---|---|
| Base, size | 0xfdd90000, 0x4200 ("vop"); gamma LUT RAM 0xfdd95000, 0x1000 | linux:`rk3588-base.dtsi:1369-1372` |
| IRQ | SPI 156, shared with vop_mmu | `rk3588-base.dtsi:1373`, `:1430` |
| Version | `VERSION_INFO` (0x004) reads VOP2_VERSION(0x40, 0x17, 0x6786) | `rockchip_drm_vop2.h:20`, `rockchip_drm_vop2.c:822` |
| Clocks | ACLK_VOP, HCLK_VOP, PCLK_VOP_ROOT, DCLK_VOP0..3; the CRU sets ACLK_VOP to 500 MHz | `rk3588-base.dtsi:1374-1389`, `:752`, `:762` |
| Power domain | PD_VOP (24) | `rk3588-base.dtsi:1391`; `rk3588-power.h:46` |
| Resets | none in the VOP node; the CRU has SRST_A_VOP (SOFTRST_CON 52 bit 9), SRST_H_VOP (52/8), SRST_D_VOP0 (52/13) | `rst-rk3588.c:533-535` |
| GRFs | sys_grf 0xfd58c000, vop_grf 0xfd5a4000, vo1_grf 0xfd5a8000 | `rk3588-base.dtsi:582-612`, `:1392-1395` |
| Video ports | VP0..VP3 at 0xC00, 0xD00, 0xE00, 0xF00; VP0-2 up to 4096x2304, VP3 2048x1536 | `rockchip_vop2_reg.c:1057-1091` |
| Windows | Cluster0-3 at 0x1000/0x1200/0x1400/0x1600; Esmart0-3 at 0x1800/0x1A00/0x1C00/0x1E00 | `rockchip_drm_vop2.h:533-542` |

Which window shows a plain RGB framebuffer: every Esmart window takes
XRGB8888, ARGB8888, XBGR8888, ABGR8888, RGB888, BGR888, RGB565, BGR565 and
the YUV formats, **linear only** (`rockchip_vop2_reg.c:106-128`,
`:1195-1267`: `formats_esmart`, any VP). Cluster windows are AFBC-capable;
on RK3588, unlike RK3568, they also accept linear
(`rockchip_drm_vop2.c:384-420`: the "AFBC only" refusal is for RK3568).
The design uses **Esmart0** (AXI bus 0, read ids 0x0a/0x0b,
`rockchip_vop2_reg.c:1199-1215`): linear, no AFBC decoder, no scaler needed.

Registers the design writes, all 32-bit (offsets from the VOP base):

- System: `REG_CFG_DONE` 0x000 (bit 15 global enable, bit n + write-enable
  bit n+16 per VP: `rockchip_drm_vop2.h:856-864`), `SYS_AUTO_GATING_CTRL`
  0x008 (bit 31 cleared by Linux, `rockchip_drm_vop2.c:849-850`),
  `DSP_IF_EN` 0x028 (HDMI0 bit 3, HDMI0 mux bits 17:16 = VP id),
  `DSP_IF_CTRL` 0x02c (HDMI0 DCLK_DIV 17:16, PCLK_DIV 18), `DSP_IF_POL`
  0x030, `SYS_PD_CTRL` 0x034 (VOP-internal power: cluster n bit n, all
  Esmarts bit 7, 0 = on; `rockchip_drm_vop2.h:69-75`, `:414`;
  `rockchip_drm_vop2.c:788-797`), interrupt blocks 0x80-0xAC
  (`rockchip_drm_vop2.h:422-432`).
- Overlay: `OVL_CTRL` 0x600, `OVL_LAYER_SEL` 0x604 (4 bits per layer, Esmart0's
  layer_sel_id is 2), `OVL_PORT_SEL` 0x608 (per-window VP select at 31:16,
  per-port layer count 15:0; PORT3_MUX fixed 7 on RK3588), `VP_BG_MIX_CTRL`
  0x6E0 + 4·vp (`rockchip_drm_vop2.h:513-528`, `:735-760`;
  `rockchip_vop2_reg.c:2098-2271`).
- Video port (VP0 base 0xC00): `DSP_CTRL` +0x00 (bit 31 STANDBY, OUT_MODE
  3:0), `MIPI_CTRL` +0x04, `CLK_CTRL` +0x0C (DCLK_CORE_DIV 1:0, DCLK_OUT_DIV
  3:2), `DSP_BG` +0x2C, `DSP_HTOTAL_HS_END` +0x48, `DSP_HACT_ST_END` +0x4C,
  `DSP_VTOTAL_VS_END` +0x50, `DSP_VACT_ST_END` +0x54
  (`rockchip_drm_vop2.h:484-510`, `:640-659`); the values as
  `vop2_crtc_atomic_enable` forms them (`rockchip_drm_vop2.c:1604-1787`:
  htotal<<16 | hsync_len; hact_st = htotal - hsync_start; and so on).
- Esmart window (offsets from 0x1800): `CTRL0` 0x00, `CTRL1` 0x04 (AXI read
  ids 8:4 and 16:12), `AXI_CTRL` 0x08 (bus id bit 1), `REGION0_CTRL` 0x10
  (enable bit 0, format 5:1, R/B swap bit 14), `REGION0_YRGB_MST` 0x14 (32-bit
  address), `REGION0_VIR` 0x1C (stride in 32-bit words, 15:0),
  `REGION0_ACT_INFO` 0x20 and `DSP_INFO` 0x24 ((h-1)<<16 | (w-1)),
  `REGION0_DSP_ST` 0x28 (`rockchip_drm_vop2.h:563-584`;
  `rockchip_vop2_reg.c:351-373`; `rockchip_drm_vop2.c:1254-1370`). Format
  code 0 is ARGB8888 (`rockchip_drm_vop2.c:72`, `:183`).

Linux's order to light a VP (`rockchip_drm_vop2.c:799-860`, `:1604-1787`):
domain and bus clocks on; read the version; clear `SYS_PD_CTRL` bits 0-3 and
7; save layer/port select; `REG_CFG_DONE` global enable; auto gating off; bus
error interrupts; then per VP: interface mux and dividers (and the two GRF
writes below), timings, pixel clock parent and rate, `REG_CFG_DONE` for the
VP, and last `DSP_CTRL` with STANDBY clear. Window registers take effect at
the next frame after `REG_CFG_DONE`. Turning a VP off is `DSP_CTRL` =
STANDBY, which takes effect at the end of the frame and raises
DSP_HOLD_VALID (VP interrupt bit 6); the bus clocks must not stop before
that (`rockchip_drm_vop2.c:950-969`).

Interface dividers for HDMI (`rockchip_vop2_reg.c:1548-1641`): the VP runs
four pixels per core cycle; for RGB HDMI the dclk is the pixel clock,
DCLK_CORE_DIV = log2(4) = 2, DCLK_OUT_DIV 0, the HDMI0 PCLK_DIV = log2(2) = 1
and DCLK_DIV = log2(4) = 2.

The IOMMU: `vop_mmu` at 0xfdd97e00 and 0xfdd97f00 (two register sets, PD_VOP,
`rk3588-base.dtsi:1428-1437`). `STATUS` +0x04 bit 0 = paging enabled,
`COMMAND` +0x08 (0 enable paging, 1 disable, 6 force reset),
`DTE_ADDR` +0x00 (`rockchip-iommu.c:33-66`). Zephyr does not use it: with
paging off the VOP fetches physical addresses, so the 32-bit `YRGB_MST` limits
the frame buffer to the low 4 GiB.

### DW HDMI QP TX0 (HDMI 2.1 controller)

| Fact | Value | Source |
|---|---|---|
| Base, size | 0xfde80000, 0x20000 (TX1 0xfdea0000) | `rk3588-base.dtsi:1606-1607`, `rk3588-extra.dtsi:243` |
| IRQs | SPI 169 avp, 170 cec, 171 earc, 172 main, 360 hpd | `rk3588-base.dtsi:1615-1620` |
| Clocks | PCLK_HDMITX0 (GATE 60 b11), CLK_HDMITX0_EARC (GATE 60 b15), CLK_HDMITX0_REF (GATE 61 b0, from aclk_hdcp1_root), CLK_HDMIHDP0 (GATE 73 b12, xin24m), HCLK_VO1 (GATE 59 b9), MCLK_I2S5_8CH_TX (audio, not needed) | `rk3588-base.dtsi:1608-1614`; `clk-rk3588.c:1962-1967`, `:2075`, `:2431` |
| Power domain | PD_VO1 (26) | `rk3588-base.dtsi:1626`; `rk3588-power.h:48` |
| Resets | SRST_HDMITX0_REF (SOFTRST 61 b0), SRST_HDMIHDP0 (73 b12), SRST_P_HDMITX0 (60 b11); no Linux driver pulses them | `rst-rk3588.c:596-599`, `:719` |
| Pins | hdmi0 CEC GPIO4_C1, HPD GPIO1_A5, SCL GPIO4_B7, SDA GPIO4_C0, function 5 | `rk3588-base-pinctrl.dtsi:564-588` |
| Register width | 32-bit, stride 4, last register `EARCRX_1_INT_FORCE` 0x482c | `dw-hdmi-qp.c:1006-1011` |

The controller takes the timing from the VOP; Linux programs no timing in it.
What Linux writes (`dw-hdmi-qp.c:1013-1032`, `:834-866`):

- at bind: `MAINUNIT_0_INT_MASK_N` 0x3014 = 0, `MAINUNIT_1_INT_MASK_N`
  0x3024 = 0; `TIMER_BASE_CONFIG0` 0x80 = 428571429 (the reference clock in
  Hz); the I2C master reset (`I2CM_CONTROL0` 0xec = 1), its SCL timing
  (`I2CM_FM_SCL_CONFIG0` 0xe4 = 0x085c085c), fast mode off; clear the I2C
  done/NACK interrupts (0x3028);
- at enable: the PHY power-on (below), then `HDCP2LOGIC_CONFIG0` 0x8e0 bit 0
  (HDCP2 bypass) set and `LINK_CONFIG0` 0x968 bit 4 (OPMODE_DVI) set for a
  DVI sink or cleared for HDMI (`dw-hdmi-qp.h:156`, `:177-179`); for HDMI
  also the AVI InfoFrame (`PKT_AVI_CONTENTS*`, `PKTSCHED_PKT_EN` 0xaa8
  bits 13 and 3, `dw-hdmi-qp.c:720-755`).

The RK3588 glue (`dw_hdmi_qp-rockchip.c`), all GRF writes with the
write-enable mask in bits 31:16:

1. `vo1_grf` CON3 0x0c (port 0) or CON6 0x18 (port 1): bit 9 SCLIN, bit 10
   SDAIN, bit 11 MODE, bit 13 I2S_SEL set (`:349-376`).
2. `sys_grf` SOC_CON7 0x31c: bit 12 HPD_HDMI0_IO_EN, bit 13 HPD_HDMI1_IO_EN.
3. `vo1_grf` CON9 0x24: bit 10 HDMI0_GRANT_SEL (bit 12 for port 1).
4. `sys_grf` SOC_CON2 0x308: the HPD interrupt mask (bit 13 HDMI0, bit 15
   HDMI1) and clear (bit 12, bit 14) (`:65-68`, `:183-196`).
5. The HPD level: `sys_grf` SOC_STATUS1 0x384 bit 16 (HDMI0), bit 24 (HDMI1)
   (`:171-181`).
6. At enable the TMDS rate (pixel clock x bpc/8, RGB) is handed to the PHY
   (`:114-135`). An optional "enable" GPIO (FRL/TMDS switch) is set high; the
   T6 tree has none (`rk3588-nanopc-t6.dtsi`, the hdmi0 node).

VOP side GRF writes for HDMI0 (`rockchip_vop2_reg.c:1690-1702`): `vop_grf`
`GRF_VOP_CON2` 0x08 bit 1 = 1 (bit 4 for HDMI1); `vo1_grf` `GRF_VO1_CON0`
0x00 bits 6:5 = sync polarity (bits 8:7 for HDMI1), from the mode's
NHSYNC/NVSYNC (`:1660-1668`).

### HDPTX PHY0 (Samsung HDMI/DP combo PHY)

| Fact | Value | Source |
|---|---|---|
| Base, size | 0xfed60000, 0x2000 (PHY1 0xfed70000) | `rk3588-base.dtsi:3118-3120`; `phy-rockchip-samsung-hdptx.c:2086-2092` |
| Its GRF | hdptxphy0_grf 0xfd5e0000 (PHY1 0xfd5e4000): CON0 0x00 (PLL_EN bit 7, BIAS_EN 6, BGR_EN 5, MODE_SEL 0: 0 HDMI, 1 DP), STATUS 0x80 (PLL_LOCK_DONE 3, PHY_CLK_RDY 2, PHY_RDY 1, SB_RDY 0) | `rk3588-base.dtsi:724-727`; `hdptx.c:24-33` |
| Clocks | "ref" CLK_USB2PHY_HDPTXRXPHY_REF (PMU1 CRU, CLKSEL 14, GATE 4 b7, xin24m, critical), "apb" PCLK_HDPTX0 (GATE 72 b5) | `rk3588-base.dtsi:3121-3122`; `clk-rk3588.c:2079`, `:2278-2281` |
| Resets | SRST_HDPTX0 (SOFTRST 72 b14), SRST_P_HDPTX0 (72 b5), SRST_HDPTX0_ROPLL (72 b12), _LCPLL (72 b13); in the PMU1 CRU (0xfd7f0000) SRST_HDPTX0_INIT, _CMN, _LANE (PMU SOFTRST 3 b11, b12, b13) | `rst-rk3588.c:695`, `:702-704`, `:788-790` |
| Power domain | none | `rk3588-base.dtsi:3118-3133` |
| Register file | 32-bit words, index n at 4·n; readable/writable only in 0x0000-0x029c (CMN), 0x0400-0x04a4 (SB), 0x0800-0x08a4 (lane top), 0x0c00-0x0cb4, 0x1000-0x10b4, 0x1400-0x14b4, 0x1800-0x18b4 (lanes 0-3) | `hdptx.c:34-45`, `:776-799` |

The TMDS bring-up in Linux (`hdptx.c:972-1066`, `:804-874`, `:1416-1476`):

1. APB reset pulse; assert LANE, CMN, INIT; CON0 PLL_EN, BIAS_EN, BGR_EN = 0.
2. The common init tables (`rk_hdtpx_common_cmn_init_seq`,
   `rk_hdtpx_tmds_cmn_init_seq`, `:459-590`), then the ROPLL dividers:
   CMN 0x51 M, 0x55 M for the AFC, 0x59 P/refdiv, 0x5a S, 0x5e/0x64/0x60/0x65
   the sigma-delta (SDM) fraction, 0x69/0x6c/0x70 the SDC terms, 0x86 the
   post divider and the pixel-clock enable.
3. CON0 BIAS_EN, BGR_EN = 1; 10 us; release INIT; PLL_EN = 1; release CMN;
   poll STATUS PHY_CLK_RDY (20 us steps, 400 us bound).
4. CON0 MODE_SEL = 0 (HDMI); sideband table, lane-top 0x200 = 0x06, the
   low-rate (TMDS up to 340 MHz) lane-top table, 0x206 = 0x07, 0x207 = 0x0f,
   the lane tables (`:591-690`); release LANE; poll STATUS PHY_RDY and
   PLL_LOCK_DONE (100 us steps, 5 ms bound).

The init tables are 212 register writes of analog settings that no document
explains (`hdptx.c:459-690`). The design does not transcribe them: the
values come from the board dump of the PHY's register file with Linux
showing the chosen mode (measurement, test data, as `x2-hdmi-dump-linux.txt`
is), and the order of the steps above is the fact taken from Linux. This is
an owner decision to confirm (question U1).

The PHY's PLL output is also a clock, "clk_hdmiphy_pixel0" (`hdptx.c:1931-1958`).
`DCLK_VOP0`'s mux (CLKSEL_CON 112 bits 8:7) selects 0 dclk_vop0_src (CRU
PLLs), 1 clk_hdmiphy_pixel0, 2 clk_hdmiphy_pixel1 (`clk-rk3588.c:503-505`,
`:2100-2111`). Linux moves VP0 onto the PHY's clock for every HDMI0 mode up
to 600 MHz (`rockchip_drm_vop2.c:1740-1774`). The design does the same: the
TMDS clock and the pixel clock come from one PLL.

### Power domains, clocks, resets

PMU at 0xfd8d8000 (`rk3588-base.dtsi:842-844`; `pm-domains.c:1493-1502`,
`:1294-1296`), writes with the mask in 31:16, power off = bit 1, power on =
mask only, "on" read from repair status 0x290:

| Domain | Power reg/bit | Idle req | Idle/ack | Repair status | Clocks it needs on while switching |
|---|---|---|---|---|---|
| PD_VOP (top level) | 0x150 bit 1 | 0x10c bits 13, 14 | 0x120/0x118 bits 13, 14 | 0x290 bit 16 | PCLK_VOP_ROOT, HCLK_VOP_ROOT, ACLK_VOP |
| PD_VO0 (child of PD_VOP; not needed for HDMI) | 0x150 bit 2 | 0x10c bit 15 | bit 15 | bit 17 | |
| PD_VO1 (top level) | 0x150 bit 3 | 0x110 bit 0 | bit 16 | bit 18 | PCLK_VO1_ROOT, PCLK_VO1_S_ROOT, HCLK_VO1_S_ROOT, HCLK_HDCP1, ACLK_HDCP1, ACLK_HDMIRX_ROOT, HCLK_VO1USB_TOP_ROOT |

Power-on order (`pm-domains.c:615-660`, `:518-563`, `:376-422`): domain clocks
on; power bit; memory reset if needed; poll repair status; SIP SMC
`SUSPEND_MODE`/`SLEEP_PD_CONFIG`; clear the idle request and wait for ack and
idle; restore QoS. This is the PD_GPU sequence `soc::rk3588` already runs for
the GPU (`nanopc-t6-mali.md`, "GPU power-on"), with other bits.

CRU at 0xfd7c0000: CLKSEL_CON(x) 0x300 + 4x, CLKGATE_CON(x) 0x800 + 4x,
SOFTRST_CON(x) 0xa00 + 4x; PMU1 CRU at +0x30000; gates "1 = gated"; reset
assert = bit and its mask, release = mask only (`clk.h:354-383`;
`clk-rk3588.c:520-522`; `softrst.c:31-70`). The display gates:

| Clock | Gate | Mux / divider | Source |
|---|---|---|---|
| aclk_vop_root | GATE 52 b0 | SEL 110 [7:5] / [4:0] (gpll, cpll, aupll, npll, spll) | `clk-rk3588.c:1908` |
| hclk_vop_root | GATE 52 b2 | SEL 110 [11:10] | `:1914` |
| pclk_vop_root | GATE 52 b3 | SEL 110 [13:12] | `:1917` |
| HCLK_VOP, ACLK_VOP | GATE 52 b8, b9 | ACLK via SEL 115 b9 | `:2087-2089`, `:1926` |
| DCLK_VOP0_SRC | GATE 52 b10 | SEL 111 [8:7] / [6:0] | `:2091` |
| DCLK_VOP0 | GATE 52 b13 | SEL 112 [8:7]: 1 = PHY0 pixel clock | `:2100` |
| pclk_vo1_root, hclk_vo1_root | GATE 59 b4, b2 | SEL 129 [3:2], SEL 128 [14:13] | `:1896-1902` |
| aclk_hdcp1_root | GATE 59 b0 | SEL 128 [6:5] / [4:0] | `:1890` |
| HCLK_VO1, PCLK_VO1GRF | GATE 59 b9, b12 | | `:2431-2436` |
| PCLK_HDMITX0, CLK_HDMITX0_EARC, CLK_HDMITX0_REF | GATE 60 b11, b15; GATE 61 b0 | EARC SEL 133 | `:1962-1967` |
| CLK_HDMIHDP0 | GATE 73 b12 | xin24m | `:2075` |
| PCLK_HDPTX0 | GATE 72 b5 | | `:2079` |

## 2. Routes

### (a) Adopt from Linux after kexec (development only)

What Linux 6.18 leaves depends on the kexec route:

- **Normal kexec** (`reboot(KEXEC)`, what `nanopc-t6.sh` does today):
  `kernel_kexec` runs `device_shutdown()` (`kernel/kexec_core.c:1184-1202`,
  `kernel/reboot.c:100-105`). rockchip-drm's shutdown runs
  `drm_atomic_helper_shutdown` (`rockchip_drm_drv.c:485-503`): VP0 goes to
  STANDBY, DCLK_VOP0 goes back to its CRU parent and is gated, the VOP bus
  clocks are disabled, the IOMMU is disabled
  (`rockchip_drm_vop2.c:936-990`, `:862-873`; `rockchip-iommu.c:1298-1310`).
  The PHY PLL is stopped when its use count reaches 0 (`hdptx.c:1134-1164`,
  `:876-898`; whether a clock reference the VOP's power-domain clock list
  takes keeps it running is not settled). PD_VO1 stays on (the HDMI device has
  no runtime PM; `pmdomain/core.c:993`); PD_VOP depends on device order (not
  settled). The GRF settings stay. **Nothing scans out.** Showing a picture
  from here is most of the cold path, not an adoption.
- **Crash kexec** (`kexec_zephyr --crash`, as on the Verdin and the X2):
  `__crash_kexec` skips `device_shutdown` (`kernel/crash_core.c:120-138`;
  `arch/arm64/kernel/machine_kexec.c:213-225`). Everything stays on: PD_VOP,
  PD_VO1, the clocks, the PHY PLL locked, DCLK_VOP0 on the PHY clock, and VP0
  scanning Linux's frame buffer **through the IOMMU, whose page tables are in
  Linux's (now dead) memory**.

So adoption needs the crash route. The smallest thing Zephyr must write to
show its own buffer, in this order:

1. Read VP0 `DSP_CTRL` (STANDBY clear), Esmart0 or the window Linux used
   (the dump says which), and the mode from the VP timing registers; refuse
   if they differ from the build's mode.
2. Stop the IOMMU translating: for both `vop_mmu` register sets, `COMMAND` =
   disable paging (or force reset) and check `STATUS` bit 0 clear. Until this
   is done the VOP reads through tables Zephyr may overwrite.
3. Point the window at Zephyr's buffer: `REGION0_YRGB_MST` (physical, below
   4 GiB), `REGION0_VIR` (stride / 4), format ARGB8888 code 0 with R/B swap 0
   for XRGB8888 memory order, AFBC off if Linux used a Cluster window with
   AFBC (then switch to Esmart0: `OVL_LAYER_SEL`/`OVL_PORT_SEL` as well).
4. `REG_CFG_DONE` = bit 15 | bit 0 | bit 16; the change lands at the next
   frame.

Steps 2-4 are a handful of 32-bit writes. Doing step 3 before step 2 shows
garbage for at most one frame, never a fault, because both take effect at
the frame boundary only after `REG_CFG_DONE`; the design does 2 first anyway.

What it needs: a `crashkernel=` reservation that holds the image window
0x10000000-0x1fffffff on Armbian's command line (today the T6 has none: the
window is free System RAM and `nanopc-t6.sh` uses the normal route), and
`nanopc-t6.sh --crash`. What it proves: the VOP2 window programming, the
frame buffer format and stride, the hand-over from a buffer to the screen.
What it does not prove: power, clocks, the PHY, the controller, the mode.
A risk of its own: the crash route also leaves panthor's GPU and its firmware
running, which the Z1-Z4 power-on has never met (they all ran after a normal
kexec). The adopt probe therefore draws with the CPU first; a Mali frame on
the adopted display is a separate step.

### (b) Adopt from U-Boot

Mainline U-Boot does not light RK3588 HDMI. At v2026.07, v2026.10,
v2027.01-rc2 and `main` (2026-11-11), `drivers/video/rockchip/` has VOP1
drivers for RK3288, RK3328 and RK3399 only (uboot:`rk3288_vop.c:91`,
`rk3328_vop.c:64`, `rk3399_vop.c:84-86`; `drivers/video/rockchip/Makefile:7-20`),
no VOP2, no DW HDMI QP and no HDPTX PHY driver; `drivers/phy/rockchip/` has no
HDPTX; `configs/nanopc-t6-rk3588_defconfig` enables no video
(`:27-28` are CPU/board info only). A VOP2 series on the U-Boot list
(2025-2026) targets RK3566/RK3568 and is not merged (web, not checked against
code). Rockchip's vendor U-Boot has all three drivers (GPL); using it would
mean carrying a vendor boot loader and would still hand Zephyr a display U-Boot
may switch off before it jumps. **Route (b) does not exist for the product.**

### (c) From cold in Zephyr (the product form)

Zephyr powers PD_VOP and PD_VO1, opens the clocks, sets the GRF bits,
brings up the PHY PLL at the mode's TMDS rate, moves DCLK_VOP0 onto it,
configures the controller (DVI), programs VP0 and Esmart0, and starts the
VP. Section 7 has the steps. It is developed through the **normal** kexec
(which leaves the display off, so any picture is ours, and forces the
full sequence) with a `power-cycle` option that first puts every block into
reset and its domain off, as the X2 HDMI design does
(`x2-hdmi-cold.md`, `force-cold`, `power-cycle`); then signed off with U-Boot
starting the image from the eMMC (`driver/tools/uboot-t6/t6-zephyr.cmd`),
which is the same code with nothing left on.

### Which first, which signs off, what is shared

- First picture: **(a) by crash kexec** (two small steps), because it needs
  only the VOP2 window module and the board's display hook, and it gives the
  read-back proofs (section 5) a known-good link to be checked against.
- Sign-off: **(c) under U-Boot**. (b) does not exist.
- (c) reuses from (a): the `ip::vop2` window and `REG_CFG_DONE` code, the
  IOMMU-off code, the board's `board_show()` and its hand-over to EGL, the
  status read-back, the dump and its golden test. (a) adds only the
  "read the running mode and refuse a mismatch" policy, which (c) keeps as
  its "is the chain already running?" check.

## 3. Mode

A fixed mode first, in DVI mode (no InfoFrames), RGB 8 bit: then the TMDS
character rate equals the pixel clock and the controller needs only
`LINK_CONFIG0` and `HDCP2LOGIC_CONFIG0`. The mode is the one Linux shows on
the display the lead plugs in, read from the dump, so every PHY value is
measured on that very rate. The expected case is **1920x1080 at 60 Hz, 148.5
MHz** (CEA-861 VIC 16), which Linux's PHY table has (`hdptx.c:427`).

What the PHY PLL needs (`hdptx.c:900-970`): the PLL output is 5x the TMDS
character rate divided out of a VCO of 2.0-4.0 GHz, `VCO = 24 MHz x M`
(P = refdiv = 1), `S` even or 1, M 20-255. For 148.5 MHz: 742.5 MHz x 4 =
2970 MHz; 2970 / 24 = 123.75 is not an integer, so the PLL runs fractional
(the SDM and SDC registers). The same holds for 74.25 MHz (1280x720, S = 8,
same VCO). Rates whose VCO is a multiple of 24 MHz run integer, with SDM off:
27 MHz (720x480p60, VIC 2/3: 135 MHz x 16 = 2160 = 24 x 90; Linux's table
has SDM off for it, `hdptx.c:453`) and 25.2 MHz (640x480 at 60 Hz, VIC 1:
126 x 16 = 2016 = 24 x 84). These are the fallback if the fractional values
from the dump do not lock: every HDMI sink must accept VIC 1 (CEA-861).

What reading the EDID adds later: the sink's own preferred mode instead of a
build-time one, and HDMI vs DVI (an HDMI sink with InfoFrames). It needs the
controller's I2C master (`I2CM_*` 0xe0-0x118, `MAINUNIT_1_INT_*` done/NACK
bits, the segment pointer for E-DDC; `dw-hdmi-qp.c:511-690`) and the GRF
SCL/SDA input bits already set in step 1 of the glue. The T6's panels may
answer late: a 1920x720 HDMI panel used with this board did not answer DDC
while it woke from power saving, and Linux then fell back to VESA modes. So
EDID is read with retries and a fixed fallback mode, never as the only source.

The design also accepts the 1920x720 bar panel's mode once the dump has its
timing (its pixel clock is not known here; the PHY then needs that rate's
fractional values from the dump, or Linux's own calculator's result read
back from the registers).

## 4. How the GPU's picture reaches the screen

The swap path on every board: `eglSwapBuffers` -> `swap_window_buffers`
(`src/gl/egl.rs:2324-2340`) calls `osal::present(mem, width, height,
stride_bytes, native_format)` (`src/osal/mod.rs:576-583`) -> the Zephyr OSAL's
`yakogl_osal_zephyr_present` (`src/osal/zephyr_timing.c:123-130`) ->
`yakogl_zephyr_present` (`driver/zephyr/lib/board/common/board.c:31-40`) ->
the board's `board_show()` (AM62P `board_am62p.c:69-89`, i.MX8MP
`board_imx8mp.c:358`, Pi 4 `board_rpi4.c:299`). The T6 board glue has no
`board_show()` yet, and the Mali build has no Zephyr OSAL yet (milestone Z4);
when it comes it fills `.present` with the same function, as `zephyr.c`,
`zephyr_v3d.c` and `zephyr_gc.c` do. A `board_display_busy()` (as the AM62P's)
lets EGL keep four buffers while the VOP may still scan one.

The Mali render target for a window today (`src/mali/backend.rs:1451-1475`,
`src/mali/frame.rs:928-1004`): **XRGB8888 linear**, bytes B, G, R, X in memory
(DRM XRGB8888), row stride = width rounded up to even x 4 bytes, rounded up to
64 bytes (`TEXTURE_ROW_ALIGN`, `backend.rs:280`). 1920 wide gives 7680 bytes,
1280 gives 5120; both are multiples of 64. The frame is written top-down for a
window (`flip_y`, `frame.rs:525`, `:568`).

Can Esmart0 scan it directly: **yes, with no copy.**

- Format: DRM XRGB8888 is a format Esmart takes; it maps to VOP2 format code
  0 (ARGB8888) with R/B swap off (`rockchip_drm_vop2.c:171-213`,
  `rockchip_vop2_reg.c:106-128`).
- Stride: `VIR` is the stride in 32-bit words, 16 bits (`REGION0_VIR` 15:0),
  Linux writes pitch / 4 rounded up (`rockchip_drm_vop2.c:1358`); 7680 bytes
  is 1920 words. No 64-pixel rule applies to a linear, unrotated Esmart
  window (`:1293-1296` is for AFBC with mirror or rotation).
- Linear vs AFBC: Esmart is linear only; our Mali targets are linear. AFBC is
  not needed.
- Address: `REGION0_YRGB_MST` is 32 bits, so with the IOMMU off the buffer
  must be below 4 GiB. The image window 0x10000000-0x1fffffff is; the GPU
  buffers come from the OSAL's carve-out inside it
  (`nanopc-t6-mali.md`, memory map). Alignment: Linux places no alignment
  check on a linear Esmart address beyond the frame buffer's own; the design
  keeps buffers 64-byte aligned (the Mali target's own rule).
- Top-down vs bottom-up: Esmart can mirror vertically (`DRM_MODE_REFLECT_Y`,
  `rockchip_vop2_reg.c:1205`); the board may then advertise
  `GLES_NATIVE_WINDOW_BOTTOM_UP` and save the flipped viewport. Not needed for
  the first picture.
- Coherence: the Mali is not coherent (`nanopc-t6-mali.md`); the frame is in
  memory once its fence has signalled, and the VOP reads memory. The CPU
  pattern of route (a) needs a cache clean before `REG_CFG_DONE`.

## 5. Proof without a camera

What can be read back (every one of these is a register Linux names; values
compared against the dump):

| Proof | Register | Meaning |
|---|---|---|
| VP0 runs frames | VP0 `INT_RAW_STATUS` 0xAC bit 0 (FS, frame start); clear with `INT_CLR` 0xA4 = bit \| bit<<16, read again after more than one frame | the VP's timing generator runs on a pixel clock |
| No underflow | the same register bit 4 (POST_BUF_EMPTY) stays 0 over many frames | the window's fetch keeps up: our buffer address is readable memory |
| No bus error | `SYS0_INT_STATUS` 0x88 / `SYS1_INT_STATUS` 0x98 bit 1 | the AXI fetch completed |
| Our buffer is the one shown | Esmart0 `REGION0_YRGB_MST` reads our address after `REG_CFG_DONE`, `REG_CFG_DONE` VP bit back to 0 | the shadow registers took the update |
| VOP clock rate | the frame-start interval measured against the arch timer (24 MHz) | pixel clock = htotal x vtotal x rate |
| PHY PLL | hdptxphy0_grf STATUS 0xfd5e0080: PLL_LOCK_DONE bit 3, PHY_CLK_RDY bit 2, PHY_RDY bit 1 | the PLL locked and the lanes are ready |
| Controller clocks | `CMU_STATUS` 0xb0 & 0x3f = 0x15 (`DISPLAY_CLK_LOCKED`), `CMU_VIDQPCLK_FREQ` 0xb8, `CMU_LINKQPCLK_FREQ` 0xbc (`dw-hdmi-qp.h:38-55`) | the controller sees its video and link clocks; the FREQ registers measure them (units unknown: compared with the dump) |
| Video at the controller | `VIDEO_MONITOR_STATUS0-6` 0x884-0x89c | fields not named by Linux; compared with the dump |
| Sink present | sys_grf SOC_STATUS1 0xfd58c384 bit 16 (HPD level, HDMI0) | a cable and a powered sink (HPD only) |
| Sink answers | an EDID read over DDC (later step) | the sink's controller is alive |

Not available: Linux v6.18 names no VOP2 CRC register and no line counter
(`SYS_STATUS0` 0x060 exists, its fields are not named), and no TMDS-side
"receiver sense". So nothing on our side proves that the sink locked onto the
signal or what it shows.

Only a person looking at the display can confirm: that the sink shows a
picture rather than "no signal", the picture's content (the pattern's colours
in the right places, i.e. the byte order), its geometry (no shift, no wrap,
the full frame), and no flicker or sparkles (an analog margin of the PHY). The
proof plan asks the person for one answer per run, chosen to be unambiguous:
a pattern with four coloured quadrants and a white 1-pixel border.

## 6. The dump list (for the lead)

Preconditions: Armbian, the display plugged into **HDMI0** (check:
`readlink /sys/class/drm/card*-HDMI-A-*/device` names `fde80000.hdmi` for the
connected one, and its `status` reads `connected`), a mode set and a picture
on it (the console is enough), debugfs mounted. The reads below are only
safe while the display is on: the VOP registers need PD_VOP and its bus
clocks, the controller PD_VO1 and its clocks, the PHY its APB clock. A read of
a block whose domain is off may hang the bus. If the display has blanked, wake
it first (any key on the console) and check `status` again.

Files to copy (as is, into the dump's header or beside it):

```
uname -a; cat /proc/cmdline; date -u
cat /sys/class/drm/card*-HDMI-A-*/status /sys/class/drm/card*-HDMI-A-*/modes
cat /sys/class/drm/card*-HDMI-A-1/edid | od -An -tx1          # the sink's EDID, binary as hex
cat /sys/kernel/debug/dri/*/state                             # the mode, the plane, its fb, the CRTC
cat /sys/kernel/debug/dri/*/vop2/summary                      # VP, window, format, address
cat /sys/kernel/debug/dri/*/vop2/regs                         # Linux's own VOP2 dump (SYS, OVL, VP0-3, windows)
cat /sys/kernel/debug/clk/clk_summary | grep -iE 'vop|hdmi|hdptx|hdcp1|vo1|v0pll|gpll|cpll'
cat /sys/kernel/debug/pm_genpd/pm_genpd_summary
cat /sys/kernel/debug/regmap/*fed60000*/registers             # the PHY0 register file through Linux's readable list
dmesg | grep -iE 'hdmi|hdptx|vop|drm'
```

`vop2/regs` reads only while the VOP is enabled and is Linux's own safe
dump (`rockchip_drm_vop2.c:2008-2070`, regions from
`rockchip_vop2_reg.c:1269-1370`). The PHY's regmap file reads only the
ranges `hdptx.c:776-790` allows. The controller's regmap file must **not**
be used: it has no readable list and would read every word up to 0x482c,
including the I2C master's data registers.

Register reads with the board's `mmio_rd` (the tool `nanopc-t6.sh` already
stages; `mmio_rd ADDRESS COUNT`, 32-bit, one line per word), in this order,
stopping at the first read that does not return:

```
# 1. PMU: domain status (read only)
mmio_rd 0xfd8d8150 1; mmio_rd 0xfd8d8180 1; mmio_rd 0xfd8d8118 1; mmio_rd 0xfd8d8120 1; mmio_rd 0xfd8d8290 1
# 2. CRU: CLKSEL 110-115, 128-136; GATE 52-53, 55, 59-61, 72-73; SOFTRST 52-53, 60-61, 72-73
mmio_rd 0xfd7c04b8 6;  mmio_rd 0xfd7c0500 9
mmio_rd 0xfd7c08d0 2;  mmio_rd 0xfd7c08dc 1;  mmio_rd 0xfd7c08ec 3;  mmio_rd 0xfd7c0920 2
mmio_rd 0xfd7c0ad0 2;  mmio_rd 0xfd7c0af0 2;  mmio_rd 0xfd7c0b20 2
# 3. GRFs
mmio_rd 0xfd58c308 1; mmio_rd 0xfd58c31c 1; mmio_rd 0xfd58c384 1        # sys_grf SOC_CON2, SOC_CON7, SOC_STATUS1
mmio_rd 0xfd5a4008 1                                                    # vop_grf GRF_VOP_CON2
mmio_rd 0xfd5a8000 16                                                   # vo1_grf CON0..CON15 (0x00-0x3c)
mmio_rd 0xfd5e0000 1; mmio_rd 0xfd5e0080 1                              # hdptxphy0_grf CON0, STATUS
# 4. VOP2 (only if vop2/regs above did not cover it): SYS 0x000-0x0ff, OVL 0x600-0x6ff,
#    VP0 0xc00-0xcff, Esmart0 0x1800-0x18ff, Cluster0 0x1000-0x10ff and 0x1100 (CLUSTER_CTRL)
mmio_rd 0xfdd90000 64; mmio_rd 0xfdd90600 64; mmio_rd 0xfdd90c00 64; mmio_rd 0xfdd91800 64; mmio_rd 0xfdd91000 65
# 5. vop_mmu: DTE_ADDR, STATUS, COMMAND (do not read FAULT/INT beyond 0x24)
mmio_rd 0xfdd97e00 3; mmio_rd 0xfdd97f00 3
# 6. HDMI QP TX0: ids, global, CMU, video interface and monitor, link, HDCP2 bypass, packet scheduler, interrupt status
#    (not the request/pulse words 0x40, 0x974, 0xab0)
mmio_rd 0xfde80000 8;  mmio_rd 0xfde80044 5;  mmio_rd 0xfde80080 2;  mmio_rd 0xfde800a0 10
mmio_rd 0xfde80180 1;  mmio_rd 0xfde80800 4;  mmio_rd 0xfde80814 1;  mmio_rd 0xfde8081c 1
mmio_rd 0xfde80880 8;  mmio_rd 0xfde808e0 1;  mmio_rd 0xfde80960 1;  mmio_rd 0xfde80968 1
mmio_rd 0xfde80970 1;  mmio_rd 0xfde80a80 11; mmio_rd 0xfde80ab4 2       # PKTSCHED config, PKT_EN, status
mmio_rd 0xfde80be0 8                                                    # PKT_AVI_CONTENTS0-7
mmio_rd 0xfde83010 2;  mmio_rd 0xfde83020 2
# 7. last, optional, one line at a time: PMU1 CRU (HDPTX ref clock and resets)
mmio_rd 0xfd7f0338 1; mmio_rd 0xfd7f0810 1; mmio_rd 0xfd7f0a0c 2
```

The controller offsets are `dw-hdmi-qp.h`'s (`:13-55`, `:97-103`,
`:147-156`, `:175-214`, `:275-286`, `:759-784`). Words that request an action
(`GLOBAL_SWRESET_REQUEST` 0x40, `TMDS_FIFO_CONTROL0` 0x974,
`PKTSCHED_PKT_SEND` 0xab0) are left out: Linux only writes them, and what a
read of them does is not documented.

Must NOT be read:

- the controller's I2C master data and control (0xe0-0x118) and the whole
  regmap file: a read may advance or disturb a transfer (not documented);
- the VOP's gamma LUT RAM 0xfdd95000 and the HDR LUT region (0x2000+): LUT RAM
  accessed while the VP uses it (Linux switches `DSP_LUT_EN` off and polls
  before writing, `rockchip_drm_vop2.c:886-915`);
- anything of a block whose domain or bus clock is off (hang), so not VP1-3
  windows or the second HDMI/PHY instance unless they are on;
- the PHY register file outside the ranges of `hdptx.c:776-790`;
- the PMU's power, idle request and memory power **writes** (none above;
  reading is what Z1 already does for PD_GPU).

Interrupt status registers (VOP 0x88/0x98/0xA8/0xAC, controller 0x3010,
0x3020) are read-only status and not cleared by a read; they are sampled, so
they go in the allow list. The dump file is
`driver/docs/nanopc-t6-display-dump-linux.txt` (`ADDRESS WIDTH VALUE`, as the
X2's), and its allow list `driver/docs/nanopc-t6-display-allow.txt`
(`ADDRESS MASK REASON`, as `x2-hdmi-allow.txt`), seeded with:

```
0xfdd900a8 0xffffffff VP0_INT_STATUS: event flags, sampled at an arbitrary time
0xfdd900ac 0xffffffff VP0_INT_RAW_STATUS: likewise
0xfdd90088 0xffffffff SYS0_INT_STATUS: likewise
0xfdd90098 0xffffffff SYS1_INT_STATUS: likewise
0xfdd91814 0xffffffff Esmart0 REGION0_YRGB_MST: Linux's buffer against ours
0xfdd97e00 0xffffffff vop_mmu DTE_ADDR: Linux uses the IOMMU, Zephyr does not
0xfdd97e04 0x00000001 vop_mmu STATUS paging enabled: likewise
0xfdd97f00 0xffffffff vop_mmu (second set) DTE_ADDR: likewise
0xfdd97f04 0x00000001 vop_mmu (second set) STATUS paging enabled: likewise
0xfde800b8 0xffffffff CMU_VIDQPCLK_FREQ: a measurement, varies by a count
0xfde800bc 0xffffffff CMU_LINKQPCLK_FREQ: likewise
0xfde83010 0xffffffff MAINUNIT_0_INT_STATUS: event flags
0xfde83020 0xffffffff MAINUNIT_1_INT_STATUS: event flags
```

## 7. The step plan

Each step: what it proves, its host test, its board proof, its size. The
host tests run against a register model in the manner of `model_hdmi.rs`:
it refuses access to a block whose domain, clock or reset does not allow it,
answers polls, takes injected failures and records the order.

| Step | What | Host test | Board proof | Size | Display plugged |
|---|---|---|---|---|---|
| D0 | The dump: section 6 as a script under `driver/tools/kexec-zephyr/` (read only, like `nanopc-t6.sh`'s dry run), its output committed as the dump file and the allow file seeded | the script's dry run on the host prints the read list; the dump parses | the lead runs it (section 6) | script 100 lines | yes |
| D1 | `ip::vop2`: register map, Esmart window (format, address, stride, size, enable, AXI ids), overlay select, VP timing and dividers, `REG_CFG_DONE`, internal PD (`SYS_PD_CTRL`), STANDBY with DSP_HOLD wait, status read-back; `ip::rk_iommu`: paging off and check | layout = section 1; timing words for the dump's mode; a window refused above 4 GiB, unaligned, or with a stride over 16 bits; CFG_DONE after the window writes; standby waits for DSP_HOLD | none alone | 700 lines + tests | no |
| D2 | Adopt (route a): `nanopc-t6.sh --crash` and the T6 crash reservation; a `t6_display` probe that adopts VP0, turns the IOMMU off, shows a CPU pattern in Esmart0, prints the section 5 read-backs | the adopt policy over the model loaded with the dump (mode match, mode mismatch refused, AFBC cluster replaced by Esmart0) | read-backs as in section 5, then the person confirms the pattern | 250 lines + script change | yes |
| D3 | `board_show()` / `board_display_busy()` for the T6 over `ip::vop2` (flip by address at `REG_CFG_DONE`, busy until the next frame start) | the flip and busy logic on the model | with the Mali Zephyr OSAL (Z4): `gl_triangle` on screen by crash kexec | 200 lines | yes |
| D4 | `soc::rk3588` display power and clocks: PD_VOP, PD_VO1 on/off (the PD_GPU code with a domain table), the gates, muxes and resets of section 1, the PMU1 CRU | the domain table = `pm-domains.c` lines cited; order: clocks, power, repair poll, idle release; reset assert/release words; off-then-on loses block state in the model | none alone | 400 lines + tests | no |
| D5 | `ip::hdptx`: the TMDS bring-up order of section 1, the ROPLL plan (integer and SDM) and the register values from the dump, lock and ready polls, power-off | the plan for 25.2, 27, 74.25, 148.5 MHz; the write order inside the reset window; no lock = timeout; the dump's register file equals the model's image | none alone | 600 lines + tests | no |
| D6 | `ip::dw_hdmi_qp`: bind values, DVI enable, HDCP2 bypass, HPD and GRF glue bits, the CMU status read; I2C master left for D10 | layout; DVI vs HDMI bit; GRF words with their masks | none alone | 250 lines + tests | no |
| D7 | `soc::rk3588::display_chain`: the staged cold bring-up (power, clocks and resets, GRF, PHY, DCLK_VOP0 to the PHY clock, controller, VOP2, video check) with `stop-after`, `power-cycle`, `force-cold`, and the golden test against the dump | a cold bring-up in block order over the combined model; every `stop-after`; each injected failure names its stage; golden: every dumped register equals the model's image outside the allow list | none alone | 500 lines + tests | no |
| D8 | Cold run by normal kexec: the probe with `power-cycle`, bisected with `stop-after`, then the CPU pattern, then `gl_triangle` through D3 | as D7 | stage lines on the UART, the section 5 read-backs, the person confirms | runs | yes |
| D9 | Sign-off: the same image started by U-Boot from the eMMC (`t6-zephyr.cmd`), `gl_triangle` and `gl_slint` on screen | none new | UART verdict, read-backs, the person confirms | runs | yes |
| D10 | EDID over the controller's I2C master; the preferred mode within the PHY plans; HDMI mode with an AVI InfoFrame for HDMI sinks | I2C transfers on the model (done, NACK, segment) | the EDID equals Linux's copy from D0 | 300 lines | yes |

Order and parallel work: D0 first (the lead; nothing else needs the board
before D2). D1, D4, D5 and D6 can run in parallel by four agents: each is a
new file (`ip/vop2.rs` with `ip/rk_iommu.rs`, `soc/rk3588/display_power.rs`
or an extension of `soc/rk3588.rs`, `ip/hdptx.rs`, `ip/dw_hdmi_qp.rs`), each
with its own register model in its tests; only the `mod.rs` lines and
`Cargo.toml` features touch shared files, one line each. D5's values wait for
the D0 dump; its structure does not. D2 needs D1; D3 needs D1 and the Mali
Zephyr OSAL (Z4); D7 needs D1, D4, D5, D6 and owns the combined model
(`model_rk3588_display.rs`, as `model_hdmi.rs` is for the X2); D8 needs D7;
D9 needs D8 and the U-Boot boot of the image (the sign-off form of Z1-Z4);
D10 needs D6 and D8.

The existing `ip::dw_hdmi_tx` is the older DesignWare HDMI TX (8-bit
registers, the i.MX8MP's); the QP controller is a different IP and is not a
variant of it.

## 8. Unknowns and risks

| # | Question | What answers it |
|---|---|---|
| U1 | May the PHY's 212 analog init values be taken from the board dump (measurement) rather than from Linux's tables, as the design proposes? | owner decision (source form) |
| U2 | Which window and format does Linux use for VP0's primary plane: Esmart or Cluster, AFBC or linear? | D0: `vop2/summary`, `state` |
| U3 | Does Linux run the PHY at the mode's rate with exactly the table values (`hdptx.c:427` for 148.5 MHz), or with its calculator (`:900-970`)? | D0: PHY register file CMN 0x51-0x86 |
| U4 | After a normal kexec, is the PHY PLL still locked (a power-domain clock reference may hold it), and is PD_VOP on? | a GRF STATUS and PMU 0x290 read in the first D8 run before any write |
| U5 | Does a read of a block whose domain is off hang the bus or return zeros? | not to be tested by reading; the code reads status only after power-on and repair status, so the answer is not needed |
| U6 | Are `SYS_PD_CTRL`'s internal domains on as soon as the bit is cleared, or is there a settle time (Linux polls nothing)? | D8: read `SYS_PD_CTRL` back, then a window read-back |
| U7 | `CMU_*_FREQ` units and `VIDEO_MONITOR_STATUS` fields | D0 values against the known mode |
| U8 | Is `REG_CFG_DONE`'s VP bit self-clearing at the frame boundary (usable as "update taken")? | D2: read it right after the write and after one frame |
| U9 | Does the crash reservation fit the image window (0x10000000-0x1fffffff) on Armbian without moving anything else, and does the crash route leave panthor's GPU in a state Z1's power-on handles? | D2: `/proc/iomem` after reboot with `crashkernel=`; the probe draws with the CPU, the Mali case is D3 |
| U10 | Which physical connector is HDMI0 on the T6? | D0: the `readlink` check in section 6 |
| U11 | The 1920x720 panel's pixel clock and whether it accepts DVI | D0 with that panel: `modes`, the EDID, the PHY registers |
| U12 | A sink that is slow to answer DDC after power saving (seen with a 1920x720 panel on this board) | D10 retries; the fixed mode stays the fallback |
| U13 | Are the TX resets SRST_HDMITX0_REF and SRST_HDMIHDP0 needed for a cold start (no Linux driver pulses them)? | D8: the cold run first without, then with a pulse if the controller does not respond |
| U14 | `TIMER_BASE_CONFIG0` = 428571429: is it the reference clock rate on this board (CLK_HDMITX0_REF from aclk_hdcp1_root)? | D0: `clk_summary` |

Risks: the PHY's fractional mode may need SDM values the dump gives but no
formula checks (the integer fallback modes of section 3 remove that risk for a
first cold picture); the VOP bus and pixel clocks come from CRU PLLs that the
boot firmware sets (GPLL, CPLL, V0PLL): on the U-Boot boot nobody but U-Boot
sets them, so D9 reads their rates before trusting them, as the Z1 power-on
does for the GPU's PLL.
