<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# uboot-mp2/qemu: the DK one-shot scripts in real U-Boot, under QEMU

An optional host test that runs `../mp2-mmc.cmd` and `../mp2-net.cmd`, built by
`../build.sh`, in mainline U-Boot v2023.10 (`qemu_arm64_defconfig`) on QEMU's
`virt` machine. `../selftest.sh` lints the scripts; this runs them: hush,
`ext4size`/`ext4load` from an SD card, `dhcp`/`tftpboot` over QEMU's user-mode
network, `itest`/`setexpr` on the loaded header, and `go`. No board, no
network beyond QEMU's own. It does not change the scripts or anything else in
`..`.

| file | job |
| --- | --- |
| `build-uboot.sh` | builds U-Boot v2023.10 `qemu_arm64_defconfig` + a fragment into the cache; prints `u-boot.bin` |
| `run.sh` | builds what it needs, stages the SD card and the TFTP directory, runs the cases |
| `drive.py` | the console side: one QEMU boot per case, typed lines, the checks |
| `image.S` | the stand-in image: an arm64 Image header and code that prints and returns |

```sh
driver/tools/uboot-mp2/qemu/run.sh     # "run.sh: N ok, 0 failed", or "run.sh: skip: <why>"
```

`run.sh` prints `run.sh: skip: <why>` and exits 0 when `qemu-system-aarch64`
is not on `PATH` (or `MP2_QEMU` names none), when there is no aarch64 cross
compiler, or when `python3`, `mke2fs`, `sfdisk`, `truncate` or `sha256sum` is
missing; it never turns a host red for a missing tool. A first run builds
U-Boot (a few minutes); later runs take about a minute.

## Tools

- QEMU: any `qemu-system-aarch64` with `virt`, `sdhci-pci`, `sd-card`,
  `e1000` and user-mode networking (slirp). Tested with 10.0.2 from the Zephyr
  SDK host tools (`zephyr-sdk-x86_64-hosttools-standalone-0.10.sh -y -d DIR`;
  the binary is `DIR/sysroots/x86_64-pokysdk-linux/usr/bin/`): put its
  directory on `PATH` or set `MP2_QEMU`.
- Cross compiler: `CROSS_COMPILE`, else the first of `aarch64-linux-gnu-`,
  `aarch64-none-elf-` on `PATH`, else the Zephyr SDK's `aarch64-zephyr-elf-`
  under `ZEPHYR_SDK_INSTALL_DIR` or `$CONNECTED_DEPS_DIR/zephyr-sdk-*`. The
  Zephyr SDK 1.0.1 compiler builds U-Boot v2023.10 as is.
- U-Boot source: `MP2_QEMU_UBOOT_SRC`, else `$CACHE/u-boot-v2023.10` or
  `$CONNECTED_DEPS_DIR/u-boot-v2023.10`, else `build-uboot.sh` clones the
  `v2023.10` tag of `https://source.denx.de/u-boot/u-boot.git` into the cache.
  The build is out of tree (`O=`); the source is only read. Host build needs as
  for any U-Boot (OpenSSL headers, bison, flex, dtc).
- The cache: `MP2_QEMU_CACHE`, else `$CONNECTED_DEPS_DIR/uboot-mp2-qemu`, else
  `$TMPDIR/uboot-mp2-qemu` (unless `TMPDIR` is under `/tmp`), else
  `~/.cache/yakogl/uboot-mp2-qemu`. A cache under `/tmp`, `/var/tmp` or inside
  the repository is refused. The build tree is keyed by the fragment, the
  source and the compiler; a run's files go to `$CACHE/run.XXXXXX`, removed
  after a passing run, kept (with the console logs) after a failure or with
  `MP2_QEMU_KEEP=1`.

The fragment `build-uboot.sh` adds: MMC on PCI (`MMC`, `DM_MMC`,
`MMC_SDHCI`, `MMC_PCI`, `CMD_MMC`; `qemu_arm64_defconfig` has no MMC), the
commands the scripts use (`CMD_EXT4`, `CMD_ITEST`, `CMD_SETEXPR`,
`CMD_SOURCE`, `CMD_CACHE`, `CMD_DHCP`, `CMD_TFTPBOOT`, `CMD_GO`, `CMD_SAVEENV`;
most are on already), `TOOLS_MKEFICAPSULE` off (no GnuTLS needed), and
`BOOTCOMMAND="echo mp2q: stock bootcmd ran"` so the typed line's `run bootcmd`
shows the fallback without booting anything. Every fragment line is checked to
hold in `.config`. The `.scr` files are built by `../build.sh` with the
`mkimage` of that same U-Boot build on `PATH`.

