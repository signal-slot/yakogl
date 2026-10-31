#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# pi4-gles3-stage1.sh: the dEQP-GLES3 stage 1 for the Pi 4, the step of
# docs/es3/board-recipe-pi4.md section 2 (its own WORK beside the gles2 one),
# from the dependency layout (tools/cts/deps-env.sh --layout):
#
#   tools/cts/pi4-gles3-stage1.sh
#
# VK-GL-CTS comes from $DEPS/VK-GL-CTS (deps-env.sh --build --board rpi_4b
# DEQP_WORK fetches it); the result is $DEPS/deqp-stage1-rpi_4b-gles3, which
# pi4-deqp-images.sh takes as DEQP_WORK with DEQP_MODULE=gles3. A failed build
# removes the directory, so its presence means a complete tree.
set -eu
REPO=$(cd "$(dirname "$0")/../.." && pwd)
DEPS=${CONNECTED_DEPS_DIR:-${CCFARM_DEPS_DIR:-}}
[ -n "$DEPS" ] || { echo "pi4-gles3-stage1: set CONNECTED_DEPS_DIR" >&2; exit 2; }
WORK=$DEPS/deqp-stage1-rpi_4b-gles3
[ -d "$DEPS/VK-GL-CTS/.git" ] || { echo "pi4-gles3-stage1: $DEPS/VK-GL-CTS missing (deps-env.sh --build --board rpi_4b DEQP_WORK)" >&2; exit 2; }
[ ! -e "$WORK" ] || { echo "pi4-gles3-stage1: $WORK exists" >&2; exit 0; }
trap 'rc=$?; [ "$rc" -eq 0 ] || rm -rf "$WORK"' EXIT
. "$DEPS/zephyr-ws/.venv/bin/activate"
export ZEPHYR_BASE="$DEPS/zephyr-ws/zephyr"
export ZEPHYR_SDK_INSTALL_DIR="$DEPS/zephyr-sdk-1.0.1" ZEPHYR_TOOLCHAIN_VARIANT=zephyr
ZEPHYR_MODULES="$("$REPO/tools/cts/zephyr-modules.sh" "$DEPS/zephyr-ws")"
export ZEPHYR_MODULES
export CFLAGS=-O2 CXXFLAGS=-O2 CMAKE_POLICY_VERSION_MINIMUM=3.5
cd "$REPO"
CTS="$DEPS/VK-GL-CTS" WORK="$WORK" BOARD=rpi_4b MODULE=gles3 tools/cts/cts-build-zephyr.sh
echo "pi4-gles3-stage1: $WORK"
