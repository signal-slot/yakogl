<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK display bring-up: Q7, and who sets the clocks, resets, RIF and supplies

Date 2026-11-12, base `1e4eefe8`. Plan #353, item display-study (roadmap
P8, `pre-study-stm32mp257f-dk.md:904`; issues #1281 and #7). A host study
from sources: **nothing here ran on the board, no code changed.** It answers
the pre-study's Q7 (`pre-study-stm32mp257f-dk.md:822`), lists every clock,
reset, supply, RIF entry and pin the path LTDC -> DSI -> ADV7535 -> HDMI
needs with who sets it before our image runs, what the image sets itself and
what it asks OP-TEE for, and proposes where the frame buffer goes.

Plan #349's display study, `docs/notes/stm32mp257f-dk-display.md` (on main
at `42dae4ba`, not in this note's base; cited **D349:**`line`), already
describes the register maps, the DSI PHY PLL search and the Linux order of
the chain. This note does not repeat those; it checks them against the trees
and adds what they leave open: the SCMI clock *enables* (not only the rates),
the HDMI 5 V supply, the I2C bus's clocks and pins, the LTDC's bus master,
the 720p numbers and the frame buffer. Where this note differs from D349 it
says so.

## Sources, and how to read the citations

| Tag | Tree | Version |
| --- | --- | --- |
| **L** | ST Linux, `STMicroelectronics/linux` | tag `v6.6-stm32mp-r3.1` (commit `548f960c`), the kernel the board runs (6.6.129, pre-study table) |
| **M** | Linux mainline, `torvalds/linux` | `master` at `9d66f26f` (7.4-rc2), depth 1, fetched 2026-11-12 |
| **O** | ST OP-TEE, `STMicroelectronics/optee_os` | tag `4.0.0-stm32mp-r3.1` (commit `00635366`), the pre-study's tag |
| **U** | ST U-Boot, `STMicroelectronics/u-boot` | tag `v2023.10-stm32mp-r3.1` (commit `f8917a01`) |
| **T** | ST TF-A, `STMicroelectronics/arm-trusted-firmware` | tag `v2.10-stm32mp-r3.1` (commit `104df653`) |
| **P** | this repository | base `1e4eefe8` |
| **D349** | `docs/notes/stm32mp257f-dk-display.md` | main `42dae4ba` |

All five trees are cloned (depth 1) under `$CONNECTED_DEPS_DIR/st-src/`
(`linux-r3.1`, `linux-mainline`, `st-optee-r3.1`, `u-boot-r3.1`,
`tfa-r3.1`). A citation is `tag:file:line`; DT files of L are under
`arch/arm64/boot/dts/st/`, of O under `core/arch/arm/dts/`. The GPL and BSD
trees are read for facts only; nothing is copied (pre-study, "What was
read"). **[D]** marks a fact that is derived (computed, or concluded from
several citations) and not stated by one source. The board's own OP-TEE,
TF-A and U-Boot binaries are not known to be exactly these tags (pre-study
table, rows *OP-TEE, TF-A* and *ST U-Boot*): every "left set by" below is
what the tag's code and configuration do, to be confirmed by the read-only
probe of item state-probe.

**Not read.** RM0457 (reference manual), UM3385 (user manual) and MB1605
(schematic). This pass tried again: RM0457's public URL answers, but the PDF
is larger than the fetch tool's 10 MiB limit and a plain download needs a
permission this run did not have; UM3385 downloaded (6.3 MB) but its text is
in CID-encoded fonts and no PDF text tool is installed on this host. So, as
in the pre-study (`:85-98`), **every register offset and bit below is the
drivers' and unconfirmed against the manual**, and the board-level facts
(supplies, pull-ups, the HPD wiring) are the device trees', not the
schematic's.

## The chain on one page

```
 OP-TEE (SCMI)                        RCC (non-secure, guarded by the peripheral's RIFSC entry)
 FLEXGEN_27  PLL8 594/4 = 148.5 MHz -> RCC_LTDCCFGR 0x840 (gate b1, reset b0) -> ck_ker_ltdc
 FLEXGEN_28  PLL8 594/22 = 27 MHz  --> RCC_DSICFGR  0x844 (gate b1, reset b0, DSIPHYCKREFSEL b15 = FLEX28) -> DSI PHY ref
 FLEXGEN_12  PLL4 1200/12 = 100 MHz -> RCC_I2C2CFGR 0x7A8 (gate b1, reset b0) -> I2C2 kernel clock
 ICN_DISPLAY (critical, always on); GPIOB, GPIOF bus clocks (SCMI 90, 94)

 SYSCFG 0x44235000 DISPLAYCLKCR = 2 (ck_ker_ltdc) -> LTDC pixel clock

 DDR (RISAF4 region 9 or 10, CID0 r/w) --AXI, RIMU master--> LTDC 0x48010000 --RGB888 DPI--> DSI 0x48000000
                                                               SPI 158/159          4 lanes, non-burst sync pulses
                                                                                       |
 I2C2 0x40130000, PF2 SCL / PF0 SDA AF9, 400 kHz  --control--> ADV7535 0x3d (cec 0x3c, edid 0x3f, packet 0x38)
 PB6 reset (active low), PB4 interrupt (falling)                v1v8 = BUCK5, v3p3 = BUCK7 (SCMI 14, 16, always on)
                                                                  |  TMDS
                                                    HDMI type A, +5 V = LDO5 (SCMI voltage domain 21, NOT always on)
```

## Q7, answered from the sources

### The LTDC

- **Block.** `st,stm32mp25-ltdc` at 0x48010000, 0x400 bytes, GIC SPI 158 and
  159 level high, clocks `bus` (`CK_BUS_LTDC` 206) and `lcd` (`CK_KER_LTDC`
  339), reset `LTDC_R` (76), power domain `d1_pd`, RIFSC entries 80 (`cmn`),
  119 (`l1l2`), 120 (`l3`), 121 (`rot`) (L:`stm32mp251.dtsi:2150-2177`); on
  the MP255/257 two more clocks, `ref` (the SYSCFG pixel-clock mux) and
  `lvds` (L:`stm32mp255.dtsi:30-33`). Identity `LTDC_IDR` at 0, version
  0x40100 or 0x40101 (L:`drivers/gpu/drm/stm/ltdc.c:2552-2553`; M:`ltdc.c:56-57`
  the same two).
- **Layers.** The driver reads `LTDC_LCR` and clamps it to 1..4
  (L:`ltdc.c:2413-2415`); on this version layer *n* is at `n * 0x100`
  (D349:89-97), so the 0x400 window holds layers 1 to 3 [D]. Layers 1 and 2
  share RIFSC entry 119, layer 3 has 120; if 120 cannot be granted, Linux
  hides layer 3 and the CRC ("reserved for secure application",
  L:`ltdc.c:2445-2465`, `:2588-2589`). **Our image uses layer 1 only.**
- **Formats.** `LxPFCR` codes on this version: 0 ARGB8888, 1 ABGR8888,
  2 RGBA8888, 3 BGRA8888, 4 RGB565, 5 BGR565, 6 RGB888, 7 none
  (L:`ltdc.c:318-326`); the rest through the flexible-format registers
  (D349:104-115). Test pattern and GPU buffer: ARGB8888, code 0, with
  constant alpha 255 [D].
- **Stride and alignment.** `LxCFBAR` = buffer address; `LxCFBLR` = pitch in
  bytes << 16 | (bytes per line + bus width / 8 - 1); `LxCFBLNR` = lines
  (L:`ltdc.c:1850-1870`). The pitch is a 16-bit field (D349:117-126). **No
  source read states an address or pitch alignment**; Linux's DMA pool on the
  DK aligns to 8 KiB (`alignment = <0x0 0x2000>`, L:`stm32mp257f-dk-resmem.dtsi:186-193`),
  which is an allocator choice, not a hardware rule. Proposal: base 4 KiB
  aligned, pitch a multiple of 64 bytes (both 1280 and 1920 ARGB8888 lines,
  5120 and 7680 bytes, already are) [D]. The bus width (`GC2R` bits 6:4,
  D349:121) is read at the board.
