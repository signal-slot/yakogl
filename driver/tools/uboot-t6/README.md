<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# uboot-t6: the NanoPC-T6 at its U-Boot prompt

Roadmap stage 5 asks for a standalone boot from the T6's own storage: U-Boot
loads the Zephyr image from the eMMC's ext4 partition and starts it, no Linux
before it. This directory holds the tools for that route.

Status:

- The reads at the prompt are a tool (`prompt-reads.sh`) and ran on the board
  on 2026-11-12. What the prompt answered is in "What the prompt answered".
- Starting an image from the prompt has not run. The script draft
  `t6-zephyr.cmd` (never run) cannot run as written on this U-Boot build: it
  uses `setexpr`, and `icache` and `dcache` unless `t6z_keepcache` is set,
  and the build has none of them. The route that
  replaces it is in "The route"; the work is issue #4627.

## Prompt reads

`prompt-reads.sh` reboots the board into U-Boot, stops autoboot on the debug
UART, enters a fixed list of read commands (`prompt-reads.py plan` prints
it), and leaves with `reset`; the board then boots Armbian by itself.

```sh
# dry run: prints the plan and the board's state, changes nothing
BOARD_HOST=root@<t6> driver/tools/uboot-t6/prompt-reads.sh
# the run
BOARD_HOST=root@<t6> T6_HAND=<who> T6_RECORD_DIR=<dir> T6_UART=<tty> \
  driver/tools/uboot-t6/prompt-reads.sh --go
```

What it guarantees:

- Every command is a read. The list has no `mw`, no `bootm`, `booti` or `go`,
  no `mmc write`, `ext4write` or `saveenv`, and no `regulator` command.
- It types only after U-Boot's own autoboot line, and only at U-Boot's
  prompt. Armbian logs root in on this UART by itself, so a byte typed after
  the kernel starts would reach a root shell: the tool stops typing when
  U-Boot leaves the prompt (exit 7).
- It takes the board lock (`../kexec-zephyr/board_lock.sh`) and refuses, never
  waits, when another runner holds it.
- The QoS words are read only if `PWR_GATE` shows PD_GPU on, and `pmic dump`
  only if `dm tree` shows the PMIC already probed.
- The self-test (`prompt-reads.py --self-test`, a fake board on a pty, no
  hardware) runs before the board is touched and in the gate
  (`../kexec-zephyr/board_lock_selftest.sh`).

The record is `record.txt`, `uart.log`, `typed.log` and `answers.txt` under
`$T6_RECORD_DIR/<date>-t6-uboot-reads`. `printenv` prints the board's MAC
addresses and serial number, and `mmc part` a partition UUID: `answers.txt`
and `uart.log` are never copied into the tree, an issue or a pull request as
they are.

Exit codes: 0 done and Armbian is back; 1 the board did not come back; 2
usage; 3 refused before anything changed, or no autoboot line; 4 autoboot
missed; 5 stopped at the prompt; 6 `reset` entered, no Linux login; 7 U-Boot
left the prompt and started Linux.

The run on 2026-11-12 (record `20261112-184729-t6-uboot-reads`): 27 commands
answered, Linux's login 19 s after `reset`, exit 0.

## What the prompt answered

U-Boot 2026.07_armbian, `bootdelay=1`, `bootcmd=bootflow scan -lb`, console
on the debug UART at 1500000 baud.

| Question | Answer |
|---|---|
| Which mmc device is the eMMC? | `mmc 0` (`mmc@fe2e0000`; `mmc 1` is `mmc@fe2c0000`). `mmc 0:1` is the ext4 root, with `/boot` on it. |
| Is the 256 MiB window free? | Yes. `bdinfo`: U-Boot's code, stack, heap and device tree are at 0xecbcd000 and above (`relocaddr` 0xefec1000, `fdt_blob` 0xedbd07e0); nothing is reserved in 0x10000000-0x1fffffff. |
| Load addresses | `kernel_addr_r=0x02000000`, `fdt_addr_r=0x12000000`, `scriptaddr=0x00c00000`, `loadaddr=0xc00800`. Only `fdt_addr_r` lies in the window; the route does not use it. |
| PD_GPU | Off, its idle request clear: `PWR_GATE` 0x0000fff9, `BUS_IDLE_REQ` 0, `BUS_IDLE_ACK` and `BUS_IDLE_ST` 0x00021fff, `REPAIR_STATUS` 0xfff70001. |
| The GPU clock | `CLKSEL_CON(158)` 0x00001f80: the SPLL, not the CPLL / 5 Linux leaves (0x1f24). `CLKSEL_CON(159)` 0, `CLKGATE_CON(66)` 0x00000004. The CPLL is at 1500 MHz in normal mode (`MODE_CON0` 0x00000545), so `gpu_clock_set` moves the GPU to CPLL / 5. |
| The RK806 | `dm tree` shows it probed, with all its regulators, before any PMIC command: U-Boot probes it on every boot. Whether DCDC1 is on was not read at the prompt (no `pmic` command); the image's hand-over report reads it over SPI2. |
| A watchdog | None: no watchdog device in `dm tree`, no `wdt` command. |
| Commands the build lacks | `setexpr`, `dcache`, `icache`, `clk`, `wdt`, `pmic`, `ext4write`, `hash`, `sha1sum`. |
| Commands the route needs | Present: `ext4size`, `ext4load`, `md`, `crc32`, `go`, `reset`. |

