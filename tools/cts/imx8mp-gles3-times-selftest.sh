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

# the Pi 4 log has no board.sh imx8mp lines: no entries, a zero total, exit 0
python3 -I "$t" --tsv "$here/log/2026-10-31-pi4-gles3-a" > "$w/pi4.tsv" || fail "pi4 run"
[ "$(wc -l < "$w/pi4.tsv")" -eq 2 ] || fail "pi4: header and total only"
grep -q '	TOTAL	0	0.0	-$' "$w/pi4.tsv" || fail "pi4 total"
python3 -I "$t" "$w/nonexistent" > /dev/null || fail "missing dir"
echo "imx8mp-gles3-times-selftest: ok"