- **The secure layer.** The DK's tree reserves two `no-map` areas for it,
  8 MiB `ltdc-sec-layer@fe800000` and 16 MiB `ltdc-sec-rotation@ff000000`
  (L:`stm32mp257f-dk-resmem.dtsi:175-183`; O:`stm32mp257f-dk-ca35tdcid-resmem.dtsi:132-140`).
  On the plain DK variant both are RISAF4 regions 13 and 14, **non-secure**,
  read/write for CID0 and CID1 (O:`stm32mp257f-dk-ca35tdcid-rif.dtsi:988-994`,
  `:1001`), and nothing on the DK uses them: only the EV1 board points the
  LTDC's `rotation-memory` at the second (L:`stm32mp257f-ev1.dts:580`; the
  driver reads that property, L:`ltdc.c:2819-2820`). They are the areas a
  secured variant would make secure, so **our image does not place anything
  there** [D].
- **Timing registers** for 1280x720@60 (CTA-861 VIC 4: 74.25 MHz, H 1280 /
  1390 / 1430 / 1650, V 720 / 725 / 730 / 750, positive syncs,
  L:`drivers/gpu/drm/drm_edid.c:753-756`), with Linux's conversion
  (L:`ltdc.c:1183-1209`) [D]: `SSCR` 0x00270004, `BPCR` 0x01030018,
  `AWCR` 0x060302E8, `TWCR` 0x067102ED, `LIPCR` 745, `GCR` `HSPOL | VSPOL`
  (L:`ltdc.c:1214-1218`). For 1080p60 (VIC 16, 148.5 MHz, H 1920 / 2008 /
  2052 / 2200, V 1080 / 1084 / 1089 / 1125, L:`drm_edid.c:820-823`) the same
  formulas give `SSCR` 0x002B0004, `BPCR` 0x00BF0028, `AWCR` 0x083F0460,
  `TWCR` 0x08970464, `LIPCR` 1121 [D].

### The DSI host and its PHY

- **Block.** `st,stm32mp25-dsi` at 0x48000000, 0x800 bytes, clocks `pclk`
  (`CK_BUS_DSI` 205), `ref` (`CK_KER_DSIPHY` 289), `px_clk` (`CK_KER_LTDC`),
  reset `DSI_R` (77) named `apb`, RIFSC 81, `d1_pd`; it is also a clock
  provider (`#clock-cells = <0>`, its byte clock feeds the RCC,
  L:`stm32mp255.dtsi:116`, `:123-135`). Supplies `vdd` = `scmi_vddcore`
  (BUCK2) and `vdda18` = `scmi_v1v8` (BUCK5) (L:`stm32mp257f-dk.dts:328-331`;
  the driver enables both, L:`drivers/gpu/drm/stm/dw_mipi_dsi-stm.c:1296-1322`).
  Version register `DSI_VERSION` (0) upper 24 bits = 0x313431, "1.41"
  (L:`dw_mipi_dsi-stm.c:27-29`, `:1344-1352`).
- **Lanes: 4** (`adi,dsi-lanes = <4>`, L:`stm32mp257f-dk.dts:433`; the MP25
  variant allows 4, L:`dw_mipi_dsi-stm.c:1249-1253`), 80 to 2500 Mbit/s a
  lane (`:127-128`).
- **Video mode.** What the ADV7535 asks: RGB888, `VIDEO | VIDEO_SYNC_PULSE |
  NO_EOT_PACKET | VIDEO_HSE`, virtual channel 0 (L:`bridge/adv7511/adv7533.c:162-195`),
  so non-burst with sync pulses and no EoTp; the host sequence is the
  generic DesignWare core's (D349:229-242). The DesignWare core pulses the
  `apb` reset (assert, 10-20 us, deassert) only when the host is not already
  powered up (L:`bridge/synopsys/dw-mipi-dsi.c:1213-1238`).
- **PHY PLL.** Lane rate = 2 x PLL out; PLL out = ref x NDIV / (IDF x ODF),
  IDF 1-16, NDIV 64-625, ODF from the rate table, `CCF = (ref MHz - 17) x 4`
  (L:`dw_mipi_dsi-stm.c:109-112`, `:198-207`, `:273-397`; D349:213-227).
  With the 27 MHz reference OP-TEE sets (below) [D]:

| Mode | Lanes | Lane rate | PLL out | IDF | NDIV | ODF | rate row (`hsfreq`, `vco`, `prop`) | `delay_to_lp` vs hfp (`:1209-1221`) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 720p60 | 4 | 445.5 Mbit/s | 222.75 MHz | 4 | 66 | 2 | 450 (0x16, 9, 0x0b) | HSTT(450): ceil((49+23)x4x8/2/24) = 48 <= 110: passes |
| 720p60 | 3 | 594 Mbit/s | 297 MHz | 3 | 66 | 2 | 600 (0x07, 9, 0x0b) | 46 <= 110 (D349:524) |
| 1080p60 | 4 | 891 Mbit/s | 445.5 MHz | 4 | 66 | 1 | 900 (0x29, 9, 0x0b) | 82 <= 88 (D349:519-523) |

  All three are exact (0 ppm against the 5 per mille the driver allows,
  `:1020-1021`, `:1170-1186`). `CCF` = 40.

  **The lane count at 720p is a difference between the trees.** ST's
  ADV7535 driver re-attaches the DSI device with 3 lanes when the pixel clock
  is at most 80 MHz (L:`adv7533.c:123-146`); mainline has no such fix-up and
  keeps the DT's lane count (M:`drivers/gpu/drm/bridge/adv7511/adv7533.c:99-110`,
  no `mode_fixup`). Both pass the DSI host's checks (table). The board runs
  ST's (3 lanes at 720p); the first try should therefore be 720p on 3 lanes
  (`CEC 0x1c = 3 << 4`, divider `CEC 0x16 = 4 << 3`, L:`adv7533.c:32`, `:42-43`,
  `:74`), and 4 lanes is the comparison (open question 4).
- **Mainline has no MP25 DSI.** M's `dw_mipi_dsi-stm.c` matches only
  `st,stm32-dsi` with 2 lanes (M:`drivers/gpu/drm/stm/dw_mipi_dsi-stm.c:594`,
  `:600`), and M's DK tree sends the LTDC to the LVDS bridge
  (M:`stm32mp257f-dk.dts:274-281`). So the DSI PHY 1.41 and its wrapper are
  documented only by ST's tree among the sources read [D].

### The ADV7535

- **Bus.** I2C2 (`st,stm32mp25-i2c` at 0x40130000, SPI 110, kernel clock
  `CK_KER_I2C2` 263, reset `I2C2_R` 43, RIFSC 42, analog filter,
  L:`stm32mp251.dtsi:935-951`), 400 kHz, SCL rise 108 ns and fall 12 ns
  (L:`stm32mp257f-dk.dts:395-402`), pins PF2 SCL and PF0 SDA, AF9, open
  drain, no bias (L:`stm32mp25-pinctrl.dtsi:276-284`). Shared with the camera
  at 0x1a and the touch controller at 0x41 (L:`stm32mp257f-dk.dts:407`, `:473-480`).
  Zephyr has no driver for this I2C on the A35 (plan, item ip-i2c).
