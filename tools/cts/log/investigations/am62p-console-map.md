<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: which UART the usb-serial-1 lease reaches, and what gl_test can do about it

Item am62p-console-map of plan #141 (2026-10-12), integration tip `db3545f`.
One probe session on the leased board over ssh. No reboot, no `tools/board.sh`
run, no farm setting changed. The board's address and the bridge host are not
written here; "port-32" and "port-33" are the last two digits of the
lease's bridge ports (35932, 35933), as in `am62p-console-empty.md`.
Each fact is marked **measured** (output below), **read** (read in the tree
today) or **inferred**.

## Answer

| Question | Answer |
|---|---|
| UART the lease (port-32) reaches | Linux `ttyS3`, MMIO `0x04a00000`: **mcu_uart0** (measured: `YK-PROBE-S3` arrived; S0 and S1 did not) |
| UART the gl_test image prints its verdict on | **main_uart0**, Linux `ttyS2`, MMIO `0x02800000` (read: `zephyr,console = &main_uart0`) |
| Can the image print on the lease's UART? | Not by a Kconfig or overlay row alone (section 4). It would move the whole console, and U-Boot and Linux stay on main_uart0 |
| What makes a run observable | Lease the bridge port that is wired to main_uart0 (port-33 in earlier records), or hand the run `BOARD_UART=<host>:35933`. `tools/board.sh` needs no change |

## 1. Probe: the UARTs of the board (measured)

Commands (the board is `<board>`; ssh user and address omitted):

```
$ ssh <board> -- 'ls -l /dev/ttyS* /dev/ttyUSB* /dev/ttyACM*; cat /proc/tty/driver/serial; for t in /sys/class/tty/ttyS*; do echo $t; readlink -f $t/device; done; cat /proc/cmdline; ps -eo pid,tty,args | grep -E "getty|agetty|ssh" | grep -v grep'
ls: cannot access '/dev/ttyUSB*': No such file or directory
ls: cannot access '/dev/ttyACM*': No such file or directory
crw-rw---- 1 root dialout 4, 64 Oct 12 10:41 /dev/ttyS0
crw-rw---- 1 root dialout 4, 65 Oct 12 10:41 /dev/ttyS1
crw-rw---- 1 root dialout 4, 66 Oct 12 11:26 /dev/ttyS2
crw-rw---- 1 root dialout 4, 67 Oct 12 10:41 /dev/ttyS3
crw-rw---- 1 root dialout 4, 69 Oct 12 10:41 /dev/ttyS5
cat: /proc/tty/driver/serial: Permission denied
/sys/class/tty/ttyS0
/sys/devices/platform/bus@f0000/2810000.serial
/sys/class/tty/ttyS1
/sys/devices/platform/bus@f0000/bus@f0000:bus@b00000/2b300050.target-module/2b300000.serial
/sys/class/tty/ttyS2
/sys/devices/platform/bus@f0000/2800000.serial
/sys/class/tty/ttyS3
/sys/devices/platform/bus@f0000/bus@f0000:bus@4000000/4a00000.serial
/sys/class/tty/ttyS5
/sys/devices/platform/serial8250
root=LABEL=otaroot rootfstype=ext4 quiet logo.nologo vt.global_cursor_default=0 plymouth.ignore-serial-consoles splash fbcon=map:3 ostree=/ostree/boot.1/torizon/...
   1005 tty1     /sbin/agetty -o -p -- \u --noclear - linux
   1006 ttyS2    /sbin/agetty -o -p -- \u --keep-baud 115200,57600,38400,9600 - vt220
   1503 ?        sshd: torizon [priv]
   1516 ?        sshd: torizon@notty
```

(`/proc/tty/driver/serial` is not readable for the ssh user, so the base
addresses come from sysfs and the kernel log.)

```
$ ssh <board> -- 'cat /proc/device-tree/chosen/stdout-path; for a in serial0 serial1 serial2 serial3 serial4; do printf "%s=" $a; tr "\0" " " < /proc/device-tree/aliases/$a; echo; done; dmesg | grep -iE "ttyS|serial" | head -20'
serial2:115200n8
serial0=/bus@f0000/serial@2810000
serial1=/bus@f0000/bus@b00000/target-module@2b300050/serial@0
serial2=/bus@f0000/serial@2800000
serial3=/bus@f0000/bus@4000000/serial@4a00000
serial4=/bus@f0000/serial@2860000
[    0.902274] Serial: 8250/16550 driver, 6 ports, IRQ sharing disabled
[    1.105364] omap8250 2b300000.serial: No clock speed specified: using default: 48000000
[    1.106256] 2b300000.serial: ttyS1 at MMIO 0x2b300000 (irq = 80, base_baud = 3000000) is a 8250
[    1.123886] 4a00000.serial: ttyS3 at MMIO 0x4a00000 (irq = 303, base_baud = 3000000) is a 8250
[    1.125467] 2800000.serial: ttyS2 at MMIO 0x2800000 (irq = 304, base_baud = 3000000) is a 8250
[    1.125546] printk: console [ttyS2] enabled
[    1.155927] 2810000.serial: ttyS0 at MMIO 0x2810000 (irq = 305, base_baud = 3000000) is a 8250
[    1.157493] 2860000.serial: ttyS4 at MMIO 0x2860000 (irq = 306, base_baud = 3000000) is a 8250
```

