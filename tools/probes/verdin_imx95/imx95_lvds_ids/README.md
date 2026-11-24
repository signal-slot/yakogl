<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# imx95_lvds_ids

**Question.** Is the X12 LVDS path there, and what do its parts hold before
the cold run writes them? What AP-NS may do with the LDB PLL (37, 38) and
TPM5's clock (154, the backlight's PWM) and what they run at when U-Boot
hands over; the LVDS CSR's four words, the display CSR's nine and the DPU
words the cold path's defaults and Linux disagree on, as reset leaves them
(display note, `driver/docs/imx95-display.md` sections 4(b') and 6; U22,
U23, U26, U27). Read only, with one SCMI write: DISPLAY on, and back off.

**What it does.**
1. SCMI, read only: DISPLAY's (13) state, the DISP level (12), and for
   LDBPLL_VCO (37), LDBPLL (38), DISP1PIX (80), TPM5 (154) and DISPAPB (75)
   CLOCK_ATTRIBUTES (on or off), CLOCK_RATE_GET and CLOCK_GET_PERMISSIONS
   (state, parent, rate).
2. DISPLAY on over SCMI (POWER_STATE_SET, read back); the LDB PLL is not
   touched.
3. One 32-bit read each: the LVDS CSR 0x4B0C0000-0x0C; the display CSR
   0x4B010000-0x20; the DPU's `COMCTRL_IPIDENTIFIER`, FetchLayer0
   `BURSTBUFFERMANAGEMENT` (+0x1D0010) and `LAYERPROPERTY0` (+0x1D0048),
   FetchYUV0 `BURSTBUFFERMANAGEMENT` (+0x20000C), FrameGen0 `SKICKCONFIG`,
   `SECSTATCONFIG`, `FGINCTRLPANIC`, `FGCCR` (+0x2B0030, 34, 84, 88), Dither0
   `POLARITYCTRL0` (+0x311008); the interleaver's channel 0 CTRL. No
   write-only or reserved word (the allow list's `#@noread`: those faulted
   on the board under Linux). TPM5 and GPIO5 are not read: their clocks may
   be off.
4. DISPLAY off again if it was off.

**Expected lines** (illustrative; the LVDS CSR's reset values are not
known, the dump's running values are 0x14, 0x00082221, 0x1075, 0):

```
imx95_lvds_ids: before pd 13 state = 0x40000000 (rc 0)
imx95_lvds_ids: perf 12 level = 3 (rc 1, SUCCESS)
imx95_lvds_ids: clk 37 (LDBPLL_VCO) = off, 0 Hz, permissions 0xe0000000 (state parent rate) [rc 5 2 1, SUCCESS SUCCESS SUCCESS]
imx95_lvds_ids: clk 154 (TPM5) = off, 24000000 Hz, permissions 0x...
imx95_lvds_ids: pd 13 set 0x00000000, reads 0x00000000 (rc 0)
imx95_lvds_ids: ldb 0x4b0c0000 = 0x...
imx95_lvds_ids: dpu 0x4b400000 = 0x21260400
imx95_lvds_ids: lvds csr = 0x... 0x... 0x... 0x...
imx95_lvds_ids: pd 13 set 0x40000000, reads 0x40000000 (rc 0)
imx95_lvds_ids: PASS lvds csr 0x... 0x... 0x... 0x..., dpu id 0x21260400 (the dump's), LDB PLL and TPM5 permitted
```

**Reading it.**
- `clk 37`/`38` permissions without `state` and `rate`: the cold path's
  CLOCK_RATE_SET would be DENIED: a System Manager configuration question
  for the owner (the design note's section 1). `clk 154` without `state`:
  the backlight's clock cannot be enabled by us (U26).
- `dpu ... 0x4b5d0010`: 0x1FE7F010 makes the dump's burst word FetchLayer0's
  reset value (U22); `dpu ... 0x4b711008` is POLARITYCTRL0 at reset (U23).
- `csr 0x4b010014`, `0x4b010018`, `0x4b01001c` equal to the dump's
  (0x1A150A00, 0x00330303, 0x33): reset values; different: Linux writes
  them (U27).
- `dpu id` not the dump's 0x21260400: another DPU revision than the board's
  Linux ran on; stop and report.
- `DISPLAY did not come on`: the `pd ... set` line has the SCMI status.

**Build**: `west build -p always -b verdin_imx95/mimx9596/a55 -DYAKOGL_WERROR=ON -d build/imx95_lvds_ids yakogl/tools/probes/verdin_imx95/imx95_lvds_ids`.
