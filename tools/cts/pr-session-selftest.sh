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

run() { env -i PATH="$PATH" BOARD_HOST=secret-user@10.9.8.7 "$S" "$@" --allow-local --dry-run 2>&1; }

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
