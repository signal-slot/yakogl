#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The two AM62P images of gl_test case 217 (forcedpr), the forced partial render
# (driver/tools/pvrlab/README.md, "Case 217"):
#
#   driver/tools/pvrlab/forcedpr-images.sh [--dry-run] [--layers N] <out-dir>
#
#   <out-dir>/off  CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=n: the known negative (the 10 s stop)
#   <out-dir>/on   CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=y
#
# Both are the gl_test sample for the Verdin AM62P with the list capped at 512
# pages (CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=512, no grow possible),
# CONFIG_GL_TEST_CASES="217" and the board's watchdog self-arm tools/board.sh
# wants (as tools/cts/common-images.sh sets it); they differ in the one knob.
# Run one with tools/board.sh am62p run <out-dir>/<off|on>/zephyr/zephyr.bin <logdir>.
#
# Environment: ZEPHYR_WS (the west workspace), optional WEST_VENV, as for
# tools/cts/common-images.sh; driver/src/fw/pvr_fw_blob.h in this checkout
# (driver/tools/mkfwblob.sh, never committed). --dry-run needs none of it.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../../.." && pwd)
die() { echo "forcedpr-images: $*" >&2; exit 2; }
DRY=0 LAYERS=
while [ $# -gt 0 ]; do
    case "$1" in
    --dry-run) DRY=1; shift ;;
    --layers) [ $# -ge 2 ] || die "--layers needs a number"; LAYERS=$2; shift 2 ;;
    -*) die "unknown option $1" ;;
    *) break ;;
    esac
done
[ $# -eq 1 ] || die "usage: $0 [--dry-run] [--layers N] <out-dir>"
[ -z "$LAYERS" ] || [[ $LAYERS =~ ^[0-9]+$ ]] || die "--layers takes a number"
OUT=$1
CASES='-DCONFIG_GL_TEST_CASES="217"'
COMMON=("$CASES" -DCONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=512
        -DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n)
[ -z "$LAYERS" ] || COMMON+=("-DCONFIG_GL_TEST_FORCEDPR_LAYERS=$LAYERS")
ZBOARD=verdin_am62p/am62p54/a53
if [ "$DRY" = 1 ]; then
    for v in off on; do
        k=n; [ $v = on ] && k=y
        echo "$v gl_test -b $ZBOARD -DYAKOGL_DIR=$REPO ${COMMON[*]} -DCONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=$k"
    done
    exit 0
fi
: "${ZEPHYR_WS:?set ZEPHYR_WS to the west workspace (it holds zephyr/)}"
[ -d "$ZEPHYR_WS/zephyr" ] || die "$ZEPHYR_WS/zephyr does not exist"
[ -f "$REPO/driver/src/fw/pvr_fw_blob.h" ] ||
    die "driver/src/fw/pvr_fw_blob.h missing: generate it with driver/tools/mkfwblob.sh (never committed)"
[ -z "${WEST_VENV:-}" ] || PATH=$WEST_VENV/bin:$PATH
command -v west > /dev/null || die "west not on PATH (set WEST_VENV)"
export ZEPHYR_BASE=$ZEPHYR_WS/zephyr
MODS=$("$REPO/tools/cts/zephyr-modules.sh" "$ZEPHYR_WS") || die "zephyr-modules.sh failed"
mkdir -p "$OUT" || die "cannot create $OUT"
OUT=$(cd "$OUT" && pwd)
fails=0
for v in off on; do
    k=n; [ $v = on ] && k=y
    (cd "$ZEPHYR_WS" && west build -p always -b "$ZBOARD" -d "$OUT/$v" "$REPO/driver/samples/zephyr/gl_test" -- \
        "-DZEPHYR_MODULES=$MODS" "-DYAKOGL_DIR=$REPO" "${COMMON[@]}" \
        "-DCONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=$k") > "$OUT/$v.log" 2>&1 < /dev/null
    rc=$?
    bin=$OUT/$v/zephyr/zephyr.bin
    if [ "$rc" = 0 ] && [ -s "$bin" ]; then
        echo "$v rc=0 sha256=$(sha256sum "$bin" | cut -d' ' -f1) $bin"
    else
        echo "$v rc=$rc (see $OUT/$v.log)"; fails=$((fails + 1))
    fi
done
[ "$fails" = 0 ]
