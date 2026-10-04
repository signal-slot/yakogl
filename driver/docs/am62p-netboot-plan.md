# AM62P: boot Zephyr from U-Boot over TFTP (plan, 2026-09-29)

Goal (user, 2026-09-29): replace "U-Boot -> Linux -> kexec-zephyr" on the Verdin
AM62P with a host-controlled boot of the Zephyr image straight from U-Boot.
The rules stay: never brick the board, every Zephyr boot is bounded by a
watchdog and falls back to Linux on its own, and the board is shared.

Status: **the env route ran end to end on the board on 2026-10-03: install,
probe (section 9), the RTI proof image (step B) PASS, the self-arm gl_test
(step C) with the same per-case verdicts as the kexec control, and the
uninstall (section 10).** Every board
fact in sections 1-8 was read from Linux on the board (read-only), from the
U-Boot binary or from a saved UART log. What only a board
run can settle is marked **unmeasured**.

Tools: `tools/board.sh am62p` (the runner, backend uboot-tftp; it replaced the board's own netboot runner in Step 1 of `one-method.md`), `tools/tftp_oneshot.py` (the
TFTP server), `tools/am62p_netboot_uart.py` (UART capture and phase scan).

## 1. What the board has today (read 2026-09-29)

Record: `records/am62p/netboot/env-2026-09-29.txt` (sha256 `6b62d72f053dc6f5...`,
52 lines, MACs and serials redacted). Saved next to it: the U-Boot control DT
decompiled from the U-Boot image (`uboot-control-dt-2026-09-29.dts`), and a
complete kexec cycle's UART log that contains a whole U-Boot boot
(`uboot-boot-sample-uart-v16.log`, copied from am62p-doutd `doutd/logs/uart-v16.log`).

| Fact | Value | Source |
|---|---|---|
| U-Boot | `2024.04-ti-g11cb0162a795` (TI tree, Toradex build) | `ver=` in the env, banner in the UART log |
| Env storage | `/dev/emmc-boot0` offset `-0x2200`, size `0x2000`, **one copy (not redundant)** | `/etc/fw_env.config` on the board |
| Linux tool | libubootenv `fw_setenv` 0.3.5, with `-s <script>` (many variables in one write) | `fw_setenv --help` on the board |
| `bootdelay` | `0` (the compiled-in default is `1`) | env; strings of `u-boot.img` |
| `bootcmd` | the **SafeUI** persistent boot: `ext4load` `/safeui/r5_ddr.bin`, `/safeui/r5_tcm.bin`, `/safeui/zephyr_a53.bin`, then `dcache off && icache off && go 0x82000000`; on any failure `bootflow scan -b` (Torizon Linux) | env; SafeUI repo `docs/uboot-boot.md` ("電源投入で自動起動", 2026-09-06) |
| `bootcmd_orig` | `bootflow scan -b` | env |
| compiled-in default `bootcmd` | `bootflow scan -b` (what a bad env CRC falls back to) | strings of `u-boot.img` |
| `/sysroot/safeui/` | `r5_ddr.bin`, `r5_tcm.bin`, `r5_layout.txt`, `zephyr_a53.bin.safeui-bak`. There is **no `zephyr_a53.bin`**, so today's `bootcmd` falls through to Linux | `ls` on the board |
| bootcount | `bootcount=1`, `bootlimit=3`, `altbootcmd=env set rollback 1; run bootcmd`, **no `upgrade_available`**: Torizon's OTA rollback, inactive | env |
| `preboot` | Toradex's `fdtfile` default | env |
| Network | `ethact=ethernet@8000000port@1`, `ethprime=eth0`. Linux `ethernet0` has the same MAC as `ethaddr`, so U-Boot's DHCP gets the board's usual lease. No `serverip`/`ipaddr` in the env | env; `/sys/class/net/ethernet0/address` |
| Addresses | `loadaddr`/`kernel_addr_r=0x88200000`, `fdt_addr_r=0x90200000`; Zephyr is linked at **0x82000000** | env; SafeUI A53 board DTS |
| Watchdog in U-Boot | **none**: no `rti_wdt` driver, no `wdt` command, no `watchdog` uclass | strings of `u-boot.img`, checked against positives found the same way (`am65_cpsw_nuss`, `cpsw_mdio`, `mmc_blk`) |
| Network commands | `dhcp`, `tftpboot [loadAddress] [[hostIPaddr:]bootfilename]`, `ping`; env `tftpblocksize`, `tftpwindowsize`, `tftptimeout`, `tftptimeoutcountmax`, `netretry`, `autoload`. **No `tftpdstp`** (always UDP port 69), no `wget`; fastboot over UDP/TCP is present | strings of `u-boot.img` |
| Ethernet in U-Boot | `Net: eth0: ethernet@8000000port@1 [PRIME]`, then `Could not get PHY for mdio@f00: addr 7` and `ethernet@8000000port@2: phy_connect() failed` | the saved UART log |

The last row corrects a SafeUI note (`docs/uboot-boot.md`, "新イメージの書き込みに
Linux が要る理由"). It read "Could not get PHY" as "U-Boot's Ethernet does not
work". But the error is for **port 2** (PHY address 7, the second Ethernet, not
the one in use). Port 1 (PHY address 0) registers as `eth0 [PRIME]`. Whether
`dhcp` and `tftpboot` then work is **unmeasured**. The first board step
(`probe`, section 7) is there to settle it.

### A trap in the current `bootcmd`

Today a file named `/sysroot/safeui/zephyr_a53.bin` makes **every** boot go to
Zephyr, and Zephyr cannot delete it. The board would then never come back to
Linux without someone at the UART. The netboot never writes that path. Nobody
should stage an image under that name while the SafeUI boot is in `bootcmd`.

## 2. The boot flow: a one-shot armed from Linux

