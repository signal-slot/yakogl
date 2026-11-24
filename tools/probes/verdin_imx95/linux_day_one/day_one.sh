#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# The Verdin iMX95's day-one reading under the module's own Linux, read only
# (README.md): the pre-study's B1 Linux half (docs/notes/pre-study-imx95.md
# section 6) and the display note's M1..M4 (driver/docs/imx95-display.md
# section 6). Runs ON THE BOARD as root (POSIX sh); writes the record to
# stdout. It writes no register, no file, no I2C device; it loads no
# module, mounts nothing and touches no service.
#
#   sh day_one.sh [--readlist FILE] [--no-m4] [--root DIR] [--reader KIND]
#
# --readlist  the M4 read list (the #@read lines of
#             driver/docs/imx95-display-allow.txt; "-" reads stdin). Without
#             it M4 is skipped and says so.
# --no-m4     skip M4 even when its gates hold.
# --root      prefix of /proc, /sys, /etc, /boot (the host selftest's fake
#             tree); the commands are taken from PATH.
# --reader    how M4 reads a word: auto (the first of python3, devmem2 and
#             devmem that reads the first gated word of the list: a reader
#             is chosen only once it has read), python3 (mmap of /dev/mem
#             O_RDONLY/PROT_READ; needs python3's mmap module, which the
#             board's Toradex image of 2026-11-25 lacks), devmem2 (`devmem2
#             ADDR w`: it opens /dev/mem read-write but writes nothing
#             without a value argument, and none is ever given), devmem
#             (busybox `devmem ADDR 32`), or file:PATH (a fixture
#             "0xADDR 0xVALUE" per line, the selftest's).
#
# M4 reads a register only when the read list names it and its gate holds:
# gate "lvds" needs an LVDS connector of /sys/class/drm connected and
# enabled (M2) and the DISPLAY power domain on in pm_genpd_summary (M3);
# "dsi" a DSI connector and DISPLAY; "display" either connector and
# DISPLAY; "camera" a DSI connector, DISPLAY and the CAMERA domain. A read
# of the DPU with DISPLAY off is the unknown U6 of the note: never tried.
# An address of a "#@noread" line is never read, whatever the list says
# (the words whose read faulted on the board, or changes state).
set -u

ROOT=
READLIST=
READER=auto
M4=1
while [ $# -gt 0 ]; do
	case $1 in
	--readlist) READLIST=$2; shift 2 ;;
	--no-m4) M4=0; shift ;;
	--root) ROOT=$2; shift 2 ;;
	--reader) READER=$2; shift 2 ;;
	*) echo "day_one.sh: unknown argument $1" >&2; exit 2 ;;
	esac
done

have() { command -v "$1" >/dev/null 2>&1; }
# cat a file of the (possibly fake) root, or a line saying it is not there
show() {
	if [ -r "$ROOT$1" ]; then
		cat "$ROOT$1" 2>/dev/null | tr -d '\000'
	else
		echo "# $1: not readable"
	fi
}
# one command's output, or a line saying it is not there
run() {
	if have "$1"; then
		"$@" 2>&1
	else
		echo "# $1: not installed"
	fi
}
section() { printf '[%s]\n' "$1"; }
dmesg_all() { run dmesg 2>/dev/null; }

echo "# Verdin iMX95 day-one record (read only). Format: tools/probes/verdin_imx95/linux_day_one/README.md"
echo "# date $(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null)"

# ---- M1: the board and the kernel ----------------------------------------
section m1
echo "model: $(show /proc/device-tree/model)"
echo "compatible: $(show /proc/device-tree/compatible | tr '\n' ' ')"
echo "uname: $(run uname -a)"
echo "kernel release: $(run uname -r)"
show /etc/os-release | sed 's/^/os-release: /'
for f in /boot/overlays.txt /boot/firmware/overlays.txt /media/boot/overlays.txt; do
	[ -r "$ROOT$f" ] && show "$f" | sed "s|^|overlays $f: |"
done
echo "u-boot version (device tree): $(show /proc/device-tree/chosen/u-boot,version)"
if have fw_printenv; then
	fw_printenv ver 2>&1 | sed 's/^/u-boot env: /'
else
	echo "u-boot env: # fw_printenv not installed"
fi
echo "dpu95 in this kernel: $(dmesg_all | grep -c 'dpu95') dmesg lines"

# ---- B1 (Linux half): memory, interrupts, the GPU's driver ---------------
section b1
show /proc/iomem | sed 's/^/iomem: /'
show /proc/interrupts | sed 's/^/interrupts: /'
dmesg_all | grep -iE 'panthor|mali|gpu@4d900000' | sed 's/^/gpu dmesg: /'
dmesg_all | grep -iE 'firmware.*(mali|csffw)|mali_csffw' | sed 's/^/gpu firmware: /'
echo "findmnt /var: $(run findmnt -T /var | tail -n 1)"
show /proc/meminfo | head -n 3 | sed 's/^/meminfo: /'
show /proc/cmdline | sed 's/^/cmdline: /'
for d in "$ROOT"/sys/bus/nvmem/devices/*; do
	[ -e "$d" ] && echo "nvmem: ${d##*/}"
