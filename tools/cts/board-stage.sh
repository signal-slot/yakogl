#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board-stage.sh -- one board run of the ES 2.0 sign-off, from HEAD, as one
# command: what a ccfarm board stage runs (.ccfarm/project.toml), and what a
# person runs by hand on a board they hold.
#
#   tools/cts/board-stage.sh <board> common   the common set (tools/cts/common-tree.txt)
#   tools/cts/board-stage.sh <board> deqp     the whole dEQP-GLES2 tree
#   tools/cts/board-stage.sh am62p ms4        the AM62P's 4x multisample tree
#
# <board> is am62p, imx8mp or pi4, as tools/board.sh names them. The images
# are built from HEAD (build/common/<board>-<sha8>, build/deqp/<board>-<sha8>)
# and the records go to records/<board>/{common,deqp,deqp-ms4}-<sha8>; a run
# that stopped resumes there (board.sh skips a row with a finished run).
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
# nothing runs (exit 2).
set -uo pipefail

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
if ! check=$(tools/cts/deps-env.sh --check "$layout"); then
    printf '%s\n' "$check" | grep -v '^ok' >&2
    die "the dependency layout is incomplete on $(hostname -s) (above)"
fi
set -a
eval "$(tools/cts/deps-env.sh "$layout")"
set +a
export RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-nightly}

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