Reading: the kernel console (`stdout-path` = serial2, `console [ttyS2]`) and
the only serial getty are on `ttyS2` = `0x02800000`. `ttyS4` (`0x02860000`) has
no `/dev` node. `ttyS5` is the legacy `serial8250` platform port, not a SoC
UART.

| Linux tty | MMIO | Name (inferred) | In use |
|---|---|---|---|
| ttyS0 | 0x02810000 | main_uart1 | no holder seen |
| ttyS1 | 0x2b300000 | wkup_uart0 | no holder seen |
| ttyS2 | 0x02800000 | main_uart0 | kernel console and serial getty (not probed here) |
| ttyS3 | 0x04a00000 | mcu_uart0 | no holder seen |

The names are TI's naming by address; they are not in this tree (the tree
has no `mcu_uart0` or `wkup_uart0` node), so they are **inferred**. The
`main_uart0` = `0x02800000` link is also **read**: Zephyr's console is
`main_uart0` and Linux's console is `0x02800000`; they are one wire (the U-Boot,
Zephyr and getty text of earlier successful runs all came on one port).

## 2. Probe: markers against the lease (measured)

Before any write, `serial_status` of the lease (the console prints nothing on
its own, no banner, no getty):

```
{"backend":"tcp adapter usb-serial-1:35932","bytes_buffered":0,"connected":true,"current_offset":0,"lease_mode":"exclusive","oldest_offset":0,"read_cursor":0,...,"state":"connected",...}
```

Per UART: `stty -F <dev> 115200` (rc 0 for all three), then the marker, then 2 s,
then `serial_status`.

```
$ ssh <board> -- 'echo YK-PROBE-S0 > /dev/ttyS0; echo "rc=$?"; sleep 2'
rc=0
  serial_status: "bytes_buffered":0,"current_offset":0

$ ssh <board> -- 'echo YK-PROBE-S1 > /dev/ttyS1; echo "rc=$?"; sleep 2'
rc=0
  serial_status: "bytes_buffered":0,"current_offset":0

$ ssh <board> -- 'echo YK-PROBE-S3 > /dev/ttyS3; echo "rc=$?"; sleep 2'
rc=0
  serial_status: "bytes_buffered":13,"current_offset":13
  serial_read:   {"bytes":13,"from_offset":0,"next_offset":13,"output":"YK-PROBE-S3\r\n"}
```

| Device | MMIO | Marker | Arrived on the lease (port-32) |
|---|---|---|---|
| /dev/ttyS0 | 0x02810000 | YK-PROBE-S0 | no (0 bytes) |
| /dev/ttyS1 | 0x2b300000 | YK-PROBE-S1 | no (0 bytes) |
| /dev/ttyS2 | 0x02800000 | none written: a getty holds it | not probed; the earlier records wrote YK-PROBE-S2 and nothing arrived (`am62p-pr133.md`, `am62p-pr133-baseline.md`). Its getty also printed no prompt in this session (offset 0 before the first marker) |
| /dev/ttyS3 | 0x04a00000 | YK-PROBE-S3 | **yes**, 13 bytes `YK-PROBE-S3\r\n` |

Result: the lease's port-32 is **mcu_uart0 = `ttyS3`**. It agrees with
`am62p-console-empty.md` (`YAKOGL-PROBE-S3` arrived) and with the two plan #141
records. It is a mapping of one session; the wiring is not touched.

## 3. Which UART the gl_test image prints on (read)

- `driver/boards/toradex/verdin_am62p/verdin_am62p_am62p54_a53.dts:29-30`:
  `zephyr,console = &main_uart0; zephyr,shell-uart = &main_uart0;`.
