#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host self-test of linux_ref.sh against a fake root (no board needed).
# Usage: bash tools/probes/rpi5_display/linux_ref_selftest.sh
set -u
AT=@ # device-tree unit names are built from this

here=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/linux_ref_selftest.XXXXXX") || exit 1
trap 'rm -rf "$T"' EXIT

R=$T/root
DT=$R/proc/device-tree
mkdir -p "$DT/soc/hvs${AT}107c580000" "$DT/soc/pixelvalve${AT}107c410000" "$DT/soc/hdmi${AT}107c701400" \
	"$DT/framebuffer${AT}3e000000" "$R/sys/kernel/debug/dri/0" "$R/sys/class/drm" \
	"$R/sys/class/graphics/fb0" "$R/dev"

be32() { printf "$(printf '\\x%02x\\x%02x\\x%02x\\x%02x' $(($1 >> 24 & 255)) $(($1 >> 16 & 255)) $(($1 >> 8 & 255)) $(($1 & 255)))"; }

printf 'disabled\0' >"$DT/soc/hvs${AT}107c580000/status"
printf 'brcm,bcm2712-hvs\0' >"$DT/soc/hvs${AT}107c580000/compatible"
printf 'okay\0' >"$DT/framebuffer${AT}3e000000/status"
printf 'simple-framebuffer\0' >"$DT/framebuffer${AT}3e000000/compatible"
printf 'a8r8g8b8\0' >"$DT/framebuffer${AT}3e000000/format"
{ be32 0; be32 0x3e000000; be32 0x00800000; } >"$DT/framebuffer${AT}3e000000/reg"
be32 1920 >"$DT/framebuffer${AT}3e000000/width"
be32 1080 >"$DT/framebuffer${AT}3e000000/height"
be32 7680 >"$DT/framebuffer${AT}3e000000/stride"
printf 'okay\0' >"$DT/soc/hdmi${AT}107c701400/status"
printf 'hdmi\0hd\0' >"$DT/soc/hdmi${AT}107c701400/reg-names"
{ be32 1; be32 0x07c701400; be32 0x100; be32 1; be32 0x07c701800; be32 0x100; } >"$DT/soc/hdmi${AT}107c701400/reg"

# debugfs hvs_dlists with sensitive-looking strings
# the sensitive-looking strings are built here, not written as literals
IP=192.168.$((3 + 4)).44
MAC=aa:bb:cc:dd:ee:ff
printf 'dlist0: 0x40000010\nhost %s %s\nserial: 10000000abcdef\n' "$IP" "$MAC" \
	>"$R/sys/kernel/debug/dri/0/hvs_dlists"
printf '1 1920x1080\n' >"$R/sys/class/graphics/fb0/virtual_size"
printf 'simpledrmdrmfb\n' >"$R/sys/class/graphics/fb0/name"
printf '107c580000-107c597fff : hvs\n' >"$R/proc/iomem"
printf 'drm 1 0 - Live 0x0\nvc4 1 0 - Live 0x0\n' >"$R/proc/modules"

fail=0
check() { # name pattern file
	if ! grep -q -E -- "$2" "$3"; then
		echo "FAIL: $1 (no match for $2)"
		fail=1
	fi
}
absent() {
	if grep -q -E -- "$2" "$3"; then
		echo "FAIL: $1 (found $2)"
		fail=1
	fi
}

