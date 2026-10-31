#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# pr-session.sh -- one AM62P board session of the PowerVR partial render
# (issue #2, #3; driver/docs/pvr-render-pool.md "Design: the firmware's
# partial render"): build the gl_test image from HEAD with the knob and
# the probe settings, run it through tools/board.sh, keep the serial log and
# the verdict.
#
#   tools/cts/pr-session.sh <name> --knob off|on [--cap-pages N] [--one-in-flight]
#                           [--cases 'list'] [--glmark2] [--allow-local] [--dry-run]
#
# <name>         names the session: results go to
#                tools/cts/log/<YYYY-MM-DD>-pr-<name>/
# --knob         CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=n|y
# --cap-pages N  CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=N (512: the
#                lists cannot grow, the firmware asks for no grow). Default
#                0, the probe off.
# --one-in-flight  CONFIG_GLMARK2_PVR_ONE_KICK_IN_FLIGHT=y (d320c8a5's
#                configuration, docs/notes/pvr-render-pool.md). That option
#                belongs to the glmark2 sample, so it needs --glmark2.
# --cases        the gl_test cases, comma or space separated, in order, on one
#                context (CONFIG_GL_TEST_CASES). Default 175 (refusehang).
# --glmark2      after gl_test, build and run the glmark2 image too, with the
#                same knob and cap (GLMARK2_SRC as tools/cts/common-images.sh).
# --allow-local  build a HEAD that is not on origin/main (the log says so);
#                without it the session refuses
# --dry-run      print the build and run commands, build and run nothing; the
#                board and the toolchain environment are not read
#
# The build is common-images.sh's west command (its board arguments, the
# watchdog self-arm, zephyr-modules.sh) plus the Kconfig overlay above. The
# run is `tools/board.sh am62p run <image> <logdir> <name>`.
#
# Environment (nothing is written to a file or a commit):
#   BOARD_HOST (or RESOURCE_AM62P_SSH)       user@host of the board's Linux
#   BOARD_UART (or RESOURCE_AM62P_CONSOLE_HOST:..._PORT)  the serial bridge
#   ZEPHYR_WS, WEST_VENV, ZEPHYR_SDK_INSTALL_DIR, GLMARK2_SRC, PVR_FW_BLOB as
#   tools/cts/board-stage.sh and tools/cts/deps-env.sh am62p set them.
# The kept log has IPv4 addresses, MACs and the board's host name redacted as
# tools/board.sh does (192.0.2.n, 02:00:00:00:00:nn, <board-host>).
#
# Exit: 0 session ran and its verdict is done; 1 build or run failed or the
# verdict is not done; 2 refused or could not start.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
die() { echo "pr-session.sh: $1" >&2; exit "${2:-2}"; }
usage() {
    echo "usage: tools/cts/pr-session.sh <name> --knob off|on [--cap-pages N] [--one-in-flight] [--cases 'list'] [--glmark2] [--allow-local] [--dry-run]" >&2
    exit 2
}

KNOB= CAP=0 ONE=0 CASES=175 GLM=0 LOCAL=0 DRY=0
[ $# -ge 1 ] || usage
case "$1" in -*) usage ;; esac
NAME=$1
shift
[[ $NAME =~ ^[A-Za-z0-9][A-Za-z0-9_.-]*$ ]] || die "name '$NAME' must be letters, digits, _ . -"
while [ $# -gt 0 ]; do
    case "$1" in
    --knob) [ $# -ge 2 ] || usage; KNOB=$2; shift 2 ;;
    --knob=*) KNOB=${1#--knob=}; shift ;;
    --cap-pages) [ $# -ge 2 ] || usage; CAP=$2; shift 2 ;;
    --cap-pages=*) CAP=${1#--cap-pages=}; shift ;;
    --cases) [ $# -ge 2 ] || usage; CASES=$2; shift 2 ;;
    --cases=*) CASES=${1#--cases=}; shift ;;
    --one-in-flight) ONE=1; shift ;;
    --glmark2) GLM=1; shift ;;
    --allow-local) LOCAL=1; shift ;;
    --dry-run) DRY=1; shift ;;
    *) usage ;;
    esac
done
case "$KNOB" in off) KCFG=n ;; on) KCFG=y ;; *) die "--knob off|on is required" ;; esac
[[ $CAP =~ ^[0-9]+$ ]] || die "--cap-pages takes a number, not '$CAP'"
CASES=$(tr -s ' ,' ',' <<< "$CASES" | sed 's/^,//; s/,$//')
[[ $CASES =~ ^[0-9]+(,[0-9]+)*$ ]] || die "--cases takes case numbers, not '$CASES'"
[ "$ONE" = 0 ] || [ "$GLM" = 1 ] || die "--one-in-flight is the glmark2 sample's option: add --glmark2"

