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

- AM62P: the board's own `bootcmd` loads SafeUI's Zephyr with
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
and caches off, as it does from kexec's purgatory. SafeUI measured the
AM62P case on 2026-09-05; the i.MX8MP case follows from the same code and
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
