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
#   tools/cts/board-stage.sh am62p gles3      the AM62P's whole dEQP-GLES3 tree
#   tools/cts/board-stage.sh x2 common        the Armadillo-X2's gl_test image (build only)
#   tools/cts/board-stage.sh x2 deqp          the Armadillo-X2's dEQP-GLES2 tree image (build only)
#   tools/cts/board-stage.sh x2 gles3         the Armadillo-X2's dEQP-GLES3 tree image (build only)
#   tools/cts/board-stage.sh stm32mp257f-dk common   the STM32MP257F-DK's gl_test image (build only)
#   tools/cts/board-stage.sh stm32mp257f-dk deqp     the STM32MP257F-DK's dEQP-GLES2 tree image (build only)
#
# The STM32MP257F-DK (the name tools/board-stm32mp2.sh prints in its verdict
# line) is staged as builds only, as x2 is: no board command, no BOARD_HOST,
# no BOARD_UART, no lease. `common` builds driver/samples/zephyr/gl_test for
# stm32mp257f_dk/stm32mp257fxx/a35 (the board files boards/stm32mp257f_dk_*,
# src/main_stm32mp2.c) with -DYAKOGL_WERROR=ON and -DCONFIG_GL_TEST_MP2_SET_HEAD=y
# (case 1 alone, the DK's first GL run). The watchdog marker ("gc_wdog_arm:
# armed") comes from the board layer's stm32mp2/app.conf
# (CONFIG_YAKOGL_STM32MP2_WDOG=y), and the stage checks the built image for it
# (the runner refuses an image without it). The image ends at
# build/common/stm32mp257f-dk-<sha8>/zephyr/zephyr.bin (log: the same name with
# .log); running it is `tools/board-stm32mp2.sh run`, a board step held by a
# runner. ms4 and gles3 are refused (exit 2). The dependency layout is the imx8mp
# one without IMX8MP_OUT and GLMARK2_SRC: the three entries every image needs.
# `deqp` builds driver/samples/zephyr/deqp for the same board with the whole
# dEQP-GLES2 table (tools/cts/imx8mp-full-tree.txt, 29 rows, one image, the
# x2 deqp shape) to build/deqp/stm32mp257f-dk-<sha8>/zephyr/zephyr.bin (log:
# the same name with .log). Its stage 1 is the DK's own (-mcpu=cortex-a35, the
# sample's README), not DEQP_STAGE1: DEQP_STAGE1_DK names it (no deps.toml
# entry; a stage 1 without build/modules/gles2/libdeqp-gles2-package.a stops
# the stage, exit 2). The built image must fit tools/board-stm32mp2.sh's
# limits, else the stage says which one it breaks and stops with exit 2 (the
# runner is not changed): zephyr.bin at most 34 MiB (DK_BIN_MAX, the free
# space of the card's bootfs) and the image's allocated sections inside
# 0xa0000000..0xb0000000 (readelf of zephyr.elf). The run is not this stage's:
# `tools/board-stm32mp2.sh run`, with RUN_BOUND=600 or more.
#
# The X2 (the Armadillo-X2, tools/board.sh's kexec-dev development board) is
# staged as builds only: this stage runs no board command for x2. It reads no
# BOARD_HOST and no BOARD_UART, sets no BOARD_BOOT (board.sh x2 is kexec-dev;
# the crash route is held by a runner, never by a stage), and clears no
# one-shot images (the X2 has no sudo and no /var/yakogl; board.sh x2 stages
# into /tmp/yakogl-kexec itself). The images are the imx8mp build scripts with
# their X2 selection (IMX8MP_BOARD=x2, tools/cts/imx8mp-env.sh):
# armadillo_x2/mimx8ml8/a53 with the watchdog marker ("gc_wdog_arm: armed")
# and the console from the sample's board conf (UART2), written under
# $IMX8MP_OUT/x2/ (IMX8MP_IMG_OUT) -- an X2 build never writes the Verdin's
# image directories beside it. The dependency layout is the imx8mp one (the
# same host entries; GLMARK2_SRC is not read: glmark2 has no Armadillo-X2
# image, so x2 common builds the gl_test row alone). The images end at
# $IMX8MP_OUT/x2/gltest-<sha8>, $IMX8MP_OUT/x2/deqp-<sha8> and
# $IMX8MP_OUT/x2/es3-<sha8>; running them is tools/board.sh x2 (a person or
# the board runner holds the board). x2 gles3 is the whole GLES3 mustpass as
# one table image (tools/cts/imx8mp-gles3-tree.txt, imx8mp-deqp-build.sh
# --gles3): its stage 1 is the gles3 one of imx8mp-deqp-stage1.sh gles3,
# DEQP_STAGE1_GLES3 (default $IMX8MP_OUT/deqp-stage1-gles3, shared with the
# Verdin), not the layout's DEQP_STAGE1; a missing one stops the stage (exit 2)
# before any build. ms4 has no X2 tree and is refused (exit 2).
#
# <board> is am62p, imx8mp, pi4, x2 or stm32mp257f-dk, as tools/board.sh names them. The images
# are built from HEAD (build/common/<board>-<sha8>, build/deqp/<board>-<sha8>)
# and the records go to records/<board>/{common,deqp,deqp-ms4}-<sha8>; a run
# that stopped resumes there (board.sh skips a row with a finished run). The
# common set's build resumes too: a complete image directory is reused, an
# interrupted one rebuilt (common-images.sh). A killed run can leave a
# once-*.bin in the board's stage directory (the Verdins' eMMC route); the run
# removes it before it starts and says so in one line.
#
# gles3 on the AM62P builds the 46 images of tools/cts/am62p-gles3-tree.txt
# (DEQP_MODULE=gles3, REPORT_ES2 off, each image checked with --gles3) into
# build/deqp/am62p-gles3-<sha8> and runs `board.sh am62p tree` into
# records/am62p/deqp-gles3-<sha8>. It needs a gles3 stage 1, which is not the
# farm's DEQP_WORK (GLES2): GLES3_WORK names it, else it is the sibling
# <DEQP_WORK>-gles3 (the deps layout: deqp-stage1-am62p-gles3 beside
# deqp-stage1-am62p). A complete one (build/modules/gles3/libdeqp-gles3-package.a)
# is reused, else it is built there by `cts-build-zephyr.sh MODULE=gles3` from the
# layout's VK-GL-CTS (CTS= overrides). Also for gles3, from the environment:
#   TREE_ROWS         only those rows (3,5-9, counted from 0 as board.sh tree does)
#                     are built and run; the build log is then appended to build.out
#   DEQP_PVR_NATIVE_3D  0|1: -DCONFIG_DEQP_PVR_NATIVE_3D=<v> for every image
#                     (driver/samples/zephyr/deqp/Kconfig; unset: -1, the
#                     library's default). The arm is named in the build and
#                     record directories (...-n3d0, ...-n3d1) and in
#                     <records>/stage-notes.txt, so two arms never mix.
#   BOARD_STAGE_DRY_RUN=1  print the commands, the rows and the directories,
#                     build and run nothing (needs no deps layout or board)
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
# needed_deps <layout> <what> [board]: the deps.toml entries a stage reads,
# one per line. Read from what the builders take from the environment:
#   every image   the west workspace (cts-build-zephyr.sh, common-images.sh)
#   common        the glmark2 source (common-images.sh; not board x2 or the DK:
#                 glmark2 has no image for either, they build gl_test alone)
#   am62p         the PowerVR firmware header (copied below, never committed)
#   imx8mp        IMX8MP_OUT, the image directory of imx8mp-env.sh (board x2
#                 uses the same layout: its images go to $IMX8MP_OUT/x2/; not
#                 the DK: its image goes under build/common/)
#   deqp, ms4     stage 1: DEQP_WORK (am62p, pi4) or DEQP_STAGE1 (imx8mp, x2)
#   gles3         DEQP_WORK on the am62p (its gles3 stage 1 sits beside it);
#                 not board x2: its stage 1 is DEQP_STAGE1_GLES3, no deps.toml
#                 entry (default under IMX8MP_OUT), checked by the stage itself
needed_deps() {
    local layout=$1 what=$2 board=${3:-}
    printf '%s\n' ZEPHYR_WS WEST_VENV ZEPHYR_SDK_INSTALL_DIR
    [ "$what" = common ] && [ "$board" != x2 ] && [ "$board" != stm32mp257f-dk ] && echo GLMARK2_SRC
    [ "$layout" = am62p ] && echo PVR_FW_BLOB
    [ "$layout" = imx8mp ] && [ "$board" != stm32mp257f-dk ] && echo IMX8MP_OUT
    if [ "$what" != common ] && ! { [ "$what" = gles3 ] && [ "$board" = x2 ]; }; then
        if [ "$board" = stm32mp257f-dk ]; then :   # DEQP_STAGE1_DK, checked by the stage itself
        elif [ "$layout" = imx8mp ]; then echo DEQP_STAGE1; else echo DEQP_WORK; fi
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
    need=$(needed_deps "$layout" "$what" "${3:-}")
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

usage() { echo "usage: tools/cts/board-stage.sh am62p|imx8mp|pi4 common|deqp|ms4|gles3 | x2 common|deqp|gles3 | stm32mp257f-dk common|deqp" >&2; exit 2; }
die() { echo "board-stage.sh: $1" >&2; exit "${2:-2}"; }
[ $# -eq 2 ] || usage
board=$1 what=$2
case "$board" in
am62p) layout=am62p key=AM62P ;;
imx8mp) layout=imx8mp key=IMX8MP ;;
pi4) layout=rpi_4b key=RPI4 ;;
x2) layout=imx8mp key=X2 ;;   # same host dependency layout as the Verdin (imx8mp-env.sh)
stm32mp257f-dk) layout=imx8mp key=DK ;;   # the three zephyr entries only (needed_deps); build only, like x2
*) usage ;;
esac
if [ "$board" = stm32mp257f-dk ]; then
    case "$what" in common | deqp) ;; *) die "$what is not staged for the stm32mp257f-dk: only 'common' (its gl_test image) and 'deqp' (its dEQP-GLES2 tree image), both build only; ms4 and gles3 are not its sets" ;; esac
