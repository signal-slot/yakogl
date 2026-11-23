<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# NanoPC-T6 (RK3588): the picture on HDMI from Zephyr

Status: the host side of D1, D2, D3a and D4 to D8 is in the tree (the
modules, the chain, the adopt probe `t6_display` and the cold probe
`t6_cold`); D0 is the only step run on the board. The T6 leaves "work in
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

1. Read VP0 `DSP_CTRL` (STANDBY clear), its background `DSP_BG`, the
   windows Linux enabled and the mode from the VP timing registers; refuse
   a mode that is not one of the adopt's modes (section 3). The adopt keeps
   the mode it finds. Linux shows its picture on Cluster0 of VP0, and of VP1
   for the second HDMI output (board reading, 2026-11-12).
2. Turn off Linux's windows (`ip::vop2::take_over`). It finds the running
   ports itself (`DSP_CTRL` STANDBY clear), since whether Linux drives VP1
   as well is not something the caller should have to know. It disables
   every window except the one to keep, whatever its port, a Cluster window
   with both its enable bits (`WIN0_EN` in `CLUSTER_WIN_CTRL0`,
   `CLUSTER_ENABLE` in `CLUSTER_CTRL`; `rockchip_vop2_reg.c:283`, `:314`;
   Linux does the same, `rockchip_drm_vop2.c:149-155`). It refuses a window
   to keep that is already enabled (its address is Linux's). Then
   `REG_CFG_DONE` for every running port and one frame start waited for on
   each, so no window of Linux fetches any more.
3. Stop the IOMMU translating (`ip::rk_iommu::paging_off`): for both
   `vop_mmu` register sets Linux's order, stall, disable paging,
   `INT_MASK` = 0 and `DTE_ADDR` = 0, stall off (`rockchip-iommu.c:911-925`),
   then check `STATUS` bit 0 clear. Done after step 2, nothing reads a
   Linux IOVA as a physical address; until it is done the VOP would read
   through tables Zephyr may overwrite.
4. Make Esmart0 the only layer of VP0 (`ip::vop2::overlay_single`:
   `OVL_PORT_SEL` with its `REG_CFG_DONE` and the wait until it is in force,
   then `OVL_LAYER_SEL`) and point it at Zephyr's buffer
   (`ip::vop2::window_show`): `REGION0_YRGB_MST` (physical, below 4 GiB),
   `REGION0_VIR` (stride / 4), format ARGB8888 code 0 with R/B swap 0 for
   XRGB8888 memory order, enable last.
5. `REG_CFG_DONE` = bit 15 | bit 0 | bit 16; the change lands at the next
   frame.

The display is dark from step 2 to step 5, for a few frames. Step 4 before
step 2 would leave Linux's Cluster0 enabled outside every port's layer
range, and the source does not say whether such a window still fetches;
step 3 before step 2 would let it read its IOVA as a physical address.

What it needs: a `crashkernel=` reservation that holds the image window
0x10000000-0x1fffffff on Armbian's command line, and `nanopc-t6.sh --crash`.
`driver/tools/kexec-zephyr/t6-crashkernel.sh set` adds
`crashkernel=256M@0x10000000` to the `extraargs=` line of
`/boot/armbianEnv.txt` (`undo` restores the file byte for byte; neither
reboots, `status` reads the result after the reboot). The fixed base has no
fallback: if anything already lies there at boot, Linux reserves nothing and
logs "memory is in use" (`kernel/crash_reserve.c:401-421`). CMA
(`cma=256M`) is placed before it but top-down below 4 GiB, far above the
window (`arch/arm64/mm/init.c:327-333`, `mm/cma.c:394`, `:423`); the
tool's header has the details. `nanopc-t6.sh --crash` refuses unless
`/proc/iomem`'s "Crash kernel" range holds the window and the image plus the
kernel's 4 KiB control page, and while a crash image is already loaded.
Without `--crash` it takes the normal route, also while the reservation is
set (the kernel limits only crash segments to the reservation,
`kernel/kexec_core.c:200-210`), unless a crash image is loaded in it; with
`--go` both routes read `kexec_crash_loaded` again under the board lock.

**The mode before the jump.** The adopt accepts two modes, 1024x768 and
1920x1080 at 60 Hz (section 3; `board::nanopc_t6_display::MODES`). Linux
runs the sink's preferred mode. HDMI-A-1 goes to an HDMI capture unit whose
EDID prefers 1920x1080 at 148.5 MHz, and Linux runs that. The capture unit
gives no picture after the board changes to any other size (measured
2026-11-23 for 1024x768 and 1280x720), so the runner changes no size when it
does not have to. `nanopc-t6.sh --crash --go` reads VP0's mode under the
board lock and, after the copies and right before the jump, holds a mode:
VP0's own mode when it is one of the adopt's (no size change), else
1024x768. It holds a mode in both cases: while the holder is the DRM master,
fbdev cannot change the screen between the read-back below and the jump
(below, "Why the mode stays").

1. `t6-kms-mode.py hold` (beside the runner; Python's standard library only,
   since the board has python3 and no DRM tools) opens `/dev/dri/card0` as
   the DRM master, finds HDMI-A-1 and the CRTC that drives it, and picks the
   connector's mode whose clock, eight timing numbers and both sync flags are
   those of the mode `--mode` names (`1024x768` or `1920x1080`, nothing
   else). When the list has none, it refuses and changes nothing. It
   sets the mode with a dumb buffer of its own by the legacy `SETCRTC`,
   which puts the buffer on the CRTC's primary plane, the plane fbcon used
   (Cluster0, U2). It reads the CRTC back, prints one line that names the
   holder's pid, and leaves a child process that keeps the card open as
   master.
2. The runner reads VP0's mode back from Linux: the `crtc[N]: video_port0`
   block of `/sys/kernel/debug/dri/0/state`, its `enable`, `active` and
   `mode:` lines (`drm_atomic.c:443-455`). It jumps only when VP0 is on and
   its clock, eight numbers and flags are those of the mode held (flags
   `0xa` for 1024x768, both syncs negative; `0x5` for 1920x1080, both
   positive; not interlaced). It also records HDMI-A-1's `output_bpc` and
   `output_format` (U15).
3. Every path that does not jump stops the holder by that pid: a refusal of
   the mode step, a jump the board refuses (the linux-wdt check), a jump that
   did not happen, an error. On the board the stop first checks the boot_id
   and the pid's command line, so it never kills another process. The holder
   sets the CRTC back as it found it (fbcon's frame buffer and mode), removes
   its buffer and closes the card. The runner then reads VP0 again and
   records whether Linux's own mode is back. The holder also ends by itself
   after 600 s, for a runner that was killed before it could stop it.

**The screen path (T6_SCREEN).** The run must be seen. `nanopc-t6.sh --go`
takes the debug UART (`T6_UART`), the capture unit on HDMI-A-1
(`T6_SCREEN`, the farm's name of the unit; environment only) or both, and
refuses with neither. Each one that is set is proven before the load. The
screen path needs `--crash`, since the proof runs through the holder, which
only the crash route starts; `T6_SCREEN` without it is refused by name. The
host tools it needs (`connected`, `djpeg`, `python3`) are checked before
anything is copied. `tools/board.sh nanopc-t6 run` passes `BOARD_SCREEN` as
`T6_SCREEN`.

1. The nonce. The runner makes one per run (`t6-screen-nonce.py new`,
   beside the runner) and passes it to the holder (`--nonce`). The holder
   draws it into its own buffer as eight full-height vertical bars of equal
   width, each one of the eight saturated colours (black, blue, green, cyan,
   red, magenta, yellow, white: bit 2 red, bit 1 green, bit 0 blue). A nonce
   is an order of all eight colours, so a blank screen, a console or any
   picture of fewer colours never decodes to one. Four orders are left out
   as well: the colour bars a capture unit may show without a signal
   (white, yellow, cyan, green, magenta, red, blue, black, and that
   reversed) and the two plain ramps (`t6-kms-mode.py`, `NONCE_NOT_USED`).
2. The proof, after VP0's read-back (step 2 above): the runner asks the
   unit for pictures (`connected screenshot <unit> -o <file>`, each request
   bounded to 30 s) until one decodes to this run's nonce, at most
   `SCREEN_PROOF_S` seconds (default 90). Each picture is read for itself:
   on 2026-11-23 a picture showed the screen of two requests ago
   (signal-slot/connected#1714), so no request is taken as the fresh one.
   The decoder (`t6-screen-nonce.py decode`, on `djpeg -pnm`'s PPM) cuts the
   picture into the same eight bars and reads a grid of samples in the
   middle half of each bar's width and of the height: a channel at 160 or
   above is on, at 96 or below off; a bar is the colour 80% of its samples
   have. That leaves room for a limited range, noise and the JPEG's colour
   blur at the bars' edges. Proven: one record line with the picture's name
   (`proof-NN.jpg`, kept in the record) and the seconds since the hold
   began. Not proven: the mode step's refusal (no load, no jump, the holder
   stopped by its pid, Linux's mode back), with how many pictures came, the
   unit's last error text and what the last picture decoded to. The unit
   gives no picture of any mode but 1920x1080 (measured 2026-11-23), so a
   run on which the holder had to set 1024x768 is refused by this rule,
   not by one of its own.
3. Across the jump: a loop takes pictures from the jump until the board is
   back in Armbian or the runner's deadline passes, `screen-SSS.jpg` by the
   seconds after the jump; a failed request goes to `screen.log` (the
   tool's error output, the unit's name written `<T6_SCREEN>`). `finish()`
   stops the loop by its pid, the request in flight with it, and records
   how many pictures came and how many requests failed.
4. The record's first lines name the paths the run has (UART, screen or
   both), and a line before the jump says which were proven. With
   `T6_SCREEN` only there is no `uart.log` and no line about one. Which
   picture shows the probe's pattern is a separate tool's verdict; the
   runner only proves the path and captures.

Why the mode stays until the image starts. While the holder is master,
fbdev cannot take the CRTC back: its restore (through
`drm_client_modeset_commit`), colour map, ioctl, pan and hotplug paths need
`drm_master_internal_acquire`, which fails while a user-space master exists
(linux v6.18 `drm_auth.c:433-442`, `drm_client_modeset.c:1229-1230`,
`drm_fb_helper.c:1029`, `:1069`, `:1435`, `:1943-1944`). The one forced
restore is `set_par` with `FB_ACTIVATE_KD_TEXT` (a VT set back to text mode
from user space), and `set_par` does nothing while `oops_in_progress` is set
(`drm_fb_helper.c:1332-1353`); nothing in the runner switches the VT. The
panic of the crash route calls `__crash_kexec` before the kmsg dumpers (where
`drm_panic` would draw) and before the console unblank (`kernel/panic.c:508-509`),
and `__crash_kexec` stops the CPUs and enters the image with no
`device_shutdown` (`kernel/crash_core.c:120-138`). So VP0 still scans out the
holder's buffer in the mode held when the image starts, through the IOMMU;
the adopt's steps above are the same in both modes.

What it leaves behind: nothing that outlives the holder. No file outside
`/tmp/yakogl-kexec`, no package, no module, no property. When the holder
ends, by a signal or killed, the kernel removes its frame buffer and its
master, and the last close of the card lets fbdev set its own mode again
(`drm_file.c:253`, `:263-264`, `:406-411`, `:440-441`;
`drm_client_event.c:104-123`). The last close needs that no other process
has the card open; the runner's read under the board lock refuses while any
process holds a file of `/dev/dri`.

The other way, and why not: debugfs `edid_override`. A made-up 1024x768 EDID
written to `/sys/kernel/debug/dri/0/HDMI-A-1/edid_override` and `detect`
written to the connector's `status` would make fbdev pick 1024x768 on the
hotplug, with no process to keep. But it changes the kernel's view of the
sink until someone writes `reset` and probes again, so a runner killed in
between leaves the board on a made-up EDID until the next boot. The EDID is a
blob we would have to make and keep right (the mode, DVI or HDMI, the size).
The mode is then fbdev's own choice on a hotplug, so the read-back is still
needed. And giving Linux's mode back is a second override, not a stop. The
holder changes only the CRTC, for as long as it runs, and the kernel undoes
it when it ends.

The crash route has one more condition: no interrupt may be left on a
chip whose bus lock sleeps. Before the jump, `machine_crash_shutdown`
masks and shuts down every started interrupt, one after the other
(`arch/arm64/kernel/machine_kexec.c:213-225`, `kernel/irq/kexec.c`), and
for each it takes the chip's bus lock (`kernel/irq/manage.c:2689-2711`).
The PMIC (RK806, on SPI) is a regmap-irq chip: its bus lock is a mutex,
and its unlock writes the PMIC's mask and ack registers over SPI
(`drivers/base/regmap/regmap-irq.c:76-80`, `:83-207`;
`drivers/mfd/rk8xx-core.c:546-556`). An SPI transfer waits for the SPI
controller's interrupt (`drivers/spi/spi-rockchip.c:382-404`), which the
same loop shut down earlier, and only one CPU is left, with interrupts
off. So the panic sleeps for ever, "Starting crashdump kernel..." is never
printed, and Linux's watchdog resets the board. The PMIC's rows in
`/proc/interrupts` (chip `rk806`) were the two of its power key (driver
`rk805-pwrkey`). Measured on 2026-11-15 with `t6_hello`:
with the driver bound the jump hung and WDT0 reset the board; with its one
device unbound the `rk806` rows were gone, and the image ran and passed,
twice. `kexec_zephyr --crash` therefore unbinds every device of
`rk805-pwrkey` before CPUs 1-7 go offline (found by listing
`/sys/bus/platform/drivers/rk805-pwrkey/`, since the device's number
changes per boot), and, as its last step that can refuse, after the CPU
check and before the links go down, reads `/proc/interrupts` and refuses while
a row with a handler is on a chip other than the ones the crash route was
measured with (GICv3, GICv3-23, ITS-PCI-MSI-\*, ITS-PCI-MSIX-\*,
rockchip_gpio_irq). A program cannot ask a chip whether its bus lock
sleeps, so an unknown chip is refused, not let through. A refusal binds
the power key again, with the CPUs and the crash image put back as before
(`crash_irq.h` beside the tool; `crash_irq_selftest.sh` tests it on the
host). The PMIC's own interrupt line on a GPIO bank
(`rockchip_gpio_irq ... rk806`) stays after the unbind and did no harm.

The crash route itself ran on the board on 2026-11-15 with `t6_hello` only
(above); the adoption has not run yet. What it proves: the VOP2 window programming, the
frame buffer format and stride, the hand-over from a buffer to the screen.
What it does not prove: power, clocks, the PHY, the controller, the mode.
A risk of its own: the crash route also leaves panthor's GPU and its firmware
running, which the Z1-Z4 power-on has never met (they all ran after a normal
kexec). The adopt probe therefore draws with the CPU first; a Mali frame on
the adopted display is a separate step.

What an image must do first after a crash kexec (facts for D3 and the
K steps). The GPU is still powered, and its CSF firmware runs from Linux's
memory through the GPU's MMU, so before the GPU's interrupt lines are
enabled at the GIC and before the power-on: mask and clear `GPU_IRQ_MASK`,
`JOB_IRQ_MASK` and `MMU_IRQ_MASK`; halt the MCU (`MCU_CONTROL` disable,
wait for `MCU_STATUS` disabled); `GPU_COMMAND` SOFT_RESET and wait for
RESET_COMPLETED in `GPU_IRQ_RAWSTAT` (HARD_RESET when that times out),
after which the address spaces are off and the GPU no longer walks
Linux's page tables; only then the power-on, which must accept a domain
that is already on and must not cut PD_GPU without the PMU idle request
first. The Z1 power-on does this when it finds PD_GPU powered
(`gpu::mali::takeover::take_over`; driver/docs/nanopc-t6-mali.md, "The GPU
power-on"). Host tests cover it; no image with the GPU has run after a
crash kexec yet. Interrupts the image did not ask for still arrive: the
ITS stays on and a redistributor's EnableLPIs cannot be cleared, so LPIs
(INTID 8192 and up) from devices Linux left running can be signalled. An
INTID that is acknowledged and not ended leaves the running priority
raised, and the timer stops (the WDT then resets the board, and the run
is lost). Zephyr's interrupt entry ends every INTID it acknowledges, also
one above its table (`arch/arm64/core/isr_wrapper.S`, `spurious_continue`,
at the revision in `west.yml`). An INTID Linux had acknowledged and not
ended when the CPU crashed leaves its bit in `ICC_AP1R0_EL1`, which
Zephyr's GICv3 init does not clear: the T6 board glue writes the active
priorities and `ICC_BPR1_EL1` to 0 at EARLY and prints `ICC_AP1R0_EL1` as
found (`aarch64::gic_cpuif_reset`, issue #6366). The INTID itself stays
Active at the distributor or the redistributor; the glue deactivates every
Active INTID at the same hook and prints which ones it found
(`aarch64::gic_active_reset`, issue #10530). `kexec_zephyr --crash` takes every network
link down before the panic to cut the largest source.

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
starting the image from the eMMC (`driver/tools/uboot-t6/prompt-go.sh`),
which is the same code with nothing left on. The chain's caller for D8 is
the probe `tools/probes/nanopc_t6/t6_cold` (`board::nanopc_t6_display_cold`,
C entry `yd_t6_display_cold`): it runs the chain with `power-cycle` on a
CPU-drawn buffer, the stage to stop after a build option.

The chain's START refusals (`soc::rk3588_display_chain`, its module text has
the full rules) and two notes on them:

- Without `power-cycle` a VOP2 that is powered but cannot be read, a port it
  clocks that has no clock now, and (without `force-cold`) a running port are
  refused before any write; D8 and D9 therefore run with `power-cycle`.
- With `power-cycle` but without `force-cold`, a running port is refused
  with `FoundRunning` only after VOP2 was made readable: when PD_VOP was
  idle-requested, `pd_on` has by then written the PMU and released the bus
  of the picture before us.
- For D9: `SRST_D_VOP1` (53/0), `SRST_D_VOP2` (53/1),
  `SRST_D_VOP2HDMI_BRIDGE0` (52/14) and `SRST_P_VOPGRF` (53/3)
  (`rst-rk3588.c:540`, `:541`, `:536`, `:543`) are neither checked nor
  released by the chain; Linux drives none of them and the D0 dump reads
  them released.

### Which first, which signs off, what is shared

- First picture: **(a) by crash kexec** (two small steps), because it needs
  only the VOP2 window module and the board's display hook, and it gives the
  read-back proofs (section 5) a known-good link to be checked against.
- Sign-off: **(c) under U-Boot**. (b) does not exist.
- (c) reuses from (a): the `ip::vop2` window and `REG_CFG_DONE` code, the
  IOMMU-off code, the board's `board_show()` and its hand-over to EGL, the
  status read-back, the dump and its golden test. (a) adds only the
  "read the running mode and refuse one it does not know" policy, which (c)
  keeps as its "is the chain already running?" check.

## 3. Mode

The cold route (c) programs one fixed mode, 1024x768. The adopt (route (a))
programs no mode: it accepts the mode Linux runs when it is one of two,
1024x768 or 1920x1080, and refuses any other by name.

A fixed mode first, in DVI mode (no InfoFrames), RGB 8 bit: then the TMDS
character rate equals the pixel clock and the controller needs only
`LINK_CONFIG0` and `HDCP2LOGIC_CONFIG0`. The mode is the one Linux shows on
the display the lead plugs in, read from the dump, so every PHY value is
measured on that very rate. A sink with an EDID usually prefers
**1920x1080 at 60 Hz, 148.5 MHz** (CEA-861 VIC 16), which Linux's PHY table
has (`hdptx.c:427`); the T6's display has none (below).

Measured (board, 2026-11-12): the T6's display gives no EDID, and Linux runs
it at **1024x768 at 60 Hz, 65 MHz** (h 1024 1048 1184 1344, v 768 771 777
806, both syncs negative), DVI. That is the build's fixed mode
(`soc::rk3588_display_chain::MODE_1024X768_60`); the PHY runs Linux's table
row for 65 MHz (`hdptx.c:445-446`), read back from the dump.

The T6's HDMI goes to an HDMI capture unit with an EDID that prefers
1920x1080 at 148.5 MHz (h 1920 2008 2052 2200, v 1080 1084 1089 1125, both
syncs positive), and Linux runs that mode, RGB at 8 bits per colour
(`driver/docs/nanopc-t6-display-dump-linux-1920x1080.txt`, 2026-11-23).

The adopt's modes are `board::nanopc_t6_display::MODES`: 1024x768 at 65 MHz
and 1920x1080 at 148.5 MHz, each its own constant, not the cold route's.
Step 1 of the adopt reads VP0's four timing words and accepts them when they
are one of the two; any other timing is `Stop::Mode`, with the first word
that differs from the closest mode. The adopt changes no timing word, no
clock and nothing of the PHY or the controller, so it needs no PHY values
for 148.5 MHz. The crash route holds the mode Linux runs when it is one of
the two, else it sets 1024x768 (section 2(a), "The mode before the jump").

The window does not depend on the mode. It sits at the screen's origin and
must lie inside `WINDOW_AREA`, the top left 1024x768 (the smaller mode), in
both modes: a window that fits is shown the same whichever mode runs. The
probes' buffer is 1024x768. On a 1920x1080 screen the rest is VP0's
background colour, `DSP_BG`. Linux writes it 0 (black) when it starts the
port (`ip::vop2` fact 8), both dumps read 0, and step 1 refuses another
value (`Stop::VpState`, index 1). Esmart0 is VP0's only layer
(`vop2::overlay_single`) with `HDR0_SRC_COLOR_CTRL` 0, so nothing else is
mixed there.

The cold route keeps 1024x768 (`soc::rk3588_display_chain::t6`). It programs
the PHY itself, and 148.5 MHz needs the fractional PLL values below, which it
does not have.

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
`present` op, which is the board library's `yakogl_zephyr_present`
(`driver/zephyr/lib/board/common/board.c`) ->
the board's `board_show()` (AM62P `board_am62p.c:69-89`, i.MX8MP
`board_imx8mp.c:358`, Pi 4 `board_rpi4.c:299`). The Mali Zephyr OSAL
(`src/osal/zephyr_mali.c`) fills `.present` with the same function, as
`zephyr.c`, `zephyr_v3d.c` and `zephyr_gc.c` do. The T6's `board_show()` and
`board_display_busy()` are step D3 (the end of this section); with a
`board_display_busy()` (as the AM62P's) EGL keeps four buffers while the VOP
may still scan one.

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

### Step D3: `board_show()` and `board_display_busy()` on the T6

The T6 takes the common board layer (`one-method.md`, section 2) as the
other boards do, so `examples/triangle` and every other sample run on it
unchanged. The step has two parts:

- **D3a, host only.** The rules below in Rust, with their tests on the
  register model: `board::nanopc_t6_display` (`Screen`, `show`, `busy`), the
  reserved end of the carve-out in `board::nanopc_t6_osal`, and their C
  entries in `yakogl_drivers.h` (`struct yd_t6_screen`, `yd_t6_screen_show`,
  `yd_t6_screen_busy`, `yd_t6_osal_mem.carve_reserved`).
- **D3b, the board library and the image.**
  `driver/zephyr/lib/board/nanopc_t6/board_nanopc_t6.c`: `board_window()`,
  `board_show()` and `board_display_busy()` call those entries; the triangle
  image (`west build -b nanopc_t6 examples/triangle`) and its gate row; then
  the board proof. The first `board_show()` prints the adopt's record with
  the lines of the `t6_display` probe under the tag `board`
  (`t6_display_record.c`), so a stopped adopt reads the same in both. The
  image has one "still making progress" for the watchdog: with the board
  layer `t6_watchdog_feed()` is `board_watchdog_feed()`, and WDT0's feeder
  asks the layer's `board_wdog_alive()`.

**Where the window's memory is.** The Mali renders into the window, so the
window must be memory the GPU's address space maps, and that is the OSAL's
carve-out and nothing else. A sample asks for its window before EGL starts
the OSAL. The OSAL therefore keeps the last `carve_reserved` bytes of the
carve-out out of its allocator: they are mapped with the rest, no
`yd_t6_osal_alloc` returns them, and a kick whose stream reaches into them
is refused. `board_window()` hands that end out, once. `carve_reserved` is
whole pages and less than the carve-out, or the OSAL does not come up
(`YD_T6_SUBMIT_STEP_VM_MAP`, refusal `YD_T6_OSAL_VM_MAP_RESERVED`, before any
register is written). The window needs no entry of its own in the GPU's
address space: the OSAL's `window()` already resolves any range inside the
carve-out. EGL's other three buffers are ordinary allocations.

**The first show adopts.** The first `board_show()` is the adopt of route
(a) with the GL window in place of the probe's CPU pattern: the same checks,
the same order, the same record (`struct yd_t6_display_adopt`). The window
sits at the screen's origin and is scanned in place; nothing is copied. An
adopt that stops is final: that show and every later one return `-ENODEV`,
and the display is not written again. It is in a state the adopt did not
expect, and a second try per frame would write to it again without knowing
more.

**The geometry is the first show's.** A later show must have the first
show's width, height and stride; another one is refused with `-ENOTSUP`
before any register is read or written. The reason is tearing. A flip writes
one register, the address, and the VOP takes a register whole at a frame
start. A size change is several registers, and a frame start between two of
the writes can take half of them when an earlier `REG_CFG_DONE` is still
pending.

**A flip never waits.** A show of another buffer writes the address, then
`REG_CFG_DONE` for VP0, then clears VP0's frame start flag, and returns. A
show of the buffer already shown does nothing.

**Which buffers are busy.** Until a frame start takes the new address, the
display scans an earlier buffer. `board_display_busy()` reports the last
buffer shown and every buffer that was the last one at a flip since the
display last caught up. The display has caught up when both hold: VP0's raw
frame start flag is set (it was cleared after the last flip's
`REG_CFG_DONE`), and VP0's bit of `REG_CFG_DONE` reads 0. Linux tests the
same two things before it completes a page flip: `vop2_crtc_atomic_flush`
ends with `vop2_cfg_done` (`rockchip_drm_vop2.c:1846-1866` in v6.18), and
`vop2_isr` sends the flip event on a frame start interrupt only when the
port's bit of `REG_CFG_DONE` is clear (`:2175-2188`). The screen remembers
two earlier buffers. A third flip inside one frame makes every buffer busy
until the display catches up: the answer may be too large for a frame and
is never too small, which is `yakogl_board.h`'s contract. EGL then waits for
a free buffer and says so once (`next_window_buffer`, `src/gl/egl.rs`).

Whether VP0's bit of `REG_CFG_DONE` clears at the frame start is U8. Both
answers are safe here. A bit that always reads 0 leaves the frame start flag
to decide, and that is enough because the flag is cleared after the
`REG_CFG_DONE` write. A bit that never clears keeps the buffers busy, and
EGL's once-only line shows it on the UART.

Before the first show no buffer is busy. After an adopt that stopped, the
buffer of that show is busy (the adopt may have stopped after it pointed
Esmart0 at the buffer) and no register is read.

**Not in D3.** The Mali OSAL has no `wait_vblank`. EGL's wait for a free
buffer is then its poll of up to 100 ms (`BUSY_WAIT_NS`), not a wait for
vertical periods. A frame start interrupt for it is a later step.

**What the triangle proves.** `examples/triangle` is single-buffered and
shows the same window every frame, so its picture proves the first show: a
Mali-rendered window adopted in place. It does not flip. The flip and the
busy rule are first run on the board by a double-buffered image (gl_test,
dEQP); the board library prints the screen's flip count at the end of a run
so that the UART record says whether they ran.

**The record screen.** A run judged from the capture unit's pictures has no
UART text, so the picture alone does not say whether the image's own check
passed. With `CONFIG_YAKOGL_BOARD_RECORD_SCREEN` (default on) the board
library keeps the console's last lines in a ring
(`driver/zephyr/lib/board/common/record_screen.c`; the rules are
`board::record_screen` in the driver crate) and draws them over the window
shown last, the last `yk-record:` line at the bottom. It draws into the
buffer the VOP already scans, so it writes no register. Three ends draw it:

- `board_reset()`, after `board_finish()`'s hold: the record stays for
  `CONFIG_YAKOGL_BOARD_RECORD_SCREEN_S` (20 s), then the reset. A run that
  failed before its first frame shows the window it took, which is the adopt;
- the watchdog's feeder when it stops for no progress or at the lifetime's
  end (a hang): the record stays until WDT0's reset, one period;
- a fatal error (`k_sys_fatal_error_handler`): Zephyr's register lines are in
  the ring; the image halts and the record stays until WDT0's reset.

The window is 1024 pixels wide at most, so a row holds 128 characters of the
8x16 font and a longer line continues on the next row.

## 5. Proof without a camera

What can be read back (every one of these is a register Linux names; values
compared against the dump):

| Proof | Register | Meaning |
|---|---|---|
| VP0 runs frames | VP0 `INT_RAW_STATUS` 0xAC bit 0 (FS, frame start); clear with `INT_CLR` 0xA4 = bit \| bit<<16, read again after more than one frame | the VP's timing generator runs on a pixel clock |
| No underflow | the same register bit 4 (POST_BUF_EMPTY) stays 0 over many frames | the window's fetch keeps up: our buffer address is readable memory |
| No bus error | `SYS0_INT_STATUS` 0x88 / `SYS1_INT_STATUS` 0x98 bit 1 | the AXI fetch completed |
| Our buffer is the one shown | Esmart0 `REGION0_YRGB_MST` reads our address after `REG_CFG_DONE` | the window register holds our address. Whether the `REG_CFG_DONE` VP bit goes back to 0 once the update is taken is open (U8): it is logged, not used as a proof |
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

## 6. The dump of the running Linux

The dump is made by `driver/tools/kexec-zephyr/t6-display-dump.sh`. Its read
list is in the script (`--plan` prints it: 76 lines, 1120 words) and is the
only copy; this section says what the list holds, what it must not hold, and
what the dump of 2026-11-12 shows.

```sh
driver/tools/kexec-zephyr/t6-display-dump.sh --plan                    # the read list, no board
BOARD_HOST=root@<t6> driver/tools/kexec-zephyr/t6-display-dump.sh      # dry run: the board's state, no register read
BOARD_HOST=root@<t6> T6_HAND=<name> T6_RECORD_DIR=<dir> \
  driver/tools/kexec-zephyr/t6-display-dump.sh --go                    # the reads
```

Preconditions, checked by the tool before the first read and again in the
same command as every read: the connector of `fde80000.hdmi` (found through
its `ddc` link, `card0-HDMI-A-1` on this board) reads `connected` and
`enabled`, the power domains `vop` and `vo1` are on, and `hclk_vop`,
`aclk_vop`, `pclk_hdmitx0` and `pclk_hdptx0` have users. A read of a block
whose domain or bus clock is off may hang the bus, so the tool stops at the
first check that does not hold. It takes the board lock and asks for
`T6_HAND`.

What the list reads, in this order:

- PMU: idle request, acknowledge and state, the power gate, the status, the
  memory power, chain and memory status, the repair status (offsets of
  `pm-domains.c`'s RK3588 table).
- CRU: the V0PLL, CPLL and GPLL words and the mode word, the selects, gates
  and resets of the VOP, VO1, the HDMI controller and the PHY.
- The GRFs (sys_grf, vop_grf, vo1_grf, hdptxphy0_grf), through their regmap
  files in debugfs, not by address.
- VOP2: the system block, the overlay, VP0 and the `DSP_CTRL` of VP1-VP3,
  Cluster0, Cluster1 and the four Esmart windows.
- vop_mmu: `DTE_ADDR`, `STATUS` and `COMMAND` of both sets.
- HDMI QP TX0: the registers that answered on the board (see below).
- HDPTX PHY0: the ranges `hdptx.c:776-790` allows.
- PMU1 CRU, last: the PHY's reference clock select and gate, its resets.

Only the first HDMI output is read. The QoS words are not read.

The record directory of a run holds `dump.txt` (the registers), `files.txt`
(the connector status, `dri/0/state`, `vop2/summary`, `vop2/regs`,
`pm_genpd_summary`, the PHY's regmap file, the display clocks of
`clk_summary`, the display lines of `dmesg`) and `record.txt`. Only
`dump.txt` goes into the tree. The kernel command line and the EDID are not
copied: the first carries the root partition's id, the second the display's
serial number.

Must NOT be read:

- the controller's I2C master data and control (0xe0-0x118) and the
  controller's whole regmap file (it has no readable list and would read
  every word up to 0x482c): a read may advance or disturb a transfer (not
  documented);
- the controller's words that request an action (`GLOBAL_SWRESET_REQUEST`
  0x40, `TMDS_FIFO_CONTROL0` 0x974, `PKTSCHED_PKT_SEND` 0xab0): Linux only
  writes them, and what a read of them does is not documented;
- **a controller register that has not been read alone on the board.** A
  register this controller does not have answers with a bus error. Measured
  on 2026-11-12 for 0x10 (not in `dw-hdmi-qp.h`) and for 0x804 and 0x80c
  (both in the header): the header is not proof that a register exists. The
  reading program dies; the board and the picture stay up. The same holds for
  the Zephyr driver: it reads only registers of the tool's list;
- the VOP's gamma LUT RAM 0xfdd95000 and the HDR LUT region (0x2000+): LUT RAM
  accessed while the VP uses it (Linux switches `DSP_LUT_EN` off and polls
  before writing, `rockchip_drm_vop2.c:886-915`);
- anything of a block whose domain or bus clock is off (hang), so not the
  second HDMI/PHY instance unless it is on;
- the PHY register file outside the ranges of `hdptx.c:776-790`;
- the PMU's power, idle request and memory power **writes** (the tool reads
  with a program that opens the memory read only).

The dump file is `driver/docs/nanopc-t6-display-dump-linux.txt`
(`ADDRESS WIDTH VALUE`, as the X2's). The same read list at 1920x1080 (the
capture unit, 2026-11-23) is
`driver/docs/nanopc-t6-display-dump-linux-1920x1080.txt`; the adopt's host
tests run on both. The allow list of the 1024x768 dump (the cold route's
mode) is `driver/docs/nanopc-t6-display-allow.txt` (`ADDRESS MASK REASON`, as
`x2-hdmi-allow.txt`): the bits that may differ between the Zephyr cold path
and the dump, each with its reason. Interrupt status registers (VOP
0x88/0x98/0xA8/0xAC, controller 0x3010, 0x3020) are read-only status and not
cleared by a read; they are sampled, so they are in the allow list.

What the dump of 2026-11-12 shows (Linux 6.18.55, an LCD on the first HDMI
output; two runs nine minutes apart, 1120 words each):

- The sink gives no EDID (0 bytes), so Linux runs its default mode:
  1024x768 at 60 Hz, 65 MHz, h 1024 1048 1184 1344, v 768 771 777 806, both
  syncs negative. `dclk_vop0` is 65 MHz. That is the fixed mode of section 3.
- Linux shows its picture in **Cluster0** of VP0, not in an Esmart window:
  linear XRGB8888, pitch 4096, buffer address 0 through the IOMMU
  (`vop2/summary`). Cluster0 `WIN_CTRL0` and `CLUSTER_CTRL` read 1, Esmart0
  `CTRL0` reads 0. The design keeps Esmart0, so the window registers and the
  overlay's layer and port selects differ from the dump by design.
- VP0 `DSP_CTRL` reads 0x0000000f; VP1, VP2 and VP3 read 0x8000000f
  (bit 31, STANDBY, set).
- The PHY's 436 words read by address equal Linux's regmap file word for
  word.
- `CMU_VIDQPCLK_FREQ` reads 16250 and `CMU_LINKQPCLK_FREQ` 16249: the unit is
  4 kHz (16250 x 4 kHz = 65 MHz). The video monitor words 0x884-0x898 hold
  the mode's timing (sync 136 and 6, front porch 24 and 3, total 1344 and
  806, active 1024 and 768).
- `TIMER_BASE_CONFIG0` reads 428571429, and `clk_hdmitx0_ref` is
  428571429 Hz in `clk_summary`.
- Four words differ between the two runs: the PMU's chain and memory status
  (bits of the video codec domains; the VOP's and VO1's bits did not move),
  the word after the repair status, and the VOP's `SYS_STATUS0`. They are in
  the allow list.
- The picture stayed on the LCD through every run, and the kernel log gained
  no display line.

What the dump of 2026-11-23 shows (the capture unit at 1920x1080; 78 of the
1120 words differ from the dump above):

- VP0's timing words are 1920x1080's, and `DSP_CTRL` reads 0x0000000f as
  before (RGB, 8 bits per colour). `DSP_BG` reads 0 in both dumps.
- `DSP_IF_EN` reads 0x00000008: HDMI0 on VP0 only. The 1024x768 dump read
  0x00040028, which also had HDMI1 enabled on VP1 (bit 5, mux 19:18 = 1);
  `DSP_IF_CTRL` differs by HDMI1's dividers the same way. The adopt checks
  HDMI0's bits only, so both pass.
- `vo1_grf` 0x00 reads 0x00000020: the sync field (bits 6:5) has bit 0 for
  a positive VSYNC (`ip::vop2` fact 10). The adopt does not touch it.
- Cluster0 shows a 1920x1080 window; the overlay's layer and port selects
  are Linux's own (0x76543210, 0xa0507873); the PMU, CRU, `VERSION_INFO`
  and `SYS_PD_CTRL` words the adopt checks are the same as at 1024x768.
- The HDMI controller's video monitor words hold 1920x1080's timing (sync
  44 and 5, total 2200 and 1125, active 1920 and 1080).

## 7. The step plan

Each step: what it proves, its host test, its board proof, its size. The
host tests run against a register model in the manner of `model_hdmi.rs`:
it refuses access to a block whose domain, clock or reset does not allow it,
answers polls, takes injected failures and records the order.

| Step | What | Host test | Board proof | Size | Display plugged |
|---|---|---|---|---|---|
| D0 | The dump: `driver/tools/kexec-zephyr/t6-display-dump.sh` (read only), its output committed as the dump file and the allow file seeded. Done on 2026-11-12 (section 6) | `t6-display-dump-selftest.sh`: the list check, the refusals and a whole run against a fake board | the run of section 6: 1120 words, the picture stayed on | script 490 lines, self-test 390 | yes |
| D1 | `ip::vop2`: register map, Esmart window (format, address, stride, size, enable, AXI ids), overlay select, VP timing and dividers, `REG_CFG_DONE`, internal PD (`SYS_PD_CTRL`), STANDBY with DSP_HOLD wait, status read-back; `ip::rk_iommu`: paging off and check | layout = section 1; timing words for the dump's mode; a window refused above 4 GiB, unaligned, or with a stride over 16 bits; CFG_DONE after the window writes; standby waits for DSP_HOLD | none alone | 700 lines + tests | no |
| D2 | Adopt (route a): `nanopc-t6.sh --crash` (with its mode step, `t6-kms-mode.py`, section 2(a)) and the T6 crash reservation; a `t6_display` probe that adopts VP0, turns the IOMMU off, shows a CPU pattern in Esmart0, prints the section 5 read-backs | the adopt policy over the model loaded with both dumps (each mode accepted and kept, a third mode refused, AFBC cluster replaced by Esmart0) | read-backs as in section 5, then the person confirms the pattern | 250 lines + script change | yes |
| D3a | The T6's show, flip and busy rules over `ip::vop2` (section 4, step D3): `board::nanopc_t6_display` `show` / `busy`, the carve-out's reserved end, the C entries. Done, host only | the first show is the adopt with the GL window; a flip writes the address then `REG_CFG_DONE` and never waits; busy until the frame start that takes the flip; a stopped adopt is final; the reserved end is mapped and never allocated | none alone | 330 lines + tests | no |
| D3b | The T6 board library: `board_window()` (the reserved end), `board_show()`, `board_display_busy()`; the triangle image and its gate row. Done, the board proof open | the image builds with `-Werror` and fits the kexec window (gate row) | `gl_triangle` on screen by crash kexec; the flip count of a double-buffered image | 200 lines | yes |
| D4 | `soc::rk3588` display power and clocks: PD_VOP, PD_VO1 on/off (the PD_GPU code with a domain table), the gates, muxes and resets of section 1, the PMU1 CRU | the domain table = `pm-domains.c` lines cited; order: clocks, power, repair poll, idle release; reset assert/release words; off-then-on loses block state in the model | none alone | 400 lines + tests | no |
| D5 | `ip::hdptx`: the TMDS bring-up order of section 1, the ROPLL plan (integer and SDM) and the register values from the dump, lock and ready polls, power-off | the plan for 25.2, 27, 74.25, 148.5 MHz; the write order inside the reset window; no lock = timeout; the dump's register file equals the model's image | none alone | 600 lines + tests | no |
| D6 | `ip::dw_hdmi_qp`: bind values, DVI enable, HDCP2 bypass, HPD and GRF glue bits, the CMU status read; I2C master left for D10 | layout; DVI vs HDMI bit; GRF words with their masks | none alone | 250 lines + tests | no |
| D7 | `soc::rk3588_display_chain`: the staged cold bring-up (power, clocks and resets, GRF, PHY, DCLK_VOP0 to the PHY clock, controller, VOP2, video check) with `stop-after`, `power-cycle`, `force-cold`, and the golden test against the dump | a cold bring-up in block order over the combined model; every `stop-after`; each injected failure names its stage; golden: every dumped register equals the model's image outside the allow list | none alone | 500 lines + tests | no |
| D8 | Cold run by normal kexec: the probe `t6_cold` with `power-cycle` (`-DT6_COLD_STOP_AFTER=N` builds it to stop after stage N, for a bisection), then the CPU pattern, then `gl_triangle` through D3. Host side done: `board::nanopc_t6_display_cold` and its C entry `yd_t6_display_cold`, the probe (its pattern, hold and read-backs shared with `t6_display` in `common/t6_picture.c`), its gate row, `t6-verdict.sh t6_cold` with a predicted record for a run to the end and for a stop after stage 4 | as D7; the state a normal kexec leaves after Linux ran 1920x1080 runs through with `power-cycle` and is refused at START without it; each stop is reported as asked with its stage line last | the reads before the chain (U4), a line per stage on the UART, the section 5 read-backs, the person confirms | runs | yes |
| D9 | Sign-off: the same image started by U-Boot from the eMMC (`driver/tools/uboot-t6/prompt-go.sh`; it takes only the t6_hello..t6_submit probes today, so `gl_triangle` and `gl_slint` need adding to it and to `t6-verdict.sh`), `gl_triangle` and `gl_slint` on screen | none new | UART verdict, read-backs, the person confirms | runs | yes |
| D10 | EDID over the controller's I2C master; the preferred mode within the PHY plans; HDMI mode with an AVI InfoFrame for HDMI sinks | I2C transfers on the model (done, NACK, segment) | the EDID equals Linux's copy from D0 | 300 lines | yes |

Order and parallel work: D0 first (the lead; nothing else needs the board
before D2). D1, D4, D5 and D6 can run in parallel by four agents: each is a
new file (`ip/vop2.rs` with `ip/rk_iommu.rs`, `soc/rk3588/display_power.rs`
or an extension of `soc/rk3588.rs`, `ip/hdptx.rs`, `ip/dw_hdmi_qp.rs`), each
with its own register model in its tests; only the `mod.rs` lines and
`Cargo.toml` features touch shared files, one line each. D5's values wait for
the D0 dump; its structure does not. D2 needs D1; D3a needs D2; D3b needs
D3a and the Mali Zephyr OSAL (Z4); D7 needs D1, D4, D5, D6 and owns the combined model
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
| U2 | Which window and format does Linux use for VP0's primary plane: Esmart or Cluster, AFBC or linear? | answered by D0: Cluster0, linear XRGB8888 (section 6) |
| U3 | Does Linux run the PHY at the mode's rate with exactly the table values (`hdptx.c:427` for 148.5 MHz), or with its calculator (`:900-970`)? | D0: PHY register file CMN 0x51-0x86 |
| U4 | After a normal kexec, is the PHY PLL still locked (a power-domain clock reference may hold it), and is PD_VOP on? | a GRF STATUS and PMU 0x290 read in the first D8 run before any write (`t6_cold`'s "before:" lines; the GRF only when the PHY's clocks and reset allow it) |
| U5 | Does a read of a block whose domain is off hang the bus or return zeros? | not to be tested by reading; the code reads status only after power-on and repair status, so the answer is not needed |
| U6 | Are `SYS_PD_CTRL`'s internal domains on as soon as the bit is cleared, or is there a settle time (Linux polls nothing)? | D8: read `SYS_PD_CTRL` back, then a window read-back (`t6_cold`'s "VOP:" line and its read-backs) |
| U7 | `CMU_*_FREQ` units and `VIDEO_MONITOR_STATUS` fields | answered by D0: 4 kHz per count; the monitor words hold sync, porch, total and active (section 6) |
| U8 | Is `REG_CFG_DONE`'s VP bit self-clearing at the frame boundary (usable as "update taken")? | D2: read it right after the write and after one frame. D3's busy rule is safe with either answer (section 4, step D3) |
| U9 | Does the crash reservation fit the image window (0x10000000-0x1fffffff) on Armbian without moving anything else, and does the crash route leave panthor's GPU in a state Z1's power-on handles? | D2: `/proc/iomem` after reboot with `crashkernel=`; the probe draws with the CPU, the Mali case is D3 |
| U10 | Which physical connector is HDMI0 on the T6? | answered by D0: the one the LCD is on is `card0-HDMI-A-1`, `fde80000.hdmi` |
| U11 | The 1920x720 panel's pixel clock and whether it accepts DVI | D0 with that panel: `modes`, the EDID, the PHY registers |
| U12 | A sink that is slow to answer DDC after power saving (seen with a 1920x720 panel on this board) | D10 retries; the fixed mode stays the fallback |
| U13 | Are the TX resets SRST_HDMITX0_REF and SRST_HDMIHDP0 needed for a cold start (no Linux driver pulses them)? | D8: the cold run first without, then with a pulse if the controller does not respond |
| U14 | `TIMER_BASE_CONFIG0` = 428571429: is it the reference clock rate on this board (CLK_HDMITX0_REF from aclk_hdcp1_root)? | answered by D0: yes, `clk_hdmitx0_ref` is 428571429 Hz |
| U15 | With the capture unit's EDID (an HDMI sink), does Linux drive RGB at 8 bits per colour, so VP0's `DSP_CTRL` is the value the adopt expects? Another depth or format changes `DSP_CTRL`, and the adopt then stops at its state check | answered for 1920x1080 by the dump of 2026-11-23: RGB, 8 bits, `DSP_CTRL` 0x0000000f (section 6). For a set 1024x768 on that sink: the record's "HDMI-A-1 output" line, then the adopt's own read |

Risks: the PHY's fractional mode may need SDM values the dump gives but no
formula checks (the integer fallback modes of section 3 remove that risk for a
first cold picture); the VOP bus and pixel clocks come from CRU PLLs that the
boot firmware sets (GPLL, CPLL, V0PLL): on the U-Boot boot nobody but U-Boot
sets them, so D9 reads their rates before trusting them, as the Z1 power-on
does for the GPU's PLL.
