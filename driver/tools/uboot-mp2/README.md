<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# uboot-mp2: the STM32MP257F-DK's typed U-Boot one-shot

Two U-Boot scripts that load a Zephyr image of the DK to its link address and
`go` to it, run from ST's U-Boot (2023.10, after TF-A v2.10 BL2/BL31 and OP-TEE
4.0; `docs/notes/pre-study-stm32mp257f-dk.md`, section 1). Shaped like
`driver/tools/uboot-t6` (not edited). Nothing here has run on a board; the
console helper (`tools/mp2_uboot_console.py`), the board backends
(`tools/board.sh`) and the design note
(`docs/notes/stm32mp257f-dk-uboot-boot.md`) are other items.

| file | job |
| --- | --- |
| `mp2-mmc.cmd` | `ext4size` + `ext4load` of `${mp2z_file}` from `mmc ${mp2z_dev}:${mp2z_part}` |
| `mp2-net.cmd` | `setenv autoload no; dhcp`, then `tftpboot` of `${mp2z_file}` from `${mp2z_server}` |
| `build.sh [OUTDIR]` | builds `mp2-mmc.scr` and `mp2-net.scr` |
| `selftest.sh` | host check, no board, no network |

## The route: typed, nothing persistent

A runner stops U-Boot's autoboot on the console and types **one line**. It
loads a staged script, sources it, and falls back to the stock boot. Nothing is
written to the environment: no `saveenv`, no `env save`, no `setenv` of a
variable other than `mp2z_*` (the net script also sets `autoload` and puts it
back before every exit and before `go`). The scripts never name `bootcmd`
in a command (only in the undo echo). `bootcmd` is not set, changed or written
by the scripts or by the typed line, which only runs it as the fallback. A hang is reset by BL2's IWDG (about 32 s); the next autoboot reaches
ST's extlinux. The undo, printed first by each script:
`mp2z: undo: nothing persistent; run bootcmd boots ST Linux`.

Typed lines (the runner substitutes `<dev>`, `<part>`, `<token>`; it may prefix
`setenv mp2z_part <part>;` and `setenv mp2z_file /yakogl/<image>;`, all
`mp2z_*`):

```
load mmc <dev>:<part> ${scriptaddr} /yakogl/<token>.scr && source ${scriptaddr}; echo mp2z: back to bootcmd; run bootcmd
setenv autoload no; dhcp; tftpboot ${scriptaddr} <server>:<token>.scr && source ${scriptaddr}; echo mp2z: back to bootcmd; run bootcmd
```

The second is the net equivalent; the exact text is the console helper's. Its
`setenv autoload no` is the typed line's, in RAM like the script's own. After
a refusal the script has returned (`exit`), so the line goes on to `echo` and
`run bootcmd`, and the stock boot runs. After `go` nothing returns.

Staged files: under `/boot/yakogl/` on bootfs, `/yakogl/` inside the
partition: `<token>.scr` and the image. Never named `extlinux.conf`,
`boot.scr` or `boot.scr.uimg`, never under an `*_extlinux` directory: the
distro loop must never find them by itself. `build.sh` writes
`mp2-mmc.scr`; the runner stages it under its token name.

Contract (shared with the sibling items):

