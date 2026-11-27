#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of imx8mp-gles3-times.py: a synthetic slice, and the Pi 4 log (no matching entries).
# Needs TMPDIR (the scratch directory is made under it, never under /tmp).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
t=$here/imx8mp-gles3-times.py
w=$(mktemp -d "${TMPDIR:?set TMPDIR to a scratch directory}/times-selftest.XXXXXX")
trap 'rm -rf "$w"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

s=$w/2026-11-imx8mp-gles3-test-rows01-03
mkdir "$s"
cat > "$s/README.md" <<'EOT'
board.sh imx8mp e00 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=1,2,3,4 back_after=51s uptime=25s
board.sh imx8mp e00-p1 rc=0 verdict=done boot=uboot-mmc row result=record:DEQP_FAILS resets=absent back_after=40s uptime=20s
board.sh imx8mp e01 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=1,2,3,4 back_after=66s
board.sh imx8mp e02 rc=5 verdict=lost boot=uboot-mmc row boot_verdict=none result=none resets=absent
EOT

python3 -I "$t" "$s" > "$w/out.txt" 2> "$w/err.txt" || fail "text run"
[ ! -s "$w/err.txt" ] || fail "warnings: $(cat "$w/err.txt")"
grep -q '^e00 .* 26 ' "$w/out.txt" || fail "e00 is back_after - uptime = 26"
grep -q '^e01 .* 66 ' "$w/out.txt" || fail "e01 without uptime is back_after = 66"
grep -q '^e02 .* none ' "$w/out.txt" || fail "e02 has no time"
if grep -q 'e00-' "$w/out.txt"; then fail "page boot listed"; fi
grep -q 'total  2 of 3 entries timed: measured 92 s' "$w/out.txt" || fail "slice total"

python3 -I "$t" --tsv "$s" > "$w/out.tsv" || fail "tsv run"
head -1 "$w/out.tsv" | grep -q '^slice	entry	measured_s	estimate_s	ratio$' || fail "tsv header"
[ "$(wc -l < "$w/out.tsv")" -eq 5 ] || fail "tsv rows"
awk -F'\t' 'NF != 5 { exit 1 }' "$w/out.tsv" || fail "tsv columns"
awk -F'\t' '$2 == "e00" && $3 == 26 && $4 + 0 > 0 && $5 + 0 > 0 { ok = 1 } END { exit !ok }' "$w/out.tsv" || fail "e00 estimate and ratio"
awk -F'\t' '$2 == "e02" && $3 == "none" && $5 == "-" { ok = 1 } END { exit !ok }' "$w/out.tsv" || fail "e02 none row"

# Armadillo-X2 slice: board.sh x2 lines (back_after only, the Linux-back time after the kexec; an uptime on an x2
# line is not subtracted), a page line, an entry without a time; the board and the time used are named
x=$w/2026-11-imx8mp-gles3-x2-0123abcd-rows01-03
mkdir "$x"
cat > "$x/README.md" <<'EOT'
board.sh x2 e00 rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=record:DEQP resets=absent record=594b0800,000002bb,00000000 back_after=41s (kexec-dev: a development run, never a row)
board.sh x2 e00-p0 rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=record:DEQP_FAILS resets=absent back_after=37s (kexec-dev)
board.sh x2 e01 rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=record:DEQP resets=absent back_after=300s uptime=250s (kexec-dev)
board.sh x2 e02 rc=5 verdict=lost boot=kexec-dev DEV boot_verdict=none result=none resets=absent
EOT
python3 -I "$t" "$x" > "$w/x.txt" 2> "$w/xerr.txt" || fail "x2 text run"
[ ! -s "$w/xerr.txt" ] || fail "x2 warnings: $(cat "$w/xerr.txt")"
grep -q '^# .*Armadillo-X2.*back_after.*after the kexec.*not U-Boot' "$w/x.txt" || fail "x2 board and time not named"
grep -q '^e00 .* 41 ' "$w/x.txt" || fail "x2 e00 is back_after = 41"
grep -q '^e01 .* 300 ' "$w/x.txt" || fail "x2 e01: uptime not subtracted"
grep -q '^e02 .* none ' "$w/x.txt" || fail "x2 e02 has no time"
if grep -q 'e00-' "$w/x.txt"; then fail "x2 page boot listed"; fi
grep -q 'total  2 of 3 entries timed: measured 341 s' "$w/x.txt" || fail "x2 slice total"
python3 -I "$t" --tsv "$x" > "$w/x.tsv" || fail "x2 tsv run"
grep -q '^# .*Armadillo-X2' "$w/x.tsv" || fail "x2 tsv board comment"
grep -v '^#' "$w/x.tsv" | awk -F'\t' 'NF != 5 { exit 1 }' || fail "x2 tsv columns"
awk -F'\t' '$2 == "e00" && $3 == 41 { ok = 1 } END { exit !ok }' "$w/x.tsv" || fail "x2 tsv e00"
# the Verdin output names no board
if grep -q 'Armadillo' "$w/out.txt" "$w/out.tsv"; then fail "Verdin output names the X2"; fi
# a Verdin and an X2 slice together: each slice has its own rule
python3 -I "$t" --tsv "$s" "$x" > "$w/both.tsv" || fail "both run"
awk -F'\t' '$1 ~ /test-rows01/ && $2 == "e00" && $3 == 26 { a = 1 } $1 ~ /x2-/ && $2 == "e01" && $3 == 300 { b = 1 } END { exit !(a && b) }' "$w/both.tsv" || fail "both: Verdin 26 and X2 300"

# the Pi 4 log has no board.sh imx8mp lines: no entries, a zero total, exit 0
python3 -I "$t" --tsv "$here/log/2026-10-31-pi4-gles3-a" > "$w/pi4.tsv" || fail "pi4 run"
[ "$(wc -l < "$w/pi4.tsv")" -eq 2 ] || fail "pi4: header and total only"
grep -q '	TOTAL	0	0.0	-$' "$w/pi4.tsv" || fail "pi4 total"
python3 -I "$t" "$w/nonexistent" > /dev/null || fail "missing dir"
echo "imx8mp-gles3-times-selftest: ok"