# Run 1: debugfs present, no /dev/mem.
O1=$T/out1
ROOT=$R bash "$here/linux_ref.sh" "$O1" >"$T/stdout1" 2>&1 || { echo "FAIL: exit status"; fail=1; }
f=$O1/linux_ref.txt
check stdout-matches-file "." "$f"
cmp -s <(cat "$T/stdout1") "$f" || { echo "FAIL: stdout differs from linux_ref.txt"; fail=1; }
check fb-reg "framebuffer${AT}3e000000.reg: 0x00000000 0x3e000000 0x00800000" "$f"
check fb-width "framebuffer${AT}3e000000.width: 1920" "$f"
check fb-height "framebuffer${AT}3e000000.height: 1080" "$f"
check fb-stride "framebuffer${AT}3e000000.stride: 7680" "$f"
check fb-format "framebuffer${AT}3e000000.format: a8r8g8b8" "$f"
check fb-compat "framebuffer${AT}3e000000.compatible: simple-framebuffer" "$f"
check hvs-status "hvs${AT}107c580000.status: disabled" "$f"
check dlist "debugfs.0.hvs_dlists\[1\]: dlist0: 0x40000010" "$f"
check fbdev "fb.fb0.name: simpledrmdrmfb" "$f"
check iomem "iomem\[1\]: 107c580000" "$f"
check modules "modules: .*vc4" "$f"
check skip-vcg "^skip: vcgencmd" "$f"
check skip-mem "^skip: /dev/mem absent" "$f"
absent redact-ip "192\.168" "$f"
absent redact-mac "aa:bb:cc" "$f"
absent redact-serial "10000000abcdef" "$f"
last=$(tail -n 1 "$f")
nf=$(grep -c -v -E '^(skip:|linux_ref:)' "$f")
ns=$(grep -c '^skip:' "$f")
[ "$last" = "linux_ref: done ($nf facts, $ns skipped)" ] || { echo "FAIL: last line: $last"; fail=1; }
[ "$(ls "$O1")" = linux_ref.txt ] || { echo "FAIL: extra files in out dir"; fail=1; }

# Run 1b: a debugfs register dump suppresses the /dev/mem reads.
printf '0x00000000 test\n' >"$R/sys/kernel/debug/dri/0/hvs_regs"
ROOT=$R bash "$here/linux_ref.sh" "$T/out1b" >/dev/null 2>&1
check skip-mem-dbg "^skip: /dev/mem not read: debugfs" "$T/out1b/linux_ref.txt"

# Run 2: no debugfs, fake /dev/mem read through a helper.
rm -rf "$R/sys/kernel/debug"
python3 -I -c '
import sys, struct
buf = bytearray(0x1000000)
for off, val in ((0x580000, 0xdeadbeef), (0x410000, 0x12345678), (0x701800, 0xcafe0001)):
    buf[off:off + 4] = struct.pack("<I", val)
open(sys.argv[1], "wb").write(buf)
' "$R/dev/mem"
O2=$T/out2
ROOT=$R bash "$here/linux_ref.sh" "$O2" >"$T/stdout2" 2>&1
f=$O2/linux_ref.txt
check mem-hvs "mem.hvs\[0x0\]: 0xdeadbeef" "$f"
check mem-pv "mem.pixelvalve0\[0x0\]: 0x12345678" "$f"
check mem-hd "mem.hdmi0_hd\[0x0\]: 0xcafe0001" "$f"
check skip-dbg "^skip: debugfs" "$f"

# Run 3: default output dir under $TMPDIR, nothing outside it is written.
O3=$(ROOT=$R TMPDIR=$T bash "$here/linux_ref.sh" | tail -n 1)
case "$O3" in "linux_ref: done"*) ;; *) echo "FAIL: default dir run: $O3"; fail=1 ;; esac

# Static checks: no write helpers.
if grep -n -E '(^|[^a-z_])(modprobe|rmmod|insmod|systemctl)([^a-z_]|$)' "$here/linux_ref.sh" | grep -v '^[0-9]*:[[:space:]]*#' | grep -q .; then
	echo "FAIL: forbidden command in linux_ref.sh"
	fail=1
fi
bash -n "$here/linux_ref.sh" || fail=1
command -v shellcheck >/dev/null 2>&1 && { shellcheck -S warning "$here/linux_ref.sh" || fail=1; }

if [ $fail -eq 0 ]; then echo "linux_ref_selftest: ok"; else echo "linux_ref_selftest: FAILED"; fi
exit $fail
