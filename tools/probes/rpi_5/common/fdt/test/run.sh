#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host test of fdt_walk.c under ASan/UBSan. Prints "fdt_walk: N checks, 0 failed".
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$(mktemp -d "${TMPDIR:-/tmp}/fdt_walk_test.XXXXXX")
trap 'rm -rf "$out"' EXIT
dtb=
if command -v dtc >/dev/null 2>&1; then
	dtc -q -I dts -O dtb -o "$out/pi5_cut.dtb" "$here/pi5_cut.dts"
	dtb=$out/pi5_cut.dtb
else
	echo "fdt_walk: SKIPPED the dtc-built tree (dtc not installed)"
fi
${CC:-cc} -std=c11 -Wall -Wextra -Werror -g -O1 \
	-fsanitize=address,undefined -fno-sanitize-recover=undefined \
	-I"$here/.." -o "$out/test_fdt" "$here/test_fdt.c" "$here/../fdt_walk.c"
"$out/test_fdt" ${dtb:+"$dtb"}