fi
case "$what" in common | deqp) ;; ms4 | gles3)
    [ "$board:$what" = x2:ms4 ] && die "ms4 is the AM62P's tree; the x2 stage builds the gl_test image, the dEQP-GLES2 and the dEQP-GLES3 tree images, and runs no board"
    [ "$board" = am62p ] || [ "$board:$what" = x2:gles3 ] || die "$what is the AM62P's tree only" ;; *) usage ;; esac

cd "$(git rev-parse --show-toplevel)" || die "not in the repository"
H8=$(git rev-parse --short=8 HEAD)

# gles3: the arm and the directories, and the dry run, before the layout is read
if [ "$what" = gles3 ] && [ "$board" = am62p ]; then
    arm=
    case "${DEQP_PVR_NATIVE_3D:-}" in
    '') ;;
    0 | 1) arm=-n3d$DEQP_PVR_NATIVE_3D ;;
    *) die "DEQP_PVR_NATIVE_3D must be 0 or 1, not '$DEQP_PVR_NATIVE_3D'" ;;
    esac
    G3_BUILD=build/deqp/am62p-gles3-$H8$arm
    G3_REC=records/am62p/deqp-gles3-$H8$arm
    G3_TREE=tools/cts/am62p-gles3-tree.txt
    if [ "${BOARD_STAGE_DRY_RUN:-}" = 1 ]; then
        echo "board-stage.sh: am62p gles3 at $H8 (dry run: nothing is built or run)"
        echo "native 3D: ${DEQP_PVR_NATIVE_3D:-unset (-1: library default)}  rows: ${TREE_ROWS:-all}"
        echo "board-stage.sh\$ DEQP_MODULE=gles3 DEQP_PVR_NATIVE_3D=${DEQP_PVR_NATIVE_3D:-} DEQP_WORK=${GLES3_WORK:-<DEQP_WORK>-gles3} tools/cts/am62p-deqp-images.sh $G3_TREE $G3_BUILD $H8"
        tools/cts/am62p-deqp-images.sh --dry-run "$G3_TREE" "$G3_BUILD" "$H8" | sed 's/^/  /'
        echo "board-stage.sh\$ TREE_IMAGE_DIR=$G3_BUILD TREE_TAG=$H8 tools/board.sh am62p tree $G3_TREE $G3_REC"
        exit 0
    fi
