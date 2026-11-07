#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/x2-glmark2-scenes.sh [--dry-run] [--commit <rev>] <outdir> <scene>...
#
# Per-scene glmark2 PERF runs on the Armadillo-X2 (kexec-dev, a development
# run) in one command. For each scene: one glmark2 image for
# armadillo_x2/mimx8ml8/a53 with CONFIG_GLMARK2_ARGS="-b <scene>", run through
# `tools/board.sh x2 run` once without a selector and once per PERF page
# selector (SEL=0x1000, 0x1001, 0x1003, and 0x1004 when the sample's
# gm_perf_page.h has a page 4). The record words are decoded with
# `yakogl-tools record --words 3` (the X2 stores w0..w2 only, so a page shows
# v0..v3). One table at the end, one row a run:
#
#   scene  commit  selector  verdict line  record words  decoded  ms/frame
#
# ms/frame is page 0's v1 (the frame, t2 - t0, in us) / 1000; '-' elsewhere.
# These are X2 development numbers: they go into commit messages, docs/notes/
# and issue comments, never into tools/cts/RESULTS.md, STATUS.md or README.md.
#
# Refusals (exit 3, before any build or board step):
#   - HEAD (or --commit <rev>) is not an ancestor of origin/main;
#   - tracked changes in the work tree when building HEAD;
#   - an image without the watchdog marker 'gc_wdog_arm: armed'.
# NETBOOT_NO_WATCHDOG and BOARD_UART are unset for every board.sh call; board
# addresses (BOARD_HOST) come from the environment, never from here. The first
# run whose board does not come back (board.sh rc 5, or rc 6) stops the whole
# command (exit 5), without a retry: the board needs a person. A refused run
# (rc 3) stops it too. Any other not-done verdict (rc 4) is a row and the next
# run goes on.
#
# --dry-run: builds the images, then passes --dry-run to board.sh, which
# prints what it would do and touches nothing.
#
# Environment (all but ZEPHYR_WS and GLMARK2_SRC have a default):
#   ZEPHYR_WS        the west workspace (required)
#   GLMARK2_SRC      the glmark2 checkout of the sample (required)
#   BOARD_HOST       the X2 (read by board.sh)
#   X2SC_BOARD_SH    board.sh (default tools/board.sh of the checkout)
#   X2SC_WEST        west
#   X2SC_YAKOGL_TOOLS  the yakogl-tools binary (default YAKOGL_TOOLS, else
#                    built with cargo from the checkout)
#   X2SC_MODULES_SH  zephyr-modules.sh (default: the one beside this script)
#   X2SC_REPO        the checkout (default: this script's)
#   X2SC_MAIN_REF    the ref HEAD must be on (default origin/main)
#
# Row to add to tools/check.sh later (not added here: another plan owns it):
#   tools/cts/x2-glmark2-scenes-selftest.sh, in the host-script rows with the
#   other tools/cts/*-selftest.sh.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=${X2SC_REPO:-$(cd "$HERE/../.." && pwd)}
MAIN_REF=${X2SC_MAIN_REF:-origin/main}
BOARD_SH=${X2SC_BOARD_SH:-$REPO/tools/board.sh}
WEST=${X2SC_WEST:-west}
ZBOARD=armadillo_x2/mimx8ml8/a53
MARKER='gc_wdog_arm: armed'

die() { echo "x2-glmark2-scenes: $1" >&2; exit "${2:-2}"; }

DRY=0 COMMIT=HEAD
while [ $# -gt 0 ]; do
    case $1 in
    --dry-run) DRY=1; shift ;;
    --commit) [ $# -ge 2 ] || die "--commit needs a revision"; COMMIT=$2; shift 2 ;;
    -h | --help) sed -n '5,50p' "$0"; exit 0 ;;
    --) shift; break ;;
    -*) die "unknown option $1" ;;
    *) break ;;
    esac
