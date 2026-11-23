<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK: boot Zephyr from ST's U-Boot (design, 2026-11-13)

Status: **design, written on the host from ST's source. Nothing here has run on
the board.** The facts that only the board can give were read
read-only on 2026-11-16 (each cited "board read 2026-11-16, <command>"); what
only a run at the U-Boot prompt can give is marked `TODO(board-run)`, with one
line saying what to record. The empty
"Board run" section at the end is for the run record.

Goal (plan #357, ROADMAP stage 5, issues #7 and #1281; item P10 of
`pre-study-stm32mp257f-dk.md`): a DK result counts as a **bootloader boot**,
the board's own U-Boot starting our image, the same sign-off as the i.MX8MP
(`imx8mp-netboot.md`) and the AM62P (`am62p-netboot.md`) have. The DK keeps
ST's chain (TF-A v2.10 BL2/BL31, OP-TEE 4.0, U-Boot 2023.10): the SCMI server
that powers the GPU lives in OP-TEE, so the direct routes of
`imx8mp-direct-boot.md`, `am62p-direct-boot.md` and `x2-direct-boot.md`, which
replace U-Boot proper, are not this board's (pre-study, "Boot route for
sign-off", and section 1). Our image is entered by U-Boot's `go`.

What exists (merged) and what this note decides:

| piece | where | state |
| --- | --- | --- |
| the two U-Boot scripts, `mkimage` step, host selftest | `driver/tools/uboot-mp2/` | merged; never run on a board |
| the typed one-shot and the phase scan of the console | `tools/mp2_uboot_console.py` | merged |
| the DK's Linux-side runner (a hook in the saved environment) | `tools/board-stm32mp2.sh`, `docs/notes/stm32mp257f-dk-bringup.md` | the current run method; **not** a bootloader sign-off route (below) |
| the runner of this note's routes | `driver/tools/uboot-mp2/oneshot.sh`, `oneshot_selftest.sh` | merged with a host selftest (plan #357, `mp2-backend`); never run on a board |
| the `stm32mp257f-dk` board in `tools/board.sh` | `BOARD_BOOT=uboot-tftp` / `uboot-mmc` | another item (`board-sh-dk`, after plan #349) |

"ST/" below is ST's U-Boot, tag `v2023.10-stm32mp-r3.1`
(github.com/STMicroelectronics/u-boot), file:line. U-Boot is GPL: these are
facts and citations, nothing is copied. "pre-study" is
`docs/notes/pre-study-stm32mp257f-dk.md`.

## 1. U-Boot as ST built it

The facts the design stands on, read from the source (the board's own binary
may differ: each place where that matters is marked):

- **The prompt and the autoboot window.** `CONFIG_SYS_PROMPT="STM32MP> "`
  (`ST/configs/stm32mp25_defconfig:30`), `CONFIG_BOOTDELAY=1` (`:27`),
  `CONFIG_BOOTCOMMAND="run bootcmd_stm32mp"` (`:28`). "Hit any key to stop
  autoboot" counts the delay in a loop that polls `tstc()` with a 10 ms
  `udelay` and stops on any key (`ST/common/autoboot.c:371-406`); the window
  is about a second, so a runner sends a key from the first BL2 line on
  (`tools/mp2_uboot_console.py`, `STOP_KEY`, `KEY_PERIOD`). The defconfig sets
  no `CONFIG_AUTOBOOT_*`, so `autoboot_keyed()` is false
  (`ST/include/autoboot.h:36-40`, `ST/common/autoboot.c:418`) and the single-key
  loop above is the one compiled in; the keyed path, which reads
  `bootdelaykey` and `bootstopkey` (`autoboot.c:240-241`), is not used, and no
  such variable is set in the stock environment's source. The stored
  environment agrees: `bootdelay=1`, and no `bootdelaykey`, `bootstopkey` or
  `stopkey` is set (board read 2026-11-16, the decoded copy 0 of the
  `u-boot-env` partition, `dd` + `tools/uboot_env.py print`).
- **Console line length.** `CONFIG_SYS_CBSIZE` has no ST override
  (`grep` of `ST/configs/stm32mp25_defconfig` and `ST/include/configs/stm32mp25*.h`
  finds none), so it is Kconfig's default **1024** (`ST/cmd/Kconfig:78-85`,
  the `default 1024` line is :85). The console buffer is `CONFIG_SYS_CBSIZE + 1`
  (`ST/common/cli_readline.c:24`) and the reader stops taking characters at
  `CBSIZE - 2` (`:534`). The typed lines of the one-shot are 116 (microSD) and 146 (TFTP)
  characters with a one-digit device and partition, a ten-character token and
  a fifteen-character server address (the lines of `driver/tools/uboot-mp2/README.md`,
  "The route"), a seventh of the limit or less; the optional `setenv mp2z_*;`
  prefixes the README names add a few tens of bytes.