- `.../verdin_am62p_am62p54_a53.dts:124-129`: `&main_uart0` at 115200 with the
  pinctrl of `verdin_am62p-pinctrl.dtsi:5-10` ("Verdin UART_3 (debug console) =
  AM62P main_uart0").
- `driver/samples/zephyr/gl_test/boards/verdin_am62p_am62p54_a53.conf:22`:
  "Console on main_uart0 (Verdin UART_3, the X18 debug USB)"; lines 24-25 add
  the early boot markers ('H', 'K') on the same UART
  (`CONFIG_SOC_AM62PX_A53_BOOT_MARKER`).
- The gl_test overlay (`.../boards/verdin_am62p_am62p54_a53.overlay`) and
  `prj.conf` have no `chosen` or UART row, so the board's console stands.
- The text that reads the verdict: `tools/board.sh:214`
  (`BOARD_UART=${BOARD_UART:-${UART_ADDR:-}}`: a `host:port` bridge), `:134-136`
  (the variable's documentation), `:249-251` (`uart_is_bridge`), `:264`
  (`am62p_netboot_uart.py capture $BOARD_UART ...`). `board.sh` also reads
  U-Boot's phases from the same stream (`:492-493`, the scan), so the stream
  must be the UART U-Boot and Zephyr use, main_uart0.
- Docs say the same: `docs/HANDOFF.md:907-908` ("The console must be the
  main_uart0 bridge port: a session leased the mcu_uart0 port and captured
  nothing"), `tools/cts/RESULTS.md:1007` ("console over usb-serial-1:35933;
  usb1:35932 is dead"). The `CLAUDE.md` rule file is not in this checkout.

So the gl_test image prints its verdict (`gl_test: OK|FAILED (rc=N)`,
`gl_test: all: ...`) on **main_uart0 (ttyS2)**, and the lease reaches
**mcu_uart0 (ttyS3)**: the verdict goes to a wire nobody on this lease reads.

## 4. Could it print on the lease's UART?

Not with one row. Moving the console to mcu_uart0 would need, at least:

1. `zephyr,console` and `zephyr,shell-uart` to `&mcu_uart0` in
   `verdin_am62p_am62p54_a53.dts:29-30` (a board-wide change, so every AM62P
   sample moves), an `&mcu_uart0 { status = "okay"; ... }` node with its
   pinctrl (the tree has none), and a check that the MCU UART pads are routed
   to a connector on the Dahlia carrier. I could not check the node in
   Zephyr's upstream dtsi (it is not in this workspace), so this is
   **inferred**.
2. Nothing else on the board changes: U-Boot/SPL, the Linux kernel console and
   getty stay on main_uart0 (section 1). `tools/board.sh` scans the one stream
   for the SPL banner and the kernel start (`:492-493`), so a Zephyr-only move
   would leave the scan with no `no_spl` / `back` evidence on the lease's port.
   The run would be half observable.
3. Every earlier AM62P record, `RESULTS.md` row and `deqp-uart-tally.py` input
   was taken on main_uart0. A console move breaks the comparison with them.

A per-sample route would still be one `chosen` overlay row in
`driver/samples/zephyr/gl_test/boards/verdin_am62p_am62p54_a53.overlay` plus
the node and pinctrl, and it would still leave points 2 and 3. I do not
recommend it.

## 5. Does the lease's port need to change? Recommendation

Yes. This is the cheaper and the safe fix: all of the code already expects
main_uart0 (sections 3 and 4).

1. **Lease the main_uart0 bridge port for AM62P gl_test sessions.** Earlier
   baselines used port-33 for `ttyS2` (`tools/cts/RESULTS.md:1007`, `:1037`,
   `:1124`, `:1418`; `docs/HANDOFF.md:907-908`). In the farm's resource row for
   `usb-serial-1`, point the serial console at port 33 (or add a second
   resource for it). That is a farm setting; this item did not change it. The
   resource row of the farm needs the owner (or an agent with that right).
2. **Check the port the same way as here before any board run**: lease the
   port, write one marker to `/dev/ttyS2` over ssh, and expect it in
   `serial_read`. The getty on `ttyS2` should also print a login prompt when
   the lease is right; here it printed nothing on port-32, which is one more
   sign that port-32 is not main_uart0. Write to `ttyS2` only when no getty or
   run holds it (a marker line is harmless to a getty, but stop the check as
   soon as one marker arrives).
3. **No `board.sh` or Kconfig change.** `BOARD_UART` takes `host:port` already
   (`tools/board.sh:134-136`, `:214`). If the lease cannot be moved, a session
   can run `BOARD_UART=<bridge host>:35933 tools/board.sh am62p ...`; but
   port-33 is then not leased to the session, another client may hold it
   ("one client at a time", `am62p_netboot_uart.py` returns 4 on `busy`), and
   that needs the owner's yes first.
4. Until one of these holds, record AM62P gl_test sessions as "not run", as
   the two plan #141 records did. A run without a readable verdict (`gl_test:
   OK|FAILED`) is not a result. Do not read the board's SNVS-like record in
   its place: the AM62P has no such channel (`tools/cts/RESULTS.md:15663`: "the
   only result channel is the UART").
5. An option for later (not needed now): a one-line check at the start of
   `tools/board.sh am62p` that reads a marker written over ssh to `ttyS2` back
   from `BOARD_UART` and stops with "BOARD_UART is not main_uart0" when it is
   empty. It would turn the long silent runs (`am62p-console-empty.md`, section
   6) into a 5 s failure.

## Left open

- Wiring (which port of the bridge is wired to which board connector) is
  inferred from the probes; the bridge was not opened or reconfigured.
- `ttyS2` was not written by this session (a getty holds it). That it does not
  reach port-32 rests on the two earlier records and on the getty printing
  nothing on port-32 here.
- `ttyS4` (`0x02860000`) has no `/dev` node and was not probed.
