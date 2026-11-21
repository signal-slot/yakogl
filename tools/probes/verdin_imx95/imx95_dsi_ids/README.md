<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# imx95_dsi_ids

**Question.** Is the X4 display path there, and what are its parts, with
nothing started: the DW MIPI DSI host's `VERSION` (U18), the combo PHY CSR
words the note's M4 names (U13 `COMBO_PHY_FREQ_CONTROL`, U16
`COMBO_PHY_MODE_CONTROL` and `TEST_MODE_CONTROL`) as the hardware holds
them before anything writes them, the module's PCAL6416A answering on LPI2C2
(U19), and how long the SN65DSI83 takes to answer on I2C after
`GPIO_10_DSI` rises (U21)? Display note, sections 6 and 7.

**What it does.**
1. DISPLAY (13) and CAMERA (3) on over SCMI (POWER_STATE_SET, read back).
2. One 32-bit read each: the DW host's VERSION, PWR_UP, CLKMGR_CFG,
   DPI_VCID, DPI_COLOR_CODING, MODE_CFG, VID_MODE_CFG, PHY_RSTZ, PHY_IF_CFG,
   PHY_STATUS; the DSI stream CSR 0x4AD00000-0x0C; the master CSR
   0x4AD10000-0x08; the combo PHY CSR 0x4AD20100-0x110. Not read:
   GEN_PLD_DATA and INT_ST0/1 (they change on read), the DPU (U6).
3. LPI2C2 and LPI2C3 clocked over SCMI (Zephyr's clock driver) and
   initialised; the PCAL6416A's eight registers read raw at 0x20 (it has no
   ID register: answering is its identity); its Zephyr driver brought up.
4. `GPIO_10_DSI` (line 3) low 20 ms, then high; the bridge's ID registers
   0x00-0x08 read every 5 ms for at most 1 s; `GPIO_10_DSI` low again.
5. Each domain that was off is turned off again.

**Expected lines** (illustrative):

```
imx95_dsi_ids: before pd 13 state = 0x40000000 (rc 0)
imx95_dsi_ids: pd 13 set 0x00000000, reads 0x00000000 (rc 0)
imx95_dsi_ids: dsi 0x4acf0000 = 0x3133302a
imx95_dsi_ids: dsi version = 0x3133302a
imx95_dsi_ids: phy 0x4ad20104 = 0x0016001c
imx95_dsi_ids: expander id = answers at 0x20: input 0x.... output 0x.... polarity 0x0000 config 0x....
imx95_dsi_ids: bridge id = 35 38 49 53 44 20 20 20 01 (SN65DSI83)
imx95_dsi_ids: bridge answered = after 55 ms, try 12 (every 5 ms)
imx95_dsi_ids: PASS dsi version 0x3133302a, expander answers, bridge SN65DSI83 after 55 ms
```

**Reading it.**
- `dsi version`: `ip::dw_mipi_dsi::probe` accepts 1.30, 1.31 and 1.41
  (`0x3133302a` is 1.30a); another one is U18's answer and the driver's
  version list grows.
- The `phy` words are the reset values (nothing wrote them): compare with
  Linux's under a running panel (`linux_day_one`'s M4) for U13 and U16.
- `expander id = no answer`: LPI2C2 or its pins (`LPI2C2 ... up:` line), or
  the SM refused the clock (U19's open half).
- `bridge id = no answer ... in 200 tries`: EN does not reach the bridge, the
  display adapter is not powered, or LPI2C3's pins; `bridge answered = after
  N ms` is U21's number (`ip::sn65dsi8x` retries for 1 s).
- `the domains did not come on`: the `pd ... set` line has the SCMI status
  (DENIED: compare `imx95_scmi`'s permissions).

**Build**: `west build -p always -b verdin_imx95/mimx9596/a55 -DYAKOGL_WERROR=ON -d build/imx95_dsi_ids yakogl/tools/probes/verdin_imx95/imx95_dsi_ids`.
