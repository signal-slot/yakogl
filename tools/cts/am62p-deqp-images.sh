#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Stage 2 of dEQP-GLES2 on Zephyr, Verdin AM62P: one driver/samples/zephyr/deqp
# image per line of a filter file, from this checkout as it is.
#
#   ZEPHYR_WS=... DEQP_WORK=... \
#       tools/cts/am62p-deqp-images.sh [--dry-run] <filters.txt> <out-dir> [tag]
#
# <filters.txt>  one image per line: the value of CONFIG_DEQP_CASE, i.e. dEQP's
#                --deqp-case, comma-separated patterns, for example
#                  dEQP-GLES2.functional.texture.size.*
#                Blank lines and lines starting with '#' are skipped.
#                tools/cts/am62p-full-tree.txt is the whole GLES2 tree in 17.
#                A line preceded by "# image <LABEL>:" (the form of
#                pi4-full-tree.txt and baseline-tree.txt) is built into
#                <out-dir>/<LABEL> and its verdict also goes to
#                <out-dir>/<LABEL>.built ("<LABEL> rc=<rc> sha256=<prefix>"),
#                which is what tools/board.sh looks for with such a label.
#                A marker "(with DEQP_GL_CONFIG_NAME=<name>)" on that line
#                builds the image with -DCONFIG_DEQP_GL_CONFIG_NAME="<name>",
#                dEQP's --deqp-gl-config-name: tools/cts/am62p-ms4-tree.txt's
#                MS4 row is rgba8888d24s8ms4, the 4x multisampled config the
#                multisample.* group needs (the other groups run on the
#                single-sampled one). Every other image gets an explicit empty
#                name, so a DEQP_GL_CONFIG_NAME left in the environment
#                (the Kconfig default reads it) cannot turn a sweep image
#                into a multisample one.
# <out-dir>      build directories go to <out-dir>/img<N>-<tag> (or <LABEL>)
# [tag]          defaults to the checkout's short hash
# --dry-run      print "<N> <dir name> <filter>" per image and build nothing
#                (needs none of the environment below)
#
# Prints "img<N> rc=<rc> bin=<sha256 prefix>" per image (redirect stdout to
# <out-dir>/build.out), the line
# tools/board.sh am62p tree waits for (TREE_IMAGE_DIR), so a tree can start on image 1 while the
# later ones build. The build log of image N is <out-dir>/<dir name>.log.
#
# Needs (all from the environment; nothing is defaulted to a path on one
# machine):
#   ZEPHYR_WS     the west workspace: $ZEPHYR_WS/zephyr is ZEPHYR_BASE and
#                 $ZEPHYR_WS/modules/fs/fatfs the FatFs module
#   (SAFEUI_DIR is no longer used: the A53 board, SoC and the DSS/DSI/TISCI
#   drivers are in driver/. The module list comes from zephyr-modules.sh, which
#   leaves out a SafeUI checkout the workspace may still carry.)
#   DEQP_WORK     stage 1's output (tools/cts/cts-build-zephyr.sh WORK=):
#                 $DEQP_WORK/cts and $DEQP_WORK/build
#   ZEPHYR_TOOLCHAIN_VARIANT  (default zephyr) and the Zephyr SDK the
#                 workspace uses, found the usual way (ZEPHYR_SDK_INSTALL_DIR
#                 or the SDK's cmake package registration)
#   WEST_VENV     optional: a Python venv whose bin/ holds west; prepended to
#                 PATH when set
#   DEQP_MODULE   gles2 (default) or gles3: the dEQP package. gles3 needs a
#                 stage 1 built with MODULE=gles3 as DEQP_WORK, and builds
#                 the images with CONFIG_DEQP_REPORT_ES2=n (the context
#                 reports ES 3.0)
#   driver/src/fw/pvr_fw_blob.h in this checkout: generated from the board's
#                 firmware by driver/tools/mkfwblob.sh, never committed
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
die() { echo "am62p-deqp-images: $*" >&2; exit 2; }
DRY=0
[ "${1:-}" = --dry-run ] && { DRY=1; shift; }
[ $# -ge 2 ] || die "usage: $0 [--dry-run] <filters.txt> <out-dir> [tag]"
FILTERS=$1; OUT=$2; TAG=${3:-$(git -C "$REPO" rev-parse --short=8 HEAD)}
[ -f "$FILTERS" ] || die "no filter file $FILTERS"
# rows FILTERS: "<N>\t<dir name>\t<labelled 0|1>\t<render config or ->\t<filter>"
# per image, the label rule of tools/board.sh tree_rows (TREE_IMAGE_DIR)
rows() {
    awk -v tag="$TAG" '
        /^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); c = ""
          if (match($0, /\(with DEQP_GL_CONFIG_NAME=[A-Za-z0-9_]+\)/)) {
              c = substr($0, RSTART + 6, RLENGTH - 7); sub(/^DEQP_GL_CONFIG_NAME=/, "", c) }
          next }
        /^[[:space:]]*(#|$)/ { next }
        { n++; lab = (l != "") ? l : "img" n "-" tag
          printf "%d\t%s\t%d\t%s\t%s\n", n, lab, (l != ""), (c != "" ? c : "-"), $0; l = ""; c = "" }' "$1"
}
if [ "$DRY" = 1 ]; then
    rows "$FILTERS" | while IFS=$'\t' read -r n lab _ conf filt; do
        if [ "$conf" = - ]; then echo "$n $lab $filt"; else echo "$n $lab $filt [DEQP_GL_CONFIG_NAME=$conf]"; fi
    done
    exit 0