fi

# x2: its dry run names the X2 outputs before the layout is read (as the
# gles3 one above); nothing is built, no board is touched, no deps needed.
if [ "$board" = x2 ] && [ "${BOARD_STAGE_DRY_RUN:-}" = 1 ]; then
    echo "board-stage.sh: x2 $what at $H8 (dry run: nothing is built; the x2 stage runs no board)"
    case "$what" in
    common)
        echo "board-stage.sh: x2 common builds the gl_test image alone: glmark2 has no Armadillo-X2 image"
        echo "board-stage.sh\$ IMX8MP_BOARD=x2 tools/cts/imx8mp-gltest-build.sh gltest-$H8 0"
        echo "  -> \$IMX8MP_OUT/x2/gltest-$H8/zephyr/zephyr.bin (arms WDOG1: \"gc_wdog_arm: armed\")" ;;
    deqp)
        echo "board-stage.sh\$ IMX8MP_BOARD=x2 tools/cts/imx8mp-deqp-build.sh deqp-$H8 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt"
        echo "  -> \$IMX8MP_OUT/x2/deqp-$H8/zephyr/zephyr.bin (arms WDOG1: \"gc_wdog_arm: armed\")" ;;
    gles3)
        echo "board-stage.sh: x2 gles3 needs the gles3 stage 1 at \${DEQP_STAGE1_GLES3:-\$IMX8MP_OUT/deqp-stage1-gles3} (tools/cts/imx8mp-deqp-stage1.sh gles3)"
        echo "board-stage.sh\$ IMX8MP_BOARD=x2 tools/cts/imx8mp-deqp-build.sh --gles3 es3-$H8 'dEQP-GLES3.info.*' --table tools/cts/imx8mp-gles3-tree.txt"
        echo "  -> \$IMX8MP_OUT/x2/es3-$H8/zephyr/zephyr.bin (arms WDOG1: \"gc_wdog_arm: armed\")" ;;
    esac
    exit 0