The model is the Pi 4's tryboot (`rpi4-v3d-backend-plan.md`, "Board port" and
the tryboot rows): the next boot runs the test image once, and the boot after
that is the normal one, whatever happens. U-Boot has no firmware flag like
tryboot, so the one-shot is an env variable that **U-Boot itself deletes and
saves before it loads anything**.

**Install (once per board, one env write; the old env is saved first):**

```
yako_bootcmd_prev = <the bootcmd as it was>
yako_addr         = 0x82000000
yako_hook         = if test -n "${yako_once}"; then
                      echo yakogl-netboot: once=${yako_once};
                      setenv yako_once;
                      if saveenv; then echo yakogl-netboot: cleared; run yako_${yako_src};
                      else echo yakogl-netboot: saveenv failed, booting Linux; fi;
                    fi;
                    <the default: docs/standalone-boot.md>
yako_net          = setenv autoload no; setenv netretry no;
                    setenv tftptimeout 1000; setenv tftptimeoutcountmax 5;
                    setenv tftpblocksize 1468; setenv tftpwindowsize 8;
                    if dhcp && tftpboot ${yako_addr} ${yako_server}:${yako_file}; then
                      echo yakogl-netboot: go ${yako_addr};
                      dcache off; icache off; go ${yako_addr};
                    fi;
                    echo yakogl-netboot: fetch failed, booting Linux
bootcmd           = run yako_hook; <the bootcmd as it was>
```

Each value is one line in the env; the line breaks above are only for reading.
`tools/uboot-netboot-lib.sh` (shared with the i.MX8MP runner,
`imx8mp-netboot-plan.md`) holds the exact strings, and `tools/board.sh`
checks the board's copy against them before every run. The hook also loads
from the eMMC and boots a default (`yako_src`, `yako_mmc`, `yako_dflt`):
`docs/standalone-boot.md` has its whole form; the block above is the TFTP
part.

With `yako_once` empty, the hook does nothing and the old `bootcmd` runs as
before. Install and uninstall each change `bootcmd` once. Uninstall writes
`yako_bootcmd_prev` back from Linux with `fw_setenv`, which stores the text
exactly. So U-Boot never has to copy the old `bootcmd` through its shell's
quoting (which would expand `${filesize}` and split on `;`).

**Arm (every run, one env write from Linux):** `yako_once=<token>`,
`yako_src=net`, `yako_file=yakogl/<token>.bin`, `yako_server=<this host>`,
then read back.

**One cycle:** reboot -> SPL -> U-Boot -> the hook sees `yako_once` -> deletes it
-> `saveenv` -> only if that succeeded: DHCP, TFTP, `go` -> Zephyr -> Zephyr ends in
a SoC reset (its own `sys_reboot`, or the RTI) -> SPL -> U-Boot -> the hook sees
nothing -> old `bootcmd` -> Linux.

Why not the mechanisms already in the env:

- `altbootcmd`/`bootcount`/`bootlimit` belong to Torizon's OTA rollback. When
  `altbootcmd` runs, it sets `rollback 1`, which tells the OS to roll back its
  deployment. Borrowing it would put Torizon's update state at risk. Not used.
