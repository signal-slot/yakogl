#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Host only, no CTS, no library: the refusal extract of
# tools/mali/deqp-host-measure.sh over fixture logs written here (heredocs
# under $TMPDIR). A fake deqp-gles2 (DEQP_RUNNER) plays the host run of four
# real mustpass cases: one refused at the link with the library's text that
# lacks the word 'refused' (#11309, docs/es2-cts-failures.md M.1 row 15),
# two refused by the back end, whose only trace is a 'yakogl-mali-refusal:'
# line in the chunk's c_NNN.out (row 17: one of them logs an empty info log,
# the other never reads one), and one clean Pass. Checked: board.tsv keeps
# the link refusal, measure.tsv gives the link case stage and family 'link'
# (the reason as its shape, the varying's name left out) and the two back-end
# cases family 'back_end' with their line's reason - the first line of a
# case wins and a line before any 'Test case' line belongs to no case -
# summary.md counts three refusals, and only the clean case is not
# reproduced. The patterns are those of the board tools (mali-deqp-run.sh's
# qpa_refusal and LINK_REFUSAL, mali-deqp-tally.py's --per-case), which the
# extract now shares.
#
#   tools/mali/deqp-host-measure-selftest.sh   prints "...: N checks passed"
#
# Host only, a second. Gate: a row of its own (tools/check.sh).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
MEASURE=$HERE/deqp-host-measure.sh
EXPECT=13
# real case names of tools/cts/gles2-mustpass.txt (--cases reads the list as
# it is; the --slice side checks them against the board's texts)
LINKC=dEQP-GLES2.functional.attribute_location.bind.float
BEC=dEQP-GLES2.functional.uniform_api.info_query.array_in_struct.sampler2D_samplerCube_vertex
NLOGC=dEQP-GLES2.functional.uniform_api.info_query.basic_struct.sampler2D_samplerCube_vertex
PASSC=dEQP-GLES2.info.version
LINK_TEXT="online shaderc: a Mali link whose stages place a varying at different offsets is not lowered yet ('v_b')"
BE1="mali: a vertex shader that reads more than one texture is not built yet"
BE2="mali: several colour targets of different sizes are not built yet"
STRAY="mali: a refusal before any Test case line"
export LINK_TEXT BE1 BE2 STRAY
base=${TMPDIR:-$REPO/build}
mkdir -p "$base"
tmp=$(mktemp -d "$base/deqp-host-measure-selftest-XXXXXX")
trap 'rm -rf "$tmp"' EXIT
passed=0 failed=0
check() { # <what> <command...>
  local what=$1
  shift
  if "$@" > /dev/null 2>&1; then
    passed=$((passed + 1))
    echo "  ok    $what"
  else
    failed=$((failed + 1))
    echo "  FAIL  $what"
  fi
}

mkdir -p "$tmp/slice"
# the board slice: only the link refusal is in its refusals.tsv (a back-end
# refusal is not in an info log on the board either; --cases stands in for
# the whole tree there)
{
  printf 'case\tstatus\n'
  printf '%s\tFail\n' "$LINKC"
} > "$tmp/slice/results.tsv"
{
  printf 'case\ttext\n'
  printf '%s\t%s\n' "$LINKC" "$LINK_TEXT"
} > "$tmp/slice/refusals.tsv"
printf '%s\n' "$LINKC" "$BEC" "$NLOGC" "$PASSC" > "$tmp/cases.txt"
printf '%s\t%s\n' "$LINKC" link "$BEC" beref "$NLOGC" nolog "$PASSC" none > "$tmp/scenario.tsv"
cat > "$tmp/runner.sh" <<'EOF'
#!/bin/bash
# a fixture deqp-gles2: on the stream run_chunk keeps in c_NNN.out it prints
# dEQP's "Test case '<case>' .." lines and, for the back-end kinds, the
# library's 'yakogl-mali-refusal:' lines; in the .qpa it writes each case's
# result block (the link kind with the link refusal in the program's info
# log, the beref kind with an empty one, the nolog kind without any log).
list=$1 log=$2
kind() { awk -F'\t' -v c="$1" '$1 == c { print $2; exit }' "${SCENARIO:-/dev/null}"; }
printf 'yakogl-mali-refusal: -95 %s\n' "$STRAY"
while read -r c; do
  [ -n "$c" ] || continue
  echo "Test case '$c' .."
  k=$(kind "$c")
  st=Pass
  [ "$k" = none ] || st=Fail
  printf '#beginTestCaseResult %s\n' "$c" >> "$log"
  printf '<TestCaseResult Version="0.3.4" CasePath="%s"><Result StatusCode="%s">%s</Result></TestCaseResult>\n' "$c" "$st" "$st" >> "$log"
  case $k in
    link) printf '<ShaderProgram LinkStatus="false"><InfoLog>%s</InfoLog></ShaderProgram>\n' "$LINK_TEXT" >> "$log" ;;
    beref)
      printf '<ShaderProgram LinkStatus="false"><InfoLog></InfoLog></ShaderProgram>\n' >> "$log"
      printf 'yakogl-mali-refusal: -95 %s\n' "$BE1"
      printf 'yakogl-mali-refusal: -95 %s\n' "$BE2" ;;
    nolog) printf 'yakogl-mali-refusal: -95 %s\n' "$BE1" ;;
  esac
  printf '#endTestCaseResult\n' >> "$log"