## What QEMU changes, and what it does not

- **The load address is not changed.** QEMU `virt` RAM starts at 0x40000000;
  with `MP2_QEMU_MEM` (default `2G`) it ends at 0xc0000000, so the DK's window
  0xa0000000..0xb0000000 is RAM as it is on the DK, and the scripts' fixed
  `mp2z_load` and `mp2z_window` run unchanged. That RAM size is the override;
  U-Boot relocates itself to the top of RAM, above the window. With less
  than 2 GiB the window runs into U-Boot or past the end of RAM.
- **The medium.** `mp2-mmc.cmd` names `mmc`, so the disk image is an SD card on
  QEMU's `sdhci-pci` (U-Boot's `pci_mmc`), `mmc 0`. It is a 32 MiB GPT image:
  partition 1 a placeholder, partition 2 an ext4 "bootfs" holding
  `/yakogl/mp2-mmc.scr` and the images (the DK stages under `/boot/yakogl/`,
  `/yakogl/` inside bootfs). The TFTP route gets its files from QEMU's own
  TFTP server (`-netdev user,tftp=`) at 10.0.2.2; `dhcp` gets its lease from
  QEMU's DHCP. Serving through `tools/tftp_oneshot.py` is another item's.
- **The typed lines** are the ones in `../README.md`, character for character:
  `load mmc 0:2 ${scriptaddr} /yakogl/mp2-mmc.scr && source ${scriptaddr}; echo
  mp2z: back to bootcmd; run bootcmd`, and the net one with
  `10.0.2.2:mp2-net.scr`, each with the runner's `setenv mp2z_*;` prefix. QEMU's
  `scriptaddr` is 0x40200000 (the DK's 0x90100000). `boot_instance` is unset
  under QEMU, so the mmc prefix also sets `mp2z_dev 0` (an `mp2z_*` variable).
- **The image returns.** `image.S` checks that it runs at 0xa0000000, prints
  `mp2q: image ran at its link address 0xa0000000` on the PL011 and returns 0,
  so the run can read the environment after `go`. A Zephyr image of the DK
  never returns; on the DK `mp2z: the image returned` is never printed.
- **No watchdog, no secure firmware, EL1.** QEMU `virt` without
  `virtualization=on` enters U-Boot at EL1; the DK enters at EL2 after TF-A and
  OP-TEE, with BL2's IWDG armed. Entry state, the watchdog and the console
  runner's timing are the board run's.
- **The prompt** is `=> ` here, `STM32MP> ` on the DK.

## The cases

One QEMU boot per case, all from the same stored environment. Before the
cases one boot runs `saveenv` (the test's only one) so the environment is a
real stored one in QEMU's second pflash (U-Boot's `ENV_ADDR` 0x04000000).

| case | what the line does | expected |
| --- | --- | --- |
| `mmc-ok` | mp2-mmc, 4 KiB image, header `image_size` 0x2000 | loads, checks pass, `go`, image runs at 0xa0000000 |
| `mmc-missing` | `mp2z_file /yakogl/missing.bin` | `refused: ... not found on mmc 0:2` |
| `mmc-file-above-window` | sparse file of 0x10000001 bytes | `refused: file 10000001 bytes, above the 0x10000000-byte window` (sized before loading) |
| `mmc-image-size-above-window` | header `image_size` 0x10000001 | `refused: the image takes 0x10000001 bytes of RAM ...` |
| `mmc-no-magic` | 4 KiB of zeros | `refused: no ARM64 Image magic at byte 56 ...` |
| `mmc-no-part` | no `mp2z_part` | `refused: mp2z_part ... is not set` |
| `net-ok` | mp2-net, the same image over TFTP | loads, checks pass, `go`, image runs |
| `net-missing` | `mp2z_file missing.bin` (TFTP error 1) | `refused: tftpboot failed` |
| `net-image-size-above-window` | header `image_size` 0x10000001 | refused, as mmc |
| `net-no-server` | no `mp2z_server` | `refused: mp2z_server is not set` |
| `net-file-above-window` | 0x10000001 bytes over TFTP (the slowest case; `MP2_QEMU_BIG_TFTP=0` drops it) | `refused: file 10000001 bytes, above the ...` after the transfer |

Checks for each case:

1. The stored environment was read (no `bad CRC`).
2. The lines starting `mp2z: `, `mp2q: `, `## Starting application` and
   `## Application terminated` are exactly the expected ones, in order: the
   undo first; the source line; `loaded ... RAM footprint ...` (size and window
   checks passed) or the refusal; `mp2z: go 0xa0000000`, U-Boot's `## Starting
   application at 0xA0000000 ...` and the image's line; then the typed line's
   `mp2z: back to bootcmd` and the stock `bootcmd`. A refusal must be followed
   by the fallback: the script's `exit` returns to the typed line.
3. `go` is reached in the two `-ok` cases and in no other.
4. `printenv` after the line equals `printenv` before it, except the
   scripts' `mp2z_*`, U-Boot's `filesize`/`fileaddr` (set by every load), on the
   net route the variables `dhcp` and the network stack set (`ethact`,
   `ipaddr`, `netmask`, `gatewayip`, `serverip`, `dnsip`, `dnsip2`, `domain`,
   `hostname`, `bootfile`, `ntpserverip`), and `autoload=no`, which the typed
   net line itself sets (the script puts back the value it found, which is the
   typed line's). Any other change, `bootcmd` or `bootdelay` say, fails.
5. No `Saving Environment` on the console, the stored environment's flash
   image and the SD card image byte for byte as before (sha256).

After the cases one more boot's `printenv` must equal the first case's
`printenv` before its line: the environment after a reset is the one the
cases started from.

## The power-cut cases

After the cases above, four more boots type the one-shot line and kill QEMU
(SIGKILL: no `poweroff`, no flush) partway through, each from the same
`env.img` the setup boot stored:

| case | cut |
| --- | --- |
| `cut-mmc-after-undo`, `cut-net-after-undo` | as soon as `mp2z: undo: ...` is printed, before the load and before `go` |
| `cut-mmc-during-load` | 0.1 s after `mp2z: mmc 0:2 /yakogl/long.bin -> ...`, while `ext4load` reads a 192 MiB sparse image (`long.bin`: a valid header, `image_size` 0xc000000) |
| `cut-net-during-load` | 0.2 s after `tftpboot`'s first progress hashes, while the same image arrives over TFTP |

Checks for each cut (one output line each):

1. `mp2z: undo: ...` was printed before the cut.
2. The cut came before `go`: no `mp2z: loaded`, `mp2z: go` or `## Starting`.
3. The env pflash file and the SD card image have the sha256 they had right
   after setup, straight after the cut.
4. A new boot from the same `env.img` reads the environment (no `bad CRC`),
   its `printenv` equals the setup boot's `printenv` apart from `mp2z_*` and
   `filesize`/`fileaddr`, and `bootcmd` is the stock `echo mp2q: stock bootcmd
   ran` (the fragment's `BOOTCOMMAND`), equal to the setup value.
5. The env pflash file still has the setup sha256 after that reboot.

The cuts trigger on console output, not on a fixed time. The during-load cuts
land inside the load only best effort: if the load finishes first, check 2
(no `mp2z: loaded`) fails.

A copy of the scripts with `setenv bootdelay 5; saveenv` added after the undo
line fails checks 4, 5 and the final readback (checked once by hand, not
part of the run).

## What it does not show

- ST's U-Boot: this is mainline v2023.10, not `v2023.10-stm32mp-r3.1`. The
  commands the scripts use are the same code in both (`../README.md`, "go, not
  booti"), but ST's environment, `boot_instance`, prompt and `bootcmd` are the
  board's.
- That the window is clear of what ST's U-Boot and the secure firmware keep
  (the `bdinfo` check in `../README.md`), the IWDG, EL2 entry, the cache state a
  Zephyr image expects, and the console runner's typing and timing.
