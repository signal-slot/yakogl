#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The common set's images for one board: one Zephyr image per row of
# tools/cts/common-tree.txt, from this checkout as it is.
#
#   tools/cts/common-images.sh [--dry-run] <am62p|pi4|imx8mp> <table> <img-dir> [LABEL...]
#
# <table>    common-tree.txt's form: "# image <LABEL>: ..." and then one line
#            naming what to build (glmark2, slint, slint-view, cluster, gl_test).
# <img-dir>  one west build directory per row, <img-dir>/<LABEL>/, so the
#            image is <img-dir>/<LABEL>/zephyr/zephyr.bin: the layout
#            tools/board.sh tree reads with TREE_IMAGE_DIR=<img-dir>. A build
#            directory that exists is refused, never reused. After each image
#            the line "<LABEL> rc=<rc> sha256=<hash or none> build=<ids>" goes
#            to <img-dir>/<LABEL>.built, the file board.sh tree waits for, so a
#            tree can start while later images still build.
# LABEL...   build only these rows (all when none).
# --dry-run  print "<LABEL> <sample> <cmake arguments>" per row and build nothing
#            (needs none of the environment below).
#
# What each row builds, per board (the board names are the Zephyr board names
# west takes):
#   glmark2     driver/samples/zephyr/glmark2 (-DGLMARK2_SRC)
#   slint       driver/samples/zephyr/slint (-DSLINT_NOSTD_DIR, -DNOSTD_PATCH_DIR)
#   slint-view  the same and -DCONFIG_YAKOGL_BOARD_SCANOUT_VIEW=y
#   cluster     driver/samples/zephyr/cluster, the AGL cluster sample
#               (-DAGL_SLINT_CLUSTER_DIR, -DSLINT_NOSTD_DIR, -DNOSTD_PATCH_DIR,
#               -DNOTO_SANS_DIR; the sample's own CMakeLists names each input)
#   gl_test     gl_test (am62p), gc_gl_test (imx8mp), v3d_gl_test (pi4), each
#               with its sample's default case list
# and every image of a board gets that board's watchdog self-arm, because
# tools/board.sh refuses an image without its arm line:
#   am62p   CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y, CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n
#           ("rti_arm: armed"; the board layer implies the first, but gl_test's
#           board .conf turns the inherited feeder on, which the self-arm excludes)
#   imx8mp  CONFIG_GC_BOARD_WDOG_SELF_ARM=y ("gc_wdog_arm: armed"; selected by
#           the board layer, not by gc_gl_test)
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
#   (slint and cluster build their Rust with -Z build-std: a nightly toolchain)
#   GLMARK2_SRC      the glmark2 checkout the glmark2 sample pins (its CMakeLists)
#   SLINT_NOSTD_DIR  the no_std Slint workspace (slint/nostd/README.md);
#   NOSTD_PATCH_DIR  optional, the femtovg/glow forks (default: beside it)
#   AGL_SLINT_CLUSTER_DIR the agl-slint-cluster clone (its ui/app-window.slint);
#   NOTO_SANS_DIR    optional, the dir holding Noto Sans Regular, Light and
#                    ExtraLight (default: /usr/share/fonts/noto)
#   am62p: driver/src/fw/pvr_fw_blob.h in this checkout (driver/tools/mkfwblob.sh,
#          never committed)
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
die() { echo "common-images: $*" >&2; exit 2; }
DRY=0
[ "${1:-}" = --dry-run ] && { DRY=1; shift; }
[ $# -ge 3 ] || die "usage: $0 [--dry-run] <am62p|pi4|imx8mp> <table> <img-dir> [LABEL...]"
BOARD=$1 TABLE=$2 OUT=$3
shift 3
case "$BOARD" in
am62p)
    ZBOARD=verdin_am62p/am62p54/a53; GLTEST=gl_test
    ARM=(-DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n)
    MARKER='rti_arm: armed' ;;
imx8mp)
    ZBOARD=verdin_imx8mp/mimx8ml8/a53; GLTEST=gc_gl_test
    ARM=(-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y)
    MARKER='gc_wdog_arm: armed' ;;
pi4)
    ZBOARD=rpi_4b; GLTEST=v3d_gl_test
    ARM=()
    MARKER='board: PM watchdog resets' ;;