- Variables: only `mp2z_*`: `mp2z_load` = 0xa0000000, `mp2z_window` =
  0x10000000 (both fixed in the script), `mp2z_dev` (default `boot_instance`,
  the medium ST's U-Boot booted from), `mp2z_part` (no default: the script
  refuses without it, the bootfs partition number is the runner's to know),
  `mp2z_file`, `mp2z_server`. The scripts also use `mp2z_min`, `mp2z_ok`,
  `mp2z_size`, `mp2z_chk`, `mp2z_imgsz`, `mp2z_autoload`.
- Every `echo` starts with `mp2z: `. A refusal prints `mp2z: refused: <why>`
  and `exit`s. The last line before the jump is `mp2z: go ${mp2z_load}`,
  followed by `dcache flush; dcache off; icache off; go ${mp2z_load}`.
- The checks, as in uboot-t6: size (at least 0x40, at most the window), the size
  read back equals the size measured, `ARM\x64` at byte 56
  (`itest.l *addr != 0x644d5241`), the header's `image_size` (byte 16, 64 bit)
  below 4 GiB and not above the window.

## Build and test

```sh
driver/tools/uboot-mp2/build.sh /path/to/out       # mkimage if present, else tools/uboot_script.py
driver/tools/uboot-mp2/selftest.sh                  # 54 checks with mkimage (52 without), host only
```

`mkimage -A arm64 -O linux -T script -C none -n <name> -d <cmd> <scr>`; without
`mkimage`, `tools/uboot_script.py build` makes the same bytes (needs no
u-boot-tools). With `SOURCE_DATE_EPOCH` set both produce identical bytes; the
selftest compares them when `mkimage` is on `PATH` (or named by
`MKIMAGE=/path/to/mkimage`). Otherwise the summary counts the skip
(`..., 1 skipped (mkimage not on PATH)`, exit 0), and `MP2_REQUIRE_MKIMAGE=1`
turns the skip into a failure. The selftest also builds both scripts, reads them back with
`uboot_script.py check`/`extract`, lints them (allowed commands only; no
`saveenv`, `env save|write|erase`, `mmc write`, `ext4write`, `fatwrite`, `gpt`,
`setenv` outside `mp2z_*`/`autoload`, no `bootcmd` in a command except the undo echo; the undo
first, `mp2z: go` last), compares `mp2z_load`/`mp2z_window` with
`driver/boards/st/stm32mp257f_dk`'s `zephyr,sram` and `tools/board-stm32mp2.sh`'s
`YAKO_ADDR`/`LOAD_END`, and shows that each of those checks fails on a mutated
script.

## What the scripts cannot check

- That nothing U-Boot keeps lies in the window. ST's `scriptaddr` is
  0x90100000 (`include/configs/stm32mp25_common.h:105`, `:113`), clear of it;
  U-Boot relocates itself to the top of RAM. `bdinfo` at the prompt (`relocaddr`,
  `sp start`, `fdt_blob`) is the check, once.
- A TFTP file larger than the window: its size is known only after
  `tftpboot` has written it, so the bytes beyond 0xb0000000 are already
  overwritten when the script refuses (the mmc script sizes first). Image sizes
  are megabytes; the window is 256 MiB.
- That `mmc ${mp2z_dev}:${mp2z_part}` is the bootfs; a wrong partition is
  "not found" and refused.
- Entry state: U-Boot enters at EL2 (pre-study, section 1), with the GIC and
  the clocks as it left them; `go` passes `argc`/`argv`, not a device tree.
  Whether the image's early code survives it is the board run's question.
- The watchdog marker (`gc_wdog_arm`) of the image: not checked here; the
  runner's image check (`tools/board-stm32mp2.sh`) owns it.

## go, not booti

Checked against ST's U-Boot, tag `v2023.10-stm32mp-r3.1`
(github.com/STMicroelectronics/u-boot; OpenSTLinux 6.6), file:line:

- `booti` (`cmd/booti.c:104` do_booti) calls `booti_setup` (`:77`), which
  moves the image unless bit 3 of the header's flags is set and
  `force_reloc` is false (`arch/arm/lib/image.c:70`, else the first DRAM
  bank's start, `:73`; the target is `ALIGN(dst, SZ_2M) + text_offset`, `:75`)
  and `memmove`s it there (`cmd/booti.c:85`). Zephyr's header has text_offset 0
  and flags bit 3 clear, so the image would leave its link address, where it
  alone runs. A zero `image_size` is read as 16 MiB (`image.c:53-55`).
- `booti` then passes the device tree in x0 through `armv8_switch_to_el2`
  (`arch/arm/lib/bootm.c:316`, `:337`) after `cleanup_before_linux`
  (`:82`, `:428`); these images take no device tree.
- `go` (`cmd/boot.c:24` do_go, `:41` the call, weak `do_go_exec` `:17-22`,
  `U_BOOT_CMD` `:50`) calls the address in place with `argc`/`argv`: no
  relocation, no `cleanup_before_linux`. So the script flushes and turns the
  caches off itself: `dcache flush`/`off` and `icache off` are
  `cmd/cache.c:62-80` (`flush_dcache_all` for `flush`), and `dcache_disable`
  is `arch/arm/cpu/armv8/cache_v8.c:579` (it returns early when SCTLR.C is
  already clear). The image's `reset.S` expects the MMU and caches off.
- Commands the scripts rely on: `ext4size` (`cmd/ext4.c:74`), `ext4load`
  (`cmd/ext4.c:87`), `tftpboot` (`cmd/net.c:54`/`:62`), `source`
  (`cmd/source.c:71`), `exit` (`cmd/exit.c:22`), `itest` with the `.l` suffix
  (`cmd/itest.c:191`, via `cmd_get_data_size`, `common/command.c:469`),
  `setexpr` with a suffix (`cmd/setexpr.c:389`). `filesize` is set as hex
  without `0x` (`fs/fs.c:745`, `:824`; `net/net.c:721`), so the size checks use
  `itest`, not `test` (which reads base 0).

U-Boot is GPL: these are facts and file:line only.

## Assumptions the board run confirms

- The `exit` in a sourced script returns to the typed line (as in uboot-t6's
  design); if ST's hush runs the rest of the line differently, the refusal
  still prints and the stock boot follows from `run bootcmd`.
- An unset `autoload` is "yes"; the net script saves `${autoload}` in
  `mp2z_autoload` and sets it back (an empty value deletes the variable).
