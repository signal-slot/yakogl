# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2-mmc.cmd - U-Boot script of the STM32MP257F-DK one-shot, mmc route
# (README.md in this directory). Built by build.sh; run by the typed line the
# console runner sends, never by the distro loop (its name is not boot.scr).
#
# It writes nothing: no saveenv, no mmc/ext4/fat write. Its variables are
# mp2z_*, in RAM. selftest.sh holds the list of allowed commands.

# The undo comes first; nothing is read or run before it.
echo "mp2z: undo: nothing persistent; run bootcmd boots ST Linux"

# The runner may set mp2z_dev, mp2z_part and mp2z_file before the script runs;
# the load address and the window are fixed here.
# The link address and the window: driver/boards/st/stm32mp257f_dk
# zephyr,sram (256 MiB at 0xa0000000) and tools/board-stm32mp2.sh's
# LOAD_END; selftest.sh compares all three.
setenv mp2z_load 0xa0000000
setenv mp2z_window 0x10000000
# 0x40 is the arm64 Image header.
setenv mp2z_min 0x40
if test -z "${mp2z_file}"; then setenv mp2z_file /yakogl/mp2z.bin; fi
if test -z "${mp2z_dev}"; then setenv mp2z_dev ${boot_instance}; fi
if test -z "${mp2z_dev}"; then
	echo "mp2z: refused: mp2z_dev is not set and boot_instance is empty"
	exit
fi
if test -z "${mp2z_part}"; then
	echo "mp2z: refused: mp2z_part (the bootfs partition number) is not set"
	exit
fi
echo "mp2z: mmc ${mp2z_dev}:${mp2z_part} ${mp2z_file} -> ${mp2z_load}"

# Size first (ext4size sets filesize and reads no data into RAM).
setenv mp2z_ok 0
if ext4size mmc ${mp2z_dev}:${mp2z_part} ${mp2z_file}; then setenv mp2z_ok 1; fi
if test ${mp2z_ok} -eq 0; then
	echo "mp2z: refused: ${mp2z_file} not found on mmc ${mp2z_dev}:${mp2z_part}"
	exit
fi
# ${filesize} is hex without 0x (env_set_hex, fs/fs.c:745, :824), so the
# comparisons use itest, which reads plain operands as hex.
if itest ${filesize} -lt ${mp2z_min}; then
	echo "mp2z: refused: file ${filesize} bytes, below the ${mp2z_min}-byte Image header"
	exit
fi
if itest ${filesize} -gt ${mp2z_window}; then
	echo "mp2z: refused: file ${filesize} bytes, above the ${mp2z_window}-byte window"
	exit
fi
setenv mp2z_size ${filesize}

# Load, then check what arrived (a successful ext4load sets filesize again).
setenv mp2z_ok 0
if ext4load mmc ${mp2z_dev}:${mp2z_part} ${mp2z_load} ${mp2z_file}; then setenv mp2z_ok 1; fi
if test ${mp2z_ok} -eq 0; then
	echo "mp2z: refused: ext4load failed"
	exit
fi
if test "${filesize}" != "${mp2z_size}"; then
	echo "mp2z: refused: loaded ${filesize} bytes, expected ${mp2z_size}"
	exit
fi
# The Image header magic "ARM\x64" at byte 56 (0x644d5241 little-endian).
setexpr mp2z_chk ${mp2z_load} + 0x38
if itest.l *${mp2z_chk} != 0x644d5241; then
	echo "mp2z: refused: no ARM64 Image magic at byte 56 of the loaded file"
	exit
fi
# The header's image_size (64 bits at byte 16: the RAM the image takes from
# its start, .bss included) must fit the window too; the file is only its
# first part.
setexpr mp2z_chk ${mp2z_load} + 0x14
if itest.l *${mp2z_chk} != 0; then
	echo "mp2z: refused: the header image_size is 4 GiB or more"
	exit
fi
setexpr mp2z_chk ${mp2z_load} + 0x10
setexpr.l mp2z_imgsz *${mp2z_chk}
if itest.l *${mp2z_chk} -gt ${mp2z_window}; then
	echo "mp2z: refused: the image takes 0x${mp2z_imgsz} bytes of RAM (header image_size), above the ${mp2z_window}-byte window"
	exit
fi
echo "mp2z: loaded ${filesize} bytes at ${mp2z_load}, RAM footprint 0x${mp2z_imgsz} of the ${mp2z_window}-byte window"

# go, not booti (README.md): booti would move the image off its link address.
# The image runs with the MMU and caches off, so flush and turn them off here.
echo "mp2z: go ${mp2z_load}"
dcache flush; dcache off; icache off; go ${mp2z_load}
echo "mp2z: the image returned"
