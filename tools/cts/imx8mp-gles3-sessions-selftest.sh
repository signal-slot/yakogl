#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of imx8mp-gles3-sessions.py on a small synthetic table, mustpass list and Pi 4 record:
# sh tools/cts/imx8mp-gles3-sessions-selftest.sh   (host only, exit 0 when every check holds)
set -eu
here=$(cd "$(dirname "$0")" && pwd)
py="$here/imx8mp-gles3-sessions.py"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/sessions-selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

# mustpass: 4 lifetime (measured), 2 filtering.3d (assumed 22 s), 6 shaders (assumed 0.3 s), 3 ubo (assumed)
{
  for i in 1 2 3 4; do echo "dEQP-GLES3.functional.lifetime.c$i"; done
  for i in 1 2; do echo "dEQP-GLES3.functional.texture.filtering.3d.f$i"; done
  for i in 1 2 3 4 5 6; do echo "dEQP-GLES3.functional.shaders.loops.s$i"; done
  for i in 1 2 3; do echo "dEQP-GLES3.functional.ubo.u$i"; done
} > "$tmp/mustpass.txt"
{
  echo '# synthetic (15 cases)'
  echo 'dEQP-GLES3.functional.lifetime.*'
  echo 'dEQP-GLES3.functional.texture.filtering.3d.*'
  echo 'dEQP-GLES3.functional.shaders.loops.s1,dEQP-GLES3.functional.shaders.loops.s2,dEQP-GLES3.functional.shaders.loops.s3'
  echo 'dEQP-GLES3.functional.shaders.loops.s4,dEQP-GLES3.functional.shaders.loops.s5,dEQP-GLES3.functional.shaders.loops.s6'
  echo 'dEQP-GLES3.functional.ubo.*'
} > "$tmp/table.txt"
# Pi 4 record: image FO2 is row 9 of the Pi 4 table, LT the 8th image (row 16): rows 0-15 are filler
mkdir "$tmp/rec"
{
  i=0; while [ $i -lt 16 ]; do echo "dEQP-GLES3.nothing$i.*"; i=$((i + 1)); done
  echo 'dEQP-GLES3.functional.lifetime.*'
} > "$tmp/pi4.txt"
cat > "$tmp/rec/cycles.txt" <<EOF
LT sha256=00 boot0=x
   4 cases in 2000 ms, record at 2 s after boot
EOF

run() { python3 "$py" --table "$tmp/table.txt" --mustpass "$tmp/mustpass.txt" --record "$tmp/rec" \
        --pi4-table "$tmp/pi4.txt" --gles2-results /nonexistent "$@"; }

run > "$tmp/out.tsv" 2> "$tmp/err.txt" || fail "the table run exited nonzero: $(cat "$tmp/err.txt")"
rows=$(grep -v '^#' "$tmp/out.tsv" | awk -F'\t' '$1 ~ /^[0-9]+$/ {n++} END {print n + 0}')
[ "$rows" = 5 ] || fail "expected 5 rows, got $rows"
row() { awk -F'\t' -v i="$1" '$1 == i' "$tmp/out.tsv"; }
field() { row "$1" | cut -f"$2"; }
# counts
[ "$(field 0 2)" = 4 ] || fail "lifetime cases: $(field 0 2)"
[ "$(field 1 2)" = 2 ] || fail "filtering.3d cases: $(field 1 2)"
[ "$(field 2 2)" = 3 ] || fail "shader row 2 cases: $(field 2 2)"
[ "$(field 3 2)" = 3 ] || fail "shader row 3 cases: $(field 3 2)"
[ "$(field 4 2)" = 3 ] || fail "ubo cases: $(field 4 2)"
total=$(grep -v '^#' "$tmp/out.tsv" | awk -F'\t' '$1 ~ /^[0-9]+$/ {s += $2} END {print s}')
[ "$total" = 15 ] || fail "case sum $total, expected 15"
# estimates: 4 x 0.5 s measured, 2 x 22 s, 3 x 0.3 s, 3 x (0.5 x 600/800)
[ "$(field 0 3)" = 2.0 ] || fail "lifetime seconds: $(field 0 3)"
[ "$(field 1 3)" = 44.0 ] || fail "filtering seconds: $(field 1 3)"
[ "$(field 2 3)" = 0.9 ] || fail "shader seconds: $(field 2 3)"
[ "$(field 4 3)" = 1.1 ] || [ "$(field 4 3)" = 1.2 ] || fail "ubo seconds: $(field 4 3)"
field 0 4 | grep -q '^pi4:LT$' || fail "lifetime source: $(field 0 4)"
field 1 4 | grep -q '^assumed:' || fail "filtering source: $(field 1 4)"
# risk flag: 44 s is at risk with --window 50 --margin 0.8 (> 40 s), not with the default
[ "$(field 1 5)" = - ] || fail "default risk flag: $(field 1 5)"
run --window 50 --margin 0.8 | awk -F'\t' '$1 == 1' | cut -f5 | grep -q '^RISK$' || fail "risk flag not raised"
run --window 50 --margin 0.8 | awk -F'\t' '$1 == 0' | cut -f5 | grep -q '^-$' || fail "risk flag raised on a quick row"

# ranges: contiguous, no gap, no overlap, cover 0..4 once
check_ranges() {
  awk -F'\t' -v last="$1" 'BEGIN { next_start = 0 } $1 ~ /^[0-9]+$/ && NF == 7 && $2 ~ /^[0-9]+$/ {
        if ($2 != next_start) { print "gap or overlap at session " $1; bad = 1 }
        if ($3 < $2) { print "empty range " $1; bad = 1 }
        next_start = $3 + 1; n++ }
      END { if (next_start != last + 1) { print "does not end at " last; bad = 1 } exit bad }'
}
for n in 1 2 3 5 9; do
  run --sessions "$n" | grep -v '^#' | check_ranges 4 || fail "--sessions $n coverage"
done
run --sessions 2 | grep -v '^#' | awk -F'\t' 'NR > 1 {n++} END {exit n != 2}' || fail "--sessions 2 gave other than 2 ranges"
for b in 1 50 100000; do
  run --session-budget "$b" | grep -v '^#' | check_ranges 4 || fail "--session-budget $b coverage"
done
# a huge budget gives one range
run --session-budget 100000 | grep -v '^#' | awk -F'\t' 'NR > 1 {n++} END {exit n != 1}' || fail "a huge budget should give 1 range"
echo "imx8mp-gles3-sessions-selftest: ok"
