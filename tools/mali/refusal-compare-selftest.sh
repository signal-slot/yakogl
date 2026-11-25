#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Host only, no cargo, no dEQP: tools/mali/refusal-compare.sh over synthetic
# measurements written under $TMPDIR. Checked: equal counts, growth, shrink,
# a refusal on one side only, a --cap that holds and one that is exceeded
# (exit 1), --fewer, --status, a cause text with commas, a host measure.tsv
# input, and the exit codes and messages of bad arguments.
#
#   tools/mali/refusal-compare-selftest.sh   prints "N checks ok"
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
RC=$HERE/refusal-compare.sh
base=${TMPDIR:-$REPO/build}
mkdir -p "$base"
tmp=$(mktemp -d "$base/refusal-compare-selftest-XXXXXX")
trap 'rm -rf "$tmp"' EXIT
passed=0 failed=0
ok() { passed=$((passed + 1)); }
bad() { failed=$((failed + 1)); echo "  FAIL  $1"; }

# run <what> <want exit> <want output substring or ''> args...
run() {
  local what=$1 want=$2 sub=$3 out rc
  shift 3
  out=$("$RC" "$@" 2>&1)
  rc=$?
  if [ "$rc" -ne "$want" ]; then
    bad "$what: exit $rc, want $want"
    printf '%s\n' "$out" | sed 's/^/        /'
  elif [ -n "$sub" ] && ! grep -qF -- "$sub" <<<"$out"; then
    bad "$what: output lacks '$sub'"
    printf '%s\n' "$out" | sed 's/^/        /'
  else
    ok
  fi
}

# mkslice <dir> <status> <stage> <why> <n> [<tag>]: n cases of one refusal,
# appended to a board-form slice
mkslice() {
  local d=$1 st=$2 stage=$3 why=$4 n=$5 tag=${6:-} i id
  mkdir -p "$d"
  [ -f "$d/results.tsv" ] || printf 'case\tstatus\tdetail\n' > "$d/results.tsv"
  [ -f "$d/refusals.tsv" ] || printf 'case\ttext\n' > "$d/refusals.tsv"
  id=$(printf '%s' "$why" | cksum | cut -d' ' -f1)
  for ((i = 1; i <= n; i++)); do
    printf 'c.%s%s.%s.%s\t%s\t-\n' "$tag" "$stage" "$id" "$i" "$st" >> "$d/results.tsv"
    printf 'c.%s%s.%s.%s\tBackend refused this %s shader: Mali(%s)\n' "$tag" "$stage" "$id" "$i" "$stage" "$why" >> "$d/refusals.tsv"
  done
}

LOOP="Valhall: a loop in a vertex shader is not lowered yet"
UNIF="Valhall: a uniform other than a float, vec2 or mat4 is not lowered yet"
COMMA="Valhall: a, b, and c, together are not lowered yet"
ONLY_B="Valhall: only in base is not lowered yet"
ONLY_N="Valhall: only in new is not lowered yet"

B=$tmp/base
N=$tmp/new
mkslice "$B" Fail vertex "$LOOP" 5
mkslice "$B" Fail vertex "$UNIF" 4
mkslice "$B" Fail vertex "$COMMA" 3
mkslice "$B" Fail vertex "$ONLY_B" 2
mkslice "$B" NotSupported vertex "$LOOP" 7 ns
mkslice "$B" Pass vertex "$LOOP" 9 pass
mkslice "$N" Fail vertex "$LOOP" 8
mkslice "$N" Fail vertex "$UNIF" 4
mkslice "$N" Fail vertex "$COMMA" 1
mkslice "$N" Fail vertex "$ONLY_N" 6
mkslice "$N" NotSupported vertex "$LOOP" 7 ns
mkslice "$N" Pass vertex "$LOOP" 9 pass

# counts: growth, equal, shrink, one side only; Pass never counted
run "growth line" 0 "5 -> 8  +3  refused, vertex shader: $LOOP" --base "$B" --new "$N"
run "equal line" 0 "4 -> 4  0  refused, vertex shader: $UNIF" --base "$B" --new "$N"
run "shrink line" 0 "3 -> 1  -2  refused, vertex shader: $COMMA" --base "$B" --new "$N"
run "base-only line" 0 "2 -> 0  -2  refused, vertex shader: $ONLY_B" --base "$B" --new "$N"
run "new-only line" 0 "0 -> 6  +6  refused, vertex shader: $ONLY_N" --base "$B" --new "$N"
run "total (Pass and NotSupported excluded)" 0 "# total refused of status Fail: 14 -> 19" --base "$B" --new "$N"
run "same set both sides: no growth" 0 "# total refused of status Fail: 14 -> 14" --base "$B" --new "$B"
run "--status sums two statuses" 0 "12 -> 15  +3  refused, vertex shader: $LOOP" --base "$B" --new "$N" --status Fail,NotSupported
run "--status NotSupported only" 0 "7 -> 7  0  refused, vertex shader: $LOOP" --base "$B" --new "$N" --status NotSupported

