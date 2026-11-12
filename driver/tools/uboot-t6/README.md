<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# uboot-t6: the NanoPC-T6 at its U-Boot prompt

Roadmap stage 5 asks for a standalone boot from the T6's own storage: U-Boot
loads the Zephyr image from the eMMC's ext4 partition and starts it, no Linux
before it. This directory holds the tools for that route.

Status:

- The reads at the prompt are a tool (`prompt-reads.sh`) and ran on the board
  on 2026-11-12. What the prompt answered is in "What the prompt answered".
  The list now also reads WDT0's gates, its words and the CRU's global reset
  words ("The watchdog"); that part has not run.
- Starting an image from the prompt is a tool (`stage.sh`, `prompt-go.sh`;
  "The route") and has not run on the board (issue #4627). The script draft
  `t6-zephyr.cmd` is gone: it needed `setexpr`, `icache` and `dcache`, which
  this build lacks.

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
   (`stage.sh`, "Staging under /boot and the undo"). Nothing of the stock
   `/boot` is changed, and U-Boot's environment is never saved.
2. `prompt-go.sh` reboots the board, stops autoboot and enters the commands
   on the UART (`prompt-go.py`), one at a time, checking each answer before
   the next: `ext4size mmc 0:1 /boot/yakogl/<image>.bin` (no answer), `printenv
   filesize` (the host's size), `ext4load mmc 0:1 0x10000000 ...` (that many
   bytes read), `md.l 0x10000038 1` (the `ARM\x64` magic at byte 56), `crc32
   0x10000000 <size>` (the host's CRC-32), then `go 0x10000000`. After `go`
   nothing is typed again. Any other answer: `reset`, no `go`.
3. The image turns off the MMU and caches `go` leaves on (`T6:G`, "What the
   image does itself"), arms WDT0, prints its record on the same UART and
   resets at its end. `bootflow scan -lb` then boots Armbian: no one-shot
   variable is needed. `prompt-go.py` judges the record with
   `t6-verdict.sh <probe>` (`T6_ROUTE=uboot`) and waits for Linux's login.

No script file, no `mkimage`, no `source`: the checks are the runner's, on
the host, where they are tested without a board (`prompt-go.py
--self-test`, `selftest.sh`; both run in
`../kexec-zephyr/board_lock_selftest.sh`, a gate row).

```sh
# once per image: stage it (the fingerprint file stays on this host)
BOARD_HOST=root@<t6> driver/tools/uboot-t6/stage.sh stage <dir>/t6_hello.bin <dir>/t6-boot.fingerprint
# dry run: the image's checks, the staged copy's sha256, the commands; changes nothing
BOARD_HOST=root@<t6> driver/tools/uboot-t6/prompt-go.sh <dir>/t6_hello.bin t6_hello
# the run
BOARD_HOST=root@<t6> T6_HAND=<who> T6_RECORD_DIR=<dir> T6_UART=<tty> \
  driver/tools/uboot-t6/prompt-go.sh <dir>/t6_hello.bin t6_hello --go
# afterwards: take it away again
BOARD_HOST=root@<t6> driver/tools/uboot-t6/stage.sh undo <dir>/t6-boot.fingerprint
```

`prompt-go.sh` refuses an image without the watchdog marker, linked
elsewhere than 0x10000000, larger than the window, or without the `T6:G`
line (an image built before its entry coped with `go`), and an image whose
staged copy is not the same bytes. Its record is `record.txt`, `uart.log`,
`typed.log`, `answers.txt`, `image.log` (the UART from `go` to Linux's
login) and `verdict.txt` under `$T6_RECORD_DIR/<date>-t6-uboot-go`.

Exit codes (`prompt-go.py`'s, passed on by `prompt-go.sh`): 0 the verdict
PASS and Armbian back; 3 no autoboot line, nothing written; 4 the autoboot
missed, Linux booted; 5 stopped at the U-Boot prompt (an echo that never
matched, no answer, the prompt lost), the board stays there until its power
is cut; 6 an answer differed, `reset` entered, no Linux login; 7 a command
made U-Boot reset the board; 8 an answer differed: no `go`, `reset`
entered, Armbian back; 9 `go` not answered (no "## Starting application");
10 the image started, no Linux login: the board needs a hand; 11 the verdict
FAIL, Armbian back. `prompt-go.sh` adds 1 the board did not come back into
Armbian, 2 usage, 3 refused before anything changed.

From `go` until the image arms WDT0 (`T6:A3`) nothing resets a board whose
image stopped: U-Boot runs no watchdog and Linux's is gone (the kexec route
has Linux's dw_wdt across the jump; this one has none). `T6_HAND` is
required for that reason.

Link address and window:
`driver/boards/friendlyarm/nanopc_t6/nanopc_t6.dts`'s `zephyr,sram`, 256 MiB at
0x10000000 (0x10000000-0x1fffffff). `driver/tools/kexec-zephyr/nanopc-t6.sh`'s
`WINDOW_BASE` and `WINDOW_SIZE`, `prompt-go.sh`'s `WINDOW_BASE` and
`WINDOW_SIZE` and `prompt-go.py`'s `LOAD` hold the same values, and
`driver/tools/kexec-zephyr/board_lock_selftest.sh` fails when one differs.

## What the image does itself

- **MMU and caches.** U-Boot runs with the MMU, the data cache and the
  instruction cache on (common/board_r.c:110-113; arch/arm/cpu/armv8/
  cache_v8.c:817, :880-894, :1172-1175), and `go` calls the entry as a plain
  function, with no cleanup (cmd/boot.c:17-22, :41; the ARM override
  arch/arm/lib/cmd_boot.c is built only for ARMv7-M, arch/arm/lib/
  Makefile:35). Nothing from `ext4load` to the entry cleans the data cache
  (cmd/ext4.c:48-51; fs/fs.c:625-627, :845-865; fs/ext4/ext4fs.c:71-200).
  The file's sectors come by SDMA into DRAM (the T6 defconfig's
  `CONFIG_MMC_SDHCI_SDMA`, :64; drivers/mmc/sdhci.c:104-105, :179-183), but
  the CPU stores a sector read in part (fs/fs_internal.c:51, :69, :91), a
  hole (fs/ext4/ext4fs.c:188) and a block cache hit
  (drivers/block/blk-uclass.c:441-443) through its data cache, so those
  bytes can still be dirty at `go`. The other boards' route runs `dcache
  off; icache off` before `go` (`tools/uboot-netboot-lib.sh`); this build has
  neither command. Zephyr's `reset.S` clears only SCTLR.A (Zephyr
  `arch/arm64/core/reset.S:62-66`). So the image's first code, at Zephyr's
  earliest hook (`z_arm64_el2_plat_prep_c`, before any stack), turns the
  MMU and both caches off, then cleans and invalidates zephyr,sram to the
  point of coherency by address, then invalidates the instruction cache and
  the EL2 TLB, and prints `T6:G` (driver/rust/src/board/nanopc_t6_go.rs).
  Under kexec the entry finds them off and does nothing (Z1 measured
  `SCTLR_EL2` 0x30400800).
- **The watchdog.** U-Boot runs none. The image arms WDT0 itself, as after
  kexec: `yd_dw_wdt_arm` opens WDT0's two gates (CLKGATE_CON(15) bits 0
  and 1) if they are closed, checks the block's identity and refuses (then
  PSCI resets the SoC) when it does not answer. What the words are under
  U-Boot is not measured: the read list now has CLKGATE_CON(15)
  (0xfd7c083c), GLB_RST_ST (0xfd7c0c04) and GLB_RST_CON (0xfd7c0c10), and,
  only when both gates read open, WDT0's CR and TORR and its
  COMP_PARAM_1, COMP_VERSION and COMP_TYPE. GLB_RST_CON decides whether a
  WDT0 timeout resets the SoC from this state; WDT0's soft reset line is not
  read (no source on this host names its CRU bit).
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

`stage.sh`, from this host over ssh as root. The fingerprint file stays on
this host: the sha256 of every file under `/boot` but `/boot/yakogl`, sorted
by name. `stage` writes it before anything changes on the board (or, when it
exists, goes on only if `/boot` still matches it); every command compares
`/boot` with it again after its own change.

```sh
BOARD_HOST=root@<t6> driver/tools/uboot-t6/stage.sh stage <image>.bin <fingerprint>
BOARD_HOST=root@<t6> driver/tools/uboot-t6/stage.sh check <image>.bin <fingerprint>
BOARD_HOST=root@<t6> driver/tools/uboot-t6/stage.sh undo <fingerprint>
```

What it writes on the board, and nothing else: the directory `/boot/yakogl`
(0755), one file per image in it (0644, written to a `.part` file in the
same directory and renamed), `sync`; `undo` is `rm -rf -- /boot/yakogl`. It
never writes another file of `/boot`, a block device, the eMMC's loader area
or U-Boot's environment, and loads no kernel module. Exit: 0 done and
`/boot`'s other files match the fingerprint; 1 the copy or a stock file
differs (named); 2 usage; 3 refused before anything was written.

The image is only in RAM after `ext4load`: `reset` or the power switch
returns to the stock `bootflow scan -lb`, which boots Armbian. A directory
`/boot/yakogl` is the only change on the partition, and `undo` removes it
and says "stock /boot restored" only when every other file matches the
fingerprint.