fi
: "${ZEPHYR_WS:?set ZEPHYR_WS to the west workspace (it holds zephyr/ and modules/)}"
[ -n "${SAFEUI_DIR:-}" ] && echo "am62p-deqp-images: SAFEUI_DIR is ignored (the board support is in driver/)" >&2
: "${DEQP_WORK:?set DEQP_WORK to stage 1 output (tools/cts/cts-build-zephyr.sh WORK=)}"
[ -d "$ZEPHYR_WS/zephyr" ] || die "$ZEPHYR_WS/zephyr does not exist"
[ -d "$ZEPHYR_WS/modules/fs/fatfs" ] || die "$ZEPHYR_WS/modules/fs/fatfs does not exist (west update)"
[ -d "$DEQP_WORK/cts" ] && [ -d "$DEQP_WORK/build" ] || die "$DEQP_WORK/cts or /build missing: run stage 1"
[ -f "$REPO/driver/src/fw/pvr_fw_blob.h" ] || \
    die "driver/src/fw/pvr_fw_blob.h missing: generate it with driver/tools/mkfwblob.sh (never commit it)"
[ -n "${WEST_VENV:-}" ] && PATH=$WEST_VENV/bin:$PATH
command -v west >/dev/null || die "west not on PATH (set WEST_VENV)"
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
export ZEPHYR_BASE=$ZEPHYR_WS/zephyr ZEPHYR_TOOLCHAIN_VARIANT=${ZEPHYR_TOOLCHAIN_VARIANT:-zephyr}
MODS=$("$HERE/zephyr-modules.sh" "$ZEPHYR_WS") || die "zephyr-modules.sh failed"
DEQP_MODULE=${DEQP_MODULE:-gles2}
case "$DEQP_MODULE" in
    gles2) MODARGS="-DDEQP_MODULE=gles2" ;;
    gles3) MODARGS="-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n" ;;
    *) die "DEQP_MODULE must be gles2 or gles3, not '$DEQP_MODULE'" ;;
esac
[ -f "$DEQP_WORK/build/modules/$DEQP_MODULE/libdeqp-$DEQP_MODULE-package.a" ] || die "$DEQP_WORK has no $DEQP_MODULE stage 1 (cts-build-zephyr.sh MODULE=$DEQP_MODULE)"

unset DEQP_GL_CONFIG_NAME
rows "$FILTERS" | while IFS=$'\t' read -r n lab labelled conf filt; do
    B=$OUT/$lab
    [ "$conf" = - ] && conf=
    (cd "$ZEPHYR_WS" && west build -p always -b verdin_am62p/am62p54/a53 -d "$B" \
        "$REPO/driver/samples/zephyr/deqp" -- "-DZEPHYR_MODULES=$MODS" \
        -DDEQP_SRC="$DEQP_WORK/cts" -DDEQP_BUILD="$DEQP_WORK/build" $MODARGS \
        "-DCONFIG_DEQP_GL_CONFIG_NAME=\"$conf\"" \
        "-DCONFIG_DEQP_CASE=\"$filt\"") > "$B.log" 2>&1 < /dev/null && rc=0 || rc=$?
    # the resolved Kconfig must still be a conformance image (the compiler, REPORT_ES2, no
    # precompiled tier, the render config the row asks for); one that is not is not built
    if [ "$rc" = 0 ] && [ "$DEQP_MODULE" = gles2 ]; then
        python3 "$HERE/check-deqp-images.py" --config "$B/zephyr/.config" ${conf:+--ms4} >> "$B.log" 2>&1 \
            || rc=not-conformance
    fi
    bin=$(sha256sum "$B/zephyr/zephyr.bin" 2>/dev/null | cut -c1-16)
    echo "img$n rc=$rc bin=${bin:-none}"
    [ "$labelled" = 1 ] && echo "$lab rc=$rc sha256=${bin:-none}" > "$OUT/$lab.built"
done
echo "BUILD-EXIT"