- **Addresses.** `reg = <0x3d>, <0x3c>, <0x3f>, <0x38>`, names main, cec,
  edid, packet (L:`stm32mp257f-dk.dts:428-431`); the driver writes the
  ancillary addresses into the main map (`EDID_I2C_ADDR` 0x43 and the others,
  L:`adv7511.h:56-58`, `adv7511_drv.c:1282-1299`; D349:280-285). Chip
  revision register 0x00 (L:`adv7511.h:21`, read at L:`adv7511_drv.c:1265`);
  ST's U-Boot expects 0x14 there (U:`board/st/stm32mp2/stm32mp2.c:249-272`).
- **Supplies.** `avdd`, `dvdd`, `pvdd`, `a2vdd`, `v1p2` = `scmi_v1v8`;
  `v3p3` = `scmi_v3v3` (L:`stm32mp257f-dk.dts:439-444`; the names the driver
  asks for, L:`adv7511_drv.c:1007-1013`). These are **SCMI voltage domains**
  14 (`STPMIC2_BUCK5`, 1.8 V) and 16 (`STPMIC2_BUCK7`, 3.3 V)
  (L:`stm32mp257f-dk.dts:736-743`, `include/dt-bindings/regulator/st,stm32mp25-regulator.h:28`, `:30`),
  both `regulator-always-on` in OP-TEE's DK file (O:`stm32mp257f-dk.dts:169-173`,
  `:216-220`) and exported to the A35 agent (O:`stm32mp257f-dk.dts:446-454`).
  The connector's `hdmi-pwr-supply` is `scmi_v5v_hdmi`, domain 21
  (`STPMIC2_LDO5`, 5 V bypass) (L:`stm32mp257f-dk.dts:129-140`, `:752-755`;
  `st,stm32mp25-regulator.h:35`), which is **neither always-on nor boot-on**
  (O:`stm32mp257f-dk.dts:293-300`), exported to the agent
  (O:`:456-459`); OP-TEE's SCMI server marks it disabled for the agent at
  boot because only always-on and boot-on regulators start enabled
  (O:`core/lib/scmi-server/scmi_regulator_consumer.c:102-117`). D349 says
  the same ("not marked always-on or boot-on", D349:294-297).
- **Reset and power-down.** `reset-gpios = <&gpiob 6 GPIO_ACTIVE_LOW>`
  (L:`stm32mp257f-dk.dts:438`). Linux's driver never drives it: the only GPIO
  it takes is an optional `pd` (L:`adv7511_drv.c:1248`), which the DK's node
  does not have. U-Boot pulses the reset line (1 ms asserted, 10 ms released,
  U:`stm32mp2.c:186-203`) only in its bridge detection, and that runs on the
  **EV1 only** (`board_stm32mp25x_eval_init`, U:`stm32mp2.c:292-327`,
  `:622-623`); the DK's init does panel detection alone (U:`:329-354`,
  `:625-626`). So **nothing in ST's chain ever drives PB6 on a DK** [D]; our
  image pulses it like U-Boot's EV1 code before the first access.
- **Interrupt and HPD.** `interrupts = <4 IRQ_TYPE_EDGE_FALLING>` on GPIOB,
  i.e. PB4 (L:`stm32mp257f-dk.dts:436-437`). For the ADV7535 Linux sets
  `POWER2.HPD_OVERRIDE` (0xd6 bit 6) on power-on, so the HPD pin is ignored
  (L:`adv7511_drv.c:352-364`, `adv7511.h:86`, `:172`); the plug state is
  `STATUS` 0x42 bit 6 (L:`adv7511.h:55`, `:155`) and the interrupt causes
  are `INT(0)` bit 7 HPD and bit 2 EDID ready (L:`adv7511.h:71`, `:105`,
  `:108`). Linux works without the interrupt line too, polling every 25 ms
  (L:`adv7511_drv.c:513-534`): **the first probe polls; PB4 is read only as a
  level** [D].
- **EDID.** With the chip powered (`POWER` 0x41 bit 6 cleared,
  L:`adv7511.h:54`, `:124`): if `DDC_STATUS` (0xc8) is not 2, write the
  segment to `EDID_SEGMENT` (0xc4) and wait for EDID ready (200 ms), then read
  256 bytes from the EDID map (0x3f) in four 64-byte reads, register offset 0
  (L:`adv7511_drv.c:536-592`, `adv7511.h:81-82`). A sink needs the +5 V on
  the connector to answer DDC [D], so LDO5 goes on first.
- **CEC clock.** A fixed 24 MHz oscillator on the board, `clk_ext_cec`
  (L:`stm32mp257f-dk.dts:51-55`, `:434-435`): nothing to set.
- **Register sequences** (fixed patches, timing generator, lane count, power
  on/off): L:`adv7533.c:10-25`, `:27-93`; D349:299-329. The DK does not set
  `adi,disable-timing-generator`, so the timing generator is used
  (L:`adv7533.c:212-213`).

### Backlight

None on the HDMI path: the ADV7535 drives TMDS and a monitor has its own.
The DK's only backlight is the LVDS panel's `pwm-backlight` (PWM3 channel 1,
panel enable PI4, L:`stm32mp257f-dk.dts:147-184`), not on this path and not
touched. D349:434-440 agrees.

### The mode

The first picture is **1280x720@60, CTA-861 VIC 4, 74.25 MHz** (numbers
above), as the plan asks; 1080p60 (VIC 16, 148.5 MHz) is the ADV7535's
ceiling (`mode->clock <= 148500`, lane rate <= 891 Mbit/s,
L:`adv7533.c:103-121`; mainline the same, M:`adv7511_drv.c:1444-1454`) and
the second step. EDID decides only after the first picture: the probe reads
and prints it, then uses VIC 4 unless the sink lists neither VIC 4 nor 16.

## Clocks, resets, supplies, RIF and pins: who sets what

"Left set by" is what ST's chain at the tags does before U-Boot's `go`
hands over at EL2 with caches off (`tools/board-stm32mp2.sh:37-40`,
pre-study `:475-484`). "Image" is a register our image writes itself;
"SCMI" is a request to OP-TEE's server, agent 1, channel 0, over the OP-TEE
pseudo-TA (O:`stm32mp25-st-scmi-cfg.dtsi:18-22`; pre-study section 4 for the
transport). SCMI message ids: clock 0x14 `RATE_SET` 0x5, `RATE_GET` 0x6,
`CONFIG_SET` 0x7 (O:`core/drivers/scmi-msg/clock.h:22-26`); voltage domain
0x17 `CONFIG_SET` 0x5, `CONFIG_GET` 0x6 (O:`core/drivers/scmi-msg/voltage_domain.h:20-27`),
config 0x7 = on and 0 = off in architectural mode
(O:`core/lib/scmi-server/SCP-firmware/module/scmi_voltage_domain/include/internal/scmi_voltage_domain.h:81-96`).

