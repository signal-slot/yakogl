#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi5_display/route_choice.sh [console.log | verdicts-output]
#
# Picks the Pi 5 display route (driver/docs/rpi5-display.md) from the U3
# verdict lines, in order of cost, and prints exactly one line:
#
#   u3 route: 1 <reason>   the firmware scanout runs and adopt-readback PASS
#   u3 route: 2 <reason>   else 'u3 mailbox-fb: PASS'
#   u3 route: 3 <reason>   else (program HVS6, pixel valve and HDMI ourselves)
#   u3 route: undecided <what>
#
# Input is a console log or the output of verdicts.sh, from the file argument
# or stdin. The first occurrence of each line is read by verdicts.sh (its
# '  [n] <line>' echo is accepted too); only the lines the rule needs have to
# be there: 'u3 scanout', 'u3 adopt-readback' (when the scanout runs) and
# 'u3 mailbox-fb' (when route 1 does not hold). Other lines are ignored.
#
# Undecided (exit 2): a needed line is missing, a needed line occurs twice
# with different values, or 'adopt-readback: PASS' next to 'scanout: stopped'.
# An unreadable file or a wrong argument count is also exit 2, with a usage
# message on stderr and no route line. A decision is exit 0.
#
# Self-test: tools/probes/rpi5_display/route_choice_selftest.sh
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
V=$HERE/verdicts.sh

[ $# -le 1 ] || { echo "usage: ${0##*/} [console.log]" >&2; exit 2; }
T=$(mktemp -d "${TMPDIR:-/tmp}/route_choice.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
if [ $# -eq 1 ]; then
    [ -r "$1" ] || { echo "${0##*/}: cannot read: $1" >&2; exit 2; }
    src=$1
else
    src=/dev/stdin
fi
# normalise: CR off, the verdicts.sh '  [n] ' echo prefix off
tr -d '\r' < "$src" | sed -E 's/^[[:space:]]*\[[0-9]\] //' > "$T/in.log"

undecided() { echo "u3 route: undecided $1"; exit 2; }
decide() { echo "u3 route: $1 $2"; exit 0; }

# the first occurrence of each line, through verdicts.sh
FIRST=$(bash "$V" "$T/in.log" 2>/dev/null | sed -nE 's/^  \[[1-9]\] //p')
first_val() { # <key> <value regex>
    { grep -E "^$1: " <<< "$FIRST" | head -n 1 | grep -oE "$2" | head -n 1; } || true
}
# the number of distinct values of a key in the input (2 = contradictory)
distinct() { # <key> <value regex>
    { grep -E "^$1: ($2)( |[[:space:]]*\$)" "$T/in.log" | grep -oE "$2" | sort -u | wc -l; } || true
}

SC=$(first_val 'u3 scanout' 'running|stopped')
AD=$(first_val 'u3 adopt-readback' 'PASS|FAIL')
MB=$(first_val 'u3 mailbox-fb' 'PASS|FAIL')

bad=""
[ "$(distinct 'u3 scanout' 'running|stopped')" -gt 1 ] && bad="$bad scanout:running+stopped"
[ "$(distinct 'u3 adopt-readback' 'PASS|FAIL')" -gt 1 ] && bad="$bad adopt-readback:PASS+FAIL"
[ "$(distinct 'u3 mailbox-fb' 'PASS|FAIL')" -gt 1 ] && bad="$bad mailbox-fb:PASS+FAIL"
[ -n "$bad" ] && undecided "contradictory lines:$bad"
if [ "$SC" = stopped ] && [ "$AD" = PASS ]; then
    undecided "contradictory lines: scanout:stopped adopt-readback:PASS"
fi

[ -n "$SC" ] || undecided "missing: u3 scanout"
if [ "$SC" = running ]; then
    [ -n "$AD" ] || undecided "missing: u3 adopt-readback"
    [ "$AD" = PASS ] && decide 1 "firmware scanout running, adopt-readback PASS"
fi
[ -n "$MB" ] || undecided "missing: u3 mailbox-fb"
if [ "$MB" = PASS ]; then
    [ "$SC" = running ] && decide 2 "adopt-readback FAIL, mailbox-fb PASS"
    decide 2 "scanout stopped, mailbox-fb PASS"
fi
[ "$SC" = running ] && decide 3 "adopt-readback FAIL, mailbox-fb FAIL"
decide 3 "scanout stopped, mailbox-fb FAIL"
