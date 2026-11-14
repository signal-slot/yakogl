# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2-standalone.cmd - U-Boot (ST v2023.10) script of the STM32MP257F-DK
# standalone default (docs/notes/stm32mp257f-dk-standalone.md, sections 2, 3
# and 7). Built by build.sh into boot.scr.uimg; staged as
# /yakogl/standalone/boot.scr.uimg on bootfs, where ST's distro loop sources
# it through the extra boot_prefixes entry the runner's "default" put in front.
#
# It counts a default of N boots (mp2s_left 1-9) or "forever", saves the
# count before anything is loaded, and when the count is used up restores
# boot_prefixes from mp2s_prefixes and deletes mp2s_*. Every exit returns to
# the distro loop, whose next prefix is ST's untouched extlinux (Linux).
# Its variables are mp2s_*; the only other one it sets is boot_prefixes, and
# only from mp2s_prefixes. selftest.sh holds the lint.

# 1. The undo comes first; nothing is read or loaded before it.
echo "mp2s: undo: setenv mp2s_left 0; saveenv; boot   (or a PC: delete yakogl/standalone/boot.scr.uimg)"

# 2. No default set: back to the loop.
if test -z "${mp2s_left}"; then
	echo "mp2s: no default set, back to the distro loop"
	exit
fi

# 3. The contract version the runner wrote.
if test "${mp2s_ver}" != 1; then
	echo "mp2s: refused: contract version ${mp2s_ver}, expected 1, booting Linux"
	exit
fi

# 4.-6. The count. setexpr reads hex and test reads decimal; for one digit
# they agree, so only 1-9 counts; anything else (0, not a number, out of
# range) is "used up". Nothing below sets a variable before the save except
# the count itself, so a save stores only the count.
if test "${mp2s_left}" = forever; then
	echo "mp2s: default /yakogl/standalone/default.bin, forever"
elif test "${mp2s_left}" -gt 0 -a "${mp2s_left}" -lt 10; then
	setexpr mp2s_left ${mp2s_left} - 1
	if saveenv; then
		echo "mp2s: default /yakogl/standalone/default.bin, boots left ${mp2s_left}"
	else
		echo "mp2s: saveenv failed, booting Linux"
		exit
	fi
else
	# An empty boot_prefixes would leave the loop nothing to boot, so the
	# prefix is only put back from a value the runner recorded.
	if test -z "${mp2s_prefixes}"; then
		echo "mp2s: refused: mp2s_prefixes is not set, boot_prefixes left as it is, booting Linux"
		exit
	fi
	echo "mp2s: default used up, booting Linux"
	setenv boot_prefixes "${mp2s_prefixes}"
	setenv mp2s_left
	setenv mp2s_prefixes
	setenv mp2s_ver
	saveenv
	exit
fi

# 7. Size, load and check the image. These variables live in RAM only: no
# save follows them.
# The link address and the window: the same as the load address and window
# of uboot-mp2/mp2-mmc.cmd (zephyr,sram of the DK board); selftest.sh
# compares them.
setenv mp2s_load 0xa0000000
setenv mp2s_window 0x10000000
# 0x40 is the arm64 Image header.
setenv mp2s_min 0x40
# The partition the loop found this script on.
if test -z "${devtype}" -o -z "${devnum}" -o -z "${distro_bootpart}"; then
	echo "mp2s: refused: devtype, devnum or distro_bootpart is not set, booting Linux"
	exit
fi

# Size first (size sets filesize and reads no data into RAM).
setenv mp2s_ok 0
if size ${devtype} ${devnum}:${distro_bootpart} /yakogl/standalone/default.bin; then setenv mp2s_ok 1; fi
if test ${mp2s_ok} -eq 0; then
	echo "mp2s: refused: /yakogl/standalone/default.bin not found on ${devtype} ${devnum}:${distro_bootpart}, booting Linux"
	exit
fi
# ${filesize} is hex without 0x, so the comparisons use itest, which reads
# plain operands as hex.
if itest ${filesize} -lt ${mp2s_min}; then
	echo "mp2s: refused: file ${filesize} bytes, below the ${mp2s_min}-byte Image header, booting Linux"
	exit
fi
if itest ${filesize} -gt ${mp2s_window}; then
	echo "mp2s: refused: file ${filesize} bytes, above the ${mp2s_window}-byte window, booting Linux"
	exit
fi
setenv mp2s_size ${filesize}

# Load, then check what arrived (a successful load sets filesize again).
setenv mp2s_ok 0
if load ${devtype} ${devnum}:${distro_bootpart} ${mp2s_load} /yakogl/standalone/default.bin; then setenv mp2s_ok 1; fi
if test ${mp2s_ok} -eq 0; then
	echo "mp2s: refused: load failed, booting Linux"
	exit
fi
if test "${filesize}" != "${mp2s_size}"; then
	echo "mp2s: refused: loaded ${filesize} bytes, expected ${mp2s_size}, booting Linux"
	exit
fi
# The Image header magic "ARM\x64" at byte 56 (0x644d5241 little-endian).
setexpr mp2s_chk ${mp2s_load} + 0x38
if itest.l *${mp2s_chk} != 0x644d5241; then
	echo "mp2s: refused: no ARM64 Image magic at byte 56 of the loaded file, booting Linux"
	exit
fi
# The header's image_size (64 bits at byte 16: the RAM the image takes from
# its start, .bss included) must fit the window too.
setexpr mp2s_chk ${mp2s_load} + 0x14
if itest.l *${mp2s_chk} != 0; then
	echo "mp2s: refused: the header image_size is 4 GiB or more, booting Linux"
	exit
fi
setexpr mp2s_chk ${mp2s_load} + 0x10
setexpr.l mp2s_imgsz *${mp2s_chk}
if itest.l *${mp2s_chk} -gt ${mp2s_window}; then
	echo "mp2s: refused: the image takes 0x${mp2s_imgsz} bytes of RAM (header image_size), above the ${mp2s_window}-byte window, booting Linux"
	exit
fi
echo "mp2s: loaded ${filesize} bytes at ${mp2s_load}, RAM footprint 0x${mp2s_imgsz} of the ${mp2s_window}-byte window"

# go, not booti: booti would move the image off its link address. The image
# runs with the MMU and caches off, so flush and turn them off here.
echo "mp2s: go ${mp2s_load}"
dcache flush; dcache off; icache off; go ${mp2s_load}
echo "mp2s: the image returned, booting Linux"
exit
