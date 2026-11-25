#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Check a glmark2 log directory before it is committed:
#   c1 layout:    tally-table.txt, tally-results-row.txt and state-compare.txt
#                 are in the directory;
#   c2 redaction: no file matches an IPv4 address, a MAC, /home/, a
#                 /dev/ttyUSB* or /dev/ttyACM* path or an @host. mail form
#                 (grep -rEna, binary files included).
# The matched text is never printed, only file:line. A grep error (status 2,
# for example an unreadable file) is a c2 failure, not a pass.
#
#   tools/cts/glmark2-log-check.sh <logdir>
#
# One verdict line per check, then "glmark2-log-check: ok" (exit 0) or
# "glmark2-log-check: FAIL <n>" (exit 1); exit 2 on usage.
set -uo pipefail
if [ $# -ne 1 ] || [ ! -d "$1" ]; then
    echo "usage: ${0##*/} <logdir>" >&2
    exit 2
fi
dir=$1
fail=0

missing=()
for f in tally-table.txt tally-results-row.txt state-compare.txt; do
    [ -f "$dir/$f" ] || missing+=("$f")
done
if [ ${#missing[@]} -eq 0 ]; then
    echo "c1 layout: ok"
else
    echo "c1 layout: FAIL missing ${missing[*]}"
    fail=$((fail + 1))
fi

pat='([0-9]{1,3}\.){3}[0-9]{1,3}|([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}|/home/|/dev/tty(USB|ACM)|@[a-z0-9-]+\.'
# Run inside the directory so the output holds relative paths, not a host path.
out=$(cd "$dir" && grep -rEna -e "$pat" . 2>/dev/null)
rc=$?
if [ "$rc" -eq 0 ]; then
    echo "c2 redaction: FAIL $(printf '%s\n' "$out" | wc -l) match(es), text masked"
    # file:line only; the matched text is dropped.
    printf '%s\n' "$out" | cut -d: -f1,2 | sed 's/^/  /'
    fail=$((fail + 1))
elif [ "$rc" -eq 1 ]; then
    echo "c2 redaction: ok"
else
    echo "c2 redaction: FAIL grep error (status $rc), files not all checked"
    if [ -n "$out" ]; then
        printf '%s\n' "$out" | cut -d: -f1,2 | sed 's/^/  /'
    fi
    fail=$((fail + 1))
fi

if [ "$fail" -eq 0 ]; then
    echo "glmark2-log-check: ok"
    exit 0
fi
echo "glmark2-log-check: FAIL $fail"
exit 1
