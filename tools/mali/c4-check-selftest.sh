#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Host only, no cargo, no dEQP: tools/mali/c4-check.sh over synthetic
# fixtures written under $TMPDIR. One board-form baseline slice (results.tsv
# and refusals.tsv) holds a few cases of each of the nine refusals of
# criterion c4; synthetic host measurements (measure.tsv) hold fewer, equal
# or more. Checked: a passing measure exits 0 with nine ok lines (name,
# base, now, limit); a cap exceeded, a strict-decrease count that only stays
# equal, and a refusal that grew each exit 1 with FAIL on that line only; a
# missing measure.tsv exits 2; --help names the caps and the strict list.
#
#   tools/mali/c4-check-selftest.sh   prints "...: N checks passed"
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
C4=$HERE/c4-check.sh
EXPECT=15
base=${TMPDIR:-$REPO/build}
mkdir -p "$base"
tmp=$(mktemp -d "$base/c4-check-selftest-XXXXXX")
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

# name:stage:text (the board text, "... is not lowered yet" kept)
NAMES=(vsunif vsloop vsconst fsunif vsuniarr fsint mixprec varyload irindex)
declare -A STAGE TEXT
STAGE=([vsunif]=vertex [vsloop]=vertex [vsconst]=vertex [fsunif]=fragment [vsuniarr]=vertex
  [fsint]=fragment [mixprec]=fragment [varyload]=fragment [irindex]=fragment)
TEXT=(
  [vsunif]="Valhall: a uniform other than a float, vec2, vec3, vec4 or mat4 is not lowered yet"
  [vsloop]="Valhall: a loop in a vertex shader is not lowered yet"
  [vsconst]="Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet"
  [fsunif]="Valhall: a mediump fragment shader uniform other than a float is not lowered yet"
  [vsuniarr]="Valhall: a uniform array is not lowered yet"
  [fsint]="Valhall: an integer or boolean value is not lowered yet"
  [mixprec]="Valhall: a fragment shader that mixes highp and mediump or lowp float values is not lowered yet"
  [varyload]="Valhall: a fragment varying load or conversion of a form no reference draws is not lowered yet"
  [irindex]="Valhall back end defect: an invariant of the pipeline did not hold (IrIndex))"
)

# mkslice <dir> <n per refusal>: board form
mkslice() {
  local d=$1 n=$2 k i
  mkdir -p "$d"
  printf 'case\tstatus\tdetail\n' > "$d/results.tsv"
  printf 'case\ttext\n' > "$d/refusals.tsv"
  for k in "${NAMES[@]}"; do
    for ((i = 0; i < n; i++)); do
      printf 'dEQP-GLES2.%s.%d\tFail\tx\n' "$k" "$i" >> "$d/results.tsv"
      printf 'dEQP-GLES2.%s.%d\tonline shaderc refused this %s shader: Mali(%s)\n' \
        "$k" "$i" "${STAGE[$k]/vertex/vertex}" "${TEXT[$k]}" >> "$d/refusals.tsv"
    done
  done
}
# mkmeasure <dir> <name=count>...: host form (text cut at ' is not lowered')
mkmeasure() {
  local d=$1 kv k n i st t
  shift
  mkdir -p "$d"
  printf 'case\tboard\tfam\thost\tstage\thfam\tshape\ttext\n' > "$d/measure.tsv"
  for kv in "$@"; do
    k=${kv%=*}
    n=${kv#*=}
    case ${STAGE[$k]} in vertex) st=vs ;; *) st=fs ;; esac
    t=${TEXT[$k]%% is not lowered*}
    t=${t%)}
    for ((i = 0; i < n; i++)); do
      printf 'dEQP-GLES2.%s.%d\tFail\tf\tFail\t%s\tfam\tshape\t%s\n' "$k" "$i" "$st" "$t" >> "$d/measure.tsv"
    done
  done
}

mkslice "$tmp/base" 5
run() { bash "$C4" "$@" --base "$tmp/base"; }

