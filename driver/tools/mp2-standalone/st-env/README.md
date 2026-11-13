# ST's default U-Boot environment of the STM32MP257F-DK

`stm32mp25-dk-initial-env.txt` is the environment that ST's U-Boot compiles
in for the STM32MP257F-DK, the one it falls back to when the stored
environment is not valid. The DK standalone scripts, their selftests and the
QEMU run check against it, so that a product that must leave `bootcmd`,
`boot_targets` and ST's recovery variables alone is tested against ST's
values and not a guess. #362 (plan), #1281 (DK bring-up).

- Source: `github.com/STMicroelectronics/u-boot`, tag
  **`v2023.10-stm32mp-r3.1`**, commit
  **`f8917a01b48e0f3bc4595cb39e2d5431774004d5`** (the tag peels to it; its
  version string is `2023.10-stm32mp-r3.1-rc3`). r3.1 is the U-Boot of the
  OpenSTLinux 6.x the DK runs (the pre-study names it for Linux 6.6).
- Built with `make stm32mp25_defconfig` and `DEVICE_TREE=stm32mp257f-dk`
  (the defconfig's own `CONFIG_DEFAULT_DEVICE_TREE` is `stm32mp257f-ev1`,
  `configs/stm32mp25_defconfig:8`; ST's build passes the board's tree), then
  `make u-boot-initial-env`, with the Zephyr SDK 1.0.1 `aarch64-zephyr-elf-`
  gcc. The DK tree does not change the environment (the env comes from
  `include/configs/` alone), so the file is the same for the EV1.
- **Regenerate:** `regen.sh` (idempotent; `--check` compares and writes
  nothing; exit 77 means no tree or compiler). `ST_UBOOT_TREE` defaults to
  `$CONNECTED_DEPS_DIR/st-uboot`, cloned at the tag when absent and refused
  when its HEAD is not the pinned commit. The build tree is
  `$CONNECTED_DEPS_DIR/st-uboot-build` (`ST_ENV_BUILD`), never `/tmp`.
  Checked 2026-11-13: `regen.sh --check` reproduces the file byte for byte.
  Not hand-derived.

## Reading the file

One `var=value` per line, in the order U-Boot's build writes them (sorted by
name, stable for equal names). Two caveats for a consumer:

- `boot_net_usb_start` appears twice (`usb start`, then `true`). U-Boot
  imports in order and the later wins: the live value is `true`
  (`include/configs/stm32mp25_common.h:95` vs `include/config_distro_bootcmd.h`).
- `boot_targets` ends with a space (`mmc1 ubifs0 mmc0 mmc2 usb0 pxe `).
- It is the *initial* env. At run time U-Boot adds `boot_device`,
  `boot_instance`, `fdtfile`, `ethaddr`, `preboot` and others
  (`arch/arm/mach-stm32mp/stm32mp2/cpu.c:190-270`), and the saved one on the
  DK's `u-boot-env` partition may differ from this file.

## Facts the other items rely on

All paths are in ST's tree at the commit above.

