#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/probes/rpi_5/rpi5_wdog/verdicts_selftest.sh
#
# Runs verdicts.sh on inline fixtures (src/main.c's line format) under $TMPDIR:
# good (exit 0), crlf with a timestamp prefix, boot loader noise after the
# stopped line (0), FAIL no reset (3), truncated before the stopped line (1),
# missing checks (3), usage (2). Prints "verdicts_selftest: N checks, M failed".
set -u

here=$(cd "$(dirname "$0")" && pwd)
V="$here/verdicts.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/rpi5_wdog_verdicts.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT INT TERM

checks=0
failed=0
fail() { failed=$((failed + 1)); echo "FAIL: $*"; }
expect_has() {
    checks=$((checks + 1))
    case "$2" in *"$3"*) ;; *) fail "$1: missing '$3'" ;; esac
}
expect_rc() {
    checks=$((checks + 1))
    [ "$2" = "$3" ] || fail "$1: exit $2, wanted $3"
}
run() { out=$(sh "$V" "$1" 2>&1); rc=$?; }

# good <file> [pattern]: a full run; lines matching the pattern are dropped.
good() {
    {
        echo "board: PM watchdog resets the SoC ~40 s after boot (tryboot -> Linux)"
        echo "rpi5_wdog: build abc1234"
        echo "rpi5_wdog: rpi_5, watchdog armed at boot, fed for 30 s"
        echo "rpi5_wdog: feeding, uptime 120 ms"
        echo "rpi5_wdog: feeding, uptime 3121 ms"
        echo "rpi5_wdog: feeding, uptime 6121 ms"
        echo "rpi5_wdog: feeding, uptime 9122 ms"
        echo "rpi5_wdog: feeding, uptime 12122 ms"
        echo "rpi5_wdog: feeding, uptime 15123 ms"
        echo "rpi5_wdog: feeding, uptime 18123 ms"
        echo "rpi5_wdog: feeding, uptime 21124 ms"
        echo "rpi5_wdog: feeding, uptime 24124 ms"
        echo "rpi5_wdog: feeding, uptime 27125 ms"
        echo "rpi5_wdog: feeding, uptime 30125 ms"
        echo "rpi5_wdog: feeding stopped, reset expected within 10 s"
    } | if [ -n "${2:-}" ]; then grep -v "$2"; else cat; fi >"$1"
}

good "$tmp/good"
run "$tmp/good"
expect_rc good "$rc" 0
expect_has good "$out" "ok: build line"
expect_has good "$out" "ok: feeding lines, uptime rising (11 lines)"
expect_has good "$out" "ok: feeding stopped line"
expect_has good "$out" "verdicts: PASS"

sed 's/^/12:00:01 /; s/$/\r/' "$tmp/good" >"$tmp/crlf"
run "$tmp/crlf"
expect_rc crlf "$rc" 0
expect_has crlf "$out" "verdicts: PASS"

# the boot loader's log after the stopped line is ignored
{ cat "$tmp/good"; echo "boot loader: starting"; echo "rpi5_wdogx: noise"; } >"$tmp/after"
run "$tmp/after"
expect_rc after "$rc" 0

{ cat "$tmp/good"; echo "rpi5_wdog: FAIL no reset"; } >"$tmp/fail"
run "$tmp/fail"
expect_rc fail "$rc" 3
expect_has fail "$out" "FAIL: no FAIL no reset"
expect_has fail "$out" "verdicts: FAIL"

grep -v 'feeding stopped' "$tmp/good" >"$tmp/trunc"
run "$tmp/trunc"
expect_rc trunc "$rc" 1
expect_has trunc "$out" "verdicts: incomplete (no end line)"

good "$tmp/nobuild" 'build'
run "$tmp/nobuild"
expect_rc nobuild "$rc" 3
expect_has nobuild "$out" "FAIL: build line"
expect_has nobuild "$out" "verdicts: FAIL"

good "$tmp/noarmed" 'armed at boot'
run "$tmp/noarmed"
expect_rc noarmed "$rc" 3
expect_has noarmed "$out" "FAIL: armed line"

good "$tmp/noannounce" 'PM watchdog'
run "$tmp/noannounce"
expect_rc noannounce "$rc" 3
expect_has noannounce "$out" "FAIL: board announce line once"

good "$tmp/nofeed" 'feeding, uptime'
run "$tmp/nofeed"
expect_rc nofeed "$rc" 3
expect_has nofeed "$out" "FAIL: feeding lines, uptime rising (0 lines)"

# uptime going backwards: the board rebooted mid-run
sed 's/uptime 15123/uptime 100/' "$tmp/good" >"$tmp/flat"
run "$tmp/flat"
expect_rc flat "$rc" 3
expect_has flat "$out" "FAIL: feeding lines, uptime rising"

: >"$tmp/empty"
run "$tmp/empty"
expect_rc empty "$rc" 1

out=$(sh "$V" 2>&1)
expect_rc usage-noarg "$?" 2
out=$(sh "$V" "$tmp/nonexistent" 2>&1)
expect_rc usage-unreadable "$?" 2

echo "verdicts_selftest: $checks checks, $failed failed"
[ "$failed" -eq 0 ]