- `preboot` would also work (it runs before autoboot), but it already holds
  Toradex's `fdtfile` logic. `bootcmd` already carries one local change
  (SafeUI's), so our one local change goes in the same place, in front.
- Interrupting U-Boot over the UART bridge and typing commands (what SafeUI's
  `uboot-boot-zephyr.sh` does) needs no env change. But if the host or the
  bridge fails half-way, the board waits at the U-Boot prompt, with no
  watchdog, until a person comes. Not used for unattended runs.

Sources: U-Boot environment variables (`bootdelay`, `serverip`, `autoload`,
`netretry`, `tftpblocksize`, `tftpwindowsize`, `tftptimeout`,
`tftptimeoutcountmax`): <https://docs.u-boot.org/en/latest/usage/environment.html>.
`if` with any command's exit status as the condition ("Any command. The test
statement set the $? variable"): <https://docs.u-boot.org/en/latest/usage/cmd/if.html>.
`test`: <https://docs.u-boot.org/en/latest/usage/cmd/test.html>.
`env`/`setenv` and saving: <https://docs.u-boot.org/en/latest/usage/cmd/env.html>.
`fw_setenv`/`fw_printenv` from Linux on Toradex modules, and that `setenv` in
U-Boot changes only RAM until `saveenv`:
<https://developer.toradex.com/linux-bsp/os-development/boot/u-boot/> and
<https://developer.toradex.com/linux-bsp/os-development/boot/best-practices-with-u-boot-environment-variables/>.
The `fw_setenv -s` script syntax: libubootenv <https://github.com/sbabic/libubootenv>
and the board's own `fw_setenv --help`.

## 3. How U-Boot loads and starts the image

- `tftpboot 0x82000000 <host>:yakogl/<token>.bin`. The docs.u-boot.org pages for
  `tftpboot`, `dhcp` and `go` return 404 today. The commands are in `cmd/net.c`
  and `cmd/boot.c` of U-Boot v2024.04
  (<https://github.com/u-boot/u-boot/blob/v2024.04/cmd/net.c>,
  <https://github.com/u-boot/u-boot/blob/v2024.04/cmd/boot.c>). The board's own
  help text confirms the `[[hostIPaddr:]bootfilename]` form.
- **`go`, not `booti`**: `booti` reads `text_offset=0` from the AArch64 header
  and moves the image to 0x80000000 (TF-A), and it does not start. With
  `dcache off; icache off; go 0x82000000` it does. SafeUI measured this on this
  board on 2026-09-05 (`docs/uboot-boot.md`, "ハマりどころ").
- **No dtb**: the Zephyr image has its devicetree built in; `go` passes none.
- The image is the same `zephyr.bin` the kexec route uses (built with
  `CONFIG_AARCH64_IMAGE_HEADER=y`, linked at 0x82000000). The runner refuses an
  image without the header's magic. It also refuses an image whose `image_size`
  (header bytes 16..23) runs past the GPU carve-out at 0x97800000 (344 MiB; a
  gl_test image is 106 MiB).
- The load address is not in U-Boot's reserved list: SafeUI's `ext4load` to
  0x82000000 worked, and U-Boot applies the same reserved-memory check ("trying
  to overwrite reserved memory") to `tftpboot`. **Unmeasured for TFTP.**
- DHCP gives the board its usual lease (same MAC). `netretry no` makes a failed
  `dhcp` or `tftpboot` fail once instead of retrying, as the environment page
  defines. Without it, a host that is down would keep U-Boot retrying, and
  U-Boot has no watchdog.

What this entry leaves different from kexec (a gain as well as a risk): no Linux
ran before the image. So there is no CPSW receive DMA writing into Zephyr's
memory, no split-EOI GIC mode, no tidss shutdown and no runtime-PM state left
behind (`driver/tools/kexec-zephyr/README.md` lists what Linux leaves). The GPU
and display paths were first brought up on this board under a U-Boot `go` entry
(SafeUI `docs/gpu-port-plan.md`, "Reflash loop"). Whether today's gl_test image
behaves the same under both entries is **unmeasured**; step C in section 7
compares the two on the same image.

## 4. Watchdog: the part the host cannot provide

- Under kexec, the image's only way back is the RTI watchdog **Linux** started
  (systemd `RuntimeWatchdogUSec=30s`; `watchdog0` "K3 RTI Watchdog", 60 s). The
  SafeUI SoC feeder (`soc/ti/k3/am62px/soc_a53_wdt.c`,
  `CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT`) feeds it for a bounded time
  (`..._SECONDS`: 150 for gl_test, 600 for deqp), then lets it reset the SoC.
- Under U-Boot `go`, **nobody starts the RTI**. This U-Boot has no watchdog
  driver (section 1). The feeder then logs "RTI watchdog not running, nothing to
  feed" and returns, as its Kconfig help says ("Does nothing when the watchdog
  is not running (a U-Boot 'go' boot)"). A hang would then hold the board until
  someone cuts the power. That happened once with kexec, on 2026-09-23; since
  then the loader refuses to jump without a running RTI.
- So **the image must arm the RTI itself** before a netboot is allowed. This is
  Z1: `driver/samples/zephyr/common/rti_arm.c`, behind
  `CONFIG_YAKOGL_AM62P_RTI_SELF_ARM` (default n; it requires the SafeUI feeder
  off, see below). At `POST_KERNEL` 47 (a `BUILD_ASSERT` keeps it after TISCI):
  1. TISCI device **125** (MAIN RTI0: `power-domains = <&k3_pds 0x7d 1>` in the
     U-Boot control DT) on, unless it already is. This is done in the arming
     code, not in the board's `tisci_power_on` list (`devices = <103 77 78>` in
     the SafeUI A53 DTS): the list belongs to the SafeUI board, every sample
     would have to override it, and here it is tied to the option and comes
     before the first RTI register read.
  2. `DWDCTRL` already `0xa98559da` (a kexec entry: Linux started it): feed
     only, for the same bound, and change nothing.
  3. Otherwise: set clock 125:0's parent to 2, read the rate back through
     TISCI, refuse a rate the board did not report or one that cannot hold
     60 s, program the preload from that rate, the 50 % window and the NMI
     reaction, enable, and read all three registers back.
  4. Log `rti_arm: armed preload 0x.. rate .. Hz: expiry .. ms, fed for .. s`
     and feed from a timer until `CONFIG_YAKOGL_AM62P_WDT_SECONDS`, then stop
     and let the RTI reset the SoC. That symbol is the image's one bound: the
     AM62P sample confs set it (150 gl_test, gl_triangle and gl_femtovg, 200
     gl_slint, 600 deqp; the two Rust samples still set the SafeUI symbol,
     because they cannot be configured on this host, see below), and a
     `configdefault` passes it on to the SafeUI feeder for kexec images. So a
     netboot build turns the SafeUI feeder off without leaving a SafeUI value
     assigned (which gave a Kconfig warning in the first build).
  Any refusal logs `rti_arm: FAILED (...)` and resets the SoC at once: an image
  that could not arm its watchdog does not run.
- The clock, measured (Linux clock debugfs on the board, read-only,
  2026-09-29): `clk:125:0` = **32768 Hz**, parent `clk:125:2`. The other
  parents are `clk:125:1` 25 MHz, `clk:125:3` 12.5 MHz and `clk:125:4`
  32552 Hz. Zephyr's `k3-am62-main.dtsi` gives `main_rti0`
  `clock-frequency = <25000000>`, which suggests 25 MHz as the reset
  default. At 25 MHz the longest expiry is 1.34 s, so the parent must be set.
  Linux's preload `0xf0` is (60 x 32768) >> 13 = 240 (Linux's driver does not
  subtract 1; the expiry is 60.25 s). Z1 writes `0xef` (exactly 60.000 s).
  Whether the parent is already 2 after U-Boot is **unmeasured**; the code
  does not depend on it.
- The SafeUI feeder must be **off** in a self-arm image (Kconfig
  `depends on !SOC_AM62PX_A53_FEED_INHERITED_WDT`). It reads `DWDCTRL` at
  `POST_KERNEL` 0, before any TISCI power-on. On a U-Boot entry, RTI0 may be
  powered off at that point, and that read could abort. Z1 covers the kexec
  entry itself (step 2 above), so one image serves both entries.
- Built (2026-09-29, `build-gltest.sh`, online compiler, tree `5abeee3f-dirty`):

  | gl_test image | options | `zephyr.bin` bytes | sha256 | marker |
  |---|---|---|---|---|
  | control (kexec) | none | 1 980 744 | `816e7467b6d30ae6...` | absent |
  | self-arm | `FEED_INHERITED_WDT=n`, `RTI_SELF_ARM=y` | 1 980 744 | `ae421f899077284329e221369d2bf249f221a430362af1bd63ee0dde2d5b5eb5` | present |
  | RTI proof (step B) | as self-arm, plus `RTI_PROOF=y`, `PVR_SAMPLE_REBOOT=n` | 1 980 724 | `aa6b4d7f36e772e828c30e17f95d5db4351a3de8f5592aaed98b08bf90562cc7` | present |

  All three builds show the same 11 gcc/Kconfig warnings, all pre-existing
  (unused cases in `gl_test.c`, `pvr_fw_boot.c`, the SafeUI DSI driver,
  Zephyr's `tisci.c`); none is in the new files. The control is byte-identical
  (same sha256) before and after the Kconfig change, so kexec images do not
  change. For deqp, gl_femtovg, gl_slint and gl_triangle a configure-only build
  before and after gives the same `.config` except the one new
  `YAKOGL_AM62P_WDT_SECONDS` line, and the SafeUI bound keeps its value (600,
  150, 200, 150). The Rust samples need `zephyr-lang-rust`, which is not on
  this host, so their confs were left unchanged (**not checked**). The runner's
  dry run accepts both self-arm images and refuses the control.
- Host test: `tools/pvr/rti_arm_check.c` (a `check.sh` row), 18 checks with the
  expected values worked by hand and recomputed with python: 32768 Hz ->
  `0xef`, 60000 ms; 32552 Hz -> `0xed`, 59894 ms; 25 MHz and 12.5 MHz refused
  for 60 s (25 MHz: 183105 units > 4096; longest 1342 ms); unknown rates
  refused; the 12-bit edges. A deliberately wrong expectation makes it fail.
- What resets the SoC when the bound expires: the RTI expiry goes through the
  ESM to the SoC reset. The boot loader sets up the ESM before Linux starts (the
  U-Boot DT carries `esm@420000` and `esm@4100000` with `ti,esm-pins`). Every
  kexec rc=6 run ("RTI feeder lifetime over", then `U-Boot SPL` in the log)
  shows that path working. The same boot loader runs before a netboot, so the
  path should be the same. Step B in section 7 proves it directly.
- **Residual gap**: between `go` and priority 47, nothing bounds a hang. SafeUI
  measured about 0.11 s from the Zephyr banner to the application, so this
  window is short, but it is not zero. With kexec there is no such window
  (Linux's RTI is already running). This is the one safety property kexec has
  and the netboot lacks. Closing it needs a watchdog in U-Boot, i.e. a rebuilt
  U-Boot, which is out of scope.

## 5. Host side

- **TFTP server**: this host has no `in.tftpd`, `tftpd`, `atftpd` or busybox.
  `dnsmasq` is installed, but it must run as root for port 69.
  `tools/tftp_oneshot.py` is a plain-socket server: RFC 1350, read-only, octet
  mode, the options U-Boot sends (`blksize` RFC 2348, `tsize`/`timeout`
  RFC 2349, `windowsize` RFC 7440), paths confined to the root. It serves each
  file **once**: after the final ACK it renames the file `<name>.served`. So a
  later reboot, by anyone, gets "File not found" and goes to Linux. It also
  refuses a file staged more than `--max-age` seconds ago (default 300; the
  runner stages seconds before it reboots), so an image left behind by a runner
  that was killed hard is never served later.
- **Port 69 is the blocker**: U-Boot has no `tftpdstp`, so it always sends to
  port 69. On this host `net.ipv4.ip_unprivileged_port_start=1024`, and we run
  as uid 1000. Every option needs the user's OK: run the server with `sudo`;
  `setcap cap_net_bind_service` on a copy of python; lower the sysctl; or run it
  in a host-network container from an image that is already local
  (`python:3.11-slim`; the user is in the `docker` group, so this is root in
  effect). Nothing was installed or changed on this host.
- **DHCP**: the board's existing lease is enough. U-Boot's `dhcp` with
  `autoload no` only takes the address; the `tftpboot` argument names the
  server, so nothing is taken from DHCP.
- **Runner** `tools/board.sh am62p` (until Step 1 the board's own netboot runner) has `status`, `install`, `uninstall`,
  `disarm`, `probe` and `run`. `--dry-run` prints every board, TFTP-root and
  bridge action and runs none of them. `run` checks the image (header,
  footprint, the `rti_arm: armed` marker). It checks with curl that the server
  answers on port 69. It takes the board lock and checks that `/dev/pvrgpu` is
  free, that the install matches and that nothing is pending. Then it stages
  the image, starts the capture, arms, reboots, waits for a kernel start after
  the cycle, then for ssh. Finally it checks that `yako_once` is gone and prints
  one line with the scan verdict and the phase times. Any failure between
  arming and the reboot disarms first.
- **UART**: `tools/am62p_netboot_uart.py capture` keeps the raw bytes, and a
  copy with a time on every line. `scan` walks the phases (SPL, hook, cleared,
  fetched, go, Zephyr banner, `rti_arm: armed`, reset, kernel) and gives one
  verdict per missing phase. An empty log is `empty`, never a pass.

Tested on the host (2026-09-29, branch am62p/netboot):

- `tftp_oneshot.py --self-test`: 17 checks, 20/20 runs PASS. They cover
  16 MiB + 1 byte with windows, an exact multiple of the block size, plain
  512-byte blocks, lost DATA packets, block numbers past 65535, the one-shot
  rename, a second request refused, four paths outside the root refused, a
  write refused, a file staged 301 s ago refused, and one `done` per offered
  file. The first version failed
  19/20 on the lost-packet case: while catching up on duplicate ACKs, the server
  dropped the client's final ACK as stale. That is fixed, and duplicate ACKs no
  longer resend the window.
- An independent client: `curl tftp://<host LAN address>:16969/...` fetched a
  real AM62P Zephyr image (the dEQP image of chain `g3-396d667a` img1,
  16 107 024 bytes) byte-equal, with 512-byte and with
  1468-byte blocks (15.2 and 42.7 MiB/s, host to host). A second fetch got
  "not found" (curl rc 68). This is not a board rate: U-Boot's TFTP rate on this
  board is **unmeasured**.
- `am62p_netboot_uart.py --self-test`: 17 checks (18 with the saved log), 20/20
  PASS, on a synthetic cycle built from real U-Boot and Zephyr lines. On the
  saved kexec log (278 lines) it reports `not_armed` and claims no netboot
  phase. `capture` against a local replay of that log: raw bytes equal, 295 of
  295 lines stamped, including the unterminated `login:` line, both when the
  bridge closes and on SIGTERM. "busy" and an unreachable bridge exit 4.
- `tools/board.sh --dry-run am62p` for every subcommand (tools/board_selftest.sh). `run` on an image without
  the marker is refused (rc 3). Dry runs create nothing.

## 6. Cycle time

Today, measured: in a back-to-back chain (am62p-land `g3-396d667a`, img5 to
img23, 2026-09-26) the next jump started at Linux uptime **32-48 s** (mostly
32-38 s). `kexec_run.sh` prints the uptime before each jump, and it includes
the wait for `systemctl is-system-running`. So the Linux leg costs about 35 s
per image, plus the firmware before the kernel (SafeUI measured SPL to its
Zephyr banner at 2.75 s, 2026-09-06). The `back_after=154s` rows of the gl_test
runners are not a board time: gl_test prints no dEQP summary, so rc 4 waits out
`RUN_BOUND`=150 s. That number measures the runner.

Predicted for the netboot (**unmeasured**):

| Leg | kexec | one-shot netboot |
|---|---|---|
| Linux to image | scp + ssh + kexec, a few s | fw_setenv + Linux shutdown (est. 5-10 s) + SPL/U-Boot ~3 s + DHCP ~1-2 s + TFTP of 16 MB (est. 1-5 s) |
| image run | same | same |
| back to Linux | reset + ~35 s | reset + ~35 s |

So **the one-shot alone does not make a cycle faster**. It trades kexec's jump
for a reboot; what it gains is a cleaner hardware state, not time. The time
saving comes from not booting Linux between images: a **chain mode**. There the
hook also fires on a persistent `yako_chain` flag, and the host's server hands
out a queue of images one by one; an empty queue answers "File not found" and
sends the board to Linux. Predicted per image: about 5-10 s of boot legs instead
of about 38 s. That saves about 30 s per image, about 15 minutes over a
30-image dEQP chain. Chain mode is designed here, not written. It needs the
one-shot proven first, and its flag stays in the env between boots, so it
needs the lead's OK.

### Chain mode: what it would cost

| Part | Change | Size (estimate) |
|---|---|---|
| U-Boot env | `yako_hook` also fires on `yako_chain` (set once from Linux for the whole chain; no env write per image, so no eMMC wear per image). It fetches the fixed name `yakogl/chain/next.bin` | the two hook strings in `uboot-netboot-lib.sh` plus its install check, ~10 lines; one re-install (one env write) |
| Server | a queue directory `chain/0001.bin, 0002.bin, ...`. An RRQ for `chain/next.bin` serves the lowest number once and renames it; an empty queue answers "File not found", which sends the board to Linux. The age limit applies per file (measured from the moment it becomes the head) | ~50 lines in `tftp_oneshot.py` plus ~40 lines of self-test (order, empty queue, a killed host) |
| UART scan | one capture holds N cycles: split it at each hook line, one verdict per image; `looped` becomes "the same file twice" | ~80 lines plus tests |
| Runner | `chain <image>...`: check every image, stage the queue, arm `yako_chain`, reboot once, follow the cycles in the capture, clear `yako_chain` from Linux at the end (the board is in Linux when the queue is empty), one row per image | ~120 lines |
| Plan | the chain rows of the failure table | ~30 lines |

In total about 300-350 lines and about one working day with the tests, then
one board session of about 30 minutes: a 3-image chain, and a chain where the
host's server is stopped half-way (the next boot must time out into Linux).

Risks it adds: while `yako_chain` is set, **every** reboot of the board, by
anyone, tries DHCP and TFTP first (about 1-7 s, longer when the host is gone:
DHCP plus 5 s of TFTP timeouts) and boots an image if the queue has one. The
queue is on our host and has the age limit, and the runner clears the flag
when the queue is empty. But a runner killed mid-chain leaves the flag set
until the next `run`/`chain` or a `disarm`. Gain: about 30 s per image, about
15 minutes per 30-image chain, and no Linux between images (the images start
from the same clean state).

## 7. On "go": the board steps, in order

Preconditions (not board-touching): the port-69 decision (section 5), and for
steps B and C the Z1 image (section 4). Each step checks that the board is free
first (the runner does this; `am62p-dscap` was using the board on 2026-09-29).

0. `tools/board.sh am62p status` (read-only).
1. `tools/board.sh am62p install`: saves the whole env to
   `records/am62p/netboot/env-before-install-<time>.txt`, writes the five
   variables in one `fw_setenv -s`, reads them back. No reboot.
2. **A. `probe`**: arms a token whose file is never staged, and reboots once.
   Pass: verdict `fetch_failed` with `DHCP client bound` and `File not found`
   in the log, and `yako_once` gone afterwards. No image runs, so no watchdog
   is needed. It proves the hook, `saveenv`, the port-1 PHY, DHCP, the TFTP path
   and the way back to Linux. If DHCP fails here, the PHY question is answered
   "no": uninstall and stay on kexec.
3. **B. The RTI proof image**: the checklist below. One run.
4. **C. gl_test**: the default set under netboot, then the same image under
   kexec. The case results must match.
5. If anything fails: `disarm` (a pending one-shot) or `uninstall` (the hook),
   both from Linux.

### Step B checklist: the RTI proof

The image: gl_test with `CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n`,
`CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y`, `CONFIG_YAKOGL_AM62P_RTI_PROOF=y`,
`CONFIG_PVR_SAMPLE_REBOOT=n` (sha256 `aa6b4d7f...` when built from this
branch). It arms the RTI and never feeds it. `PVR_SAMPLE_REBOOT=n` keeps the
sample's own reset at its end from coming first and proving nothing. If the
RTI has not reset the SoC 90 s after arming, the image logs
`rti_arm: proof FAILED: no RTI reset within 90 s ...` and resets itself by
software.

Before:
- [ ] Steps 0-2 passed (the hook, DHCP and TFTP work; the probe returned to
      Linux).
- [ ] A person who can switch the board's power off and on is named, and is
      at the board (or on a call next to it) for the ~3 minutes of this run.
      Why, with the software backstop: the backstop covers "the RTI does not
      reset the SoC"; it does not cover a CPU that hangs before
      `POST_KERNEL` 47 or inside the arming itself (the residual gap, section 4).
- [ ] The board is free (the runner checks `/dev/pvrgpu`; ask the other
      agents' owner first).
- [ ] The dmesg capture rule does not apply (no module loads), but the UART
      capture must start before the reboot (the runner does this).

Run: `tools/board.sh am62p run <proof zephyr.bin> <logdir> rtiproof`.

Expected, in the stamped UART log (times relative to the first SPL banner):
- `yakogl-netboot: once=...`, `cleared`, `Bytes transferred = 1980724`,
  `## Starting application at 0x82000000`, the Zephyr banner;
- `rti_arm: armed preload 0xef rate 32768 Hz: expiry 60000 ms, fed for 0 s`;
- `rti_arm: proof: not feeding ...`;
- the next `U-Boot SPL` banner **60 s (+-1 s) after the `armed` line**; then
  the hook does nothing and Linux starts; the runner's verdict is `done`.

Verdicts:
- PASS: the reset comes 59-61 s after `armed`, and there is no `proof FAILED` line.
- FAIL, safe: `proof FAILED` then a reset at about 90 s: the RTI is armed as
  read back, but its expiry does not reset the SoC on this entry (the ESM path
  is not as on the kexec entry). Netboot stays off until that is understood;
  the board is back in Linux without help.
- FAIL, safe: `rti_arm: FAILED (...)` and an immediate reset: arming was
  refused (rate, TISCI, read-back). The line says which.
- FAIL, needs the person: no reset within 120 s after `go` (the runner's
  bound for this run should be about 300 s). The person switches the power
  off and on; U-Boot then finds `yako_once` already cleared and boots Linux.
  Record the last UART line: it says how far the image got.

Afterwards: record the log under `records/am62p/netboot/`, with the image
sha256 and the armed-to-reset time.

## 8. Failure analysis: why none of these bricks the board

| Failure | What happens | Why it is safe |
|---|---|---|
| `fw_setenv` (arm or install) interrupted | The env has one copy: a half-written copy fails its CRC. U-Boot prints "using default environment" (the string is in the binary) and runs the compiled-in `bootcmd=bootflow scan -b`, i.e. Linux | The SafeUI `bootcmd` and our variables are lost, not the boot. The runner saved the env before install, and Linux-side `fw_setenv` restores it |
| `saveenv` in U-Boot fails | `if saveenv` is false: echo, skip the image, old `bootcmd`, Linux | The image loads only after the clear is on eMMC, so a one-shot cleared only in RAM can never loop |
| `saveenv` interrupted by power loss | Bad CRC, default env, Linux | Same as the first row |
| DHCP fails (PHY, no lease) | With `netretry no` it fails once: echo, Linux | U-Boot has no watchdog, so an endless retry would be a hang; `netretry no` removes it |
| TFTP server down | 5 timeouts of 1 s, then fail, Linux | Bounded by `tftptimeout`, `tftptimeoutcountmax` and `netretry` |
| File not staged, or already served | Immediate "File not found", Linux | The server is one-shot too: a stray reboot by anyone cannot boot our image twice |
| Transfer cut half-way | `tftpboot` fails, no `go`, Linux | `go` runs only after `tftpboot` succeeded (`&&`) |
| Image hangs after priority 47 | The RTI it armed resets the SoC after its bounded feeding; next boot the hook has nothing to do, Linux | Needs Z1; the runner refuses images without the marker |
| Image hangs before priority 47 | **Nothing resets it** | The residual gap (section 4): short (about 0.1 s of boot), but a person must be able to power-cycle the board during step B |
| Image ends normally | `sys_reboot`, SPL, U-Boot, empty hook, Linux | The one-shot was cleared before the image loaded |
| The runner dies after arming, before the reboot | Its trap disarms and removes the staged file. If it is killed hard (no trap), `yako_once` stays set and the image stays in the TFTP root; the next reboot by anyone fires the hook. The server refuses the image once it is older than `--max-age` (300 s), so that boot gets "File not found" and goes to Linux, and the one-shot is cleared by that boot. Inside the first 300 s such a reboot WOULD boot the image, which is then bounded by its own RTI arming like any run | Two independent limits: the env one-shot and the server's one-shot plus age limit. The next `run` also refuses while `yako_once` is set |
| The host goes away during a cycle | TFTP timeouts, then Linux | The way back needs no host |
| Someone stages `/sysroot/safeui/zephyr_a53.bin` | Every boot runs SafeUI's Zephyr | Not caused by this plan (it never writes that path), but the trap is real: section 1 |

What could still need a person: a hang in the first 0.1 s of an image
(section 4), and broken arming in step B.

## 9. Board run 2026-10-03: env route, probe

Steps 0, 1, A and an uninstall, with the user's approval of the env route
(one install write, the probe, the uninstall right after). The `fw_setenv`
steps ran from the lead's session; this agent's session was refused them by
its permission check. Runner build `713343ac` (its first line). No image ran.
Files (main checkout, `records/am62p/netboot/`, gitignored, sha256 in
`INDEX.txt`): `env-saved-20261003-153109.raw` (the whole env before the
install, 46 lines, unredacted because it is the restore and diff reference,
mode 600) and `probe-20261003-153604/`: the runner's outputs, the UART capture
(raw and stamped), `dmesg -w` of the old boot (860 lines), `journalctl -k -b 0`
of the new boot (572 lines), the TFTP server's lines, the env read-backs
(`*.raw`, mode 600). Addresses and serial numbers are redacted in the copies.

Before: Linux, uptime 2808 s, boot ID `7f644cd8...`, `kexec_loaded` 0,
`/dev/pvrgpu` absent (`pvrgpu.ko` not loaded), `yako_once` empty. The env read
at 15:00 and at 15:31 was the 46 variables of the 09-29 record, value for value
(the four identifiers the record redacts could not be compared).

1. `install` printed `am62p-netboot: installed; bootcmd = 'run yako_hook; ' +
   the saved bootcmd`. Read back: 50 variables, equal to the expected list
   (the 46 plus `yako_addr`, `yako_bootcmd_prev`, `yako_hook`, `yako_net`,
   with `bootcmd` = `run yako_hook; ` + SafeUI's `bootcmd`) in every variable
   but the four redacted identifiers, which are present in both.
2. **A. `probe`: the board did everything; the runner's verdict was wrong.**
   The UART capture (`probe.uart.ts`, times are the host's):

   ```
   1791009410.254 U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
   1791009412.441 U-Boot 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
   1791009412.804 Hit any key to stop autoboot:  0
   1791009412.804 yakogl-netboot: once=probe-none-1791009398
   1791009412.820 Saving Environment to MMC... Writing to MMC(0)... OK
   1791009412.820 yakogl-netboot: cleared
   1791009415.411 ethernet@8000000port@1 Waiting for PHY auto negotiation to complete...... done
   1791009415.414 link up on port 1, speed 1000, full duplex
   1791009416.201 DHCP client bound to address <BOARD> (785 ms)
   1791009416.230 TFTP from server <HOST>; our IP address is <BOARD>
   1791009416.234 Filename 'yakogl/probe-none-1791009398.bin'.
   1791009416.260 TFTP error: 'File not found' (1)
   1791009416.266 Not retrying...
   1791009416.266 am65_cpsw_nuss_port ethernet@8000000port@1: RX dma free_pkt failed -22
   1791009416.269 yakogl-netboot: fetch failed, booting Linux
   1791009417.409 Starting kernel ...
   ```

   The TFTP server's log (`build/tftp.log`; the first line is the runner's
   canary, the second is U-Boot on the board):

   ```
   1791009398.410 tftp: done peer=<HOST>:59643 file=yakogl/canary-probe-none-1791009398 bytes=7 ... renamed_to=canary-probe-none-1791009398.served
   1791009416.244 tftp: refused peer=<BOARD>:3689 file=yakogl/probe-none-1791009398.bin cause=not_found
   ```

   Times from "Hit any key": hook +0.0 s, `saveenv` done +0.02 s, link up
   +2.6 s (PHY auto-negotiation), DHCP bound +3.4 s, TFTP refusal +3.46 s,
   kernel start +4.6 s. From the arm (the canary, 1791009398.4): SPL after
   about 12 s (the Linux shutdown), the TFTP request after about 18 s. Boot ID
   after: `b5b65d0c...`, `yako_once` read back empty.
   So on this board: the hook fires, `saveenv` clears the one-shot before
   anything is fetched, port 1's PHY, DHCP and TFTP work in U-Boot (this
   settles the port-1 question of section 1), and the fall-through boots
   Torizon. U-Boot also prints `RX dma free_pkt failed -22` after the
   refused transfer; it did not stop the boot. Whether it matters for a
   transfer that succeeds is **unmeasured** (step C).

   The runner returned `rc=4` with `back_after=427s` and `linux=absent`. This
   is an instrument defect, not a board result: the scan of the same capture
   says `kernel_after_spl` is true. The runner waited with
   `scan --json | python3 -c ...` under `set -o pipefail`; `scan` exits 1 for
   every verdict but `done`, so the pipeline failed for every probe and the
   loop ran out `RUN_BOUND` (420 s). Reproduced on the saved capture (rc 1
   with pipefail, 0 without). Fixed by `scan --back`, whose exit status is the
   condition itself; the runner calls it with no pipe. The self-test replays
   this capture (redacted, `tools/testdata/`). (`linux` stays absent for a
   probe by design: that phase is a kernel after an image's reset; the
   report line now also prints `back=yes`.)
3. `uninstall` printed `am62p-netboot: uninstalled; bootcmd restored`. The env
   read back is byte-identical to the copy saved before the install (46 lines,
   0-line diff). Unlike the Verdin iMX8M Plus run, no identifier changed.
   The probe's `saveenv` stores U-Boot's whole RAM environment, including
   `serial#` and `carrier_serial#`, which U-Boot sets from the config blocks
   at every boot (`board/toradex/common/tdx-common.c`, `settings_r()`, read
   for U-Boot v2024.07 in `imx8mp-netboot-plan.md`; not re-read for this
   board's 2024.04-ti tree). Here the stored values were already the current
   ones, so the stored copy did not change.

Not run: steps B and C (an image, with the RTI self-arm). They wait for the
user and a person at the power switch (section 7).

## 10. Board run 2026-10-03: steps B and C (PASS)

Steps B and C ran from 16:42, build `950c1ea4` (the runner's first line),
route `env`. Records: main checkout, `records/am62p/netboot/stepsBC-20261003-1642/`
(gitignored; 19 files and an `INDEX.txt`; addresses, MACs and serials redacted
in the copies, 0 hits by grep). Board before: uptime 3943.57 s, boot ID
`b5b65d0c-...`, `clients=0` (`board-before.txt`); after the uninstall: boot ID
`408a836b-...`, uptime 17.84 s at the first read (`newboot.txt`).

### Step B: the RTI proof image: PASS

```text
netboot rtiproof: build 950c1ea4 route env image sha256 691c3e985b1f068d footprint 45846528 token rtiproof-691c3e985b1f068d-1791013366
netboot rtiproof rc=0 arm_to_reboot=0s back_after=92s verdict=done back=yes lines=2045 hooks=rtiproof-691c3e985b1f068d-1791013366 fetched_bytes=16238444 spl1=+0.0s hook=+2.6s cleared=+2.6s fetched=+6.3s go=+6.3s zephyr=+6.7s rti_armed=absent reset=+66.5s linux=+70.2s
```

TFTP: `bytes=16238444 sha256=691c3e985b1f068d
seconds=0.242 mib_per_s=64.0 windows_resent=0`. In the UART capture
(`runB/rtiproof.uart.ts`):

- line 77, 1791013384.573: `rti_arm: armed preload 0xef rate 32768 Hz:
  expiry 60000 ms, fed for 0 s`; line 78: `proof: not feeding; the RTI must
  reset the SoC within 60 s (software backstop at 90 s)`.
- line 1959, 1791013444.766: the next `U-Boot SPL` banner. That is **60.19 s**
  after the arming line, inside the 60 s expiry plus the reset's own time and
  before the 90 s software backstop. No `proof FAILED` line in the capture.
- The Linux kernel came up again 92 s after the hook (`back_after=92s`).

The RTI reset the SoC and the board returned to Linux by itself. The image
also ran `gl_test` before the reset: `gl_test: all: 1 of 38 cases failed (37
passed; 0 timed out; first failure case 48 at index 31)` (line 1951).

Instrument note: the runner's phase line says `rti_armed=absent` although the
capture has the `rti_arm: armed` line above. Its phase scan does not match
this wording; the capture is the evidence, the phase flag is not. Not fixed
here.

### Step C: self-arm gl_test: same verdicts as the control

```text
netboot selfarmC: build 950c1ea4 route env image sha256 f047d34c0b11cb05 footprint 45838336 token selfarmC-f047d34c0b11cb05-1791013634
netboot selfarmC rc=0 arm_to_reboot=1s back_after=66s verdict=done back=yes lines=2045 hooks=selfarmC-f047d34c0b11cb05-1791013634 fetched_bytes=16234368 spl1=+0.0s hook=+2.5s cleared=+2.6s fetched=+6.6s go=+6.6s zephyr=+7.1s rti_armed=absent reset=+37.5s linux=+41.2s
```

TFTP: `bytes=16234368 sha256=f047d34c0b11cb05 seconds=0.240 mib_per_s=64.4
windows_resent=0`. UART (`runC/selfarmC.uart.ts`): `rti_arm: armed preload
0xef rate 32768 Hz: expiry 60000 ms, fed for 150 s` (line 77), then

```text
gl_test: all: 1 of 38 cases failed (37 passed; 0 timed out; first failure case 48 at index 31)
gl_test: done
gl_test: FAILED (rc=-1)
I: sample done (-1): resetting the SoC
```

The reset here is the sample's own (the image resets after its 20 s hold
because `gl_test` returned -1), **not** the RTI: the RTI expiry is the
backstop, and this run did not need it. The RTI path is proven by step B.

### The control arm: the same cases under kexec

The control image (sha256 `7fda54ee...`, the same `gl_test` without the
self-arm; **this hash and the loader's `198d1338...` are the lead's, they are
not in the run directory**) ran under kexec with the AM62P kexec runner of the time (deleted in Step 1; today `BOARD_BOOT=kexec-dev tools/board.sh am62p run`)
from `.claude/worktrees/am62p-es2-full`. The capture is `ctl/ctl.uart.log`;
the per-case lines are in `ctl/cases.txt` and `runC/cases.txt` (38 lines
each). `diff runC/cases.txt ctl/cases.txt`: **0 lines**. The only failing
case in both is `gl_test: case 48 es3vol: FAIL`. That failure is
pre-existing: `RESULTS.md` records case 7's regression set as "37/38 before
and after; the one failure, case 48 `es3vol`, is pre-existing (a blue channel
off by 10 in a 3D-texture fetch)" and again as "1 of 38 failed, case 48
es3vol". Not a netboot effect.

Instrument note: that kexec runner returned `rc 4` ("no summary",
`ctl/run.out`: `ctl.uart rc=4 back_after=156s`) because it waits for a
`deqp: summary` line and `gl_test` prints `gl_test: done`. The cases were
read from the capture; the script's rc is not a board result.

### The uninstall

`uninstall.out`: `am62p-netboot: uninstalled; bootcmd restored`. The env read
back (`env-after-uninstall.txt`, 46 lines) against the copy saved at 15:31:
`uninstall-vs-saved.diff` is 0 lines. The hook is not left on this board.

### Result

Boot from U-Boot over TFTP is proven end to end on the AM62P: an image of
16.2 MB was fetched in 0.24 s and started by U-Boot (no kexec), the RTI
reset the SoC on its own within 60.19 s of arming, and `gl_test` gave the
same per-case verdicts as the kexec control (37 of 38, es3vol pre-existing).
