<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 4: Zephyr as the default boot

Status (2026-10-03): **a procedure, not yet done on a board.** Today the Pi 4
boots Raspberry Pi OS by default, and each Zephyr image runs once through the
firmware's tryboot (`tools/board.sh pi4 run`). The product form has no Linux:
the board's own firmware boots `zephyr.bin` on every power-on. A persistent
boot change is made only with a person at the board (user rule), so this file
says how to make such a card; nothing here has been written to the board.

## 1. The boot partition

The Pi 4 loads its second-stage bootloader from SPI EEPROM, not from
`bootcode.bin` ([boot EEPROM flow][eeprom-flow]). The bootloader then reads the
first FAT partition of the card. In a tryboot of our images on 2026-10-03 the
UART log (`uart_2ndstage=1`) shows the firmware reading these files, in this
order (`records/pi4/es2-full-29460f96/` and `records/pi4/record_DEMO-29460f96.txt`,
outside git):

| file | from where | read as |
|------|------------|---------|
| `start4.elf`, `fixup4.dat` | Raspberry Pi OS boot partition | the firmware and its matched memory fix-up ([start_file, fixup_file][boot-options]) |
| `config.txt` (`tryboot.txt` in a tryboot) | written by us, below | the configuration |
| `bcm2711-rpi-4-b.dtb` | Raspberry Pi OS boot partition | the device tree the firmware loads and passes on |
| `overlays/overlay_map.dtb` | Raspberry Pi OS boot partition | the overlay map |
| `cmdline.txt` | Raspberry Pi OS boot partition | the kernel command line (Zephyr does not read it) |
| `zephyr.bin` | `build/.../zephyr/zephyr.bin` | the kernel, loaded at 0x200000 |

That is the set the firmware read. It is not proven to be the smallest set:
no card without `bcm2711-rpi-4-b.dtb`, `overlay_map.dtb` or `cmdline.txt` has
been booted. Keep all of them until such a card has been measured.

`config.txt` for a card that boots Zephyr every time. These are the same
lines that `tools/board.sh pi4` writes into `tryboot.txt` for each run; they booted
31 images on 2026-10-03:

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

| key | what it does | documentation |
|-----|--------------|---------------|
| `[all]` | the lines below apply to every boot | [conditional filters][filters] |
| `kernel=zephyr.bin` | the file to load as the kernel (the Pi 4 default is `kernel8.img`) | [kernel][kernel] |
| `arm_64bit=1` | start the kernel in 64-bit mode (already the Pi 4 default; kept explicit) | [arm_64bit][arm64] |
| `enable_uart=1` | the UART on GPIO 14/15, which carries every Zephyr console line | [enable_uart][uart] |
| `uart_2ndstage=1` | the firmware's own boot log on the UART; optional, useful on a bench | [uart_2ndstage][uart2] |
| `hdmi_group=2`, `hdmi_mode=87`, `hdmi_timings=...` | the 1920x720 bar panel, a custom DMT mode; the 17 fields in the documented order (h_active, h_sync_polarity, h_front_porch, h_sync_pulse, h_back_porch, v_active, v_sync_polarity, v_front_porch, v_sync_pulse, v_back_porch, v_sync_offset_a, v_sync_offset_b, pixel_rep, frame_rate, interlaced, pixel_freq, aspect_ratio) | [legacy video options][legacy-video] |

`hdmi_timings` is the custom mode that `hdmi_group=2` and `hdmi_mode=87`
select ([legacy video options][legacy-video]). The `hdmi_*` keys "only apply to
the legacy FKMS and firmware-based graphics driver"
([Raspberry Pi 4 HDMI pipeline][pi4-hdmi]). That is our case: Zephyr asks
the firmware for a framebuffer over the property mailbox. Do not copy Raspberry
Pi OS's `dtoverlay=vc4-kms-v3d` or `disable_fw_kms_setup=1` into this file:
they belong to Linux's KMS display driver.

## 2. Making the card from Raspberry Pi OS Lite

Only the firmware files come from Raspberry Pi OS. No Linux kernel, initramfs
or root file system goes on the card. These steps have not been carried out
yet.

1. Write Raspberry Pi OS Lite (64-bit) to a spare card with Raspberry Pi
   Imager, and mount its first (FAT) partition.
2. Make the product card: one MBR partition, FAT32. Copy into it from the
   Lite card: `start4.elf`, `fixup4.dat`, `bcm2711-rpi-4-b.dtb`,
   `overlays/overlay_map.dtb` and `cmdline.txt` (the set in section 1).
