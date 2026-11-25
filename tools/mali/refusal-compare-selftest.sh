#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Host only, no cargo, no dEQP: tools/mali/refusal-compare.sh over synthetic
# measurements written under $TMPDIR, in the two forms it reads: board-form
# slices (results.tsv + refusals.tsv) and a host measure.tsv (the columns of
# tools/mali/deqp-host-measure.sh's extract). Checked: equal counts, growth,
# shrink, a refusal only on one side, a --cap that holds and one that is
# exceeded (exit 1), a --cap that sums several causes, --fewer, --status, a
# cause text containing commas, the output's shape (comment line first, most
# grown first, "# total" last), the refusal text forms (Mali(...) taken off,
# a text cut short, a text without a stage, a link error, a tab joined to a
# space, a refusal with no results.tsv row), the host form (stage and text
# columns, stage 'be', its crash and not-run rows skipped, the board status
# column selects the count), a case in two slices of one set counted once
# with the later status and refusal winning, empty sets, and the exit codes
# and messages of bad arguments (--help 0; a missing --base/--new set, an
# unknown option, a positional outside them and a valueless --status, --cap
# or --fewer 2 with the usage; a directory that is neither form 1).
#
#   tools/mali/refusal-compare-selftest.sh   prints "...: N checks ok"
#
# Host only, a second. Gate: a row of its own (tools/check.sh).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
RC=$HERE/refusal-compare.sh
EXPECT=68
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
# present/absent <what> <substring> <output>: the text must or must not be
# there (checks of what is never counted)
present() {
  if grep -qF -- "$2" <<<"$3"; then ok; else bad "$1: output lacks '$2'"; fi
}
absent() {
  if grep -qF -- "$2" <<<"$3"; then bad "$1: output has '$2'"; else ok; fi
}

# mkslice <dir> <status> <stage> <why> <n> [<tag>]: n cases of one refusal,
# appended to a board-form slice (the text form mali-deqp-run.sh writes)
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

# an empty board-form measurement (a slice that refused nothing)
E=$tmp/empty
mkdir -p "$E"
printf 'case\tstatus\tdetail\n' > "$E/results.tsv"
printf 'case\ttext\n' > "$E/refusals.tsv"

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

# counts: growth, equal, shrink, one side only, Pass never counted
run "growth line" 0 "5 -> 8  +3  refused, vertex shader: $LOOP" --base "$B" --new "$N"
run "equal line" 0 "4 -> 4  0  refused, vertex shader: $UNIF" --base "$B" --new "$N"
run "shrink line" 0 "3 -> 1  -2  refused, vertex shader: $COMMA" --base "$B" --new "$N"
run "base-only line" 0 "2 -> 0  -2  refused, vertex shader: $ONLY_B" --base "$B" --new "$N"
run "new-only line" 0 "0 -> 6  +6  refused, vertex shader: $ONLY_N" --base "$B" --new "$N"
run "total (a Pass case logging a refusal is never counted)" 0 "# total refused of status Fail: 14 -> 19" --base "$B" --new "$N"
run "same set both sides: no growth" 0 "# total refused of status Fail: 14 -> 14" --base "$B" --new "$B"

# output shape: comment line, then the rows most grown first (line 2 is the
# +6 row, line 3 the +3 row), "# total" last
out=$("$RC" --base "$B" --new "$N")
first=$(sed -n 1p <<<"$out")
second=$(sed -n 2p <<<"$out")
third=$(sed -n 3p <<<"$out")
last=$(tail -n 1 <<<"$out")
case $first in "# refusal-compare: status Fail;"*) ok ;; *) bad "comment line first: '$first'" ;; esac
case $second in "0 -> 6"*) ok ;; *) bad "most grown first: line 2 is '$second'" ;; esac
case $third in "5 -> 8"*) ok ;; *) bad "next most grown: line 3 is '$third'" ;; esac
case $last in "# total refused"*) ok ;; *) bad "'# total' line last: '$last'" ;; esac

# --status selects and sums statuses (Pass still never counted)
run "--status sums Fail and NotSupported" 0 "12 -> 15  +3  refused, vertex shader: $LOOP" --base "$B" --new "$N" --status Fail,NotSupported
run "--status NotSupported only" 0 "7 -> 7  0  refused, vertex shader: $LOOP" --base "$B" --new "$N" --status NotSupported
run "--status sums the two statuses in the total" 0 "# total refused of status Fail,NotSupported: 21 -> 26" --base "$B" --new "$N" --status Fail,NotSupported

