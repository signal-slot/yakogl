<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Standalone boot from eMMC or SD

The rule (user, 2026-10-04): tests may boot Zephyr through U-Boot over TFTP
(the two Verdin boards) or through the firmware's tryboot (Raspberry Pi 4).
But every board must also be able to boot a Zephyr image from its own eMMC or
SD card, with no host and no network.

Status (2026-10-04): the code is in `tools/board.sh` (backend `uboot-mmc`,
verbs `default` and `nodefault`) and checked without a board
(`tools/board_selftest.sh`). **No step below has run on a board yet.** The
section "On the boards" lists the steps and what each must print.

This file replaces `driver/docs/rpi4-zephyr-default-boot.md`.

## Summary

| | Verdin AM62P | Verdin iMX8M Plus | Raspberry Pi 4 |
|---|---|---|---|
| Where the image lives | a file on the eMMC's first user-area partition (ext4) | the same | a file on the SD card's FAT boot partition |
| What loads it | U-Boot: `ext4load mmc 0:1`, then `go 0x82000000` | U-Boot: `ext4load mmc 2:1`, then `go 0x60000000` | the Pi firmware: `kernel=` in `config.txt` |
| Test, unattended | `BOARD_BOOT=uboot-mmc tools/board.sh am62p run` (one boot, then Linux) | `BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run` | `tools/board.sh pi4 run` (tryboot: the same card, files and lines) |
| Make it every boot's | `tools/board.sh am62p default <image>` | `tools/board.sh imx8mp default <image>` | `tools/board.sh pi4 default <image>` |
| Undo without Linux | at the U-Boot prompt: `setenv yako_default; saveenv; boot` (over the serial bridge) | the same, but the board has no UART: **a person** with a UART cable | **a person**: the SD card in a PC, copy `config-linux.txt` over `config.txt` |

`default` reboots nothing. It refuses unless `BOARD_PERSON_PRESENT=1`,
because after the next reboot the board no longer comes back to Linux by
itself. `nodefault` undoes it from Linux.

## Verdin AM62P and Verdin iMX8M Plus

### Why a file, and not the eMMC boot partition

Each eMMC boot partition on both modules is 4 MiB (8192 sectors in
`/sys/block/mmcblk*boot*/size`; AM62P in `docs/direct-boot.md`, i.MX8MP in
`driver/docs/imx8mp-netboot-plan.md` section 1). boot0 holds the boot loader
and U-Boot's environment. Our images are larger: `gl_triangle` 15.7 MB,
`gl_slint` 18.9 MB, a `gl_test` image up to 106 MiB in memory. They do not
fit. (The AM62P direct-boot route in `docs/direct-boot.md` needs a second-stage
loader for exactly this reason.)

The eMMC user area has room, and U-Boot can already read it: it loads Linux
from there. So the image is a plain file there, and U-Boot loads it with
`ext4load`, the way it loads a kernel.

### The product form

A product without Linux puts `zephyr.bin` on an eMMC partition and sets one
U-Boot variable. On the AM62P:

```
setenv bootcmd 'ext4load mmc 0:1 0x82000000 /zephyr.bin && dcache off && icache off && go 0x82000000'
saveenv
```

On the i.MX8MP the same with `mmc 2:1` and `0x60000000`. Why `go` and not
`booti`, and why the caches go off first: `driver/docs/am62p-netboot-plan.md`
section 3 (measured on this board by SafeUI). The image is the same
`zephyr.bin` as for TFTP and kexec. It must arm its own watchdog (RTI on the
AM62P, WDOG1 on the i.MX8MP), because U-Boot does not.

### On our shared boards

The boards run Torizon, and other people use them. So we keep Torizon's
`bootcmd` and add one hook in front of it, the same hook the TFTP one-shot
already uses (`tools/uboot-netboot-lib.sh`; design in
`driver/docs/am62p-netboot-plan.md` section 2).

