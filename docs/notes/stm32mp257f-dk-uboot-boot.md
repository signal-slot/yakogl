<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK: boot Zephyr from ST's U-Boot (design, 2026-11-13)

Status: **design, written on the host from ST's source. Nothing here has run on
the board.** Every fact that only the board can give is marked
`TODO(board-env-read)` (a read-only look at the board) or `TODO(board-run)`
(something the first run records), with one line saying what to read. The empty
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
| the runner of this note's routes | `tools/board-stm32mp2-uboot.sh` | another item (plan #357, `mp2-backend`) |
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
  (`tools/mp2_uboot_console.py`, `STOP_KEY`, `KEY_PERIOD`). No
  `bootdelaykey` is set in the stock environment's source
  (`autoboot.c:240` reads it; `TODO(board-env-read)`: the live `bootdelay`,
  `bootdelaykey`, `stopkey` from `fw_printenv`).
- **Console line length.** `CONFIG_SYS_CBSIZE` has no ST override
  (`grep` of `ST/configs/stm32mp25_defconfig` and `ST/include/configs/stm32mp25*.h`
  finds none), so it is Kconfig's default **1024** (`ST/cmd/Kconfig:78-84`,
  the `default 1024` line). The console buffer is `CONFIG_SYS_CBSIZE + 1`
  (`ST/common/cli_readline.c:24`) and the reader stops taking characters at
  `CBSIZE - 2` (`:534`). The typed lines of the one-shot are about 120 bytes
  (`driver/tools/uboot-mp2/README.md`, "Typed lines"), an eighth of the limit.
- **The environment** is on the medium U-Boot booted from, in the partition
  named `u-boot-env`: `CONFIG_ENV_IS_IN_MMC=y` (`defconfig:69`),
  `CONFIG_SYS_REDUNDAND_ENVIRONMENT=y` (`:72`), `CONFIG_SYS_MMC_ENV_DEV=-1`
  (`:77`, so the device is the boot instance,
  `ST/board/st/stm32mp2/stm32mp2.c:604-613`), the DK's device tree names the
  partition (`u-boot,mmc-env-partition = "u-boot-env"`,
  `ST/arch/arm/dts/stm32mp257f-dk-u-boot.dtsi:11`). `env/mmc.c` puts the
  copy at the **top of the partition**: offset `start + size - (1 + copy) *
  ENV_SIZE` (`ST/env/mmc.c:91-92`, `:117-130`), with `ENV_SIZE` 0x2000
  (`ST/arch/arm/mach-stm32mp/Kconfig:33-34`). In the 512 KiB partition that is
  **0x7e000 for copy 0 and 0x7c000 for the redundant copy 1**, and 0x7e000 is
  where the board read found the CRC, a flag byte and the strings (pre-study,
  section 1). The layout of a copy is `crc32` (4 bytes, over the data), `flags`
  (1 byte, the redundancy flag) and the data (`ST/include/env_internal.h:59-64`,
  `:80-86`). `TODO(board-env-read)`: both copies' CRCs and flag bytes, and the
  partition size, read with `dd` from Linux (read-only); the runner's
  `tools/board-stm32mp2.sh` already assumes these offsets and refuses when a
  CRC fails.
- **`bootcmd` and `env_check`.** The compiled-in `bootcmd_stm32mp` prints
  "Boot over ...", runs `env_check`, sets `boot_targets` to `mmc${boot_instance}`
  when it booted from mmc, and runs `distro_bootcmd`
  (`ST/include/configs/stm32mp25_common.h:75-87`; the ST-board variant,
  `ST/include/configs/stm32mp25_st_common.h:26-40`, is the same for mmc).
  `env_check` is `if env info -p -d -q; then env save; fi`
  (`stm32mp25_common.h:94`). `env info -p -d -q` succeeds only when the
  environment in use is the **default** one (`gd->flags & GD_FLG_ENV_DEFAULT`,
  `ST/cmd/nvedit.c:1116`) **and** can be persisted (`:1128`; the results are
  ANDed, `:1146`). So a valid stored environment is never rewritten by the
  stock boot; an invalid one (bad CRC on both copies) is replaced by the
  defaults and saved. This is the one way the stock path writes the
  environment, and it matters for the undo (section 8).
- **The distro loop.** `boot_targets` defaults to the order mmc1, ubifs0, mmc0,
  mmc2, usb0, pxe (`stm32mp25_common.h:60-66`); on an mmc boot it is narrowed to
  the boot instance (above). For each device and bootable partition,
  `scan_dev_for_boot` runs, for each prefix in `boot_prefixes` (`/ /boot/`,
  `ST/include/config_distro_bootcmd.h:494`), **first** `scan_dev_for_extlinux`
  (`${prefix}extlinux/extlinux.conf`, `:499-510`), **then**
  `scan_dev_for_scripts` (`boot.scr.uimg`, then `boot.scr`, `:495`, `:518-530`),
  `:532-536`. A successful extlinux boot does not return, so a script is reached
  only after extlinux has failed. The pre-study found `mmc0_extlinux/` and
  `mmc1_extlinux/` on the board's bootfs, not the source's `extlinux/`:
  `TODO(board-env-read)`: the live `boot_syslinux_conf`, `boot_prefixes`,
  `boot_scripts`, and the listing of `/boot` (names, sizes, sha256), which the
  runner needs for the undo.