# caps and fewer
run "cap holds" 0 "PASS cap 'a loop in a vertex shader' <= 8: base 5, new 8" --base "$B" --new "$N" --cap 'a loop in a vertex shader=8'
run "cap exceeded exits 1" 1 "FAIL cap 'a loop in a vertex shader' <= 7: base 5, new 8" --base "$B" --new "$N" --cap 'a loop in a vertex shader=7'
run "cap 0 on a vanished cause holds" 0 "PASS cap 'only in base' <= 0" --base "$B" --new "$N" --cap 'only in base=0'
run "cap with commas in the reason holds" 0 "PASS cap 'a, b, and c' <= 1: base 3, new 1" --base "$B" --new "$N" --cap 'a, b, and c=1'
run "cap with commas exceeded exits 1" 1 "FAIL cap 'a, b, and c' <= 0" --base "$B" --new "$N" --cap 'a, b, and c=0'
run "cap with = in the reason matches nothing" 2 "matches no cause" --base "$B" --new "$N" --cap 'a=b=3'
run "cap summing several causes" 0 "base 9, new 12 (2 cause(s))" --base "$B" --new "$N" --cap 'Valhall: a =100'
run "one FAIL among PASS exits 1" 1 "PASS cap" --base "$B" --new "$N" --cap 'only in new=6' --cap 'a loop=1'
run "fewer holds" 0 "PASS fewer 'a, b, and c': base 3, new 1" --base "$B" --new "$N" --fewer 'a, b, and c'
run "fewer on equal fails" 1 "FAIL fewer 'a uniform other': base 4, new 4" --base "$B" --new "$N" --fewer 'a uniform other'
run "fewer on growth fails" 1 "FAIL fewer 'a loop'" --base "$B" --new "$N" --fewer 'a loop'
run "a cap and a fewer together all PASS exit 0" 0 "PASS fewer" --base "$B" --new "$N" --cap 'a loop=8' --fewer 'a, b, and c'

# several slices in one set sum; a case in two slices of one set is counted
# once, the later slice's status and refusal winning (a rerun listed after
# what it reruns)
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
run "later slice's status wins (a rerun passes the case)" 0 "5 -> 7  +2  refused, vertex shader: $LOOP" --base "$B" --new "$N" "$R"
S1=$tmp/s1
S2=$tmp/s2
mkdir -p "$S1" "$S2"
printf 'case\tstatus\tdetail\ndup\tFail\t-\n' > "$S1/results.tsv"
printf 'case\ttext\ndup\tBackend refused this vertex shader: Mali(%s)\n' "$UNIF" > "$S1/refusals.tsv"
printf 'case\tstatus\tdetail\ndup\tFail\t-\n' > "$S2/results.tsv"
printf 'case\ttext\ndup\tBackend refused this vertex shader: Mali(%s)\n' "$LOOP" > "$S2/refusals.tsv"
run "later slice's refusal wins (same case, two refusals)" 0 "0 -> 1  +1  refused, vertex shader: $LOOP" --base "$E" --new "$S1" "$S2"
out=$("$RC" --base "$E" --new "$S1" "$S2")
absent "the earlier refusal leaves no cause row" "$UNIF" "$out"

# refusal text forms (the readings mali-deqp-tally.py makes)
T=$tmp/forms
mkdir -p "$T"
printf 'case\tstatus\tdetail\n' > "$T/results.tsv"
printf 'a\tFail\t-\nb\tFail\t-\nc\tFail\t-\nd\tFail\t-\n' >> "$T/results.tsv"
printf 'case\ttext\n' > "$T/refusals.tsv"
printf 'a\tlink error: x refused this fragment shader: Mali(Valhall: f is not lowered yet)\n' >> "$T/refusals.tsv"
printf 'b\tsomething else, entirely\n' >> "$T/refusals.tsv"
printf 'c\tBackend refused this vertex shader: Mali(Valhall: cut sh\n' >> "$T/refusals.tsv"
printf 'd\tBackend refused this vertex shader: Mali(Valhall:\ttabbed cause)\n' >> "$T/refusals.tsv"
printf 'e\ta refusal logged for no results.tsv row\n' >> "$T/refusals.tsv"
run "link error reads as its stage" 0 "0 -> 1  +1  refused, fragment shader: Valhall: f is not lowered yet" --base "$B" --new "$T"
run "a text without a stage reads whole" 0 "refused: something else, entirely" --base "$B" --new "$T"
run "a text cut short is kept as it is" 0 "refused, vertex shader: Valhall: cut sh" --base "$B" --new "$T"
run "a tab in the text joins to a space" 0 "refused, vertex shader: Valhall: tabbed cause" --base "$B" --new "$T"
out=$("$RC" --base "$B" --new "$T")
absent "a case with no results.tsv row is not counted" "no results.tsv row" "$out"

