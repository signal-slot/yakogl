#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi_5/rpi5_wdog/verdicts.sh <console.log>
#
# Reads the console log of the rpi5_wdog image (src/main.c) and prints one
# line per check, then a last line `verdicts: PASS` or `verdicts: FAIL ...`.
# The image's lines (others, and any prefix before `rpi5_wdog: `, are ignored):
#
#   rpi5_wdog: build <id>
#   rpi5_wdog: <board>, watchdog armed at boot, fed for <n> s
#   rpi5_wdog: feeding, uptime <ms> ms         (one per feed interval)
#   board: PM watchdog resets the SoC ~<n> s after boot (tryboot -> Linux)
#   rpi5_wdog: feeding stopped, reset expected within <n> s
#   rpi5_wdog: FAIL no reset                   (the watchdog did not reset)
#
# The reset itself shows as silence after the stopped line (the boot loader's
# log follows); a log cut there is a PASS. Checks: build line, armed line,
# the board's announce line once, at least one feeding line with uptime rising,
# the stopped line, no `FAIL no reset`.
#
# Exit status: 0 PASS, 1 no end line (neither the stopped line nor FAIL),
# 2 usage, 3 FAIL.
#
# Self-test: tools/probes/rpi_5/rpi5_wdog/verdicts_selftest.sh
set -u

if [ $# -ne 1 ]; then
    echo "usage: ${0##*/} <console.log>" >&2
    exit 2
fi
if [ ! -r "$1" ] || [ -d "$1" ]; then
    echo "${0##*/}: cannot read $1" >&2
    exit 2
fi

tr -d '\r' <"$1" | awk '
function chk(ok, name, detail) {
    printf "%s: %s%s\n", ok ? "ok" : "FAIL", name, detail
    if (!ok)
        bad++
}
/board: PM watchdog resets/ { announce++ }
{
    i = index($0, "rpi5_wdog: ")
    if (i == 0)
        next
    l = substr($0, i + 11)
}
l ~ /^build [^ ]/ { build++ }
l ~ /, watchdog armed at boot, fed for [0-9]+ s$/ { armed++ }
l ~ /^feeding, uptime [0-9]+ ms$/ {
    feeds++
    t = l
    sub(/^feeding, uptime /, "", t)
    sub(/ ms$/, "", t)
    if (feeds > 1 && t + 0 <= last + 0)
        flat++
    last = t
}
l ~ /^feeding stopped, reset expected within [0-9]+ s$/ { stopped++ }
l == "FAIL no reset" { noreset++ }
END {
    chk(build >= 1, "build line", "")
    chk(armed >= 1, "armed line", "")
    chk(announce == 1, "board announce line once", " (" announce + 0 ")")
    chk(feeds >= 1 && flat == 0, "feeding lines, uptime rising", " (" feeds + 0 " lines)")
    chk(stopped >= 1, "feeding stopped line", "")
    chk(noreset == 0, "no FAIL no reset", "")
    if (noreset > 0) {
        print "verdicts: FAIL (the watchdog did not reset the board; power-cycle it)"
        exit 3
    }
    if (stopped == 0) {
        print "verdicts: incomplete (no end line)"
        exit 1
    }
    if (bad > 0) {
        print "verdicts: FAIL (" bad " check(s) failed)"
        exit 3
    }
    print "verdicts: PASS"
    exit 0
}'