- **Memory layout variables** (`stm32mp25_common.h:101-118`): `kernel_addr_r`
  0x8a000000, `fdt_addr_r` 0x90000000, **`scriptaddr` 0x90100000**,
  `pxefile_addr_r` 0x90200000, `fdtoverlay_addr_r` 0x90300000,
  `ramdisk_addr_r` 0x90400000, `kernel_comp_addr_r` 0x84000000 (size
  0x4000000). `CONFIG_SYS_LOAD_ADDR=0x84000000` (`defconfig:15`), the stack
  before relocation `CONFIG_CUSTOM_SYS_INIT_SP_ADDR=0x90000000` (`:5`).
  `TODO(board-env-read)`: `fw_printenv scriptaddr kernel_addr_r fdt_addr_r
  ramdisk_addr_r boot_targets boot_device boot_instance bootcmd` on the live
  board; the stored environment may carry other values than the compiled-in
  defaults, which is why the scripts never hard-code `scriptaddr` (section 3).
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
  `:127-130` when it is at EL2). The pre-study (section 1, Q8, Q13) records
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
  (`ST/arch/arm/mach-stm32mp/stm32mp2/cpu.c:36-55`, `arch_cpu_init`) and
  rebuilds its page tables after relocation (`:73-96`, `enable_caches`).
  `go` does nothing about them, and `cleanup_before_linux` is only on the
  `booti`/`bootm` path (`ST/arch/arm/lib/bootm.c:82`, `:428`). So the script
  does it itself, as the sibling boards' one-shots do: `dcache flush; dcache
  off; icache off; go` (`ST/cmd/cache.c:27-80`; `dcache_disable` clears
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
| the image | **0xa0000000**, `go 0xa0000000` | the link address: `zephyr,sram` of `driver/boards/st/stm32mp257f_dk` (a 256 MiB `memory@a0000000`), `YAKO_ADDR` of `tools/board-stm32mp2.sh`, `mp2z_load` of the scripts. The image runs only there, so it is loaded in place, never `booti`'d (which moves it: `ST/cmd/booti.c:77-104`, `ST/arch/arm/lib/image.c:53-75`) |
| the script | **`${scriptaddr}`**, read from the live environment | the compiled-in value is 0x90100000 (`stm32mp25_common.h:105`, `:113`), but the stored environment is the board's; the typed line uses `${scriptaddr}` and nothing hard-codes it. `TODO(board-env-read)`: the live `scriptaddr` |
| the window | 0xa0000000 .. 0xb0000000 | `mp2z_window` = 0x10000000; `LOAD_END` in the runner |
| the size guard | the file and the header's `image_size` stay below **255 MiB** | see below |

**Why the window is safe.** Each fact is from ST's tree unless it says so:

- DDR is 4 GiB at 0x80000000 on the DK
  (`ST/arch/arm/dts/stm32mp257f-dk.dts:142-144`; `STM32_DDR_BASE` and
  `STM32_DDR_SIZE`, `ST/arch/arm/mach-stm32mp/include/mach/stm32.h:181-183`).
- U-Boot relocates **to the top of RAM**, clamped below 4 GiB:
  `gd->ram_top = clamp_val(ram_base + ram_size, 0, SZ_4G - 1)`, then a block of
  `ALIGN(8 MiB + CONFIG_SYS_MALLOC_LEN + mon_len, 2 MiB)` is taken from the top
  not covered by a reserved region (`ST/arch/arm/mach-stm32mp/dram_init.c:52-85`,
  `ST/common/board_f.c:378-380`; `SYS_MALLOC_LEN` is 0x2000000,
  `ST/arch/arm/mach-stm32mp/Kconfig:30-31`). The DK's reserved DDR regions
  (`ST/arch/arm/dts/stm32mp257f-dk-resmem.dtsi:112-181`) are 0x80000000 to
  0x84000000 (several small ones and the 32 MiB at 0x82000000), the 64 MiB
  `gpu-reserved` at 0xfa800000 and two at 0xfe800000 and 0xff000000. So U-Boot,
  its heap and its stack sit at or above roughly 0xf0000000, hundreds of MiB
  above the window; the copy of U-Boot's own relocation, the `fdt_blob`, the
  `bd` and the malloc area are in that same block (`bdinfo` prints
  `relocaddr`, `sp start` and `fdt_blob`, `ST/cmd/bdinfo.c:153-158`,
  `ST/arch/arm/lib/bdinfo.c:47`). `TODO(board-env-read)`: `bdinfo` at the
  prompt, once, to confirm `relocaddr` is far above 0xb0000000. The source
  reading says so; this is a source reading, not a measurement.
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
(`TODO(board-env-read)`, above). The 255 MiB guard is one MiB tighter than the
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
(`YAKO_HOOK`, `fw_setenv`, `saveenv`): the DK's Linux has `fw_printenv` and no
`fw_setenv` (`board-stm32mp2.sh` header), and the route needs no arming at all.
The i.MX8MP record's TFTP tuning (`tftptimeout`, `tftpblocksize`,
`tftpwindowsize` in `YAKO_NET`) is not in the scripts: it was for a different
MAC and U-Boot; `TODO(board-run)`: the transfer rate and any timeouts.

`TODO(board-env-read)`: that the DK's U-Boot has a link and a DHCP lease on
the lab network (`dhcp` at the prompt, then `printenv serverip ipaddr`;
read-only; never recorded with its addresses), and that the TFTP server is
reachable from it. The compiled-in defaults include a fixed `serverip`
(`defconfig:78-79`); the scripts do not use it.

## 6. The runner's whole cycle

1. On the host, no board: build the scripts (`build.sh`), check the image
   (header, footprint inside the window, size under 255 MiB, the watchdog
   marker `gc_wdog_arm: armed`). Print the undo of everything the next steps do.
2. Take the board lock, check ssh and the supply (below), the console is free
   (`mp2_uboot_console.py probe`), record `fw_printenv` and the `sha256` of
   every file of `/boot` (the before record).
3. Stage `<token>.scr` and the image under `/boot/yakogl/` (route 1) or into
   the TFTP root (route 2); read them back by sha256.
4. Start the capture (`mp2_uboot_console.py capture`/`typed`), reboot the
   board, the typed runner stops autoboot, types the line, waits for the
   scan to say `done` (or one of the other verdicts), see below.
5. When Linux is back: remove the staged files, record `fw_printenv` and the
   `/boot` sha256 again and compare (the after record).

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

- **No `saveenv`, no `env save`, no `env set -f` or `erase`:** the scripts'
  lint (`driver/tools/uboot-mp2/selftest.sh`) fails on `saveenv`, `env
  save|write|erase`, `mmc write`, `ext4write`, `fatwrite`, `gpt`, and on any
  `bootcmd` inside a command. (The binary has `CONFIG_CMD_EXT4_WRITE`,
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
  U-Boot's runtime variables): the before/after `fw_printenv` diff is what shows
  it. `TODO(board-run)`: the diff is expected empty.

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
- compare `fw_printenv` with the before copy; `u-boot-env`'s two copies are
  not written by the one-shot, so the dump of the partition (`dd` of the
  512 KiB, read-only) can be compared too;
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
  binary may differ (`TODO(board-env-read)`, below).
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
  (`ST/common/cyclic.c:103-114`), and `schedule()` is called wherever U-Boot
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
case, with the undo of section 8. `TODO(board-env-read)`: whether the IWDG or
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
- **Never write the eMMC** (`mmcblk0`; the card is the disk `/boot` is mounted
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
| `TODO(board-env-read)` | `fw_printenv`: `scriptaddr`, `kernel_addr_r`, `fdt_addr_r`, `ramdisk_addr_r`, `boot_targets`, `boot_device`, `boot_instance`, `boot_prefixes`, `boot_scripts`, `boot_syslinux_conf`, `bootdelay`, `bootdelaykey`, `bootcmd` (section 1) |
| `TODO(board-env-read)` | the `u-boot-env` partition, read-only `dd`: both copies' CRC, the flag bytes, the partition size (section 1) |
| `TODO(board-env-read)` | `bdinfo` at the prompt once: `relocaddr`, `sp start`, `fdt_blob` far above 0xb0000000 (section 3) |
| `TODO(board-env-read)` | whether the watchdog is serviced at the prompt: the `WDT:   Started` line in a console capture, `wdt list` (section 9) |
| `TODO(board-env-read)` | the listing of `/boot`: names, sizes, `sha256sum`, free space; the partition number of bootfs (sections 1, 8) |
| `TODO(board-env-read)` | link and DHCP in U-Boot, the TFTP server's reachability (section 5) |
| `TODO(board-run)` | the `mp2-handover:` line: `el`, `sctlr`, `m`, `c`, `i`, `x0`..`x3` (sections 2, 4) |
| `TODO(board-run)` | the prompt text as printed; the TFTP rate; the verdict (sections 5, 6) |
| `TODO(board-run)` | the `fw_printenv` diff before and after, expected empty (section 7) |
| `TODO(board-run)` | the `/boot` sha256 comparison, expected identical but for `yakogl/` (section 8) |
| `TODO(board-run)` | the watchdog's behaviour from U-Boot's `go` (section 9) |

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
