<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_ident

D2 part 1 (`docs/notes/pre-study-rpi5.md`, section 6): can Zephyr read the
V3D 7.1's identity on the Raspberry Pi 5 with no power or clock setup, and are
the words the ones Linux's debugfs `v3d_ident` shows (D3)? This answers U2
(the V3D is powered and clocked when the firmware hands over) and U5 (the
identity and MMU widths seen from the ARM) if the reads come back.

What it does, in order:

1. **The watchdog first.** The board has no remote power, so the board layer
   (`driver/zephyr/lib/board/rpi4/rpi_wdog.c`, `CONFIG_YAKOGL_RPI5_WDOG`)
   arms the BCM2712 `pm` watchdog (10 s period) in the first `PRE_KERNEL_1`
   hook, before `main()`, and feeds it every 3 s for 30 s
   (`CONFIG_YAKOGL_RPI5_WDOG_HOLD_S`). Its reset ends every run; with
   `tryboot` the board comes back into Linux.
2. The hub (`BCM2712_V3D_HUB_BASE`, `0x10_02000000`, 16 KiB) and core 0
   (`BCM2712_V3D_CORE0_BASE`, `0x10_02008000`, 24 KiB) from
   `driver/src/v3d/v3d_regs.h` lie above 4 GiB, outside Zephyr's rpi_5 MMU
   map; each is `device_map()`ped uncached (device memory) first.
3. One read and one line each, printed right after the read:

```
rpi5_ident: build <git describe>
rpi5_ident: rpi_5, V3D 7.1, no power or clock setup, watchdog armed at boot
rpi5_ident: hub and core0 mapped, reading
rpi5_ident: HUB_IDENT0 0x........      (hub + 0x0008)
rpi5_ident: HUB_IDENT1 0x........      (hub + 0x000c)
rpi5_ident: HUB_IDENT2 0x........      (hub + 0x0010)
rpi5_ident: HUB_IDENT3 0x........      (hub + 0x0014)
rpi5_ident: CORE_IDENT0 0x........     (core0 + 0x0000)
rpi5_ident: CORE_IDENT1 0x........     (core0 + 0x0004)
rpi5_ident: CORE_IDENT2 0x........     (core0 + 0x0008)
rpi5_ident: MMU_DEBUG_INFO 0x........  (hub + 0x1238)
rpi5_ident: V3D t.r, n core(s), q QPUs, VPM v bytes, PA p / VA a bits, MMU yes|no
rpi5_ident: done
```

The raw words are the record; the decoded line is a convenience
(`tools/v3d/ident71.py` decodes them in full). Proof for D2: the words equal
D3's `v3d_ident` from the board's Linux.

**A fault on a read.** The board layer has no way to catch a synchronous
external abort (or an SError) and go on. If the V3D is unpowered or
unclocked and a read faults, Zephyr's fatal handler may print its
`ZEPHYR FATAL ERROR` report (`CONFIG_LOG`, immediate mode), the system halts,
the watchdog's feeding stops and it resets the board. What shows on the
console is then: the last value line is the register *before* the faulting
one, no value line for the faulting one, no `rpi5_ident: done`, and the boot
loader's log after the reset. A read that does not fault but returns words
unlike D3's (all zeros, for example) is a result too; it is printed as read
and the image still ends with `done` (it never prints `FAIL`: the comparison
with D3 is made off the board).

Not here: the power and clock fallback of section 3.4 and section 9 (SMS
`CLEAR_POWER_OFF`, firmware clock 5 through the mailbox, the `pm` reset bit)
comes in probe-d2 (D2 part 2), after the mailbox lands in this probe. This
image does no power, clock or reset setup at all.

`get_throttled` is recorded by `tools/board.sh` in `meta.txt`, not by the
image (the image cannot read it); check it before reading a fault as a
hardware one.

Build (from the west workspace; `-DYAKOGL_WERROR=ON` as the `zephyr` rows
build):

```sh
west build -p always -b rpi_5 -d build/rpi5_ident yakogl/tools/probes/rpi_5/rpi5_ident -- -DYAKOGL_WERROR=ON
```

Run: `tools/board.sh pi5` with the staged `zephyr.bin` (tryboot, `kernel=`
naming it; config.txt restored and the staged files removed after the run).
Its default end matcher already knows `rpi5_ident: done` and
`rpi5_ident: FAIL`. The lifetime to wait for is about 30 s + 10 s + the
reboot. `BOARD_HOST` and `BOARD_UART` come from the leases, never from a file.
Run it only after `rpi5_wdog` has shown that the watchdog resets the board.

Built, not run on the board.
