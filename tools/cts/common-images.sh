#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The common set's images for one board: one Zephyr image per row of
# tools/cts/common-tree.txt, from this checkout as it is.
#
#   tools/cts/common-images.sh [--dry-run] [--cases LIST] [--pipe] <am62p|pi4|imx8mp> <table> <img-dir> [LABEL...]
#
# <table>    common-tree.txt's form: "# image <LABEL>: ..." and then one line
#            naming what to build (glmark2, gl_test).
# <img-dir>  one west build directory per row, <img-dir>/<LABEL>/, so the
#            image is <img-dir>/<LABEL>/zephyr/zephyr.bin: the layout
#            tools/board.sh tree reads with TREE_IMAGE_DIR=<img-dir>. A build
#            directory is reused when it is complete: it holds the marker
#            <LABEL>/.complete, which this script writes last, with this
#            checkout's HEAD and a hash of the row's cmake arguments (so of
#            --cases and --pipe), and the image's sha256 in it still matches
#            (the row is reported again from the marker, nothing is built). A
#            directory without the marker is an interrupted build and is
#            rebuilt from scratch. A complete one from another commit or built
#            with other arguments is refused (rc=exists), never reused: use
#            another <img-dir>. After each image
#            the line "<LABEL> rc=<rc> sha256=<hash or none> build=<ids>" goes
#            to <img-dir>/<LABEL>.built, the file board.sh tree waits for, so a
#            tree can start while later images still build.
# LABEL...   build only these rows (all when none).
# --cases LIST  the gl_test rows run just these cases (comma-separated case
#            numbers of driver/tools/pvrlab/gl_test.c, in order, on one
#            context) instead of the sample's default list: am62p
#            CONFIG_GL_TEST_CASES=LIST; pi4 CONFIG_V3D_GL_TEST_CASE=N for one
#            case (a fresh context), else CONFIG_V3D_GL_TEST_CASES=LIST. The
#            i.MX8MP's gl_test has no list option: refused there. Other
#            rows (glmark2) are unchanged.
# --pipe     compile with gcc's -pipe (-DEXTRA_CFLAGS=-pipe) on every row
# --dry-run  print "<LABEL> <sample> <cmake arguments>" per row and build nothing
#            (needs none of the environment below). For a row whose build
#            directory exists it adds "<LABEL>: existing <dir>: reuse|other|
#            interrupted", what a real run would do with it.
#
# What each row builds, per board (the board names are the Zephyr board names
# west takes):
#   glmark2     driver/samples/zephyr/glmark2 (-DGLMARK2_SRC)
#   gl_test     driver/samples/zephyr/gl_test (the board's configuration of the
#               one sample) with its default case list
# and every image of a board gets that board's watchdog self-arm, because
# tools/board.sh refuses an image without its arm line:
#   am62p   CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y, CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n
#           ("rti_arm: armed"; the board layer implies the first, but gl_test's
#           board .conf turns the inherited feeder on, which the self-arm excludes)
#   imx8mp  CONFIG_GC_BOARD_WDOG_SELF_ARM=y ("gc_wdog_arm: armed"; selected by
#           the board layer, not by gl_test)
#   pi4     nothing ("board: PM watchdog resets", rpi_wdog.c, always built)
# An image that has neither its board's arm line nor the board layer's
# ("board: watchdog armed", tools/board.sh NEW_MARKER) is reported rc=no_marker:
# board.sh would refuse it at the board. On the AM62P and the Pi 4 an image
# without the printf format of a build line ("<name>: build ...%s") is reported
# rc=no_build_line: its console would not say which build ran.
#
# Environment (nothing defaults to a path on one machine):
#   ZEPHYR_WS        the west workspace ($ZEPHYR_WS/zephyr is ZEPHYR_BASE)
#   WEST_VENV        optional: a venv whose bin/ holds west
#   ZEPHYR_SDK_INSTALL_DIR, RUSTUP_TOOLCHAIN as for any sample with Rust in it
#   GLMARK2_SRC      the glmark2 checkout the glmark2 sample pins (its CMakeLists)
#   am62p: driver/src/fw/pvr_fw_blob.h in this checkout (driver/tools/mkfwblob.sh,
#          never committed)
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
die() { echo "common-images: $*" >&2; exit 2; }
DRY=0 CASES= PIPE=0
while [ $# -gt 0 ]; do
    case "$1" in
    --dry-run) DRY=1; shift ;;
    --pipe) PIPE=1; shift ;;
    --cases) [ $# -ge 2 ] || die "--cases needs a list"; CASES=$2; shift 2 ;;
    --cases=*) CASES=${1#--cases=}; shift ;;
    -*) die "unknown option $1" ;;
    *) break ;;
    esac
