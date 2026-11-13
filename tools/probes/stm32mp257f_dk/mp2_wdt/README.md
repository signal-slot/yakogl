<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2_wdt

The bounded proof of the opt-in OP-TEE watchdog refresh for STM32MP257F-DK
images (plan #362, issues #7 and #1281). BL2 starts the IWDG before U-Boot,
and OP-TEE serves it over SMC (`arm,smc-wdt`, function id 0xbc000000, 32 s).
U-Boot does not pet it after `go 0xa0000000`, so an image that runs for
longer than about 32 s is reset (OP-TEE panics at about 27 s). An image that
runs indefinitely must refresh it. This probe shows that the refresh works,
and it ends in a reset by itself so the board is never stranded.

The client is `driver/rust/src/soc/stm32mp2/smc_wdt_refresh.rs`. The module's
header documents the protocol and cites the Linux driver and ST's OP-TEE
sources. Its C ABI is declared in `src/mp2_wdtr.h`. Nothing calls it unless
an image does: the board layer's own watchdog (`stm32mp2_wdog.c`,
`CONFIG_YAKOGL_STM32MP2_WDOG`) and every other image are unchanged. This
image turns the layer's watchdog off, so the probe is the watchdog's only
client.

The rules the client follows by construction:

- INIT is sent first. PET and GET_TIMELEFT are refused, with no call made,
  before INIT has succeeded.
- A timeout under 6 s is refused with no call made. OP-TEE's early interrupt
  comes 5 s before the timeout and needs 1 s more.
- ENABLE is only ever sent with 1. OP-TEE's ENABLE 0 makes it reload the
  counter at every early interrupt, after which a hang is never reset.

## What it does

1. Prints its banner.
2. `yd_mp2_wdtr_init(0xbc000000, 32)` sends INIT, SET_TIMEOUT 32, ENABLE 1
   and PET. If any of them is refused, it prints a FAIL line and resets
   through PSCI at once.
3. Calls PET every `CONFIG_MP2_WDT_PET_S` (5 s), `REFRESH_S / PET_S` times,
   with `CONFIG_MP2_WDT_REFRESH_S` defaulting to 60 s. The loop has a fixed
   count, so the petting stops by construction. After each PET it prints the
   time left (GET_TIMELEFT).
4. Prints `mp2_wdt: stopped petting` and from then on only reads the time
   left. About 27 s after the last PET OP-TEE panics (`Watchdog`), and about
   5 s later the IWDG resets the SoC. The board then boots ST's Linux again.

The layer's lifetime (`CONFIG_YAKOGL_STM32MP2_LIFETIME_S=150` here) is a
PSCI SYSTEM_RESET that comes well after that. If it shows up
(`mp2: lifetime over`), the watchdog did not fire. That is a failed run, but
the board still comes back. A build assert keeps the lifetime at least
`REFRESH_S + TIMEOUT_S + 30` s.

## What it prints

Expected. No board run has happened yet; the board-wdt item runs it.

```
MP2:A EL2
MP2:A1
MP2:B console up (USART2, polled)
mp2-handover: v=1 el=2 ...
mp2: system counter at PRE_KERNEL_1: N ticks
mp2: no watchdog armed by this image; PSCI SYSTEM_RESET at 150 s of uptime at the latest
*** Booting Zephyr OS build ... ***
mp2_wdt: build <git describe>
mp2_wdt: stm32mp257f_dk, console usart2 at 115200 baud, image at 0xa0000000, timer <CNTFRQ> Hz
mp2_wdt: PSCI version 0x10001, reset at 150000 ms of uptime at the latest
mp2_wdt: fid 0xbc000000, timeout 32 s, PET every 5 s for 60 s, then stop petting
mp2_wdt: init ok: bounds 0..131 s, timeout 32 s
mp2_wdt: init t=0 left=32
mp2_wdt: pet t=5 left=32
...
mp2_wdt: pet t=60 left=32
mp2_wdt: stopped petting at t=60 after 12 pets: OP-TEE panics in about 27 s, the IWDG resets in about 32 s
mp2_wdt: unpetted t=65 left=27
...
mp2_wdt: unpetted t=85 left=7
(OP-TEE's panic, then TF-A, OP-TEE, U-Boot and ST's Linux)
```

