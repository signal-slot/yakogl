<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Standalone boot from eMMC or SD

A product powers on and boots Zephyr from its own storage: the eMMC on the
two Verdin boards, the SD card on the Raspberry Pi 4. No host, no network.
This file is that design for each board, and how we test it on the shared
boards without leaving a board that needs a person to come back.

Test runs boot an image once and come back to Linux: on the Verdin boards
U-Boot loads it from the eMMC where Linux staged it (`BOARD_BOOT=uboot-mmc`,
the backend the runs of record use; `tools/board.sh` still defaults to
`uboot-tftp`, which fetches it over TFTP), on the Pi 4 the firmware's tryboot.
Those are development runners; the TFTP side is "The TFTP runner" below, and
its bring-up records are
`docs/notes/am62p-netboot.md` and
`docs/notes/imx8mp-netboot.md`.

State: the code is in `tools/board.sh` (backend `uboot-mmc`, verbs `default`
and `nodefault`) and in `tools/uboot-netboot-lib.sh`, and
`tools/board_selftest.sh` checks it without a board. The steps ran on the
boards on 2026-10-05; the records are
`records/am62p/standalone-2026-10-05/` and `records/imx8mp/standalone-2026-10-05/`
(main checkout, not in git). What they prove:

- **Both Verdins**: the hook installed (env backups
  `env-before-install-20261005-*.txt` in those records).
- **AM62P**: `probe` (`rc=0 verdict=probe_ok boot_verdict=load_failed`,
  probe2.out), `run` (`rc=0 verdict=done boot=uboot-mmc boot_verdict=done
  resets=0 back_after=98s`, run-SO.out), and `default <image> 2` end to end:
  the console shows `boots left 1`, the image boots (`4569568 bytes read`,
  `go 0x82000000`, `Booting Zephyr OS`), then `boots left 0`, the image
  again, then `default used up, booting Linux` and Torizon (default2.raw).
- **i.MX8MP**: `probe` (`rc=0 verdict=probe_ok boot_verdict=probe_ok`,
  probe1.out) and `run` (`rc=0 verdict=done boot=uboot-mmc
  boot_verdict=cycled back_after=95s`, run-SO.out). `default <image> 2` was
  set (default2.out); the record proves the set, not the cycle — same-day
  `boot=uboot-mmc` runs after it prove no default was left in the env by
  then.
- **Pi 4**: every `board.sh pi4 run` boots the product's configuration from
  the SD card through `tryboot.txt` ("The test is the product's boot").
- Not run on any board yet: `default <image> forever` (the product form),
  and the Pi 4's literal `config.txt` swap.

## Summary

| | Verdin AM62P | Verdin iMX8M Plus | Raspberry Pi 4 |
|---|---|---|---|
| The image | a file on the eMMC's user-area partition 1 (ext4) | the same | a file on the SD card's FAT boot partition |
| What loads it | U-Boot: `ext4load mmc 0:1`, `go 0x82000000` | U-Boot: `ext4load mmc 2:1`, `go 0x60000000` | the firmware: `kernel=` in `config.txt` |
| One boot, then Linux | `BOARD_BOOT=uboot-mmc board.sh am62p run` | `BOARD_BOOT=uboot-mmc board.sh imx8mp run` | `board.sh pi4 run` (tryboot) |
| Default for N boots, then Linux by itself | `board.sh am62p default <image> N` | `board.sh imx8mp default <image> N` | not possible (no boot count in the firmware) |
| Default forever (the product) | `default <image> forever`, a person present | the same | the same |
| Undo of "forever" without Linux | at the U-Boot prompt: `setenv yako_default; saveenv; boot` (serial bridge or a person) | the same; no UART on the board, so **a person** with a cable | **a person**: the SD card in a PC, `config-linux.txt` over `config.txt` |

(`board.sh` is `tools/board.sh`.) `default` reboots nothing. It prints the
exact undo before it changes anything. `forever` refuses unless
`BOARD_PERSON_PRESENT=1`. `nodefault` undoes any default from Linux.

## Verdin AM62P and Verdin iMX8M Plus

### Where the image lives

U-Boot already reads the eMMC's user area: it loads Linux from there. So the
image is a plain file there, and U-Boot loads it with `ext4load`, as it
loads a kernel.

The eMMC boot partitions do not fit. Each is 4 MiB (8192 sectors in
`/sys/block/mmcblk*boot*/size` on both modules), and boot0 already holds the
boot loader and U-Boot's environment. Our images are 15.7 MB (`gl_triangle`)
to 106 MiB in memory (`gl_test`).

### The product form

`zephyr.bin` on an eMMC partition, and one U-Boot variable. AM62P:

```
setenv bootcmd 'ext4load mmc 0:1 0x82000000 /zephyr.bin && dcache off && icache off && go 0x82000000'
saveenv
```

i.MX8MP: the same with `mmc 2:1` and `0x60000000`. `go`, not `booti`, with
both caches off: "What the image does on a `go` entry" below says why. The
image is the same `zephyr.bin` the other routes boot (an AArch64 Image header,
linked at the load address). U-Boot runs no watchdog for it, so the image arms
its own (the RTI on the AM62P, WDOG1 on the i.MX8MP); `board.sh` refuses an
image without that.

### On the shared boards

The boards run Torizon, and Torizon's `bootcmd` stays. One hook goes in front
of it (`tools/uboot-netboot-lib.sh` has the exact strings; each value is one
line in the env):

```
bootcmd   = run yako_hook; <Torizon's bootcmd>
yako_hook = if test -n "${yako_once}"; then
              echo yakogl-netboot: once=${yako_once}; setenv yako_once;
              if saveenv; then echo yakogl-netboot: cleared; run yako_${yako_src};
              else echo yakogl-netboot: saveenv failed, booting Linux; fi;
            fi;
            if test -n "${yako_default}"; then run yako_dflt; fi
yako_dflt = if test -n "${yako_default_left}"; then
              if test ${yako_default_left} -gt 0; then
                setexpr yako_default_left ${yako_default_left} - 1;
                if saveenv; then echo yakogl-netboot: default ${yako_default}, boots left ${yako_default_left};
                  setenv yako_file ${yako_default}; run yako_mmc;
                else echo yakogl-netboot: saveenv failed, booting Linux; fi;
              else echo yakogl-netboot: default used up, booting Linux;
                setenv yako_default; setenv yako_default_left; saveenv; fi;
            else echo yakogl-netboot: default ${yako_default};
              setenv yako_file ${yako_default}; run yako_mmc; fi
yako_mmc  = if ext4load mmc 0:1 ${yako_addr} ${yako_file}; then
              echo yakogl-netboot: go ${yako_addr}; dcache off; icache off; go ${yako_addr};
            fi; echo yakogl-netboot: mmc load failed, booting Linux
yako_net  = DHCP, TFTP and go (the TFTP runner)
```

- **One boot** (`yako_once`, written from Linux by `board.sh run`): U-Boot
  deletes it and saves the env before it loads anything, then runs `yako_mmc`
  (or `yako_net`, as `yako_src` says).
- **The default** (`yako_default`, written by `board.sh default`): with a
  count in `yako_default_left`, U-Boot saves the count less one before each
  load. At 0 it deletes the default and boots Linux. Without a count it boots
  the default every time: the product form.
- **Any load that fails** falls through to Torizon's `bootcmd`, that is, Linux.

