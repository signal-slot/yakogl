<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: why the serial console captured nothing in am62p-es2-r1

Host-only diagnosis (item am62p-console-diag, 2026-10-09, integration tip
`d23ed8d`). No board was touched. Every fact is marked **measured** (a number
or output in a committed record), **read** (read in the code or a document
today) or **inferred**. "Port-32" and "port-33" below are the two ports of
the `usb-serial-1` bridge, named by the last two digits; the host and the
full address are not written here.

## Summary

1. The prompt's premise needs correcting. The committed record
   `docs/board-sessions/am62p-es2-r1.md` is **not** a run that built image 8
   and got `verdict=empty`. It says nothing was built and nothing was run.
   The `verdict=empty ... back_after=426s` line belongs to a different record
   (`am62p-gles3-percase.md`, image 7), and that one has the same cause.
2. The most likely cause is the lease: the session was handed **port-32**,
   which is wired to mcu_uart0 (Linux ttyS3). Zephyr, U-Boot and the Linux
   getty are on main_uart0 (ttyS2). Every successful AM62P run used **port-33**.
3. **No change since the last successful AM62P capture touches the console
   path** (git evidence in section 4). The UART choice is the same in the
   devicetree, the defconfig and the dEQP image config.
4. "The board stayed in Linux" cannot be told apart from "Zephyr booted and
   was silent" without a working console: both end with Linux up and the
   one-shot cleared. The first step of am62p-es2-r1c is therefore a probe of
   the port, before any build (section 6).

## 1. What the am62p-es2-r1 record says (read)

Quoted from `docs/board-sessions/am62p-es2-r1.md`:

- Title: "AM62P dEQP-GLES2 images 8-17, ms4, gl_test (BLOCKED, nothing run on the
  board)".
- "The serial bridge named by the lease (`usb-serial-1 port-32`) reports
  "connected" but delivered 0 bytes for the whole session. No image was built
  and no dEQP case was run".
