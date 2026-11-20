#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi_5/rpi5_scanout16/verdicts_selftest.sh
#
# Hermetic self-test of verdicts.sh on fixture logs written to $TMPDIR
# (removed at the end; nothing is written into the repository):
#
#   pass      a complete log -> 'verdicts: PASS', exit 0, q2 'no'
#   failing   one log per failing condition (adopt, readback 1, readback 2,
#             frcnt stuck, EN 0, VIDEN 0, the probe's own FAIL) -> FAIL
#             naming that reason, exit 3; a passing line the probe printed
#             over failing evidence does not rescue it
#   wrap      FRCNT 60 -> 3 advanced (PASS); 17 -> 17 stuck; 70 is no counter
#   q2        the hold line's answers: redrawn, list changed, both, unclear
#   cut       a log cut after the pattern line -> incomplete, exit 1, the
#             stop point named, no PASS/FAIL
#   gap       lines skipped between seen ones -> incomplete, the gap named
#   badline   a malformed frcnt line counts as missing
#   redact    an IPv4, host:port, IPv6, MAC, /dev/tty path and tty name in
#             printed lines -> none reach the output; 0x values print
#   usage     wrong argument count, unreadable file -> exit 2; an empty log
#             -> incomplete, exit 1
set -u
unset LC_ALL LANGUAGE
export LANG=C
HERE=$(cd "$(dirname "$0")" && pwd)
V=$HERE/verdicts.sh
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
check() { case "$out" in *"$1"*) ok "$2" ;; *) bad "$2: '$1' not in output" ;; esac; }
nope()  { case "$out" in *"$1"*) bad "$2: '$1' appeared in output" ;; *) ok "$2" ;; esac; }
expect_rc() { [ "$rc" -eq "$1" ] && ok "$2: exit $1" || bad "$2: rc=$rc, out: $out"; }

