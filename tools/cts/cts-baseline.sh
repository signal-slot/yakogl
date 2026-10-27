#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The dEQP-GLES2 baseline on a board, booted from its bootloader: the five
# groups of tools/cts/baseline-tree.txt (info, prerequisite, negative_api,
# state_query, shaders.functions), one image per group. The successor of the
# retired Linux cts-baseline.sh (tools/cts/README.md, the retired-script table).
#
#   tools/cts/cts-baseline.sh [--dry-run] <board> build <img-dir>
#   tools/cts/cts-baseline.sh [--dry-run] <board> run <img-dir> <outdir> [<earlier outdir>]
#
# <board>  am62p | pi4 | imx8mp, as tools/board.sh names them.
# build    makes the five images into <img-dir> with the board's own builder:
#            am62p   am62p-deqp-images.sh: <img-dir>/<LABEL>/ (BI BP BN BQ BF),
#                    the verdicts in <img-dir>/<LABEL>.built and build.out
#            pi4     pi4-deqp-images.sh: <img-dir>/<LABEL>/ and <LABEL>.built
#            imx8mp  imx8mp-deqp-build.sh --table: one image,
#                    <img-dir>/baseline/zephyr/zephyr.bin, the rows by selector
#          Each builder takes its usual environment (ZEPHYR_WS and DEQP_WORK for
#          am62p and pi4; imx8mp-env.sh and DEQP_STAGE1 for the i.MX8MP).
#          --dry-run lists the images and builds nothing.
# run      runs the images through `tools/board.sh <board> tree` (resumable;
#          BOARD_HOST and the rest from the environment, tools/board.sh's
#          header), which tallies each row from its record (console or SNVS):
#          passed, failed, not supported per group, over the mustpass cases
#          the five filters select (the 16,258 others are "outside, not run").
#          With <earlier outdir> (an earlier baseline's, or a whole tree's)
#          it adds the per-case diff over the same scope (TREE_PREV).
#          --dry-run is tools/board.sh's: every command printed, nothing done.
#
# The tallies, the diff and the per-row verdict lines are tools/board.sh's own
# output (tree, tally, diff); this script only names the table, the builder and
# the image layout per board, so the baseline is the full-tree runner's method
# with a five-row table, not a second runner.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TABLE=$HERE/baseline-tree.txt
die() { echo "cts-baseline: $*" >&2; exit 2; }
usage="usage: $0 [--dry-run] <am62p|pi4|imx8mp> build <img-dir> | run <img-dir> <outdir> [<earlier outdir>]"

DRY=()
[ "${1:-}" = --dry-run ] && { DRY=(--dry-run); shift; }
[ $# -ge 3 ] || die "$usage"
board=$1 verb=$2 img=$3
case "$board" in am62p | pi4 | imx8mp) ;; *) die "board must be am62p, pi4 or imx8mp, not '$board'" ;; esac
[ -f "$TABLE" ] || die "no $TABLE"

case "$verb" in
build)
    if [ ${#DRY[@]} -gt 0 ]; then
        case "$board" in
        am62p) exec "$HERE/am62p-deqp-images.sh" --dry-run "$TABLE" "$img" ;;
        pi4) grep -E '^# image [A-Za-z0-9]+:' "$TABLE" | sed 's/^# image //' ;;
        imx8mp) echo "IMX8MP_OUT=$img $HERE/imx8mp-deqp-build.sh baseline 'dEQP-GLES2.info.*' --table $TABLE" ;;
        esac
        exit 0
    fi
    mkdir -p "$img"
    case "$board" in
    am62p) "$HERE/am62p-deqp-images.sh" "$TABLE" "$img" | tee "$img/build.out" ;;
    pi4) "$HERE/pi4-deqp-images.sh" "$TABLE" "$img" ;;
    imx8mp) IMX8MP_OUT=$img "$HERE/imx8mp-deqp-build.sh" baseline 'dEQP-GLES2.info.*' --table "$TABLE" ;;
    esac
    ;;
run)
    [ $# -ge 4 ] || die "$usage"
    out=$4 prev=${5:-}
    [ -z "$prev" ] || [ -d "$prev" ] || die "no earlier outdir $prev"
    env_prev=()
    [ -n "$prev" ] && env_prev=("TREE_PREV=$prev")
    if [ "$board" = imx8mp ]; then
        image=$img/baseline/zephyr/zephyr.bin
        [ -f "$image" ] || die "no $image: run '$0 imx8mp build $img' first"
        exec env TREE_IMAGE="$image" "${env_prev[@]}" "$REPO/tools/board.sh" "${DRY[@]}" imx8mp tree "$TABLE" "$out"
    fi
    exec env TREE_IMAGE_DIR="$img" "${env_prev[@]}" "$REPO/tools/board.sh" "${DRY[@]}" "$board" tree "$TABLE" "$out"
    ;;
*) die "$usage" ;;
esac
