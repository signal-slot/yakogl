#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# deps-env.sh -- the project's dependency layout, as environment lines.
#
# Everything the gate and the board tools need that is not in the repository
# lives in one directory per host, in ccfarm's layout for this project:
# $CCFARM_DEPS_DIR when ccfarm set it (a farm agent's turn), else
# ~/.local/share/ccfarm/deps/yakogl. A host that keeps a tree elsewhere links
# it into that directory under the name below. This script is the one place
# that names the entries; .github/workflows/gate.yml and the board recipes
# read them from here, so a renamed entry changes one file.
#
#   tools/cts/deps-env.sh [board]
#     Print NAME=value lines for `$GITHUB_ENV` or `export $(...)`: the gate's
#     entries, and with a board (am62p | imx8mp | rpi_4b) that board's too
#     (its dEQP stage-1 tree as DEQP_WORK; for the i.MX8MP also IMX8MP_OUT
#     and DEQP_STAGE1).
#   tools/cts/deps-env.sh --check [board]
#     One "ok" or "missing" line per entry; exit 1 when an entry is
#     missing. Each entry is checked by a file that only a usable tree has
#     (the marker shown on the line), so an empty directory or a venv
#     without west is "missing", not "ok". A missing entry is reported,
#     never skipped: a gate run without it would be a gap, not a pass.
#
#   entry                      name                    what
#   zephyr-ws/                 ZEPHYR_WS, ZEPHYR_WORKSPACE  the west workspace (Zephyr, hal_nxp, yakogl)
#   zephyr-ws/.venv/           WEST_VENV, ZEPHYR_VENV  the venv holding west
#   zephyr-ws/modules/hal/nxp/ HAL_NXP_DIR             hal_nxp (the i.MX8MP register check)
#   zephyr-sdk-1.0.1/          ZEPHYR_SDK_INSTALL_DIR  the Zephyr SDK
#   PCO_SHC/pco_shc            PCO_SHC                 Mesa's PowerVR shader compiler
#   glsl2usc-cache/            GLSL2USC_CACHE          its compile cache (created when absent)
#   glmark2-src/               GLMARK2_SRC             glmark2 at the pinned commit
#   pvr_fw_blob.h              PVR_FW_BLOB             the PowerVR firmware header (never committed)
#   with a board:
#   deqp-stage1-<board>/       DEQP_WORK               the board's dEQP stage-1 tree
#   imx8mp-out/                IMX8MP_OUT              i.MX8MP image output (imx8mp)
#   deqp-stage1-imx8mp/        DEQP_STAGE1             the i.MX8MP dEQP stage-1 tree (imx8mp)
#
# GLSLANG is glslangValidator on PATH (a distro package, tools/setup-gate-host.sh).
set -euo pipefail

CHECK=0
[ "${1-}" = "--check" ] && { CHECK=1; shift; }
BOARD="${1-}"
case "$BOARD" in
  "" | am62p | imx8mp | rpi_4b) ;;
  *) echo "deps-env.sh: unknown board '$BOARD' (am62p | imx8mp | rpi_4b)" >&2; exit 2 ;;
esac

deps="${CCFARM_DEPS_DIR:-$HOME/.local/share/ccfarm/deps/yakogl}"
glslang="$(command -v glslangValidator || echo /usr/bin/glslangValidator)"

missing=0
# row <NAME> <path> <marker> [more NAMEs with the same value...]
#   prints NAME=path (and the same value under the extra names); with --check
#   tests <marker> (a path under or equal to <path>) instead.
row() {
  local name=$1 path=$2 marker=$3
  shift 3
  if [ "$CHECK" -eq 1 ]; then
    if [ -e "$marker" ]; then
      echo "ok       $name  $marker"
    else
      echo "missing  $name  $marker"
      missing=$((missing + 1))
    fi
    return
  fi
  echo "$name=$path"
  for name in "$@"; do echo "$name=$path"; done
}

[ -d "$deps" ] || { echo "deps-env.sh: no dependency layout at $deps on $(hostname)" >&2; exit 1; }
[ "$CHECK" -eq 1 ] && echo "layout   $deps"

row ZEPHYR_WS "$deps/zephyr-ws" "$deps/zephyr-ws/.west/config" ZEPHYR_WORKSPACE
row WEST_VENV "$deps/zephyr-ws/.venv" "$deps/zephyr-ws/.venv/bin/west" ZEPHYR_VENV
row HAL_NXP_DIR "$deps/zephyr-ws/modules/hal/nxp" "$deps/zephyr-ws/modules/hal/nxp/zephyr/module.yml"
row ZEPHYR_SDK_INSTALL_DIR "$deps/zephyr-sdk-1.0.1" "$deps/zephyr-sdk-1.0.1/sdk_version"
row PCO_SHC "$deps/PCO_SHC/pco_shc" "$deps/PCO_SHC/pco_shc"
row GLSLANG "$glslang" "$glslang"
[ "$CHECK" -eq 1 ] || mkdir -p "$deps/glsl2usc-cache"
row GLSL2USC_CACHE "$deps/glsl2usc-cache" "$deps"
row GLMARK2_SRC "$deps/glmark2-src" "$deps/glmark2-src/src/default-benchmarks.h"
row PVR_FW_BLOB "$deps/pvr_fw_blob.h" "$deps/pvr_fw_blob.h"
if [ -n "$BOARD" ]; then
  row DEQP_WORK "$deps/deqp-stage1-$BOARD" "$deps/deqp-stage1-$BOARD/build"
fi
if [ "$BOARD" = imx8mp ]; then
  row IMX8MP_OUT "$deps/imx8mp-out" "$deps/imx8mp-out"
  row DEQP_STAGE1 "$deps/deqp-stage1-imx8mp" "$deps/deqp-stage1-imx8mp/build"
fi

if [ "$CHECK" -eq 1 ] && [ "$missing" -gt 0 ]; then
  echo "deps-env.sh: $missing entr$([ "$missing" -eq 1 ] && echo y || echo ies) missing in $deps"
  exit 1
fi