| # | Resource | Left set by TF-A / OP-TEE / U-Boot | Set by the image | Via SCMI (protocol, id, message) |
| --- | --- | --- | --- | --- |
| 1 | LTDC registers: RIFSC 80 `cmn`, 119 `l1l2`, 120 `l3`, 121 `rot` | OP-TEE: non-secure, non-privileged, unlocked, CID filter off (O:`stm32mp257f-dk-ca35tdcid-rif.dtsi:82-85`; ids O:`core/include/dt-bindings/soc/stm32mp25-rifsc.h:85`, `:118-120`) | nothing; read `RIFSC_RISC_SECCFGRx` (0x10 + 4 x (id / 32)) and `PERx_CIDCFGR` (0x100 + 8 x id) at RIFSC 0x42080000 to confirm (O:`core/drivers/firewall/stm32_rifsc.c:25-28`, `:84`; L:`stm32mp251.dtsi:434`) | none |
| 2 | DSI registers: RIFSC 81 `cmn`, 122 `trig`, 123 `rdfifo` | OP-TEE: same (O:`...-rif.dtsi:86-88`; ids O:`stm32mp25-rifsc.h:86`, `:121-122`) | nothing; read as row 1 | none |
| 3 | I2C2 registers: RIFSC 42 | OP-TEE: same (O:`...-rif.dtsi:50`; O:`stm32mp25-rifsc.h:51`) | nothing; read as row 1 | none |
| 4 | LTDC's DMA master (RIMU) | OP-TEE: RIMU 11, 12 and 13, the only masters without a peripheral pairing, are CID0, non-secure, privileged, CID from the master (`RIF_CIDSEL_M`) (O:`...-rif.dtsi:636-638`; pairing table O:`stm32_rifsc.c:146-211`). **Which RIMU is the LTDC is not named in any source read** (open question 1) | nothing | none |
| 5 | Frame-buffer memory (RISAF4) | OP-TEE: region 9 `linuxkernel1` 0x84000000-0xfa7fffff and region 10 `gpu_reserved` 0xfa800000-0xfe7fffff non-secure, read/write CID0 and CID1 (O:`...-rif.dtsi:980-986`, `:1001`; areas O:`stm32mp257f-dk-ca35tdcid-resmem.dtsi:122-129`) | place the buffer there (section "Frame buffer") | none |
| 6 | LTDC bus + kernel clock gate, `RCC_LTDCCFGR` (0x44200840) bit 1; reset bit 0 | U-Boot: its LTDC probe enables both clocks (U:`drivers/video/stm32/stm32_ltdc.c:895-921`) and releases the reset only after a panel was found (`:962-1008`), which on a DK with no LVDS panel fails [D, D349:409-432]: gate possibly on, reset as the ROM left it | assert reset, gate on, release reset, read back (`set` asserts, L:`drivers/clk/stm32/clk-stm32mp25.c:550`, `:2329`; polarity D349:400-405). Guarded by RIFSC 80 (L:`clk-stm32mp25.c:2115`, `:2242`) | none |
| 7 | DSI gate `RCC_DSICFGR` (0x44200844) bit 1, reset bit 0; muxes `DSIBLSEL` bit 12, `DSIPHYCKREFSEL` bit 15 | OP-TEE boot: PHY ref = FLEXGEN_28, lane byte = DSI PHY (O:`stm32mp257f-dk-ca35tdcid-rcc.dtsi:105-106`); gate and reset untouched by U-Boot on a DK [D] | gate on, reset pulse; **keep the two mux bits** (L:`clk-stm32mp25.c:233-234`, `:491`, `:2330`); guarded by RIFSC 81 (`:2114`, `:2238-2239`) | none |
| 8 | Pixel-clock mux `SYSCFG_DISPLAYCLKCR` (0x44235000) = 2, `ck_ker_ltdc` | U-Boot writes 2 in its LTDC probe (U:`stm32_ltdc.c:321-323`, `:887`), if the probe ran (open question 6) | write 2 and read back (L:`drivers/clk/stm32/clk-stm32mp25-syscfg.c:13-17`, `:56-63`; Linux picks `ck_ker_ltdc` for any non-LVDS encoder, L:`ltdc.c:1114-1130`) | none |
| 9 | `FLEXGEN_27`, the LTDC kernel clock's source | OP-TEE boot: cross-bar PLL8, divider 4: 148.5 MHz (O:`stm32mp257f-dk-ca35tdcid-rcc.dtsi:64`, `:189-196`; `rate / (findiv + 1)`, O:`core/drivers/clk/clk-stm32mp25.c:2987-3019`); the channel's cross-bar is enabled (`XBARxEN`, O:`clk-stm32mp25.c:2170-2181`) but the **output enable `FINDIVxEN` is not** (`:3178-3219`) and nothing in the chain is shown to set it on a DK (open question 2). TF-A sets no flexgen 12, 27, 28 and no PLL8 (T:`fdts/stm32mp257f-dk-ca35tdcid-rcc.dtsi`, by search) | nothing: the flexgen registers are RCC resource 27, secure, CID1 (O:`...-rif.dtsi:733`) | clock 0x14, id 27 (O:`core/include/dt-bindings/clock/stm32mp25-clks.h:427`): `CONFIG_SET` enable; for 720p `RATE_SET` 74 250 000 Hz (`ALLOW_SET_RATE`, O:`stm32mp25-st-scmi-cfg.dtsi:243-249`; 594 / 8 exactly, divider search O:`clk-stm32mp25.c:3025-3068`, set only the dividers, `:3140-3176`); `RATE_GET` back |
| 10 | `FLEXGEN_28`, the DSI PHY reference | OP-TEE boot: PLL8 / 22 = 27 MHz (O:`...-rcc.dtsi:65`); output enable as row 9 | nothing (RCC resource 28 secure, O:`...-rif.dtsi:734`) | clock 0x14, id 28 (O:`stm32mp25-clks.h:428`): `CONFIG_SET` enable, `RATE_GET`; no rate change allowed (O:`stm32mp25-st-scmi-cfg.dtsi:251-256`) |
| 11 | `ICN_DISPLAY`, the display interconnect | OP-TEE: critical in the ca35tdcid build, always on (O:`clk-stm32mp25.c:3554-3556`, `:3770`) | nothing | clock 0x14, id 3: `CONFIG_SET` enable is harmless; optional (O:`stm32mp25-st-scmi-cfg.dtsi:70-75`) |
| 12 | `ICN_APB4` (LTDC, DSI APB) and `ICN_APB1` (I2C2 APB) | dividers of `ck_icn_ls_mcu`, no gate (O:`clk-stm32mp25.c:3843`, `:3846`); OP-TEE: `DIV_APB1`, `DIV_APB4` = 0 (O:`...-rcc.dtsi:29-32`) | nothing | none needed |
| 13 | I2C2 kernel clock `FLEXGEN_12` (PLL4 1200 MHz / 12 = 100 MHz) | OP-TEE boot (O:`...-rcc.dtsi:49`, `:152-158`); output enable: U-Boot's touch-panel detection on the DK reads I2C2 (U:`stm32mp2.c:223-247`, `:336`; `CONFIG_SYS_I2C_STM32F7`, U:`configs/stm32mp25_defconfig:98-99`), so it is probably on [D] | nothing (RCC resource 12 secure, O:`...-rif.dtsi:718`) | clock 0x14, id 12 (O:`stm32mp25-clks.h:412`): `CONFIG_SET` enable; `RATE_GET` for the I2C timing |
| 14 | I2C2 gate `RCC_I2C2CFGR` (0x442007A8) bit 1, reset bit 0 | U-Boot: probably on after the touch detection [D] | gate on, reset pulse (L:`clk-stm32mp25.c:514`, `:1100-1136`, `:2296`; offset L:`drivers/clk/stm32/stm32mp25_rcc.h:405`); guarded by RIFSC 42 (`:2075`, `:2150`) | none |
| 15 | GPIOB, GPIOF bus clocks | U-Boot's pin control for I2C2 (GPIOF) and its LED/button GPIOs [D]; not shown for GPIOB | nothing (an SCMI clock, L:`stm32mp251.dtsi:2778`, `:2826`) | clock 0x14, ids 90 (GPIOB) and 94 (GPIOF) (O:`stm32mp25-clks.h:489`, `:493`; O:`stm32mp25-st-scmi-cfg.dtsi:641-646`, `:669-674`): `CONFIG_SET` enable |
| 16 | Pins PF0, PF2 (I2C2 SDA, SCL): pin RIF | OP-TEE: non-secure, unlocked (O:`...-rif.dtsi:429-433`) | mode AF, AF9, open drain, no pull (`MODER` 0x00, `OTYPER` 0x04, `PUPDR` 0x0c, `AFRL` 0x20, L:`drivers/pinctrl/stm32/pinctrl-stm32.c:41-50`); GPIOF at 0x44290000 (L:`stm32mp251.dtsi:2819-2829`) | none |
| 17 | PB6 (ADV7535 reset), PB4 (its interrupt): pin RIF | OP-TEE: non-secure (O:`...-rif.dtsi:347-364`, pins 4 and 6 at `:353`, `:355`); nothing drives PB6 on a DK (above) | PB6 output, push-pull, low 1 ms, high, wait 10 ms (as U:`stm32mp2.c:186-203`); PB4 input, read `IDR` (0x10); GPIOB at 0x44250000 (L:`stm32mp251.dtsi:2771-2781`) | none |
| 18 | ADV7535 1.8 V (`BUCK5`) and 3.3 V (`BUCK7`); DSI `vdda18` (`BUCK5`), `vdd` (`BUCK2`, vddcore) | OP-TEE: always on (O:`stm32mp257f-dk.dts:106-110`, `:169-173`, `:216-220`) | nothing | voltage 0x17, ids 14, 16, 11: `CONFIG_GET` to confirm on (0x7); no set needed |
| 19 | HDMI +5 V, `LDO5` | not always on, not boot on (O:`stm32mp257f-dk.dts:293-300`); its hardware state at hand-over is the PMIC's (open question 3) | nothing | voltage 0x17, id 21: `CONFIG_GET`, then `CONFIG_SET` 0x7, `CONFIG_GET` back |
| 20 | Power domain `d1_pd` (LTDC, DSI, I2C2) | the generic `st,stm32mp-pm-domain` (L:`stm32mp251.dtsi:189-193`); no SCMI domain behind it | nothing [D] | none |
| 21 | Display interconnect QoS `SYSCFG_ICNLTDCBWLCR` (SYSCFG + 0x202C) | TF-A leaves it at its reset value; its write is commented out (T:`plat/st/stm32mp2/stm32mp2_syscfg.c:112`, `:545-561`) | nothing | none |