The read list keeps `clk dump`, `wdt list` and the `pmic` commands: "Unknown
command" is the measurement that this build lacks them, and another build may
have them.

## The route

1. The image goes on the eMMC's ext4 root as `/boot/yakogl/<image>.bin`
   ("Staging under /boot and the undo"). Nothing of the stock `/boot` is
   changed, and U-Boot's environment is never saved.
2. The runner stops autoboot and enters the commands on the UART, one at a
   time, checking each answer: `ext4size mmc 0:1 /boot/yakogl/<image>.bin`
   (the size must equal the host's), `ext4load mmc 0:1 0x10000000 ...`,
   `md.l 0x10000038 1` (the `ARM\x64` magic at byte 56), `crc32 0x10000000
   <size>` (must equal the host's), then `go 0x10000000`.
3. The image prints its record on the same UART and resets at its end.
   `bootflow scan -lb` then boots Armbian: no one-shot variable is needed.

No script file, no `mkimage`, no `source`: the checks are the runner's, on
the host, where they can be tested without a board.

Link address and window: `nanopc_t6.dts`'s `zephyr,sram`, 256 MiB at
0x10000000 (0x10000000-0x1fffffff); `driver/tools/kexec-zephyr/nanopc-t6.sh`'s
WINDOW_BASE and WINDOW_SIZE and the draft's `t6z_load`, `t6z_entry` and
`t6z_window` hold the same values, and
`driver/tools/kexec-zephyr/board_lock_selftest.sh` fails when one differs.

What the image must do itself, because this U-Boot cannot (issue #4627):

- **MMU and caches.** U-Boot runs with the MMU, the data cache and the
  instruction cache on (common/board_r.c:110-113; arch/arm/cpu/armv8/
  cache_v8.c:817, :880-894, :1172-1175), and `go` calls the entry as a plain
  function, with no cleanup (cmd/boot.c:18-22, :41). The other boards' route
  runs `dcache off; icache off` before `go` (`tools/uboot-netboot-lib.sh`);
  this build has neither command. The image's `reset.S` clears only SCTLR.A
  (Zephyr `arch/arm64/core/reset.S:63-65, 77-79`). So the T6 image's entry
  has to flush its own range and turn the MMU and caches off before anything
  else. Under kexec the entry already finds them off.
- **The watchdog.** U-Boot runs none. The image arms WDT0 itself, as it does
  after kexec; that the watchdog's clock is open under U-Boot is not
  measured.
- **The GPU.** PD_GPU off and the GPU clock on the SPLL are both states
  `gpu_up` handles (docs/notes/nanopc-t6-uboot-handover.md, sections 1 and
  2); neither has run on the board from U-Boot's state.

## go, not booti

The image does carry an arm64 Image header (`ARM\x64` at byte 56: Zephyr's
`arch/arm64/core/header.S`, text_offset 0, flags `HEADER_PGSIZE << 1`, so
bit 3 clear), yet `go` is the right command (U-Boot v2026.07, commit
ece349ad):

- `booti` (cmd/booti.c:112 do_booti, :23 booti_start) calls `booti_setup`
  (cmd/booti.c:77). That moves the image when bit 3 of the flags is clear:
  `dst` is the first DRAM bank's start (arch/arm/lib/image.c:69-72) and the
  target is `ALIGN(dst, SZ_2M) + text_offset` (arch/arm/lib/image.c:74), then
  `memmove` there (cmd/booti.c:83-85). A Zephyr image runs only at its link
  address, so it would be moved off it. A zero `image_size` is read as 16 MiB
  (arch/arm/lib/image.c:52-55); the Zephyr header carries `_flash_used`.
- `booti` also looks for ramdisk and device tree (cmd/booti.c:105-106) and
  passes the device tree in x0 (arch/arm/lib/bootm.c:290-291, via
  `armv8_switch_to_el2`); these images take none.
- `go` (cmd/boot.c:24-48 do_go, :41 do_go_exec; weak default at :18-22)
  calls the address in place with `argc`, `argv`, no relocation, and without
  `cleanup_before_linux`/`bootm_final` (compare arch/arm/lib/bootm.c:257-270,
  where booti's path does them).
- `ext4load` is `U_BOOT_CMD(ext4load ...)` at cmd/ext4.c:86 (do_ext4_load
  :48); the load sets `filesize` (fs/fs.c:865), `ext4size` sets it too
  (fs/fs.c:787).

U-Boot is GPL: the above are facts and file:line only.

## Staging under /boot and the undo

From Armbian over ssh as root. Nothing below touches a stock file; the stock
boot files are recorded first and compared after. The eMMC's loader area is
never written.

```sh
# 0. fingerprint of the stock /boot files (kept off the board's disk)
find /boot -maxdepth 1 -type f -exec sha256sum {} + | sort -k2 > /tmp/boot-before.sha256
# 1. stage: one new directory, one file
install -d -m 0755 /boot/yakogl
install -m 0644 <image>.bin /boot/yakogl/<image>.bin
sync
# 2. check the copy against the host's
sha256sum /boot/yakogl/*
```

Undo:

```sh
rm -r /boot/yakogl && sync
find /boot -maxdepth 1 -type f -exec sha256sum {} + | sort -k2 | diff - /tmp/boot-before.sha256 && echo "stock /boot restored"
```

The image is only in RAM after `ext4load`: `reset` or the power switch
returns to the stock `bootflow scan -lb`, which boots Armbian. A directory
entry `/boot/yakogl` is the only change on the partition, and `rm -r`
removes it; the rest of `/boot` is byte for byte as before (the `diff` above
says so).

## The script draft, t6-zephyr.cmd

Never run. It sizes the file, loads it, checks size, magic and the header's
`image_size`, turns the caches off and runs `go`, as one U-Boot script,
loaded with `load mmc 0:1 ${scriptaddr} ...` and run with `source`. It cannot
run as written on this build: the magic and header-size checks use
`setexpr` (t6-zephyr.cmd:73, :82, :87-88), and it runs `icache off` and
`dcache off` before `go` unless `t6z_keepcache` is set (:103-104); the build
has none of the three ("What the prompt answered"). The other commands it
uses the build has: `load`, `source`, `test`, `itest`, `setenv`, `echo`,
`exit`, `ext4size`, `ext4load` and `go` (`help` at the prompt, 2026-11-12).
The route above does the same checks from the host. The draft stays until the
runner of issue #4627 replaces it, because `board_lock_selftest.sh` compares
its window with the device tree's.

The reads the draft's first run was to begin with (`mmc list`, the partition
of `0:1`, `scriptaddr`, `bdinfo`) were made by `prompt-reads.sh` on
2026-11-12: `mmc 0:1` is the eMMC's ext4 root, `scriptaddr` is 0x00c00000,
and `relocaddr`, `sp start` and `fdt_blob` lie above 0xecbcd000, outside
0x10000000-0x1fffffff. Still unproven on the board, for any route:

- whether the image's early code survives `go`'s EL2 state (HCR_EL2, ICC) and
  the registers x0..x3: host-tested separately;
- the GPU's clock, PD_GPU and vdd_gpu_s0 from U-Boot's state: the Z1 power-on
  was written for it (handover note).

### Compiled with v2026.07 mkimage

The script compiles clean. The host had no `mkimage` installed, so the tools
came from the v2026.07 tag (commit `ece349ad`, the commit U-Boot
2026.07_armbian names) built host-tools-only:

```sh
make O=<build-dir> tools-only_defconfig
# then, for a host without gnutls dev headers: drop CONFIG_TOOLS_MKEFICAPSULE
# (scripts/config --disable TOOLS_MKEFICAPSULE; make olddefconfig)
make O=<build-dir> tools-only
```

The build is clean; `mkimage -V` says `mkimage version 2026.07`. The compile
of the draft:

```sh
mkimage -A arm64 -O linux -T script -C none -n t6-zephyr -d t6-zephyr.cmd t6-zephyr.scr
```

```
Image Name:   t6-zephyr
Created:      Thu Nov 12 22:25:20 2026
Image Type:   AArch64 Linux Script (uncompressed)
Data Size:    5020 Bytes = 4.90 KiB = 0.00 MiB
Load Address: 00000000
Entry Point:  00000000
Contents:
   Image 0: 5012 Bytes = 4.89 KiB = 0.00 MiB
```

`dumpimage -l` on the `.scr` prints the same lines. The `.cmd` is 5012
bytes; the `.scr` is 5084: the 64-byte legacy image header, then the script
type's size table — the file's 4-byte big-endian size, then a 4-byte 0
end-of-list — then the script byte for byte (its last 5012 bytes compare
equal to the draft; tools/mkimage.c:688-724). Load address and entry point
are 0 in the header — a script image's address is `source`'s argument
(`${scriptaddr}`), and the image's own load address is the script's
`t6z_load`.

The commands and variables the script uses stand checked against the
v2026.07 source: `exit` (cmd/exit.c:22), `itest` incl. `-lt`/`-gt` and
`*addr` (cmd/itest.c:36-50, :59-84; the `.b/.w/.l/.q` name suffix parses in
cmd_get_data_size, common/command.c:469), `setexpr` with the same suffix
(cmd/setexpr.c:410), `ext4size`/`ext4load` (cmd/ext4.c:73, :86),
`icache`/`dcache` (cmd/cache.c), `go` (cmd/boot.c:50), and
`scriptaddr=0x00c00000` for the RK3588 (include/configs/rk3588_common.h:22).
The `${filesize}` comparisons use `itest`, not `test`: `filesize` is hex
without `0x` and `test` reads base 0 (cmd/test.c:152-173). Compiling with
`mkimage -T script` does not read the script body — it stores it — so the
compile and these citations together stand for the hush check.

What the compile does not show: whether the board's build has these
commands. It lacks `setexpr`, `icache` and `dcache` ("What the prompt
answered").