done
[ $# -ge 2 ] || die "usage: x2-glmark2-scenes.sh [--dry-run] [--commit <rev>] <outdir> <scene>..."
OUT=$1
shift
SCENES=("$@")
for s in "${SCENES[@]}"; do
    [[ $s =~ ^[A-Za-z0-9_:=.-]+$ ]] || die "scene '$s': letters, digits and _:=.- only"
done
[ -n "${ZEPHYR_WS:-}" ] || die "set ZEPHYR_WS to the west workspace"
[ -n "${GLMARK2_SRC:-}" ] || die "set GLMARK2_SRC to the glmark2 checkout"

# ---- the commit: on origin/main, or refused ----------------------------------
SHA=$(git -C "$REPO" rev-parse --verify -q "$COMMIT^{commit}") || die "no commit $COMMIT in $REPO" 3
git -C "$REPO" rev-parse --verify -q "$MAIN_REF^{commit}" > /dev/null ||
    die "no $MAIN_REF in $REPO: cannot show that $COMMIT is on it" 3
git -C "$REPO" merge-base --is-ancestor "$SHA" "$MAIN_REF" ||
    die "refused: $SHA is not an ancestor of $MAIN_REF (only images from a commit on main run on the board)" 3
SHORT=$(git -C "$REPO" rev-parse --short=8 "$SHA")
SRC=$REPO
if [ "$SHA" = "$(git -C "$REPO" rev-parse HEAD)" ]; then
    git -C "$REPO" diff --quiet HEAD -- ||
        die "refused: tracked changes in $REPO; the image would not be the commit $SHORT" 3
fi

mkdir -p "$OUT" || die "cannot create $OUT"
OUT=$(cd "$OUT" && pwd)
if [ "$SHA" != "$(git -C "$REPO" rev-parse HEAD)" ]; then
    SRC=$OUT/tree-$SHORT
    [ -e "$SRC" ] || git -C "$REPO" worktree add --detach -q "$SRC" "$SHA" || die "cannot check out $SHORT"
fi

PAGE4=0
if git -C "$REPO" show "$SHA:driver/samples/zephyr/glmark2/src/gm_perf_page.h" 2> /dev/null |
    grep -Eq '^#define[[:space:]]+GM_PERF_PAGES[[:space:]]+[5-9]'; then PAGE4=1; fi
SELS=("" 0x1000 0x1001 0x1003)
[ "$PAGE4" = 1 ] && SELS+=(0x1004)

ytools() {
    local bin=${X2SC_YAKOGL_TOOLS:-${YAKOGL_TOOLS:-}}
    if [ -z "$bin" ]; then
        (cd "$REPO" && cargo build -q -p yakogl-tools) >&2 || die "cannot build yakogl-tools"
        bin=$(cd "$REPO" && cargo metadata --no-deps --format-version 1 2> /dev/null |
            sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')/debug/yakogl-tools
        [ -x "$bin" ] || die "no yakogl-tools binary at $bin"
        X2SC_YAKOGL_TOOLS=$bin
    fi
    "$bin" "$@"
}

# ---- one image a scene ---------------------------------------------------------
build_image() {
    local scene=$1 b=$OUT/build/$1 mods
    mods=$("${X2SC_MODULES_SH:-$HERE/zephyr-modules.sh}" "$ZEPHYR_WS") || die "zephyr-modules.sh failed"
    mkdir -p "$OUT/build"
    (cd "$ZEPHYR_WS" && "$WEST" build -p always -b "$ZBOARD" -d "$b" "$SRC/driver/samples/zephyr/glmark2" -- \
        "-DZEPHYR_MODULES=$mods" "-DGLMARK2_SRC=$GLMARK2_SRC" \
        "-DCONFIG_GLMARK2_ARGS=\"-b $scene\"") > "$b.log" 2>&1 < /dev/null ||
        die "build of $scene failed (see $b.log)" 4
    [ -s "$b/zephyr/zephyr.bin" ] || die "build of $scene left no $b/zephyr/zephyr.bin" 4
    grep -aqF "$MARKER" "$b/zephyr/zephyr.bin" ||
        die "refused: the image of $scene has no watchdog marker '$MARKER'" 3
    IMG=$b/zephyr/zephyr.bin
}

ROWS=()
STOP=0
row() { ROWS+=("$(printf '%s\t' "$@")"); }

run_one() {
    local scene=$1 sel=$2 name rc line verdict rec words dec ms v1 label
    label=${sel:-none}
    if [ -n "$sel" ]; then name=${scene//[:=]/_}-p$((sel & 0xff)); else name=${scene//[:=]/_}-nosel; fi
    local logdir=$OUT/${scene//[:=]/_}
    mkdir -p "$logdir"
    # no NETBOOT_NO_WATCHDOG, no BOARD_UART, whatever the caller exported
    local -a envc=(env -u NETBOOT_NO_WATCHDOG -u BOARD_UART -u SEL)
    [ -n "$sel" ] && envc+=("SEL=$sel")
    local -a dry=()
    [ "$DRY" = 1 ] && dry=(--dry-run)
    "${envc[@]}" "$BOARD_SH" ${dry[@]+"${dry[@]}"} x2 run "$IMG" "$logdir" "$name" < /dev/null > "$logdir/$name.out"
    rc=$?
    line=$(grep -a ' verdict=' "$logdir/$name.out" | tail -1)
    verdict=$(sed -n 's/.* verdict=\([^ ]*\).*/\1/p' <<< "$line")
    rec=$(sed -n 's/.* record=\([^ ]*\).*/\1/p' <<< "$line")
    words=- dec=- ms=-
    if [ -n "$rec" ] && [ "$rec" != absent ]; then
        words=${rec//,/ }
        # shellcheck disable=SC2086  # three hex words
        dec=$(ytools record --words 3 ${words//0x/} 2> /dev/null | tr '\n' ' ' | sed 's/ *$//')
        [ -n "$dec" ] || dec=-
        if [[ $dec =~ page\ ([0-9]+):\ ([0-9]+)\ ([0-9]+)\ ([0-9]+)\ ([0-9]+) ]]; then
            dec="page ${BASH_REMATCH[1]}: v0=${BASH_REMATCH[2]} v1=${BASH_REMATCH[3]} v2=${BASH_REMATCH[4]} v3=${BASH_REMATCH[5]}"
            if [ "${BASH_REMATCH[1]}" = 0 ] && [ -n "$sel" ]; then
                v1=${BASH_REMATCH[3]}
                ms=$(printf '%d.%03d' $((v1 / 1000)) $((v1 % 1000)))
            fi
        fi
    fi
    [ "$DRY" = 1 ] && [ -z "$line" ] && line="(dry-run, rc=$rc)"
    row "$scene" "$SHORT" "$label" "${line:-no verdict line, rc=$rc}" "$words" "$dec" "$ms"
    case $rc in
    0 | 4) ;;
    *) STOP=$rc ;;
    esac
}

for scene in "${SCENES[@]}"; do
    build_image "$scene"
    for sel in "${SELS[@]}"; do
        run_one "$scene" "$sel"
        [ "$STOP" = 0 ] || break 2
    done
done

# ---- the table -----------------------------------------------------------------
{
    printf 'scene\tcommit\tselector\tverdict line\trecord words\tdecoded\tms/frame\n'
    printf '%s\n' "${ROWS[@]}" | sed 's/\t$//'
} | tee "$OUT/table.tsv"

case $STOP in
0) exit 0 ;;
3) echo "x2-glmark2-scenes: a run was refused, stopped (nothing was retried)" >&2; exit 3 ;;
5 | 6) echo "x2-glmark2-scenes: the board did not come back (rc $STOP): stopped, no retry; it needs a person" >&2; exit 5 ;;
*) echo "x2-glmark2-scenes: board.sh failed (rc $STOP): stopped, no retry" >&2; exit "$STOP" ;;
esac