Where the file goes. Torizon's root file system, the ext4 on partition 1, is
mounted read-only, but `/var` is writable: a bind mount of the directory
`/ostree/deploy/torizon/var` of the same file system
([Toradex: OSTree](https://developer.toradex.com/torizon/in-depth/ostree/)).
Linux writes `/var/yakogl/<file>`; U-Boot reads
`/ostree/deploy/torizon/var/yakogl/<file>`. `board.sh` reads this path on the
board (`findmnt -T /var/yakogl`: device, mount point, FSROOT) instead of
assuming it, and refuses unless the device is the partition U-Boot reads
(`/dev/mmcblk0p1` = `mmc 0:1` on the AM62P, `/dev/mmcblk2p1` = `mmc 2:1` on
the i.MX8MP).

`board.sh <board> install` writes the hook in one env write and saves the
whole env first. If a board has a hook of another version, `install` replaces
the strings in one write and leaves `bootcmd` as it is. `uninstall` restores
Torizon's `bootcmd`.

### Why the default counts its own boots

The default must not be the step that strands a board. A test image ends in
its watchdog reset, so a default with a count of N boots the image N times
and then gives the board back to Linux, with no person and no host. Only the
product form (`forever`) needs a person to undo, and it asks for one. The
product form has no fallback on purpose: a product has no Linux to fall back
to, so a "good boot" mark that returns to Linux would serve only the lab.

The count is our own variable, not U-Boot's `bootcount`. On these boards
`bootcount`, `bootlimit` and `altbootcmd` belong to Torizon's update
rollback: `altbootcmd` is `env set rollback 1; run bootcmd`, and on the
i.MX8MP the count only runs while `upgrade_available` is set (both env
records). Borrowing them would tell Torizon to roll back its deployment.

The count is 1 to 9. `setexpr` computes in hexadecimal and `test -gt` reads
decimal; for one digit they agree. Both boards' own environments use
`setexpr` (`update_tiboot3`, `update_uboot`).

### A test run (`BOARD_BOOT=uboot-mmc board.sh <board> run <image> <logdir>`)

The `board.sh` sequence (its header) with the eMMC as the source:

1. Pre-checks: the hook installed, no one-shot pending, no default set (with
   one, the boot after the image would not be Linux), `/var/yakogl` on the
   right partition with twice the image's size free, no leftover
   `once-*.bin`.
2. Stage: `/var/yakogl/once-<token>.bin`, written as `.part`, synced, renamed,
   synced. The page cache is dropped and the file read back from the eMMC by
   sha256.
3. Arm: `yako_once=<token>`, `yako_src=mmc`, `yako_file=<the path U-Boot
   sees>`, read back. Reboot.
4. U-Boot clears the one-shot, loads the file, starts it. The image ends in
   its watchdog reset; the next boot has no one-shot and boots Linux.
5. In Linux: `yako_once` must be gone (exit 6 otherwise), the file is
   removed, the record is read (AM62P: the console through the serial bridge;
   i.MX8MP: the SNVS record).

The verdict line says `boot=uboot-mmc row`: the boot loader started the image
from the board's own storage.

`probe` arms a file that does not exist. On the AM62P it passes only when the
console has U-Boot's `yakogl-netboot: mmc load failed` line
(`boot_verdict=load_failed`). On the i.MX8MP it passes when the board came back
from a new boot with the one-shot cleared and the record untouched. That
proves the hook ran; it cannot tell a missing file from a missing `ext4load`
command. A real run tells.

### Why no step strands the board

| What fails | What happens |
|---|---|
| Staging or its read-back | the file is removed; nothing is armed; no reboot |
| `saveenv` in the hook | nothing is loaded; Linux boots |
| `ext4load` (no file, wrong path) | `mmc load failed, booting Linux` |
| The image hangs | its watchdog resets the SoC; the one-shot is already gone, or the count already lower; Linux boots, at the latest after N boots |
| The host dies after the reboot | nothing on the boot path needs the host |

Linux reboots through systemd, which unmounts the file system or remounts it
read-only, so the ext4 journal is on the disk before U-Boot reads the file. If
Linux crashes instead, the load can fail, and Linux boots.

### Facts and sources

- AM62P: the board's own `bootcmd` loads the stock Zephyr with
  `ext4load mmc 0:1 0x82000000 ... && dcache off && icache off && go 0x82000000`
  (env record `records/am62p/netboot/env-before-install-*.txt`, main checkout,
  not in git), a path measured on this board
  (`docs/notes/am62p-netboot.md` sections 1 and 3).
- i.MX8MP: the eMMC is U-Boot's `mmc 2` (its env's `update_uboot`: `mmc dev 2
  1`); Torizon is on partition 1 (`distro_bootpart_uuid=8725ec00-01`); the
  env's `distro_bootcmd` runs `run bootcmd_${target}`, the form of `run
  yako_${yako_src}` (env record `records/imx8mp/netboot/env-before-install-*.txt`).
- Boot partitions: 8192 sectors each, read on both boards
  (`/sys/block/mmcblk0boot*/size` on the AM62P, `mmcblk2boot*` on the i.MX8MP:
  `docs/notes/imx8mp-netboot.md` section 1).
- `ext4load`: both boards' mainline defconfigs set `CONFIG_CMD_EXT4_WRITE=y`,
  which needs `CMD_EXT4`
  ([verdin-am62p_a53_defconfig](https://github.com/u-boot/u-boot/blob/master/configs/verdin-am62p_a53_defconfig),
  [verdin-imx8mp_defconfig](https://github.com/u-boot/u-boot/blob/master/configs/verdin-imx8mp_defconfig)).
  The boards run Toradex's 2024.04 (TI tree) and 2024.07 builds; mainline is a
  reference, not a byte match.
- The `load` family, `filesize`, and the line `<n> bytes read in <t> ms` that
  the AM62P console scan reads:
  [U-Boot: load](https://docs.u-boot.org/en/latest/usage/cmd/load.html).
- The env from Linux (`fw_setenv`), and `setenv` changing only RAM until
  `saveenv`: [Toradex: U-Boot](https://developer.toradex.com/linux-bsp/os-development/boot/u-boot/).

Measured on the boards 2026-10-05 (the records at the top): `ext4load` and
the hook's nested `if` on both Verdins (the `run` rows), `setexpr` and the
count path on the AM62P (the `default ... 2` cycle decremented 1 → 0 and
deleted the default). Still unmeasured: the load time of a 100 MB file (a
4.5 MB file read in 46 ms on the AM62P, default2.raw).

### What the image does on a `go` entry

These facts hold for every U-Boot route, eMMC or TFTP. They were measured on
the boards on 2026-10-03 (the records are in
`docs/notes/am62p-netboot.md` and `imx8mp-netboot.md`).

**`go`, not `booti`.** `zephyr.bin` carries an AArch64 Image header with
`text_offset` 0. `booti` places such an image at the start of DRAM
(0x80000000 on the AM62P, where TF-A is; 0x40000000 on the i.MX8MP; U-Boot
v2024.07 `arch/arm/lib/image.c`), and the image, linked at its load address,
does not start there. `go` jumps to the load address and passes no device
tree; Zephyr has its devicetree built in. `dcache off; icache off` before
`go` cleans the D-cache and turns the MMU off, so Zephyr enters with the MMU
and caches off, as it does from kexec's purgatory. This was measured on the
AM62P on 2026-09-05; the i.MX8MP case follows from the same code and
the self-arm images ran there.

**The image arms its own watchdog.** A hang must not hold a board until a
person cuts the power.

- AM62P: this U-Boot has no watchdog driver, so after `go` the RTI is
  stopped. `CONFIG_YAKOGL_AM62P_RTI_SELF_ARM`
  (`driver/zephyr/lib/board/am62p/rti_arm.c`) powers MAIN RTI0 (TISCI device
  125) on, sets its clock parent to 2 (32768 Hz, measured; the 25 MHz
  parent cannot hold 60 s), reads the rate back, programs a 60.000 s expiry
  (preload `0xef`), enables it and feeds it until
  `CONFIG_YAKOGL_AM62P_WDT_SECONDS`. Any refusal resets the SoC at once. The
  RTI expiry reaches the SoC reset through the ESM, which the boot loader
  sets up. Measured: the proof image (armed, never fed) was reset 60.19 s
  after the arming line. One gap remains: between `go` and the arming at
  `POST_KERNEL` nothing bounds a hang (about 0.11 s). Closing it needs a
  watchdog in U-Boot.
- i.MX8MP: U-Boot starts WDOG1 itself with 60 s (`WCR` 0x773f found by the
  proof image on 2026-10-03), so there is no gap. The i.MX WDOG cannot be
  stopped once on. `CONFIG_GC_BOARD_WDOG_SELF_ARM`
  (`driver/zephyr/lib/board/imx8mp/gc_wdog_arm.c` over Rust
  `ip::imx_wdog`) arms it in either case at `PRE_KERNEL_1`: it sets 60 s
  (`WT` 0x77), clears `WMCR.PDE`, reads `WCR` back, and stops feeding after
  `CONFIG_GC_BOARD_WDOG_SECONDS`. A refusal writes a WDOG record and resets
  through PSCI. `WCR.WDT` makes the timeout assert `WDOG_B`, the board reset.

**What a cold entry lacks that kexec had.** No Linux ran before the image.
That removes hazards (no Ethernet DMA into Zephyr's memory, no split-EOI GIC
state, no stray interrupts) and also removes help. On the i.MX8MP this
U-Boot has no display driver: MEDIAMIX, the LCDIF, the DSIM, the bridge and
the backlight are as reset left them, and the GPU power domains are off. A
read of a powered-down domain is never answered; the core stalls until the
watchdog resets the SoC. That was the step C failure on 2026-10-03: the
panel console read an LCDIF register before anything powered MEDIAMIX on.
The rule since then: code that may run before its domain is powered asks
first (`soc::imx8mp::media::lcdif_unreachable` reads only the GPC and the
CCM, which always answer), and the image brings the chain up itself
(`CONFIG_GC_COMMON_PANEL_CONSOLE_COLD_BRINGUP`, the GPU power-up in Rust
`gpu::vivante`).

**The TFTP runner** (`BOARD_BOOT=uboot-tftp`, development only). The hook's
`yako_net` sets `netretry no` (a host that is down must not keep U-Boot
retrying, and U-Boot runs no watchdog on the AM62P), takes an address by
`dhcp` and fetches with `tftpboot`. U-Boot always sends to UDP port 69 (no
`tftpdstp`), so the server needs that port. `tools/tftp_oneshot.py` serves
each file once (it renames it after the final ACK) and refuses a file staged
more than `--max-age` seconds ago, so a later reboot by anyone gets "File not
found" and boots Linux. Its log is the only witness of the U-Boot leg on the
i.MX8MP, which has no UART.

## Raspberry Pi 4

### The product form

The Pi 4 boots from the SD card without U-Boot. Its boot loader is in the SPI
EEPROM and reads the card's first FAT partition
([boot EEPROM flow](https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#eeprom-boot-flow)).
The firmware reads these files (the `uart_2ndstage=1` log of our tryboot runs):

| file | from | what it is |
|---|---|---|
| `start4.elf`, `fixup4.dat` | Raspberry Pi OS boot partition | the firmware and its memory fix-up ([boot options](https://www.raspberrypi.com/documentation/computers/config_txt.html#boot-options)) |
| `config.txt` | below | the configuration |
| `bcm2711-rpi-4-b.dtb`, `overlays/overlay_map.dtb`, `cmdline.txt` | Raspberry Pi OS boot partition | read by the firmware; Zephyr uses none of them |
| `zephyr.bin` | the build | the kernel, loaded at 0x200000 |

That is the set the firmware read, not a proven minimum: keep all of it until
a card without one of them has booted.

`config.txt`:

```ini
[all]
kernel=zephyr.bin
arm_64bit=1
enable_uart=1
uart_2ndstage=1
hdmi_group=2
hdmi_mode=87
hdmi_timings=1920 1 50 44 50 720 1 10 20 50 0 0 0 60 0 99070000 1
```

| key | what it does |
|---|---|
| `kernel=` | the file to boot ([kernel](https://www.raspberrypi.com/documentation/computers/config_txt.html#kernel)) |
| `arm_64bit=1` | 64-bit, the Pi 4 default, kept explicit ([arm_64bit](https://www.raspberrypi.com/documentation/computers/config_txt.html#arm_64bit)) |
| `enable_uart=1` | the UART on GPIO 14/15, Zephyr's console ([enable_uart](https://www.raspberrypi.com/documentation/computers/config_txt.html#enable_uart)) |
| `uart_2ndstage=1` | the firmware's own log on the UART; optional ([uart_2ndstage](https://www.raspberrypi.com/documentation/computers/config_txt.html#uart_2ndstage)) |
| `hdmi_*` | the 1920x720 bar panel as a custom mode, for the firmware's display path, which Zephyr uses through the property mailbox ([legacy video options](https://www.raspberrypi.com/documentation/computers/legacy_config_txt.html#legacy-video-options)) |

A card from Raspberry Pi OS Lite (64-bit): one FAT32 partition, the files of
the table, `zephyr.bin` and this `config.txt`. No kernel, initramfs or root
file system. Record the sha256 of every file next to the build.

### The test is the product's boot

A tryboot reads `tryboot.txt` instead of `config.txt`, once
([tryboot](https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#fail-safe-os-updates-tryboot)).
Everything else is the same card, the same firmware files and the same lines:
`board.sh` writes both files from one function (`pi4_config`). So each
`board.sh pi4 run` boots the product's configuration from the SD card; only
the file name differs.

### Default: forever only, with a person

`BOARD_PERSON_PRESENT=1 board.sh pi4 default <image> forever` copies the image
to `/boot/firmware/yakogl-default.bin`, keeps Linux's `config.txt` as
`config-linux.txt` byte for byte, writes `config.txt`, and reads everything
back. It does not reboot. `nodefault` (from Linux) puts `config-linux.txt`
back. A tryboot run refuses while `config-linux.txt` exists.

The firmware has no boot count, so after the reboot only a person brings
Linux back: the card in a PC, `config-linux.txt` over `config.txt`. The EEPROM
is never written, so no image can make the board unbootable.

A count would need Zephyr to set the firmware's tryboot flag at start
(property `0x00038064`, as Linux's `drivers/firmware/raspberrypi.c` does for
`reboot "0 tryboot"`), so that its watchdog reset boots a Linux `tryboot.txt`.
There is no such code, and whether the flag survives our watchdog's reset is
unmeasured; until both are, the Pi 4 default is a person's step.

Watchdog: `rpi_wdog.c` resets the SoC at a deadline (`RPI_WDOG_HOLD_S`).
As the default, that reset boots the same image again. A product image needs
a watchdog without a deadline, fed only while the application is healthy;
the board layer does not have that mode.

## Raspberry Pi 5

The Pi 5 (BCM2712, V3D 7.1) boots the way the Pi 4 does: the boot loader in
the SPI EEPROM reads the SD card's first FAT partition, and a one-shot
`tryboot.txt` replaces `config.txt` for one boot. `tools/board.sh pi5` is the
Pi 4's tryboot backend with the three differences of
`docs/notes/pre-study-rpi5.md` section 4, and the Pi 5's timings. The Pi 4
section above applies wherever this one does not say otherwise (`board.sh`
is `tools/board.sh`).

### What differs from the Pi 4

| | Pi 4 | Pi 5 |
|---|---|---|
| kernel file | `kernel=zephyr.bin`-style, written by the same function | `kernel=` must name our file: the Pi 5's default is `kernel_2712.img` |
| `arm_64bit=1` | written | not written: the Pi 5's firmware ignores it |
| `start4.elf`, `fixup4.dat` | in the firmware set | none: the Pi 5 has no such files, nothing to keep |
| HDMI timings | `BOARD_HDMI_TIMINGS` into the config | not written, not read: `cmdline.txt` carries `video=` for the capture device, and is left as it is |
| console | GPIO 14/15 | `uart10`, the 3-pin debug connector, 115200 8N1 (Linux: `ttyAMA10`) |
| waits | 420 s for the image, 300 s for Linux | 420 s for the image, 420 s more for Linux |

### What a run does

The sequence is the one `board.sh --dry-run pi5 run <zephyr.bin> <logdir>`
prints; the board-side steps in order:

1. Take the board lease (`/var/tmp/yakogl-board.lease`, one holder per
   board), check `sudo -n true`, read `boot_id`, check that nobody holds the
   console device, that `/boot/firmware` has at least twice the image free,
   that no `/boot/firmware/yakogl-<name>.bin` is left over and that no
   `config-linux.txt` (a permanent default) exists.
2. `vcgencmd get_throttled`, recorded as `throttled_before` in `meta.txt`.
3. Read the existing `tryboot.txt` (if any), copy the image to
   `/boot/firmware/yakogl-<name>.bin`, write `tryboot.txt`, read both back
   (sha256 of the image, the text of `tryboot.txt`); on any mismatch, undo
   and stop before the reboot.
4. Start reading the console into `console.txt`, then `sudo reboot "0 tryboot"`.
5. Wait up to 420 s for the image's end line (`v3d_clear: PASS`/`FAIL`, the
   deqp and gl_test end lines, or a `yk-record:` line), then 5 s more.
6. Stop the reader and wait up to 420 s more for a `boot_id` other than the
   first. A reboot to ssh takes about 150 s on the owner's board (kernel
   11 s, user space 98 s, measured 2026-11-06), and the SD card sits behind
   a bridge that writes at about 0.4 MB/s, so the wait is long. `RUN_BOUND`
   and `RETURN_BOUND` override the two limits.
7. When Linux is back: `get_throttled` again (`throttled_after`), remove the
   staged image, put the previous `tryboot.txt` back and read both back. A
   card that had no `tryboot.txt` before the run (the stock Pi OS card) has
   none after it: step 3 records whether one existed, and the undo removes
   the run's file instead of writing an empty one.

`tryboot.txt` (the same lines `board.sh` would write to `config.txt` for a
`default`; `config.txt` itself is never touched by a run):

```ini
# tools/board.sh: <name>, one-shot boot (reboot "0 tryboot"); config.txt stays the Linux default
[all]
kernel=yakogl-<name>.bin
enable_uart=1
uart_2ndstage=1
```

`kernel=` names the staged file, since the firmware would otherwise load
`kernel_2712.img`. There is no `arm_64bit` line. `uart_2ndstage=1` puts the
boot loader's log on the same UART before the image's banner.

`get_throttled` is recorded with every run because the supply is weak (the
boot loader reports USB-PD 3000 mA, Linux logs "Undervoltage detected", the
value read is `0x50000`): a GPU result that looks like a hardware fault is
checked against it first.

Other verbs: `board.sh pi5 probe <logdir>` does the pre-checks of step 1 and
reboots nothing; `status` prints the lease, `boot_id`, free space, any
staged `yakogl-*.bin` and the current `tryboot.txt`; `disarm` lists a staged
image and `tryboot.txt` (tryboot arms nothing that persists, so a leftover is
removed by hand). `default <image> forever` and `nodefault` work as on the
Pi 4, with a person at the board.

### The environment

`BOARD_HOST` (`user@host` of the board's Linux) and `BOARD_UART` come from the
environment, never from a file; `PI5_HOST` and `PI5_UART` are read as well.
`BOARD_UART` is a serial character device wired to the Pi's debug connector,
or `host:port` of a serial bridge to it. The board has no remote power or
reset; `board.sh` only reads the console. Since 2026-11-19 the console is a
TCP serial bridge that also takes input (not needed by any run). Under a farm
console lease the lease's serial MCP server holds that bridge's one client
slot, so `board.sh`'s probe gets `busy`: run it beside the feed bridge of
`tools/cts/README.md` ("The console under a farm console lease"), with
`BOARD_UART` naming the feed and the MCP console's output appended to the
feed's file, as the first Pi 5 session did. `console.ts` times are then the
relay's, not the board's; `back_after` is measured over ssh and is not
affected.

### The first image arms the watchdog

A Zephyr image that hangs without resetting holds the board until a person
power-cycles it. So every Pi 5 image starts by arming the BCM2712 `pm`
watchdog (`rpi_wdog.c` of the board layer, the `board: PM watchdog resets`
line `board.sh` checks the image for), and tryboot then boots Linux, because
`tryboot.txt` is read once. If the board does not answer within 420 s + 420 s
of the reboot, `board.sh` stops, retries nothing, and says that a person must
power-cycle the board; no other run takes it until the owner says it is back.

### Measured on the board

The first Pi 5 session (2026-11-19, Pi 5 Model B Rev 1.1 8 GB, BCM2712 D0,
boot loader of 2025-06-13, `get_throttled` 0x0 before and after every run)
ran `rpi5_wdog` and `rpi5_dtb` through `tools/board.sh pi5`:

| fact | state |
|---|---|
| tryboot works with the boot loader of 2025-06-13 | known: `reboot '0 tryboot'` booted from `tryboot.txt` (`/proc/device-tree/chosen/bootloader/tryboot` read 1) and the next plain reboot read 0 (owner's check, 2026-11-06; U7). Confirmed by every run of the session: the boot loader logs `TRYBOOT` and `Read /tryboot.txt`, boots our file, and the reset after it reads `config.txt` and boots Linux |
| where the firmware loads a `kernel=` image (U6) | at 0x200000, where Zephyr's `rpi_5` links `sram0`: the boot loader logs `Loading 'yakogl-<name>.bin' to 0x00000000 offset 0x200000`, with no `kernel_address` or other key beyond the three `tryboot.txt` lines above. There is no `armstub8-2712.bin` on the card: the firmware's own TF-A BL31 (v2.6) runs first and enters the image at EL2 (CurrentEL read by `rpi5_dtb`), on the core with MPIDR 0x81000000, at Zephyr's entry 0x2011bc |
| what the firmware passes in x0 | the DTB's address: x0 = 0x2efec600, where the boot loader logs `Device tree loaded to 0x2efec600 (size 0x13949)` (80201 bytes, 322 nodes; the `bcm2712d0` overlay applied, no `vc4-kms-v3d`, since `tryboot.txt` names no overlay). The same address on all three boots |
| the DTB's GPU nodes (D1) | `/axi/v3d@2000000` `brcm,2712-v3d`: reg `hub` 0x10_02000000 + 0x4000, `core0` 0x10_02008000 + 0x6000, `sms` 0x10_02030800 + 0x700; interrupts GIC SPI 250 and 249 (level high); clocks: firmware clock 5; resets and power-domains: the `pm` node, reset 0, domain 1. `/soc@107c000000/watchdog@7d200000` `brcm,bcm2712-pm` (reg 0x7d200000 + 0x308, the `pm` block; its phandle is the V3D's reset and power domain provider). `/soc@107c000000/firmware/clocks` `raspberrypi,firmware-clocks`. `/hvs@107c580000` `brcm,bcm2712-hvs` (reg 0x10_7c580000 + 0x1a000, firmware clocks 4 and 16). `/axi/gpu` `brcm,bcm2712-vc6` |
| the BCM2712 PM watchdog resets the board | yes: `rpi5_wdog` stopped feeding 30 s after boot, the boot loader restarted within 10 s with `PM_RSTS 00000020` and booted Linux from `config.txt` with nobody at the board. `board.sh` reports such a run as `verdict=no_end_line` (rc 4): the probe's pass is the reset, it prints no end line |
| the time from the tryboot reboot to Linux | about 3 min to the login prompt after `rpi5_wdog` (30 s fed, then the reset); `board.sh` reads `back_after` 218 s and 221 s after `rpi5_dtb` (the watchdog resets ~40 s after boot) and 446 s after `rpi5_wdog`, whose run waits out the 420 s bound first |
| staging | copying the 45 KB image, writing `tryboot.txt` and reading both back took 3 to 6 min of the run (the card's slow writes and `sync`) |

## NanoPC-T6

The NanoPC-T6 (RK3588, Mali-G610) has no boot loader step of ours: its
standalone path is a development route. A `nanopc_t6` Zephyr image is
started by kexec from the board's Armbian, and the board comes back to Armbian
through the watchdog or PSCI `SYSTEM_RESET`. The facts, the memory window and
the per-stage procedures are in `driver/docs/nanopc-t6-mali.md` ("Z1",
"Z2 board procedure", "Z3 board procedure", "Z4 board procedure"); this
section is the overview.
A kexec run is a development result, as `driver/tools/kexec-zephyr/README.md`
says.

### The boot form

- `driver/tools/kexec-zephyr/nanopc-t6.sh <zephyr.bin>` checks the image and
  the board read-only and prints the plan; with `--go` it jumps. It refuses an
  image without the board glue's watchdog marker, because Armbian leaves WDT0
  stopped and only the image arms it.
- Nothing is staged under `/boot` and no boot file changes. The runner copies
  the loader (`kexec_zephyr`), the register reader and the image to
  `/tmp/yakogl-kexec` on the board, loads the image at its link address
  (0x10000000, inside System RAM) with `kexec_load`, and jumps. The staging
  directory goes with the reboot (the next boot clears `/tmp`).
- By default it opens `/dev/watchdog0` for the jump, so Linux's watchdog
  covers the image's pre-arm window, and jumps only after `WDT_CR.WDT_EN` read
  back as 1. `--no-linux-wdt` skips that. That a WDT0 timeout brings the board
  back into Armbian is not proven; Z1 returned through PSCI `SYSTEM_RESET`.
- `--go` needs the name of the person at the power switch (`T6_HAND`), a
  record directory outside the checkout (`T6_RECORD_DIR`) and the host's
  read-only tty on the debug UART (`T6_UART`); the board is reached through
  `BOARD_HOST`. All of them come from the environment, never from a file.
- The probes print on uart2 (1500000 baud, the board's debug UART): the early
  banner `T6:A` .. `T6:C`, the board glue's lines (marker, GIC, watchdog
  armed), the probe's own lines (`t6_fw: build ...`, `t6_fw: PASS`, ...) and
  the reset line. `driver/tools/kexec-zephyr/t6-verdict.sh <probe> <uart.log>`
  checks the log against those lines and prints `VERDICT <probe> PASS` or
  `FAIL`.
- `tools/board.sh nanopc-t6 run <zephyr.bin> <logdir>` runs the same jump
  through `nanopc-t6.sh --go`, with every rule above, and writes the run
  directory, the verdict line (`kexec-dev`, DEV) and `INDEX.txt` as on the
  other boards; a probe's verdict there is `t6-verdict.sh`'s. It has no boot
  loader backend yet.

### Z1 to Z4

| Stage | What | Probe (`tools/probes/nanopc_t6/`) | Board module (`driver/rust/src/board/`) | `tools/check.sh` row | State |
|---|---|---|---|---|---|
| Z1 | the image, its watchdog, GPU power-on | `t6_hello`, `t6_gpu` | `nanopc_t6.rs`, `nanopc_t6_early.rs`, `nanopc_t6_gpu.rs` | `zephyr  nanopc_t6 t6_hello image (Z1, the watchdog glue)`, `... t6_gpu image (Z1, GPU power-on)` | `t6_hello` passed on the board by kexec (2026-10-19), `t6_gpu` passed on the board (2026-11-11) |
| Z2 | AS0 and a cache flush | `t6_mmu` | `nanopc_t6_mmu.rs` | `zephyr  nanopc_t6 t6_mmu image (Z2, AS0 and a cache flush)` | passed on the board (2026-11-11), "Z2 board procedure" in `driver/docs/nanopc-t6-mali.md` |
| Z3 | the CSF firmware boot | `t6_fw` | `nanopc_t6_fw.rs` | `zephyr  nanopc_t6 t6_fw image (Z3, the firmware boot)` | passed on the board (2026-11-12), "Z3 board procedure" in `driver/docs/nanopc-t6-mali.md` |
| Z4 | CSG 0 / CS 0, tiler heap, the L1 frame, the sync wait | `t6_submit` | `nanopc_t6_submit.rs` (`board::nanopc_t6_submit`) with the C entry `yd_t6_submit` (`driver/rust/src/ffi/rk3588.rs`) | `zephyr  nanopc_t6 t6_submit image (Z4), -Werror on our C files` | passed on the board (2026-11-12, second run), "Z4 board procedure" in `driver/docs/nanopc-t6-mali.md` |

Each of the rows is `step "..., -Werror on our C files"` in `tools/check.sh`.
Every stage has passed on the board: the runs of 2026-11-11 and 2026-11-12,
their verdict lines and their records are in `driver/docs/nanopc-t6-mali.md`
("Z2 board procedure", "Z3 board procedure", "Z4 board procedure") and in
the rows of `tools/cts/RESULTS.md` ("Mali: NanoPC-T6 on Zephyr (by kexec)").
Each board run still needs the owner's go-ahead and a person at the power
switch (`driver/docs/nanopc-t6-mali.md`), Z3 runs only after Z2 has passed,
and Z4 only after Z3. Z4's kernel side is `driver/rust/src/gpu/mali/sched.rs`,
`sched/job.rs` and `heap.rs` over the host MCU model, with `csf.rs` and
`fw.rs` from Z3; the frame it submits is built by `src/mali/frame.rs`.

### What a run needs from the host

- `ZEPHYR_WS`: the pinned west workspace (`west.yml`). The image rows are a
  gap without it, and a gap is not a pass.
- For Z3 and Z4 images, the CSF firmware header
  `driver/src/fw/mali_csffw_blob.h`, made by `driver/tools/mkmalifw.sh` from
  Arm's `mali_csffw.bin` (the board's Armbian carries the file). The header
  is gitignored and never committed. `tools/cts/deps-env.sh --build
  MALI_CSFFW` fetches the pinned file (`tools/cts/deps.toml`), or name it in
  `MALI_CSFFW`; `CONTRIBUTING.md` has the details. Without it the firmware
  rows are a gap.

## Raspberry Pi Zero 2 W

**Not yet run on our board.** Everything in this section is upstream
Zephyr's statement (the board page `boards/raspberrypi/rpi_zero_2w/doc/index.rst`
at `1b3f4f14ad20`) or a build on the host, not a measurement.

The board (BCM2710A1 in the RP3A0, four Cortex-A53, VideoCore IV) and its SoC
are upstream Zephyr's, carried in `driver/` until the `west.yml` pin has them
(`driver/UPSTREAM.zephyr-rpi_zero_2w`: the upstream commit, the hashes, and
three patches for the pin). An image builds with the pin as for any other
board, `west build -b rpi_zero_2w <app>`, with this repository's `driver/` in
the module list (`tools/cts/zephyr-modules.sh`); the gate row
`zephyr  rpi_zero_2w hello_world image` builds Zephyr's `samples/hello_world`.
On the pin the image is single-core: the four-core start upstream added
later needs arch code the pin does not have (patch 0001).

The Zero 2 W has no EEPROM: the boot ROM reads `bootcode.bin` from the
card's first FAT partition, and that loads the firmware. The card, as the
board page gives it: a Raspberry Pi OS Lite (64-bit) card, whose boot
partition holds the firmware, with two changes in its root directory:

| file | from | what it is |
|---|---|---|
| `bootcode.bin` | Raspberry Pi OS boot partition | the second stage the boot ROM loads from the card |
| `start.elf`, `fixup.dat` | Raspberry Pi OS boot partition | the firmware and its memory fix-up (a matching pair) |
| `config.txt` | below, replacing the card's | the configuration |
| `zephyr.bin` | the build (`build/zephyr/zephyr.bin`) | the kernel, loaded at 0x200000 |

`config.txt`:

```ini
arm_64bit=1
enable_uart=1
core_freq=250
kernel_address=0x200000
kernel=zephyr.bin
```

| key | what it does (the board page's reasons) |
|---|---|
| `arm_64bit=1` | 64-bit; needed when the kernel file is not named `kernel8*` |
| `enable_uart=1` | the mini-UART on GPIO 14/15 (header pins 8 and 10), Zephyr's console at 115200 8N1; otherwise the Bluetooth radio has those pins |
| `core_freq=250` | fixes the VPU clock, from which the mini-UART's baud divisor is derived |
| `kernel_address=0x200000` | must match the DRAM base of the board's devicetree (`dram0: memory@200000`, a 256 MiB window) |
| `kernel=zephyr.bin` | the file to boot |

The console is expected to print `*** Booting Zephyr OS build ...` and
`Hello World! rpi_zero_2w/bcm2710` (the pin's `hello_world` prints
`CONFIG_BOARD_TARGET`). On the pin, the console UART uses the pin's
own mini-UART driver, which keeps the baud rate the firmware set
(patch 0002), so `enable_uart=1` and `core_freq=250` are what set the rate.
The board is not a farm resource yet; the first run is day one of
`docs/notes/pre-study-rpi-zero-2w.md` section 6.

## Toradex Verdin iMX95

The Verdin iMX95 (NXP i.MX 95, six Cortex-A55, Mali-G310) on the Mallow
carrier, with the owner's Toradex Capacitive Touch Display 10.1" LVDS on
X12. The board arrived on 2026-11-24. **What has run on it** (the owner's
session, 2026-11-25, images of main `bd6cd44938a7`): the probes
`imx95_wdog`, `imx95_scmi`, `imx95_ident`, `imx95_fw` and `gl_test` on
read-back, through the U-Boot route below typed at U-Boot's prompt (not
through `tools/board.sh imx95`, not from the boot command); all ran to their
end and the board came back to Linux each time (results:
`docs/notes/pre-study-imx95.md` section 6, `tools/cts/RESULTS.md`). **The
display has not run**: no image of ours has driven the X12 panel. What is
built: `tools/check.sh` builds `examples/triangle` for
`verdin_imx95/mimx9596/a55` (row `zephyr  verdin_imx95 examples/triangle
image (Mali-G310, built only), -Werror on our C files`), from the board
`driver/boards/toradex/verdin_imx95`, which is derived from Zephyr's
`imx95_evk` A55 board at the workspace pin, and the board glue
`driver/zephyr/lib/board/verdin_imx95`. The image arms WDOG3 (below),
brings the GPU up over SCMI behind `eglInitialize`, renders the triangle
into a window in the Mali carve-out, and shows it on the Toradex Capacitive
Touch Display 10.1" LVDS on the Mallow's X12 (`board_show()`: the window,
1024x600, is copied into a 1280x800 screen buffer at the panel's top left,
and the first show brings the display up cold,
`driver/docs/imx95-display.md` section 4(b'); the X4 DSI display of section
4(b) with `CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY_X4_DSI=y`). The route below is
the one that ran, and the open questions at the end say what the first
session answered.

### The boot chain

The sources are U-Boot's `doc/board/toradex/verdin-imx95.rst` and
`board/toradex/verdin-imx95/` (u-boot `ece349ade297`), and the pin's
`boards/nxp/imx95_evk/doc/index.rst` (zephyr `af17c0c8aa2a`).

- The boot core is the Cortex-M33, not an A55. Its boot ROM, with the
  EdgeLock Enclave (the ELE firmware container `mx95b0-ahab-container.img`),
  loads the OEI, which trains the DDR (LPDDR4X firmware `lpddr4x*v202409.bin`),
  then the **System Manager** (`m33_image.bin`, Toradex's `imx-sm-toradex`,
  `config=verdin-imx95`). The System Manager configures isolation, starts the
  A55, and from then on serves clocks, power domains, pins and sensors to the
  other cores over SCMI. It keeps running for the whole life of the board,
  whatever the A55 runs.
- On the A55: U-Boot SPL, then TF-A BL31 (`nxp-imx/imx-atf`, `lf_v2.12`,
  `PLAT=imx95`), then U-Boot proper ("SPL ---> ATF (TF-A) ---> U-Boot proper").
  All of these are one `flash.bin` in the eMMC's boot partition (`mmc dev 0
  1`, written from offset 0; the env's `update_uboot`). U-Boot's console is
  LPUART1 (`serial@44380000`, Verdin UART_3), the same UART our image uses.
- What must be on the board: that `flash.bin`, as Toradex ships the module.
  Nothing of ours replaces any of it. Our image is one file that U-Boot
  loads, as on the other two Verdins.

### How U-Boot would start the image

- The image is `zephyr.bin` with an AArch64 Image header, linked and loaded at
  **0xd0000000** (`zephyr,sram`, 256 MiB, in
  `verdin_imx95_mimx9596_a55.dts`), the address of the pin's `imx95_evk` A55
  board and of its documented U-Boot route ("Option 2": `fatload mmc 1:1
  0xd0000000 zephyr.bin`, then `dcache off; icache flush; go 0xd0000000`).
- Toradex's U-Boot keeps the first 256 MiB of DRAM for the firmware (DRAM
  from 0x80000000, its own from 0x90000000; TF-A and OP-TEE at 0x8a000000,
  64 MiB) and uses 0x90400000 (`kernel_addr_r`, `loadaddr`) to 0x9c800000
  (`ramdisk_addr_r`) for its loads; it relocates itself below 4 GiB. 0xd0000000
  to 0xdfffffff is clear of those and below 4 GiB on every module size (U-Boot
  probes 2, 4, 8 or 16 GiB; its doc shows an 8 GB module).
- **What ran on 2026-11-25**: Linux staged the image as a file under
  `/var/yakogl` on the eMMC's root file system (partition 2, ext4); at
  U-Boot's prompt, on the UART_3 console: `ext4load mmc 0:2 0xd0000000
  <file>` (md5 compared), then `dcache off; icache flush; go 0xd0000000`.
  No boot partition was written.
- The product form is then the other Verdins' (above): `ext4load mmc <dev>:<part>
  0xd0000000 /zephyr.bin && dcache off && icache off && go 0xd0000000`, and
  the test form `BOARD_BOOT=uboot-mmc`: the one-shot hook of
  `tools/uboot-netboot-lib.sh` in front of the board's `bootcmd` (mainline's is
  `bootflow scan -b`), the file staged by Linux under `/var/yakogl`. `go`, not
  `booti`, for the reason in "What the image does on a `go` entry": `booti`
  moves an image with `text_offset` 0 to the start of DRAM. The runner is
  `tools/board.sh imx95` (`BOARD_BOOT=uboot-tftp`, its default, or
  `uboot-mmc`): the other Verdins' hook and sequence with `NB_MMC=0:1`,
  `MMC_PART=/dev/mmcblk0p1` (the AM62P's; `mmc_where` refuses another mount),
  the window `0xd0000000`..`0xe0000000` (the DTS's `zephyr,sram`), the
  marker `verdin_imx95: WDOG3 armed`, and the record of the next section.
- The development route `kexec-dev` (`driver/tools/kexec-zephyr`, as on the
  Verdin iMX8M Plus and the NanoPC-T6) carries over in its form: Linux loads
  the image at 0xd0000000 with `kexec_load` and jumps. The Verdin iMX95's
  Linux has a CMA pool that may cover that address (`linux,cma`: 960 MiB
  allocated from 0x80000000 to 0xff000000, `imx95-verdin.dtsi`); a
  `crashkernel=` reservation there, as the iMX8M Plus's
  `crashkernel=64M@0x60000000`, would be the clean form. Not tried, and not
  possible on the board's image (2026-11-25: `CONFIG_KEXEC` not set).
- At the end of a run the image resets through PSCI `SYSTEM_RESET` (TF-A,
  which asks the System Manager), the way back to U-Boot and Linux. A
  software timer takes that reset after `CONFIG_YAKOGL_VERDIN_IMX95_LIFETIME_S`
  (the board layer's `YAKOGL_BOARD_LIFETIME_S`, 300 s, in an image with the
  layer; else 60 s).
- The watchdog is **WDOG3**, the A55 non-secure agent's
  (`verdin_imx95_wdog.c` over the yakogl-drivers crate's `ip::wdog32`). The
  image arms it first thing (`PRE_KERNEL_1` 0, before any other init hook)
  for 60 s, with Linux's configuration for this module (`CS.INT` on, which
  the System Manager's fault is), prints `verdin_imx95: WDOG3 armed: ...` and
  `board: watchdog armed`, and a 1 s timer feeds it up to the lifetime and
  while the board layer's alive window holds. A refusal resets through PSCI
  at once; a PSCI reset that returns halts with interrupts off and WDOG3
  bites. Its timeout is the System Manager's `FAULT_WDOG3` with
  `reaction=sys_reset`, which on Toradex's System Manager pulls `WDOG_ANY`
  to reset the PMIC: the whole board, System Manager included, boots again
  as from power-on into U-Boot and Linux, and no SCMI power vote of the image
  (the GPU's) survives. Sources and the comparison with WDOG4 and the System
  Manager: `docs/notes/pre-study-imx95.md`, section 1.6.

### The record without a console

The Mallow has no console the farm can read yet (pre-study U7), and the
i.MX8MP's sink, the SNVS LPGPR, has no counterpart the A55 can write: on
the i.MX 95 the battery-backed block (BBNSM) belongs to the System
Manager. The SM lends four of its eight general purpose registers: its BBM
protocol (NXP vendor protocol 0x81) has BBM_GPR_SET (0x3: index, value;
EXCLUSIVE needed) and BBM_GPR_GET (0x4: index -> value; GET needed;
imx-sm-toradex 3aa8a77a `sm/rpc/scmi/rpc_scmi_bbm.c:746-829`), and
Toradex's `verdin-imx95` configuration gives the A55's non-secure agent
GPR4..7 at ALL (`configs/verdin-imx95.cfg:756-759`, generated
`configs/verdin-imx95/config_scmi.h:453-456`) and none of GPR0..3, where
the SM's own board code keeps its shutdown record, read and cleared at each
SM boot (`boards/tdx-common/sm/brd_sm_tdx_common.c:52-53, 363-461`). Nothing
in the SM touches GPR4..7 otherwise (`devices/MIMX95/sm/dev_sm_bbm.c:158-210`
writes them only on request).

- **The format** is the one record of every board
  (`driver/rust/src/board/record.rs`): the four words of the image's last
  `yk-record:` line, w0 (the `YK` magic) in GPR4, w3 in GPR7, so
  `yakogl-tools record` decodes them as it decodes the SNVS words and the
  console line. The store keeps the record's rule: GPR4 cleared first, then
  w1..w3, then w0, so a store cut off half way never shows the new magic over
  old words, and a reader that finds the magic can trust the rest.
- **The writer** is the board layer's sink
  (`driver/zephyr/lib/board/verdin_imx95/verdin_imx95_record.c`,
  `CONFIG_YAKOGL_VERDIN_IMX95_RECORD_BBM`, on with the layer); the messages
  are the drivers crate's (`soc::imx95::bbm`, `yd_imx95_bbm_record_store`),
  over the board's one SCMI channel. The layer stores with interrupts locked,
  from any context, where the channel's lock cannot be waited for; on this
  one core nobody can start an exchange then, so the store goes out at once
  when nobody is in one (`verdin_imx95_scmi_idle`), else it waits as pending
  for the next store, for `POST_KERNEL` (a breadcrumb stored before the
  channel was up) or for `verdin_imx95_reset`, which flushes it first and,
  from a thread, waits for the lock. The console line is printed either way;
  a refusal prints one `verdin_imx95: record not stored in the BBM GPRs`
  line with the GPR and the SM's status.
- **The reader**: Linux's BBM driver knows BBM_GPR_GET but exports only the
  RTC and the button (`drivers/firmware/arm_scmi/vendors/imx/imx-sm-bbm.c:338`,
  `include/linux/scmi_imx_protocol.h:25`, at Linux 65e4fd94 and in
  linux-toradex `toradex_6.18-2.1.x-imx` 851ac0f9 alike), and the GPRs sit
  behind the SM, not in `/dev/mem`. So `tools/imx95_bbm_record.sh` sends the
  same messages itself through the kernel's SCMI raw interface
  (`/sys/kernel/debug/scmi/<n>/raw/message`,
  `Documentation/ABI/testing/debugfs-scmi-raw`): `read` prints
  `w0 w1 w2 w3`, `clear` zeroes GPR4 first, `probe` says whether the
  interface is there. That interface exists only with
  `CONFIG_ARM_SCMI_RAW_MODE_SUPPORT` and, so that the normal SCMI drivers keep
  running, `CONFIG_ARM_SCMI_RAW_MODE_SUPPORT_COEX`; neither `imx_v8_defconfig`
  nor `defconfig` of Toradex's tree sets them. `board.sh` pipes the script to
  the board's `sh` as root (nothing is staged), clears the record before the
  boot and keeps it as `bbm.txt` in the run directory after it, read as the
  i.MX8MP's `snvs.txt` is. Without the interface a run reads the console
  instead when `BOARD_UART` is set (`rec_sink=console only` in `meta.txt`) and
  refuses (rc 3) when it is not. The image reads no selector, so `SEL` is
  refused.
- **Tests**: `driver/rust/src/soc/imx95/bbm_tests.rs` runs the store and the
  load over the mock SM with AP-NS's rights (the order, the words decoded as
  the console line's, DENIED and a store cut off half way);
  `tools/imx95_bbm_record_selftest.sh` runs the reader against a fake SM (the
  header words the Rust test pins, `read` and `clear` under `sh`, dash and
  busybox); `tools/board_selftest.sh` runs `board.sh imx95` dry and replays
  its verdicts from `bbm.txt` and from the console.
- **For the board** (B1, B3): whether the board's kernel has the raw
  interface (`board.sh imx95 status` runs `probe`); whether GPR4..7 outlive
  WDOG3's fault, which resets the PMIC through `WDOG_ANY`, and PSCI
  `SYSTEM_RESET` (the BBNSM is in the battery-backed domain; what powers it on
  the Verdin through a PMIC reset is not read); and that the SM on the module
  grants GPR4..7 as its source does (U1: a DENIED shows as the refusal line).

### Open questions for sign-off

1. **Can U-Boot start our image with the System Manager still serving
   SCMI?** Answered on 2026-11-25: yes. `imx95_scmi` after `go` read 146
   facts (agent 2 "AP-NS", implementation 0x23e, power domains 14, 13 and 3
   off at entry), and `imx95_ident`, `imx95_fw` and `gl_test` powered the GPU
   over the same channel. The System Manager runs on the M33 on its own, so it is still
   there after `go`; the image's first SCMI messages (the LPUART1 pins and
   clock, at `PRE_KERNEL`) go through the same channel U-Boot used (MU2,
   shared memory at 0x445b1000; the same channel in Linux's `imx95.dtsi`).
   To check on the board: that U-Boot leaves no message pending on that
   channel, and that the System Manager's configuration for this module
   (`imx-sm-toradex`, `verdin-imx95`) gives the A55 non-secure agent the
   LPUART1 pins and clock and, for the GPU later, power domain 14 and the
   GPU clocks.
2. **The watchdog.** Answered on 2026-11-25: U-Boot leaves WDOG3
   **stopped** at `go` (`imx95_wdog`: "wdog found CS 0x00002900 TOVAL
   0x00000400 (stopped); armed rc 0 CS 0x000035e3 TOVAL 0x00001d4c timeout
   60000 ms"); unfed, it reset the whole board after the 60 s (U-Boot SPL's
   banner, then "LM Boot reason: fccu, origin: -1, errid: 19"); the PSCI
   reset returns to U-Boot ("LM Boot reason: sw, origin: 0, errid: 0").
   Toradex's U-Boot has no `wdt` command. The image arms WDOG3 itself
   (above), so it does not depend on U-Boot's. What was to be checked: the armed line's
   `CS`/`TOVAL` found (running with `TOVAL` 0xea6 if U-Boot left it so), the
   poll counts and time of the arming, and a proof run that arms and never
   feeds: WDOG3's timeout must reach the System Manager's fault (with
   `CS.INT` set) and reset the board through `WDOG_ANY` back into Linux.
3. **Caches at `go`.** Answered on 2026-11-25: `dcache off; icache flush`
   (the pin's EVK route) works on this U-Boot; `icache off` was not tried.
4. **The eMMC device and partition U-Boot reads**: answered on 2026-11-25:
   the eMMC is `mmc 0`, partition 1 FAT (the boot files), partition 2 ext4
   (Linux's root, where `/var` is); Linux stages the file under
   `/var/yakogl`, U-Boot reads it with `ext4load mmc 0:2`. The boot areas
   are 31.5 MiB each and not write-protected. `tools/board.sh imx95`'s
   `NB_MMC=0:1` / `MMC_PART=/dev/mmcblk0p1` (the AM62P's) do not match this
   layout and were not used.
5. **The SoC package.** The board takes the 19x19 package's pin list
   (`mimx9596avzxn-pinctrl.dtsi`, as `imx95_evk`); every pad the module's
   Linux uses is in it and in the 15x15 one, so the console does not depend
   on it, but the module's part is not confirmed.
6. **The Mali-G310's bring-up** ran on 2026-11-25 (`imx95_ident`: `GPU_ID
   0xac740001`, one shader core; `imx95_fw`: GLB 1.5.0; `gl_test`: 50 of 50
   on read-back): the CSF firmware
   is the T6's file (`MALI_CSFFW`; panthor loads
   `arm/mali/arch<major>.<minor>/mali_csffw.bin` by the GPU_ID's
   architecture, and linux-firmware's `arch10.12` file, the G310's, is the
   `arch10.8` bytes, `docs/notes/pre-study-imx95.md`), the GPU is powered
   over SCMI to the System Manager (`docs/notes/imx95-gpu-power.md`), and
   from `GPU_ID` on the bring-up is the T6's
   (`driver/zephyr/lib/board/verdin_imx95/verdin_imx95_mali.c`,
   `yd_imx95_osal_up`). What the board answers on day one: the SM's
   replies over the channel after U-Boot's `go`, the G310's identity words,
   the firmware's GLB version.
7. **The X12 LVDS display** is in the image, not run: the board layer's
   first `board_show()` puts the backlight's pads (GPIO_IO06 as TPM5 channel
   0, XSPI1_DATA4 as GPIO5_IO04) through the System Manager's pin control
   and runs `yd_imx95_lvds_up`: DISPLAY and the LDB PLL over SCMI, the
   display CSR, the interleaver, the DPU, the LDB and LVDS PHY 0, FrameGen0,
   then the backlight (`driver/docs/imx95-display.md` section 4(b'),
   `driver/zephyr/lib/board/verdin_imx95/verdin_imx95_lvds.c`). The console
   then has `verdin_imx95: display: X12 LVDS, ...` and one `verdin_imx95:
   display up: ...` line (the pixel clock, the LVDS CSR's words, TPM5's
   rate) or `verdin_imx95: display stopped: ...` after the line that names
   the step; the note's section 6 has the expected lines. What the board
   answers: those lines, then whether the panel shows the triangle at its
   top left on black. Before it, the probe `imx95_lvds_ids` (read only)
   reads the X12 path's identity.
8. **The X4 display** (with `CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY_X4_DSI=y`;
   not this board's setup) is in the image, not run: the board layer's first
   `board_show()` turns the LPI2C2 and LPI2C3 clocks on over SCMI,
   initialises Zephyr's `nxp,lpi2c` buses and the module's PCAL6416A
   (`nxp,pcal6416`; all three are `zephyr,deferred-init` in the board's
   devicetree, nothing touches them at boot), drives `GPIO_10_DSI`
   (expander line 3) low, puts TPM6 channel 2 on its pad, and runs
   `yd_imx95_display_up`: the twelve steps of `driver/docs/imx95-display.md`
   section 4(b) over the same SCMI channel as the GPU
   (`driver/zephyr/lib/board/verdin_imx95/verdin_imx95_display.c`,
   `verdin_imx95_scmi.c`). The console then has one `verdin_imx95: display
   up: ...` line (pixel clock, lane rate, the bridge's `IRQ_STAT`) or
   `verdin_imx95: display stopped: ...` after the line that names the step.
   What the board answers on day one: that line, then whether the panel
   shows the triangle at its top left on black.

## On the boards

In order; stop at the first step that prints something else.

### Verdin AM62P

1. `tools/board.sh am62p status`: the hook variables, then `install: ok`,
   `install: absent`, or `install: stale: yako_hook differs from this
   script's` (a hook of another version).
2. `tools/board.sh am62p install`: `installed; ...` or `upgraded in place
   (...); bootcmd unchanged`. `status` again: `install: ok`. From here on a
   `board.sh` older than this commit refuses to run on the board.
3. `BOARD_BOOT=uboot-mmc tools/board.sh am62p probe <logdir>`: `rc=0
   verdict=probe_ok boot=uboot-mmc row boot_verdict=load_failed`; the console
   has `yakogl-netboot: mmc load failed, booting Linux`, then Torizon's boot.
4. `BOARD_BOOT=uboot-mmc tools/board.sh am62p run <zephyr.bin> <logdir>`, with
   an image that has a TFTP row: `boot_verdict=done`; the console has
   `<n> bytes read in` with n = the image's size; the same verdict as the TFTP
   row. `status`: no `once-*.bin` in `/var/yakogl`.
5. `tools/board.sh am62p default <zephyr.bin> 2` (no person needed): it prints
   the undo, stages `default.bin`, writes `yako_default` and
   `yako_default_left=2`. Then reboot the board from Linux
   (`sudo systemctl reboot`). The console shows `default ..., boots left 1`,
   the image, its reset, `boots left 0`, the image, its reset,
   `default used up, booting Linux`, Torizon. `status`: no `yako_default`,
   no `yako_default_left`. `tools/board.sh am62p nodefault` removes the file.
6. `forever` only with someone watching (it takes the board from everyone):
   `BOARD_PERSON_PRESENT=1 tools/board.sh am62p default <zephyr.bin> forever`,
   reboot, the image runs at every boot. Undo over the bridge:
   `python3 tools/am62p_netboot_uart.py typed $BOARD_UART raw.log ts.log
   'setenv yako_default; saveenv; boot'` stops the next autoboot and types the
   line. It waits 120 s for U-Boot SPL (`typed: EXPIRED` otherwise), so start
   it again until it prints `echo exact: CR sent` (stderr), then stop it (it
   keeps capturing). Then `nodefault` from Linux.

### Verdin iMX8M Plus

1. and 2. as on the AM62P.
3. `BOARD_BOOT=uboot-mmc tools/board.sh imx8mp probe <logdir>`: `rc=0
   verdict=probe_ok boot=uboot-mmc row boot_verdict=probe_ok`. `rc=6` means the
   hook did not clear the one-shot (`board.sh` cleared it).
4. `BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run <zephyr.bin> <logdir>`:
   `boot_verdict=cycled` and the image's record, e.g. `verdict=done ...
   result=record:APP`. A record still cleared (`verdict=no_record`) means the
   image did not run: `ext4load` may be missing from this U-Boot build
   (unmeasured), or the path is wrong.
5. `tools/board.sh imx8mp default <zephyr.bin> 2`, reboot from Linux. No
   console: Linux must answer again within about two image runs plus three
   boots; then `status` shows no `yako_default` and no `yako_default_left`,
   and the SNVS record holds the second run's result. `nodefault` removes the
   file.
6. `forever`: **a person** with a UART cable at the U-Boot prompt is the only
   way back (`setenv yako_default; saveenv; boot`); without one, USB recovery
   mode and Toradex Easy Installer.

### Raspberry Pi 4

1. Nothing to install. `tools/board.sh pi4 run <zephyr.bin> <logdir>`
   (tryboot) is the test.
2. `forever`: **a person** at the board.
   `BOARD_PERSON_PRESENT=1 tools/board.sh pi4 default <zephyr.bin> forever`,
   reboot, watch the panel or the UART; undo with the card in a PC.

### Verdin iMX95

The first session (2026-11-25) did not use `board.sh`: it typed the route
of "Toradex Verdin iMX95" above at U-Boot's prompt (the file staged by
Linux under `/var/yakogl`, `ext4load mmc 0:2`, `dcache off; icache flush;
go 0xd0000000`). The board's kernel has no SCMI raw interface, so step 1
below reports `no SCMI raw interface` and the console is the only record;
`NB_MMC`/`MMC_PART` (now `0:1`, `/dev/mmcblk0p1`) would have to name partition 2 for
this layout. The order with `board.sh`, not yet run:

1. `tools/board.sh imx95 status`: the hook variables and `install: absent`,
   the staging directory's mount (`/dev/mmcblk0p1` expected), then
   `raw: /sys/kernel/debug/scmi/<n>/raw` or `no SCMI raw interface` and the
   BBM record. Without the interface the console (`BOARD_UART`) is the only
   record until the kernel has `CONFIG_ARM_SCMI_RAW_MODE_SUPPORT_COEX`.
2. `tools/board.sh imx95 install`, then `status`: `install: ok`.
3. `BOARD_BOOT=uboot-mmc tools/board.sh imx95 probe <logdir>`: `rc=0
   verdict=probe_ok boot=uboot-mmc row boot_verdict=probe_ok`, as on the
   i.MX8MP.
4. `BOARD_BOOT=uboot-mmc tools/board.sh imx95 run <zephyr.bin> <logdir>`:
   `boot_verdict=cycled` and the image's record from `bbm.txt`
   (`result=record:<KIND>`), or from the console with `BOARD_UART` set.

### NanoPC-T6

No install step and no `board.sh` backend: the runner is
`driver/tools/kexec-zephyr/nanopc-t6.sh`, run with the owner's go-ahead and a
person at the power switch. See "NanoPC-T6" above and
`driver/docs/nanopc-t6-mali.md`.
