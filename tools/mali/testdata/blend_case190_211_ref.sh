#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The reference run of blend_case190_211_ref.py on Mesa 26.2.1's drm-shim, as
# tools/mali/drm_shim_ref.sh runs gl_ref.py (the same environment).
#   blend_case190_211_ref.sh <work dir of drm_shim_ref.sh build> 190|211 <out prefix>
# keeps <out prefix>.trace (pandecode), .stdout and .stderr.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
INST="$(cd "$1" && pwd)/inst/lib64"
CASE=$2 OUT=$3
rm -f "$OUT".trace*
LD_LIBRARY_PATH=$INST LD_PRELOAD=$INST/libpanfrost_noop_drm_shim.so \
  PAN_GPU_ID=a867 MESA_LOADER_DRIVER_OVERRIDE=panthor MESA_SHADER_CACHE_DISABLE=true \
  BIFROST_MESA_DEBUG=shaders,shaderdb PAN_MESA_DEBUG=trace PANDECODE_DUMP_FILE="$OUT.trace" \
  stdbuf -o0 python3 "$HERE/blend_case190_211_ref.py" "$INST" "$CASE" > "$OUT.stdout" 2> "$OUT.stderr"
cat "$OUT".trace.* > "$OUT.trace"
rm -f "$OUT".trace.*
