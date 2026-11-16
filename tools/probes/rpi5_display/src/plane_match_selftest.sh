#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Host self-test of src/plane_match.h (the U3 decision "does the HVS6 display
# list name the buffer the image has?", and the IOMMU4 pass-through question in
# front of it). Header-only, so the test compiles the same header the image
# compiles. Builds with -Wall -Wextra -Werror and ASan+UBSan into
# build/rpi5_display/plane_match/. Last line: "plane_match_test: N of N checks,
# 0 failed". Exit 0 ok, 1 failure, 77 with "gap: CC" when no C compiler is found.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
out=$root/build/rpi5_display/plane_match

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
	-o "$out/plane_match_test" \
	"$here/test/plane_match_test.c" || exit 1
"$out/plane_match_test"