fi

# stm32mp257f-dk common: the build command, before the layout is read
DK_BUILD=build/common/stm32mp257f-dk-$H8
DK_ARGS=(-DYAKOGL_WERROR=ON -DCONFIG_GL_TEST_MP2_SET_HEAD=y)
DK_SAMPLE=gl_test
DK_STAGE1=${DEQP_STAGE1_DK:-}
DK_BIN_MAX=$((34 * 1024 * 1024))   # bootfs's free space (tools/board-stm32mp2.sh header)
DK_WIN_LO=$((0xa0000000)) DK_WIN_HI=$((0xb0000000))   # the runner's load window
if [ "$board" = stm32mp257f-dk ] && [ "$what" = deqp ]; then
    DK_BUILD=build/deqp/stm32mp257f-dk-$H8
    DK_SAMPLE=deqp
    DK_ARGS=(-DYAKOGL_WERROR=ON "-DDEQP_SRC=${DK_STAGE1:-<DEQP_STAGE1_DK>}/cts" "-DDEQP_BUILD=${DK_STAGE1:-<DEQP_STAGE1_DK>}/build"
        '-DCONFIG_DEQP_CASE="dEQP-GLES2.info.*"' "-DEXTRA_CONF_FILE=$PWD/$DK_BUILD.table.conf")
fi
if [ "$board" = stm32mp257f-dk ] && [ "${BOARD_STAGE_DRY_RUN:-}" = 1 ]; then
    echo "board-stage.sh: stm32mp257f-dk $what at $H8 (dry run: nothing is built; the stage runs no board)"
    if [ "$what" = deqp ]; then
        echo "board-stage.sh: stage 1 at \${DEQP_STAGE1_DK} (the DK's own, -mcpu=cortex-a35; driver/samples/zephyr/deqp/README.md)"
        echo "board-stage.sh: CONFIG_DEQP_CASE_TABLE from tools/cts/imx8mp-full-tree.txt -> $DK_BUILD.table.conf"
    fi
    echo "board-stage.sh\$ west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d $DK_BUILD driver/samples/zephyr/$DK_SAMPLE -- -DZEPHYR_MODULES=<modules> -DYAKOGL_DIR=\$PWD ${DK_ARGS[*]}"
    echo "  -> $DK_BUILD/zephyr/zephyr.bin (arms OP-TEE's watchdog: \"gc_wdog_arm: armed\")"
    [ "$what" = deqp ] && echo "  limits: zephyr.bin <= $DK_BIN_MAX bytes (bootfs), footprint inside $(printf '0x%x..0x%x' $DK_WIN_LO $DK_WIN_HI) (exit 2 when broken)"
    exit 0
