#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gltest-build.sh - one gc_gl_test image for the Verdin iMX8M Plus, in the sweep configuration
# (prj_sweep.conf: online compiler, the ES 1.00 shader set, 48 case numbers from GC_GL_TEST_SWEEP_FIRST).
#
#   tools/cts/imx8mp-gltest-build.sh <name> <first case> [extra cmake -D...]
#   tools/cts/imx8mp-gltest-build.sh sweep96-1234abcd 96
#
# Run one case alone with selector SEL=<case - first> (tools/board.sh imx8mp run): the image then ends with that case's
# CASE record (first bad check, got, want). SEL=256 runs the whole range and ends with the counters record.
#
# Every image is built with -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y ("gc_wdog_arm: armed"): tools/board.sh refuses an
# image without that line (rc=3), and gc_gl_test does not select it itself (imx8mp-plan1.md, Step 3).
#
# Needs: the variables of imx8mp-env.sh (Zephyr part). Output: $IMX8MP_OUT/<name>/zephyr/zephyr.bin.
set -u
. "$(dirname "$0")/imx8mp-env.sh"
[ $# -ge 2 ] || { echo "usage: $0 <name> <first case> [cmake args...]" >&2; exit 2; }
name=$1; first=$2; shift 2
case "$name" in */* | '' | .*) echo "imx8mp-gltest-build: <name> must be a plain directory name, not '$name'" >&2; exit 2 ;; esac
case "$first" in '' | *[!0-9]*) echo "imx8mp-gltest-build: <first case> must be a number, not '$first'" >&2; exit 2 ;; esac
imx8mp_zephyr_env
cd "$ZEPHYR_WORKSPACE" || exit 1
west build -p always -b "$BOARD" -d "$IMX8MP_OUT/$name" "$YAKOGL_DIR/driver/samples/zephyr/gc_gl_test" -- \
	-DZEPHYR_MODULES="$ZEPHYR_MODULES" -DYAKOGL_DIR="$YAKOGL_DIR" \
	-DEXTRA_CONF_FILE=prj_sweep.conf -DGL_TEST_ES100=ON -DCONFIG_GC_GL_TEST_SWEEP_FIRST="$first" \
	-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y "$@" \
	> "$IMX8MP_OUT/$name.log" 2>&1
rc=$?
# a build that exits 0 but wrote no image is not a build
if [ "$rc" = 0 ] && [ ! -s "$IMX8MP_OUT/$name/zephyr/zephyr.bin" ]; then rc=no_image; fi
echo "rc=$rc"
tail -3 "$IMX8MP_OUT/$name.log"
[ "$rc" = 0 ]
