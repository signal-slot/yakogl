#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/probes/rpi_5/rpi5_hello/verdicts_selftest.sh
#
# Feeds verdicts.sh inline rpi5_hello logs and checks its line and exit code:
# good (0), short count (3), fast and slow count (3), truncated (1), CR line
# ends with a timestamp prefix (0), usage (2). Prints
# "verdicts_selftest: N checks, M failed"; exits 0 when none failed, never 77.
set -u

here=$(cd "$(dirname "$0")" && pwd)
V="$here/verdicts.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/rpi5_hello_verdicts.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT INT TERM

checks=0
failed=0

head_lines() {
    echo "rpi5_hello: build 0123abcd"
    echo "rpi5_hello: rpi_5, console at 115200 baud, image RAM 0x200000 + 0x2000000"
    echo "rpi5_hello: 1 s (uptime 1010 ms)"
    echo "rpi5_hello: 2 s (uptime 2010 ms)"
    echo "rpi5_hello: 3 s (uptime 3010 ms)"
}

# check <name> <exit> <line>: the log is $tmp/<name>.log
check() {
    checks=$((checks + 1))
    out=$(sh "$V" "$tmp/$1.log" 2>/dev/null)
    rc=$?
    if [ "$rc" -ne "$2" ]; then
        failed=$((failed + 1))
        echo "FAIL: $1: exit $rc, expected $2"
    fi
    if [ "$out" != "$3" ]; then
        failed=$((failed + 1))
        echo "FAIL: $1: got '$out', expected '$3'"
    fi
}

{ head_lines; echo "rpi5_hello: counted 3 s in 3012 ms"; } >"$tmp/good.log"
check good 0 "verdicts: PASS (counted 3 s in 3012 ms)"

{ head_lines; echo "rpi5_hello: counted 2 s in 3012 ms"; } >"$tmp/short.log"
check short 3 "verdicts: FAIL (counted 2 s, expected 3 s)"

{ head_lines; echo "rpi5_hello: counted 3 s in 2500 ms"; } >"$tmp/fast.log"
check fast 3 "verdicts: FAIL (3 s took 2500 ms, expected 3000..4000 ms)"

{ head_lines; echo "rpi5_hello: counted 3 s in 9000 ms"; } >"$tmp/slow.log"
check slow 3 "verdicts: FAIL (3 s took 9000 ms, expected 3000..4000 ms)"

head_lines | head -4 >"$tmp/truncated.log"
check truncated 1 "verdicts: no end line (no 'rpi5_hello: counted' line in the log)"

{ head_lines; echo "rpi5_hello: counted 3 s in 3012 ms"; } | sed 's/^/[00:00:01.000,000] /; s/$/\r/' >"$tmp/crlf.log"
check crlf 0 "verdicts: PASS (counted 3 s in 3012 ms)"

checks=$((checks + 1))
sh "$V" >/dev/null 2>&1
[ $? -eq 2 ] || { failed=$((failed + 1)); echo "FAIL: usage: no argument did not exit 2"; }
checks=$((checks + 1))
sh "$V" "$tmp/does-not-exist" >/dev/null 2>&1
[ $? -eq 2 ] || { failed=$((failed + 1)); echo "FAIL: usage: unreadable file did not exit 2"; }

echo "verdicts_selftest: $checks checks, $failed failed"
[ "$failed" -eq 0 ]
