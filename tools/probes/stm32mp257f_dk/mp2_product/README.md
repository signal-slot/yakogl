<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2_product

The shape of a product image for the STM32MP257F-DK's standalone default
(plan #362, issues #7 and #1281; `docs/notes/stm32mp257f-dk-standalone.md`
section 6). BL2 starts the IWDG before U-Boot, and OP-TEE serves it over SMC
(`arm,smc-wdt`, function id 0xbc000000, 32 s). Nothing pets it after
`go 0xa0000000`, so an image that runs indefinitely must refresh it itself,
and only while it is healthy. This image does that through
`driver/rust/src/soc/stm32mp2/smc_wdt_refresh.rs` (C ABI in
`../mp2_wdt/src/mp2_wdtr.h`), and it carries the product watchdog marker
that `default <image> forever` requires.

Like `mp2_wdt`, it turns the board layer's own watchdog off
(`CONFIG_YAKOGL_STM32MP2_WDOG=n`), so the image is the watchdog's only
client.

## What it does

1. Prints its banner and its lifetime.
2. `yd_mp2_wdtr_init(0xbc000000, 32)` sends INIT, SET_TIMEOUT 32, ENABLE 1
   and PET. If any of them is refused, it prints a FAIL line and resets
   through PSCI at once.
3. Prints the marker line `mp2_wdtr: product refresh armed: ...`.
4. Starts the heartbeat, a `k_timer` that advances a counter every
   `CONFIG_MP2_PRODUCT_HEARTBEAT_MS` (1000 ms).
5. In the main loop, every `CONFIG_MP2_PRODUCT_PET_S` (5 s), it PETs only if
   the heartbeat moved since the previous PET. If the heartbeat stalls or the
   main loop stops, the PETs stop. OP-TEE then panics about 27 s after the
   last PET, and the IWDG resets the SoC about 5 s after that. A real
   product puts its own health condition here.
6. Prints one `alive` line a minute.
7. With `CONFIG_MP2_PRODUCT_LIFETIME_S` set (120 s by default), it does a
   PSCI SYSTEM_RESET at that much uptime (`mp2_product lifetime over`).

## The lifetime: counted runs and products

`CONFIG_MP2_PRODUCT_LIFETIME_S` defaults to 120. A counted
`default <image> N` of this build therefore ends each boot in a reset, the
product script counts down, and after N boots the board is back in
OpenSTLinux by itself.

**A real product sets `CONFIG_MP2_PRODUCT_LIFETIME_S=0`.** Then the image
never resets by itself and runs for as long as the heartbeat holds. **Only
`default <image> forever` with `BOARD_PERSON_PRESENT=1` may carry such an
image.** Under a counted default it would never reset, so the count would
never run out and the board would be stranded. Neither the runner nor
`tools/mp2s_image_check.py` refuses that case yet (see the plan #362
report).

This lifetime is the image's own, not the board layer's
`CONFIG_YAKOGL_STM32MP2_LIFETIME_S`. The layer's lifetime has no off switch
(range 5-3600), so `prj.conf` sets it to its maximum, 3600 s, and it remains
only a backstop. A build assert keeps a non-zero product lifetime below it.
Even with `CONFIG_MP2_PRODUCT_LIFETIME_S=0`, the layer still resets the board
after one hour until it gets an off switch.

## The marker

The product watchdog marker is the literal

```
mp2_wdtr: product refresh armed
```

(`MP2_PRODUCT_WDT_MARKER` in `src/main.c`). The image prints it once INIT has
succeeded, so it is both in the file and in the console record. The runner
checks the file before `forever`:

```
tools/mp2s_image_check.py <image> --free-bytes <bootfs free> \
  --require-marker 'mp2_wdtr: product refresh armed'
```

A product image of its own carries the same literal only if it refreshes
the watchdog as this one does.

## What it prints

This is the expected output. No board run has happened yet.

```
MP2:A EL2
...
mp2: no watchdog armed by this image; PSCI SYSTEM_RESET at 3600 s of uptime at the latest
*** Booting Zephyr OS build ... ***
mp2_product: build <git describe>
mp2_product: stm32mp257f_dk, console usart2 at 115200 baud, image at 0xa0000000, timer <CNTFRQ> Hz
mp2_product: PSCI version 0x10001, lifetime 120 s (PSCI SYSTEM_RESET), layer's at 3600000 ms
mp2_product: fid 0xbc000000, timeout 32 s, PET every 5 s while the heartbeat (1000 ms) advances
mp2_wdtr: product refresh armed: bounds 0..131 s, timeout 32 s
mp2_product: alive t=60 heartbeat=60 pets=12 left=32
mp2: mp2_product lifetime over: PSCI SYSTEM_RESET at 12xxxx ms
(TF-A, OP-TEE, U-Boot: the next boot of the default, or ST's Linux)
```

With `LIFETIME_S=0` the third line reads `no lifetime of its own (product
build)`, and the `alive` lines continue. A
`heartbeat stalled at <n> t=<s>, not petting` line means the health
condition failed, and the IWDG resets the board about 32 s after the last
PET.

## Build

With a west workspace in `ZEPHYR_WS` (`tools/setup-gate-host.sh`), run from
the repository root:

```
mods=$(PATH="$ZEPHYR_WS/.venv/bin:$PATH" tools/cts/zephyr-modules.sh "$ZEPHYR_WS")
(cd "$ZEPHYR_WS" && PATH="$ZEPHYR_WS/.venv/bin:$PATH" ZEPHYR_BASE="$ZEPHYR_WS/zephyr" \
  west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 \
    -d "$PWD/build/zephyr-werror/mp2_product" "$PWD/tools/probes/stm32mp257f_dk/mp2_product" -- \
    "-DZEPHYR_MODULES=$mods" "-DYAKOGL_DIR=$PWD" -DYAKOGL_WERROR=ON)
```

The image is `build/zephyr-werror/mp2_product/zephyr/zephyr.bin`, linked and
run at 0xa0000000. To build the product form, add
`-DCONFIG_MP2_PRODUCT_LIFETIME_S=0` after the `--`.

## Running it

Its runs go through the standalone runner (`tools/board-stm32mp2-standalone.sh`,
plan #362): `default <image> N` with the 120 s build, or `forever` with
`BOARD_PERSON_PRESENT=1`. It does not print the one-shot runner's
`gc_wdog_arm: armed` marker.
