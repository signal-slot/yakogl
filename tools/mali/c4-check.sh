#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# c4-check.sh - plan #402's criterion c4 as one command. HOST-ONLY at the
# tip, the refusals of the whole-tree run at 8ff01055 (the four slices
# tools/cts/log/2026-11-24-t6-gles2-S1..S4-8ff01055) are re-measured with
# tools/mali/deqp-host-measure.sh; this script compares the measure's
# measure.tsv with those slices through tools/mali/refusal-compare.sh (status
# Fail) and prints one line per refusal:
#   <name> base=<baseline> now=<now> limit=<limit> ok|FAIL
# and exits 1 on any FAIL (2 when the check could not run).
#
# Caps (cases still first-refused by the refusal must be at most the limit,
# plan #377 c7's 37305608 counts):
#   vsunif    vertex shader 'a uniform other than a float, vec2, vec3, vec4
#             or mat4'                                              <= 362
#   vsloop    vertex shader 'a loop in a vertex shader'             <= 105
#   vsconst   vertex shader 'Valhall: a constant operation on or to a value
#             that is not a finite normal float'                    <= 114
# Strict decrease (the count must be below its 8ff01055 count):
#   fsunif    fragment shader 'a mediump fragment shader uniform ...'
#   vsuniarr  vertex shader 'a uniform array'
#   fsint     fragment shader 'an integer or boolean value'
#   mixprec   fragment shader 'a fragment shader that mixes highp and mediump
#             or lowp float values'
#   varyload  fragment shader 'a fragment varying load or conversion'
#   irindex   fragment shader 'back end defect: an invariant of the pipeline
#             did not hold (IrIndex'
# A refusal is matched as a substring of the cause text; host texts are cut at
# ' is not lowered', board texts are not, so the substrings stop before it.
#
# Usage:
#   tools/mali/c4-check.sh [MEASURE_DIR]
#       [--base SLICE_DIR]...   baseline slices (default: the four 8ff01055 ones)
#       [--cts DIR] [--slice DIR]... [--out DIR] [--jobs N]
#                               used only without MEASURE_DIR: passed to
#                               deqp-host-measure.sh, which is run first
#                               (--slice defaults to the baseline slices)
# MEASURE_DIR is the --out directory of deqp-host-measure.sh (its measure.tsv).
set -uo pipefail
me=c4-check
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
COMPARE=${C4_COMPARE:-$HERE/refusal-compare.sh}
MEASURE=${C4_MEASURE:-$HERE/deqp-host-measure.sh}

measure_dir=
base=()
mopts=()
slices=()
out=
while [ $# -gt 0 ]; do
	case $1 in
	-h | --help) sed -n '5,40p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
	--base) base+=("${2:?--base needs a directory}"); shift 2 ;;
	--slice) slices+=("${2:?--slice needs a directory}"); shift 2 ;;
	--cts | --jobs) mopts+=("$1" "${2:?$1 needs a value}"); shift 2 ;;
	--out) out=${2:?--out needs a directory}; shift 2 ;;
	-*) echo "$me: unknown option '$1'" >&2; exit 2 ;;
	*)
		[ -z "$measure_dir" ] || { echo "$me: two measurement directories" >&2; exit 2; }
		measure_dir=$1
		shift ;;
	esac
done
if [ "${#base[@]}" -eq 0 ]; then
	for s in 1 2 3 4; do base+=("$REPO/tools/cts/log/2026-11-24-t6-gles2-S$s-8ff01055"); done
fi
if [ -z "$measure_dir" ]; then
	[ "${#slices[@]}" -gt 0 ] || slices=("${base[@]}")
	measure_dir=${out:-$(mktemp -d "${TMPDIR:-$REPO/build}/c4-check-XXXXXX")}
	sa=()
	for s in "${slices[@]}"; do sa+=(--slice "$s"); done
	bash "$MEASURE" --out "$measure_dir" "${sa[@]}" "${mopts[@]}" >&2 ||
		{ echo "$me: deqp-host-measure.sh failed" >&2; exit 2; }
fi
[ -f "$measure_dir/measure.tsv" ] || { echo "$me: $measure_dir has no measure.tsv" >&2; exit 2; }

VS="refused, vertex shader: Valhall"
FS="refused, fragment shader: Valhall"
# name|kind|limit|reason; caps first, as refusal-compare prints its verdicts
specs=(
	"vsunif|cap|362|$VS: a uniform other than a float, vec2, vec3, vec4 or mat4"
	"vsloop|cap|105|$VS: a loop in a vertex shader"
	"vsconst|cap|114|$VS: a constant operation on or to a value that is not a finite normal float"
	"fsunif|fewer||$FS: a mediump fragment shader uniform"
	"vsuniarr|fewer||$VS: a uniform array"
	"fsint|fewer||$FS: an integer or boolean value"
	"mixprec|fewer||$FS: a fragment shader that mixes highp and mediump or lowp float values"
	"varyload|fewer||$FS: a fragment varying load or conversion"
	"irindex|fewer||$FS back end defect: an invariant of the pipeline did not hold (IrIndex"
)
args=(--base "${base[@]}" --new "$measure_dir")
for sp in "${specs[@]}"; do
	IFS='|' read -r name kind limit reason <<<"$sp"
	if [ "$kind" = cap ]; then args+=(--cap "$reason=$limit"); else args+=(--fewer "$reason"); fi
done
res=$(bash "$COMPARE" "${args[@]}" 2>&1)
rc=$?
if [ "$rc" -gt 1 ]; then
	printf '%s\n' "$res" >&2
	echo "$me: refusal-compare.sh could not compare (exit $rc)" >&2
	exit 2
fi
mapfile -t verdicts < <(printf '%s\n' "$res" | grep -E '^(PASS|FAIL) (cap|fewer) ')
if [ "${#verdicts[@]}" -ne "${#specs[@]}" ]; then
	printf '%s\n' "$res" >&2
	echo "$me: ${#verdicts[@]} verdicts for ${#specs[@]} refusals" >&2
	exit 2
fi
fail=0
for i in "${!specs[@]}"; do
	IFS='|' read -r name kind limit _ <<<"${specs[$i]}"
	v=${verdicts[$i]}
	b=$(sed -E 's/.*: base ([0-9]+), new ([0-9]+).*/\1/' <<<"$v")
	n=$(sed -E 's/.*: base ([0-9]+), new ([0-9]+).*/\2/' <<<"$v")
	if [ "$kind" = cap ]; then lim="<=$limit"; else lim="<$b"; fi
	if [ "${v%% *}" = PASS ]; then ok=ok; else ok=FAIL; fail=1; fi
	printf '%-9s base=%s now=%s limit=%s %s\n' "$name" "$b" "$n" "$lim" "$ok"
done
exit $fail