The `mp2:` line says "no watchdog armed by this image" because the layer's
own arming is off. The refresh starts in `main()`, after that report.

How to read the record:

- Pass: twelve `pet` lines whose `left` stays near 32, then
  `stopped petting`, then `unpetted` lines counting down. After that comes
  the next boot, without `mp2: lifetime over` and within about 35 s of
  `stopped petting`. That shows the PETs held the reset off for 60 s, about
  twice the bare 32 s, and that stopping lets it come.
- `left=? (rc -5 code -8)` (DISABLED) means OP-TEE thinks the watchdog is
  stopped. `left=? (... code -1)` means GET_TIMELEFT is not supported. In
  both cases the PET lines still prove the refresh if the board outlives
  32 s.
- `FAIL: init rc ...`: the command and code say what was refused (INIT
  -6: no watchdog driver behind the service; SET_TIMEOUT -2: out of
  bounds).
- A reset before `stopped petting` means the refresh does not work.

The time left is OP-TEE's computation (`stm32_iwdg.c`,
`iwdg_wdt_get_timeleft`): from the last refresh, with whole-second
truncation. Its exact values are a board reading.

## Build

With a west workspace in `ZEPHYR_WS` (`tools/setup-gate-host.sh`), run from
the repository root, the way `tools/check.sh` builds the probes:

```
mods=$(PATH="$ZEPHYR_WS/.venv/bin:$PATH" tools/cts/zephyr-modules.sh "$ZEPHYR_WS")
(cd "$ZEPHYR_WS" && PATH="$ZEPHYR_WS/.venv/bin:$PATH" ZEPHYR_BASE="$ZEPHYR_WS/zephyr" \
  west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 \
    -d "$PWD/build/zephyr-werror/mp2_wdt" "$PWD/tools/probes/stm32mp257f_dk/mp2_wdt" -- \
    "-DZEPHYR_MODULES=$mods" "-DYAKOGL_DIR=$PWD" -DYAKOGL_WERROR=ON)
```

The image is `build/zephyr-werror/mp2_wdt/zephyr/zephyr.bin`, linked and run
at 0xa0000000. A different bound goes after the `--`, for example
`-DCONFIG_MP2_WDT_REFRESH_S=90 -DCONFIG_YAKOGL_STM32MP2_LIFETIME_S=160`.

The yakogl-drivers crate is built with the feature `stm32mp2` and linked
because `CONFIG_YAKOGL_STM32MP2_GPU=y` (prj.conf), as for `mp2_gpu`. This
image has no GPU driver, so nothing of the GPU's runs.

Host tests of the client: `cargo test -p yakogl-drivers --features stm32mp2
smc_wdt_refresh`.

## Running it

This image does not print the `gc_wdog_arm: armed` marker the DK runner
requires, and it ends in a watchdog reset rather than a PSCI reset. Run it
like `mp2_hello`'s hang image:

```
MP2_NO_WATCHDOG=1 WDOG_BOUND=200 tools/board-stm32mp2.sh run --expect-watchdog <image> <logdir> mp2_wdt
```

`WDOG_BOUND` must cover the 60 s of petting as well as the reset. As of this
probe, the runner's `--expect-watchdog` verdict looks for the literal
`mp2_hello: hanging on purpose` (`tools/board-stm32mp2.sh`, its boot
verdict). For this image it would answer `no_hang`, although the board comes
back either way. Accepting `mp2_wdt: stopped petting` as the hang line, and
counting the `pet` lines, is a runner change for the board-wdt item. The
record printed above holds the proof in either case.
