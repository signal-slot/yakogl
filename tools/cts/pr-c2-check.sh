#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pr-c2-check.sh [LOGDIR]
#
# Mechanical check of acceptance c2 of the partial-render plan (issue #3):
# the forced-PR case (gl_test case 217 forcedpr, list capped at 512 pages)
# reproduces the stop with the knob off and, with the knob on, completes with
# a pixel-for-pixel read-back and 0 resets. It scans LOGDIR/*-pr-*/*.log
# (LOGDIR defaults to tools/cts/log) with tools/cts/pr-verdict.py:
#
#   knob-off leg  a log whose --expect stop verdict is OK (a 'render:
#                 stopped' line of cause 2 or 4, no knob-on wait line) and
#                 that holds no 'forcedpr PASS' line;
#   knob-on leg   a log whose --expect pr verdict for case 217 is OK, whose
#                 forcedpr line is PASS with partial_renders >= 1 and
#                 recoveries 0 (a FAIL pixels / FAIL depth run is no pass).
#
# Prints 'pr-c2-check: OK knob-off <record> knob-on <record>' (records are
# <dir>/<file>) and exits 0, or 'pr-c2-check: FAIL <missing leg(s)>' and exits
# 1. Exit 2: LOGDIR or pr-verdict.py missing.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
LOGDIR=${1:-$HERE/log}
TOOL=$HERE/pr-verdict.py

[ -d "$LOGDIR" ] || { echo "pr-c2-check: missing $LOGDIR" >&2; exit 2; }
[ -f "$TOOL" ] || { echo "pr-c2-check: missing $TOOL" >&2; exit 2; }

off=
on=
for f in "$LOGDIR"/*-pr-*/*.log; do
    [ -f "$f" ] || continue
    rec=$(basename "$(dirname "$f")")/$(basename "$f")
    if [ -z "$off" ]; then
        if out=$(python3 -I "$TOOL" --expect stop "$f" 217 175 2>/dev/null) &&
            ! printf '%s\n' "$out" | grep -q '^pr-verdict: forcedpr PASS'; then
            off=$rec
        fi
    fi
    if [ -z "$on" ]; then
        if out=$(python3 -I "$TOOL" --expect pr "$f" 217 2>/dev/null) &&
            printf '%s\n' "$out" | grep -Eq '^pr-verdict: forcedpr PASS .*partial_renders=([1-9][0-9]*) recoveries=0$'; then
            on=$rec
        fi
    fi
done

missing=
[ -n "$off" ] || missing="knob-off leg missing (no record with the stop, cause 2 or 4)"
if [ -z "$on" ]; then
    [ -z "$missing" ] || missing="$missing; "
    missing="${missing}knob-on leg missing (no record with case 217 PASS, partial renders >= 1, HWR 0)"
fi
if [ -n "$missing" ]; then
    echo "pr-c2-check: FAIL $missing"
    exit 1
fi
echo "pr-c2-check: OK knob-off $off knob-on $on"