# host measure.tsv form: case board boardfam hoststage? stage hostfam shape text nextfam
# (the columns of deqp-host-measure.sh's extract: board status, board family,
# host status, stage, host family, shape, host text, next family)
M=$tmp/host
mkdir -p "$M"
printf 'case\tboard\tbfam\thost\tstage\thfam\tshape\ttext\tnext\n' > "$M/measure.tsv"
printf 'h1\tFail\tf\tFail\tvs\tvalhall\tshape\tValhall: a loop, in a vertex shader\t-\n' >> "$M/measure.tsv"
printf 'h2\tFail\tf\tFail\tfs\tvalhall\tshape\tValhall: fs thing\t-\n' >> "$M/measure.tsv"
printf 'h3\tFail\tf\tFail\t\tvalhall\tshape\tother\t-\n' >> "$M/measure.tsv"
printf 'h4\tFail\tf\tFail\tvs\t\tshape\tnot refused on the host\t-\n' >> "$M/measure.tsv"
printf 'h5\tPass\tf\tFail\tvs\tvalhall\tshape\tValhall: Pass host refusal\t-\n' >> "$M/measure.tsv"
printf 'h6\tFail\tf\tcrash\t\t\t\texit=139\t\n' >> "$M/measure.tsv"
printf 'h7\tFail\tf\tNotRun\t\t\t\t\t\n' >> "$M/measure.tsv"
printf 'h8\tFail\tf\tPass\tvs\tvalhall\tshape\tValhall: board Fail host Pass\t-\n' >> "$M/measure.tsv"
printf 'h9\tNotSupported\tf\tFail\tfs\tvalhall\tshape\tValhall: board NS host Fail\t-\n' >> "$M/measure.tsv"
printf 'h10\tFail\tf\tFail\tbe\tback_end\tshape\tValhall back end defect: an invariant did not hold (IrIndex))\t-\n' >> "$M/measure.tsv"
run "host vs line" 0 "0 -> 1  +1  refused, vertex shader: Valhall: a loop, in a vertex shader" --base "$B" --new "$M"
run "host fs line" 0 "0 -> 1  +1  refused, fragment shader: Valhall: fs thing" --base "$B" --new "$M"
run "host line with no stage reads whole" 0 "refused: other" --base "$B" --new "$M"
run "host stage be reads as no stage (the back end line)" 0 "refused: Valhall back end defect: an invariant did not hold (IrIndex))" --base "$B" --new "$M"
run "host counts on the board status column, not the host one" 0 "0 -> 1  +1  refused, vertex shader: Valhall: board Fail host Pass" --base "$B" --new "$M"
run "host total: board Fail refusals only (h1 h2 h3 h8 h10)" 0 "# total refused of status Fail: 14 -> 5" --base "$B" --new "$M"
out=$("$RC" --base "$B" --new "$M")
absent "a crash row is not a refusal (its exit=N text)" "exit=139" "$out"
absent "a Pass board row logging a host refusal is not counted" "Pass host refusal" "$out"
run "host against host" 0 "1 -> 1  0  refused, vertex shader: Valhall: a loop, in a vertex shader" --base "$M" --new "$M" --cap 'a loop, in=1'

# empty measurements
run "empty both sets" 0 "# no refused case of status Fail in either set" --base "$E" --new "$E"
run "empty base" 0 "0 -> 8  +8  refused, vertex shader: $LOOP" --base "$E" --new "$N"
run "empty new" 0 "5 -> 0  -5  refused, vertex shader: $LOOP" --base "$B" --new "$E"

# --help and bad arguments
run "--help exits 0 with the header" 0 "refusal-compare.sh - the per-refusal" --help
run "no arguments" 2 "no --base slice"
run "--base only" 2 "no --new slice" --base "$B"
run "--new only" 2 "no --base slice" --new "$N"
run "unknown option" 2 "unknown option '--bogus'" --bogus --base "$B" --new "$N"
run "a slice outside --base/--new" 2 "names a slice outside" "$B"
run "a directory that is not a slice" 1 "neither a slice" --base "$tmp" --new "$N"
mkdir -p "$tmp/norefusals"
printf 'case\tstatus\tdetail\n' > "$tmp/norefusals/results.tsv"
run "a slice without refusals.tsv" 1 "has no refusals.tsv" --base "$tmp/norefusals" --new "$N"
run "a missing directory" 1 "neither a slice" --base "$tmp/nonexistent" --new "$N"
run "--cap without N" 2 "N after the last =" --base "$B" --new "$N" --cap 'a loop'
run "--cap with an empty reason" 2 "--cap needs 'REASON=N'" --base "$B" --new "$N" --cap '=3'
run "--cap with a non-numeric N" 2 "N is not a number" --base "$B" --new "$N" --cap 'a loop=x'
run "--cap with an empty N" 2 "N is not a number" --base "$B" --new "$N" --cap 'a loop='
run "--cap matching no cause" 2 "matches no cause" --base "$B" --new "$N" --cap 'no such cause=3'
run "--fewer matching no cause" 2 "matches no cause" --base "$B" --new "$N" --fewer 'no such cause'
run "--fewer with an empty reason" 2 "--fewer needs 'REASON'" --base "$B" --new "$N" --fewer ''
run "--cap with a missing value" 2 "--cap needs 'REASON=N'" --base "$B" --new "$N" --cap
run "--fewer with a missing value" 2 "--fewer needs 'REASON'" --base "$B" --new "$N" --fewer
run "--status with a missing value" 2 "--status needs" --base "$B" --new "$N" --status
run "--status with an empty value" 2 "--status needs" --base "$B" --new "$N" --status ''

# the usage text on a bad call shows the Usage line
out=$("$RC" 2>&1)
present "usage text shows the Usage line" "refusal-compare.sh --base" "$out"

total=$((passed + failed))
if [ "$failed" -ne 0 ] || [ "$total" -ne "$EXPECT" ]; then
  echo "refusal-compare-selftest: $failed of $total checks failed (the self-test pins $EXPECT checks)" >&2
  exit 1
fi
echo "refusal-compare-selftest: $total checks ok"