cd "$REPO" || die "no repository"
H8=$(git rev-parse --short=8 HEAD) || die "not a git checkout"

# HEAD is on origin/main, or --allow-local says so
if git merge-base --is-ancestor HEAD origin/main 2> /dev/null; then
    ONMAIN="HEAD $H8 is on origin/main"
elif [ "$LOCAL" = 1 ]; then
    ONMAIN="HEAD $H8 is NOT on origin/main (--allow-local): a local build"
else
    die "HEAD $H8 is not on origin/main (git merge-base --is-ancestor); --allow-local builds it anyway"
fi

DAY=$(date +%Y-%m-%d)
OUT=tools/cts/log/$DAY-pr-$NAME
BUILD=build/pr/$NAME-$H8
CMD="pr-session.sh $NAME --knob $KNOB"
[ "$CAP" = 0 ] || CMD+=" --cap-pages $CAP"
[ "$ONE" = 0 ] || CMD+=" --one-in-flight"
CMD+=" --cases '$CASES'"
[ "$GLM" = 0 ] || CMD+=" --glmark2"
[ "$LOCAL" = 0 ] || CMD+=" --allow-local"

# the Kconfig overlay, one -D per option (the knob and the cap are for both images)
OVERLAY=("-DCONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=$KCFG" "-DCONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=$CAP")
# common-images.sh's am62p arguments
ZBOARD=verdin_am62p/am62p54/a53
ARM=(-DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n)

# images: label, sample, extra cmake arguments
LABELS=(gltest)
SAMPLES=(gl_test)
EXTRA_0="-DCONFIG_GL_TEST_CASES=\"$CASES\""
EXTRA_1=
if [ "$GLM" = 1 ]; then
    LABELS+=(glmark2)
    SAMPLES+=(glmark2)
    EXTRA_1="-DGLMARK2_SRC=${GLMARK2_SRC:-<GLMARK2_SRC>}"
    [ "$ONE" = 0 ] || EXTRA_1+=" -DCONFIG_GLMARK2_PVR_ONE_KICK_IN_FLIGHT=y"
fi

build_cmd() { # build_cmd I: the west command of image I
    local i=$1 e=EXTRA_$1
    echo "(cd \$ZEPHYR_WS && west build -p always -b $ZBOARD -d $BUILD/${LABELS[i]} $REPO/driver/samples/zephyr/${SAMPLES[i]} -- -DZEPHYR_MODULES=\$MODS -DYAKOGL_DIR=$REPO ${ARM[*]} ${OVERLAY[*]} ${!e}) > $BUILD/${LABELS[i]}.log"
}
run_cmd() { echo "tools/board.sh am62p run $BUILD/$1/zephyr/zephyr.bin $BUILD/run-$1 pr-$NAME-$1"; }

echo "pr-session.sh: $ONMAIN"
echo "pr-session.sh: session $NAME at $H8, knob $KNOB, cap $CAP, cases $CASES, glmark2 $GLM, one-in-flight $ONE"
if [ "$DRY" = 1 ]; then
    echo "pr-session.sh: dry run: nothing is built or run"
    echo "command: tools/cts/$CMD"
    echo "MODS=\$(tools/cts/zephyr-modules.sh \$ZEPHYR_WS)"
    for i in "${!LABELS[@]}"; do build_cmd "$i"; done
    for l in "${LABELS[@]}"; do run_cmd "$l"; done
    echo "python3 tools/cts/pr-verdict.py $OUT/<label>.log   # when the file exists"
    echo "results: $OUT/"
    exit 0
fi

# the environment, as board-stage.sh
set -a
eval "$(tools/cts/deps-env.sh am62p)" || die "the dependency layout is incomplete (tools/cts/deps-env.sh --check am62p)"
set +a
export RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-nightly}
: "${ZEPHYR_WS:?set ZEPHYR_WS to the west workspace}"
[ -z "${WEST_VENV:-}" ] || PATH=$WEST_VENV/bin:$PATH
command -v west > /dev/null || die "west not on PATH (set WEST_VENV)"
if [ ! -e driver/src/fw/pvr_fw_blob.h ]; then
    mkdir -p driver/src/fw && cp "$PVR_FW_BLOB" driver/src/fw/ || die "cannot copy PVR_FW_BLOB to driver/src/fw/"
fi
export BOARD_HOST=${BOARD_HOST:-${RESOURCE_AM62P_SSH:-}}
[ -n "$BOARD_HOST" ] || die "no board: set BOARD_HOST or lease target:am62p (RESOURCE_AM62P_SSH)"
if [ -z "${BOARD_UART:-}" ] && [ -n "${RESOURCE_AM62P_CONSOLE_HOST:-}" ] && [ -n "${RESOURCE_AM62P_CONSOLE_PORT:-}" ]; then
    BOARD_UART=$RESOURCE_AM62P_CONSOLE_HOST:$RESOURCE_AM62P_CONSOLE_PORT