T=$(mktemp -d "${TMPDIR:-/tmp}/verdicts16-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
[ -r "$V" ] || { echo "verdicts_selftest: $V not found" >&2; exit 2; }

bash -n "$V" && ok "verdicts.sh parses (bash -n)" || bad "verdicts.sh: bash -n"
bash -n "$0" && ok "verdicts_selftest.sh parses (bash -n)" || bad "selftest: bash -n"

# mk <file> [key=value ...]: a complete passing log; a key=value replaces
# that line's body (keys: adopt rb1 rb2 frcnt pv hold term).
mk() {
    local f=$1; shift
    local adopt='u3 adopt-readback: PASS (32 KiB read back byte-equal)'
    local rb1='u3 readback 1: PASS (1920 x 1080, 4147200 bytes equal)'
    local rb2='u3 readback 2: PASS (1920 x 1080, 4147200 bytes equal)'
    local frcnt='u3 frcnt: before 12 after 15'
    local pv='u3 pv: control=0x00000011 en=1 viden=1'
    local hold='u3 hold: 5000 ms plane=kept bytes=0 list=kept words=0'
    local term='rpi5_scanout16: PASS (adopted, 2 read-backs, frcnt advanced)'
    local kv
    for kv in "$@"; do
        case "$kv" in
            adopt=*) adopt=${kv#adopt=} ;; rb1=*) rb1=${kv#rb1=} ;; rb2=*) rb2=${kv#rb2=} ;;
            frcnt=*) frcnt=${kv#frcnt=} ;; pv=*) pv=${kv#pv=} ;; hold=*) hold=${kv#hold=} ;;
            term=*) term=${kv#term=} ;;
        esac
    done
    {
        echo '--- Zephyr rpi5_scanout16 on BCM2712 (probe fixture) ---'
        echo 'get_throttled 0x50020 next to the boot banner'
        echo 'u3 scanout: running'
        echo 'u3 plane: addr=0x3c000000 fmt=RGB565 pitch=3840 w=1920 h=1080'
        echo "$adopt"
        echo 'rpi5_scanout16: text rows 24 changed'
        echo 'rpi5_scanout16: pattern drawn (8 colour bars below the text)'
        echo "$rb1"
        echo "$rb2"
        echo "$frcnt"
        echo "$pv"
        echo "$hold"
        echo "$term"
        echo 'rpi5_scanout16: feeding stopped, reset expected'
    } > "$f"
}

# ---- pass -------------------------------------------------------------------
mk "$T/pass.log"
out=$(bash "$V" "$T/pass.log" 2>&1); rc=$?
expect_rc 0 "pass log"
check "verdicts: PASS (" "PASS named"
check "11 of 11 expected verdict lines" "all eleven lines counted"
check "missing: none" "nothing missing"
check "[12] rpi5_scanout16: feeding stopped, reset expected" "feeding-stopped line printed next to PASS"
check "u3 plane: addr=0x3c000000 fmt=RGB565 pitch=3840 w=1920 h=1080" "plane line printed with its address"
check "q2: no (the firmware neither redrew the plane nor changed its list in 5000 ms)" "q2: no"
nope "get_throttled" "noise lines are not verdict lines"
nope "verdicts: FAIL" "pass log: not FAIL"

# the watchdog ending alone closes the run, and evidence decides
mk "$T/wd.log" "term=rpi5_scanout16: feeding stopped, reset expected"
out=$(bash "$V" "$T/wd.log" 2>&1); rc=$?
expect_rc 0 "ending without a result line"
check "verdicts: PASS (" "evidence passes without a result line"
check "11 of 11" "the ending counts as the terminal line"

# ---- each failing condition -------------------------------------------------
t_fail() { # name reason args...
    local name=$1 reason=$2; shift 2
    mk "$T/$name.log" "$@"
    out=$(bash "$V" "$T/$name.log" 2>&1); rc=$?
    expect_rc 3 "$name"
    check "verdicts: FAIL (" "$name: FAIL named"
    check "$reason" "$name: reason named"
    nope "verdicts: PASS" "$name: not PASS"
    nope "verdicts: incomplete" "$name: complete"
}
t_fail adopt "the plane is not adopted" 'adopt=u3 adopt-readback: FAIL (no active display list)'
t_fail rb1 "readback 1 FAIL" 'rb1=u3 readback 1: FAIL (first diff at byte 3840)'
t_fail rb2 "readback 2 FAIL" 'rb2=u3 readback 2: FAIL (123 bytes differ)'
t_fail stuck "frcnt did not advance (17 -> 17)" 'frcnt=u3 frcnt: before 17 after 17'
t_fail en0 "PV_CONTROL.EN is 0" 'pv=u3 pv: control=0x00000010 en=0 viden=1'
t_fail viden0 "PV_V_CONTROL.VIDEN is 0" 'pv=u3 pv: control=0x00000011 en=1 viden=0'
t_fail probefail "the probe's own result line says FAIL" 'term=rpi5_scanout16: FAIL (the tail check)'
t_fail overpass "readback 2 FAIL" 'rb2=u3 readback 2: FAIL (x)' 'term=rpi5_scanout16: PASS (the probe says so)'

mk "$T/multi.log" 'adopt=u3 adopt-readback: FAIL (x)' 'pv=u3 pv: control=0x0 en=0 viden=0'
out=$(bash "$V" "$T/multi.log" 2>&1); rc=$?
expect_rc 3 "several failing conditions"
check "the plane is not adopted" "multi: adopt named"
check "PV_CONTROL.EN is 0" "multi: EN named"
check "PV_V_CONTROL.VIDEN is 0" "multi: VIDEN named"

# ---- FRCNT wrap -------------------------------------------------------------
mk "$T/wrap.log" 'frcnt=u3 frcnt: before 60 after 3'
out=$(bash "$V" "$T/wrap.log" 2>&1); rc=$?
expect_rc 0 "frcnt 60 -> 3 (wrapped)"
check "frcnt 60 -> 3 advanced" "wrapped counter advanced"
mk "$T/edge.log" 'frcnt=u3 frcnt: before 63 after 0'
out=$(bash "$V" "$T/edge.log" 2>&1); rc=$?
expect_rc 0 "frcnt 63 -> 0"
mk "$T/range.log" 'frcnt=u3 frcnt: before 10 after 70'
out=$(bash "$V" "$T/range.log" 2>&1); rc=$?
expect_rc 3 "frcnt 70 is no 6-bit counter"
check "is not a 6-bit counter" "out-of-range counter named"

# ---- q2 answers -------------------------------------------------------------
q2() { # name expect hold-line
    mk "$T/$1.log" "hold=$3"
    out=$(bash "$V" "$T/$1.log" 2>&1); rc=$?
    expect_rc 0 "q2 $1 (the hold never changes the verdict)"
    check "$2" "q2 $1"
}
q2 redrawn "q2: yes, redrawn (7680 plane bytes changed in 10000 ms)" 'u3 hold: 10000 ms plane=redrawn bytes=7680 list=kept words=0'
q2 listch "q2: yes, list changed (2 list words changed in 10000 ms)" 'u3 hold: 10000 ms plane=kept bytes=0 list=changed words=2'
q2 both "q2: yes, redrawn and list changed (16 plane bytes, 1 list words in 3000 ms)" 'u3 hold: 3000 ms plane=redrawn bytes=16 list=changed words=1'
q2 contra "q2: unclear (plane=kept with bytes=4)" 'u3 hold: 3000 ms plane=kept bytes=4 list=kept words=0'
q2 zero "q2: unclear (a hold of 0 ms holds nothing)" 'u3 hold: 0 ms plane=kept bytes=0 list=kept words=0'

# ---- cut after the pattern line --------------------------------------------
head -n 7 "$T/pass.log" > "$T/cut.log"
out=$(bash "$V" "$T/cut.log" 2>&1); rc=$?
expect_rc 1 "cut log"
check "verdicts: incomplete" "cut log: incomplete"
check "5 of 11 expected verdict lines" "cut log: five lines seen"
check "missing: u3 readback 1 u3 readback 2 u3 frcnt u3 pv u3 hold rpi5_scanout16 terminal" "cut log names every missing line"
check "[5] rpi5_scanout16: pattern drawn (8 colour bars below the text): the image stopped there" "cut log: stop point named"
check "q2: unanswered (no hold line)" "cut log: q2 unanswered"
nope "verdicts: PASS" "cut log: no PASS"
nope "verdicts: FAIL" "cut log: no FAIL"

# ---- gap: a result line, lines skipped between seen ones -------------------
grep -v -e '^u3 plane:' -e '^u3 readback 2:' -e '^u3 pv:' "$T/pass.log" > "$T/gap.log"
out=$(bash "$V" "$T/gap.log" 2>&1); rc=$?
expect_rc 1 "gap log"
check "verdicts: incomplete" "gap log: incomplete"
check "gap: missing between seen lines: u3 plane u3 readback 2 u3 pv" "gap log names the gap"
nope "the image stopped there" "gap log: no stop claim after a terminal line"
nope "verdicts: PASS" "gap log: no PASS"

# ---- a malformed line is missing --------------------------------------------
mk "$T/badfr.log" 'frcnt=u3 frcnt: before 12'
out=$(bash "$V" "$T/badfr.log" 2>&1); rc=$?
expect_rc 1 "malformed frcnt line"
check "gap: missing between seen lines: u3 frcnt" "a malformed frcnt line is a gap"
mk "$T/badpv.log" 'pv=u3 pv: control=0x11 en=2 viden=1'
out=$(bash "$V" "$T/badpv.log" 2>&1); rc=$?
expect_rc 1 "malformed pv line"
check "u3 pv" "a malformed pv line is named"

# ---- redaction ---------------------------------------------------------------
mk "$T/redact.log" \
    'adopt=u3 adopt-readback: PASS (via arm.example.invalid:40001 on /dev/ttyAMA0 for 100.64.7.9)' \
    'rb1=u3 readback 1: FAIL (100.64.7.9:443 over /dev/ttyUSB3, link local fe80::1, eth de:ad:be:ef:12:34, nic aa-bb-cc-dd-ee-ff, unit serial-getty@ttyS2.service)' \
    'term=rpi5_scanout16: FAIL (pv control=0x00000011)'
out=$(bash "$V" "$T/redact.log" 2>&1); rc=$?
expect_rc 3 "redact log"
check "addr=0x3c000000" "the plane DRAM address prints unredacted"
check "control=0x00000011" "a 0x value prints unredacted"
nope "100.64" "no IPv4 fragment"
nope "arm.example" "no host name with a port"
nope "fe80" "no IPv6 address"
nope "de:ad:be:ef" "no MAC address (colon form)"
nope "aa-bb-cc-dd-ee-ff" "no MAC address (dash form)"
nope "/dev/ttyUSB3" "no /dev/tty path"
nope "/dev/ttyAMA0" "no second /dev/tty path"
nope "ttyUSB3" "no UART device name"
nope "ttyS2" "no tty name inside a unit"
check "<host-port>" "a host:port marker is printed"
check "<ipv6/mac>" "an IPv6/MAC marker is printed"
check "<uart>" "a UART marker is printed"
check "<ipv4>" "an IPv4 marker is printed"

# ---- usage and unreadable files ----------------------------------------------
out=$(bash "$V" 2>&1); rc=$?
expect_rc 2 "no arguments"
out=$(bash "$V" "$T/pass.log" extra 2>&1); rc=$?
expect_rc 2 "two arguments"
out=$(bash "$V" "$T/no-such.log" 2>&1); rc=$?
expect_rc 2 "unreadable log"
: > "$T/empty.log"
out=$(bash "$V" "$T/empty.log" 2>&1); rc=$?
expect_rc 1 "empty log"
check "verdicts: incomplete" "empty log: incomplete"
check "no expected verdict line was seen at all" "empty log says nothing was seen"

# ---- summary -----------------------------------------------------------------
if [ $fail -eq 0 ]; then
    echo "verdicts_selftest: all checks passed"
else
    echo "verdicts_selftest: $fail failed"
    exit 1
fi
