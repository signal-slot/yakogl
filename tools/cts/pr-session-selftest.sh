#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# pr-session-selftest.sh -- pr-session.sh's --dry-run output and refusals.
# Host only: no board, no toolchain, no environment. Not yet listed in
# tools/cts/cts-selftest.sh; add it there as the other selftests are.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
S=$HERE/pr-session.sh
fails=0
fail() { echo "FAIL: $1" >&2; fails=$((fails + 1)); }
has() { grep -qE -- "$2" <<< "$1" || fail "$3: no '$2' in: $1"; }
lacks() { ! grep -qE -- "$2" <<< "$1" || fail "$3: unexpected '$2'"; }

# a HEAD off origin/main is refused (only testable when this checkout is off it)
if ! git -C "$HERE" merge-base --is-ancestor HEAD origin/main 2> /dev/null; then
    out=$(env -i PATH="$PATH" "$S" t --knob off --dry-run 2>&1) && fail "no refusal off main"
    has "$out" 'not on origin/main' "refusal"
fi

run() { env -i PATH="$PATH" ${PR_CONFIGS_DIR:+PR_CONFIGS_DIR="$PR_CONFIGS_DIR"} BOARD_HOST=secret-user@10.9.8.7 "$S" "$@" --allow-local --dry-run 2>&1; }

off=$(run t --knob off) || fail "knob off failed"
has "$off" 'on origin/main' "off: the main line"
has "$off" 'CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=n' "off"
has "$off" 'CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=0' "off: cap"
has "$off" 'CONFIG_GL_TEST_CASES="175"' "off: default case"
has "$off" 'tools/board.sh am62p run .*gltest' "off: run"
has "$off" 'nothing is built or run' "off: dry"
lacks "$off" 'secret|10\.9\.8\.7' "off: the board address"
lacks "$off" 'GLMARK2' "off: no glmark2"

on=$(run t --knob on) || fail "knob on failed"
has "$on" 'CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=y' "on"

cap=$(run t --knob on --cap-pages 512 --cases '175 176') || fail "cap failed"
has "$cap" 'CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=512' "cap 512"
has "$cap" 'CONFIG_GL_TEST_CASES="175,176"' "cases"
has "$cap" '--cap-pages 512' "cap in the command"

glm=$(run t --knob on --glmark2 --one-in-flight) || fail "glmark2 failed"
has "$glm" 'CONFIG_GLMARK2_PVR_ONE_KICK_IN_FLIGHT=y' "one in flight"
has "$glm" 'tools/board.sh am62p run .*glmark2' "glmark2 run"

# case 217 legs (acceptance c2); the fragments are a parallel item's, so the
# selftest points PR_CONFIGS_DIR at stand-ins
FD=$(mktemp -d) || exit 1
trap 'rm -rf "$FD"' EXIT
: > "$FD/gl_test-217-off.conf"
: > "$FD/gl_test-217-on.conf"
leg() { PR_CONFIGS_DIR=$FD run "$@"; }
for k in off on; do
    l=$(leg t --leg 217-$k) || fail "leg 217-$k failed"
    if [ $k = on ]; then K=y; ex=pr; else K=n; ex=stop; fi
    has "$l" "CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER=$K" "217-$k: knob"
    has "$l" 'CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES=512' "217-$k: cap"
    has "$l" 'CONFIG_GL_TEST_CASES="217"' "217-$k: case"
    has "$l" "-DEXTRA_CONF_FILE=$FD/gl_test-217-$k.conf" "217-$k: fragment"
    has "$l" 'CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y' "217-$k: watchdog self-arm"
    has "$l" 'CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n' "217-$k: watchdog feed"
    has "$l" 'tools/board.sh am62p run .*gltest' "217-$k: run"
    has "$l" "pr-verdict.py --expect $ex .*<label>[.]log 217" "217-$k: verdict command"
    has "$l" "pr-session.sh t --leg 217-$k" "217-$k: command line"
    lacks "$l" 'GLMARK2|secret|10[.]9[.]8[.]7' "217-$k: no glmark2/board address"
done
# a missing fragment: a clear refusal in a real run, a warning in a dry run
miss=$(env -i PATH="$PATH" PR_CONFIGS_DIR="$FD/none" "$S" t --leg 217-on --allow-local 2>&1) && fail "a missing fragment was accepted"
has "$miss" 'gl_test-217-on.conf is missing' "missing fragment"
miss=$(PR_CONFIGS_DIR=$FD/none run t --leg 217-on) || fail "dry run with a missing fragment failed"
has "$miss" 'warning: .*gl_test-217-on.conf is missing' "missing fragment warning"
leg t --leg 217-on --knob off > /dev/null && fail "--leg with a disagreeing knob was accepted"
leg t --leg 217-off --cap-pages 4 > /dev/null && fail "--leg with another cap was accepted"
leg t --leg 217-on --cases 175 > /dev/null && fail "--leg with --cases was accepted"
leg t --leg 218-on > /dev/null && fail "a bad leg was accepted"
leg t --leg 217-on --knob on --cap-pages 512 > /dev/null || fail "--leg with agreeing options failed"

run t --knob maybe > /dev/null && fail "a bad knob was accepted"
run t > /dev/null && fail "a missing knob was accepted"
run t --knob on --cap-pages x > /dev/null && fail "a bad cap was accepted"
run t --knob on --one-in-flight > /dev/null && fail "--one-in-flight without --glmark2 was accepted"

if [ "$fails" = 0 ]; then
    echo "pr-session-selftest: ok"
else
    echo "pr-session-selftest: $fails failed" >&2
    exit 1
fi