What changes against D349: D349 says "1080p60 needs no SCMI clock request
at all" (D349:362). The *rate* needs none, but OP-TEE's flexgen boot
configuration does not set the output enable (row 9), and OP-TEE's SCMI
server starts every exported clock as disabled for the agent ("Default OFF
for RIF issues", O:`core/lib/scmi-server/scmi_clock_consumer.c:275-280`;
`SCMI_CLOCK_DEFAULT_ENABLED` is parsed and masked, O:`:210-215`, but never
acted on in this file). So **the image enables FLEXGEN_27, FLEXGEN_28,
FLEXGEN_12, GPIOB and GPIOF through SCMI `CONFIG_SET` in every case**, and
sets the 74.25 MHz rate for 720p. OP-TEE never turns an unused clock off
(`clk_disable_unused` is behind `CFG_STM32_CLK_DISABLE_UNUSED`,
O:`core/drivers/clk/clk-stm32-core.c:708-716`, which no makefile in the tag
sets), so a clock U-Boot enabled stays on, which the state probe will show.

**The non-secure side cannot set these clocks directly.** The flexgen
dividers are RCC resources 0 to 63, all secure and CID1 on this variant
(O:`...-rif.dtsi:706-769`), and OP-TEE's `set_rate` returns success without
writing when it has no access (O:`clk-stm32mp25.c:3150-3151`). A flexgen
divider that does not settle makes OP-TEE `panic()` (`:3155-3173`): every
SCMI clock request is logged before it is sent, as the GPU's are
(`driver/rust/src/soc/stm32mp2/power.rs:24-31`).

**Our SCMI codec has no voltage-domain protocol yet.** `firmware::scmi`
defines base, power, performance, clock and reset (0x10, 0x11, 0x13, 0x14,
0x16; `driver/rust/src/firmware/scmi.rs:16-20`), not 0x17. Rows 18 and 19
need it (item soc-display). And the SCMI transport from our image has not
run on the board in this tree (`tools/probes/stm32mp257f_dk/mp2_gpu/README.md:93`,
"Not yet run on the board"; the open session question is pre-study Q3).

## The LTDC pixel clock on the MP25

