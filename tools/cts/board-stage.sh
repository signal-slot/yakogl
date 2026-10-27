#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board-stage.sh -- one board run of the ES 2.0 sign-off, from HEAD, as one
# command: what a ccfarm board stage runs (ccfarm's settings), and what a
# person runs by hand on a board they hold.
#
#   tools/cts/board-stage.sh <board> common   the common set (tools/cts/common-tree.txt)
#   tools/cts/board-stage.sh <board> deqp     the whole dEQP-GLES2 tree
#   tools/cts/board-stage.sh am62p ms4        the AM62P's 4x multisample tree
#
# <board> is am62p, imx8mp or pi4, as tools/board.sh names them. The images
# are built from HEAD (build/common/<board>-<sha8>, build/deqp/<board>-<sha8>)
# and the records go to records/<board>/{common,deqp,deqp-ms4}-<sha8>; a run
# that stopped resumes there (board.sh skips a row with a finished run). The
# common set's build resumes too: a complete image directory is reused, an
# interrupted one rebuilt (common-images.sh). A killed run can leave a
# once-*.bin in the board's stage directory (the Verdins' eMMC route); the run
# removes it before it starts and says so in one line.
#
# The board, from the environment (a farm stage gets it from its leases):
#   BOARD_HOST   user@host of the board's Linux; else RESOURCE_<B>_SSH, the
#                stage's lease of target:<b> (B = AM62P, IMX8MP, RPI4)
#   BOARD_UART   host:port of the console's serial bridge (AM62P, Pi 4); else
#                RESOURCE_<B>_CONSOLE_HOST:RESOURCE_<B>_CONSOLE_PORT, the
#                lease of target:<b>-console
#   BOARD_HDMI_TIMINGS  the Pi 4 panel's hdmi_timings (tools/board.sh)
# The two Verdins boot by U-Boot from the eMMC (BOARD_BOOT=uboot-mmc), the
# Pi 4 by the firmware's tryboot (board.sh's default). Everything the build
# needs comes from tools/cts/deps-env.sh's layout; a missing entry is named and
# nothing runs (exit 2). Only the entries the board and the set read are
# required (needed_deps below): the shader compilers and Mesa trees the gate
# uses (PCO_SHC, MESA_PAN_TREE, GLSLANG, ...) are not, as no image is built
# with them (the generated shader tables are committed).
set -uo pipefail

# clear_stale_once: remove the one-shot images an earlier, killed run left in
# the board's stage directory (board.sh would refuse the run, or a later boot
# could pick one up). One line says what went. Needs BOARD_HOST.
STAGE_DIR=/var/yakogl
clear_stale_once() {
    local gone
    gone=$(ssh -o BatchMode=yes -o ConnectTimeout=10 "$BOARD_HOST" \
        "sudo -n sh -c 'ls $STAGE_DIR/once-*.bin 2>/dev/null; rm -f $STAGE_DIR/once-*.bin'" < /dev/null 2>/dev/null |
        sed 's|.*/||' | paste -sd' ' -)
    if [ -n "$gone" ]; then
        echo "board-stage.sh: removed stale one-shot image(s) from $STAGE_DIR: $gone"
    fi
    return 0
}
# needed_deps <layout> <what>: the deps.toml entries a stage reads, one per
# line. Read from what the builders take from the environment:
#   every image   the west workspace (cts-build-zephyr.sh, common-images.sh)
#   common        the glmark2 source (common-images.sh)
#   am62p         the PowerVR firmware header (copied below, never committed)
#   imx8mp        IMX8MP_OUT, the image directory of imx8mp-env.sh
#   deqp, ms4     stage 1: DEQP_WORK (am62p, pi4) or DEQP_STAGE1 (imx8mp)
needed_deps() {
    local layout=$1 what=$2
    printf '%s\n' ZEPHYR_WS WEST_VENV ZEPHYR_SDK_INSTALL_DIR
    [ "$what" = common ] && echo GLMARK2_SRC
    [ "$layout" = am62p ] && echo PVR_FW_BLOB
    [ "$layout" = imx8mp ] && echo IMX8MP_OUT
    if [ "$what" != common ]; then
        if [ "$layout" = imx8mp ]; then echo DEQP_STAGE1; else echo DEQP_WORK; fi
    fi
    return 0
}

# check_deps <layout> <what>: succeed when every needed entry is present. The
# entries deps-env.sh --check reports missing that the stage does not read are
# not an error; a needed one (or an unreadable layout) prints and fails.
check_deps() {
    local layout=$1 what=$2 check rc=0 need line name missing=()
    check=$(tools/cts/deps-env.sh --check "$layout") || rc=$?
    [ "$rc" = 0 ] && return 0
    need=$(needed_deps "$layout" "$what")
    while read -r line; do
        case "$line" in
        missing*) name=$(awk '{print $2}' <<< "$line"); grep -qx "$name" <<< "$need" && missing+=("$line") ;;
        esac
    done <<< "$check"
    if [ ${#missing[@]} -gt 0 ]; then
        printf '%s\n' "${missing[@]}" >&2
        return 1
    fi
    # exit 1 with no missing entry: no layout at all, say what deps-env.sh said
    grep -q '^missing' <<< "$check" && return 0
    [ -z "$check" ] || printf '%s\n' "$check" >&2
    return 1
}
[ "${BOARD_STAGE_SOURCE_ONLY:-}" = 1 ] && return 0 2>/dev/null

usage() { echo "usage: tools/cts/board-stage.sh am62p|imx8mp|pi4 common|deqp|ms4" >&2; exit 2; }
die() { echo "board-stage.sh: $1" >&2; exit "${2:-2}"; }
[ $# -eq 2 ] || usage
board=$1 what=$2
case "$board" in
am62p) layout=am62p key=AM62P ;;
imx8mp) layout=imx8mp key=IMX8MP ;;
pi4) layout=rpi_4b key=RPI4 ;;
*) usage ;;
esac
case "$what" in common | deqp) ;; ms4) [ "$board" = am62p ] || die "ms4 is the AM62P's tree only" ;; *) usage ;; esac

