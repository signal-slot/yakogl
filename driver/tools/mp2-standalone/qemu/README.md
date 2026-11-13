<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2-standalone/qemu: the DK standalone scripts in real U-Boot, under QEMU

A host-only harness: mainline U-Boot v2023.10 (`qemu_arm64_defconfig` plus
`mp2-standalone.config`) on QEMU's `virt` machine, with a **persistent
environment** (CFI flash, `saveenv`) and a **GPT microSD image with an ext4
`bootfs` partition**, as ST's distro loop sees the DK's card. A case is a boot
script, environment presets and N boots; the N boots are N separate QEMU runs on
the same flash and disk, so what a script saves is what the next boot reads.
No board, no network. The harness has no product script of its own: the one
case here, `demo-counter`, proves the harness (the product scripts and their
cases are other items').

| file | job |
| --- | --- |
| `run.sh` | entry: builds what it needs, runs the cases, prints `run.sh: N ok, M failed` or `run.sh: skip: <why>` |
| `build-uboot.sh` | builds U-Boot v2023.10 with the fragment into the cache; prints `u-boot.bin`. Reuses the compiler search and cache rules of `../../uboot-mp2/qemu/build-uboot.sh` (sourced, not changed) |
| `mp2-standalone.config` | the Kconfig fragment: env in flash, `CMD_SAVEENV`, `setexpr`/`itest`/`source`/`go`, bootstd (script and extlinux over mmc), MMC on PCI, ext4 |
| `drive.py` | the console side: cases, the QEMU boots, the assertions. Builds `boot.scr.uimg` with `tools/uboot_script.py` (no `mkimage`) |
| `mkdisk.sh` | `mkdisk.sh DISK BOOTFS_DIR`: 32 MiB GPT, partition 1 `fsbl` (placeholder), partition 2 `bootfs` (ext4, bootable flag), no root |
| `image.S` | the stand-in image: an arm64 Image header and code that prints and returns to `go`; staged as `/yakogl/mp2z.bin` in bootfs |
| `demo-counter.cmd` | the demo script: decrement `mp2s_test`, `saveenv`, `echo boots left <n>` |

```sh
driver/tools/mp2-standalone/qemu/run.sh    # "run.sh: N ok, 0 failed", or "run.sh: skip: <why>"
```

Expected log of the demo (three QEMU boots, `boots left 2`, `1`, `0`; after
each boot `bootcmd` is what the setup boot printed). Console logs of every boot
are `logs/<case>-setup.log`, `<case>-boot<n>.log` in the run directory, kept
after a failure or with `MP2S_QEMU_KEEP=1`.

## How a case runs

1. `drive.py` builds `bootfs/boot.scr.uimg` from the case's `.cmd`, copies the
   stand-in image to `bootfs/yakogl/mp2z.bin`, runs `mkdisk.sh`, and makes a
   fresh 64 MiB environment flash (QEMU `virt`'s second pflash, U-Boot's
   `ENV_ADDR` 0x04000000).
2. **setup** boot: stops autoboot, types the presets (`setenv ...`), `saveenv`,
   reads `printenv`.
3. **boot 1..N**: nothing typed. U-Boot's own `bootcmd` (`bootflow scan -lb`)
   finds `boot.scr.uimg` in bootfs and sources it; at the prompt the harness
   reads `printenv` and powers QEMU off.
4. The harness asserts per boot: no `bad CRC` (the stored environment was
   read), and `bootcmd`, `boot_targets`, `boot_prefixes`, `bootcount`,
   `bootlimit`, `altbootcmd`, `upgrade_available` equal the setup boot's. The
   case's own check then reads each boot's console lines and environment.

The `loop_*` cases prove the distro-loop claims the product route rests on
(`exit` continues to the next prefix, a `boot_prefixes` change mid-loop does
not change the walked list, `saveenv` in a sourced script persists, a corrupt or
missing script falls through). QEMU's bootstd walks prefixes differently, so
they preset ST's legacy loop text (`scan_dev_for_scripts`, `boot_a_script`,
`SCRIPT FAILED: continuing...`) and a `bootcmd` that runs it on the bootfs.
Their test scripts are `cases/*.cmd`; a `Case` lists them as `files` (bootfs
path to `.cmd`) and may `corrupt` one (data CRC broken).

To add a case, add a `Case(name, script, presets, boots, check)` to `cases()` in
`drive.py`; `MP2S_QEMU_ONLY=name` runs one.

## Tools and environment

- QEMU: `MP2_QEMU` or `QEMU_SYSTEM_AARCH64`, else `qemu-system-aarch64` on
  `PATH`, else `$CONNECTED_DEPS_DIR/zephyr-hosttools/sysroots/*/usr/bin/`. Any build with `virt`, `sdhci-pci`, `sd-card`. The Zephyr SDK host
  tools carry one (`zephyr-sdk-x86_64-hosttools-standalone-0.10.sh -y -d DIR`,
  binary in `DIR/sysroots/x86_64-pokysdk-linux/usr/bin/`).
- Cross compiler: `CROSS_COMPILE`, else `aarch64-linux-gnu-`,
  `aarch64-none-elf-` on `PATH`, else the Zephyr SDK's under
  `ZEPHYR_SDK_INSTALL_DIR` or `$CONNECTED_DEPS_DIR/zephyr-sdk-*`.
- U-Boot source: `UBOOT_V2023_10_SRC`, else `$CONNECTED_DEPS_DIR/u-boot-v2023.10`,
  else `$CACHE/u-boot-v2023.10`; absent: `build-uboot.sh` clones the `v2023.10`
  tag of `https://source.denx.de/u-boot/u-boot.git`. The build is out of tree.
- Cache: `MP2S_QEMU_CACHE`, else `$CONNECTED_DEPS_DIR/mp2-standalone-qemu`, else
  `$TMPDIR/mp2-standalone-qemu`, else `~/.cache/yakogl/mp2-standalone-qemu`;
  never `/tmp`, `/var/tmp` or the repository.
- `run.sh` exits 0 with `run.sh: skip: <why>` when QEMU, the cross compiler,
  `python3`, `mke2fs`, `sfdisk` or `truncate` is missing.

## Differences from the DK

- **bootstd, not the legacy distro loop.** QEMU's `qemu_arm64_defconfig` uses
  standard boot (`bootflow scan`); ST's 2023.10 for the stm32mp25 uses the
  legacy `distro_bootcmd` (`bootcmd_mmc0`, `boot_targets`, `scan_dev_for_scripts`).
  Both end in `source` of `boot.scr.uimg` from the bootfs partition; the
  scan order and `boot_targets` are not ST's. QEMU's default `boot_targets`
  has no mmc, so the demo case presets `boot_targets=mmc0` (saved by the
  setup boot, so it is the baseline of the "unchanged" checks). ST's own environment is
  another item's (`st-env`).
- The env is CFI flash, not ST's `mmc` env at 0x7e000. `saveenv` and
  `env_check` behave differently; what carries over is that a saved variable
  survives a reset and that a script must not touch `bootcmd`.
- The stand-in image returns from `go`; a Zephyr image does not.
- No BL2 watchdog: a hang on QEMU is bounded by the driver's timeouts only.