- Deployment and boot: none. "What was tried (before any build)" lists only
  `serial_status` (connected, offset 0); a CR/LF written to the console, 5 s,
  nothing; over ssh a marker to ttyS2 and to `/dev/kmsg`, nothing on the
  bridge; **an ssh reboot, 120 s wait for U-Boot, kernel or login output:
  nothing, "the board came back (ssh up)"**; `stty -F /dev/ttyS3 115200` and a
  marker to ttyS3, 5 s, nothing ("differs from the earlier record, where a
  ttyS3 probe arrived verbatim").
- Console reader result: zero bytes, no open error and no timeout. The
  bridge accepted the connection ("connected", offset 0). The reader was the
  session's console client, not `tools/board.sh`; `board.sh` was never run.
- The record's own conclusion: "Probable cause: the leased bridge port, not
  the wiring"; "Port-33 is not leased, so it was not used."

So there is no "image 8, `verdict=empty`" in this record, no deployed image,
and no `/var/yakogl` evidence in it (the task text mentions that directory
being empty; the record does not). With nothing staged, an empty
`/var/yakogl` is expected and says nothing about the console.

The `verdict=empty` line comes from `docs/board-sessions/am62p-gles3-percase.md`:

> `board.sh am62p img7 rc=4 verdict=empty boot=uboot-mmc row boot_verdict=empty
> result=none resets=absent record=absent back_after=426s`. `console.raw`,
> `console.ts` and `console.txt` are all 0 bytes, and the leased console MCP
> client of this session, connected to the same adapter, also read 0 bytes

and its table "Why the console was silent" (**measured** there): with the
lease's port-32, the data of ttyS2 (main_uart0), ttyS0 and ttyS1 never
arrived; ttyS3 (mcu_uart0) arrived verbatim (`YAKOGL-PROBE-S3`).

## 2. The console path in tools/board.sh (read)

| Step | Where | What it does |
|---|---|---|
| device | `BOARD_UART` (`board.sh:207`: `BOARD_UART=${BOARD_UART:-${UART_ADDR:-}}`) | AM62P: `host:port` of a TCP serial bridge, not a tty. Unset: `: "${BOARD_UART:?set BOARD_UART (or UART_ADDR)...}"` (lines 978, 1210) stops the run. Dry run uses `BRIDGE:PORT`. No default. |
| speed | none in board.sh | There is no `stty`, picocom, socat or ser2net for the AM62P. `stty -F ... 115200 cs8 ...` (line 822) is the **Pi 4** path only. The bridge owns the baud; `docs/direct-boot.md` documents 115200. The reader cannot detect a wrong baud: garbage would be captured (non-empty). |
| start | `run_uboot` step 5 (lines ~1075-1095), `kexec_am62p` (1228-1245) | `python3 tools/am62p_netboot_uart.py capture $BOARD_UART console.raw console.ts &`, **before** the env is armed and before the reboot. `sleep 3`, then `kill -0`: if the capture already exited, `die "the UART capture ended at once (bridge busy or unreachable)"` (the kexec route retries 12 x 10 s). |
| reader | `am62p_netboot_uart.py:223-285` | `socket.create_connection(..., timeout=10)`; unreachable returns 4; first 3 s: if the stream contains `busy`, returns 4; otherwise `recv` loop with a 1 s timeout, bytes to RAW unchanged, completed lines to STAMPED. It runs until killed or the bridge closes the stream. |
| stop | `stop_cap` after the cycle (`run_uboot` step 9) | SIGTERM; the unfinished last line is flushed. |
| the cycle's end | `run_uboot` step 8 (1143-1150) | Polls `am62p_netboot_uart.py scan console.ts --back` every 5 s until a kernel start follows the first SPL banner, or `RUN_BOUND` (default 420 s) passes; then ssh until `RUN_BOUND + RETURN_BOUND` (180 s). |
| `empty` | `verdict_line` (`board.sh:447-451`) and `scan()` (`am62p_netboot_uart.py:173`) | `bv` is the `verdict=` word of the scan of `console.ts`. The scan documents `empty` as "the log has no line at all (the capture saw nothing)" (line 69). Other cases get their own words (`no_spl`, `no_zephyr`, `no_reset`, `load_failed`). So `empty` means zero completed lines, not a tty error or a timeout. |

Environment the console path needs (all **read**): `BOARD_UART` (or
`UART_ADDR`), `BOARD_HOST` (ssh, for the env write and the return), `BOARD_BOOT`
(default for AM62P: the first of its `BOOTS`; the record used `uboot-mmc`),
`NETBOOT_SERVER` and the TFTP variables only for `uboot-tftp`, `RUN_BOUND`
(420) and `RETURN_BOUND` (180), `YAKOGL_TOOLS`. `board.sh` also needs `gawk`
(`console_from_ts`, line 911): without it `console.txt` is never derived even
when `console.ts` has data (the record saw `gawk: command not found`). That
defect breaks the post-run tally; it cannot make `console.ts` empty, and the
verdict for the AM62P is computed from `console.ts`. It belongs to the
`gate-env-fixes` item.

No script defect was found that makes a zero-byte capture on its own. The
design assumption that is fragile is that BOARD_UART points at the port wired
to main_uart0; nothing in the script checks this before it reboots the board.

## 3. Which UART the images print on (read)

- `driver/boards/toradex/verdin_am62p/verdin_am62p_am62p54_a53.dts:29-30`:
  `zephyr,console = &main_uart0;` and `zephyr,shell-uart = &main_uart0;`.
- `..._a53_defconfig:4-7`: `CONFIG_SERIAL=y`, `CONFIG_CONSOLE=y`,
  `CONFIG_UART_CONSOLE=y`, `CONFIG_UART_INTERRUPT_DRIVEN=y`.
- `driver/samples/zephyr/deqp/boards/verdin_am62p_am62p54_a53.conf:28`: "Console
  on main_uart0 (Verdin UART_3, the X18 debug USB)", with
  `CONFIG_LOG_MODE_MINIMAL=y` and the early boot markers
  (`CONFIG_SOC_AM62PX_A53_BOOT_MARKER=y`: 'H', 'K').
- `driver/samples/zephyr/deqp/prj.conf` and `tools/cts/am62p-deqp-images.sh`
  contain no console, UART or serial setting; the recipe does not change it.
- main_uart0 is what Linux calls ttyS2 (the getty and U-Boot console), **measured**
  in `am62p-gles3-percase.md`. mcu_uart0 is ttyS3.

The earlier successful sessions used the same UART (main_uart0) but on bridge
port-33: `tools/cts/RESULTS.md` section 1a ("console over usb-serial-1 port-33 ...;
port-32 is dead") and every AM62P row after it (1r, 1x, 1y, 1z ...), and
`docs/board-sessions/am62p-plan1.md` ("boot `uboot-mmc`, serial bridge
`usb-serial-1 port-33`", images 1-7 `verdict=done`, 10,092 pass, 9 fail, run
2026-10-08). So the UART did not change; the bridge port named in the lease did.

## 4. Changes since the last successful capture (git log)

Last successful AM62P capture: the plan1 session, 2026-10-08 (image tip
`b83216d4`, its record `docs/board-sessions/am62p-plan1.md`).

```
git log --format='%h %ad %s' --date=short -- tools/board.sh tools/am62p_netboot_uart.py \
    tools/cts/am62p-deqp-images.sh driver/boards/toradex/verdin_am62p driver/samples/zephyr/deqp
e6e5f14 2026-10-07 cts: dEQP image recipes checked against the current defaults; AM62P ms4 table; per-board full-tree recipe
72d1686 2026-10-06 vivante: MRT, ... (touches driver/samples/zephyr/deqp)
260711b 2026-10-06 vivante: the debug knobs leave the product ...
17c7634 2026-10-05 board.sh: uboot-mmc no longer reads NETBOOT_SERVER
c1ca07b 2026-10-04 Remove the AM62P direct-boot route: uboot-mmc is the board's standalone boot
72f06c2 2026-10-04 tools: a default that counts its boots and returns the board to Linux
e86958e 2026-10-04 tools: boot Zephyr standalone from eMMC (uboot-mmc) and make it the default
afb1633 2026-10-04 YakoGL ... (the squashed first commit)
```

**Result (measured with git log): no commit after 2026-10-08 touches these
files; the newest, `e6e5f14` (2026-10-07), is older than the successful plan1
session, so the successful run already contained it.** Of the listed commits,
none changes a console, UART or baud setting: `e6e5f14` changes
`deqp/Kconfig` and the image recipe (checked: no line mentioning console, uart,
serial or tty in its `am62p-deqp-images.sh` diff), `c1ca07b` removes the
direct-boot variant and its own board files, `17c7634` is about
`NETBOOT_SERVER`. History before 2026-10-04 is squashed into `afb1633`, so
nothing earlier can be compared. The only changes since 2026-10-08 are
documents (`am62p-es2-r1.md` and similar). Hypothesis 6 (a recent script or
recipe change) is therefore ruled out by git; what changed is the lease.

## 5. Ranked hypotheses and the one board check for each

Run the checks in this order; each is a single command, and the answers are
mutually exclusive for the first two. `$BR` is the bridge address from the
lease (`usb-serial-1`); `$SSH` is the board's ssh destination (resource
`verdin-am62p`).

### H1 (most likely): the lease names the mcu_uart0 port (port-32), not main_uart0 (port-33)

Evidence: sections 1 and 3; the previous session `am62p-gles3-percase.md`
measured it; plan1 (port-33) captured fine.
Check: print what the lease hands out, and compare the last digits.
```
echo "$RESOURCE_USB_SERIAL_1_PORT"      # the lease's port, nothing else
```
- ends in `32`: H1 holds. Use port-33 if the lease permits; otherwise stop
  and ask the owner to fix the resource (the harness cannot choose it).
- ends in `33`: H1 is ruled out; go to H2.

### H2: main_uart0 is bridged but silent or wrongly wired / the bridge is stuck

Check, only when the port ends in 33: drive the real UART from Linux while the
bridge is read in a second shell.
```
# shell A (host):   timeout 15 python3 -c 'import socket,sys; s=socket.create_connection((sys.argv[1].rsplit(":",1)[0], int(sys.argv[1].rsplit(":",1)[1])),10); s.settimeout(1)
d=b""
import time; t=time.time()
while time.time()-t<12:
    try: d+=s.recv(4096)
    except socket.timeout: pass
print(len(d), d[-60:])' "$BR"
# shell B (while A runs):  ssh "$SSH" 'sudo -n sh -c "stty -F /dev/ttyS2 115200; echo YK-PROBE-S2 > /dev/ttyS2"'
```
- A prints `N` > 0 and the tail contains `YK-PROBE-S2`: the bridge, wiring and
  baud are fine; go to H4.
- A prints `0 b''`: the bridge is connected but receives nothing from
  main_uart0: wiring, adapter, or a stuck bridge. Repeat with `ttyS3`
  (mcu_uart0): `YK-PROBE-S3` arriving means this port is the mcu one (H1 in
  disguise); nothing at all means the adapter or cable (ask the owner).
- A prints `N` > 0 but garbage (many `\x00`, `\xff`): a baud mismatch of the
  bridge: its rate is not 115200 (cannot be set from the host session).
- A fails with `busy`/refused: another client holds the bridge; stop the
  other reader first (board.sh retries 12 x 10 s only on the kexec route).

### H3: wrong tty or baud inside the board

Not a host-script matter (board.sh sets no baud on the AM62P). The Linux side
confirms which controller is ttyS2:
```
ssh "$SSH" 'cat /proc/tty/driver/serial 2>/dev/null | head -8; readlink /sys/class/tty/ttyS2/device; stty -F /dev/ttyS2 -a | head -1'
```
- `ttyS2` resolves to the controller at `2800000` (main_uart0) with
  `speed 115200`: the board side is as documented; H3 ruled out.
- another address or a different speed: the board's Linux changed; the
  `zephyr,console` choice would then need re-checking against it.

### H4: the image never booted (the board stayed in Linux) versus booted silent

Only meaningful once H1 and H2 are cleared. Before any dEQP image, use the
tiny probe: arm a one-shot for a file that does not exist (board.sh's own
probe mode: a missing file; `uboot-mmc` answers `load_failed`), or run one
small Zephyr sample, with the console on the verified port:
```
tools/board.sh am62p probe <logdir>    # verb from the board.sh header; BOARD_BOOT=uboot-mmc, BOARD_UART set
python3 tools/am62p_netboot_uart.py scan <rundir>/console.ts --json
```
- scan phases include the first SPL banner, U-Boot, and `load_failed`
  (uboot-mmc probe): the console path works end to end; an image that then
  gives `empty` truly did not print (go to H5).
- `Booting Zephyr` or `sample: build tree=` present: Zephyr did boot.
- zero bytes with port-33 proven in H2: the capture itself failed (H5) or the
  board was not reset (check `fw_printenv yako_once` was consumed and the
  uptime restarted).
Also read `ssh "$SSH" 'sudo -n fw_printenv yako_src yako_once; cut -d. -f1 /proc/uptime'`
after a run: uptime smaller than the time since the reboot means a new boot
happened; `yako_once` empty means U-Boot consumed the one-shot (the record of
gles3-percase had both, which proves a reset and U-Boot's read, not that
Zephyr ran).

### H5: capture started too late or stopped early

Read: the capture starts before the arm and reboot, and `sleep 3` plus
`kill -0` catches an early exit; the stop is after the cycle ends. A zero
byte `console.ts` on a connected bridge cannot come from timing, because
U-Boot and SPL print at every reboot. So H5 reduces to H1/H2 for an empty log;
it matters only for a truncated log (cycle ends at a kernel start after SPL).
Check: size of the three files of one run.
```
wc -c <rundir>/console.raw <rundir>/console.ts <rundir>/console.txt
```
- all 0: no bytes ever arrived: H1/H2, not timing.
- `console.raw` > 0 and `console.txt` 0: `gawk` missing (`which gawk`), the
  `console_from_ts` defect (install gawk, or cts-tool-fixes below).
- `console.raw` > 0, stops before the image's output: cycle ended early; read
  the last line against `scan --back`.

### H6: a recent change in board.sh or the image recipes

Ruled out by section 4 (no console-path commit since the last success).
Check, should anyone doubt it: `git log --since=2026-10-08 -- tools/board.sh
tools/am62p_netboot_uart.py driver/boards/toradex/verdin_am62p
driver/samples/zephyr/deqp tools/cts/am62p-deqp-images.sh` prints nothing.

## 6. Checklist for am62p-es2-r1c's first step

1. Before building: `echo "$RESOURCE_USB_SERIAL_1_PORT"`. If it ends in 32,
   do not build or reboot; the lease is the known wrong mcu_uart0 port. Report
   it (the run cannot proceed) unless port-33 is also available.
2. If 33: run the H2 probe (marker to ttyS2 while reading the bridge). It
   must show `YK-PROBE-S2`. Zero bytes: stop, report H2, do not build.
3. `ssh "$SSH" 'stty -F /dev/ttyS2 -a | head -1'` shows 115200 (H3).
4. Check `which gawk` on the host; install it or note that `console.txt` will
   not be derived.
5. Only then build images 8-17, and run `tools/board.sh am62p tree` with
   `BOARD_UART` set to the verified address. Prove the capture with the first
   row: its `console.raw` must contain the SPL banner and `Booting Zephyr`
   (or `sample: build tree=`) before the row counts.
6. If rows show `verdict=empty` after steps 1-3 passed, apply H4: probe the
   board with the missing-file probe and read the scan phases.

## 7. For the cts-tool-fixes item (no definite defect in the console reader)

- **Real but separate:** `tools/board.sh` `console_from_ts` (line 908)
  needs `gawk` (`strftime`). Change: use `python3 -I` or a portable awk to
  produce `HH:MM:SS line`, or have the host check `command -v gawk` in
  `common_prechecks` and stop early with a clear message.
- **Suggested guard (not a defect):** in `run_uboot` step 5, before arming and
  rebooting, wait up to ~10 s for any byte or write a CR to the bridge and
  require some reply; with zero bytes, stop with "console silent: check the
  lease's port (main_uart0, not mcu_uart0)" instead of rebooting the board and
  waiting `RUN_BOUND` (420 s) for nothing. The 426 s wait and an unnecessary
  reboot in `am62p-gles3-percase.md` are what such a guard would save.

## Resolution (2026-10-09)

Facts:

- The owner verified by hand that the AM62P console is `usb-serial-1`
  (100.118.139.121) port **35933**: Enter returns
  `verdin-am62p-08641335 login:`. A read-only check from a ccfarm worker the
  same day (connect, `\r`, read) got the same prompt. See
  `docs/boards/console-map.md`.
- `docs/board-sessions/am62p-es2-r1.md` (commit `324fa54`, not in every
  branch) records that the lease named `usb-serial-1:35932` (mcu_uart0, Linux
  ttyS3) and that 35933 was not leased. Nothing in `tools/` hard-codes a host
  or port: `BOARD_UART` / `UART_ADDR` come from the environment
  (`tools/board.sh:207`), so the run used whatever the lease supplied.

Cause: the run read the wrong UART channel, 35932, which accepts connections
but never carries main_uart0 output. This is H1 above. The 35932 value is
taken from the committed record, not from a dump of that session's
environment (not available here).

Fix: the repository holds no value to correct. The route is recorded in
`docs/boards/console-map.md`; the lease, or whatever sets `BOARD_UART`, must
say `100.118.139.121:35933`.

Not determined from the host side: why 35932 was also silent for the ttyS3
probe in that session (it arrived verbatim earlier).
