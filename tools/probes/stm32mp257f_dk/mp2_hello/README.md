<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2_hello

The first image for the STM32MP257F-DK's Cortex-A35
(docs/notes/pre-study-stm32mp257f-dk.md, sections 4 and 8; issues #7 and
#1281). It shows that our Zephyr image runs on the A35 once U-Boot has started
it, prints what U-Boot handed over (Q8, Q13), and gives the board back. It
does that by itself through PSCI SYSTEM_RESET to TF-A's BL31, so the board
boots back into ST's Linux.

The work is done by the stm32mp2 board layer core
(`driver/zephyr/lib/board/stm32mp2/`, `stm32mp2.c` "Order"). The probe prints
the build, counts five seconds on the system timer and ends the run.

## What it prints

The console is USART2 at 115200 8N1, on the line setup TF-A and U-Boot left.
In order:

```
MP2:A EL2                                   the image's first code (__wrap___start, raw USART2)
MP2:A1                                      end of z_arm64_el1_init: now at EL1
MP2:B console up (USART2, polled)           PRE_KERNEL_1 99, through the console
mp2-handover: v=1 el=2 sctlr=0x... m=0 c=0 i=1 x0=0x... x1=0x0 x2=0x0 x3=0x0 pc=0xa0000000 cntfrq=... mpidr=0x... psci=0x10001
mp2: system counter at PRE_KERNEL_1: N ticks
mp2: no watchdog armed by this image; PSCI SYSTEM_RESET at 20 s of uptime at the latest
*** Booting Zephyr OS build ... ***
mp2_hello: build <git describe>
mp2_hello: stm32mp257f_dk, console usart2 at 115200 baud, image at 0xa0000000, timer <CNTFRQ> Hz
mp2_hello: PSCI version 0x10001, reset at 20000 ms of uptime at the latest
mp2_hello: 1 s (uptime ... ms)
...
mp2_hello: 5 s (uptime ... ms)
mp2_hello: PASS: counted 5 s in ... ms
mp2: mp2_hello done: PSCI SYSTEM_RESET at ... ms
```

After the last line TF-A resets the SoC, and the console shows TF-A, OP-TEE,
U-Boot and ST's Linux again. The values shown are what is expected, not
readings: no board run has happened yet (the return-trial item).

What each line means:

- `MP2:A EL<n>`: the EL the image was entered at. U-Boot's `booti` and `go`
  both start it at EL2 (the board-read study).
- `MP2:A1`: the EL2-to-EL1 step worked. `MP2:A` without `MP2:A1` means the
  image stopped in Zephyr's EL2 set-up.
- `mp2-handover:`: one line of `key=value` pairs, split on spaces after the
  tag.
  - `el`: CurrentEL[3:2].
  - `sctlr`: SCTLR_EL2 at EL2, else SCTLR_EL1, as found.
  - `m`, `c`, `i`: its MMU, D-cache and I-cache bits.
  - `x0`-`x3`: the entry values. With booti, x0 is the DTB pointer and
    x1-x3 are 0; with `go`, x0 is argc and x1 is argv. The image is linked
    with `-Wl,--wrap=__start`, so the header's branch reaches the layer's
    `__wrap___start` before Zephyr's reset.S, which reuses x0 at once. This
    is the method of `tools/probes/rpi_5/common/boot/`.
  - `pc`: where the entry code ran. It is the load address if nothing
    moved the image.
  - `cntfrq`: CNTFRQ_EL0 in Hz, decimal.
  - `mpidr`: MPIDR_EL1.
  - `psci`: PSCI_VERSION from BL31, read through the console report.
  - Without the entry code's record the line is `mp2-handover: v=1 missing`.
- `mp2: ... PSCI SYSTEM_RESET at ...`: the end of the run. If BL31 returns,
  `mp2: PSCI SYSTEM_RESET returned <rc>: halted` follows and the CPU halts.

A run that never reaches its end resets the board in one of two ways:

- The lifetime timer (`CONFIG_YAKOGL_STM32MP2_LIFETIME_S`, 20 s) ends with
  `mp2: lifetime over: PSCI SYSTEM_RESET ...`.
- A fatal error ends with `mp2: fatal error <reason>`, then
  `mp2: fatal error: PSCI SYSTEM_RESET ...`.

A hang with interrupts locked is covered only by the OP-TEE watchdog. The
board layer does not arm that watchdog yet (the zephyr-board-layer item);
BL2's IWDG is running when U-Boot hands over, and resets the board in about
32 s.

### The hang option

`CONFIG_MP2_HELLO_HANG` (Kconfig in the probe, default off) is the watchdog
trial of the return trial. The image prints its banner, then

```
mp2_hello: hanging on purpose with interrupts locked, the OP-TEE watchdog must reset
```

and spins with interrupts locked (`irq_lock()` and a busy loop): no five
seconds, no PASS line, no PSCI reset, and the layer's lifetime timer cannot
fire. Build it by adding `-DCONFIG_MP2_HELLO_HANG=y` after the `--` of the
build command below (the default build is unchanged without it). Run it with

```
MP2_NO_WATCHDOG=1 tools/board-stm32mp2.sh run --expect-watchdog <image> <logdir> mp2_hello_hang
```

(`MP2_NO_WATCHDOG=1` because this image does not arm the OP-TEE watchdog
itself; BL2's IWDG is what resets it). The runner's verdict is
`watchdog_reset` when the console has the banner and the hang line, no
`PSCI SYSTEM_RESET` line followed, and Linux answered over ssh within
`WDOG_BOUND` s (default 120) of the reboot.

## Build

With a west workspace in `ZEPHYR_WS` (`tools/setup-gate-host.sh`), run from
the repository root, the way `tools/check.sh` builds the probes:

```
mods=$(PATH="$ZEPHYR_WS/.venv/bin:$PATH" tools/cts/zephyr-modules.sh "$ZEPHYR_WS")
(cd "$ZEPHYR_WS" && PATH="$ZEPHYR_WS/.venv/bin:$PATH" ZEPHYR_BASE="$ZEPHYR_WS/zephyr" \
  west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 \
    -d "$PWD/build/zephyr-werror/mp2_hello" "$PWD/tools/probes/stm32mp257f_dk/mp2_hello" -- \
    "-DZEPHYR_MODULES=$mods" "-DYAKOGL_DIR=$PWD" -DYAKOGL_WERROR=ON)
```

For the hanging image add `-DCONFIG_MP2_HELLO_HANG=y` to the `--` arguments
(and a different `-d` directory, e.g. `build/zephyr-werror/mp2_hello_hang`).

The image is `build/zephyr-werror/mp2_hello/zephyr/zephyr.bin`. It is linked
and run at 0xa0000000 (`zephyr,sram` of the board's dts). The last build step
runs `check_early_hooks.cmake`, which fails the build in two cases: the
image header does not branch to `__wrap___start`, or the hand-over record is
not in line-aligned `.data`.

The board layer (`driver/zephyr/lib/board/stm32mp2`) comes from the shared
board library, as the other boards' do: `CONFIG_YAKOGL_STM32MP2` is on by
default for the SoC.

## Running it

The runner item adds the board to `tools/board.sh`; it is not there yet. Until
then this image is not run on the board by hand. Two points for the runner:

- An image run with `booti` must sit where booti places it. The Zephyr image
  header has no "place anywhere" flag, so booti moves an image that is
  somewhere else.
- With `go`, x0 is argc and x1 is argv, not a DTB pointer.

Both show in the `mp2-handover:` line (`pc`, `x0`).
