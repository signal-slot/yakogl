<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# uboot-t6: the ext4load route of a NanoPC-T6 Zephyr image (draft)

Status: draft, never run on a board. Roadmap stage 5 asks for a standalone
boot from the T6's own storage; this is the first step of the route:
U-Boot loads the image from the eMMC's ext4 partition and starts it, no Linux
before it. Everything the owner session needs is here; the first run at the
prompt needs the owner with someone at the power switch.

## The route

1. U-Boot 2026.07_armbian is on the eMMC. Its environment is the compiled-in
   one and is never saved (`bootcmd` is `bootflow scan -lb`, `boot_targets`
   starts `mmc1 mmc0`), measured read-only under Armbian by the t6-reads
   session.
2. The only ext4 partition is the eMMC's first (Linux `mmcblk0p1`, 10 GB
   free), which holds `/boot`. The image goes there as
   `/boot/yakogl/t6-zephyr.bin`, beside the script's compiled form
   `/boot/yakogl/t6-zephyr.scr`. Nothing of the stock `/boot` is changed.
3. At the U-Boot prompt: `load mmc 0:1 ${scriptaddr} /boot/yakogl/t6-zephyr.scr`
   then `source ${scriptaddr}`. The script (`t6-zephyr.cmd`) prints the undo,
   sizes the file (`ext4size`), refuses outside 0x40..0x10000000 bytes, loads it
   to 0x10000000 (`ext4load`), checks the size read back, the `ARM\x64`
   magic at byte 56 and that the header's `image_size` (the RAM the image
   takes, `.bss` included) fits the window, flushes and turns the caches
   off, and runs `go 0x10000000`.
   No `saveenv`, no write of any partition; its variables are all `t6z_*`.

Link address and window: `nanopc_t6.dts`'s `zephyr,sram`, 256 MiB at
0x10000000 (0x10000000-0x1fffffff); the script's `t6z_load`, `t6z_entry` and
`t6z_window` and `driver/tools/kexec-zephyr/nanopc-t6.sh`'s WINDOW_BASE and
WINDOW_SIZE hold the same values, and
`driver/tools/kexec-zephyr/board_lock_selftest.sh` fails when one differs.
The entry is the start of the image. What the script cannot check: that
nothing U-Boot keeps (its relocated code, stack, heap and device tree) lies
in the window. Under Linux, `nanopc-t6.sh` reads that from `/proc/iomem`;
here `bdinfo` shows it (below). The mmc numbering in the script
(`0:1`) is a default, not a measurement: the owner checks `mmc list` and
`part list mmc 0` first and sets `t6z_dev` if it differs.

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
  where booti's path does them). Hence the script runs `icache off` and
  `dcache off` itself (cmd/cache.c:24 and :53; arm64 `dcache_disable` flushes
  and clears SCTLR.C and SCTLR.M, arch/arm/cpu/armv8/cache_v8.c:897-919):
  `driver/tools/kexec-zephyr/nanopc-t6.sh` enters the image with the MMU and
  caches off, and the image's `reset.S` clears only SCTLR.A at EL1 and EL2
  (Zephyr `arch/arm64/core/reset.S:63-65, 77-79`), not M or C.
- `ext4load` is `U_BOOT_CMD(ext4load ...)` at cmd/ext4.c:86 (do_ext4_load
  :50); the load sets `filesize` (fs/fs.c:865), `ext4size` sets it too
  (fs/fs.c:787).

Not part of this draft: entering at EL2 with U-Boot's HCR_EL2 and GIC state,
x0..x3 = argc/argv not 0, and where U-Boot leaves PD_GPU; see
`docs/notes/nanopc-t6-uboot-handover.md`. U-Boot is GPL: the above are facts
and file:line only.

## Staging under /boot and the undo (the later owner session)

From Armbian over ssh as root, with the owner present. Nothing below touches
a stock file; the stock boot files are recorded first and compared after.

```sh
# 0. fingerprint of the stock /boot files (kept off the board's disk)
find /boot -maxdepth 1 -type f -exec sha256sum {} + | sort -k2 > /tmp/boot-before.sha256
# 1. stage: one new directory, two files
install -d -m 0755 /boot/yakogl
install -m 0644 t6-zephyr.bin /boot/yakogl/t6-zephyr.bin
install -m 0644 t6-zephyr.scr /boot/yakogl/t6-zephyr.scr
sync
# 2. check the copies against the host's
sha256sum /boot/yakogl/*
```

The `.scr` is built on the host and is not committed:
`mkimage -A arm64 -O linux -T script -C none -n t6-zephyr -d t6-zephyr.cmd t6-zephyr.scr`.

Undo:

```sh
rm -r /boot/yakogl && sync
find /boot -maxdepth 1 -type f -exec sha256sum {} + | sort -k2 | diff - /tmp/boot-before.sha256 && echo "stock /boot restored"
```

The script's own undo (printed at its start): `reset` or the power switch;
the image is only in RAM, and the stock `bootflow scan -lb` boots Armbian.
There is nothing to restore in U-Boot's environment: it is never saved.
A directory entry `/boot/yakogl` is the only change on the partition, and
`rm -r` removes it; the rest of `/boot` is byte for byte as before (the
`diff` above says so).

## What still needs the owner

- The first run at the U-Boot prompt, with someone at the power switch:
  `mmc list`, `part list mmc 0` (is `0:1` the eMMC's ext4?), `printenv
  scriptaddr`, `bdinfo` (`relocaddr`, `sp start`, `fdt_blob`:
  each must be outside 0x10000000-0x1fffffff, and `scriptaddr` too), then
  the two commands of step 3. Reboot into U-Boot interrupts
  autoboot at the prompt; no other command is needed.
- "First run": if the console faults after `dcache off` (the MMU is off and
  U-Boot's own code may issue unaligned access), set `t6z_keepcache 1` before
  `source`; the image then starts with the MMU and caches on, which its
  `reset.S` does not expect, so this is a diagnostic only.
- Whether the image's early code survives `go`'s EL2 state (HCR_EL2, ICC) and
  the registers x0..x3: host-tested separately, unproven on the board.
- The GPU's clock, PD_GPU and vdd_gpu_s0 from U-Boot's state: the Z1 power-on
  was written for it (handover note), and is also unproven there.

## Not compiled here

`mkimage` is not installed on the host. One was built from the v2026.07
tree (tools-only, mkimage alone) but this session could not run it, so the
script's compilation with `mkimage -T script` is unchecked: the owner's host
runs the command above before staging.
