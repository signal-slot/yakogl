#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The common set on a board, booted from its bootloader: the four images of
# tools/cts/common-tree.txt (glmark2, slint with the view off, slint with the
# view on, gl_test), the first thing every board runs (driver/docs/one-method.md).
#
#   tools/cts/cts-common.sh [--dry-run] <board> build <img-dir> [LABEL...]
#   tools/cts/cts-common.sh [--dry-run] <board> run <img-dir> <outdir> [<earlier outdir>]
#   tools/cts/cts-common.sh <board> tally <outdir> [<earlier outdir>]
#
# <board>  am62p | pi4 | imx8mp, as tools/board.sh names them. The boot is
#          board.sh's default for the board (am62p uboot-tftp, pi4 tryboot,
#          imx8mp uboot-tftp); BOARD_BOOT=kexec-dev picks the development
#          route (the i.MX8MP's crash route), whose runs are never rows.
# build    makes the four images into <img-dir>/<LABEL>/ with
#          tools/cts/common-images.sh (its header lists the environment:
#          ZEPHYR_WS, GLMARK2_SRC, SLINT_NOSTD_DIR, ...); <LABEL>.built says
#          each image's rc, sha256 and build ids. --dry-run lists the builds.
# run      runs the images through `tools/board.sh <board> tree` (resumable: a
#          row with a finished run of the same image is skipped; BOARD_HOST and
#          the rest from the environment, tools/board.sh's header), then the
#          tally below over <outdir>. --dry-run is board.sh's: every command
#          printed, nothing done, no tally.
# tally    tools/cts/common-tally.py over <outdir>: per row the record words,
#          board.sh's verdict (recomputed from the files), the glmark2 score
#          and its failed / unsupported scenes, the slint probes, gl_test's
#          passed of run and the failing cases, the GPU resets and the image's
#          sha256 and build ids. With <earlier outdir> (an earlier common run)
#          it adds the row-by-row diff.
#
# This script only names the table, the builder and the tally; running is the
# full-tree runner's method (board.sh tree) with a four-row table, not a second
# runner. board.sh's own dEQP diff (TREE_PREV) is not used: these rows carry
# no dEQP cases, and common-tally.py's diff compares what they do carry.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TABLE=$HERE/common-tree.txt
die() { echo "cts-common: $*" >&2; exit 2; }
usage="usage: $0 [--dry-run] <am62p|pi4|imx8mp> build <img-dir> [LABEL...] | run <img-dir> <outdir> [<earlier outdir>] | tally <outdir> [<earlier outdir>]"

DRY=()
[ "${1:-}" = --dry-run ] && { DRY=(--dry-run); shift; }
[ $# -ge 3 ] || die "$usage"
board=$1 verb=$2
case "$board" in am62p | pi4 | imx8mp) ;; *) die "board must be am62p, pi4 or imx8mp, not '$board'" ;; esac
[ -f "$TABLE" ] || die "no $TABLE"

tally() { # tally OUTDIR [PREV]
    local prev=()
    [ -n "${2:-}" ] && prev=(--prev "$2")
    python3 "$HERE/common-tally.py" "$board" "$1" "${prev[@]}"
}

case "$verb" in
build)
    img=$3
    shift 3
    exec "$HERE/common-images.sh" "${DRY[@]}" "$board" "$TABLE" "$img" "$@"
    ;;
run)
    { [ $# -ge 4 ] && [ $# -le 5 ]; } || die "$usage"
    img=$3 out=$4 prev=${5:-}
    [ -z "$prev" ] || [ -d "$prev" ] || die "no earlier outdir $prev"
    if [ ${#DRY[@]} -eq 0 ]; then
        for l in $(awk '/^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); print l }' "$TABLE"); do
            [ -f "$img/$l.built" ] || die "no $img/$l.built: run '$0 $board build $img' first"
        done
    fi
    env -u TREE_PREV -u TREE_IMAGE TREE_IMAGE_DIR="$img" "$REPO/tools/board.sh" "${DRY[@]}" "$board" tree "$TABLE" "$out"
    rc=$?
    [ ${#DRY[@]} -gt 0 ] && exit "$rc"
    echo "cts-common: board.sh tree rc=$rc"
    tally "$out" "$prev"
    trc=$?
    [ "$rc" = 0 ] || exit "$rc"
    exit "$trc"
    ;;
tally)
    [ ${#DRY[@]} -eq 0 ] || die "tally has no --dry-run (it only reads files)"
    [ $# -le 4 ] || die "$usage"
    tally "$3" "${4:-}"
    ;;
*) die "$usage" ;;
esac