| Fact | Value | Source |
|---|---|---|
| `bootcmd` | `run bootcmd_stm32mp` (`CONFIG_BOOTCOMMAND`; the env file has it) | `configs/stm32mp25_defconfig:28` |
| `bootcmd_stm32mp` | echo `Boot over ${boot_device}${boot_instance}!`; for serial/usb `stm32prog`; else `run env_check`, then for `boot_device=mmc` **`env set boot_targets mmc${boot_instance}`** (an SD boot leaves only that card), nand/spi-nand `ubifs0 mmc0`, nor `mmc0`, then `run distro_bootcmd` | `include/configs/stm32mp25_st_common.h:26-41` (it replaces the generic one at `stm32mp25_common.h:75-90`) |
| `bootdelay` | 1 s | `configs/stm32mp25_defconfig:27` |
| `boot_targets` (before the `env set` above) | `mmc1 ubifs0 mmc0 mmc2 usb0 pxe ` | `include/configs/stm32mp25_common.h:60-66`, `include/config_distro_bootcmd.h:477` |
| `distro_bootcmd` | `for target in ${boot_targets}; do run bootcmd_${target}; done` | `include/config_distro_bootcmd.h:562-564` |
| `boot_prefixes` | `/ /boot/` | `include/config_distro_bootcmd.h:494` |
| `boot_scripts` | `boot.scr.uimg boot.scr` (yes, `boot.scr.uimg` first); `boot_script_dhcp=boot.scr.uimg` | `include/config_distro_bootcmd.h:495-496` |
| `boot_syslinux_conf` | `extlinux/extlinux.conf` | `include/config_distro_bootcmd.h:499` |
| per-partition scan | for each bootable partition (`part list ... -bootable`, else partition 1), for each prefix: `scan_dev_for_extlinux` then `scan_dev_for_scripts`; extlinux that boots never returns, a script that returns lets the loop go on | `include/config_distro_bootcmd.h:504-536` (the env file's `scan_dev_for_*`) |
| `altbootcmd` | **exists**: `run bootcmd_stm32mp` (the same boot as `bootcmd`) | `include/configs/stm32mp25_st_common.h:43` |
| who runs `altbootcmd` | only the forced boot mode "recovery" (`preboot=env set preboot; run altbootcmd`), set from the boot-mode pins/TAMP; **not** a boot counter | `arch/arm/mach-stm32mp/stm32mp2/cpu.c:267-269` |
| `bootcount`, `bootlimit`, `upgrade_available` | **none** in the initial env; no `CONFIG_BOOTCOUNT_*` is set (`# CONFIG_BOOTCOUNT_LIMIT is not set` in the built `.config`; `CMD_BOOTCOUNT` not in the defconfig), so nothing in U-Boot increments or stores a counter, and `altbootcmd` is never reached through a limit | `configs/stm32mp25_defconfig` (no such line); built `.config` |
| `env_check` | `if env info -p -d -q; then env save; fi` | `include/configs/stm32mp25_common.h:94` |
| when `env_check` saves | only when the **default** env is in use (`GD_FLG_ENV_DEFAULT`: no valid stored env) **and** the env is persistable (`env_get_location` not NOWHERE/UNKNOWN). A valid stored env is never rewritten by it. It runs on every `bootcmd_stm32mp` that is not serial/usb | `cmd/nvedit.c:1080-1140`; `board/st/stm32mp2/stm32mp2.c:540-575` |
| where the env is stored | by boot device: SD or eMMC boot (`BOOT_FLASH_SD/EMMC`) selects **MMC**, NAND `UBI`, NOR SPI flash; otherwise nowhere. `CONFIG_SYS_MMC_ENV_DEV=-1`, so the MMC device is the boot instance | `board/st/stm32mp2/stm32mp2.c:540-575, 604-612`; `configs/stm32mp25_defconfig:67-77` |
| MMC env place | `CONFIG_ENV_OFFSET=0x900000` / `CONFIG_ENV_OFFSET_REDUND=0x940000` (`CONFIG_ENV_SECT_SIZE=0x40000`) are the flash offsets and **do not apply** to the DK's SD: the GPT partition named `u-boot-env` is searched (DT `u-boot,mmc-env-partition`) and the env sits at the **top of that partition**, `CONFIG_ENV_SIZE=0x2000` (8 KiB). Copy 0 at `end - 0x2000`, the redundant copy (`CONFIG_SYS_REDUNDAND_ENVIRONMENT=y`) at `end - 0x4000`. For the 512 KiB partition the pre-study measured: copy 0 at **0x7e000**, copy 1 would be 0x7c000. A type-GUID lookup is the fallback | `env/mmc.c:57-95, 105-135`; `arch/arm/dts/stm32mp257f-dk-u-boot.dtsi:11`; `arch/arm/mach-stm32mp/Kconfig:33-34` (`ENV_SIZE 0x2000`); `configs/stm32mp25_defconfig:6-16, 72` |
| watchdog config | `CONFIG_WATCHDOG=y`, `CONFIG_WATCHDOG_TIMEOUT_MSECS=60000`, `CONFIG_WDT=y`, `CONFIG_WDT_STM32MP=y` (the IWDG driver), `CONFIG_WDT_ARM_SMC=y`; `CONFIG_WATCHDOG_AUTOSTART` **not set**; no `CONFIG_CMD_WDT` (so no `wdt` command) | `configs/stm32mp25_defconfig:183-185` and built `.config` |
| watchdog nodes | the DK enables only `arm_wdt` (`arm,smc-wdt`, SMC id `0xbc000000`), `timeout-sec = <32>`; `iwdg1`/`iwdg2` are `disabled` in the SoC tree | `arch/arm/dts/stm32mp251.dtsi:78-82, 1963-1972`; `arch/arm/dts/stm32mp257f-dk.dts:216-219` |
| does U-Boot pet it | **Possibly, if BL2 has armed it.** `initr_watchdog` starts a wdt only when autostart is set; the SMC driver sets it itself when `SMCWD_GET_TIMELEFT` says the watchdog is already running, and then `wdt_start` registers a `wdt_cyclic` that resets it every `reset_period` (1 s default) from `schedule()`. The cyclic only runs where U-Boot calls `schedule()` (console wait, `sleep`, `mmc`/`usb` loops, `sysboot`), not inside `go` into a standalone image | `drivers/watchdog/wdt-uclass.c:46-70, 80-92, 96`; `drivers/watchdog/arm_smc_wdt.c:95-110`; `common/cyclic.c:102` |

## Where the tree differs from the pre-study

`docs/notes/pre-study-stm32mp257f-dk.md` (section 1, `:244-280`; not edited):

- **0x7e000** is confirmed, as *the top of `u-boot-env` minus `ENV_SIZE`*
  (`0x80000 - 0x2000`), not as a fixed `CONFIG_ENV_OFFSET` (0x900000 is the
  NOR/flash value). A resized `u-boot-env` partition moves it. There is also a
  second, redundant copy 0x2000 below it that the pre-study does not mention;
  whether the DK's partition has ever held one was not read.
- "`env_check` saves a default environment when the stored one is not valid":
  true, with the limit above (default env in use *and* persistable).
- "The board's U-Boot has no `wdt` support": the **`wdt` command** is absent
  (matches), but the tree has the `arm,smc-wdt` driver enabled for the DK and
  it may pet the watchdog through a cyclic when BL2 started it. That is the
  tree's source reading, not a board result; the board's own U-Boot log
  (`WDT:   Started arm_wdt` or `Not starting`) and dk-facts decide. It
  changes what a `go` window assumes: U-Boot's own sleeping and loading steps
  may be refreshed, a standalone image still gets nothing.
- "The distro loop tries extlinux and leaves `boot.scr.uimg` alone while
  extlinux succeeds": per partition and prefix the loop looks for
  `extlinux/extlinux.conf` first, then `boot.scr.uimg`, `boot.scr`
  (`config_distro_bootcmd.h:504-536`). The pre-study lists `mmc0_extlinux/` and
  `mmc1_extlinux/` in bootfs, not `extlinux/`, so on the DK it is likely that
  `/boot.scr.uimg` (ST's, which sysboots `mmc${boot_instance}_extlinux`) is the
  step that runs, not the plain extlinux. A product `boot.scr.uimg` in the
  same place would replace ST's rather than precede it. This is a reading of
  names, to be confirmed by dk-facts (`ls` of bootfs and the script's text).
- "`boot.scr` does `env save`": nothing in the tree's default env does; it is
  a script on the board.
- `boot_targets` on an SD boot is reduced to `mmc${boot_instance}` by
  `bootcmd_stm32mp` before the loop (above), so a product reached through the
  distro loop is on the card the board booted from, and `mmc1`/`mmc0` ordering
  from the file does not apply.

## What it means for a product (not decided here)

- A route through the distro loop needs no `bootcmd` change; `bootcmd`,
  `bootcmd_stm32mp`, `distro_bootcmd` and `boot_targets` stay ST's.
- There is no ST boot counter to protect, only `altbootcmd` (same command as
  `bootcmd`, used by the recovery boot mode): a product's own count must not
  be named `bootcount`, `bootlimit`, `upgrade_available` or `altbootcmd`, and
  must not be stored by an `env save` that this file's `env_check` could
  overwrite. `env_check` never rewrites a valid stored env.
- Anything the product writes into the `u-boot-env` area competes with the
  stored env's CRC and the redundant copy; a count kept in a file in bootfs
  avoids it.
