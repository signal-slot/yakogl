#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-deqp-stage1.sh - dEQP stage 1 for the Verdin iMX8M Plus A53: the dEQP libraries built once with the
# Zephyr toolchain by tools/cts/cts-build-zephyr.sh. Stage 2 (imx8mp-deqp-build.sh) links against it.
#
#   tools/cts/imx8mp-deqp-stage1.sh
#   DEQP_MODULE=gles3 DEQP_STAGE1=<another directory> tools/cts/imx8mp-deqp-stage1.sh
#
# DEQP_MODULE: gles2 (default; the behaviour is the one this script always had) or gles3, which builds the
# dEQP-GLES3 package (cts-build-zephyr.sh MODULE=gles3). Keep one DEQP_STAGE1 directory per module.
# Needs: the variables of imx8mp-env.sh (Zephyr part) and DEQP_STAGE1, the stage-1 work directory
# (about 1.9 GB; cts-build-zephyr.sh fetches and patches the VK-GL-CTS tree there).
# Output: $DEQP_STAGE1/cts (sources) and $DEQP_STAGE1/build (libraries); log in $IMX8MP_OUT/deqp-stage1.log.
set -u
. "$(dirname "$0")/imx8mp-env.sh"
imx8mp_zephyr_env
# `imx8mp-deqp-stage1.sh gles3`: the gles3 stage 1 in $DEQP_STAGE1_GLES3 (default $IMX8MP_OUT/deqp-stage1-gles3),
# never in the gles2 directory.
if [ "${1:-}" = gles3 ]; then
	DEQP_MODULE=gles3
	DEQP_STAGE1=${DEQP_STAGE1_GLES3:-$IMX8MP_OUT/deqp-stage1-gles3}
fi
imx8mp_need DEQP_STAGE1
export BOARD WORK=$DEQP_STAGE1
# cts-build-zephyr.sh reads ZEPHYR_SDK, not ZEPHYR_SDK_INSTALL_DIR; and its toolchain file passes no -O, so
# libpng's NEON code (needs constant lane indices) fails at -O0 unless the environment supplies one.
export ZEPHYR_SDK=${ZEPHYR_SDK:-$ZEPHYR_SDK_INSTALL_DIR}
export CFLAGS=${CFLAGS:--O2} CXXFLAGS=${CXXFLAGS:--O2}
case "${DEQP_MODULE:-gles2}" in
	gles2) ;;
	gles3) export MODULE=gles3 ;;
	*) echo "imx8mp: DEQP_MODULE must be gles2 or gles3, not '$DEQP_MODULE'" >&2; exit 2 ;;
esac
export ZEPHYR_EXTRA_MODULES="$YAKOGL_DIR/driver;$YAKOGL_DIR"   # ZEPHYR_MODULES comes from imx8mp-env.sh
cd "$YAKOGL_DIR" || exit 1
sh tools/cts/cts-build-zephyr.sh > "$IMX8MP_OUT/deqp-stage1.log" 2>&1
rc=$?
echo "rc=$rc"
tail -5 "$IMX8MP_OUT/deqp-stage1.log"
# A failed ninja step prints its error lines after "FAILED:", far above the tail: show them.
[ "$rc" -eq 0 ] || grep -m1 -A30 '^FAILED:' "$IMX8MP_OUT/deqp-stage1.log" | cut -c1-400
[ "$rc" -eq 0 ] || grep -m1 -A1 '^FAILED:' "$IMX8MP_OUT/deqp-stage1.log" | tr ' ' '\n' | grep -i 'include' | sort -u | head -40
exit $rc