Where the file goes. Torizon's root file system is the ext4 on the eMMC's
partition 1. It is mounted read-only, but `/var` is writable: it is a bind
mount of the directory `/ostree/deploy/torizon/var` of the same file system
([Toradex: OSTree](https://developer.toradex.com/torizon/in-depth/ostree/),
[Toradex: Torizon technical overview](https://developer.toradex.com/torizon/torizoncore/torizoncore-technical-overview/)).
Linux writes the image to `/var/yakogl/`. U-Boot sees the same file as
`/ostree/deploy/torizon/var/yakogl/<file>`. `board.sh` does not assume this
path: it reads it on the board (`findmnt -T /var/yakogl`: the source device,
the mount point and the FSROOT), and it refuses unless the device is the
partition U-Boot reads (`/dev/mmcblk0p1` = `mmc 0:1` on the AM62P,
`/dev/mmcblk2p1` = `mmc 2:1` on the i.MX8MP).

The hook (one line each in the env; the exact strings are in
`tools/uboot-netboot-lib.sh`):

```
yako_hook = if test -n "${yako_once}"; then
              echo yakogl-netboot: once=${yako_once}; setenv yako_once;
              if saveenv; then echo yakogl-netboot: cleared; run yako_${yako_src};
              else echo yakogl-netboot: saveenv failed, booting Linux; fi;
            fi;
            if test -n "${yako_default}"; then
              echo yakogl-netboot: default ${yako_default};
              setenv yako_file ${yako_default}; run yako_mmc;
            fi
yako_net  = (unchanged: DHCP, TFTP, go)
yako_mmc  = if ext4load mmc 0:1 ${yako_addr} ${yako_file}; then
              echo yakogl-netboot: go ${yako_addr}; dcache off; icache off; go ${yako_addr};
            fi; echo yakogl-netboot: mmc load failed, booting Linux
bootcmd   = run yako_hook; <Torizon's bootcmd, as it was>
```

- **The one-shot** (`yako_once`, set from Linux by `board.sh run`): U-Boot
  deletes it and saves the env **before** it loads anything. Then it runs
  `yako_net` or `yako_mmc`, as `yako_src` says. `run yako_${yako_src}` is the
  same form as the distro boot's `run bootcmd_${target}` in the i.MX8MP's
  environment (`distro_bootcmd`).
- **The default** (`yako_default`, set by `board.sh default`): every boot loads
  that file. It writes nothing to the env, so it wears nothing.
- **Any load that fails** falls through to Torizon's `bootcmd`: Linux.

`tools/board.sh <board> install` writes all of this in one env write. On a
board that has the older hook (TFTP only), the same command upgrades the
strings in place; `bootcmd` is not touched. `uninstall` restores `bootcmd`
from both versions.

### What a test run does (`BOARD_BOOT=uboot-mmc`)

The same sequence as the TFTP run (`tools/board.sh`, header), with these
differences:

1. No TFTP server, no `NETBOOT_*` variables.
2. Pre-checks: the hook is installed, no one-shot pending, **no default set**
   (with one, the boot after the image would not be Linux), `/var/yakogl` on
   the right partition, twice the image's size free, no leftover
   `once-*.bin`.
3. Stage: the image goes to `/var/yakogl/once-<token>.bin` (written as
   `.part`, synced, renamed, synced). Then the page cache is dropped and the
   file is read back from the eMMC by sha256.
4. Arm: `yako_once=<token>`, `yako_src=mmc`, `yako_file=<the path U-Boot
   sees>`, read back. Reboot.
5. U-Boot clears the one-shot, loads the file, starts it. The image ends in a
   watchdog reset; the next boot has no one-shot and boots Linux.
6. Back in Linux: `yako_once` must be gone (exit 6 otherwise), the file is
   removed, the record is read (AM62P console through the serial bridge;
   i.MX8MP SNVS record).

The verdict line says `boot=uboot-mmc row`. A `uboot-mmc` run is a row, like a
TFTP run: the bootloader starts the image.

`probe` arms a file that does not exist. On the AM62P it passes only when the
console shows U-Boot's own `yakogl-netboot: mmc load failed` line
(`boot_verdict=load_failed`). On the i.MX8MP it passes when the board came
back from a new boot with the one-shot cleared and the record untouched. That
proves the hook ran; it cannot tell a missing file from a missing `ext4load`
command. The first real run settles that.

### Why none of this can strand the board

| What fails | What happens |
|---|---|
| Staging or the read-back | the file is removed; nothing is armed; no reboot |
| `saveenv` in the hook | the image is not loaded (by design); Linux boots |
| `ext4load` (no file, wrong path, a journal not yet written back) | `mmc load failed, booting Linux`; Linux boots |
| The image hangs | its own watchdog resets the SoC; the one-shot is already gone; Linux boots |
| The host dies after the reboot | nothing on the boot path needs the host |

Linux reboots through systemd, which unmounts and remounts the file system
read-only, so the ext4 journal is written back before U-Boot reads the file.
If Linux crashes instead, U-Boot may not find the file: the load fails and
Linux boots.

The default is different on purpose. A reset boots the default image again,
so each boot of it must end, and getting Linux back needs the U-Boot prompt.

### Facts and sources

- AM62P: `ext4load mmc 0:1 0x82000000 ... && dcache off && icache off && go
  0x82000000` is in the board's own `bootcmd` today (SafeUI's persistent boot,
  env record `records/am62p/netboot/env-before-install-*.txt`, main checkout),
  and it booted Zephyr there (`driver/docs/am62p-netboot-plan.md` section 1).
- i.MX8MP: the eMMC is U-Boot's `mmc 2` (its env's `update_uboot`: `mmc dev 2
  1`), and the distro boot finds Torizon on partition 1
  (`distro_bootpart_uuid=8725ec00-01`; env record
  `records/imx8mp/netboot/env-before-install-*.txt`).
- `ext4load` in U-Boot: both boards' mainline defconfigs set
  `CONFIG_CMD_EXT4_WRITE=y`, which depends on `CMD_EXT4`
  ([verdin-am62p_a53_defconfig](https://github.com/u-boot/u-boot/blob/master/configs/verdin-am62p_a53_defconfig),
  [verdin-imx8mp_defconfig](https://github.com/u-boot/u-boot/blob/master/configs/verdin-imx8mp_defconfig)).
  The boards run Toradex builds of 2024.04 (TI) and 2024.07; mainline is a
  reference, not a byte match. **On the i.MX8MP `ext4load` is unmeasured.**
- The `load` family and its output line (`<n> bytes read in <t> ms`), which
  the AM62P phase scan reads:
  [U-Boot: load](https://docs.u-boot.org/en/latest/usage/cmd/load.html).
- Saving the env from Linux, and `setenv` changing only RAM until `saveenv`:
  [Toradex: U-Boot](https://developer.toradex.com/linux-bsp/os-development/boot/u-boot/).

## Raspberry Pi 4

### The product form

The Pi 4 boots from the SD card with no U-Boot. Its boot loader is in the SPI
EEPROM; it reads the first FAT partition of the card
([boot EEPROM flow](https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#eeprom-boot-flow)).
In our tryboot runs the firmware read these files
(`uart_2ndstage=1` log, 2026-10-03):

| file | from | what it is |
|---|---|---|
| `start4.elf`, `fixup4.dat` | Raspberry Pi OS boot partition | the firmware and its memory fix-up ([boot options](https://www.raspberrypi.com/documentation/computers/config_txt.html#boot-options)) |
| `config.txt` (`tryboot.txt` in a tryboot) | below | the configuration |
| `bcm2711-rpi-4-b.dtb`, `overlays/overlay_map.dtb`, `cmdline.txt` | Raspberry Pi OS boot partition | read by the firmware; Zephyr uses none of them |
| `zephyr.bin` | the build | the kernel, loaded at 0x200000 |

This is the set the firmware read, not a proven minimum: keep all of it until
a card without one of them has booted.

`config.txt` for Zephyr on every boot:

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
| `arm_64bit=1` | 64-bit (the Pi 4 default; kept explicit) ([arm_64bit](https://www.raspberrypi.com/documentation/computers/config_txt.html#arm_64bit)) |
| `enable_uart=1` | the UART on GPIO 14/15, Zephyr's console ([enable_uart](https://www.raspberrypi.com/documentation/computers/config_txt.html#enable_uart)) |
| `uart_2ndstage=1` | the firmware's own log on the UART; optional ([uart_2ndstage](https://www.raspberrypi.com/documentation/computers/config_txt.html#uart_2ndstage)) |
| `hdmi_*` | the 1920x720 bar panel as a custom mode; only the firmware's display path reads these, which is ours (Zephyr asks the firmware for a frame buffer) ([legacy video options](https://www.raspberrypi.com/documentation/computers/legacy_config_txt.html#legacy-video-options)) |

To make such a card from Raspberry Pi OS Lite (64-bit): one FAT32 partition;
copy the files of the table; add `zephyr.bin` and this `config.txt`. No
kernel, initramfs or root file system. Record the sha256 of every file next to
the build.

### The test is the product path

A tryboot reads `tryboot.txt` instead of `config.txt`, once
([tryboot](https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#fail-safe-os-updates-tryboot)).
Everything else is the same: the same card, the same firmware files, the same
lines. `tools/board.sh` writes both files from one function (`pi4_config`), so
a tryboot run that passes has booted the product's configuration from the SD
card. Only the file name differs. So every `tools/board.sh pi4 run` is
already the unattended test.

### Making it permanent

`BOARD_PERSON_PRESENT=1 tools/board.sh pi4 default <image>` copies the image
to `/boot/firmware/yakogl-default.bin`, keeps Linux's `config.txt` as
`config-linux.txt` (byte for byte), writes `config.txt` with the lines above,
and reads everything back. It does not reboot. `nodefault` (from Linux)
puts `config-linux.txt` back. A tryboot run refuses while `config-linux.txt`
exists.

After the reboot only a person can undo it: take the card out, copy
`config-linux.txt` over `config.txt` on a PC, put it back. The EEPROM is never
touched, so a bad image cannot make the board unbootable.

Watchdog: `rpi_wdog.c` arms the PM watchdog with a deadline
(`RPI_WDOG_HOLD_S`). With a tryboot, the reset returns to Linux. As the
default, the reset boots the same image again, in a loop. A product image
needs a watchdog without a deadline, fed only while the application is
healthy. That change is not made.

Not built: an unattended test of `config.txt` itself. Zephyr would have to set
the firmware's tryboot flag at start (property `0x00038064`, as Linux's
`drivers/firmware/raspberrypi.c` does for `reboot "0 tryboot"`), so that its
reset boots a Linux `tryboot.txt`. No code for it exists, and if the flag did
not survive the watchdog reset, the board would loop until a person came. The
tryboot run above already covers everything but the file name.

## On the boards

Each step in order; stop at the first that prints something else. The run
directories and verdict lines are as for any `board.sh` run.

### Verdin AM62P

1. `tools/board.sh am62p status` prints the hook variables and `install: ok`,
   `install: absent`, or (a board that has the TFTP-only hook)
   `install: stale: yako_hook differs from this script's`.
2. `tools/board.sh am62p install`: `installed; ...` (absent) or
   `upgraded in place (was: yako_hook differs from this script's); bootcmd
   unchanged` (stale). Then `status` again: `install: ok`. From now on an
   older `board.sh` refuses to run on this board ("yako_hook differs"): it
   must be this commit or later.
3. `BOARD_BOOT=uboot-mmc tools/board.sh am62p probe <logdir>`: the verdict line
   `rc=0 verdict=probe_ok boot=uboot-mmc row boot_verdict=load_failed`; the
   console has `yakogl-netboot: mmc load failed, booting Linux` and then
   Torizon's boot.
4. `BOARD_BOOT=uboot-mmc tools/board.sh am62p run <zephyr.bin> <logdir>` with an
   image that already has a TFTP row: `boot_verdict=done`, the console has
   `<n> bytes read in` with n = the image's size, and the same verdict as its
   TFTP row. Then `status`: no `once-*.bin` left in `/var/yakogl`.
5. Only with someone watching (it takes the board away from everyone):
   `BOARD_PERSON_PRESENT=1 tools/board.sh am62p default <zephyr.bin>`, reboot,
   see the image start twice (its reset boots it again), then undo:
   `python3 tools/am62p_netboot_uart.py typed $BOARD_UART raw.log ts.log
   'setenv yako_default; saveenv; boot'` stops the next autoboot and types the
   line. It waits only 120 s for U-Boot SPL (`typed: EXPIRED` otherwise), so
   start it again until it prints `echo exact: CR sent` (stderr); then stop it
   (it keeps capturing). Linux comes back; `tools/board.sh am62p nodefault`
   removes the file. If the bridge fails, a person resets the board at the
   prompt.

### Verdin iMX8M Plus

1. and 2. as on the AM62P.
3. `BOARD_BOOT=uboot-mmc tools/board.sh imx8mp probe <logdir>`: `rc=0
   verdict=probe_ok boot=uboot-mmc row boot_verdict=probe_ok`. `rc=6` means the
   hook never cleared the one-shot (board.sh disarmed it).
4. `BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run <zephyr.bin> <logdir>`:
   `boot_verdict=cycled` and the image's own record, e.g. `verdict=done ...
   result=record:APP`. A cleared record (`verdict=no_record`) means the image
   did not run: `ext4load` may be missing from this U-Boot (unmeasured), or
   the path is wrong.
5. **Needs a person** with a UART cable on the board: `default`, the reboot,
   and the undo at the U-Boot prompt (`setenv yako_default; saveenv; boot`).
   Without a UART the fallback is USB recovery mode and Toradex Easy Installer.

### Raspberry Pi 4

1. Nothing to install. `tools/board.sh pi4 run <zephyr.bin> <logdir>` (tryboot)
   is the test, as today.
2. **Needs a person** at the board: `BOARD_PERSON_PRESENT=1 tools/board.sh pi4
   default <zephyr.bin>`, reboot, watch the panel or the UART, then the undo
   with the SD card in a PC.
