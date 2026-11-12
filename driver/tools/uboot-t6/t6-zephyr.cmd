# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# t6-zephyr.cmd - DRAFT. U-Boot script source: ext4load a NanoPC-T6 Zephyr
# image to its link address and `go` to it. Never run on a board; the first
# run belongs to the owner at the U-Boot prompt (README.md in this directory).
#
# Compile (mkimage from U-Boot's tools; the compiled file is not committed):
#   mkimage -A arm64 -O linux -T script -C none -n t6-zephyr -d t6-zephyr.cmd t6-zephyr.scr
# Run at the U-Boot prompt:
#   load mmc 0:1 ${scriptaddr} /boot/yakogl/t6-zephyr.scr; source ${scriptaddr}
#
# It writes nothing: no environment save, no mmc write, no ext4write, no fatwrite. Only
# its own t6z_* variables are set, in RAM, and a reset forgets them. Set
# t6z_dev, t6z_file or t6z_keepcache before `source` to change the defaults.

# Where the image is: the eMMC's ext4 partition (Linux: mmcblk0p1, /boot).
# Check the U-Boot numbering first: `mmc list; part list mmc 0`.
if test -z "${t6z_dev}"; then setenv t6z_dev 0:1; fi
if test -z "${t6z_file}"; then setenv t6z_file /boot/yakogl/t6-zephyr.bin; fi
# The image's link address and entry, and the window's size: nanopc_t6.dts
# zephyr,sram, 256 MiB at 0x10000000 (nanopc-t6.sh WINDOW_BASE/WINDOW_SIZE
# hold the same; driver/tools/kexec-zephyr/board_lock_selftest.sh compares
# all three). The Image header's first word is a branch to the start, so
# the entry is the link address itself.
setenv t6z_load 0x10000000
setenv t6z_entry 0x10000000
setenv t6z_window 0x10000000
# 0x40 is the arm64 Image header.
setenv t6z_min 0x40

# The undo, printed before anything is read or run.
echo "t6-zephyr: nothing is written to any disk or to the saved environment."
echo "t6-zephyr: UNDO: run 'reset' or use the power switch (the image lives in RAM only);"
echo "t6-zephyr:       before the 'go' ctrl-c leaves the script;"
echo "t6-zephyr:       the stock boot is untouched: 'bootflow scan -lb' boots Armbian."
echo "t6-zephyr: the watchdog marker is not checked here: use only an image nanopc-t6.sh accepted in its dry run."
echo "t6-zephyr: file ${t6z_dev} ${t6z_file} -> ${t6z_load}, entry ${t6z_entry}"

# Size first (ext4size sets ${filesize} and reads no data to RAM).
setenv t6z_ok 0
if ext4size mmc ${t6z_dev} ${t6z_file}; then setenv t6z_ok 1; fi
if test ${t6z_ok} -eq 0; then
	echo "t6-zephyr: STOP: ${t6z_file} not found on mmc ${t6z_dev}"
	exit
fi
# ${filesize} is hex without 0x (env_set_hex, fs/fs.c:787, :865). `test` reads
# operands in base 0, so as decimal (cmd/test.c:152-173); `itest` reads plain
# operands as hex (cmd/itest.c, hextoul), so the size checks use itest.
if itest ${filesize} -lt ${t6z_min}; then
	echo "t6-zephyr: STOP: file ${filesize} bytes, below the ${t6z_min}-byte Image header"
	exit
fi
if itest ${filesize} -gt ${t6z_window}; then
	echo "t6-zephyr: STOP: file ${filesize} bytes, above the ${t6z_window}-byte window"
	exit
fi
setenv t6z_size ${filesize}

# Load, then check what arrived.
setenv filesize
setenv t6z_ok 0
if ext4load mmc ${t6z_dev} ${t6z_load} ${t6z_file}; then setenv t6z_ok 1; fi
if test ${t6z_ok} -eq 0; then
	echo "t6-zephyr: STOP: ext4load failed"
	exit
fi
if test "${filesize}" != "${t6z_size}"; then
	echo "t6-zephyr: STOP: loaded ${filesize} bytes, expected ${t6z_size}"
	exit
fi
# The Image header magic "ARM\x64" at byte 56 (0x644d5241 little-endian).
setexpr t6z_magic ${t6z_load} + 0x38
if itest.l *${t6z_magic} != 0x644d5241; then
	echo "t6-zephyr: STOP: no ARM64 Image magic at byte 56 of the loaded file"
	exit
fi
# The header's image_size (64 bits at byte 16; Zephyr's _flash_used: the RAM
# the image takes from its start, .bss and .noinit included) must fit the
# window as well: the file is only the first part of it. The image zeroes
# its .bss itself; the rest of the window is as U-Boot left it.
setexpr t6z_hdr ${t6z_load} + 0x14
if itest.l *${t6z_hdr} != 0; then
	echo "t6-zephyr: STOP: the header's image_size is 4 GiB or more"
	exit
fi
setexpr t6z_hdr ${t6z_load} + 0x10
setexpr.l t6z_imgsz *${t6z_hdr}
if itest.l *${t6z_hdr} -gt ${t6z_window}; then
	echo "t6-zephyr: STOP: the image takes 0x${t6z_imgsz} bytes of RAM (header image_size), above the ${t6z_window}-byte window"
	exit
fi
echo "t6-zephyr: loaded ${filesize} bytes at ${t6z_load}, RAM footprint 0x${t6z_imgsz} bytes of the ${t6z_window}-byte window"

# `go`, not `booti`: booti would move the image to RAM base + text_offset (the
# header has text_offset 0 and flags bit 3 clear) and a Zephyr image runs only
# at its link address; booti also wants a device tree for x0. `go` runs the
# entry in place. The image expects MMU and data cache off (kexec enters that
# way, nanopc-t6.sh), U-Boot runs with both on: `dcache off` flushes the cache
# and clears SCTLR.C and .M. Skip it with t6z_keepcache=1 if the console
# faults with the MMU off (README.md, "First run").
if test -z "${t6z_keepcache}"; then
	icache off
	dcache off
fi
echo "t6-zephyr: go ${t6z_entry}"
go ${t6z_entry}
echo "t6-zephyr: the image returned"
