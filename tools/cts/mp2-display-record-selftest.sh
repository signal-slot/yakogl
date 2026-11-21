#!/usr/bin/env bash
# Selftest of mp2-display-record.py on synthetic logs: PASS, FAIL, a missing
# line, and a log carrying an address, host, tty path and USB serial that
# must not reach the record.  No board.  Exit 0 = all good.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/mp2-display-record.py"
T=$(mktemp -d "$TMPDIR/mp2rec.XXXXXX" 2>/dev/null || mktemp -d) || exit 2
trap 'rm -rf "$T"' EXIT
fails=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fails=$((fails + 1)); }

mklog() { # $1 file, $2 verdict word, $3 extra line
    cat > "$1" <<LOG
board-stm32mp2.sh stm32mp257f-dk: mp2_display: build abc1234-dirty boot uboot-sd image sha256 0123456789abcdef footprint none
mp2_display: build x, hold 20 s
mp2_display: chain step done err 0, hpd 1, edid 1 (header 00 ff, ext 1), choice 0
mp2_display: mode 1280x720 74250000 Hz h 110/40/220 v 5/5/20 flags 0x5, hdmi vic 4
mp2_display: $2: 1280x720 XRGB8888 60 frames
hold: result seen at 2026-11-21T10:00:00Z, holding 20s
mp2_display: hold: 19 frames ok, 0 failed
board-stm32mp2.sh stm32mp257f-dk: stock boot files: unchanged since install
board-stm32mp2.sh stm32mp257f-dk mp2_display rc=0 verdict=done boot=uboot-sd row boot_verdict=complete result=pass:mp2_display back_after=41s
$3
LOG
}
printf 'colorbars: PASS: 8 bars white yellow cyan green magenta red blue black\n' > "$T/judge"
LEAK='ssh root@10.20.30.40 failed; host=dk-lab-07 via /dev/ttyUSB3 serial: A1B2C3D4E5 usb-FTDI_Dual_RS232-HS_A1B2C3 fe80::1:2:3:4 box.example.com'
LEAKS='10.20.30.40 dk-lab-07 ttyUSB3 A1B2C3D4E5 FTDI_Dual fe80:: root@ box.example.com'

# PASS
mklog "$T/pass" PASS ""
out=$(python3 -I "$tool" "$T/pass" "$T/judge") && rc=0 || rc=$?
[ $rc = 0 ] && ok "pass exits 0" || bad "pass exits $rc"
for want in 'abc1234-dirty' '1280x720' '| mode source | EDID |' 'probe verdict | PASS' \
    '20s from the result line' 'colorbars: PASS' 'unchanged since install' 'back after 41s'; do
    grep -qF -- "$want" <<<"$out" && ok "pass has '$want'" || bad "pass lacks '$want'"
done
out=$(python3 -I "$tool" "$T/pass") && grep -q 'not judged' <<<"$out" && ok "no judge file: not judged" || bad "no judge file"

# FAIL (and the fallback mode source)
mklog "$T/fail" FAIL ""
sed -i 's/edid 1 /edid 0 /' "$T/fail"
printf 'colorbars: FAIL: bar 3 is black\n' > "$T/jfail"
out=$(python3 -I "$tool" "$T/fail" "$T/jfail") && rc=0 || rc=$?
if [ $rc = 0 ] && grep -q 'probe verdict | FAIL' <<<"$out" && grep -q 'fallback' <<<"$out" \
    && grep -q 'colorbars: FAIL' <<<"$out"; then ok "fail record (fallback source, judge FAIL)"; else bad "fail record rc=$rc"; fi

# a missing required line is refused with a reason
grep -v ': PASS:' "$T/pass" > "$T/nores"
out=$(python3 -I "$tool" "$T/nores" 2>"$T/err") && rc=0 || rc=$?
if [ $rc = 1 ] && [ -z "$out" ] && grep -q 'PASS|FAIL' "$T/err"; then ok "missing result line refused (exit 1, says why)"; else bad "missing result line rc=$rc"; fi
grep -v 'hold: result' "$T/pass" > "$T/nohold"
if python3 -I "$tool" "$T/nohold" >/dev/null 2>"$T/err"; then bad "missing hold accepted"; elif grep -q 'hold' "$T/err"; then ok "missing hold refused"; else bad "missing hold: no reason"; fi

# redaction: in the whole record, and in the redactor on a free line
mklog "$T/leak" PASS "$LEAK"
sed -i 's#^mp2_display: PASS: .*#mp2_display: PASS: via /dev/ttyACM1 192.168.7.9 60 frames#' "$T/leak"
out=$(python3 -I "$tool" "$T/leak" "$T/judge") || bad "leak log refused"
if grep -qE '192\.168|ttyACM' <<<"$out"; then bad "leak inside the probe line"; else ok "leak inside the probe line redacted"; fi
red=$(python3 -I -c '
import importlib.util, sys
sp = importlib.util.spec_from_file_location("m", sys.argv[1])
m = importlib.util.module_from_spec(sp)
sp.loader.exec_module(m)
print(m.redact(sys.argv[2]))' "$tool" "$LEAK")
leak=0
for s in $LEAKS; do
    grep -qF -- "$s" <<<"$red$out" && { bad "leaked '$s'"; leak=1; }
done
[ $leak = 0 ] && ok "address, host, tty path, serial redacted"

echo "$fails failure(s)"
[ $fails = 0 ]
