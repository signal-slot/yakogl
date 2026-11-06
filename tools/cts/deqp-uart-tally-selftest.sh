#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/deqp-uart-tally-selftest.sh
#
# Host-only check of tools/cts/deqp-uart-tally.py on synthetic captures shaped
# like the Armadillo-X2's UART2 (board.sh x2 run with BOARD_UART): the Linux
# kexec / crash-kernel lines before the first Zephyr line, CRLF line ends, a
# Linux or U-Boot line between dEQP lines after a watchdog reset, a capture
# cut mid-line or before the closing summary (the 600 s watchdog ended the
# run: lost, never a pass), and a clean run whose failing names and counts
# must come out exactly. Nothing here is a real capture.
set -u

HERE=$(dirname "$0")
TALLY=$HERE/deqp-uart-tally.py
D=$(mktemp -d "${TMPDIR:-.}/deqp-tally.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
# chk <label> <command...>: ok when the command succeeds
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }

LINUX='U-Boot 2024.04 (Jan 01 2026)
Starting kernel ...
[    0.000000] Booting Linux on physical CPU 0x0
[    0.000000] crashkernel reserved: 0x0000000060000000 - 0x0000000070000000 (256 MB)
kexec_core: Starting new kernel
Bye!'
ZBOOT='*** Booting Zephyr OS build v4.1.0 ***
gc_wdog_arm: armed WCR 0x0000 -> 0x0f84 (was stopped), WMCR 0x0000, flags 0x1: expiry 600000 ms, fed for 590 s
deqp: selector 0
deqp: case filter '"'"'dEQP-GLES3.functional.*'"'"
CASES='deqp: Pass dEQP-GLES3.functional.a.one
deqp: Fail dEQP-GLES3.functional.a.two
deqp: Pass dEQP-GLES3.functional.b.three
deqp: NotSupported dEQP-GLES3.functional.b.four
deqp: Crash dEQP-GLES3.functional.c.five
deqp: Pass dEQP-GLES3.functional.c.six'
DONE='deqp: done: 3 passed, 1 failed, 1 not supported, 0 warnings, complete=1
deqp: summary: 6 cases begun, 6 with a result, 0 without'

# tally <file> -> $OUT, $RC
tally() { OUT=$(python3 "$TALLY" "$1" 2>&1); RC=$?; }
has() { printf '%s\n' "$OUT" | grep -qF -- "$1"; }
hasre() { printf '%s\n' "$OUT" | grep -Eq -- "$1"; }
rc_is() { [ "$RC" -eq "$1" ]; }
failing() { printf '%s\n' "$OUT" | grep -E '^(Fail|Crash) '; }
names_exact() { [ "$(failing)" = "$want" ]; }
no_trouble() { ! has MISSING && ! has INCOMPLETE && ! has FATAL; }

# (e) a clean run, LF: the reference every other shape is compared with.
printf '%s\n%s\n%s\n' "$ZBOOT" "$CASES" "$DONE" > "$D/clean.txt"
tally "$D/clean.txt"
want='Fail dEQP-GLES3.functional.a.two
Crash dEQP-GLES3.functional.c.five'
chk "clean run: exit 0" rc_is 0
chk "clean run: no MISSING / INCOMPLETE / FATAL" no_trouble
chk "clean run: done line passed through" has 'deqp: done: 3 passed, 1 failed, 1 not supported, 0 warnings, complete=1'
chk "clean run: summary line passed through" has 'deqp: summary: 6 cases begun, 6 with a result, 0 without'
chk "clean run: group a is 1 pass 1 fail" hasre '^a +1 +1 +0 +0$'
chk "clean run: TOTAL 3 pass 1 fail 1 n/s 1 other" hasre '^TOTAL +3 +1 +1 +1$'
chk "clean run: the failing names come out exactly, nothing else" names_exact

# (a) Linux kexec and crash-kernel lines before the first Zephyr line.
printf '%s\n%s\n%s\n%s\n' "$LINUX" "$ZBOOT" "$CASES" "$DONE" > "$D/linux-first.txt"
tally "$D/linux-first.txt"
chk "Linux lines first: exit 0" rc_is 0
chk "Linux lines first: no MISSING / INCOMPLETE / FATAL" no_trouble
chk "Linux lines first: TOTAL as the clean run" hasre '^TOTAL +3 +1 +1 +1$'
chk "Linux lines first: the same failing names" names_exact

