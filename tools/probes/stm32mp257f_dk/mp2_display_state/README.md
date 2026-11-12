<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2_display_state

A read-only probe for the STM32MP257F-DK's Cortex-A35 (issues #7 and #1281,
P8 of `docs/notes/pre-study-stm32mp257f-dk.md`, plan #353). It answers, from
the board, what ST's boot chain (TF-A, OP-TEE, U-Boot) leaves set of the
display path LTDC -> DSI -> ADV7535 -> HDMI before our image writes anything:
clock gates, resets, the pixel-clock mux, the RIF settings that decide whether
the non-secure A35 may touch the blocks, and the pins of the I2C2 and the
ADV7535. The board layer (`driver/zephyr/lib/board/stm32mp2/`) prints the
banner and the hand-over record, arms OP-TEE's watchdog
(`CONFIG_YAKOGL_STM32MP2_WDOG`) and resets back into ST's Linux through PSCI
at the end, as for `mp2_identify`.

**It only reads.** The one accessor is `sys_read32()` in `rd()`
(`src/main.c`); the source has no `sys_write*`, no `write32`, no store to a
mapped register. The pages are mapped with `device_map()`.

## What it prints

One line per register:

```
mp2_display_state: <block> <name> 0x<addr> = 0x<val>
```

`<addr>` is the physical address read. Markers between them:

- `mp2_display_state: block <name> begin`: a block starts. If the board then
  resets with a fatal-error report from the board layer, that block's read
  faulted: it is the last `begin` line, and there is no PASS line.
- `mp2_display_state: block <name> SKIPPED: <why>`: not read, with the reason.
- `mp2_display_state: <block> id <n> secure-only (RISC_SECCFGR)`, `static CID
  <c>, not 1`, `compartment semaphore mode`: why RIFSC closes a block to the
  non-secure A35 (CID 1). `privileged-only` is only noted (EL1 is privileged).
- `mp2_display_state: ltdc idr ... is neither ...`: the LTDC's identity is not
  `HWVER_40100`/`40101` (a finding, not a failure).
- The end: `mp2_display_state: PASS: <n> registers read`, or
  `mp2_display_state: FAIL: no register read`, then the layer's
  `mp2: mp2_display_state done: PSCI SYSTEM_RESET at ... ms`.

## Blocks read, in order

Chapter names are RM0457's; **RM0457 was not available when this was written,
so no section number is claimed**, and every offset is from ST's trees as
`docs/notes/stm32mp257f-dk-display.md` cites them (L = ST Linux
`v6.6-stm32mp-r3.1`, O = ST OP-TEE 4.0.0-stm32mp-r3.1).