- **The environment** is on the medium U-Boot booted from, in the partition
  named `u-boot-env`: `CONFIG_ENV_IS_IN_MMC=y` (`defconfig:69`),
  `CONFIG_SYS_REDUNDAND_ENVIRONMENT=y` (`:72`), `CONFIG_SYS_MMC_ENV_DEV=-1`
  (`:77`, so the device is the boot instance,
  `ST/board/st/stm32mp2/stm32mp2.c:604-613`), the DK's device tree names the
  partition (`u-boot,mmc-env-partition = "u-boot-env"`,
  `ST/arch/arm/dts/stm32mp257f-dk-u-boot.dtsi:11`). `env/mmc.c` puts the
  copy at the **top of the partition**: offset `start + size - (1 + copy) *
  ENV_SIZE` (`ST/env/mmc.c:91-92`, the partition lookup `:97-132`), with `ENV_SIZE` 0x2000
  (`ST/arch/arm/mach-stm32mp/Kconfig:33-34`). In the 512 KiB partition that is
  **0x7e000 for copy 0 and 0x7c000 for the redundant copy 1**, and 0x7e000 is
  where the board read found the CRC, a flag byte and the strings (pre-study,
  section 1). The layout of a copy is `crc32` (4 bytes, over the data), `flags`
  (1 byte, the redundancy flag) and the data (`ST/include/env_internal.h:59-64`,
  `:80-86`). **Read on the board** (board read 2026-11-16, `lsblk -o
  NAME,SIZE,TYPE,PARTLABEL,MOUNTPOINT` and `dd if=<u-boot-env partition>
  bs=64k status=none` into a host file): the microSD is `mmcblk0` (59.5 GiB)
  and `u-boot-env` is its partition 7, **512 KiB**, so the offsets above
  hold. **Copy 0 (0x7e000) is valid**: CRC 0x7e7016c1, flag byte 0x02,
  73 variables, 5489 of 8187 data bytes used. **Copy 1 (0x7c000) is all
  zeros: stored CRC 0x00000000, which does not match its data, so it is
  invalid** (never written, or erased). Flag 0x02 against an invalid other
  copy means U-Boot uses copy 0 and a later `env save` writes copy 1 with
  flag 0x03. `tools/uboot_env.py ... check` therefore exits 2 ("1 bad
  copy(ies): copy 2") on this image although the board's U-Boot reads it
  fine (see "Contradictions" in section 11). The eMMC (`mmcblk1`, 7.3 GiB)
  carries its own `u-boot-env` (1 MiB) and `bootfs`; it was not read.
  **The layout, in one table** (ST's tree, tag `v2023.10-stm32mp-r3.1`;
  `uboot_env.py` numbers the copies 1 and 2, U-Boot's `env/mmc.c` 0 and 1):

  | item | value | source |
  | --- | --- | --- |
  | size of one copy | `CONFIG_ENV_SIZE` 0x2000 (8 KiB) | `ST/arch/arm/mach-stm32mp/Kconfig:33-34`; the defconfig sets none |
  | header | 4 bytes CRC32, then 1 flag byte, because the environment is redundant; the data is 0x2000 - 5 bytes | `ST/include/env_internal.h:59-64`, `:80-86` |
  | redundant | `CONFIG_SYS_REDUNDAND_ENVIRONMENT=y` | `ST/configs/stm32mp25_defconfig:72` |
  | copy 0 (`uboot_env.py` copy 1) | partition start + size - 0x2000: **0x7e000** in the 512 KiB partition | `ST/env/mmc.c:91-92` with `copy` = 0 (`:422`) |
  | copy 1 (`uboot_env.py` copy 2) | partition start + size - 2 * 0x2000: **0x7c000**, *below* copy 0 | `ST/env/mmc.c:91-92` with `copy` = 1 (`:423`) |
  | the partition is found by name | `u-boot-env`, no `CONFIG_ENV_MMC_PARTITION`, so the device tree's `u-boot,mmc-env-partition` | `ST/env/mmc.c:117-127`, `ST/arch/arm/dts/stm32mp257f-dk-u-boot.dtsi:11` |
  | the device | `CONFIG_SYS_MMC_ENV_DEV=-1`, the boot instance | `ST/configs/stm32mp25_defconfig:77` |
  | the active copy | both CRCs valid: the higher flag byte, except 0xff against 0x00 where the 0x00 one is newer; equal flags: copy 0 | `ST/env/common.c:365-374` |
  | a write | the other copy, flag = previous + 1 | `ST/env/common.c:424`, `ST/env/mmc.c:270-299` |

  Because copy 1 sits below copy 0, `uboot_env.py`'s default for the second
  offset (`--offset + --size`) is wrong here and **`--offset2` must be given**.
  The command that reads a `dd` image of the whole partition (read-only) is

  ```
  tools/uboot_env.py --size 0x2000 --redundant --offset 0x7e000 --offset2 0x7c000 check IMAGE
  tools/uboot_env.py --size 0x2000 --redundant --offset 0x7e000 --offset2 0x7c000 print IMAGE
  ```

  (`env-layout-check` found this reads the DK's environment byte for byte as
  ST's own tools do; the board read of 2026-11-16 confirms the offsets and
  the CRC of copy 0 on the DK. Because copy 1 is zeros, `check` and `print`
  refuse the whole image **unless `--allow-blank-copy` is given**, which
  accepts an all-zero copy and loads the other one; the runner and the
  collector pass it. Without it, copy 0's bytes over copy 1's in a host-side
  copy of the image first, as the read did.)

  **Linux's `fw_printenv` does not run as shipped** (board read 2026-11-16,
  `fw_printenv` prints "Cannot initialize environment"; `cat /etc/fw_env.config`:
  no such file; `ls /etc/fw_env.config*`: `.mmc`, `.nand`, `.nor` only).
  `/etc/fw_env.config.mmc` names `/dev/disk/by-partlabel/u-boot-env` twice,
  both `-0x2000 0x2000`, i.e. the **same** copy for both entries, and
  `fw_printenv -c /etc/fw_env.config.mmc` prints "Cannot read environment,
  using default". There is no `fw_setenv` (`/usr/sbin/fw_*` absent;
  `/usr/bin/fw_printenv` is the only one). So the stored environment is read
  by `dd` of the partition and `tools/uboot_env.py`, as above, and neither the
  runner nor the collector depends on `fw_printenv`:
  `tools/board-stm32mp2-uboot.sh:533-583` (`env_read`) runs
  `dd if=/dev/disk/by-partlabel/u-boot-env bs=64k status=none` over ssh
  (`:546-549`, `:565`), insists on exactly 512 KiB (`:568-569`), and runs
  `python3 -I tools/uboot_env.py --size 0x2000 --redundant --offset 0x7e000
  --offset2 0x7c000 --allow-blank-copy check` and then `print` on the host
  (`ENV_LAYOUT`, `:548`; `:570-573`).
  `tools/probes/stm32mp257f_dk/env_read/collect.sh:60-64` keeps
  `fw_printenv`'s output only as a secondary record (its failure is not an
  error) and dumps the partition the same way; `summarize.py:65-75` takes the
  variables from `u-boot-env.bin`. `--allow-blank-copy` (`uboot_env.py:21`,
  `:259-263`) accepts one **all-zero** copy, which is the stock card's copy 1
  (see the contradictions below); any other bad CRC is still refused.
- **`bootcmd` and `env_check`.** The compiled-in `bootcmd_stm32mp` prints
  "Boot over ...", runs `env_check`, sets `boot_targets` to `mmc${boot_instance}`
  when it booted from mmc, and runs `distro_bootcmd`
  (`ST/include/configs/stm32mp25_common.h:75-87`; the ST-board variant,
  `ST/include/configs/stm32mp25_st_common.h:26-40`, is the same for mmc).
  `env_check` is `if env info -p -d -q; then env save; fi`
  (`stm32mp25_common.h:94`). `env info -p -d -q` succeeds only when the
  environment in use is the **default** one (`gd->flags & GD_FLG_ENV_DEFAULT`,
  `ST/cmd/nvedit.c:1117`) **and** can be persisted (`:1127-1131`; the results are
  ANDed, `:1145-1146`). So a valid stored environment is never rewritten by the
  stock boot; an invalid one (bad CRC on both copies) is replaced by the
  defaults and saved. This is the one way the stock path writes the
  environment, and it matters for the undo (section 8).