*) die "board must be am62p, pi4 or imx8mp, not '$BOARD'" ;;
esac
NEW_MARKER='board: watchdog armed'
# the printf format of the build line the image prints first ("sample: build
# tree=%s ..." from the board layer's sample_glue.c, "v3d_gl_test: build %s"),
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
    case "$1" in
    glmark2) SAMPLE=glmark2; ARGS+=("-DGLMARK2_SRC=${GLMARK2_SRC:-<GLMARK2_SRC>}") ;;
    slint | slint-view)
        SAMPLE=slint; ARGS+=("-DSLINT_NOSTD_DIR=${SLINT_NOSTD_DIR:-<SLINT_NOSTD_DIR>}")
        [ -n "${NOSTD_PATCH_DIR:-}" ] && ARGS+=("-DNOSTD_PATCH_DIR=$NOSTD_PATCH_DIR")
        [ "$1" = slint-view ] && ARGS+=(-DCONFIG_YAKOGL_BOARD_SCANOUT_VIEW=y) ;;
    cluster)
        SAMPLE=cluster
        ARGS+=("-DAGL_SLINT_CLUSTER_DIR=${AGL_SLINT_CLUSTER_DIR:-<AGL_SLINT_CLUSTER_DIR>}"
               "-DSLINT_NOSTD_DIR=${SLINT_NOSTD_DIR:-<SLINT_NOSTD_DIR>}")
        [ -n "${NOSTD_PATCH_DIR:-}" ] && ARGS+=("-DNOSTD_PATCH_DIR=$NOSTD_PATCH_DIR")
        [ -n "${NOTO_SANS_DIR:-}" ] && ARGS+=("-DNOTO_SANS_DIR=$NOTO_SANS_DIR") ;;
    gl_test) SAMPLE=$GLTEST ;;
    *) return 1 ;;
    esac
    return 0
}

rows > /dev/null || exit 2
if [ "$DRY" = 1 ]; then
    selected "$@" | while IFS=$'\t' read -r l w; do
        row_args "$w" || { echo "$l: '$w' is not glmark2, slint, slint-view, cluster or gl_test"; continue; }
        echo "$l $SAMPLE -b $ZBOARD ${ARGS[*]}"
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
if grep -qE '^(slint(-view)?|cluster)$' <<<"$need" && [ ! -d "${SLINT_NOSTD_DIR:-}/internal" ]; then
    die "set SLINT_NOSTD_DIR to the no_std Slint workspace (driver/samples/zephyr/slint/nostd/README.md)"
fi
if grep -qx cluster <<<"$need" && [ ! -f "${AGL_SLINT_CLUSTER_DIR:-}/ui/app-window.slint" ]; then
    die "set AGL_SLINT_CLUSTER_DIR to the agl-slint-cluster clone (driver/samples/zephyr/cluster/CMakeLists.txt)"
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
        echo "$l: $B exists, not reused" >&2
        echo "$l rc=exists sha256=none build=none" | tee "$OUT/$l.built"
        fails=$((fails + 1)); continue
    fi
    (cd "$ZEPHYR_WS" && west build -p always -b "$ZBOARD" -d "$B" "$REPO/driver/samples/zephyr/$SAMPLE" -- \
        "-DZEPHYR_MODULES=$MODS" "${ARGS[@]}") > "$B.log" 2>&1 < /dev/null
    rc=$? sha=none ids=none
    bin=$B/zephyr/zephyr.bin
    if [ "$rc" = 0 ] && [ -s "$bin" ]; then
        sha=$(sha256sum "$bin" | cut -d' ' -f1)
        # the build ids board.sh's image_build reads from the same build.ninja
        ids=$(grep -oE '(PVR_SAMPLE_TREE|PVR_SAMPLE_TABLE|V3D_BUILD|GC_BUILD_STAMP)=\\"[^\\"]*' "$B/build.ninja" |
            sed 's/\\"//' | sort -u | paste -sd, -)
        if ! grep -aqF "$MARKER" "$bin" && ! grep -aqF "$NEW_MARKER" "$bin"; then rc=no_marker
        elif [ "$BOARD" != imx8mp ] && ! grep -aqE "$BUILD_FMT" "$bin"; then rc=no_build_line; fi
    elif [ "$rc" = 0 ]; then
        rc=no_image
    fi
    [ "$rc" = 0 ] || fails=$((fails + 1))
    echo "$l rc=$rc sha256=$sha build=${ids:-none}" | tee "$OUT/$l.built"
done < <(selected "$@")
echo "common-images: $fails row(s) not built"
[ "$fails" = 0 ]
