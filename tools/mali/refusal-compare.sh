#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# refusal-compare.sh - the per-refusal case counts of two dEQP-GLES2
# measurements (a baseline set of slice directories and a candidate set), and
# PASS/FAIL verdicts on caps. A directory of a set is one of two measurement
# forms: a board slice, tools/cts/log/<date>-t6-gles2-<session>-<sha8>/
# (tools/cts/mali-deqp-run.sh), holding results.tsv (case status detail) and
# refusals.tsv (case refusal) as tools/cts/mali-deqp-tally.py reads them; or
# the OUT directory of tools/mali/deqp-host-measure.sh (HOST-ONLY), read from
# its measure.tsv: per case its board status, the refusing stage and the host
# refusal text. The two forms have different cause labels (a host text is cut
# at " is not lowered", a board text is not): compare board forms with board
# forms (those match the docs/es2-cts-failures.md tables byte for byte) and
# host forms with host forms.
#
# Counting (the same joining tools/cts/mali-deqp-tally.py --tally does):
# a refused case's cause is its refusals.tsv text:
#   "... refused this <stage> shader: Mali(<why>)" -> "refused, <stage> shader: <why>"
#   (the "Mali(...)" taken off, a text cut short kept as it is: a cut at a
#   different length is a different cause row, as in the tally), a text
#   without "refused this <stage> shader:" -> "refused: <whole text>";
#   a "link error: ... refused this <stage> shader: ..." reads as that stage,
#   as the tally reads it. measure.tsv gives the same cause from its stage and
#   host refusal columns (its "crash" and "not run on the host" rows are not
#   refusals and are skipped), counted under the case's board status column.
#   A cause's count is the cases of it whose
#   results.tsv status is one of --status (default Fail: the docs/es2-cts-
#   failures.md tables count Fail and NotSupported apart; Pass cases that log
#   a refusal are never counted). One directory is one measurement: a case in
#   two directories of a set is counted once, the later directory's status
#   and refusal winning (list a rerun slice after what it reruns; this tool
#   does not implement mali-deqp-tally.py's reruns= and replaces= keys).
#   A case with no results.tsv row is not counted.
#
# Output: one line per cause that has a count in either set, most grown
# first (tie: higher new count, then the cause text):
#   <base> -> <new>  <delta>  <cause>
# with a leading comment line naming the sets and a trailing "# total" line.
# --cap 'REASON=N' prints "PASS|FAIL cap ..." : PASS when the cases of the
# causes matching REASON total at most N in the new set. --fewer 'REASON'
# prints "PASS|FAIL fewer ...": PASS when that total is strictly below the
# base total. REASON is a fixed substring of the cause text; if it matches
# several causes their counts are summed (a truncated text is a second cause
# row), and a REASON matching no cause of either set is an error (exit 2),
# not a verdict. Any FAIL verdict exits 1.
#
# Usage:
#   tools/mali/refusal-compare.sh --base <slice dir>... --new <slice dir>...
#       [--status <S>[,<S>...]] [--cap '<REASON>=<N>']... [--fewer '<REASON>']...
#
# Example (the tree at 37305608 against the whole-tree run at 8ff01055, the
# three refusals plan #402 lowers first; the caps are plan #377 c7's "no
# taken refusal exceeds its 37305608 count", so all three FAIL and the run
# exits 1):
#   tools/mali/refusal-compare.sh \
#     --base tools/cts/log/2026-11-15-t6-gles2-S1-37305608 \
#            tools/cts/log/2026-11-15-t6-gles2-S2-37305608 \
#            tools/cts/log/2026-11-15-t6-gles2-S3-37305608 \
#            tools/cts/log/2026-11-15-t6-gles2-S4-37305608 \
#     --new  tools/cts/log/2026-11-24-t6-gles2-S1-8ff01055 \
#            tools/cts/log/2026-11-24-t6-gles2-S2-8ff01055 \
#            tools/cts/log/2026-11-24-t6-gles2-S3-8ff01055 \
#            tools/cts/log/2026-11-24-t6-gles2-S4-8ff01055 \
#     --cap  'a uniform other than a float, vec2, vec3, vec4 or mat4=362' \
#     --cap  'a loop in a vertex shader=105' \
#     --cap  'vertex shader: Valhall: a constant operation on or to a value that is not a finite normal float=114'
# whose three headline lines are:
#   105 -> 176  +71  refused, vertex shader: Valhall: a loop in a vertex shader is not lowered yet
#   362 -> 371  +9  refused, vertex shader: Valhall: a uniform other than a float, vec2, vec3, vec4 or mat4 is not lowered yet
#   114 -> 144  +30  refused, vertex shader: Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet
# ('a constant operation on or to a value that is not a finite normal float'
# is a cause of both stages, fragment 489 -> 33 and vertex 114 -> 144 at Fail;
# a REASON naming one stage says so, and a REASON matching several causes
# sums them).
set -euo pipefail