# order: most grown first
first=$("$RC" --base "$B" --new "$N" 2>&1 | sed -n 3p)
if [[ $first == "0 -> 6"* ]]; then ok; else bad "most grown first: '$first'"; fi

# caps and fewer
run "cap holds" 0 "PASS cap 'a loop in a vertex shader' <= 8: base 5, new 8" --base "$B" --new "$N" --cap 'a loop in a vertex shader=8'
run "cap exceeded exits 1" 1 "FAIL cap 'a loop in a vertex shader' <= 7: base 5, new 8" --base "$B" --new "$N" --cap 'a loop in a vertex shader=7'
run "cap 0 on a vanished cause holds" 0 "PASS cap 'only in base' <= 0" --base "$B" --new "$N" --cap 'only in base=0'
run "cap with commas in the reason" 0 "PASS cap 'a, b, and c' <= 1: base 3, new 1" --base "$B" --new "$N" --cap 'a, b, and c=1'
run "cap with commas exceeded" 1 "FAIL cap 'a, b, and c' <= 0" --base "$B" --new "$N" --cap 'a, b, and c=0'
run "cap with = in the reason matches nothing" 2 "matches no cause" --base "$B" --new "$N" --cap 'a=b=3'
run "cap summing several causes" 0 "base 9, new 12 (2 cause(s))" --base "$B" --new "$N" --cap 'Valhall: a =100'
run "one FAIL among PASS still exits 1" 1 "PASS cap" --base "$B" --new "$N" --cap 'only in new=6' --cap 'a loop=1'
run "fewer holds" 0 "PASS fewer 'a, b, and c': base 3, new 1" --base "$B" --new "$N" --fewer 'a, b, and c'
run "fewer on equal fails" 1 "FAIL fewer 'a uniform other': base 4, new 4" --base "$B" --new "$N" --fewer 'a uniform other'
run "fewer on growth fails" 1 "FAIL fewer 'a loop'" --base "$B" --new "$N" --fewer 'a loop'

# several slices in one set; a case in two slices is counted once, later wins
B2=$tmp/base2
mkslice "$B2" Fail vertex "$LOOP" 2 other
run "two base slices sum" 0 "7 -> 8  +1  refused, vertex shader: $LOOP" --base "$B2" "$B" --new "$N"
R=$tmp/rerun
mkdir -p "$R"
printf 'case\tstatus\tdetail\n' > "$R/results.tsv"
printf 'case\ttext\n' > "$R/refusals.tsv"
first_case=$(sed -n 2p "$N/results.tsv" | cut -f1)
printf '%s\tPass\t-\n' "$first_case" >> "$R/results.tsv"
printf '%s\tBackend refused this vertex shader: Mali(%s)\n' "$first_case" "$LOOP" >> "$R/refusals.tsv"
run "later slice's status wins (rerun passes)" 0 "5 -> 7  +2  refused, vertex shader: $LOOP" --base "$B" --new "$N" "$R"
run "trailing slash on a dir" 0 "5 -> 8" --base "$B/" --new "$N/"

# refusal text forms
T=$tmp/forms
mkdir -p "$T"
printf 'case\tstatus\tdetail\na\tFail\t-\nb\tFail\t-\nc\tFail\t-\n' > "$T/results.tsv"
printf 'case\ttext\na\tlink error: x refused this fragment shader: Mali(Valhall: f is not lowered yet)\nb\tsomething else, entirely\nc\tBackend refused this vertex shader: Mali(Valhall: cut sh\n' > "$T/refusals.tsv"
run "link error reads as its stage" 0 "0 -> 1  +1  refused, fragment shader: Valhall: f is not lowered yet" --base "$B" --new "$T"
run "text without a stage" 0 "refused: something else, entirely" --base "$B" --new "$T"
run "truncated text kept as is" 0 "refused, vertex shader: Valhall: cut sh" --base "$B" --new "$T"