done
[ -z "$CASES" ] || [[ $CASES =~ ^[0-9]+(,[0-9]+)*$ ]] || die "--cases takes comma-separated case numbers, not '$CASES'"
[ $# -ge 3 ] || die "usage: $0 [--dry-run] [--cases LIST] [--pipe] <am62p|pi4|imx8mp> <table> <img-dir> [LABEL...]"
BOARD=$1 TABLE=$2 OUT=$3
shift 3
case "$BOARD" in
am62p)
    ZBOARD=verdin_am62p/am62p54/a53
    ARM=(-DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n)
    MARKER='rti_arm: armed' ;;
imx8mp)
    ZBOARD=verdin_imx8mp/mimx8ml8/a53
    ARM=(-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y)
    MARKER='gc_wdog_arm: armed' ;;
pi4)
    ZBOARD=rpi_4b
    ARM=()
    MARKER='board: PM watchdog resets' ;;
*) die "board must be am62p, pi4 or imx8mp, not '$BOARD'" ;;
esac
[ -z "$CASES" ] || [ "$BOARD" != imx8mp ] || die "--cases: gl_test has no case-list option (the i.MX8MP)"
NEW_MARKER='board: watchdog armed'
# the printf format of the build line the image prints first ("sample: build
# tree=%s ..." from the board layer's sample_glue.c, "v3d_gl_test: build %s", a log prefix the Pi 4 sample keeps),
# the line tools/board.sh's boot_build reads from the console; the i.MX8MP has
# no UART, so no line could reach a record there and the check is skipped
BUILD_FMT='[A-Za-z0-9_]+: build (tree=)?%s'
[ -f "$TABLE" ] || die "no table $TABLE"

# rows: "<LABEL>\t<what>" per row, in file order (board.sh tree_rows' label rule;
# a row without a label is refused: board.sh would name it img<N>-<tag>)
rows() {
    awk '/^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); next }
         /^[[:space:]]*(#|$)/ { next }
         { if (l == "") { print "common-images: a row without a label: " $0 > "/dev/stderr"; exit 1 }
           print l "\t" $1; l = "" }' "$TABLE"
}
selected() {
    local want=" $* " l w
    rows | while IFS=$'\t' read -r l w; do
        { [ $# -eq 0 ] || [[ $want == *" $l "* ]]; } && printf '%s\t%s\n' "$l" "$w"
    done
}
# row_args WHAT: sets SAMPLE and ARGS (the row's cmake arguments, the board's arm included)
row_args() {
    ARGS=("-DYAKOGL_DIR=$REPO" "${ARM[@]}")
    if [ "$PIPE" = 1 ]; then ARGS+=("-DEXTRA_CFLAGS=-pipe"); fi
    case "$1" in
    glmark2) SAMPLE=glmark2; ARGS+=("-DGLMARK2_SRC=${GLMARK2_SRC:-<GLMARK2_SRC>}") ;;
    gl_test) SAMPLE=gl_test
        if [ -n "$CASES" ]; then
            case "$BOARD" in
            am62p) ARGS+=("-DCONFIG_GL_TEST_CASES=\"$CASES\"") ;;
            pi4) if [[ $CASES == *,* ]]; then ARGS+=("-DCONFIG_V3D_GL_TEST_CASES=\"$CASES\"")
                 else ARGS+=("-DCONFIG_V3D_GL_TEST_CASE=$CASES"); fi ;;
            esac
        fi ;;
    *) return 1 ;;
    esac
    return 0
}

HEAD_ID=$(git -C "$REPO" rev-parse HEAD 2>/dev/null || echo unknown)
# args_key: a hash of the row's cmake arguments (ARGS, set by row_args), so a
# build directory made with other arguments (another --cases, --pipe) is told apart
args_key() { printf '%s\n' "${ARGS[@]}" | sha256sum | cut -d' ' -f1; }
# marker_field NAME DIR: a field of DIR/.complete, "head=<HEAD> args=<key> sha256=<hash> build=<ids>"
marker_field() { tr ' ' '\n' < "$2/.complete" 2> /dev/null | sed -n "s/^$1=//p"; }
# existing_state DIR: interrupted (no .complete), reuse (complete at this HEAD
# with this row's arguments, and the image still matches its hash) or other.
# Needs row_args to have run for the row.
existing_state() {
    if [ ! -s "$1/.complete" ]; then echo interrupted
    elif [ "$(marker_field head "$1")" = "$HEAD_ID" ] && [ "$HEAD_ID" != unknown ] &&
        [ "$(marker_field args "$1")" = "$(args_key)" ] && [ -s "$1/zephyr/zephyr.bin" ] &&
        [ "$(sha256sum "$1/zephyr/zephyr.bin" | cut -d' ' -f1)" = "$(marker_field sha256 "$1")" ]; then echo reuse
    else echo other; fi
}