done

# ---- M2: which display does Linux drive (X12 LVDS, X4 DSI)? -------------
section m2
dsi_live=0
lvds_live=0
for c in "$ROOT"/sys/class/drm/card*-*; do
	[ -d "$c" ] || continue
	n=${c##*/}
	st=$(cat "$c/status" 2>/dev/null)
	en=$(cat "$c/enabled" 2>/dev/null)
	echo "$n status $st"
	echo "$n enabled $en"
	echo "$n dpms $(cat "$c/dpms" 2>/dev/null)"
	if [ -r "$c/modes" ]; then
		sed "s/^/$n mode /" "$c/modes"
	fi
	case $n in
	*DSI*) [ "$st" = connected ] && [ "$en" = enabled ] && dsi_live=1 ;;
	*LVDS*) [ "$st" = connected ] && [ "$en" = enabled ] && lvds_live=1 ;;
	esac
done
echo "dsi connector connected and enabled: $dsi_live"
echo "lvds connector connected and enabled: $lvds_live"
dmesg_all | grep -iE 'dpu95|imx95-mipi-dsi|imx95-ldb|ldb|lvds|Pixel Link|sn65dsi8|panel|ili2132|dw-mipi-dsi' | sed 's/^/display dmesg: /'
# the backlight as Linux runs it (the PWM's period, duty and polarity)
for d in "$ROOT"/sys/class/backlight/*; do
	[ -d "$d" ] || continue
	echo "backlight ${d##*/}: brightness $(cat "$d/brightness" 2>/dev/null) of $(cat "$d/max_brightness" 2>/dev/null)"
done
if [ -r "$ROOT/sys/kernel/debug/pwm" ]; then
	sed 's/^/pwm: /' "$ROOT/sys/kernel/debug/pwm"
else
	echo "pwm: # /sys/kernel/debug/pwm not readable"