- **The distro loop.** `boot_targets` defaults to the order mmc1, ubifs0, mmc0,
  mmc2, usb0, pxe (`stm32mp25_common.h:60-66`); on an mmc boot it is narrowed to
  the boot instance (above). For each device and bootable partition,
  `scan_dev_for_boot` runs, for each prefix in `boot_prefixes` (`/ /boot/`,
  `ST/include/config_distro_bootcmd.h:494`), **first** `scan_dev_for_extlinux`
  (`${prefix}extlinux/extlinux.conf`, `:499`, `:504-511`), **then**
  `scan_dev_for_scripts` (`boot.scr.uimg`, then `boot.scr`, `:495`, `:518-529`),
  `:530-539` (the two `run` lines are `:534-535`). A successful extlinux boot does not return, so a script is reached
  only after extlinux has failed. The pre-study found `mmc0_extlinux/` and
  `mmc1_extlinux/` on the board's bootfs, not the source's `extlinux/`:
  **The board's values differ from the source's here** (board read
  2026-11-16, the stored environment's copy 0): `boot_targets=mmc0`,
  `boot_device=mmc`, `boot_instance=0`, `boot_prefixes=/mmc0_` (not `/ /boot/`),
  `boot_syslinux_conf=extlinux/extlinux.conf`, `boot_scripts=boot.scr.uimg
  boot.scr`, `bootfstype=ext4`, `devplist=8`. With the prefix `/mmc0_` the
  loop looks for `/mmc0_extlinux/extlinux.conf` (present, 302 bytes) and then
  `/mmc0_boot.scr.uimg` (absent); the stock `/boot.scr.uimg` (4237 bytes) is
  **not** reached by the loop. `bootcmd=run bootcmd_stm32mp` and
  `altbootcmd=run bootcmd_stm32mp`; `bootcmd_stm32mp` is the compiled-in text
  quoted above (`env_check`, then `boot_targets` narrowed to
  `mmc${boot_instance}`, then `distro_bootcmd`). `scan_dev_for_boot_part`
  lists the bootable partitions of the device (`part list ... -bootable`) and
  the bootfs is partition 8 (`devplist=8`; `/boot` is `/dev/mmcblk0p8`, 64 MiB,
  34500 KiB free of 60033 KiB, board read 2026-11-16, `df -P /boot`). The
  stored environment also holds `fileaddr=90100000` and `filesize=108d`
  (4237 bytes, the size of the stock `boot.scr.uimg`): an earlier boot loaded
  that script to `scriptaddr` and the environment was saved afterwards.
  `/boot` (board read 2026-11-16, `ls -lR /boot`; sha256 from `sha256sum`):
  `Image.gz` -> `Image.gz-6.6.129` (11221833 bytes, sha256 2e7f9a88...ca87),
  `boot.scr.uimg` (4237, 2aa6e59e...2c70), the DTBs (`stm32mp257f-dk.dtb`
  119375 bytes, sha256 dc0a182d...0232, among 15 others),
  `st-image-resize-initrd` (8647706), two splash bitmaps, `devicetree/`
  (one `.dtbo`), `mmc0_extlinux/` (`extlinux.conf` 302 bytes, sha256
  814f707a...25e7, and three board variants) and `mmc1_extlinux/`
  (`extlinux.conf` 302 bytes, sha256 3e412763...bf2a, and one variant).
  The `mmc0_` file boots `/Image.gz` with `FDTDIR /` and the initrd, its
  `APPEND` naming the SD card's rootfs by PARTUUID. The `mmc1_` file differs
  only in that PARTUUID (the eMMC's rootfs). There is no `yakogl/` directory
  at the time of the read. The full 64-hex sha256 of every file is in the
  `env_read` probe's output (`tools/probes/stm32mp257f_dk/env_read`), the
  tool the undo and the c5 comparison use.
- **Memory layout variables** (`stm32mp25_common.h:101-118`): `kernel_addr_r`
  0x8a000000, `fdt_addr_r` 0x90000000, **`scriptaddr` 0x90100000**,
  `pxefile_addr_r` 0x90200000, `fdtoverlay_addr_r` 0x90300000,
  `ramdisk_addr_r` 0x90400000, `kernel_comp_addr_r` 0x84000000 (size
  0x4000000). `CONFIG_SYS_LOAD_ADDR=0x84000000` (`defconfig:15`), the stack
  before relocation `CONFIG_CUSTOM_SYS_INIT_SP_ADDR=0x90000000` (`:5`).
  Board read 2026-11-16 (the stored environment's copy 0, decoded):
  `kernel_addr_r=0x8a000000`, `fdt_addr_r=0x90000000`,
  `scriptaddr=0x90100000`, `ramdisk_addr_r=0x90400000`,
  `pxefile_addr_r=0x90200000`, `fdtoverlay_addr_r=0x90300000`,
  `kernel_comp_addr_r=0x84000000`, `kernel_comp_size=0x04000000`,
  `loadaddr=0x84000000`: **all equal the compiled-in defaults** and none
  overlaps 0xa0000000..0xb0000000. The scripts still never hard-code
  `scriptaddr` (section 3).
- **Net.** `CONFIG_SYS_DISABLE_AUTOLOAD=y` (`defconfig:54`; `ST/cmd/Kconfig:1893`):
  `dhcp` does not fetch the DHCP bootfile by itself. The scripts set
  `autoload no` anyway and put it back. The ethernet is the DWC QoS MAC
  (`defconfig:135`). `tftpboot` takes `<addr> <server>:<file>`
  (`ST/cmd/net.c:55`, `:63`; `ST/net/net.c:1703`, `net_parse_bootfile`), so the
  server is named per call and the DHCP reply's `serverip` is not relied on.
  `filesize` is set as hex without `0x` after a transfer (`ST/net/net.c:721`),
  which is why the scripts compare with `itest`, not `test`.

## 2. How `go` enters the image

`go` is `do_go` (`ST/cmd/boot.c:24`, `U_BOOT_CMD` `:50`, `CONFIG_CMD_GO` is
default y, `ST/cmd/Kconfig:474-476`). It prints `## Starting application at
0x... ...`, flushes the console and calls `do_go_exec((void *)addr, argc - 1,
argv + 1)` (`cmd/boot.c:34-41`). The arm64 build uses the weak `do_go_exec`
(`cmd/boot.c:17-22`): `arch/arm/lib/cmd_boot.c:34`, the Thumb variant that ORs
bit 0 into the address, is built only for `CONFIG_CPU_V7M`
(`ST/arch/arm/lib/Makefile:31`). So the entry is a plain function call:

- **Exception level:** none is changed. U-Boot runs at the level TF-A's BL31
  gave it, and `go` does not call `armv8_switch_to_el2` or any EL switch.
  U-Boot's own start code branches on the current level (`ST/arch/arm/cpu/armv8/start.S:120`,
  `:155`, `:360`; it sets `HCR_EL2.AMO` and its own `VBAR_EL2` at
  `:127-131` when it is at EL2). The pre-study (section 1, Q8, Q13) records
  EL2 as read from the board. `TODO(board-run)`: the image's own record
  (`mp2-handover:` line, `driver/zephyr/lib/board/stm32mp2/stm32mp2.c:38-49`:
  `el`, `sctlr`, `m`, `c`, `i`, `x0..x3`) is the measurement; this note does
  not assume it.
- **Registers:** `x0 = argc - 1`, `x1 = argv + 1` for the called function, so
  for `go 0xa0000000`: `x0 = 1` and `x1` a pointer to the argument array in
  U-Boot's memory (`cmd/boot.c:41`). **No device tree pointer is in `x0`**
  (unlike `booti`'s Linux entry). `x2`, `x3` are whatever U-Boot's call left
  in them. The record's comment in `stm32mp2.c:43-45` says "x0 = 0 and x1
  non-null"; by the source above `x0` should be 1: the board run settles it,
  and the image prints the values and asserts none.
- **Caches and MMU:** U-Boot enables the MMU and data cache early
  (`ST/arch/arm/mach-stm32mp/stm32mp2/cpu.c:36-58`, `early_enable_caches` and
  `arch_cpu_init`) and
  rebuilds its page tables after relocation (`:73-96`, `enable_caches`).
  `go` does nothing about them, and `cleanup_before_linux` is only on the
  `booti`/`bootm` path (`announce_and_cleanup`, `ST/arch/arm/lib/bootm.c:82`,
  called at `:305` and `:366`; the other call, `:428`, is VxWorks's). So the script
  does it itself, as the sibling boards' one-shots do: `dcache flush; dcache
  off; icache off; go` (`ST/cmd/cache.c:27-85`; `dcache_disable` clears
  `SCTLR.C` and `SCTLR.M` and flushes, `ST/arch/arm/cpu/armv8/cache_v8.c:579-603`;
  `icache_disable` clears `SCTLR.I`, `:812-815`). The image's `reset.S` expects
  MMU and caches off. The flush matters because the image was just written by
  the CPU (DMA from the SD or the MAC, then maybe copied through a cached
  mapping): the image's own instruction fetches must see the bytes.
- **Interrupts:** U-Boot on arm64 does not enable IRQ or FIQ for itself (its
  only DAIF change is unmasking SError, `ST/arch/arm/cpu/armv8/start.S:139`), so
  the image starts with IRQ and FIQ masked and SError unmasked. The GIC is as
  TF-A and OP-TEE left it (SGIs 8-15 and PPIs 29 and 31 are theirs; pre-study
  Q13). U-Boot's exception vectors stay in `VBAR_EL2` until the image
  installs its own. Secondary cores stay where BL31 parked them.
  `TODO(board-run)`: the record's `el`, `m`, `c`, `i` and the first lines
  after the handover.
- **The watchdog is still armed** (section 7), and U-Boot's code is not running
  after `go`: nothing in U-Boot feeds it from then on.
- **What does not return:** after `go` the image ends in a PSCI `SYSTEM_RESET`
  (`mp2: ... PSCI SYSTEM_RESET at <n> ms`, the console tool's `image_end`) or
  is reset by the watchdog; if it did return, the script's last line would
  print `mp2z: the image returned` and the typed line would continue to
  `run bootcmd`.

## 3. Load addresses (a)

| what | address | why |
| --- | --- | --- |
| the image | **0xa0000000**, `go 0xa0000000` | the link address: `zephyr,sram` of `driver/boards/st/stm32mp257f_dk` (a 256 MiB `memory@a0000000`), `YAKO_ADDR` of `tools/board-stm32mp2.sh`, `mp2z_load` of the scripts. The image runs only there, so it is loaded in place, never `booti`'d (which moves it: `ST/cmd/booti.c:77-86`, `ST/arch/arm/lib/image.c:70-75`) |
| the script | **`${scriptaddr}`**, read from the live environment | the compiled-in value is 0x90100000 (`stm32mp25_common.h:105`, `:113`), but the stored environment is the board's, and it holds 0x90100000 too (board read 2026-11-16, the stored environment's copy 0, `dd` + `tools/uboot_env.py print`); the typed line uses `${scriptaddr}` and nothing hard-codes it |
| the window | 0xa0000000 .. 0xb0000000 | `mp2z_window` = 0x10000000; `LOAD_END` in the runner |
| the size guard | the file and the header's `image_size` stay below **255 MiB** | see below |

**Why the window is safe.** Each fact is from ST's tree unless it says so:

- DDR is 4 GiB at 0x80000000 on the DK
  (`ST/arch/arm/dts/stm32mp257f-dk.dts:142-144`; `STM32_DDR_BASE` and
  `STM32_DDR_SIZE`, `ST/arch/arm/mach-stm32mp/include/mach/stm32.h:181-183`).
- U-Boot relocates **to the top of RAM**, clamped below 4 GiB:
  `gd->ram_top = clamp_val(ram_base + ram_size, 0, SZ_4G - 1)`, then a block of
  `ALIGN(8 MiB + CONFIG_SYS_MALLOC_LEN + mon_len, 2 MiB)` is taken from the top
  not covered by a reserved region (`ST/arch/arm/mach-stm32mp/dram_init.c:52-83`,
  `ST/common/board_f.c:378-380`; `SYS_MALLOC_LEN` is 0x2000000,
  `ST/arch/arm/mach-stm32mp/Kconfig:30-31`). The DK's reserved DDR regions
  (`ST/arch/arm/dts/stm32mp257f-dk-resmem.dtsi:112-181`) are 0x80000000 to
  0x84000000 (several small ones and the 32 MiB at 0x82000000), the 64 MiB
  `gpu-reserved` at 0xfa800000 and two at 0xfe800000 and 0xff000000. So U-Boot,
  its heap and its stack sit at or above roughly 0xf0000000, hundreds of MiB
  above the window; the copy of U-Boot's own relocation, the `fdt_blob`, the
  `bd` and the malloc area are in that same block (`bdinfo` prints
  `relocaddr`, `sp start` and `fdt_blob`, `ST/cmd/bdinfo.c:153-158`,
  `ST/arch/arm/lib/bdinfo.c:47`). `TODO(board-run)`: `bdinfo` at the
  prompt, once, to confirm `relocaddr` is far above 0xb0000000 (it needs the
  U-Boot console; Linux cannot read it). The source reading says so; this is
  a source reading, not a measurement. The stored `fdtcontroladdr=f80048b0`
  and `api_address=f80747f0` (board read 2026-11-16, the stored environment)
  are U-Boot's own device-tree and API addresses of an earlier boot, far
  above 0xb0000000, which agrees with the reading.
- U-Boot's working addresses are all below the window: stack before
  relocation 0x90000000, `kernel_comp` 0x84000000-0x88000000, `kernel_addr_r`
  0x8a000000, `fdt_addr_r`, `scriptaddr`, `pxefile`, overlay and ramdisk at
  0x90000000-0x90400000 and up (`stm32mp25_common.h:101-118`), `SYS_LOAD_ADDR`
  0x84000000 (`defconfig:15`). The fastboot buffer is 0x84000000
  (`defconfig:85`).
- The secure side is clear too: TF-A's BL31 and the Cortex-M33 and DDR-parameter
  regions are in internal SRAM at 0x0a000000-0x0a09ffff (`resmem.dtsi:20-71`),
  not 0xa0000000 (the digits are easy to confuse); OP-TEE's reserved DDR is at
  the top (above); the DK's Zephyr board file keeps the GPU carveout out of
  `zephyr,sram` (`driver/boards/st/stm32mp257f_dk`).
- The scripts guard `filesize` at most `mp2z_window` and the header's
  `image_size` (byte 16, 64 bits: the RAM the image takes from its start, .bss
  included) at most the window, so neither the file nor the image's footprint
  can pass 0xb0000000 (`driver/tools/uboot-mp2/mp2-mmc.cmd`).

**Summary of the window check.** The plan's `go-window-overlap` item (a
read-only check of the window against U-Boot's and the secure firmware's
memory) handed its summary paragraph to this item, but the paragraph's text did
not reach the agent that wrote this note; this note does not quote it and does
not claim to. What it states is the derivation above, from ST's source:
nothing U-Boot or the secure side keeps is in 0xa0000000..0xb0000000. The
handoff's rules that this note takes over: load the image to 0xa0000000 and the
script to `${scriptaddr}`; read the addresses from the live environment; keep
the image's load size under 255 MiB; and the one point resting on the source
alone is U-Boot's relocation address, which `bdinfo` confirms
(`TODO(board-run)`, above). The 255 MiB guard is one MiB tighter than the
committed scripts, which compare against the full 256 MiB window
(`mp2z_window` = 0x10000000): an image between 255 and 256 MiB passes the
scripts and fails the handoff's rule. Images are megabytes; a planner may want
the scripts or the runner to carry the 255 MiB figure (not changed by this
item).

One window problem the scripts cannot avoid: `tftpboot` writes before it knows
the size, so a file larger than the window is written past 0xb0000000 before
the script refuses (`driver/tools/uboot-mp2/README.md`, "What the scripts
cannot check"). The runner guards the size on the host before it stages the
file (as `nb_check_image` does for the other boards).

## 4. Which device tree (b)

**None is passed, and none is needed.** A bare `go` gives the image `argc` and
`argv` (section 2), not a device tree, and our Zephyr image carries its own
compiled-in devicetree (`driver/boards/st/stm32mp257f_dk`). The DK's console
(USART2), the GIC-400 and the 40 MHz counter are described in that board
file, not discovered (pre-study, Q6, Q13). U-Boot's own control DTB
(`fdt_blob`, relocated with it) is U-Boot's, and `fdt_addr_r` (0x90000000) is
where the distro loop would put Linux's: the image neither reads nor
overwrites either. `booti ... <fdt>` would hand over a DTB in `x0`
(`ST/arch/arm/lib/bootm.c:316`, `:337`), but it also relocates the image
(section 3) and so is the wrong command for a fixed link address. The device
trees ST's Linux uses are files on bootfs (pre-study, section 1) and are never
loaded by the one-shot. `TODO(board-run)`: the handover record's `x1` points
into U-Boot's memory and must not be dereferenced as an FDT; the image does
not.

## 5. The two routes (d)

Both are **typed one-shots**: a runner stops U-Boot's autoboot on the console
and types **one line** that loads a staged script and sources it
(`tools/mp2_uboot_console.py typed`; `driver/tools/uboot-mp2/README.md`, "The
route"). Nothing is written to the environment, and nothing the distro loop
could find is left on the card. The script then loads the image to 0xa0000000,
checks it (size, the ARM64 magic at byte 56, the header's `image_size`), and
does `dcache flush; dcache off; icache off; go ${mp2z_load}`. A refusal prints
`mp2z: refused: <why>` and `exit`s; the typed line then goes on to `echo
mp2z: back to bootcmd; run bootcmd` and ST's stock boot runs.

### Route 1: the microSD one-shot (U-Boot's own boot medium)

Typed line (the runner substitutes `<dev>`, `<part>`, `<token>`; the `mp2z_*`
variables it sets are in RAM):

```
load mmc <dev>:<part> ${scriptaddr} /yakogl/<token>.scr && source ${scriptaddr}; echo mp2z: back to bootcmd; run bootcmd
```

The image and `<token>.scr` (built by `driver/tools/uboot-mp2/build.sh`,
`mkimage -A arm64 -O linux -T script -C none`) are staged from Linux under
`/boot/yakogl/` on bootfs, which U-Boot reads as `/yakogl/` on that partition,
with `ext4load`/`load` (the partition number is the runner's to read from
Linux, where bootfs is mounted on `/boot`; the device is `${boot_instance}`,
the medium U-Boot booted from). Names are never `extlinux.conf`, `boot.scr` or
`boot.scr.uimg`, and never under an `*_extlinux` directory.

**Why not the distro loop's own files** (the shape asked for in the plan: an
`extlinux` entry or a `boot.scr.uimg` staged under `/boot` and removed again).
That is the approach the other boards' hooks follow, and here it fails twice.
(1) The loop tries extlinux first and a successful extlinux boot does not
return (section 1), so a staged `boot.scr.uimg` is never reached while ST's
extlinux works (the pre-study found the same). The only way to be found first
is to change ST's extlinux files, which this design forbids (section 10).
(2) Whatever the distro loop finds it runs **on every boot**, with nobody
typing: a staged file left behind by a cut-off run would boot Zephyr on the
next reset, and a Zephyr that hangs resets the board in a loop. The typed route
has no such state: if the runner dies before it types, the next boot is ST's
stock Linux with all the files inert. The distro-loop variant is therefore a
fallback design, not built: it would need an `extlinux` label or a
`boot.scr.uimg` that returns to the loop on every path but one, and a person
present to remove it. `board-stm32mp2.sh`'s older hook (the saved environment
edited from the host) is also not this route: it writes the environment
(`saveenv`) and needs a person to put a hook in front of `bootcmd`.

### Route 2: the TFTP one-shot (shaped like `imx8mp-netboot.md`)

```
setenv autoload no; dhcp; tftpboot ${scriptaddr} <server>:<token>.scr && source ${scriptaddr}; echo mp2z: back to bootcmd; run bootcmd
```

`mp2-net.cmd` then does `dhcp` again with `autoload` saved and restored in RAM,
`tftpboot ${mp2z_load} ${mp2z_server}:${mp2z_file}` and the same checks and
`go`. The host side is the one the other boards use:
`tools/tftp_oneshot.py --root DIR` serves each file **once** (renamed
`<name>.served` after a complete transfer, refused after `--max-age` 300 s), so
a reboot nobody asked for finds nothing to fetch;
`nb_tftp_canary` of `tools/uboot-netboot-lib.sh` fetches a canary with `curl`
to prove the server answers before the board is touched. What is **not**
shared with the i.MX8MP and AM62P is the library's hook
(`YAKO_HOOK`, `fw_setenv`, `saveenv`): the DK's Linux has no working
`fw_printenv` and no `fw_setenv` (section 1; `board-stm32mp2.sh` header), and the route needs no arming at all.
The i.MX8MP record's TFTP tuning (`tftptimeout`, `tftpblocksize`,
`tftpwindowsize` in `YAKO_NET`) is not in the scripts: it was for a different
MAC and U-Boot; `TODO(board-run)`: the transfer rate and any timeouts.

ST's `stm32mp25_defconfig` does not set `CONFIG_TFTP_PORT` (the option is
`ST/net/Kconfig:95`, off unless set), so `ST/net/tftp.c` compiles out the
`tftpdstp` lookup (`#ifdef CONFIG_TFTP_PORT`, `tftp.c:938-944`) and always asks
`WELL_KNOWN_PORT`, UDP port 69 (`tftp.c:26`, `:933`): the TFTP server must
listen on port 69, and setting `tftpdstp` changes nothing.

What Linux shows of the network (board read 2026-11-16, `ip -o link show`,
`ip route`, `systemctl is-active`): the board's Ethernet is `end0` (the DWC
QoS MAC) and it has a default route under `systemd-networkd`, so the lab
network has a DHCP server and the cable is in. The stored environment has
`autoload=0` and no `ipaddr`, `serverip` or `ethaddr` variable, so U-Boot
takes all of them from `dhcp` (the board's MAC comes from the device tree
or the fuses, not the environment). What Linux cannot show, and the first run
records, is `TODO(board-run)`: that U-Boot's own link and DHCP lease come up
(`dhcp` at the prompt, then `printenv serverip ipaddr`; read-only; never
recorded with its addresses), and that the TFTP server is reachable from it. The compiled-in defaults include a fixed `serverip`
(`defconfig:78-79`); the scripts do not use it.

## 6. The runner's whole cycle

1. On the host, no board: build the scripts (`build.sh`), check the image
   (header, footprint inside the window, size under 255 MiB, the watchdog
   marker `gc_wdog_arm: armed`). Print the undo of everything the next steps do.
2. Take the board lock, check ssh and the supply (below), the console is free
   (`mp2_uboot_console.py probe`), record the stored environment (`dd` of
   `u-boot-env` decoded by `uboot_env.py`, section 1; `env-before.txt`,
   `tools/board-stm32mp2-uboot.sh:853`, read again under the lock at `:727`)
   and the `sha256` of every file of `/boot` (the before record).
3. Stage `<token>.scr` and the image under `/boot/yakogl/` (route 1) or into
   the TFTP root (route 2); read them back by sha256.
4. Start the capture (`mp2_uboot_console.py capture`/`typed`), reboot the
   board, the typed runner stops autoboot, types the line, waits for the
   scan to say `done` (or one of the other verdicts), see below.
5. When Linux is back: remove the staged files, record the environment (the
   same `dd` + `uboot_env.py` read, `env-after.txt`, `:939`) and the `/boot`
   sha256 again and compare (the after record, `:942-943`).

`mp2_uboot_console.py scan` phases and verdicts (its docstring): `bl2`,
`optee`, `uboot`, `stopped`, `script`, `refused`, `go`, `image`, `image_end`,
`watchdog`, `linux`; verdicts `done`, `refused`, `watchdog_reset`,
`not_stopped`, `no_go`, `not_started`, `hang`. The assumed prompt is
`STM32MP> `, which `defconfig:30` gives; `TODO(board-run)`: the prompt as
printed.

## 7. What the one-shot leaves unchanged (e)

The rule of `board-stm32mp2.sh`: **`bootcmd` is never written persistently**
(by the runner; ST's variable, ST's boot). The one-shot goes further and writes
no persistent environment at all.

- **No `saveenv`, no `env save`, no `env write` or `erase`:** the scripts'
  lint (`driver/tools/uboot-mp2/selftest.sh`) allows only `echo`, `setenv`,
  `test`, `itest`, `setexpr`, `dcache`, `icache`, `go`, `exit` and the route's
  own load commands, so `env` is not a command it accepts at all; it also
  fails on the text `saveenv`, `mmc write`, `ext4write`, `fatwrite`, `gpt `,
  `extlinux` and `boot.scr`, on a `setenv` of anything but `mp2z_*` or
  `autoload`, and on `bootcmd` in any command but `echo`. (The binary has `CONFIG_CMD_EXT4_WRITE`,
  `CONFIG_CMD_ERASEENV`, `defconfig:38`, `:62`, and `saveenv` is default y,
  `ST/cmd/Kconfig:601-603`: the lint is what keeps them out.)
- **Only the runner's own variables, each in RAM, each with its undo printed
  first.** Variables: `mp2z_load`, `mp2z_window`, `mp2z_min`, `mp2z_dev`,
  `mp2z_part`, `mp2z_file`, `mp2z_server`, `mp2z_ok`, `mp2z_size`,
  `mp2z_chk`, `mp2z_imgsz`, `mp2z_autoload`, and in the net route also
  `autoload` (saved in `mp2z_autoload`, set back before every exit and before
  `go`). The first line either script prints is `mp2z: undo: nothing
  persistent; run bootcmd boots ST Linux`: the undo of RAM variables is a
  reset, which the one-shot ends in anyway. The `setenv autoload no` in the
  typed line is in RAM like the script's.
- **`bootcmd` is only *run*,** by the typed line's fallback `run bootcmd`;
  no command names it as a target.
- **The one caveat:** `run bootcmd` runs ST's `env_check` (section 1). If the
  stored environment on the card is not valid (a bad CRC on both copies, so
  U-Boot uses the defaults), `env_check` saves the **whole RAM environment**,
  including the `mp2z_*` variables the one-shot set. That is ST's own save
  path and not a command of ours, but the `mp2z_*` leftovers would then be
  stored. The runner therefore reads the stored environment first and refuses
  to run when either copy's CRC fails or the stored environment is not the one
  it recorded (the existing runner's check, `tools/board-stm32mp2.sh`, "both
  copies' CRCs must check before any write"; here before any run). The
  `imx8mp-netboot.md` run saw the same class of effect (a `saveenv` stores
  U-Boot's runtime variables): the before/after environment diff is what shows
  it. `TODO(board-run)`: the diff is expected empty.

**What "unchanged" means, as the runner measures it.** Two records:

1. The runner reads the partition with `dd` before and after (both copies, at
   0x7e000 and 0x7c000, 0x2000 each), requires both to pass
   `uboot_env.py check` (`--allow-blank-copy`, so the stock all-zero copy 1
   counts as blank-but-accepted; section 1), prints the **active** copy's
   variables, sorts them and `diff`s the two texts
   (`tools/board-stm32mp2-uboot.sh:570-572`, `:942`): empty gives
   `env_diff=empty` in the verdict line (`:794`). The raw images are not kept
   in the run directory (they carry the MAC and the serial); their `sha256`
   go to `meta.txt` (`:574`), and the before and after values must be equal.
2. A text diff does not see the flag bytes or the blank copy; the equal image
   `sha256` does (both copies' bytes, flag bytes and the blank copy 1
   identical). For a person with the two `dd` files,
   `tools/uboot_env.py --size 0x2000 --redundant --offset 0x7e000
   --offset2 0x7c000 --allow-blank-copy diff before.bin after.bin` is the
   comparison (section 8). `env_read/summarize.py` does the same for a
   collector's two directories (its `## fw_printenv diff` heading keeps the
   old name; the variables come from the partition).

"Unchanged" is therefore: both copies' CRCs valid before and after, the same
active copy, equal image `sha256`, and an empty variable diff. The runner also
refuses to start on a bad CRC (`:570-571`), no `bootcmd`, or a `scriptaddr`
unset or near the go window (`:575-582`).

## 8. The undo of a cut-off run (f)

What exists at each phase of the cycle, and who removes it. Everything in
RAM is gone at the next reset; only the card and the TFTP root persist.

| phase | left behind | undone by |
| --- | --- | --- |
| host checks, lock | nothing on the board | the lock's release |
| staged (step 3) | `/boot/yakogl/<token>.scr` and the image on bootfs (route 1); the files in the TFTP root (route 2) | the runner's EXIT trap removes them (also on TERM, INT, HUP); a SIGKILL leaves them; a state file beside the lock records what was left (as `board-stm32mp2.sh`'s `<lock>.mp2-left` does) |
| reboot, BL2, OP-TEE, U-Boot | nothing | the next boot |
| autoboot not stopped (the runner missed the window) | nothing; U-Boot ran on to Linux (verdict `not_stopped`) | the staged files are removed on the return |
| prompt reached, line typed, script running | RAM variables only | a reset |
| a refusal | RAM variables; `run bootcmd` boots Linux | nothing to do |
| `go` taken, the image running | RAM | the image's PSCI reset, or the watchdog |
| the image hung | RAM | the watchdog (section 9) |
| back in Linux | the staged files if not yet removed | the runner, then the sha256 comparison |

**A person's undo** (the runner prints these lines before it changes anything):

- remove `/boot/yakogl/<token>.scr` and `/boot/yakogl/<image>` (and the
  directory when empty), nothing else, and `rm` the files of the TFTP root;
- compare `sha256sum` of every file under `/boot` (the stock ones:
  `Image.gz`, the device trees, `boot.scr.uimg`, `mmc0_extlinux/`,
  `mmc1_extlinux/`, the extlinux files) with the listing recorded **before**
  the run: they are **never written**, so they must match byte for byte, and
  the extra files must be only the `yakogl/` ones. `TODO(board-run)`: the
  before listing of `/boot` and its sha256 file, and the after comparison;
- the exact dump and compare (the layout of section 1): `dd` the
  `u-boot-env` partition to `before.bin` and `after.bin`, then
  `tools/uboot_env.py --size 0x2000 --redundant --offset 0x7e000 --offset2 0x7c000 check before.bin`
  (exit 0 only if both CRCs are valid; add `--allow-blank-copy` on the stock
  card, whose copy 1 is zeros) and the same options with
  `diff before.bin after.bin` (exit 0 when they are identical). The active
  copy is the one with the **higher flag byte** (`ST/env/common.c:365-374`;
  0xff against 0x00 counts the 0x00 one as newer, equal flags take copy 0), so
  the flag bytes of both copies must also match before and after, and `print`
  shows the active one for the variable comparison (what the runner diffs as
  `env-before.txt` and `env-after.txt`, section 7);
- the one-shot writes neither copy, so `sha256sum before.bin after.bin` must
  be equal (the runner records both in `meta.txt`); `fw_printenv` is no part
  of this, it does not run on the DK;
- if the board sits at the `STM32MP> ` prompt and nobody is typing: `run
  bootcmd` or `reset` (neither saves anything), or, if U-Boot's watchdog is not
  fed (section 9), wait for the reset;
- a board that does not come back within the bounds: **check the USB-C supply
  and cable first** (section 10), then power-cycle; never type `saveenv`.

What the old design (the saved-environment hook) left behind and this one
does not: `yako_once` and the `yako_*` variables in the stored environment, an
environment backup, and a hook in `bootcmd` that needs a person to remove.

## 9. The watchdog bound (g)

**The pre-study's reading:** BL2 starts the IWDG, so it is armed and running
when U-Boot starts, OP-TEE's SMC service (`arm,smc-wdt`, `arm,smc-id =
<0xbc000000>`, `ST/arch/arm/dts/stm32mp251.dtsi:78-82`; enabled with
`timeout-sec = <32>` in the DK file, `stm32mp257f-dk.dts:216-219`) serves it,
and a hang resets the board in **about 32 s** (27 s after an OP-TEE panic); a
client calls INIT first, uses timeouts of 6 s or more and never sends ENABLE
with 0 (pre-study, section 1). The board's U-Boot was reported to have no
`wdt` support (board-read, 2026-11-06).

**What ST's source says U-Boot does with it**, which does not match the last
sentence and is why the bound has two cases:

- The defconfig enables `CONFIG_WDT`, `CONFIG_WDT_STM32MP` and
  `CONFIG_WDT_ARM_SMC` (`defconfig:183-185`). `WDT_STM32MP` and `WDT_ARM_SMC`
  both `imply WATCHDOG` (`ST/drivers/watchdog/Kconfig:340-345`, `:388-393`), so
  `CONFIG_WATCHDOG` is on: `make stm32mp25_defconfig` of this tag (ST's
  defconfig run through Kconfig, no U-Boot build) gives `CONFIG_WATCHDOG=y`,
  `CONFIG_CYCLIC=y`, `CONFIG_WATCHDOG_TIMEOUT_MSECS=60000`,
  `# CONFIG_WATCHDOG_AUTOSTART is not set`, `CONFIG_ENV_SIZE=0x2000` and
  `CONFIG_SYS_CBSIZE=1024`. That is the source's configuration; the board's
  binary may differ (`TODO(board-run)`, below). The stored environment's
  `ver=U-Boot 2023.10-stm32mp-r3.1 (Oct 02 2023 - 14:39:59 +0000)` (board read
  2026-11-16, the stored environment) is the tag the citations use.
- At init `initr_watchdog` probes every watchdog and `init_watchdog_dev` starts
  those flagged to autostart (`ST/drivers/watchdog/wdt-uclass.c:70-95`,
  `:96-118`). `WATCHDOG_AUTOSTART` itself defaults to n for STM32MP
  (`Kconfig:13-18`), but a probe can force it: the SMC driver does when the
  watchdog is already counting (`ST/drivers/watchdog/arm_smc_wdt.c:104-109`),
  the IWDG driver when `IWDG_SR.ONF` says so (`stm32mp_wdt.c:129-133`). BL2 has
  started it, so U-Boot's `wdt_start` re-arms it with U-Boot's timeout (the
  device tree's `timeout-sec`, 32 s for the SMC node; Kconfig's
  `WATCHDOG_TIMEOUT_MSECS` is 60 s, `Kconfig:28-37`, when nothing says) and
  prints `WDT:   Started <dev> with servicing every <n>ms (<t>s timeout)`
  (`wdt-uclass.c:120-157`).
- With `CONFIG_WATCHDOG` on, `wdt_start` registers a **cyclic** function that
  pets the watchdog every `reset_period` (1 s by default, `wdt-uclass.c:55-67`,
  `:136-149`, `:270-297`; for the SMC driver `SMCWD_PET`,
  `arm_smc_wdt.c:57-59`). Cyclic functions run from `schedule()`
  (`ST/common/cyclic.c:102-114`), and `schedule()` is called wherever U-Boot
  waits for a character: the prompt's line reader (`ST/common/cli_readline.c:489`),
  `fgetc` (`ST/common/console.c:505`) and the serial driver's `getc` retry
  loop (`ST/drivers/serial/serial-uclass.c:257`). The autoboot countdown itself
  (`autoboot.c:371-406`) polls with `tstc` and `udelay` and does not call
  `schedule()` in its loop.

**So the bound, in the two cases:**

| where the hang is | if U-Boot services the watchdog (the source's reading) | if it does not (the board-read) |
| --- | --- | --- |
| in the image after `go` | **about 32 s** after `go`, less the time since the last U-Boot pet (up to 1 s): U-Boot's cyclic code does not run after the jump, our image pets it (`gc_wdog_arm`, below) or the counter expires | about 32 s from BL2's arming |
| at U-Boot's prompt, nobody typing | **no bound from the hardware**: the prompt's reader pets every second, so a prompt the runner abandoned stays until a person types `reset` or the supply is cut | about 32 s from BL2: a stuck prompt ends by itself (the assumption in `mp2_uboot_console.py`'s docstring) |
| in a long U-Boot command (`tftpboot`, `ext4load`) | depends on whether that code calls `schedule()` | about 32 s |

The consequence for the runner: it must not rely on the watchdog to end a
prompt it left. It sends nothing after a missed echo or a missed prompt
(`mp2_uboot_console.py`: "nothing is typed after a miss"), so the **runner's own bounds** (its `KEY_BOUND`, `MISS_WINDOW`, `NEXT_BOUND`) end the run, and
the verdict says where it stopped; a prompt left behind is a "NEEDS A PERSON"
case, with the undo of section 8. `TODO(board-run)`: whether the IWDG or
the SMC watchdog is counting at the prompt: read an existing console capture
of a boot for the `WDT:   Started` line, then at the prompt `wdt list` (if the
command exists: `help wdt`), then watch whether the prompt resets about 32 s
after it appears when nothing is typed (`TODO(board-run)` for the last, with a
person present: that the board does come back by itself is the property the
design leans on).

**In our image:** `driver/zephyr/lib/board/stm32mp2/stm32mp2_wdog.c` arms
OP-TEE's `arm,smc-wdt` itself (the marker `gc_wdog_arm: armed`, required by
the runner's image check; `MP2_NO_WATCHDOG=1` overrides when BL2's counter is
the only reliance) and the layer resets the board at a lifetime of 20 s
(`CONFIG_YAKOGL_STM32MP2_LIFETIME_S`, `board-stm32mp2.sh` header), well inside
the 32 s. The return trial of a run that hangs on purpose (`mp2_hello` built
with `CONFIG_MP2_HELLO_HANG=y`, `run --expect-watchdog`) is the existing proof
that a hang in the image resets the board; it has the same console phases
(`watchdog_reset`). `TODO(board-run)`: that proof **from U-Boot's `go`**, if
the first run is not the hang image: the boot of an image that hangs is the
only measurement of the bound.

## 10. Board constraints (h)

These are the DK's, from `board-stm32mp2.sh` and `stm32mp257f-dk-bringup.md`,
and bind every step of both routes. The runner enforces them before every
write (the sibling runner's `dev_guard`) and the typed lines contain no
command that could break them:

- **Never write** the card's partitions `fsbla1`, `fsbla2`, `metadata1`,
  `metadata2`, `fip-a`, `fip-b` (TF-A, OP-TEE and U-Boot live there: a bad
  write leaves a board whose ROM finds nothing), nor `vendorfs`, `rootfs`,
  `userfs`, nor `u-boot-env`: the one-shot does not write the environment.
  The only writes are the files under `/boot/yakogl/` on bootfs, from Linux.
- **Never write the eMMC** (the disk whose type reads MMC, `mmcblk1` here; the card is the disk `/boot` is mounted
  from, and it must say it is an SD card). `mmc write`, `ext4write` and
  friends are not in the scripts.
- **No repartitioning** (`gpt`, `part`, `sgdisk`), no filesystem creation, no
  change to ST's files under `/boot`.
- **No kernel module load or unload** (loading etnaviv reportedly panicked
  OP-TEE: pre-study, Q9; galcore is ST's).
- **Never touch Tailscale or sshd.** The board is reached over them; the
  runner's ssh uses the host key as known and never records one.
- **Never write board addresses, host names, UART paths, serials or
  credentials** to a file or a commit: environment only (`BOARD_HOST`,
  `BOARD_UART`; the records are scrubbed). This note has none.
- **A result that looks like a hardware fault is checked against the supply
  first.** The DK's only supply is its USB-C cable: a brown-out reboots or
  stalls the board mid-run and reads like a GPU fault. The discriminator is
  BL2's `Reset reason:` line at the start of the boot the run did not look at
  (`stm32mp257f-dk-bringup.md`, "When it looks like a hardware fault: supply
  first"): `Power-on reset`, `Brownout reset`, or no BL2 line at all.
- **Autoboot must be stopped by the runner only with a key**, never `boot`,
  `saveenv` or a CR outside its own typed lines (`mp2_uboot_console.py`: "It
  sends nothing else").
- **TF-A, OP-TEE and U-Boot stay ST's** (pre-study, "Boot route for
  sign-off"): this note never replaces any of them.

## 11. What the board has to answer (index of the placeholders)

| marker | what to read or record |
| --- | --- |
| done 2026-11-16 | the stored environment's variables: section 1 (distro loop, memory layout), section 3 |
| done 2026-11-16 | the `u-boot-env` partition: both copies' CRC and flag, the size (section 1) |
| `TODO(board-run)` | `bdinfo` at the prompt once: `relocaddr`, `sp start`, `fdt_blob` far above 0xb0000000 (section 3) |
| `TODO(board-run)` | whether the watchdog is serviced at the prompt: the `WDT:   Started` line in a console capture, `wdt list` (section 9) |
| done 2026-11-16 | the listing of `/boot`: names, sizes, `sha256sum`, free space; the partition number of bootfs (section 1) |
| `TODO(board-run)` | link and DHCP in U-Boot, the TFTP server's reachability (section 5) |
| `TODO(board-run)` | the `mp2-handover:` line: `el`, `sctlr`, `m`, `c`, `i`, `x0`..`x3` (sections 2, 4) |
| `TODO(board-run)` | the prompt text as printed; the TFTP rate; the verdict (sections 5, 6) |
| `TODO(board-run)` | the environment diff before and after (`dd` + `uboot_env.py`), expected empty, the two image `sha256` equal (section 7) |
| `TODO(board-run)` | the `/boot` sha256 comparison, expected identical but for `yakogl/` (section 8) |
| `TODO(board-run)` | the watchdog's behaviour from U-Boot's `go` (section 9) |

### Contradictions the board read found

Board read 2026-11-16 against the design above:

1. **`boot_prefixes=/mmc0_`**, not `/ /boot/`: the loop never reaches the
   stock `/boot.scr.uimg`; the one-shot is typed, so the design does not
   depend on it.
2. **Linux's `fw_printenv` is unusable as shipped** (no `/etc/fw_env.config`;
   the `.mmc` sample lists the same offset twice). A runner or the `env_read`
   probe that calls `fw_printenv` on this card fails; they read the
   partition with `dd` and decode it with `tools/uboot_env.py` (done:
   `board-stm32mp2-uboot.sh:533-583`, `env_read/collect.sh:60-64`).
3. **Copy 1 (0x7c000) is zeros**, so a plain `tools/uboot_env.py check`
   refuses the image. The runner and the collector pass `--allow-blank-copy`
   (`board-stm32mp2-uboot.sh:548`), which accepts exactly an all-zero copy and
   reads copy 0; the c5 comparison is copy 0's variables plus the `dd`
   image's `sha256`, and copy 1 stays "blank before and after" unless a run
   saves the environment (then it is no longer blank and the `sha256`
   differs).
4. The stored environment carries `fileaddr=90100000`, `filesize=108d`: values
   a one-shot's own `load` changes in RAM only; the stored ones must come out
   identical after the run.
5. Nothing contradicts the 0xa0000000..0xb0000000 window: `scriptaddr`,
   `kernel_addr_r`, `fdt_addr_r`, `ramdisk_addr_r` are the compiled-in
   defaults, all below it.

## Board run

*(Empty: filled in by the `board-run` item, in the shape of
`imx8mp-netboot.md`'s "Board run" sections: the date and the build, what was
idle before, one numbered step per action with what the runner printed, and
the board's state after. No board address, host name, UART path, serial or
credential.)*

What goes here:

1. **Image and routes:** the image run (`mp2_hello`, or `gc_triangle` once
   plan #349's triangle has merged), its sha256 and size; the route (microSD or
   TFTP); the build of `driver/tools/uboot-mp2` scripts (the commit).
2. **The verdict line**, verbatim, as `tools/board.sh stm32mp257f-dk` or the
   runner prints it (`... rc=<rc> verdict=<v> boot=uboot-sd|uboot-tftp ...`),
   and the scan's phases (`bl2 optee uboot stopped script go image image_end
   linux`).
3. **The `fw_printenv` diff before and after** the run (expected: empty), and
   the before and after `u-boot-env` dumps' sha256 when taken.
4. **The `/boot` sha256 comparison** before and after (expected: identical for
   every stock file; only `yakogl/` differs, and is gone again after the
   cleanup).
5. **What U-Boot handed over:** the `mp2-handover:` line, the console excerpt
   from `Hit any key` to the image's banner, `bdinfo` and the live
   `scriptaddr`/`boot_targets` from the env read.
6. **The watchdog:** whether `WDT:   Started` appeared, the time from `go` to
   the end (the image's own end or the reset).
7. **Failures:** the BL2 `Reset reason:` of each boot, the supply check, and
   the undo taken. A new defect gets a new issue text, not a line here.