done < "$list"
EOF
chmod +x "$tmp/runner.sh"
out=$tmp/out

check "the run stage over the fixture logs exits 0" \
  env DEQP_RUNNER="$tmp/runner.sh" SCENARIO="$tmp/scenario.tsv" \
  bash "$MEASURE" --out "$out" --slice "$tmp/slice" --cases "$tmp/cases.txt" --jobs 1 --chunk 10 --stage run
check "the extract stage over them exits 0" bash "$MEASURE" --out "$out" --stage extract
check "a link refusal without the word 'refused' reaches board.tsv (#11309, M.1 row 15)" \
  grep -qF 'varying at different offsets' "$out/board.tsv"
check "all four cases ran" \
  bash -c "[ \$(wc -l < '$out/cases.txt') = 4 ]"
check "the link case reads as a link refusal (stage 'link', family 'link')" \
  awk -F'\t' -v c="$LINKC" '$1 == c && $5 == "link" && $6 == "link" { found = 1 } END { exit !found }' "$out/measure.tsv"
check "its shape is the link's reason, the varying's name left out" \
  awk -F'\t' -v c="$LINKC" -v w='a Mali link whose stages place a varying at different offsets is not lowered yet' \
    '$1 == c && $7 == w { found = 1 } END { exit !found }' "$out/measure.tsv"
check "a back-end refusal in c_NNN.out gives family 'back_end' and its reason (M.1 row 17)" \
  awk -F'\t' -v c="$BEC" '$1 == c && $5 == "be" && $6 == "back_end" && index($8, "vertex shader that reads more than one texture") { found = 1 } END { exit !found }' "$out/measure.tsv"
check "a case that never read its info log is counted the same way" \
  awk -F'\t' -v c="$NLOGC" '$1 == c && $5 == "be" && $6 == "back_end" { found = 1 } END { exit !found }' "$out/measure.tsv"
check "a case keeps its first refusal line" \
  bash -c "! grep -q 'several colour targets' '$out/measure.tsv'"
check "a refusal line before any Test case line belongs to no case" \
  bash -c "! grep -q 'before any Test case' '$out/measure.tsv'"
check "the clean Pass case keeps no host family" \
  awk -F'\t' -v c="$PASSC" '$1 == c && $6 == "" { found = 1 } END { exit !found }' "$out/measure.tsv"
check "only the clean case is not reproduced, by 'no refusal on the host'" \
  bash -c "[ \$(wc -l < '$out/not-reproduced.tsv') = 2 ] && grep -q 'no refusal on the host (dEQP Pass)' '$out/not-reproduced.tsv'"
check "summary.md counts three refusals of the four cases" \
  grep -q '4 cases asked, 4 began on the host, 3 with a refusal on the host' "$out/summary.md"

total=$((passed + failed))
if [ "$failed" -ne 0 ] || [ "$total" -ne "$EXPECT" ]; then
  echo "deqp-host-measure-selftest: $failed of $total checks failed (the self-test pins $EXPECT checks)" >&2
  exit 1
fi
echo "deqp-host-measure-selftest: $total checks passed"