rows > /dev/null || exit 2
if [ "$DRY" = 1 ]; then
    selected "$@" | while IFS=$'\t' read -r l w; do
        row_args "$w" || { echo "$l: '$w' is not glmark2 or gl_test"; continue; }
        echo "$l $SAMPLE -b $ZBOARD ${ARGS[*]}"
        # an existing build directory: what a real run would do with it
        [ ! -e "$OUT/$l" ] || echo "$l: existing $OUT/$l: $(existing_state "$OUT/$l")"
    done
    exit 0
fi

: "${ZEPHYR_WS:?set ZEPHYR_WS to the west workspace (it holds zephyr/)}"
[ -d "$ZEPHYR_WS/zephyr" ] || die "$ZEPHYR_WS/zephyr does not exist"
[ -n "${WEST_VENV:-}" ] && PATH=$WEST_VENV/bin:$PATH
command -v west > /dev/null || die "west not on PATH (set WEST_VENV)"
if [ "$BOARD" = am62p ]; then
    [ -f "$REPO/driver/src/fw/pvr_fw_blob.h" ] ||
        die "driver/src/fw/pvr_fw_blob.h missing: generate it with driver/tools/mkfwblob.sh (never commit it)"
fi
# each input a selected row needs, checked before the first build
need=$(selected "$@" | cut -f2 | sort -u)
if grep -qx glmark2 <<<"$need" && [ ! -f "${GLMARK2_SRC:-}/src/default-benchmarks.h" ]; then
    die "set GLMARK2_SRC to the glmark2 checkout (driver/samples/zephyr/glmark2/CMakeLists.txt)"
fi
export ZEPHYR_BASE=$ZEPHYR_WS/zephyr
MODS=$("$HERE/zephyr-modules.sh" "$ZEPHYR_WS") || die "zephyr-modules.sh failed"
mkdir -p "$OUT" || die "cannot create $OUT"
OUT=$(cd "$OUT" && pwd)
echo "common-images: build $(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown) ($(git -C "$REPO" rev-parse HEAD 2>/dev/null || echo unknown)) board $BOARD"

fails=0
while IFS=$'\t' read -r l w; do
    B=$OUT/$l
    if ! row_args "$w"; then
        echo "$l rc=unknown_row sha256=none build=none" | tee "$OUT/$l.built"
        fails=$((fails + 1)); continue
    fi
    if [ -e "$B" ]; then
        case $(existing_state "$B") in
        interrupted)
            echo "common-images: $l: $B is an interrupted build (no .complete), rebuilt"
            rm -rf "$B" "$B.log" "$OUT/$l.built" ;;
        reuse)
            echo "common-images: $l: $B is complete at this commit with these arguments, reused"
            echo "$l rc=0 sha256=$(marker_field sha256 "$B") build=$(marker_field build "$B")" | tee "$OUT/$l.built"
            continue ;;
        *)
            echo "$l: $B exists from another commit or with other arguments, or changed since, not reused" >&2
            echo "$l rc=exists sha256=none build=none" | tee "$OUT/$l.built"
            fails=$((fails + 1)); continue ;;
        esac
    fi
    (cd "$ZEPHYR_WS" && west build -p always -b "$ZBOARD" -d "$B" "$REPO/driver/samples/zephyr/$SAMPLE" -- \
        "-DZEPHYR_MODULES=$MODS" "${ARGS[@]}") > "$B.log" 2>&1 < /dev/null
    rc=$? sha=none ids=none
    bin=$B/zephyr/zephyr.bin
    if [ "$rc" = 0 ] && [ -s "$bin" ]; then
        sha=$(sha256sum "$bin" | cut -d' ' -f1)
        # the build ids board.sh's image_build reads from the same build.ninja
        ids=$(grep -oE '(YAKOGL_SAMPLE_TREE|YAKOGL_SAMPLE_TABLE|V3D_BUILD|GC_BUILD_STAMP)=\\"[^\\"]*' "$B/build.ninja" |
            sed 's/\\"//' | sort -u | paste -sd, -)
        if ! grep -aqF "$MARKER" "$bin" && ! grep -aqF "$NEW_MARKER" "$bin"; then rc=no_marker
        elif [ "$BOARD" != imx8mp ] && ! grep -aqE "$BUILD_FMT" "$bin"; then rc=no_build_line; fi
    elif [ "$rc" = 0 ]; then
        rc=no_image
    fi
    if [ "$rc" = 0 ]; then
        echo "head=$HEAD_ID args=$(args_key) sha256=$sha build=${ids:-none}" > "$B/.complete"
    else
        fails=$((fails + 1))
    fi
    echo "$l rc=$rc sha256=$sha build=${ids:-none}" | tee "$OUT/$l.built"
done < <(selected "$@")
echo "common-images: $fails row(s) not built"
[ "$fails" = 0 ]
