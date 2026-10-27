#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gltest-build.sh - one gl_test image for the Verdin iMX8M Plus, in the sweep configuration
# (prj_sweep.conf: online compiler, the ES 1.00 shader set, 48 case numbers from GC_GL_TEST_SWEEP_FIRST).
#
#   tools/cts/imx8mp-gltest-build.sh <name> <first case> [extra cmake -D...]
#   tools/cts/imx8mp-gltest-build.sh sweep96-1234abcd 96
#
# Run one case alone with selector SEL=<case - first> (tools/board.sh imx8mp run): the image then ends with that case's
# CASE record (first bad check, got, want). SEL=256 runs the whole range and ends with the counters record.
#
# Sweep lists: pass -DCONFIG_GC_GL_TEST_SWEEP_LISTS=<lists> as the plain value, e.g. 1-6,8-10;10;2 (lists split by ';').
# CMake reads a bare ';' as its list separator and drops it (the image got "1-6,8-10102"), so this script escapes every
# ';' of that argument and adds the quotes Kconfig needs. A value that is already escaped or quoted gives the same result.
#
# Watchdog: the image gets -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y by default (it arms WDOG1 itself, "gc_wdog_arm: armed").
# tools/board.sh refuses an image without that line (rc=3, "would not arm WDOG1"), and gl_test does not select it
# itself (imx8mp-plan1.md, Step 3). WDOG_SELF_ARM=0 in the environment turns the default off. A
# -DCONFIG_GC_BOARD_WDOG_SELF_ARM=... among the extra arguments wins over both.
#
# DRYRUN=1 prints the west command and the CONFIG_ lines it asks for, and builds nothing (needs none of the variables).
#
# Needs: the variables of imx8mp-env.sh (Zephyr part). Output: $IMX8MP_OUT/<name>/zephyr/zephyr.bin.
set -u
. "$(dirname "$0")/imx8mp-env.sh"
[ $# -ge 2 ] || { echo "usage: $0 <name> <first case> [cmake args...]" >&2; exit 2; }
name=$1; first=$2; shift 2
case "$name" in */* | '' | .*) echo "imx8mp-gltest-build: <name> must be a plain directory name, not '$name'" >&2; exit 2 ;; esac
case "$first" in '' | *[!0-9]*) echo "imx8mp-gltest-build: <first case> must be a number, not '$first'" >&2; exit 2 ;; esac
dry=${DRYRUN:-0}
if [ "$dry" != 1 ]; then imx8mp_zephyr_env; fi   # exits on a missing variable; a dry run needs none
extra=(); has_wdog=0
for a in "$@"; do
	case $a in
	-DCONFIG_GC_GL_TEST_SWEEP_LISTS=*)
		v=${a#-DCONFIG_GC_GL_TEST_SWEEP_LISTS=}
		v=${v//\\;/;}
		v=${v#\"}; v=${v%\"}
		a="-DCONFIG_GC_GL_TEST_SWEEP_LISTS=\"${v//;/\\;}\"" ;;
	-DCONFIG_GC_BOARD_WDOG_SELF_ARM=*) has_wdog=1 ;;
	esac
	extra+=("$a")
done
if [ "$has_wdog" = 0 ] && [ "${WDOG_SELF_ARM:-1}" != 0 ]; then
	extra+=(-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y)
fi
cmd=(west build -p always -b "${BOARD:-}" -d "${IMX8MP_OUT:-}/$name" "${YAKOGL_DIR:-}/driver/samples/zephyr/gl_test" --
	-DZEPHYR_MODULES="${ZEPHYR_MODULES:-}" -DYAKOGL_DIR="${YAKOGL_DIR:-}"
	-DEXTRA_CONF_FILE=prj_sweep.conf -DGL_TEST_ES100=ON -DCONFIG_GC_GL_TEST_SWEEP_FIRST="$first" ${extra[@]+"${extra[@]}"})
if [ "$dry" = 1 ]; then
	printf '%q ' "${cmd[@]}"; echo
	for a in ${extra[@]+"${extra[@]}"}; do
		case $a in
		-DCONFIG_GC_GL_TEST_SWEEP_LISTS=*) v=${a#-D}; echo "${v//\\;/;}" ;;
		-DCONFIG_*) echo "${a#-D}" ;;
		esac
	done
	exit 0
fi
cd "$ZEPHYR_WORKSPACE" || exit 1
"${cmd[@]}" > "$IMX8MP_OUT/$name.log" 2>&1
rc=$?
# a build that exits 0 but wrote no image is not a build
if [ "$rc" = 0 ] && [ ! -s "$IMX8MP_OUT/$name/zephyr/zephyr.bin" ]; then rc=no_image; fi
echo "rc=$rc"
tail -3 "$IMX8MP_OUT/$name.log"
[ "$rc" = 0 ]
