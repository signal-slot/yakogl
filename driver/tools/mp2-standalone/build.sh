#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# build.sh [OUTDIR]: compile mp2-standalone.cmd into OUTDIR/boot.scr.uimg
# (default OUTDIR: the current directory) and print its sha256. The image is
# not committed; the runner stages it as /boot/yakogl/standalone/boot.scr.uimg
# (docs/notes/stm32mp257f-dk-standalone.md, section 7).
#
# With mkimage on PATH: mkimage -A arm64 -O linux -T script -C none
# -n mp2-standalone -d mp2-standalone.cmd OUT. Without it, tools/uboot_script.py
# build (the same bytes; needs no u-boot-tools). MP2_NO_MKIMAGE=1 forces the
# second.
#
# The header's time is SOURCE_DATE_EPOCH when set, else the commit time of
# mp2-standalone.cmd (0 outside git), so two builds of one commit have the
# same sha256 and the runner can compare the staged script with its build.
#
# Output, last line: "build.sh: OUT sha256 HEX (via mkimage|uboot_script.py)".
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-.}
if [ "$out" != / ]; then out=${out%/}; fi
mkdir -p "$out"
cmd="$here/mp2-standalone.cmd"
img="$out/boot.scr.uimg"

if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
	SOURCE_DATE_EPOCH=$(git -C "$here" log -1 --format=%ct -- mp2-standalone.cmd 2> /dev/null || true)
	export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-0}
fi

mk=
if [ -z "${MP2_NO_MKIMAGE:-}" ]; then mk=$(command -v mkimage || true); fi

rm -f "$img"
if [ -n "$mk" ]; then
	"$mk" -A arm64 -O linux -T script -C none -n mp2-standalone \
		-d "$cmd" "$img" > /dev/null
	via=mkimage
else
	python3 -I "$root/tools/uboot_script.py" build "$cmd" \
		-o "$img" --name mp2-standalone --arch arm64 > /dev/null
	via=uboot_script.py
fi
sum=$(sha256sum "$img" | cut -d' ' -f1)
echo "build.sh: $img sha256 $sum (via $via)"
