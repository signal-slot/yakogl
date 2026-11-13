# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2-net.cmd - U-Boot script of the STM32MP257F-DK one-shot, TFTP route
# (README.md in this directory). Built by build.sh; run by the typed line the
# console runner sends, never by the distro loop (its name is not boot.scr).
#
# It writes nothing: no saveenv, no mmc/ext4/fat write. Its variables are
# mp2z_*, in RAM; the other variables, autoload, tftptimeout and
# tftptimeoutcountmax, are set in RAM and put back before every exit and
# before go.
#
# The server must listen on UDP port 69. ST's stm32mp25_defconfig does not set
# CONFIG_TFTP_PORT, so U-Boot ignores tftpdstp (net/tftp.c:938) and the script
# does not use it; the server address is a bare host, never host:port. selftest.sh holds the list of allowed
# commands.

# The undo comes first; nothing is read or run before it.
echo "mp2z: undo: nothing persistent; run bootcmd boots ST Linux"

# The runner may set mp2z_server and mp2z_file before the script runs; the
# load address and the window are fixed here.
# The link address and the window: driver/boards/st/stm32mp257f_dk
# zephyr,sram (256 MiB at 0xa0000000) and tools/board-stm32mp2.sh's
# LOAD_END; selftest.sh compares all three.
setenv mp2z_load 0xa0000000
setenv mp2z_window 0x10000000
# 0x40 is the arm64 Image header.
setenv mp2z_min 0x40
if test -z "${mp2z_file}"; then setenv mp2z_file mp2z.bin; fi
if test -z "${mp2z_server}"; then
	echo "mp2z: refused: mp2z_server is not set"
	exit
fi
echo "mp2z: tftp ${mp2z_server}:${mp2z_file} -> ${mp2z_load}"

# dhcp without autoload (it would tftp the DHCP bootfile otherwise). An unset
# autoload is "yes"; setenv with an empty value deletes the variable again.
setenv mp2z_autoload ${autoload}
setenv autoload no
setenv mp2z_ok 0
if dhcp; then setenv mp2z_ok 1; fi
if test ${mp2z_ok} -eq 0; then
	echo "mp2z: refused: dhcp failed"
	setenv autoload ${mp2z_autoload}
	exit
fi
# A dead server: U-Boot's defaults (5 s, 10 retries) wait about 56 s, longer
# than BL2's IWDG (about 32 s). 2 s and 3 retries give up after 4 timeouts,
# 8 s (net/tftp.c:701, the count resets with every block received). Both
# variables are put back right after tftpboot (empty deletes them again).
setenv mp2z_tmo ${tftptimeout}
setenv mp2z_tmocnt ${tftptimeoutcountmax}
setenv tftptimeout 2000
setenv tftptimeoutcountmax 3
setenv mp2z_ok 0
if tftpboot ${mp2z_load} ${mp2z_server}:${mp2z_file}; then setenv mp2z_ok 1; fi
setenv autoload ${mp2z_autoload}
setenv tftptimeout ${mp2z_tmo}
setenv tftptimeoutcountmax ${mp2z_tmocnt}
if test ${mp2z_ok} -eq 0; then
	echo "mp2z: refused: tftpboot failed"
	exit
fi
# ${filesize} is hex without 0x (env_set_hex, net/net.c:721), so the
# comparisons use itest. The size is known only now: a file above the window
# has already run over what follows it (README.md, "What the scripts cannot
# check").
setenv mp2z_size ${filesize}
if itest ${filesize} -lt ${mp2z_min}; then
	echo "mp2z: refused: file ${filesize} bytes, below the ${mp2z_min}-byte Image header"
	exit
fi
if itest ${filesize} -gt ${mp2z_window}; then
	echo "mp2z: refused: file ${filesize} bytes, above the ${mp2z_window}-byte window"
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
