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
        --pi4-table "$tmp/pi4.txt" --gles2-results /nonexistent --no-board-rates "$@"; }

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
# estimates: 4 x 0.5 s measured, 2 x 22 s, 3 x 0.3 s, 3 x (0.5 x 600/600)
[ "$(field 0 3)" = 2.0 ] || fail "lifetime seconds: $(field 0 3)"
[ "$(field 1 3)" = 44.0 ] || fail "filtering seconds: $(field 1 3)"
[ "$(field 2 3)" = 0.9 ] || fail "shader seconds: $(field 2 3)"
[ "$(field 4 3)" = 1.5 ] || fail "ubo seconds: $(field 4 3)"
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

# measured GC7000UL rates: a synthetic slice with e00 (lifetime, 4 cases, 40 s) and e02 (a shader row, 3 cases,
# 30 s; its duplicate line and its e02-p0 page line are not counted); e00 has 1 failure; e04 (ubo) is unmeasured
sl="$tmp/slices/2026-11-imx8mp-gles3-9bc82915-rows00-02"
mkdir -p "$sl"
cat > "$sl/verdicts.txt" <<EOF
# synthetic
board.sh imx8mp e00 rc=0 verdict=done boot=x back_after=40s uptime=20s
board.sh imx8mp e02 rc=0 verdict=done boot=x back_after=30s uptime=20s
board.sh imx8mp e02 rc=0 verdict=done boot=x back_after=30s uptime=20s
board.sh imx8mp e02-p0 rc=0 verdict=done boot=x back_after=999s uptime=20s
EOF
printf '# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tnames\n0\tx\t4\t3\t1\t0\t0\t0\t-\n2\tx\t3\t3\t0\t0\t0\t0\t-\n' > "$sl/results.tsv"
runb() { python3 "$py" --table "$tmp/table.txt" --mustpass "$tmp/mustpass.txt" --record "$tmp/rec" \
         --pi4-table "$tmp/pi4.txt" --gles2-results /nonexistent --board-slices "$tmp/slices/*" "$@"; }
runb > "$tmp/outb.tsv" 2> "$tmp/errb.txt" || fail "the measured run exited nonzero: $(cat "$tmp/errb.txt")"
rowb() { awk -F'\t' -v i="$1" '$1 == i' "$tmp/outb.tsv" | cut -f"$2"; }
[ "$(rowb 0 3)" = 40.0 ] || fail "measured lifetime seconds: $(rowb 0 3)"
[ "$(rowb 0 4)" = imx8mp:rows00-02 ] || fail "measured source: $(rowb 0 4)"
[ "$(rowb 2 3)" = 30.0 ] || fail "measured shader seconds: $(rowb 2 3)"
[ "$(rowb 1 3)" = 44.0 ] || fail "unmeasured filtering must stay assumed: $(rowb 1 3)"
[ "$(rowb 4 4 | cut -c1-8)" = assumed: ] || fail "unmeasured ubo source: $(rowb 4 4)"
grep -q '^# rate functional.lifetime = 10.0000 s/case (imx8mp:rows00-02' "$tmp/outb.tsv" || fail "header lacks the measured rate"
# --pages: lifetime fail fraction 1/4 x 4 cases = 1 expected fail = 1 page of 90 s; the passing rows gain nothing
runb --pages --page-seconds 90 | awk -F'\t' '$1 == 0 {print $3}' | grep -qx 130.0 || fail "--pages estimate of row 0"
runb --pages --page-seconds 90 | awk -F'\t' '$1 == 2 {print $3}' | grep -qx 30.0 || fail "--pages changed a passing row"
for n in 1 3; do
  runb --sessions "$n" | grep -v '^#' | check_ranges 4 || fail "measured --sessions $n coverage"
done
# a slice of any campaign commit: the 05769bf2-style name (full date, other hash) reads the same way; the
# different back_after figures prove these rates come from this slice (e00 44 s / 4 cases, e02 33 s / 3 cases)
sl2="$tmp/slices2/2026-11-16-imx8mp-gles3-05769bf2-rows00-02"
mkdir -p "$sl2"
cat > "$sl2/verdicts.txt" <<EOF
# synthetic
board.sh imx8mp e00 rc=0 verdict=done boot=x back_after=44s uptime=20s
board.sh imx8mp e02 rc=0 verdict=done boot=x back_after=33s uptime=20s
EOF
cp "$sl/results.tsv" "$sl2/results.tsv"
runb2() { python3 "$py" --table "$tmp/table.txt" --mustpass "$tmp/mustpass.txt" --record "$tmp/rec" \
          --pi4-table "$tmp/pi4.txt" --gles2-results /nonexistent --board-slices "$tmp/slices2/*" "$@"; }