The LTDC's pixel clock is the SYSCFG mux `clk_pix_ltdc`, `DISPLAYCLKCR`
bits 1:0 at SYSCFG + 0x5000: 0 = `txbyteclk` (the DSI PHY's byte clock), 1 =
`clk_pix_lvds` (the LVDS PHY's), 2 = `ck_ker_ltdc` (L:`clk-stm32mp25-syscfg.c:13-17`,
`:56-63`). For a DSI sink both Linux and U-Boot select **2**, the RCC path
(L:`ltdc.c:1122-1130`; U:`stm32_ltdc.c:887`, `:1013-1021`), not the DSI's
byte clock. `ck_ker_ltdc` is the gate of `RCC_LTDCCFGR` on `FLEXGEN_27`
(L:`clk-stm32mp25.c:897-907`; O:`clk-stm32mp25.c:4171`), and `FLEXGEN_27`
is PLL8 (594 MHz, `cfg = <297 5 1 4>` from the 40 MHz HSE,
O:`stm32mp257f-dk-ca35tdcid-rcc.dtsi:7`, `:189-196`) divided by
`findiv + 1` (and a pre-divider 1, 2, 4 or 1024). Linux sets the rate on the
mux, which passes it to `FLEXGEN_27` (`CLK_SET_RATE_PARENT`,
L:`clk-stm32mp25-syscfg.c:56-59`, `ltdc.c:1132`), and for the non-secure side
that is SCMI `CLOCK_RATE_SET` on clock 27, the one display clock with
`ALLOW_SET_RATE` (row 9). Exact rates from PLL8 [D]: 148.5 (/4), 99 (/6),
74.25 (/8), 59.4 (/10) MHz. The DSI's PHY reference is separate:
`CK_KER_DSIPHY`, `FLEXGEN_28` (27 MHz) or the HSE (40 MHz) by
`DSIPHYCKREFSEL` (L:`clk-stm32mp25.c:166-175`; D349:191-211).

## Frame buffer placement

Our image is linked at 0xa0000000 and owns 256 MiB as `zephyr,sram`
(`driver/boards/st/stm32mp257f_dk/stm32mp257f_dk_stm32mp257fxx_a35.dts:22-24`,
`:44-53`); the GPU's 64 MiB carveout is `no-map` at 0xfa800000 (`:55-68`).

**Proposal.**

1. **The test pattern (mp2_display): a new `no-map` reserved area at
   0xb0000000, 32 MiB**, directly above `zephyr,sram` (0xa0000000 +
   256 MiB), declared in the probe's own overlay, not in the board file
   (which plan #349 owns). It is inside System RAM and RISAF4 region 9,
   non-secure and read/write for CID0 and CID1 (row 5), so the LTDC master
   reaches it if it carries CID0 or CID1 (row 4). It holds two 1080p ARGB8888
   buffers (2 x 8 294 400 bytes) or four 720p ones [D]. Nothing of ST's chain
   uses it after `go`: U-Boot's load addresses are below 0x90500000 and U-Boot
   relocates to the top of usable RAM (board file `:13-24`); TF-A and OP-TEE
   live below 0x84000000 (pre-study `:623-628`). The image maps it
   non-cacheable (or write-combine), like the GPU carveout (pre-study
   `:638-641`), so the CPU's pattern needs no cache clean before the LTDC
   reads it; if it is mapped cacheable instead, clean to the point of
   coherency before the reload.
2. **The GPU-rendered frame (item gpu-scanout): scan out the render target
   where the GPU wrote it, inside the carveout** (RISAF4 region 10, the same
   CID0 and CID1 access, row 5), by pointing `L1CFBAR` at it: no copy. The
   carveout is below 4 GiB, so the LTDC's 32-bit `CFBAR` reaches it [D].
   Alignment: as above (4 KiB base, 64-byte pitch); the Vivante side's own
   resolve alignment is the stricter one and comes from item gpu-scanout.
3. **Not** the two LTDC secure-layer areas at 0xfe800000 and 0xff000000
   (see "The secure layer"), not inside `zephyr,sram` (the image's own heap
   and stacks; also cacheable).

## Framebuffer memory and bus-master access

Why it matters: the LTDC (and later the GPU) reads and writes DDR as a bus
master. A master passes two gates, the RIF master configuration in RIFSC
(RIMU: security, privilege and CID it presents) and the DDR firewall RISAF4
(regions that name the CIDs allowed to read and to write). A region that does
not grant the master's CID fails the access with no error at the LTDC; the
screen stays black. Citations as in "Sources" (O = ST OP-TEE `core/`, T = ST
TF-A). ST's Linux and U-Boot trees were **not** re-read for this section (the
sparse checkouts of this pass were refused); the L and U facts reused here
are the ones cited above. RM0457's RIMU and RISAF chapters are unread, so the
RIMU-to-master mapping of the LTDC is **not** in any source (item 1).

### 1. RIMU settings (who the LTDC and the GPU are on the bus)

- **Fields.** OP-TEE writes one `RIFSC_RIMC_ATTRx` per `st,rimu` entry at
  RIFSC + 0xC10 + 4 x n: bit 2 `CIDSEL` (0 = the CID comes from the master's
  peripheral entry, "RISUP", 1 = from the RIMU's own `MCID`), bits 6:4
  `MCID`, bit 8 `MSEC`, bit 9 `MPRIV` (O:`drivers/firewall/stm32_rifsc.c:30-31`,
  `:102-107`, `:618-632`; DT macro O:`include/dt-bindings/soc/stm32mp25-rifsc.h:151-172`,
  `RIF_CIDSEL_P` = config from RISUP, `RIF_CIDSEL_M` = config from RIMU).
- **GPU = RIMU 9**, paired with the GPU's RISUP entry
  (O:`stm32_rifsc.c:183-186`, pair table `:146-211`). The DK file programs
  it `RIF_UNUSED` CID, **non-secure, privileged, `CIDSEL_P`** (inherit)
  (O:`dts/stm32mp257f-dk-ca35tdcid-rif.dtsi:634`). Its RISUP entry (RIFSC 79)
  is non-secure, non-privileged, unlocked, **CID filter off**
  (`RIF_CFDIS`, `:81`). With the filter off OP-TEE's own check treats the
  inherited CID as CID0 ("inheritance mode with CID0", O:`stm32_rifsc.c:596-612`;
  that check runs only with `st,errata-ahbrisab`, which no DK file sets), so
  the GPU reaches RISAF4 as **CID0, non-secure** [D].
- **LTDC = not named.** RIMU 11, 12, 13 are the only entries without a
  RISUP pairing (`risup_id = 0`, O:`stm32_rifsc.c:191-202`); the DK file sets
  all three to **CID0, non-secure, privileged, `CIDSEL_M`** (O:`...-rif.dtsi:636-638`).
  The LTDC has three RIFSC entries `l1l2`, `l3`, `rot` (RIFSC 119 to 121,
  O:`...-rif.dtsi:83-85`) and no pairing of its own, so the three
  unpaired RIMUs are the likely LTDC masters [D, unconfirmed]. All three
  present CID0 and non-secure; nothing in TF-A touches a display RIMU (TF-A
  programs only RIMU 4, USB3DR, and only for USB boot: T:`plat/st/stm32mp2/bl2_plat_setup.c:612-625`,
  `stm32mp2_def.h:745-749`). RIMU 0 (CID1, non-secure, privileged, `CIDSEL_M`,
  `...-rif.dtsi:625`) has no peripheral either and may be a different master
  (open question: board item 1 below).
- **What the image must do.** Nothing: the RIFSC grants of the DK file are
  already the ones that let CID0 masters through; our image runs at EL2
  non-secure and cannot write RIMU anyway (RIFSC RIMC is secure-written by
  OP-TEE at its boot). The image only reads them back.

### 2. RISAF4 on the DK's DDR

- **Instance.** RISAF4 at 0x420D0000 (T:`stm32mp2_def.h:699`;
  O:`dts/stm32mp251.dtsi:748-754`), covers `st,mem-map` 0x80000000 + 4 GiB
  (`:752`), the whole 4 GiB DDR of the DK (O:`dts/stm32mp257f-dk.dts:35-37`).
  Region n sits at RISAF4 + 0x40 + 0x40 x n: `CFGR` (+0, `BREN` bit 0,
  `SEC` bit 8, `ENC` bit 15, `PRIVC` bits 23:16), `STARTR` (+4) and `ENDR`
  (+8) as offsets from 0x80000000, `CIDCFGR` (+0xC, read CIDs bits 7:0, write
  CIDs bits 23:16) (O:`drivers/firewall/stm32_risaf.c:66-79`, `:98-128`,
  `:540-551`). A CID `c` may read when bit `c` of `CIDCFGR[7:0]` is set, and
  write when bit `16 + c` is set (`:125-128`).
- **Regions of the DK (O:`...-rif.dtsi:905-1001`, areas O:`...-resmem.dtsi`).**

  | Region | Area (resmem) | Range | Sec | Read CIDs | Write CIDs | Note |
  | --- | --- | --- | --- | --- | --- | --- |
  | 1 | `tfm_code` (`:82`) | 0x80000000, 1 MiB | S | 2 | 2 | enc. (`:949`) |
  | 2 | `cm33_cube_fw` | 0x80100000, 8 MiB | NS | 0,1,2 | 0,1,2 | (`:953`) |
  | 3 | `tfm_data` | 0x80900000, 1 MiB | S | 2 | 2 | (`:957`) |
  | 4 | `cm33_cube_data` | 0x80a00000, 8 MiB | NS | 0,1,2 | 0,1,2 | (`:961`) |
  | 5 | `ipc_shmem` (`:102`) | 0x81200000, 1 MiB | NS | 0,1,2 | 0,1,2 | (`:965`) |
  | 6 | `spare1` (`:107`) | 0x81300000, 0xcc0000 | NS | 0,1,2 | 0,1,2 | (`:969`) |
  | 7 | `bl31_context` (`:112`) | 0x81fc0000, 256 KiB | S | 0,1 | 0,1 | enc. (`:973`) |
  | 8 | `op_tee` (`:117`) | 0x82000000, 32 MiB | S | 0,1 | 0,1 | enc. (`:977`) |
  | 9 | `linuxkernel1` (`:122`) | **0x84000000, 0x76800000 (to 0xfa7fffff)** | **NS** | **0,1** | **0,1** | (`:981`) |
  | 10 | `gpu_reserved` (`:127`) | **0xfa800000, 64 MiB (to 0xfe7fffff)** | **NS** | **0,1** | **0,1** | (`:985`) |
  | 13 | `ltdc_sec_layer` (`:132`) | 0xfe800000, 8 MiB | **NS** | 0,1 | 0,1 | (`:989`) |
  | 14 | `ltdc_sec_rotation` (`:137`) | 0xff000000, 16 MiB | **NS** | 0,1 | 0,1 | (`:993`) |
  | 15 | `linuxkernel2` (`:142`) | 0x100000000, 2 GiB | NS | 0,1 | 0,1 | (`:997`) |

  The `PRIVC` list is `RIF_UNUSED` (0) for every non-secure region, so
  non-privileged and privileged accesses both pass (O:`stm32mp25-risaf.h:62-69`,
  the `cid_priv_list` argument). The region table is applied by
  `&risaf4 { memory-region = ... }` (O:`...-rif.dtsi:1000-1001`). The areas are
  contiguous from 0x80000000 to 0xffffffff and 0x100000000 to 0x17fffffff,
  so no part of the DDR is left to the reset default [D]; the RISAF's
  behaviour for an address outside every enabled region is RM0457's and is
  not asserted here.
- **Consequence.** A master with **CID0 or CID1, non-secure**, reads and
  writes everything from 0x84000000 up (regions 9, 10, 13, 14, 15). CID2 (the
  M33) is not allowed there. The GPU is CID0 (item 1) and the LTDC, if it is
  RIMU 11 to 13, is CID0: both pass. If the LTDC's CID were anything but 0
  or 1 (for example if RIMU 11-13 are not its masters and the board resets
  them to a different CID), it fails at every address of the table; the
  state probe's RIMC read (board item 1) decides.
- The area below 0x84000000 is OP-TEE's and the M33's (regions 1 to 8), CID2,
  secure or encrypted: **never** a framebuffer. Internal RAM (RISAB 3/4/5)
  is not considered: TF-A and OP-TEE keep BL31 and the SCMI buffers there
  (O:`...-resmem.dtsi:12-45`), and the RISAB errata forbids CID0 masters on
  RISAB 3/4/5 once CID filtering is on there (O:`stm32_rifsc.c:546-552`).

### 3. The two LTDC secure-layer areas at 0xfe800000

- **What they are.** 8 MiB `ltdc-sec-layer@fe800000` and 16 MiB
  `ltdc-sec-rotation@ff000000`, `no-map`, directly above the GPU carveout
  and the last 24 MiB below 4 GiB (O:`...-resmem.dtsi:132-140`; the same in
  ST's Linux tree, `stm32mp257f-dk-resmem.dtsi:175-183`, from the section
  "The secure layer" above). RISAF4 regions 13 and 14 (table above).
- **On this DK variant they are not secure.** OP-TEE's DK file marks both
  `RIF_NSEC`, CID0 and CID1 read/write (O:`...-rif.dtsi:989`, `:993`), so a
  non-secure CID0 master would pass. The reasons the framebuffer still stays
  out [D]: (a) they are the areas ST's configuration reserves for a
  *secured* LTDC layer 3 and a rotation buffer; Linux hides layer 3
  as "reserved for secure application" when RIFSC 120 is not granted
  (section "The LTDC", L:`ltdc.c:2445-2465`), and on any other variant of
  OP-TEE's DT (secure or CID-filtered) regions 13 and 14 become secure or
  CID-limited, so an image that uses them works only on this DK file;
  (b) the EV1 board points the LTDC's `rotation-memory` at the second area
  (L:`stm32mp257f-ev1.dts:580`), so the area is owned by a driver feature
  we do not use but ST's tree would, and a later Linux boot of the same
  board assumes it is free; (c) they are only 24 MiB and are the top of the
  map, where U-Boot relocates (board file `:13-24`), while the region 9
  range above is large and free. None of these is a hardware measurement.

### 4. Where the image lives and the proposed framebuffer

| Range | What | Source |
| --- | --- | --- |
| 0x84000000 - 0x9fffffff | System RAM, U-Boot's load addresses (kernel 0x84000000/0x8a000000, FDT 0x90000000, ramdisk 0x90400000) | `driver/boards/st/stm32mp257f_dk/stm32mp257f_dk_stm32mp257fxx_a35.dts:10-24`, `:36-41` |
| 0xa0000000 - 0xaFFFFFFF | **our image**, `zephyr,sram`, 256 MiB, link and load address | board `.dts:22-24`, `:44-47`; `tools/board-stm32mp2.sh:189-190`, `:449-456` (image footprint is rejected above `LOAD_END` 0xb0000000) |
| 0xb0000000 - 0xfa7fffff | free System RAM (RISAF4 region 9) | this section |
| 0xfa800000 - 0xfe7fffff | GPU carveout, `no-map` (region 10) | board `.dts:55-68` |
| 0xfe800000 - 0xffffffff | LTDC secure-layer areas (regions 13, 14), not used | above |

**Proposal.**

- **LTDC test pattern and display buffers:** base **0xb0000000**, 32 MiB
  reserved, declared `no-map` in the probe's own overlay (not the board file).
  It is directly past `LOAD_END`, so the load script can never overlap it,
  inside region 9 (CID0/1, non-secure, read/write), clear of U-Boot's low
  load addresses and of its relocation at the top of RAM (board file `:13-24`),
  and below 4 GiB, so the LTDC's 32-bit `CFBAR` reaches it [D].
  - 1280x720 XRGB8888: pitch 5120 B, 3 686 400 B (0x384000) per frame:
    frame 0 at 0xb0000000, frame 1 at 0xb0400000.
  - 1920x1080 XRGB8888: pitch 7680 B, 8 294 400 B (0x7E9000) per frame:
    frame 0 at 0xb0000000, frame 1 at 0xb0800000 (8 MiB steps; each fits
    with 94 208 B to spare).
  - Base is 4 KiB aligned and each pitch a multiple of 64 B, the stricter of
    the two alignments the repository already asserts for the GPU side
    (`driver/samples/zephyr/gc_triangle/src/main.c:272-276`); the LTDC's own
    requirement is not in any source read (RM0457).
- **GPU-rendered frame (gc_triangle's target):** leave it in the carveout
  (region 10, same access) and point `L1CFBAR` at the address
  gc_triangle prints (`gc_triangle: linear: ... phys 0x...`,
  `gc_triangle/src/main.c:38`), as `CONFIG_GC_TRIANGLE_LINEAR_OUT_ADDR` = 0
  does; or, to keep the LTDC's pitch fixed, set that option to 0xb0000000 (it
  imports a 64-byte aligned, below-4-GiB, non-secure address, and
  `CONFIG_GC_TRIANGLE_LINEAR_OUT_PITCH` must be a multiple of 64; Kconfig
  `:57-66`). Both ranges are readable and writable for the GPU and the
  LTDC by the table. The carveout is the better choice: the GPU driver already
  maps it, and an address in it must lie past what the pool hands out
  (Kconfig `:64-66`).
- **Not** inside `zephyr,sram` (cacheable, the heap), not 0xfe800000 and up.

**Cache maintenance.** The LTDC and the GPU are not cache-coherent with the
A35 in any source read (pre-study Q14, "Coherence is in no tree",
`docs/notes/pre-study-stm32mp257f-dk.md:640-641`). So:
(1) Map the framebuffer **non-cacheable** (`K_MEM_CACHE_NONE`, as every other
device or carveout mapping of this image, e.g. `gpu_vivante_gc_rust.c:231`,
`tools/probes/stm32mp257f_dk/mp2_gpu/src/main.c:88`): the CPU then writes
straight to DDR, a `dsb sy` before the LTDC is enabled or the address
register reloaded is the only barrier needed. (2) If it is mapped cacheable
(the default for RAM under `zephyr,sram`), the CPU must clean the written
range to the point of coherency (`sys_cache_data_flush_range`, `dc cvac` +
`dsb sy`) before the reload, and must not write the buffer again while the
LTDC scans it without repeating that. (3) After the GPU writes a frame the
LTDC needs no maintenance (it reads DDR), but the CPU must **invalidate**
(`dc ivac`, `sys_cache_data_invd_range`) before it reads the GPU's frame, as
gc_triangle does (`gc_render_cache_flush`, `main.c:313`). (4) No stale dirty
line may exist for the buffer before the first LTDC read: U-Boot hands over
with caches off (`tools/board-stm32mp2.sh:37-40`), so nothing of ours is dirty
at entry; the first frame written non-cacheable never is.

### 5. What only the board can confirm (registers state-probe reads)

All read-only, from EL2 non-secure; any read that bus-errors is itself the
answer (the entry is secure to us).

| # | Read | Answer |
| --- | --- | --- |
| 1 | `RIFSC_RIMC_ATTRn` = RIFSC 0x42080000 + 0xC10 + 4n, for n = 0, 9, 11, 12, 13 (and all 16) | `CIDSEL` bit 2, `MCID` bits 6:4, `MSEC` bit 8, `MPRIV` bit 9. Expected: n = 9 `CIDSEL` 0, MSEC 0; n = 11 to 13 `CIDSEL` 1, MCID 0, MSEC 0, MPRIV 1 (the DK file). Which of 11 to 13 is the LTDC: the one whose value changes when the LTDC reads (compare before and after the first frame), or RM0457 |
| 2 | `RIFSC_RIMC_CR` (RIFSC + 0xC00) | `GLOCK` bit 0, `TDCID` bits 6:4 (O:`stm32_rifsc.c:30`, `:73-74`) |
| 3 | `PERx_CIDCFGR` = RIFSC + 0x100 + 8x for x = 79 (GPU), 80, 119, 120, 121 (LTDC) | `CFEN` 0 expected, so inherited CID = 0 |
| 4 | `RISAF4_REGn_CFGR/STARTR/ENDR/CIDCFGR` = 0x420D0000 + 0x40 + 0x40 n + {0, 4, 8, 0xC} for n = 9, 10, 13, 14, 15 (and 1 to 8) | `BREN`, `SEC` 0 for the non-secure ones; start/end as the table (offset from 0x80000000); `CIDCFGR` 0x00030003 (read 0,1; write 0,1). A bus error means RISAF4's register page is secure to us (then only SCMI/OP-TEE can say) |
| 5 | `RISAF4_SR` (+0x04), `_IASR` (+0x08), `_IAESR0/1` (+0x20, +0x28), `_IADDR0/1` (+0x24, +0x2C) | after the first LTDC frame: an illegal-access record names the failing address and CID (O:`stm32_risaf.c:26-32`) |
| 6 | `LTDC_L1CFBAR` / `_CFBLR` / `_CFBLNR` readback, `LTDC_CDSR`, and the LTDC's FIFO underrun interrupt flag | the buffer address landed; a DMA that is denied shows as an underrun or a constant colour [D] |
| 7 | `GPU` (RIFSC 79) entry and the first GPU write | gc_triangle's `phys` readback verifies the GPU side; the same RISAF4 record (item 5) names a denied GPU write |
| 8 | the image's own loader: `md.l 0xb0000000` from U-Boot (non-secure, CID0 read) | reads back what a CPU write left; shows the region is real DDR and not a fault |

## Open questions for the board

Each is a question with what answers it; the state probe (item state-probe)
answers most of them by reading, before anything is written.

1. **Which RIMU entry is the LTDC's master, and its CID.** The DK file sets
   RIMU 11 to 13 to CID0, non-secure, master-chosen CID (row 4) and names
   none of them; the LTDC has three RIF-guarded parts (`l1l2`, `l3`, `rot`),
   which fits three masters but is an inference [D]. RM0457's RIMU table;
   on the board, `RIFSC_RIMC_ATTRx` (RIFSC + 0xC10 + 4 x n,
   O:`stm32_rifsc.c:30-31`, fields `:102-107`). If the LTDC is CID0 or CID1,
   both proposed buffers pass RISAF4. D349's open question 2 is the same.
2. **Are FLEXGEN_27, 28 and 12 running at hand-over?** Read `FINDIVxCFGR`
   (RCC + 0x1224 + 4 x n, bit `FINDIVxEN`, L:`stm32mp25_rcc.h:601`) for
   n = 12, 27, 28, and the SCMI `CLOCK_ATTRIBUTES` and `RATE_GET` replies.
   The plan above enables them regardless.
3. **Is LDO5 on at hand-over, and what does `st,pwrctrl-sel = <0>` with
   `st,pwrctrl-enable` mean for it** (O:`stm32mp257f-dk.dts:298-299`; the
   PMIC's PWRCTRL handling, O:`core/drivers/stpmic2.c:509-529`)? SCMI
   voltage `CONFIG_GET` on domain 21 answers the first half; the STPMIC25
   datasheet the second.
4. **720p on 3 or 4 lanes.** ST's driver drops to 3 lanes below 80 MHz
   (L:`adv7533.c:123-146`), mainline keeps 4. Whether the ADV7535 locks on 4
   lanes at 445.5 Mbit/s is a board test (try 3 first, then 4).
5. **The ADV7535's state with PB6 never driven.** Does 0x3d answer before our
   reset pulse (the pad's reset state and the board's pull, MB1605)? The
   probe reads 0x00 once before and once after the pulse.
6. **What U-Boot left of the LTDC.** Whether ST's U-Boot probes the LTDC on a
   DK at all (`DM_FLAG_OS_PREPARE`, U:`stm32_ltdc.c:1121`; video is in the
   defconfig, U:`configs/stm32mp25_defconfig:168-178`) and so leaves the gate
   on, `DISPLAYCLKCR` = 2 and the RIFSC grants taken: read `RCC_LTDCCFGR`,
   `RCC_DSICFGR`, `RCC_I2C2CFGR`, `DISPLAYCLKCR`, `LTDC_IDR` (only if the gate
   is on), the RIFSC entries of rows 1 to 3. D349's open question 4.
7. **Can the non-secure side write SYSCFG + 0x5000?** Linux does (through a
   syscon regmap), so yes on this variant [D]; RM0457 says which SYSCFG
   registers are secure-only. Read it back after the write.
8. **The LTDC's bus width, and which IRQ line carries which flags.**
   `GC2R` bits 6:4 at the board; RM0457 for the IRQ split (D349:158-164).
9. **Frame buffer alignment.** No source states one (above). RM0457's LTDC
   chapter.
10. **The SCMI session from a non-Linux agent** (pre-study Q3, `:818`): the
    display needs it for rows 9 to 19. The GPU's first board run answers it.
11. **I2C2's timing.** The kernel clock is 100 MHz [D, row 13]; `TIMINGR` for
    400 kHz with 108 ns rise and 12 ns fall is computed by item ip-i2c from
    the I2C chapter of RM0457 or ST's driver (`drivers/i2c/busses/i2c-stm32f7.c`);
    a scope or a first transfer confirms it. The camera (0x1a) and touch
    controller (0x41) on the same bus are left alone.
12. **RM0457, UM3385, MB1605.** Unread (see "Sources"); every offset and bit
    above waits for RM0457, the supplies and the HPD wiring for MB1605.

## Corrections and additions to the pre-study and to D349 (not made there)

- Pre-study `:219-222` ("the DK board file does not enable" the LVDS
  bridge): ST's tree enables it (L:`stm32mp257f-dk.dts:555-578`); D349:546-551
  found the same. U-Boot disables it again without a panel
  (U:`stm32mp2.c:342-349`, `:739`).
- Pre-study Q7 (`:822`): answered above, except the items in "Open questions".
- D349:362 and its step 1 (D349:456-460): the FLEXGEN_27 and FLEXGEN_28
  *enables* go through SCMI even when the rate is kept (rows 9 and 10), and
  the I2C2 kernel clock and the GPIOB/GPIOF bus clocks are SCMI clocks too
  (rows 13 and 15).
- D349:461 ("Pulse PB6"): right, and the reason is that no part of ST's
  chain drives PB6 on a DK: U-Boot's ADV753x detection is EV1-only
  (U:`stm32mp2.c:622-626`).
