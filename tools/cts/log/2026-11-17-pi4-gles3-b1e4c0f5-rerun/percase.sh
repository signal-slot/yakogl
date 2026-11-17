#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# One dEQP process per case, for the cases of a list: the deqp-gles3 and the
# V3D library that tools/cts/host-deqp-run.sh built (GPU=v3d MODULE=gles3),
# run with its own flags (run_chunk). Why: the V3D library logs a draw
# refusal once per cause and process (src/v3d/backend.rs count_refusal), so
# in a 100-case chunk only the first case of a cause shows it; alone, every
# case does. HOST-ONLY, as the run it repeats.
#
#   percase.sh <case list> <out dir> [jobs]
#
# Writes <out dir>/<n>.out (dEQP's stdout and the library's stderr) and
# <n>.qpa per case, in the c_*.out / c_*.qpa form rerun-map.py reads.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/../../../.." && pwd)
BASE=$REPO/build/host-deqp
LIST=$(realpath "$1"); OUT=$2; JOBS=${3:-4}
mkdir -p "$OUT"; OUT=$(realpath "$OUT")
export YAKOGL_V3D_TILED_COPIES_BYTES=${YAKOGL_V3D_TILED_COPIES_BYTES:-16777216}
export BASE OUT
one() {
    local n=$1 c=$2
    echo "$c" > "$OUT/c_$n"
    (cd "$BASE/deqp-gles3/modules/gles3" &&
     LD_LIBRARY_PATH="$BASE/lib-v3d-es3" timeout 300 ./deqp-gles3 --deqp-case="$c" \
        --deqp-log-filename="$OUT/c_$n.qpa" --deqp-log-shader-sources=enable --deqp-log-images=disable \
        --deqp-surface-type=pbuffer --deqp-surface-width=256 --deqp-surface-height=256 \
        > "$OUT/c_$n.out" 2>&1) || true
}
export -f one
nl -nrz -w5 "$LIST" | xargs -P "$JOBS" -L1 bash -c 'one "$0" "$1"'
echo "percase: $(ls "$OUT"/c_*.qpa | wc -l) of $(wc -l < "$LIST") cases logged in $OUT"