# host measure.tsv form: case status ? ? stage family ? text
M=$tmp/host
mkdir -p "$M"
printf 'case\tstatus\tx\ty\tstage\tfamily\tz\ttext\n' > "$M/measure.tsv"
printf 'h1\tFail\t-\t-\tvs\tvalhall\t-\tValhall: a loop, in a vertex shader\n' >> "$M/measure.tsv"
printf 'h2\tFail\t-\t-\tfs\tvalhall\t-\tValhall: fs thing\n' >> "$M/measure.tsv"
printf 'h3\tFail\t-\t-\t-\tvalhall\t-\tother\n' >> "$M/measure.tsv"
printf 'h4\tFail\t-\t-\tvs\t\t-\tnot refused\n' >> "$M/measure.tsv"
printf 'h5\tPass\t-\t-\tvs\tvalhall\t-\tValhall: a loop, in a vertex shader\n' >> "$M/measure.tsv"
run "host vs line" 0 "0 -> 1  +1  refused, vertex shader: Valhall: a loop, in a vertex shader" --base "$B" --new "$M"
run "host fs line" 0 "refused, fragment shader: Valhall: fs thing" --base "$B" --new "$M"
run "host no stage line" 0 "refused: other" --base "$B" --new "$M"
run "host total counts refused Fail only" 0 "# total refused of status Fail: 14 -> 3" --base "$B" --new "$M"
run "host against host" 0 "1 -> 1  0  refused, vertex shader: Valhall: a loop, in a vertex shader" --base "$M" --new "$M" --cap 'a loop, in=1'

# empty measurements
E=$tmp/empty
mkdir -p "$E"
printf 'case\tstatus\tdetail\n' > "$E/results.tsv"
printf 'case\ttext\n' > "$E/refusals.tsv"
run "empty both sets" 0 "# no refused case of status Fail in either set" --base "$E" --new "$E"
run "empty base" 0 "0 -> 8  +8  refused, vertex shader: $LOOP" --base "$E" --new "$N"
run "empty new" 0 "5 -> 0  -5  refused, vertex shader: $LOOP" --base "$B" --new "$E"

# --help and bad arguments
run "--help" 0 "refusal-compare.sh - the per-refusal" --help
run "no arguments" 2 "no --base slice"
run "--base only" 2 "no --new slice" --base "$B"
run "--new only" 2 "no --base slice" --new "$N"
run "unknown option" 2 "unknown option '--bogus'" --bogus --base "$B" --new "$N"
run "slice outside --base/--new" 2 "names a slice outside" "$B"
run "not a slice" 1 "neither a slice" --base "$tmp" --new "$N"
mkdir -p "$tmp/norefusals"
printf 'case\tstatus\tdetail\n' > "$tmp/norefusals/results.tsv"
run "slice without refusals.tsv" 1 "has no refusals.tsv" --base "$tmp/norefusals" --new "$N"
run "missing dir" 1 "neither a slice" --base "$tmp/nonexistent" --new "$N"
run "--cap without N" 2 "N after the last =" --base "$B" --new "$N" --cap 'a loop'
run "--cap with empty reason" 2 "--cap needs 'REASON=N'" --base "$B" --new "$N" --cap '=3'
run "--cap non-numeric" 2 "N is not a number" --base "$B" --new "$N" --cap 'a loop=x'
run "--cap empty N" 2 "N is not a number" --base "$B" --new "$N" --cap 'a loop='
run "--cap unmatched reason" 2 "matches no cause" --base "$B" --new "$N" --cap 'no such cause=3'
run "--fewer unmatched reason" 2 "matches no cause" --base "$B" --new "$N" --fewer 'no such cause'
run "--fewer empty reason" 2 "--fewer needs 'REASON'" --base "$B" --new "$N" --fewer ''
run "--cap missing value" 2 "--cap needs" --base "$B" --new "$N" --cap
run "--status missing value" 2 "--status needs" --base "$B" --new "$N" --status

# the usage text on a bad call shows the Usage line
out=$("$RC" 2>&1)
if grep -q 'refusal-compare.sh --base' <<<"$out"; then ok; else bad "usage text shows the Usage line: $out"; fi

if [ "$failed" -ne 0 ]; then
  echo "refusal-compare-selftest: $failed of $((passed + failed)) checks FAILED" >&2
  exit 1
fi
echo "$passed checks ok"