fi

# the dependency layout: every entry or nothing
if ! check_deps "$layout" "$what" "$board"; then
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

# the board, from the leases when not given. The x2 stage touches no board
# (header): no lease, no console, no BOARD_BOOT (board.sh x2 is kexec-dev), no
# clear_stale_once (the X2 has no sudo and no /var/yakogl); its builds run
# under the build scripts' X2 selection instead.
if [ "$board" = x2 ]; then
    export IMX8MP_BOARD=x2
elif [ "$board" = stm32mp257f-dk ]; then
    :   # builds only: no lease, no console, no BOARD_BOOT, no clear_stale_once
else
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
fi
# dk_build: west build of driver/samples/zephyr/$DK_SAMPLE for the DK into
# $DK_BUILD (the common and the deqp stage); running it is
# `tools/board-stm32mp2.sh run`, a board step, never this stage.
dk_build() {
    [ -n "${WEST_VENV:-}" ] && PATH=$WEST_VENV/bin:$PATH
    export ZEPHYR_BASE=$ZEPHYR_WS/zephyr ZEPHYR_TOOLCHAIN_VARIANT=${ZEPHYR_TOOLCHAIN_VARIANT:-zephyr}
    DK_MODS=$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS") || die "no zephyr module list for $ZEPHYR_WS"
    mkdir -p "$(dirname "$DK_BUILD")"
    DK_REPO=$PWD
    echo "board-stage.sh\$ west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d $DK_BUILD driver/samples/zephyr/$DK_SAMPLE -- ${DK_ARGS[*]}   (log: $DK_BUILD.log)"
    (cd "$ZEPHYR_WS" && west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d "$DK_REPO/$DK_BUILD" \
        "$DK_REPO/driver/samples/zephyr/$DK_SAMPLE" -- "-DZEPHYR_MODULES=$DK_MODS" "-DYAKOGL_DIR=$DK_REPO" "${DK_ARGS[@]}") \
        > "$DK_BUILD.log" 2>&1 < /dev/null || die "DK $DK_SAMPLE image build failed ($DK_BUILD.log)" 1
    [ -s "$DK_BUILD/zephyr/zephyr.bin" ] || die "DK $DK_SAMPLE build wrote no image ($DK_BUILD.log)" 1
    grep -aqF 'gc_wdog_arm: armed' "$DK_BUILD/zephyr/zephyr.bin" ||
        die "DK image has no \"gc_wdog_arm: armed\" (the runner would refuse it): $DK_BUILD/zephyr/zephyr.bin" 1
    echo "board-stage.sh: DK image: $DK_BUILD/zephyr/zephyr.bin"
}
# dk_limits: the built image against the runner's limits (tools/board-stm32mp2.sh
# header); the first one it breaks is named and stops the stage with exit 2.
dk_limits() {
    local bin=$DK_BUILD/zephyr/zephyr.bin elf=$DK_BUILD/zephyr/zephyr.elf size lo hi
    size=$(stat -c %s "$bin")
    echo "board-stage.sh: DK image: $size bytes (limit $DK_BIN_MAX: bootfs's free space)"
    [ "$size" -le "$DK_BIN_MAX" ] ||
        die "DK dEQP image does not fit: $bin is $size bytes, over bootfs's free space of $DK_BIN_MAX (tools/board-stm32mp2.sh)"
    [ -f "$elf" ] || die "DK dEQP build wrote no $elf: the footprint cannot be checked against the runner's window" 1
    # allocated sections: start of the lowest, end of the highest (the heap is noinit, in the footprint)
    local n name type addr off len es flags a e
    lo=$((1 << 62)) hi=0
    while read -r n name type addr off len es flags _; do
        case "$n" in "["[0-9]*"]") ;; *) continue ;; esac
        case "$flags" in *A*) ;; *) continue ;; esac
        a=$((16#$addr)) e=$((a + 16#$len))
        [ "$a" -gt 0 ] || continue
        [ "$a" -ge "$lo" ] || lo=$a
        [ "$e" -le "$hi" ] || hi=$e
    done < <(readelf -SW "$elf" | sed 's/\[ */[/')
    [ "$hi" -gt 0 ] || die "DK dEQP build: no allocated section in $elf" 1
    printf 'board-stage.sh: DK footprint: 0x%x..0x%x, %d MiB (runner window 0x%x..0x%x)\n' \
        "$lo" "$hi" $(((hi - lo) >> 20)) "$DK_WIN_LO" "$DK_WIN_HI"
    if [ "$lo" -lt "$DK_WIN_LO" ] || [ "$hi" -gt "$DK_WIN_HI" ]; then
        die "DK dEQP image does not fit: footprint $(printf '0x%x..0x%x' "$lo" "$hi") is outside the runner's load window $(printf '0x%x..0x%x' "$DK_WIN_LO" "$DK_WIN_HI")"
    fi
}
echo "board-stage.sh: $board $what at $H8 ($(git describe --always --dirty 2>/dev/null || echo unknown))"
run(){ echo "board-stage.sh\$ $*"; "$@"; }
case "$what" in
common)
    if [ "$board" = stm32mp257f-dk ]; then
        dk_build   # gl_test for the DK (the sample's README)
    elif [ "$board" = x2 ]; then
        # the common set's gl_test row for the X2 (the sweep configuration;
        # one case with SEL=n, the set with SEL=256, tools/board.sh x2 run).
        # glmark2 has no Armadillo-X2 image: named, not silently skipped.
        echo "board-stage.sh: x2 common: glmark2 has no Armadillo-X2 image, building the gl_test image alone"
        run tools/cts/imx8mp-gltest-build.sh "gltest-$H8" 0 ||
            die "X2 gl_test image build failed ($IMX8MP_OUT/x2/gltest-$H8.log)" 1
        echo "board-stage.sh: X2 image: $IMX8MP_OUT/x2/gltest-$H8/zephyr/zephyr.bin"
    else
        run tools/cts/cts-common.sh "$board" build "build/common/$board-$H8" || die "common build failed" 1
        run tools/cts/cts-common.sh "$board" run "build/common/$board-$H8" "records/$board/common-$H8" || die "common run failed" 1
    fi
    ;;
