#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-deqp-stage1.sh - dEQP stage 1 for the Verdin iMX8M Plus A53: the dEQP libraries built once with the
# Zephyr toolchain by tools/cts/cts-build-zephyr.sh. Stage 2 (imx8mp-deqp-build.sh) links against it.
#
#   tools/cts/imx8mp-deqp-stage1.sh
#
# Needs: the variables of imx8mp-env.sh (Zephyr part) and DEQP_STAGE1, the stage-1 work directory
# (about 1.9 GB; cts-build-zephyr.sh fetches and patches the VK-GL-CTS tree there).
# Output: $DEQP_STAGE1/cts (sources) and $DEQP_STAGE1/build (libraries); log in $IMX8MP_OUT/deqp-stage1.log.
set -u
. "$(dirname "$0")/imx8mp-env.sh"
imx8mp_zephyr_env
imx8mp_need DEQP_STAGE1
export BOARD WORK=$DEQP_STAGE1
export ZEPHYR_EXTRA_MODULES="$YAKOGL_DIR/driver;$YAKOGL_DIR"   # ZEPHYR_MODULES comes from imx8mp-env.sh
cd "$YAKOGL_DIR" || exit 1
sh tools/cts/cts-build-zephyr.sh > "$IMX8MP_OUT/deqp-stage1.log" 2>&1
rc=$?
echo "rc=$rc"
tail -5 "$IMX8MP_OUT/deqp-stage1.log"
exit $rc