me=refusal-compare
TAB=$'\t'
status=Fail
base=()
new=()
caps=()
fewer=()
mode=

usage() {
	sed -n '49,51p' "$0" | sed 's/^# \{0,1\}//' >&2
	exit 2
}

while [ $# -gt 0 ]; do
	case $1 in
	--base) mode=base; shift ;;
	--new) mode=new; shift ;;
	--status) [ -n "${2:-}" ] || { echo "$me: --status needs Fail[,NotSupported,...]" >&2; usage; }
		status=$2; shift 2 ;;
	--cap) [ -n "${2:-}" ] || { echo "$me: --cap needs 'REASON=N'" >&2; usage; }
		caps+=("$2"); shift 2 ;;
	--fewer) [ -n "${2:-}" ] || { echo "$me: --fewer needs 'REASON'" >&2; usage; }
		fewer+=("$2"); shift 2 ;;
	-h | --help) sed -n '5,76p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
	-*) echo "$me: unknown option '$1'" >&2; usage ;;
	*)
		case $mode in
		base) base+=("$1") ;;
		new) new+=("$1") ;;
		*) echo "$me: '$1' names a slice outside --base/--new" >&2; usage ;;
		esac
		shift ;;
	esac
done
[ "${#base[@]}" -gt 0 ] || { echo "$me: no --base slice" >&2; usage; }
[ "${#new[@]}" -gt 0 ] || { echo "$me: no --new slice" >&2; usage; }
base=("${base[@]%/}")
new=("${new[@]%/}")
# Checked here, before any process substitution: an exit inside one would
# stop only its subshell and the comparison would run on an empty set.
for d in "${base[@]}" "${new[@]}"; do
	if [ -f "$d/measure.tsv" ]; then continue; fi
	[ -f "$d/results.tsv" ] || { echo "$me: $d is neither a slice (no results.tsv) nor a host measurement (no measure.tsv)" >&2; exit 1; }
	[ -f "$d/refusals.tsv" ] || { echo "$me: $d has no refusals.tsv (not a slice)" >&2; exit 1; }
done

# measure <dir>...: one TSV row per cause: "<cause>\t<count>", joining each
# directory's refusal text onto its status, as in the header: a slice's two
# files, or a host measurement's measure.tsv.
measure() {
	local d files=()
	for d in "$@"; do
		if [ -f "$d/measure.tsv" ]; then
			files+=("$d/measure.tsv")
		else
			files+=("$d/results.tsv" "$d/refusals.tsv")
		fi
	done
	awk -F'\t' -v stat="$status" '
		BEGIN { n = split(stat, sel, ","); for (i = 1; i <= n; i++) want[sel[i]] = 1 }
		FNR == 1 { next }
		FILENAME ~ /\/results\.tsv$/ { st[$1] = $2; next }
		FILENAME ~ /\/measure\.tsv$/ {
			st[$1] = $2 # board status column
			if ($6 != "") { # host_family set: the host refused something
				if ($5 == "vs") cause = "refused, vertex shader: " $8
				else if ($5 == "fs") cause = "refused, fragment shader: " $8
				else cause = "refused: " $8
				ref[$1] = cause
			}
			next
		}
		{
			text = substr($0, index($0, "\t") + 1)
			gsub(/\t/, " ", text) # a text with a tab would break the TSV rows
			if (match(text, /refused this [A-Za-z0-9_]+ shader: /)) {
				stage = substr(text, RSTART + 13, RLENGTH - 22)
				why = substr(text, RSTART + RLENGTH)
				if (substr(why, 1, 5) == "Mali(") {
					why = substr(why, 6)
					if (substr(why, length(why), 1) == ")") why = substr(why, 1, length(why) - 1)
				}
				cause = "refused, " stage " shader: " why
			} else {
				cause = "refused: " text
			}
			ref[$1] = cause
		}
		END {
			for (c in ref) {
				if (c in st && st[c] in want) cnt[ref[c]]++
			}
			for (k in cnt) printf "%s\t%d\n", k, cnt[k]
			# the sentinel row: a measure that found nothing still emits one
			# line, so the join below always sees two non-empty inputs (an
			# empty first stream would make NR == FNR read the second as the
			# first and swap base and new)
			print "#\t0"
		}
	' "${files[@]}"
}