3. Copy `zephyr.bin` from the build and write `config.txt` as above.
4. Record the sha256 of every file on the card next to the build that made
   `zephyr.bin`. The firmware files are part of what was tested.

Not needed: `kernel8.img` and the other kernels, `initramfs*`, `start.elf`,
`start_x.elf` and the other firmware variants, `bootcode.bin` (the Pi 4 uses
EEPROM), every overlay except `overlay_map.dtb`, and the second (ext4)
partition.

## 3. Updating the image

- **Card swap (the only path that needs no new code).** Power off, take the
  card out, replace `zephyr.bin` on a PC, check its sha256, put the card back.
  The EEPROM is never touched, so a bad image cannot make the board
  unbootable: the next card swap fixes it.
- **A Linux service image through tryboot.** Keep Raspberry Pi OS on the card
  as a maintenance system that only a tryboot starts: `tryboot.txt` names its
  kernel, `config.txt` names `zephyr.bin`. The firmware clears the one-shot
  flag before it starts, so a crash or reset of the maintenance system goes
  back to `config.txt`, that is, to Zephyr ([tryboot][tryboot]). For A/B
  partitions without editing configuration files, see `autoboot.txt` and
  `tryboot_a_b` ([autoboot.txt][autoboot]).
  Missing piece: Zephyr cannot request a tryboot yet. Linux does it in the
  Raspberry Pi kernel's firmware driver (`drivers/firmware/raspberrypi.c`,
  branch rpi-6.12.y). When the reboot argument contains " tryboot", the driver
  sends the property `RPI_FIRMWARE_SET_REBOOT_FLAGS` (0x00038064) with bit 0
  set, then `RPI_FIRMWARE_NOTIFY_REBOOT`. No code in this repository sends that
  property (grep for `tryboot` and `0x00038064`, 2026-10-03). It also goes
  through the same property mailbox that fails today (section 5).

## 4. What the watchdog does

`driver/samples/zephyr/v3d_clear/src/rpi_wdog.c` arms the BCM2711 PM watchdog
at boot (10 s period, full reset). A kernel timer feeds it every 3 s while
`rpi_wdog_alive()` returns true and the uptime is below `RPI_WDOG_HOLD_S`.

- On a hang, a fault or a wedged GPU the feeding stops with the system, or
  `rpi_wdog_alive()` returns false (as the cluster demo does when a frame is
  over its budget). The SoC then resets within 10 s.
- In the tryboot form that reset clears the flag and boots Linux.
- **In the default-boot form the reset boots `zephyr.bin` again.** Today's test
  images would reset at their deadline (`RPI_WDOG_HOLD_S`, for example 240 s
  for v3d_deqp) and boot again, in a loop. A product image needs a watchdog
  with no time deadline, fed only while the application is healthy. That code
  change has not been made: `rpi_wdog_init()` always sets the deadline.

## 5. The property mailbox (found and fixed on 2026-10-03)

From 2026-10-01 every property-mailbox call of our Zephyr images returned
-116 (timeout): no framebuffer and no V3D clock raise. The cluster DEMO image
of 29460f96 printed `v3d_gl_cluster: FAIL (the firmware framebuffer was not
granted)`. The cause was two mailbox clients: Zephyr af17c0c8's rpi_4b board
enables its own mailbox and firmware driver, whose ISR drains the replies our
`rpi_mbox.c` polls for. `driver/samples/zephyr/common/rpi_4b_sram.dtsi` now
disables both (`driver/docs/rpi4-v3d-backend-plan.md`, "Zephyr", has the
measurement). The tryboot request of section 3 would use the same mailbox.

[eeprom-flow]: https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#eeprom-boot-flow
[boot-options]: https://www.raspberrypi.com/documentation/computers/config_txt.html#boot-options
[filters]: https://www.raspberrypi.com/documentation/computers/config_txt.html#conditional-filters
[kernel]: https://www.raspberrypi.com/documentation/computers/config_txt.html#kernel
[arm64]: https://www.raspberrypi.com/documentation/computers/config_txt.html#arm_64bit
[uart]: https://www.raspberrypi.com/documentation/computers/config_txt.html#enable_uart
[uart2]: https://www.raspberrypi.com/documentation/computers/config_txt.html#uart_2ndstage
[legacy-video]: https://www.raspberrypi.com/documentation/computers/legacy_config_txt.html#legacy-video-options
[pi4-hdmi]: https://www.raspberrypi.com/documentation/computers/legacy_config_txt.html#raspberry-pi-4-hdmi-pipeline
[tryboot]: https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#fail-safe-os-updates-tryboot
[autoboot]: https://www.raspberrypi.com/documentation/computers/config_txt.html#autoboot-txt