runb2 > "$tmp/outb2.tsv" 2> "$tmp/errb2.txt" || fail "the 05769bf2-style slice run exited nonzero: $(cat "$tmp/errb2.txt")"
rowb2() { awk -F'\t' -v i="$1" '$1 == i' "$tmp/outb2.tsv" | cut -f"$2"; }
[ "$(rowb2 0 3)" = 44.0 ] || fail "05769bf2-style measured lifetime seconds: $(rowb2 0 3)"
[ "$(rowb2 0 4)" = imx8mp:rows00-02 ] || fail "05769bf2-style source label: $(rowb2 0 4)"
[ "$(rowb2 2 3)" = 33.0 ] || fail "05769bf2-style measured shader seconds: $(rowb2 2 3)"
grep -q '^# rate functional.lifetime = 11.0000 s/case (imx8mp:rows00-02' "$tmp/outb2.tsv" || fail "header lacks the 05769bf2-style measured rate"
# mixed commits: one glob must not mix the slices of two campaign commits (imx8mp-gles3-merge.py --check mirrors this)
mkdir -p "$tmp/mixed"
cp -r "$sl" "$sl2" "$tmp/mixed/"
rc=0; python3 "$py" --table "$tmp/table.txt" --mustpass "$tmp/mustpass.txt" --record "$tmp/rec" \
     --pi4-table "$tmp/pi4.txt" --gles2-results /nonexistent --board-slices "$tmp/mixed/*" \
     > "$tmp/mixout" 2> "$tmp/mixerr" || rc=$?
[ "$rc" = 2 ] || fail "mixed commits exited $rc, expected 2"
grep -q '9bc82915' "$tmp/mixerr" && grep -q '05769bf2' "$tmp/mixerr" || fail "mixed-commit message names no commits: $(cat "$tmp/mixerr")"
grep -q 'mixed commits\|campaign commits' "$tmp/mixerr" || fail "mixed-commit message unclear: $(cat "$tmp/mixerr")"
[ ! -s "$tmp/mixout" ] || fail "mixed commits wrote output"
# a slice directory whose name carries no commit hash is skipped with a note, not used
mkdir -p "$tmp/slices3/2026-11-imx8mp-gles3-rows00-02"
cp "$sl/verdicts.txt" "$sl/results.tsv" "$tmp/slices3/2026-11-imx8mp-gles3-rows00-02/"
python3 "$py" --table "$tmp/table.txt" --mustpass "$tmp/mustpass.txt" --record "$tmp/rec" \
         --pi4-table "$tmp/pi4.txt" --gles2-results /nonexistent --board-slices "$tmp/slices3/*" \
         > "$tmp/skipout" 2> "$tmp/skiperr" || fail "a run with an unparseable slice name exited nonzero: $(cat "$tmp/skiperr")"
grep -q '^# note: skipped 2026-11-imx8mp-gles3-rows00-02: ' "$tmp/skipout" || fail "no skip note for the unparseable name"
[ "$(awk -F'\t' '$1 == 0 {print $3}' "$tmp/skipout")" = 2.0 ] || fail "the skipped slice still gave a measured rate"
[ "$(awk -F'\t' '$1 == 0 {print $4}' "$tmp/skipout")" = pi4:LT ] || fail "the skipped slice changed the rate source"
# --rows: only the named entries are listed, with the numbers they have without --rows; ranges stay inside
run --rows 1..3 | grep -v '^#' | awk -F'\t' '$1 ~ /^[0-9]+$/ {printf "%s ", $1}' | grep -qx '1 2 3 ' || fail "--rows 1..3 lists other entries"
[ "$(run --rows 3 | awk -F'\t' '$1 ~ /^[0-9]+$/' | cut -f1-4)" = "$(row 3 | cut -f1-4)" ] || fail "--rows 3 changes entry 3's numbers"
run --rows 1..3 | grep -q '^# rows 1\.\.3 of 5 entries: 8 cases' || fail "--rows header line"
run --rows 1..3 --sessions 2 | grep -v '^#' | awk -F'\t' 'NR > 1 {if (NR == 2 && $2 != 1) bad = 1; last = $3; n++} END {exit bad || last != 3 || n != 2}' || fail "--rows 1..3 --sessions 2 ranges"
run --rows 2..4 --session-budget 100000 | grep -v '^#' | awk -F'\t' 'NR == 2 {exit !($2 == 2 && $3 == 4)}' || fail "--rows 2..4 --session-budget range"
for bad in 3..1 x 1..9 7 1..; do
  rc=0; run --rows "$bad" > "$tmp/badout" 2> "$tmp/baderr" || rc=$?
  [ "$rc" = 2 ] || fail "--rows $bad exited $rc, expected 2"
  grep -q 'bad --rows' "$tmp/baderr" || fail "--rows $bad gave no message"
  [ ! -s "$tmp/badout" ] || fail "--rows $bad wrote output"
