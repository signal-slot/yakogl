#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# build.sh [OUTDIR]: compile mp2-mmc.cmd and mp2-net.cmd into mp2-mmc.scr and
# mp2-net.scr in OUTDIR (default: the current directory). The .scr files are
# not committed.
#
# With mkimage on PATH: mkimage -A arm64 -O linux -T script -C none -n NAME -d
# CMD SCR. Without it, tools/uboot_script.py build (the same bytes; needs no
# u-boot-tools). MP2_NO_MKIMAGE=1 forces the second. SOURCE_DATE_EPOCH, when
# set, is the header's time in both, so a build is reproducible; otherwise both
# use the script's mtime.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-.}
mkdir -p "$out"

mk=
if [ -z "${MP2_NO_MKIMAGE:-}" ]; then mk=$(command -v mkimage || true); fi

for name in mp2-mmc mp2-net; do
	if [ -n "$mk" ]; then
		"$mk" -A arm64 -O linux -T script -C none -n "$name" \
			-d "$here/$name.cmd" "$out/$name.scr" > /dev/null
	else
		python3 -I "$root/tools/uboot_script.py" build "$here/$name.cmd" \
			-o "$out/$name.scr" --name "$name" > /dev/null
	fi
	echo "build.sh: $out/$name.scr (via $([ -n "$mk" ] && echo mkimage || echo uboot_script.py))"
done
