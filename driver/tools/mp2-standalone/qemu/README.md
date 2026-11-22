<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2-standalone/qemu: the DK standalone scripts in real U-Boot, under QEMU

A host-only harness: mainline U-Boot v2023.10 (`qemu_arm64_defconfig` plus
`mp2-standalone.config`) on QEMU's `virt` machine, with a **persistent
environment** (CFI flash, `saveenv`) and a **GPT microSD image with an ext4
`bootfs` partition**, as ST's distro loop sees the DK's card. A case is a boot
script, environment presets and N boots; the N boots are N separate QEMU runs on
the same flash and disk, so what a script saves is what the next boot reads.
No board, no network. `demo-counter` proves the harness; the `loop_*` and
`env_*` cases prove the loop and environment claims with test scripts; the
`prod_redund_*` cases run the real product script `../mp2-standalone.cmd`
under the redundant environment.

| file | job |
| --- | --- |
| `run.sh` | entry: builds what it needs, runs the cases, prints `run.sh: N ok, M failed` or `run.sh: skip: <why>` |
| `build-uboot.sh` | builds U-Boot v2023.10 with the fragment into the cache; prints `u-boot.bin`. Reuses the compiler search and cache rules of `../../uboot-mp2/qemu/build-uboot.sh` (sourced, not changed). `build-uboot.sh redund` builds the `env_*` cases' U-Boot |
| `mp2-standalone.config` | the Kconfig fragment: env in flash, `CMD_SAVEENV`, `setexpr`/`itest`/`source`/`go`, bootstd (script and extlinux over mmc), MMC on PCI, ext4 |
| `mp2-standalone-redund.config` | layered on it by `redund`: the environment as the DK keeps it, `ENV_IS_IN_MMC` with `SYS_REDUNDAND_ENVIRONMENT`, two 8 KiB copies at the top of a 512 KiB `u-boot-env` partition; `env info` |
| `redund-default.env` | the `redund` U-Boot's compiled-in default environment: qemu-arm's addresses, `boot_targets=mmc0`, and ST's `env_check` run by `bootcmd` before the scan |
| `drive.py` | the console side: cases, the QEMU boots, the assertions. Builds `boot.scr.uimg` with `tools/uboot_script.py` (no `mkimage`); reads and breaks the environment copies with `tools/uboot_env.py`'s parser |
| `mkdisk.sh` | `mkdisk.sh DISK BOOTFS_DIR [env]`: 32 MiB GPT, partition 1 `fsbl` (placeholder), partition 2 `bootfs` (ext4, bootable flag), no root; with `env`, partition 3 `u-boot-env` (512 KiB) |
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

The `env_*` cases prove the two claims of the design note's section 8 on the
redundant environment. They run the `redund` U-Boot on a disk with a
`u-boot-env` partition, with `demo-counter.cmd` and `mp2s_test=3`; the setup
saves three times, so both copies hold the count. After each boot the harness
types `env info` and `env info -d -p` and reads both copies from the disk
image; between two boots a `Case(between={n: f})` breaks stored CRCs:

- `env_powercut_saveenv`: after boot 1 the copy its `saveenv` wrote (the
  newer, `mp2s_test=2`) gets a bad CRC, as a power cut during that save leaves
  it. Boot 2 loads the older copy without a warning and says `boots left 2`
  again (the one extra boot), its save rewrites the torn copy, and boots 3 and
  4 say `boots left 1` and `0`.
- `env_both_bad_default`: after boot 1 both copies get a bad CRC. Boot 2 says
  `*** Warning - bad CRC, using default environment`, `env_check` saves the
  defaults (`Saving Environment to MMC... Writing to MMC(0)... OK`) and the
  script finds no `mp2s_test`; boot 3 loads the saved defaults and
  `env_check` saves nothing.