# pass: caps met (5 and 4), others fewer
mkmeasure "$tmp/pass" vsunif=5 vsloop=0 vsconst=3 fsunif=4 vsuniarr=0 fsint=1 mixprec=2 varyload=4 irindex=0
check "a passing measure exits 0" run "$tmp/pass"
out=$(run "$tmp/pass" 2>&1)
check "it prints nine lines, all ok" bash -c "[ \$(grep -c ' ok\$' <<<'$out') = 9 ]"
check "a line carries name, base, now and limit" grep -q 'vsunif .*base=5 now=5 limit=<=362 ok' <<<"$out"
check "a strict line's limit is below the base" grep -q 'fsint .*base=5 now=1 limit=<5 ok' <<<"$out"

# cap exceeded: vsloop 6 > 105 is not reachable with five cases, so give the
# cap a fixture of 106 cases
mkmeasure "$tmp/capfail" vsunif=5 vsloop=106 vsconst=3 fsunif=4 vsuniarr=0 fsint=1 mixprec=2 varyload=4 irindex=0
check "a cap exceeded exits 1" bash -c "! bash '$C4' '$tmp/capfail' --base '$tmp/base'"
out=$(bash "$C4" "$tmp/capfail" --base "$tmp/base" 2>&1)
check "only the vsloop line is FAIL" bash -c "[ \$(grep -c 'FAIL\$' <<<'$out') = 1 ] && grep -q '^vsloop .*now=106 limit=<=105 FAIL' <<<'$out'"

# strict decrease: equal is a FAIL
mkmeasure "$tmp/equal" vsunif=5 vsloop=5 vsconst=5 fsunif=5 vsuniarr=0 fsint=1 mixprec=2 varyload=4 irindex=0
check "an equal count of a strict refusal exits 1" bash -c "! bash '$C4' '$tmp/equal' --base '$tmp/base'"
out=$(bash "$C4" "$tmp/equal" --base "$tmp/base" 2>&1)
check "only the fsunif line is FAIL" bash -c "[ \$(grep -c 'FAIL\$' <<<'$out') = 1 ] && grep -q '^fsunif .*now=5 limit=<5 FAIL' <<<'$out'"

# grown
mkmeasure "$tmp/grown" vsunif=5 vsloop=5 vsconst=5 fsunif=4 vsuniarr=0 fsint=1 mixprec=2 varyload=4 irindex=9
check "a grown IrIndex count exits 1" bash -c "! bash '$C4' '$tmp/grown' --base '$tmp/base'"
out=$(bash "$C4" "$tmp/grown" --base "$tmp/base" 2>&1)
check "irindex is FAIL with now=9" grep -q '^irindex .*now=9 limit=<5 FAIL' <<<"$out"

# a refusal the host no longer has at all is 0, not an error (a base has it)
mkmeasure "$tmp/gone" fsunif=1
check "a measure with most refusals gone exits 0" run "$tmp/gone"

# not runnable
mkdir -p "$tmp/empty"
bash "$C4" "$tmp/empty" --base "$tmp/base" > /dev/null 2>&1
rc=$?
check "a directory without measure.tsv exits 2" test "$rc" = 2
bash "$C4" "$tmp/pass" --base "$tmp/nonexistent" > /dev/null 2>&1
rc=$?
check "a missing baseline slice exits 2 or 1, not 0" test "$rc" != 0

bash "$C4" --help > "$tmp/help.txt" 2>&1
check "--help lists the three caps" bash -c "grep -q '<= 362' '$tmp/help.txt' && grep -q '<= 105' '$tmp/help.txt' && grep -q '<= 114' '$tmp/help.txt'"
check "--help lists the strict-decrease refusals" bash -c "grep -q 'Strict decrease' '$tmp/help.txt' && grep -q 'IrIndex' '$tmp/help.txt' && grep -q 'varyload' '$tmp/help.txt'"

total=$((passed + failed))
if [ "$failed" -ne 0 ] || [ "$total" -ne "$EXPECT" ]; then
  echo "c4-check-selftest: $failed of $total checks failed (the self-test pins $EXPECT checks)" >&2
  exit 1
fi
echo "c4-check-selftest: $total checks passed"
