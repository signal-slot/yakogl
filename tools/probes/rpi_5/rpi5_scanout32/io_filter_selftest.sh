#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# io_filter_selftest: host test of src/io_filter.h (scanout32_write_ok): channel
# 0's LPTRS and list words >= 32 are accepted; list words 0..31, the other
# channels' LPTRS and the other HVS registers are refused. Prints
# "io_filter_selftest: N ok, 0 failed"; exit 0, nonzero on any failure.
# No cc: a gap, exit 77. Scratch goes to a mktemp dir under $TMPDIR.
here=$(cd "$(dirname "$0")" && pwd) || exit 1
root=$(cd "$here/../../../.." && pwd) || exit 1
cc=${CC:-cc}
if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
tmp=$(mktemp -d "${TMPDIR:?TMPDIR is not set}/io_filter.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
cat >"$tmp/t.c" <<'EOF'
#include <stdio.h>
#include <stdint.h>
#define HVS_SIZE 0x1a000u
#define CHAN 0u
#include "io_filter.h"

static int ok, bad;
static void want(const char *what, uint32_t off, int expect)
{
	int got = scanout32_write_ok(off) ? 1 : 0;

	if (got == expect) {
		ok++;
	} else {
		bad++;
		printf("FAIL %s off 0x%x: got %d want %d\n", what, off, got, expect);
	}
}

int main(void)
{
	uint32_t w;
	unsigned int c;

	want("LPTRS(0)", hvs6s_lptrs_reg(0), 1);
	for (c = 1; c < 3; c++) {
		want("LPTRS(other)", hvs6s_lptrs_reg(c), 0);
		want("STATUS(other)", hvs6s_status_reg(c), 0);
		want("DL(other)", hvs6s_dl_reg(c), 0);
	}
	want("STATUS(0)", hvs6s_status_reg(0), 0);
	want("CTRL1(0)", hvs6s_ctrl1_reg(0), 0);
	want("DL(0)", hvs6s_dl_reg(0), 0);
	want("DISPCTRL", 0x0u, 0);
	want("0x4", 0x4u, 0);
	want("just below list memory", HVS6_DLIST_OFFSET - 4u, 0);
	for (w = 0; w < HVS6S_KEPT; w++) {
		want("list word < KEPT", hvs6s_word_off(w), 0);
	}
	want("list word KEPT", hvs6s_word_off(HVS6S_KEPT), 1);
	want("list word KEPT+1", hvs6s_word_off(HVS6S_KEPT + 1u), 1);
	want("last word", HVS_SIZE - 4u, 1);
	want("HVS_SIZE", HVS_SIZE, 0);
	want("above HVS_SIZE", HVS_SIZE + 4u, 0);
	want("huge", 0xffffffffu, 0);
	printf("io_filter_selftest: %d ok, %d failed\n", ok, bad);
	return bad != 0;
}
EOF
if ! "$cc" -std=c11 -Wall -Wextra -Werror \
    -I"$here/src" -I"$root/driver/zephyr/lib/board/rpi5_scanout" \
    -o "$tmp/t" "$tmp/t.c"; then
  echo "io_filter_selftest: compile failed"
  exit 1
fi
"$tmp/t" || exit 1
exit 0