# rows: one row per cause the two sets share or one of them has alone,
# "<new - base>\t<new>\t<base>\t<cause>", most grown first.
rows=$(
	awk -F'\t' '
		$1 == "#" { next }
		NR == FNR { b[$1] = $2; seen[$1] = 1; next }
		{ n[$1] = $2; seen[$1] = 1 }
		END {
			for (c in seen) {
				bb = (c in b) ? b[c] + 0 : 0
				nn = (c in n) ? n[c] + 0 : 0
				printf "%d\t%d\t%d\t%s\n", nn - bb, nn, bb, c
			}
		}
	' <(measure "${base[@]}") <(measure "${new[@]}") |
		LC_ALL=C sort -t "$TAB" -k1,1nr -k2,2nr -k4,4
)

echo "# $me: status $status; base ${#base[@]} slice(s), new ${#new[@]} slice(s); lines: base -> new delta cause"
if [ -n "$rows" ]; then
	printf '%s\n' "$rows" | awk -F'\t' '
		{
			d = $1 + 0
			printf "%d -> %d  %s  %s\n", $3, $2, (d > 0 ? "+" d : d ""), $4
		}
	'
	printf '%s\n' "$rows" | awk -F'\t' -v st="$status" '
		{ b += $3; n += $2 }
		END { printf "# total refused of status %s: %d -> %d\n", st, b, n }
	'
else
	echo "# no refused case of status $status in either set"
fi

fail=0
verdict() {
	local kind=$1 spec=$2 reason="" n=0 m=0 s=0 b=0 v
	if [ "$kind" = cap ]; then
		n=${spec##*=}
		reason=${spec%"=$n"}
		[ "$reason" != "$spec" ] && [ -n "$reason" ] ||
			{ echo "$me: --cap needs 'REASON=N' (N after the last =): '$spec'" >&2; exit 2; }
		case $n in '' | *[!0-9]*) echo "$me: --cap '$spec': N is not a number" >&2; exit 2 ;; esac
	else
		reason=$spec
		[ -n "$reason" ] || { echo "$me: --fewer needs 'REASON'" >&2; exit 2; }
	fi
	read -r m s b <<<"$(printf '%s\n' "$rows" |
		awk -F'\t' -v r="$reason" 'index($4, r) { m++; s += $2; b += $3 } END { print m + 0, s + 0, b + 0 }')"
	[ "$m" -gt 0 ] || { echo "$me: '$reason' matches no cause of the two sets" >&2; exit 2; }
	if { [ "$kind" = cap ] && [ "$s" -le "$n" ]; } || { [ "$kind" = fewer ] && [ "$s" -lt "$b" ]; }; then
		v=PASS
	else
		v=FAIL
		fail=1
	fi
	if [ "$kind" = cap ]; then
		echo "$v cap '$reason' <= $n: base $b, new $s ($m cause(s))"
	else
		echo "$v fewer '$reason': base $b, new $s ($m cause(s))"
	fi
}
for spec in "${caps[@]}"; do verdict cap "$spec"; done
for spec in "${fewer[@]}"; do verdict fewer "$spec"; done
exit $fail