fi
# the I2C_2_DSI bus: LPI2C3, i2c@42530000
bus=
for a in "$ROOT"/sys/bus/i2c/devices/i2c-*; do
	[ -e "$a/of_node" ] || continue
	case $(readlink "$a/of_node") in
	*i2c@42530000) bus=${a##*/i2c-} ;;
	esac
done
echo "i2c_2_dsi bus: ${bus:-not found}"
if [ -n "$bus" ]; then
	run i2cdetect -r -y "$bus" | sed 's/^/i2cdetect: /'
	# the bridge's registers (U20 is 0x18): its driver's regmap first (no
	# bus access of ours), else i2cdump's reads (no -f: a bound driver's
	# address is refused, which the record shows)
	rm_file=$ROOT/sys/kernel/debug/regmap/$bus-002c/registers
	if [ -r "$rm_file" ]; then
		sed 's/^/bridge regmap: /' "$rm_file"
	else
		echo "bridge regmap: # $bus-002c not in debugfs"
		run i2cdump -y -r 0x00-0xff "$bus" 0x2c b | sed 's/^/bridge i2cdump: /'
	fi
fi

# ---- M3: what the System Manager lets the AP set; can route (a) run? -----
section m3
dmesg_all | grep -iE 'scmi|system manager|imx-sm|SM Version|SM build' | sed 's/^/sm dmesg: /'
for d in "$ROOT"/sys/kernel/debug/scmi/*/info; do
	[ -d "$d" ] || continue
	for f in "$d"/*; do
		[ -r "$f" ] && echo "scmi info ${f#"$ROOT"/sys/kernel/debug/scmi/}: $(cat "$f")"
	done
done
clk=$ROOT/sys/kernel/debug/clk/clk_summary
if [ -r "$clk" ]; then
	head -n 3 "$clk" | sed 's/^/clk: /'
	grep -E '^[[:space:]]*(dispapb|dispaxi|dispocram|disp1pix|videopll1|videopll1_vco|ldbpll_vco|ldbpll|ldb_phy_div|ldb_pll_div7|lvds_ch[01]_gate|lvds_di[01]_gate|mipiphycfg|mipiphypllref|mipiphypllbypass|camapb|gpu|gpuapb|gpu_cgc|lpi2c2|lpi2c3|tpm5|tpm6)[[:space:]]' "$clk" |
		sed 's/^/clk: /'
else
	echo "clk: # clk_summary not readable (debugfs not mounted?)"
fi
pd=$ROOT/sys/kernel/debug/pm_genpd/pm_genpd_summary
display_on=0
camera_on=0
if [ -r "$pd" ]; then
	sed 's/^/genpd: /' "$pd"
	grep -qiE '^display[[:space:]]+on' "$pd" && display_on=1
	grep -qiE '^camera[[:space:]]+on' "$pd" && camera_on=1
else
	echo "genpd: # pm_genpd_summary not readable"
fi
echo "display domain on: $display_on"
echo "camera domain on: $camera_on"
if [ -r "$ROOT/proc/config.gz" ] && have zcat; then
	zcat "$ROOT/proc/config.gz" | grep -E '^(# )?CONFIG_(KEXEC|KEXEC_FILE|CRASH_DUMP|CRASH_RESERVE|STRICT_DEVMEM|IO_STRICT_DEVMEM|DRM_PANTHOR|DRM_IMX95_DPU|DRM_TI_SN65DSI83)[ =]' |
		sed 's/^/config: /'
else
	echo "config: # /proc/config.gz not readable"
fi
show /proc/iomem | grep -i 'crash' | sed 's/^/crash reservation: /'
grep -qi crash "$ROOT/proc/iomem" 2>/dev/null || echo "crash reservation: none in /proc/iomem"

# ---- M4: the golden registers, behind the gates -------------------------
section m4
read_word() { # $1 address (0x...): prints 0x%08x or ERR
	case $READER in
	file:*)
		v=$(awk -v a="$1" 'tolower($1) == tolower(a) { print $2; exit }' "${READER#file:}")
		[ -n "$v" ] && printf '0x%08x\n' "$v" || echo ERR
		;;
	devmem)
		v=$(devmem "$1" 32 2>/dev/null) && printf '0x%08x\n' "$v" || echo ERR
		;;
	devmem2)
		# "Value at address 0x4B0C0000 (0xffff8...): 0x14": the last field
		v=$(devmem2 "$1" w 2>/dev/null | sed -n 's/^Value at address .*: \(0x[0-9a-fA-F]*\)$/\1/p') &&
			[ -n "$v" ] && printf '0x%08x\n' "$v" || echo ERR
		;;
	python3)
		# a child per word: a bus error costs that word, not the record
		python3 -I -c '
import mmap, os, struct, sys
a = int(sys.argv[1], 16); base = a & ~0xfff
fd = os.open("/dev/mem", os.O_RDONLY | os.O_SYNC)
m = mmap.mmap(fd, 0x1000, mmap.MAP_SHARED, mmap.PROT_READ, offset=base)
print("0x%08x" % memoryview(m).cast("I")[(a - base) // 4])
' "$1" 2>/dev/null || echo ERR
		;;
	esac
}
m4_gate() { # $1 gate: 0 when it holds
	case $1 in
	display) { [ "$dsi_live" = 1 ] || [ "$lvds_live" = 1 ]; } && [ "$display_on" = 1 ] ;;
	lvds) [ "$lvds_live" = 1 ] && [ "$display_on" = 1 ] ;;
	dsi) [ "$dsi_live" = 1 ] && [ "$display_on" = 1 ] ;;
	camera) [ "$dsi_live" = 1 ] && [ "$display_on" = 1 ] && [ "$camera_on" = 1 ] ;;
	*) return 1 ;;
	esac
}
if [ "$M4" = 0 ]; then
	echo "# M4 skipped (--no-m4)"
elif [ -z "$READLIST" ]; then
	echo "# M4 skipped: no --readlist"
else
	if [ "$READLIST" = - ]; then rl=$(cat); else rl=$(cat "$READLIST"); fi
	noread=$(echo "$rl" | awk '$1 == "#@noread" { print tolower($2) }')
	# the reader: the first candidate that reads the first gated word
	first=$(echo "$rl" | while read -r tag addr n name gate _; do
		[ "$tag" = "#@read" ] && m4_gate "$gate" && { printf '0x%08x\n' $((addr)); break; }
	done)
	if [ "$READER" = auto ]; then
		READER=none
		if [ -z "$first" ]; then
			echo "# reader: no gate holds, no reader tried"
		else
			for k in python3 devmem2 devmem; do
				if ! have "$k"; then
					echo "# reader $k: not installed"
					continue
				fi
				v=$(READER=$k read_word "$first")
				if [ "$v" = ERR ]; then
					echo "# reader $k: could not read $first"
					continue
				fi
				echo "# reader $k: read $first = $v"
				READER=$k
				break
			done
		fi
	fi
	echo "reader: $READER"
	if [ "$READER" = none ]; then
		[ -n "$first" ] && echo "# M4 skipped: no reader could read $first"
		READER=
	fi
	echo "$rl" | while read -r tag addr n name gate _; do
		[ "$tag" = "#@read" ] || continue
		if ! m4_gate "$gate"; then
			echo "# $name: gate $gate does not hold, not read"
			continue
		fi
		[ -n "$READER" ] || continue
		i=0
		while [ "$i" -lt "$n" ]; do
			a=$(printf '0x%08x' $((addr + 4 * i)))
			if echo "$noread" | grep -qx "$a"; then
				echo "# $a: #@noread, not read ($name+$(printf '0x%x' $((4 * i))))"
			else
				echo "$a $(read_word "$a") $name+$(printf '0x%x' $((4 * i)))"
			fi
			i=$((i + 1))
		done
	done
fi
echo "# end"