deqp)
    case "$board" in
    stm32mp257f-dk)
        # the whole dEQP-GLES2 table as one image (the x2 shape), from the DK's own stage 1
        [ -f "$DK_STAGE1/build/modules/gles2/libdeqp-gles2-package.a" ] ||
            die "no stage 1 for the DK at '$DK_STAGE1': set DEQP_STAGE1_DK to the -mcpu=cortex-a35 stage 1 (driver/samples/zephyr/deqp/README.md)"
        mkdir -p "$(dirname "$DK_BUILD")"
        printf 'CONFIG_DEQP_CASE_TABLE="%s"\n' "$(grep -vE '^[[:space:]]*(#|$)' tools/cts/imx8mp-full-tree.txt | paste -sd';' -)" > "$DK_BUILD.table.conf"
        unset DEQP_GL_CONFIG_NAME
        dk_build
        dk_limits
        echo "board-stage.sh: DK dEQP image fits the runner; running it is tools/board-stm32mp2.sh run (RUN_BOUND=600 or more)"
        ;;
    x2)
        # the whole dEQP-GLES2 tree as one table image (29 rows, selector i);
        # running it is `TREE_IMAGE=$IMX8MP_OUT/x2/deqp-$H8/zephyr/zephyr.bin
        # tools/board.sh x2 tree` -- a board step, never this stage.
        run tools/cts/imx8mp-deqp-build.sh "deqp-$H8" 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt ||
            die "X2 dEQP image build failed ($IMX8MP_OUT/x2/deqp-$H8.log)" 1
        echo "board-stage.sh: X2 image: $IMX8MP_OUT/x2/deqp-$H8/zephyr/zephyr.bin"
        ;;
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
gles3)
    if [ "$board" = x2 ]; then
        # the whole GLES3 mustpass as one table image (133 rows, selector i);
        # running it is `TREE_IMAGE=$IMX8MP_OUT/x2/es3-$H8/zephyr/zephyr.bin
        # tools/board.sh x2 tree tools/cts/imx8mp-gles3-tree.txt <dir>` -- a
        # board step, never this stage. The stage 1 is the one --gles3 takes.
        X2_S1=${DEQP_STAGE1_GLES3:-$IMX8MP_OUT/deqp-stage1-gles3}
        [ -f "$X2_S1/build/modules/gles3/libdeqp-gles3-package.a" ] ||
            die "no gles3 stage 1 at $X2_S1 (tools/cts/imx8mp-deqp-stage1.sh gles3, or set DEQP_STAGE1_GLES3)"
        run tools/cts/imx8mp-deqp-build.sh --gles3 "es3-$H8" 'dEQP-GLES3.info.*' --table tools/cts/imx8mp-gles3-tree.txt ||
            die "X2 dEQP-GLES3 image build failed ($IMX8MP_OUT/x2/es3-$H8.log)" 1
        echo "board-stage.sh: X2 image: $IMX8MP_OUT/x2/es3-$H8/zephyr/zephyr.bin"
        echo "board-stage.sh: $board $what at $H8 done"
        exit 0
    fi
    # the gles3 stage 1: GLES3_WORK, else beside the GLES2 one
    G3_WORK=${GLES3_WORK:-${DEQP_WORK%/}-gles3}
    if [ ! -f "$G3_WORK/build/modules/gles3/libdeqp-gles3-package.a" ]; then
        G3_CTS=${CTS:-$(dirname "$DEQP_WORK")/VK-GL-CTS}
        [ -d "$G3_CTS" ] || die "no VK-GL-CTS at $G3_CTS for the gles3 stage 1 (set CTS=)"
        [ -n "${WEST_VENV:-}" ] && PATH=$WEST_VENV/bin:$PATH
        echo "board-stage.sh: building the gles3 stage 1 in $G3_WORK"
        (
            export ZEPHYR_BASE=$ZEPHYR_WS/zephyr ZEPHYR_TOOLCHAIN_VARIANT=${ZEPHYR_TOOLCHAIN_VARIANT:-zephyr}
            ZEPHYR_MODULES=$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS") || exit 1
            export ZEPHYR_MODULES CFLAGS=-O2 CXXFLAGS=-O2   # as DEQP_WORK's build (deps.toml)
            CTS=$G3_CTS WORK=$G3_WORK MODULE=gles3 run tools/cts/cts-build-zephyr.sh
        ) || { rm -rf "$G3_WORK"; die "gles3 stage 1 build failed" 1; }
    else
        echo "board-stage.sh: reusing the gles3 stage 1 in $G3_WORK"
    fi
    mkdir -p "$G3_BUILD" "$G3_REC"
    # TREE_ROWS resumes: keep the rows already in build.out
    redir='>'
    [ -n "${TREE_ROWS:-}" ] && redir='>>'
    {
        echo "commit $H8 ($(git describe --always --dirty 2>/dev/null || echo unknown))"
        echo "DEQP_PVR_NATIVE_3D=${DEQP_PVR_NATIVE_3D:-unset (-1: library default)}"
        echo "TREE_ROWS=${TREE_ROWS:-all}"
        echo "gles3 stage 1: $G3_WORK"
    } >> "$G3_REC/stage-notes.txt"
    echo "board-stage.sh\$ DEQP_MODULE=gles3 DEQP_WORK=$G3_WORK tools/cts/am62p-deqp-images.sh $G3_TREE $G3_BUILD $H8 $redir $G3_BUILD/build.out"
    if [ "$redir" = '>>' ]; then
        DEQP_MODULE=gles3 DEQP_WORK=$G3_WORK tools/cts/am62p-deqp-images.sh "$G3_TREE" "$G3_BUILD" "$H8" >> "$G3_BUILD/build.out" 2>&1
    else
        DEQP_MODULE=gles3 DEQP_WORK=$G3_WORK tools/cts/am62p-deqp-images.sh "$G3_TREE" "$G3_BUILD" "$H8" > "$G3_BUILD/build.out" 2>&1
    fi || die "gles3 image build failed ($G3_BUILD/build.out)" 1
    TREE_IMAGE_DIR=$G3_BUILD TREE_TAG=$H8 \
        run tools/board.sh am62p tree "$G3_TREE" "$G3_REC" || die "gles3 tree failed" 1
    ;;
esac
echo "board-stage.sh: $board $what at $H8 done"
