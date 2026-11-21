#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2-display-record-selftest.sh - mp2-display-record.py on synthetic logs.
#   bash tools/cts/mp2-display-record-selftest.sh
# Cases: PASS, FAIL, a judge FAIL, a missing result line, a missing hold
# line, a missing judge verdict, and a log carrying an address, host, tty
# path and USB serial that must be redacted. Exits 0 when all match.
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tool=$here/mp2-display-record.py
work=$(mktemp -d "$TMPDIR/mp2-display-record.XXXXXX" 2>/dev/null || mktemp -d) || exit 2
trap 'rm -rf "$work"' EXIT
bad=0
ok() { echo "ok   $1"; }
ko() { echo "FAIL $1"; bad=$((bad + 1)); }

mklog() { # file, result word, extra line
    cat > "$1" <<LOG
runner: tools/board-stm32mp2.sh build abc1234
mp2_display: build x, hold 20 s
mp2_display: chain step done err 0, hpd 1, edid 1 (header 00 ff, ext 1), choice 0
mp2_display: mode 1280x720 74250000 Hz h 110/40/220 v 5/5/20 flags 0x5, hdmi vic 4
mp2_display: $2: 1280x720 XRGB8888 3 frames
mp2_display: hold: 19 frames ok, 0 failed
hold: result seen at 2026-11-21T10:00:00Z, holding 20s
stock boot files: unchanged since install
board-stm32mp2.sh stm32mp257f-dk mp2_display rc=0 verdict=done boot=uboot-sd row boot_verdict=complete result=pass:mp2_display back_after=41s
$3
LOG
}
echo 'colorbars: PASS: 8 bars white yellow cyan green magenta red blue black' > "$work/j.pass"
echo 'colorbars: FAIL: no signal' > "$work/j.fail"

mklog "$work/pass.log" PASS ""
out=$("$tool" "$work/pass.log" --judge "$work/j.pass" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'abc1234' <<<"$out" && grep -q '1280x720' <<<"$out" && grep -q 'source: EDID' <<<"$out" \
    && grep -q 'held 20 s' <<<"$out" && grep -q 'colorbars: PASS' <<<"$out" \
    && grep -q 'unchanged since install' <<<"$out" && grep -q 'mp2_display, PASS' <<<"$out"; then
    ok pass
else ko "pass (rc $rc)"; fi

mklog "$work/fail.log" FAIL ""
out=$("$tool" "$work/fail.log" --judge "$work/j.pass" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'mp2_display, FAIL' <<<"$out" && grep -q 'mp2_display: FAIL' <<<"$out"; then ok fail; else ko "fail (rc $rc)"; fi

out=$("$tool" "$work/pass.log" --judge "$work/j.fail" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'mp2_display, FAIL' <<<"$out"; then ok judge-fail; else ko "judge-fail (rc $rc)"; fi

grep -v 'mp2_display: PASS' "$work/pass.log" > "$work/noresult.log"
out=$("$tool" "$work/noresult.log" 2>&1); rc=$?
if [ $rc = 1 ] && grep -q 'refused' <<<"$out" && grep -q 'PASS|FAIL' <<<"$out"; then ok missing-result; else ko "missing-result (rc $rc)"; fi

grep -v '^hold:' "$work/pass.log" > "$work/nohold.log"
out=$("$tool" "$work/nohold.log" 2>&1); rc=$?
if [ $rc = 1 ] && grep -q 'hold' <<<"$out"; then ok missing-hold; else ko "missing-hold (rc $rc)"; fi

echo 'nothing here' > "$work/nojudge.txt"
out=$("$tool" "$work/pass.log" --judge "$work/nojudge.txt" 2>&1); rc=$?
if [ $rc = 1 ] && grep -q 'judge' <<<"$out"; then ok missing-judge; else ko "missing-judge (rc $rc)"; fi

mklog "$work/red.log" PASS "ssh root@192.168.7.42 console /dev/ttyUSB3 usb-FTDI_Quad_RS232-HS_FT9ABCDE-if02 host dk1.example.net SerialNumber: A50285BI"
sed -i 's|^stock boot files: .*|stock boot files: no record (192.168.7.42 /dev/ttyACM0 not installed)|' "$work/red.log"
out=$("$tool" "$work/red.log" 2>&1); rc=$?
if [ $rc = 0 ] && ! grep -Eq '192\.168|ttyUSB|ttyACM|FT9ABCDE|dk1\.example|A50285BI|root@' <<<"$out" && grep -q '<ip>' <<<"$out"; then
    ok redaction
else ko "redaction (rc $rc)"; echo "$out"; fi

echo 'root@10.1.2.3 only' > "$work/junk.log"
out=$("$tool" "$work/junk.log" --commit deadbee 2>&1); rc=$?
if [ $rc = 1 ] && ! grep -q '10\.1\.2\.3' <<<"$out"; then ok junk-refuses; else ko "junk-refuses (rc $rc)"; fi

if [ $bad = 0 ]; then echo "mp2-display-record-selftest: all ok"; else echo "mp2-display-record-selftest: $bad failed"; fi
exit $((bad != 0))