fi
export BOARD_UART=${BOARD_UART:-}
[ -n "$BOARD_UART" ] || die "no console: set BOARD_UART or lease target:am62p-console"
export BOARD_BOOT=${BOARD_BOOT:-uboot-mmc}
export ZEPHYR_BASE=$ZEPHYR_WS/zephyr
MODS=$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS") || die "zephyr-modules.sh failed"
if [ "$GLM" = 1 ] && [ ! -f "${GLMARK2_SRC:-}/src/default-benchmarks.h" ]; then
    die "set GLMARK2_SRC to the glmark2 checkout"
fi

mkdir -p "$OUT" "$BUILD" || die "cannot create $OUT"
BUILD=$(cd "$BUILD" && pwd)
rc=0
VERDICTS=()
for i in "${!LABELS[@]}"; do
    l=${LABELS[i]}
    e=EXTRA_$i
    echo "pr-session.sh\$ $(build_cmd "$i")"
    # shellcheck disable=SC2086  # the extra arguments are words
    (cd "$ZEPHYR_WS" && west build -p always -b "$ZBOARD" -d "$BUILD/$l" "$REPO/driver/samples/zephyr/${SAMPLES[i]}" -- \
        "-DZEPHYR_MODULES=$MODS" "-DYAKOGL_DIR=$REPO" "${ARM[@]}" "${OVERLAY[@]}" ${!e}) > "$BUILD/$l.log" 2>&1 < /dev/null ||
        die "$l build failed ($BUILD/$l.log)" 1
    # the knob and cap in the image's .config, not only on the command line
    for kv in "CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=$KCFG" "CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=$CAP"; do
        want=$kv
        [ "${kv##*=}" != n ] || want="# ${kv%=n} is not set"
        grep -qxF "$want" "$BUILD/$l/zephyr/.config" || die "$l: .config lacks '$want' (the overlay did not take)" 1
    done
    echo "pr-session.sh\$ $(run_cmd "$l")"
    tools/board.sh am62p run "$BUILD/$l/zephyr/zephyr.bin" "$BUILD/run-$l" "pr-$NAME-$l" | tee "$BUILD/run-$l.out"
    VERDICTS+=("$l: $(grep -a 'verdict=' "$BUILD/run-$l.out" | tail -1)")
    grep -aq 'verdict=done' "$BUILD/run-$l.out" || rc=1
    # the serial log, redacted again as board.sh does (its copy already is)
    if [ -f "$BUILD/run-$l/console.txt" ]; then
        cp "$BUILD/run-$l/console.txt" "$OUT/$l.log"
        for s in "${BOARD_HOST#*@}" "${BOARD_UART%%:*}"; do
            [ ${#s} -ge 6 ] || continue
            esc=$(printf '%s' "$s" | sed 's/[][\\.*^$/|(){}+?]/\\&/g')
            LC_ALL=C sed -i -E "s/$esc/<board-host>/g" "$OUT/$l.log"
        done
        LC_ALL=C sed -i -E 's/\b([0-9]{1,3}\.){3}[0-9]{1,3}\b/192.0.2.0/g; s/\b([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}\b/02:00:00:00:00:00/g' "$OUT/$l.log"
    else
        echo "pr-session.sh: no console log for $l" >&2
        rc=1
    fi
done

# the verdict lines, when the tool exists
PV=
if [ -f tools/cts/pr-verdict.py ]; then
    for l in "${LABELS[@]}"; do
        [ -f "$OUT/$l.log" ] || continue
        PV+="### $l"$'\n'"$(python3 tools/cts/pr-verdict.py "$OUT/$l.log" 2>&1)"$'\n'
    done
else
    PV="tools/cts/pr-verdict.py is absent: no partial-render verdict lines"$'\n'
    echo "pr-session.sh: $PV"
fi

{
    echo "# pr-$NAME ($DAY)"
    echo
    echo "- commit: $H8 ($ONMAIN)"
    echo "- command: \`tools/cts/$CMD\`"
    echo "- Kconfig overlay: \`${OVERLAY[0]#-D}\`, \`${OVERLAY[1]#-D}\`"
    [ "$ONE" = 0 ] || echo "  - glmark2 image also \`CONFIG_GLMARK2_PVR_ONE_KICK_IN_FLIGHT=y\`"
    echo "- cases: $CASES"
    echo "- serial logs: ${LABELS[*]/%/.log}"
    echo
    echo "## board.sh verdict lines"
    echo
    printf '%s\n' "${VERDICTS[@]}" | sed 's/^/    /'
    echo
    echo "## pr-verdict"
    echo
    printf '%s' "$PV" | sed 's/^/    /'
} > "$OUT/README.md"
echo "pr-session.sh: results in $OUT/"
exit "$rc"
