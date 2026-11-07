<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2_identify

The image for the STM32MP257F-DK's return trial
(docs/notes/pre-study-stm32mp257f-dk.md, sections 4 and 8; issues #7 and
#1281). It is `mp2_hello` with OP-TEE's watchdog armed at boot: the image
prints the early banner and what U-Boot handed over (Q8, Q13), arms the
watchdog, and gives the board back through PSCI SYSTEM_RESET to TF-A's
BL31, so the board boots back into ST's Linux. If the image hangs in a way
the board layer's lifetime timer cannot end (interrupts locked, a PSCI that
returns), the watchdog resets the SoC.

It touches nothing but the console, the system timer and SMCs to TF-A
(PSCI) and to OP-TEE's watchdog service: no GPU access (nothing at
0x48280000), no SCMI, no RCC write.

The work is done by the stm32mp2 board layer
(`driver/zephyr/lib/board/stm32mp2/`): `stm32mp2.c` (its "Order") and
`stm32mp2_wdog.c` (`CONFIG_YAKOGL_STM32MP2_WDOG`).

## The watchdog

`stm32mp2_wdog.c` speaks the `arm,smc-wdt` protocol of
`driver/rust/src/soc/stm32mp2/smc_wdt.rs` over Zephyr's `arm_smccc_smc`,
with the function id 0xbc000000 (the node's `arm,smc-id` in ST's tree) and
Linux's timeout, 32 s:

- PRE_KERNEL_1 0: SET_TIMEOUT 32, ENABLE 1, PET, stopping at the first
  refusal.
- PRE_KERNEL_1 99: `gc_wdog_arm: armed ...`, the literal `tools/board.sh`
  requires in an image (as on the i.MX8MP). If arming was refused:
  `mp2: OP-TEE watchdog arming FAILED: <command> answered <code> ...`, then
  PSCI SYSTEM_RESET at once.
- POST_KERNEL 0: a PET every second until the layer's lifetime
  (`CONFIG_YAKOGL_STM32MP2_LIFETIME_S`, 20 s), then no more.

`driver/zephyr/lib/board/test/stm32mp2_wdog_mock.c` pins every SMC on the
host.

## What it prints

```
MP2:A EL2
MP2:A1
MP2:B console up (USART2, polled)
mp2-handover: v=1 el=2 sctlr=0x... m=0 c=0 i=1 x0=0x... x1=0x0 x2=0x0 x3=0x0 pc=0xa0000000 cntfrq=... mpidr=0x... psci=0x10001
mp2: system counter at PRE_KERNEL_1: N ticks
mp2: OP-TEE watchdog armed by this image (32 s); PSCI SYSTEM_RESET at 20 s of uptime at the latest
gc_wdog_arm: armed OP-TEE arm,smc-wdt fid 0xbc000000, timeout 32 s, PET every 1000 ms until 20000 ms of uptime
*** Booting Zephyr OS build ... ***
mp2_identify: build <git describe>
mp2_identify: stm32mp257f_dk, console usart2 at 115200 baud, image at 0xa0000000, timer <CNTFRQ> Hz
mp2_identify: handed over at EL2, pc 0x..., x0 0x..., sctlr 0x..., cntfrq ... Hz, mpidr 0x..., PSCI 0x10001
mp2_identify: watchdog fid 0xbc000000 armed rc 0, timeout 32 s, feeder running, reset at 20000 ms of uptime at the latest
mp2_identify: watchdog time left at main: N s
mp2_identify: 1 s (uptime ... ms), N PETs, 0 refused
...
mp2_identify: watchdog time left after the wait: N s
mp2_identify: PASS: watchdog armed (32 s), N PETs in ... ms
mp2: mp2_identify done: PSCI SYSTEM_RESET at ... ms
```

The `mp2-handover:` line is described in `../mp2_hello/README.md`. The order
of the two lines at PRE_KERNEL_1 99 (the core's report, the watchdog's
marker) is the link order. The values shown are what is expected, not
readings: no board run has happened yet (the return-trial item). Whether
OP-TEE accepts SET_TIMEOUT without an INIT first, and whether it answers
GET_TIMELEFT, the board run shows; the PASS line does not depend on
GET_TIMELEFT.

## Build

With a west workspace in `ZEPHYR_WS` (`tools/setup-gate-host.sh`), from the
repository root, as `tools/check.sh` builds the probes:

```
mods=$(PATH="$ZEPHYR_WS/.venv/bin:$PATH" tools/cts/zephyr-modules.sh "$ZEPHYR_WS")
(cd "$ZEPHYR_WS" && PATH="$ZEPHYR_WS/.venv/bin:$PATH" ZEPHYR_BASE="$ZEPHYR_WS/zephyr" \
  west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 \
    -d "$PWD/build/zephyr-werror/mp2_identify" "$PWD/tools/probes/stm32mp257f_dk/mp2_identify" -- \
    "-DZEPHYR_MODULES=$mods" "-DYAKOGL_DIR=$PWD" -DYAKOGL_WERROR=ON)
```

The image is `build/zephyr-werror/mp2_identify/zephyr/zephyr.bin`, linked to
run at 0xa0000000. As for `mp2_hello`, the probe adds the board layer
itself (`add_subdirectory` in `CMakeLists.txt`, `rsource` in `Kconfig`), and
also compiles `stm32mp2_wdog.c` into its app; the item zephyr-layer-hook
moves the layer into `driver/zephyr/lib/board/`, and the layer's own
`CMakeLists.txt` takes the watchdog source over after that.