The `prod_redund_*` cases run the **real product script** under the same
redundant environment: `../build.sh` builds `../mp2-standalone.cmd` into the
case's `boot.scr.uimg`, staged as `/yakogl/standalone/boot.scr.uimg` with the
stand-in image as `/yakogl/standalone/default.bin`. The bootfs also holds
`/mmc0_boot.scr.uimg` (`cases/prod-stock.cmd`, the stock prefix: on the DK
ST's extlinux) and `/boot.scr.uimg` (`cases/prod-root-stock.cmd`, what the
default environment's `bootflow scan` finds: on the DK ST's root script). The
setup presets the legacy loop (as the `loop_*` cases, with ST's `env_check`
run first, as `bootcmd_stm32mp` does), `bootcount=0`, an `altbootcmd`, and
"default 2" as the runner saves it (`boot_prefixes=/yakogl/standalone/ /mmc0_`,
`mp2s_prefixes=/mmc0_`, `mp2s_ver=1`, `mp2s_left=2`), saved three times so both
copies hold it. The script saves once per boot that loads a count of 1-9 and
once in the used-up branch (section 3 of
`docs/notes/stm32mp257f-dk-standalone.md`); each case cuts one of those saves
(`cut_newer` after that boot) or breaks both copies:

| case | after boot | boots, as the console shows them (section 7) |
| --- | --- | --- |
| `prod_redund_nocut` | (no cut) | left 1 + image, left 0 + image, used up, stock |
| `prod_redund_cut_save1` | 1 (2 to 1) | left 1 + image, **left 1** + image, left 0 + image, used up, stock |
| `prod_redund_cut_save2` | 2 (1 to 0) | left 1 + image, left 0 + image, **left 0** + image, used up, stock |
| `prod_redund_cut_restore` | 3 (the restore) | left 1 + image, left 0 + image, used up, **used up**, stock |
| `prod_redund_both_bad` | 1, both copies | left 1 + image; `bad CRC`, the defaults, `env_check` saves them, the stock root script; the stock root script |

"image" is the ordered run `Found U-Boot script /yakogl/standalone/...`, the
undo line, `boots left n`, `loaded ...`, `go 0xa0000000`, the stand-in's line,
`the image returned`, `SCRIPT FAILED: continuing...`, the stock prefix's line;
"used up" is `default used up, booting Linux` then the stock prefix, and the
store after it has `boot_prefixes=/mmc0_` and no `mp2s_*`; "stock" is the stock
prefix with no `mp2s:` line. Each case asserts the section 8 claim: after a
single cut the next boots end in the stock path within the count that was left
(the image runs at most N + 1 = 3 times), the count each boot loads never grows
(e.g. `2, 2, 1, 0, -1` for `cut_save1`, -1 once `mp2s_left` is gone), and the
console's `boots left` never grows. Per boot the harness compares `bootcmd`,
`boot_targets`, `bootcount`, `bootlimit`, `altbootcmd`, `upgrade_available`
(printenv and the stored copy) with the environment the boot **loaded** (the
copy a load takes, or `redund-default.env` when both are bad), and asserts that
`bootcmd`, `bootcount` and `altbootcmd` are never in the boot's saved-env diff
(stored after the boot against loaded before it). A boot that loaded a stored
copy may change only `mp2s_*`, `boot_prefixes` and what the loop itself sets
before it sources the script (`devtype`, `devnum`, `distro_bootpart`, and the
script load's `fileaddr`, `filesize`); a both-bad boot's diff is `env_check`'s
save of the defaults plus U-Boot's runtime variables (`ethaddr`,
`fdtcontroladdr`, `stdin`, `stdout`, `stderr`).

```sh
MP2S_QEMU_ONLY=prod_redund driver/tools/mp2-standalone/qemu/run.sh   # the five prod_redund_* cases
```

To add a case, add a `Case(name, script, presets, boots, check)` to `cases()` in
`drive.py`; `MP2S_QEMU_ONLY=name` runs the case `name`, or every case
`name_*` (`prod_redund` selects all `prod_redund_*`). `run.sh` builds the
`redund` U-Boot only when an `env_*` or `prod_redund*` case runs.

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
  survives a reset and that a script must not touch `bootcmd`. The `env_*`
  cases are the exception: their U-Boot keeps two copies on the card through
  mainline's `env/mmc.c` and `env_import_redund`, as ST's 2023.10 does, at
  absolute offsets (QEMU's device tree names no `u-boot,mmc-env-partition`).
  Its SD controller is on PCI, bound only once PCI is scanned, so it is built
  with `CONFIG_SYS_EARLY_PCI_INIT` (a board define, passed in `KCFLAGS`) to
  scan PCI before the environment loads; the DK's SDMMC is on the SoC.
  Its `bootcmd` is `run env_check; bootflow scan -lb`, not ST's
  `bootcmd_stm32mp`; `env_check` is ST's text.
- The stand-in image returns from `go`; a Zephyr image does not.
- No BL2 watchdog: a hang on QEMU is bounded by the driver's timeouts only.
