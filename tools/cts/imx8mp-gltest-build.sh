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
# Needs: the variables of imx8mp-env.sh (Zephyr part). Output: $IMX8MP_OUT/<name>/zephyr/zephyr.bin.
set -u
. "$(dirname "$0")/imx8mp-env.sh"
[ $# -ge 2 ] || { echo "usage: $0 <name> <first case> [cmake args...]" >&2; exit 2; }
imx8mp_zephyr_env
name=$1; first=$2; shift 2
cd "$ZEPHYR_WORKSPACE" || exit 1
west build -p always -b "$BOARD" -d "$IMX8MP_OUT/$name" "$YAKOGL_DIR/driver/samples/zephyr/gc_gl_test" -- \
	-DZEPHYR_MODULES="$ZEPHYR_MODULES" -DYAKOGL_DIR="$YAKOGL_DIR" \
	-DEXTRA_CONF_FILE=prj_sweep.conf -DGL_TEST_ES100=ON -DCONFIG_GC_GL_TEST_SWEEP_FIRST="$first" "$@" \
	> "$IMX8MP_OUT/$name.log" 2>&1
rc=$?
echo "rc=$rc"
tail -3 "$IMX8MP_OUT/$name.log"
exit $rc