| Block (line prefix) | Base | What is read | RM0457 chapter | Source |
| --- | --- | --- | --- | --- |
| `rifsc` | 0x42080000 | `RISC_SECCFGR`, `RISC_PRIVCFGR`, `RISC_PERx_CIDCFGR`, `RISC_PERx_SEMCR` of the I2C2 (id 42), LTDC (80, 119 `l1l2`, 120 `l3`, 121 `rot`) and DSI (81); `RISC_RIMC_CR` and `RISC_RIMC_ATTR0..15` (the RIMU: which master the LTDC is, open question 2 of the note) | RIFSC (RISUP: the per-peripheral part; RIMU: the master part) | O `stm32_rifsc.c`, `stm32mp25-rifsc.h:51,85-86,118-120` |
| `rcc` | 0x44200000 | `I2C2CFGR` 0x7A8, `GPIOBCFGR` 0x530, `GPIOFCFGR` 0x540, `XBAR27/28CFGR` 0x1084/0x1088, `PREDIV27/28` 0x1184/0x1188, `FINDIV27/28` 0x1290/0x1294, `PLL4CFGR1..5` 0x1360, `PLL8CFGR1..5` 0x1400 (ST `stm32mp25_rcc.h`), and `RCC_GPUCFGR` 0x83C (cross-check against mp2_gpu), `RCC_LTDCCFGR` 0x840, `RCC_DSICFGR` 0x844 (bit 0 reset, bit 1 clock, bit 2 sleep; DSI bit 12 `DSIBLSEL`, bit 15 `DSIPHYCKREFSEL` kernel-clock selects) | RCC | `driver/rust/src/soc/stm32mp2/rcc.rs` (0x83C), L `stm32mp25_rcc.h:439-440` |
| `ltdc` | 0x48010000 | global block `IDR` ... `FUT` and layer 1 `L1C0R` ... `L1CFBLNR`; only when RIFSC opens ids 80 and 119 and `RCC_LTDCCFGR` shows the clock on and no reset | LTDC | L `stm32mp251.dtsi:2150`, `ltdc.c:64-94,461-505` |
| `dsi` | 0x48000000 | DesignWare host (`VERSION` ... `PTCR0`) and ST wrapper (`WCFGR` ... `WRPCR2`, 0x400-0x43c); only when RIFSC opens id 81 and `RCC_DSICFGR` shows the clock on and no reset | DSI | L `stm32mp255.dtsi:123-134`, `dw_mipi_dsi-stm.c:37-107` |
| `rcc_raw` | 0x44200000 | every non-zero word of RCC 0x000-0x14ff, raw (the rest of flexgen, PLL2/3/5-7, kernel-clock selects, mapped to ST's `stm32mp25_rcc.h` offline) | RCC | |
| `gpiob`, `gpiof` | 0x44250000, 0x44290000 | `MODER`, `OTYPER`, `OSPEEDR`, `PUPDR`, `IDR`, `ODR`, `LCKR`, `AFRL`, `AFRH`, `SECCFGR`, `PRIVCFGR`: PB4 (ADV7535 interrupt), PB6 (ADV7535 reset), PF0 (I2C2 SDA), PF2 (I2C2 SCL, AF9) | GPIO | note, "Register blocks"; L `stm32mp25-pinctrl.dtsi:276-284`, `stm32mp257f-dk.dts:436-438` |
| `syscfg` | 0x44235000 | `DISPLAYCLKCR` (2 bit mux: 0 `txbyteclk`, 1 `clk_pix_lvds`, 2 `ck_ker_ltdc`) | SYSCFG | L `clk-stm32mp25-syscfg.c:13-62` |
| `risaf4` | 0x420d0000 | `RISAF_CR` and regions 10 (GPU carveout, 0xfa800000), 13 (LTDC secure layer, 0xfe800000, 8 MiB), 14 (rotation, 0xff000000, 16 MiB): `CFGR`, `STARTR`, `ENDR`, `CIDCFGR` | RISAF (the RISAL/RISAB-style area registers) | O `...-rif.dtsi:984-995`, `stm32mp257f-dk-resmem.dtsi:175-183` |

The I2C2 (the ADV7535's bus, 0x40130000, ST `stm32mp251.dtsi:935-937`, PF0 and
PF2) is read through its RIFSC entry (id 42) and the pins; its own registers
and its RCC gate (`I2C2CFGR`, in `rcc_raw`) are not gated by this probe, so
its registers are not read.

## Confidence, and what to do when a read faults

The RIFSC layout (0x10, 0x30, 0x100 + 8 * id), the RIMU offsets (0xC00,
0xC10), the RCC LTDC/DSI offsets and the LTDC and DSI maps are the ones the
note derives from the trees. **Least certain**: the RISAF4 region stride (the base 0x420d0000 is ST `stm32mp251.dtsi`) and
(OP-TEE `stm32_risaf.c`, from memory), the SYSCFG address (derived: base 0x44230000 + 0x5000), the
GPIO bank addresses (one 0x10000 step per bank from GPIOA 0x44240000; GPIOB is
the note's) and whether the non-secure side may read the RCC page word by
word. These sit last. A fault resets the board through the layer's handler
(or OP-TEE's watchdog); rerun with the group off, e.g.
`-DCONFIG_MP2_DISPLAY_STATE_RISAF=n` (also `_SYSCFG`, `_GPIO`, `_RCC_RAW`),
and report the faulting address as a finding.

## How it is run

As `mp2_identify` (the board runner stages the image, arms a one-shot boot,
and the layer resets back into Linux): from the repository root,

```
tools/board-stm32mp2.sh run build/mp2_display_state/zephyr/zephyr.bin <logdir> mp2_display_state
```

(on `resource:stm32mp257f-dk` with its console and reset resources). The
verdict line's `result=pass:mp2_display_state` comes from the `PASS` line.
The image's own end is the layer's `mp2: mp2_display_state done` line.

## Build

With a west workspace in `ZEPHYR_WS` (`tools/setup-gate-host.sh`), from the
repository root, as `tools/check.sh` builds the probes:

```
mods=$(PATH="$ZEPHYR_WS/.venv/bin:$PATH" tools/cts/zephyr-modules.sh "$ZEPHYR_WS")
(cd "$ZEPHYR_WS" && PATH="$ZEPHYR_WS/.venv/bin:$PATH" ZEPHYR_BASE="$ZEPHYR_WS/zephyr" \
  west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 \
    -d "$PWD/build/zephyr-werror/mp2_display_state" "$PWD/tools/probes/stm32mp257f_dk/mp2_display_state" -- \
    "-DZEPHYR_MODULES=$mods" "-DYAKOGL_DIR=$PWD" -DYAKOGL_WERROR=ON)
```

The image is `zephyr/zephyr.bin` in the build directory, linked to run at
0xa0000000. No board run has happened yet: the output above is what the code
prints, not a reading.