cd "$(git rev-parse --show-toplevel)" || die "not in the repository"
H8=$(git rev-parse --short=8 HEAD)

# the dependency layout: every entry or nothing
if ! check_deps "$layout" "$what"; then
    die "the dependency layout is incomplete on $(hostname -s) (above)"
fi
set -a
eval "$(tools/cts/deps-env.sh "$layout")"
set +a
export RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-nightly}
# every AM62P image needs the PowerVR firmware header, which is never
# committed: copied from the layout as the CI gate copies it (gate.yml)
if [ "$board" = am62p ] && [ ! -e driver/src/fw/pvr_fw_blob.h ]; then
    mkdir -p driver/src/fw && cp "$PVR_FW_BLOB" driver/src/fw/ || die "cannot copy PVR_FW_BLOB ($PVR_FW_BLOB) to driver/src/fw/"
fi

# the board, from the leases when not given
ssh_var=RESOURCE_${key}_SSH
export BOARD_HOST=${BOARD_HOST:-${!ssh_var:-}}
[ -n "$BOARD_HOST" ] || die "no board: set BOARD_HOST, or lease target:$board ($ssh_var)"
if [ "$board" != imx8mp ]; then
    chost=RESOURCE_${key}_CONSOLE_HOST cport=RESOURCE_${key}_CONSOLE_PORT
    if [ -z "${BOARD_UART:-}" ] && [ -n "${!chost:-}" ] && [ -n "${!cport:-}" ]; then
        BOARD_UART=${!chost}:${!cport}
    fi
    export BOARD_UART=${BOARD_UART:-}
    [ -n "$BOARD_UART" ] || die "no console: set BOARD_UART, or lease target:$board-console ($chost, $cport)"
fi
[ "$board" = pi4 ] || export BOARD_BOOT=${BOARD_BOOT:-uboot-mmc}
if [ "$board" = pi4 ] && [ -z "${BOARD_HDMI_TIMINGS:-}" ]; then
    die "set BOARD_HDMI_TIMINGS (the panel's hdmi_timings, tools/board.sh)"
fi

[ "$board" = pi4 ] || clear_stale_once   # the Pi 4 boots by tryboot: nothing is staged on its eMMC
echo "board-stage.sh: $board $what at $H8 ($(git describe --always --dirty 2>/dev/null || echo unknown))"
run() { echo "board-stage.sh\$ $*"; "$@"; }
case "$what" in
common)
    run tools/cts/cts-common.sh "$board" build "build/common/$board-$H8" || die "common build failed" 1
    run tools/cts/cts-common.sh "$board" run "build/common/$board-$H8" "records/$board/common-$H8" || die "common run failed" 1
    ;;
deqp)
    case "$board" in
    am62p)
        mkdir -p "build/deqp/am62p-$H8"
        echo "board-stage.sh\$ tools/cts/am62p-deqp-images.sh tools/cts/am62p-full-tree.txt build/deqp/am62p-$H8 $H8 > build/deqp/am62p-$H8/build.out"
        tools/cts/am62p-deqp-images.sh tools/cts/am62p-full-tree.txt "build/deqp/am62p-$H8" "$H8" > "build/deqp/am62p-$H8/build.out" 2>&1 ||
            die "dEQP image build failed (build/deqp/am62p-$H8/build.out)" 1
        TREE_IMAGE_DIR=build/deqp/am62p-$H8 TREE_TAG=$H8 \
            run tools/board.sh am62p tree tools/cts/am62p-full-tree.txt "records/am62p/deqp-$H8" || die "dEQP tree failed" 1
        ;;
    imx8mp)
        run tools/cts/imx8mp-deqp-build.sh "deqp-$H8" 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt ||
            die "dEQP image build failed" 1
        TREE_IMAGE=$IMX8MP_OUT/deqp-$H8/zephyr/zephyr.bin \
            run tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt "records/imx8mp/deqp-$H8" || die "dEQP tree failed" 1
        ;;
    pi4)
        run tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt "build/deqp/pi4-$H8" || die "dEQP image build failed" 1
        TREE_IMAGE_DIR=build/deqp/pi4-$H8 \
            run tools/board.sh pi4 tree tools/cts/pi4-full-tree.txt "records/pi4/deqp-$H8" || die "dEQP tree failed" 1
        ;;
    esac
    ;;
ms4)
    mkdir -p "build/deqp/am62p-ms4-$H8"
    echo "board-stage.sh\$ tools/cts/am62p-deqp-images.sh tools/cts/am62p-ms4-tree.txt build/deqp/am62p-ms4-$H8 > build/deqp/am62p-ms4-$H8/build.out"
    tools/cts/am62p-deqp-images.sh tools/cts/am62p-ms4-tree.txt "build/deqp/am62p-ms4-$H8" > "build/deqp/am62p-ms4-$H8/build.out" 2>&1 ||
        die "ms4 image build failed (build/deqp/am62p-ms4-$H8/build.out)" 1
    TREE_IMAGE_DIR=build/deqp/am62p-ms4-$H8 \
        run tools/board.sh am62p tree tools/cts/am62p-ms4-tree.txt "records/am62p/deqp-ms4-$H8" || die "ms4 tree failed" 1
    ;;
esac
echo "board-stage.sh: $board $what at $H8 done"
