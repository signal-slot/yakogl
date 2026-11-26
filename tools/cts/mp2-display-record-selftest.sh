#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2-display-record-selftest.sh - mp2-display-record.py on synthetic logs.
#   bash tools/cts/mp2-display-record-selftest.sh
# Cases: PASS, FAIL, a judge FAIL, a missing result line, a blind run's hold (meta, option, console, unknown)
# line, a missing judge verdict, and a log carrying an address, host, tty
# path and USB serial that must be redacted; the crc and per-check lines, a
# FAIL naming a check, and two KVM verdicts with offsets.
# Verdict-line shapes: resets=/record= before or after back_after=, record:<KIND>,
# bad_record, lifetime, unknown trailing fields. Exits 0 when all match.
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tool() { python3 -I "$here/mp2-display-record.py" "$@"; }
work=$(mktemp -d "${TMPDIR:-/tmp}/mp2-display-record.XXXXXX") || exit 2
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
out=$(tool "$work/pass.log" --judge "$work/j.pass" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'abc1234' <<<"$out" && grep -q '1280x720' <<<"$out" && grep -q 'source: EDID' <<<"$out" \
    && grep -q 'held 20 s' <<<"$out" && grep -q 'colorbars: PASS' <<<"$out" \
    && grep -q 'unchanged since install' <<<"$out" && grep -q 'mp2_display, PASS' <<<"$out"; then
    ok pass
else ko "pass (rc $rc)"; fi

mklog "$work/fail.log" FAIL ""
out=$(tool "$work/fail.log" --judge "$work/j.pass" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'mp2_display, FAIL' <<<"$out" && grep -q 'mp2_display: FAIL' <<<"$out"; then ok fail; else ko "fail (rc $rc)"; fi

out=$(tool "$work/pass.log" --judge "$work/j.fail" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'mp2_display, FAIL' <<<"$out"; then ok judge-fail; else ko "judge-fail (rc $rc)"; fi

grep -v 'mp2_display: PASS' "$work/pass.log" > "$work/noresult.log"
out=$(tool "$work/noresult.log" 2>&1); rc=$?
if [ $rc = 1 ] && grep -q 'refused' <<<"$out" && grep -q 'PASS|FAIL' <<<"$out"; then ok missing-result; else ko "missing-result (rc $rc)"; fi

grep -v '^hold:' "$work/pass.log" > "$work/nohold.log"
out=$(tool "$work/nohold.log" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'hold: unknown' <<<"$out" && ! grep -q 'held [0-9]' <<<"$out"; then ok hold-unknown; else ko "hold-unknown (rc $rc)"; echo "$out"; fi

# blind run: no console hold: line; the value comes from meta.txt or --hold-s
mkdir "$work/rd"; printf 'blind=1\nhold_s=60\n' > "$work/rd/meta.txt"
out=$(tool "$work/nohold.log" --meta "$work/rd" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'held 60 s (source: meta.txt hold_s)' <<<"$out"; then ok hold-from-meta; else ko "hold-from-meta (rc $rc)"; echo "$out"; fi
out=$(tool "$work/nohold.log" --meta "$work/rd/meta.txt" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'source: meta.txt' <<<"$out"; then ok hold-from-meta-file; else ko "hold-from-meta-file (rc $rc)"; fi
out=$(tool "$work/nohold.log" --meta "$work/rd" --hold-s 45 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'held 45 s (source: option' <<<"$out"; then ok hold-from-option; else ko "hold-from-option (rc $rc)"; echo "$out"; fi
printf 'blind=1\n' > "$work/rd/meta.txt"
out=$(tool "$work/nohold.log" --meta "$work/rd" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'hold: unknown' <<<"$out"; then ok hold-meta-without-hold_s; else ko "hold-meta-without-hold_s (rc $rc)"; fi
# the console's hold: line wins over meta and option (unchanged behaviour)
printf 'hold_s=99\n' > "$work/rd/meta.txt"
out=$(tool "$work/pass.log" --meta "$work/rd" --hold-s 98 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'held 20 s (source: console)' <<<"$out" && ! grep -q 'held 9[89]' <<<"$out"; then ok hold-from-console; else ko "hold-from-console (rc $rc)"; echo "$out"; fi

echo 'nothing here' > "$work/nojudge.txt"
out=$(tool "$work/pass.log" --judge "$work/nojudge.txt" 2>&1); rc=$?
if [ $rc = 1 ] && grep -q 'judge' <<<"$out"; then ok missing-judge; else ko "missing-judge (rc $rc)"; fi

ip="192.168.7.$((40 + 2))"; ip2="10.1.2.$((1 + 2))"; usr=ro""ot
mklog "$work/red.log" PASS "ssh $usr@$ip console /dev/ttyUSB3 usb-FTDI_Quad_RS232-HS_FT9ABCDE-if02 host dk1.example.net SerialNumber: A50285BI"
sed -i "s|^stock boot files: .*|stock boot files: no record ($ip /dev/ttyACM0 not installed)|" "$work/red.log"
out=$(tool "$work/red.log" 2>&1); rc=$?
if [ $rc = 0 ] && ! grep -Eq '192\.168|ttyUSB|ttyACM|FT9ABCDE|dk1\.example|A50285BI|root@' <<<"$out" && grep -q '<ip>' <<<"$out"; then
    ok redaction
else ko "redaction (rc $rc)"; echo "$out"; fi

echo "$usr@$ip2 only" > "$work/junk.log"
out=$(tool "$work/junk.log" --commit deadbee 2>&1); rc=$?
if [ $rc = 1 ] && ! grep -q '10\.1\.2\.' <<<"$out"; then ok junk-refuses; else ko "junk-refuses (rc $rc)"; fi

# crc diagnostic, per-check lines and a check-naming FAIL (never decide PASS)
mklog "$work/diag.log" PASS "mp2_display: crc: 0x2fab 0x2fab
mp2_display: frames: 3 seen at the cpsr wrap
mp2_display: layer: addr 0x1000 enable 1 after reload
mp2_display: underrun: 0 transfer_err: 0"
out=$(tool "$work/diag.log" --judge "$work/j.pass" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'mp2_display, PASS' <<<"$out" && grep -q 'mp2_display: crc: 0x2fab 0x2fab' <<<"$out" \
    && grep -q 'mp2_display: frames: 3 seen' <<<"$out" && grep -q 'layer: addr 0x1000' <<<"$out" \
    && grep -q 'not the verdict' <<<"$out"; then ok crc-and-check-lines; else ko "crc-and-check-lines (rc $rc)"; echo "$out"; fi

mklog "$work/crcfail.log" PASS "mp2_display: crc: 0x1234"
sed -i 's|^mp2_display: PASS: .*|mp2_display: FAIL: frames -3|' "$work/crcfail.log"
out=$(tool "$work/crcfail.log" --judge "$work/j.pass" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'mp2_display, FAIL' <<<"$out" && grep -q 'failed check.*frames, error -3' <<<"$out" \
    && ! grep -q 'mp2_display, PASS' <<<"$out"; then ok fail-names-check; else ko "fail-names-check (rc $rc)"; echo "$out"; fi

mklog "$work/crconly.log" PASS "mp2_display: crc: 0x1234"
sed -i 's|^mp2_display: PASS: .*|mp2_display: FAIL: crc -5|' "$work/crconly.log"
out=$(tool "$work/crconly.log" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'mp2_display, FAIL' <<<"$out" && grep -q 'crc, error -5' <<<"$out"; then ok fail-crc; else ko "fail-crc (rc $rc)"; fi

# two KVM verdicts with their offsets; a FAIL in either decides FAIL
echo 'colorbars: PASS: ok' > "$work/j25"; echo 'colorbars: PASS: still ok' > "$work/j45"
out=$(tool "$work/pass.log" --judge "$work/j25@25" --judge "$work/j45@45" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'mp2_display, PASS' <<<"$out" && grep -q '25 s after the result line.*colorbars: PASS: ok' <<<"$out" \
    && grep -q '45 s after the result line.*still ok' <<<"$out"; then ok two-screenshots; else ko "two-screenshots (rc $rc)"; echo "$out"; fi
out=$(tool "$work/pass.log" --judge "$work/j25@25" --judge "$work/j.fail@45" 2>&1); rc=$?
if [ $rc = 0 ] && grep -q 'mp2_display, FAIL' <<<"$out" && grep -q '45 s after' <<<"$out"; then ok two-screenshots-fail; else ko "two-screenshots-fail (rc $rc)"; fi

# verdict-line shapes: optional fields absent/present, both orders, record:<KIND>,
# bad_record (rc 4), lifetime (rc 8), unknown trailing fields. Fixtures only.
vline() { # file, new verdict line (replaces the runner's one)
    sed -i "s|^board-stm32mp2.*back_after=.*|$2|" "$1"
}
vcase() { # name, expected result (PASS|FAIL), rc= part, tail after result, grep pattern
    cp "$work/pass.log" "$work/v.log"
    vline "$work/v.log" "board-stm32mp2.sh stm32mp257f-dk mp2_display $3"
    out=$(tool "$work/v.log" --judge "$work/j.pass" 2>&1); rc=$?
    if [ $rc = 0 ] && grep -q "mp2_display, $2" <<<"$out" && grep -Eq "$4" <<<"$out"; then ok "$1"; else ko "$1 (rc $rc)"; echo "$out"; fi
}
B='boot=uboot-sd row boot_verdict=complete'
vcase verdict-resets-record-after "PASS" "rc=0 verdict=done $B result=pass:mp2_display back_after=41s resets=1 record=0,0,0,0" 'Linux answered 41s after'
vcase verdict-uboot-order "PASS" "rc=0 verdict=done $B result=pass:mp2_display resets=1 record=0,0,0,0 back_after=41s" 'Linux answered 41s after'
vcase verdict-resets-absent "PASS" "rc=0 verdict=done $B result=pass:mp2_display back_after=41s resets=absent" 'Linux answered 41s after'
vcase verdict-record-kind "FAIL" "rc=4 verdict=bad_record $B result=record:KIND_X resets=1 record=1,2,3,4 back_after=12s" 'result=record:KIND_X'
vcase verdict-bad-record "FAIL" "rc=4 verdict=bad_record $B result=none back_after=12s resets=1 record=1,2,3,4" 'verdict=bad_record'
vcase verdict-lifetime "FAIL" "rc=8 verdict=lifetime $B result=none back_after=90s lifetime_after=100s" 'verdict=lifetime'
vcase verdict-unknown-trailing "PASS" "rc=0 verdict=done $B result=pass:mp2_display back_after=41s console=ok ramlog=none future=1" 'Linux answered 41s after'
vcase verdict-unknown-middle "PASS" "rc=0 verdict=done $B result=pass:mp2_display future=1 back_after=41s" 'Linux answered 41s after'

if [ $bad = 0 ]; then echo "mp2-display-record-selftest: all ok"; else echo "mp2-display-record-selftest: $bad failed"; fi
exit $((bad != 0))
