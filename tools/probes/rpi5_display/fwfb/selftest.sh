#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Host self-test of fw_fb_dt (the simple-framebuffer node of the DTB a kernel=
# image receives). Builds with -Wall -Wextra -Werror and ASan+UBSan into
# build/rpi5_display/fwfb/. Last line: "fw_fb_dt_test: N of N checks, 0 failed".
# Exit 0 ok, 1 failure, 77 with "gap: CC" when no C compiler is found.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
out=$root/build/rpi5_display/fwfb
fdt=$root/tools/probes/rpi_5/common/fdt

cc=${CC:-}
if [ -z "$cc" ]; then
	for c in cc gcc clang; do
		if command -v "$c" >/dev/null 2>&1; then cc=$c; break; fi
	done
fi
if [ -z "$cc" ] || ! command -v "$cc" >/dev/null 2>&1; then
	echo "gap: CC"
	exit 77
fi

mkdir -p "$out"
"$cc" -std=c11 -D_DEFAULT_SOURCE -Wall -Wextra -Werror -g -O1 \
	-fsanitize=address,undefined -fno-sanitize-recover=undefined \
	-I"$fdt" -o "$out/fw_fb_dt_test" \
	"$here/test/fw_fb_dt_test.c" "$here/fw_fb_dt.c" "$fdt/fdt_walk.c" || exit 1
"$out/fw_fb_dt_test"