# (b) CRLF line ends, and a NUL of the raw stream inside a line.
sed 's/$/\r/' "$D/linux-first.txt" > "$D/crlf.txt"
sed -i '9s/^/\x00/' "$D/crlf.txt"
tally "$D/crlf.txt"
chk "CRLF + NUL: exit 0" rc_is 0
chk "CRLF + NUL: no MISSING / INCOMPLETE / FATAL" no_trouble
chk "CRLF + NUL: TOTAL as the clean run" hasre '^TOTAL +3 +1 +1 +1$'
chk "CRLF + NUL: the same failing names, no CR in them" names_exact

# (c) Linux / U-Boot lines between dEQP lines: the watchdog reset the SoC
# mid-run and the next boot's output landed inside the capture.
HEAD3=$(printf '%s\n' "$CASES" | head -3)
TAIL3=$(printf '%s\n' "$CASES" | tail -3)
printf '%s\n%s\n%s\n%s\n' "$ZBOOT" "$HEAD3" "$LINUX" "$TAIL3" > "$D/interleaved-nosum.txt"
tally "$D/interleaved-nosum.txt"
chk "Linux between dEQP lines, no summary: exit 1" rc_is 1
chk "Linux between dEQP lines, no summary: MISSING" has MISSING
chk "Linux between dEQP lines: all 6 cases still read" hasre '^TOTAL +3 +1 +1 +1$'
printf '%s\n%s\n%s\n%s\n%s\n' "$ZBOOT" "$HEAD3" "$LINUX" "$TAIL3" "$DONE" > "$D/interleaved.txt"
tally "$D/interleaved.txt"
chk "Linux between dEQP lines, with the summary: exit 0" rc_is 0
chk "Linux between dEQP lines, with the summary: names exact" names_exact

# (d) the capture ends before the summary, or in the middle of a line (the
# 600 s watchdog ended the run): lost, never a pass.
printf '%s\n%s\n' "$ZBOOT" "$CASES" > "$D/nosummary.txt"
tally "$D/nosummary.txt"
chk "no summary: exit 1" rc_is 1
chk "no summary: MISSING" has 'MISSING: no summary line'
printf '%s\n%s\n%s' "$ZBOOT" "$CASES" 'deqp: Pass dEQP-GLES3.functional.d.sev' > "$D/midline.txt"
tally "$D/midline.txt"
chk "ends mid case line: exit 1" rc_is 1
chk "ends mid case line: MISSING" has MISSING
chk "ends mid case line: the cut name is not counted as a case" hasre '^TOTAL +3 +1 +1 +1$'
printf '%s\n%s\n%s\n%s' "$ZBOOT" "$CASES" 'deqp: done: 3 passed, 1 failed, 1 not supported, 0 warnings, complete=1' 'deqp: summary: 6 cases begun, 6 with a res' > "$D/cutsummary.txt"
tally "$D/cutsummary.txt"
chk "summary cut mid-line: exit 1" rc_is 1
chk "summary cut mid-line: MISSING" has MISSING
printf '%s\n%s\n%s\n' "$ZBOOT" "$CASES" "$LINUX" > "$D/reset-after.txt"
tally "$D/reset-after.txt"
chk "reset into Linux after the cases, no summary: exit 1" rc_is 1
chk "reset into Linux after the cases, no summary: MISSING" has MISSING
printf '%s\n%s\n%s\n' "$ZBOOT" "$HEAD3" "$DONE" > "$D/lostlines.txt"
tally "$D/lostlines.txt"
chk "case lines lost, summary seen: exit 1" rc_is 1
chk "case lines lost, summary seen: INCOMPLETE" has INCOMPLETE
chk "case lines lost, summary seen: Lost in capture, 3 cases" has 'Lost in capture: 3 cases'
printf '%s\n%s\n%s\n' "$ZBOOT" "$CASES" 'deqp: fatal: die' > "$D/fatal.txt"
tally "$D/fatal.txt"
chk "fatal: exit 1" rc_is 1
chk "fatal: FATAL" has FATAL
printf '%s\n' "$LINUX" > "$D/linux-only.txt"
tally "$D/linux-only.txt"
chk "Linux only, no Zephyr line: exit 1" rc_is 1
chk "Linux only, no Zephyr line: MISSING" has MISSING

printf '%d ok, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