done

# --gles2: the 29-row full tree against the GLES2 mustpass, assumed rates, GLES3 slices and Pi 4 record not used
{
  for i in 1 2 3; do echo "dEQP-GLES2.functional.color_clear.c$i"; done
  for i in 1 2 3 4; do echo "dEQP-GLES2.functional.shaders.loops.s$i"; done
  for i in 1 2; do echo "dEQP-GLES2.functional.fbo.f$i"; done
} > "$tmp/mp2.txt"
{
  echo 'dEQP-GLES2.functional.color_clear.*'
  echo 'dEQP-GLES2.functional.shaders.loops.*'
  echo 'dEQP-GLES2.functional.fbo.*'
} > "$tmp/t2.txt"
run2() { python3 "$py" --gles2 --table "$tmp/t2.txt" --mustpass "$tmp/mp2.txt" --record "$tmp/rec" \
         --pi4-table "$tmp/pi4.txt" --gles2-results /nonexistent --board-slices "$tmp/slices/*" "$@"; }
run2 > "$tmp/o2.tsv" 2> "$tmp/e2.txt" || fail "--gles2 run failed: $(cat "$tmp/e2.txt")"
grep -q '^# i.MX8MP dEQP-GLES2 sessions: 3 entries, 9 mustpass cases' "$tmp/o2.tsv" || fail "--gles2 header"
[ "$(awk -F'\t' '$1 == 1 {print $2, $3}' "$tmp/o2.tsv")" = "4 1.2" ] || fail "--gles2 shader row: $(awk -F'\t' '$1 == 1' "$tmp/o2.tsv")"
[ "$(awk -F'\t' '$1 == 2 {print $3}' "$tmp/o2.tsv")" = 1.5 ] || fail "--gles2 fbo seconds"
awk -F'\t' '$1 ~ /^[0-9]+$/ && $4 !~ /^assumed:/ {bad = 1} END {exit bad}' "$tmp/o2.tsv" || fail "--gles2 rate not marked assumed"
run2 --sessions 2 | grep -v '^#' | check_ranges 2 || fail "--gles2 --sessions coverage"
# times in the GLES2 results file give measured rates (seconds column, per entry)
printf '# index\tfilter\tbegun\tseconds\n0\tx\t3\t30\n' > "$tmp/res2.tsv"
run2 --gles2-results "$tmp/res2.tsv" | awk -F'\t' '$1 == 0 {print $3, $4}' | grep -qx '30.0 imx8mp-gles2:res2.tsv' || fail "--gles2 times not used"
# the real 29-row table and the GLES2 mustpass plan 29 entries; --rows 0..28 is the same plan
pyr() { python3 "$py" --gles2 "$@"; }
pyr > "$tmp/full2.tsv" 2> "$tmp/fe.txt" || fail "real --gles2 run: $(cat "$tmp/fe.txt")"
[ "$(grep -v '^#' "$tmp/full2.tsv" | awk -F'\t' '$1 ~ /^[0-9]+$/' | wc -l)" = 29 ] || fail "--gles2 did not list 29 rows"
pyr --rows 0..28 | grep -v '^# rows' | cmp -s - "$tmp/full2.tsv" || fail "--gles2 --rows 0..28 differs from --gles2"
pyr --sessions 1 | grep -v '^#' | awk -F'\t' 'NR == 2 {exit !($2 == 0 && $3 == 28 && $4 == 29)}' || fail "--gles2 one session over 29 rows"
echo "imx8mp-gles3-sessions-selftest: ok"
