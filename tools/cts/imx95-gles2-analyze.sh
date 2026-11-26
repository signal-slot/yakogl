#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# imx95-gles2-analyze.sh - the analysis chain of plan #407 over the Verdin
# iMX95 dEQP-GLES2 sessions.
#
#   tools/cts/imx95-gles2-analyze.sh [options] OUTDIR SESSION...
#
#   SESSION  a tools/cts/log/<date>-imx95-gles2-<S1a|S1b|S2|S3a|S3b|S4>-<sha8>/
#            folder: console/<label>.log, rows.tsv, bbm/ (as the board sessions
#            leave them). It is only read: the chain works on a copy of it
#            in OUTDIR/slices/, so neither the sessions nor any doc of the
#            repository is written.
#   OUTDIR   the output folder (made if absent; must not hold slices/ yet)
#
#   --mustpass FILE  the case list (default tools/cts/gles2-mustpass.txt)
#   --tree FILE      the '# image' tree (default tools/cts/mali-gles2-tree.txt)
#   --t6 SLICE       a T6 slice for the diff; repeatable (default: the
#                    baseline tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055)
#   --commit SHA     the build commit for imx95-uart-slice.py (default: its
#                    own: the directory name's sha8 or the console's build)
#   --expect TAG=FIRST-LAST  the rows session TAG (S1a ...) must hold, as
#                    imx95-rows-tsv.py --expect (default: its span of TAG)
#   --whole          the sessions together hold every '# image' row exactly
#                    once (default when the six sessions are given with
#                    the default tree)
#   --no-git, --main-ref REF  passed to mali-deqp-tally.py --check-cover
#
# Paths must not hold white space. The stages, each with its own output:
#
#   1 rows      imx95-rows-tsv.py --check on every session (and --whole)
#   2 slice     imx95-uart-slice.py, mali-slice-readme.py --write and
#               mali-slice-redact.py on the copies         -> slices/*
#   3 tally     mali-deqp-tally.py --check-cover, --tally and
#               --tally --per-case --markdown              -> tally.txt,
#               tally.md
#   4 triage    mali-fail-triage.py                        -> triage.md (the
#               per-cause table and the draft issues)
#   5 family    mali-family-tree.py                        -> family.txt
#   6 diff      imx95-t6-diff.py against the T6 slices     -> diff.txt, diff.md
#   7 cause     imx95-g310-cause.py --summary over the cases both sides hold
#               (case, T6 result, iMX95 result)            -> cause.tsv,
#               cause-summary.txt, cause-differences.tsv (without
#               no-difference). Lost is passed as Missing and ResourceError
#               as InternalError, the cause tool's own results.
#
# A stage that fails is named on stderr as `stage <n> <name>: FAIL ...` and the
# chain stops there (a missing row stops it at stage 1, before anything is
# converted); the files of earlier stages stay. A stage that passes prints
# `stage <n> <name>: ok`. Exit 0: every stage passed, 1: a stage failed,
# 2: a usage error. Only the tools' existing command lines are used.
set -u

here=$(cd "$(dirname "$0")" && pwd)
mustpass=$here/gles2-mustpass.txt
tree=
tree_default=$here/mali-gles2-tree.txt
t6=
commit=
expects=
whole=
cover_opts=

usage() { sed -n '5,7p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }

while [ $# -gt 0 ]; do
	case $1 in
	--mustpass) [ $# -ge 2 ] || usage; mustpass=$2; shift 2 ;;
	--tree) [ $# -ge 2 ] || usage; tree=$2; shift 2 ;;
	--t6) [ $# -ge 2 ] || usage; t6="$t6 $2"; shift 2 ;;
	--commit) [ $# -ge 2 ] || usage; commit=$2; shift 2 ;;
	--expect) [ $# -ge 2 ] || usage; expects="$expects $2"; shift 2 ;;
	--whole) whole=1; shift ;;
	--no-git) cover_opts="$cover_opts --no-git"; shift ;;
	--main-ref) [ $# -ge 2 ] || usage; cover_opts="$cover_opts --main-ref $2"; shift 2 ;;
	--) shift; break ;;
	-*) echo "imx95-gles2-analyze: unknown option $1" >&2; usage ;;
	*) break ;;
	esac
done
[ $# -ge 2 ] || usage
out=$1
shift

if [ -z "$tree" ]; then
	tree=$tree_default
	# the six sessions of the plan, with the default tree: --whole
	if [ "$#" -eq 6 ] && [ -z "$expects" ]; then whole=1; fi
fi
if [ -z "$t6" ]; then
	t6=$(ls -d "$here"/log/2026-11-24-t6-gles2-S*-8ff01055 2>/dev/null | tr '\n' ' ')
fi

for d in "$@"; do
	[ -d "$d/console" ] || { echo "imx95-gles2-analyze: $d: no console/ folder" >&2; exit 2; }
	[ -f "$d/rows.tsv" ] || { echo "imx95-gles2-analyze: $d: no rows.tsv" >&2; exit 2; }
done
for f in "$mustpass" "$tree"; do
	[ -f "$f" ] || { echo "imx95-gles2-analyze: $f: no such file" >&2; exit 2; }
done
[ -n "$t6" ] || { echo "imx95-gles2-analyze: no T6 slice (--t6)" >&2; exit 2; }
case $out in /*) ;; *) out=$(pwd)/$out ;; esac
case $mustpass in /*) ;; *) mustpass=$(pwd)/$mustpass ;; esac
[ ! -e "$out/slices" ] || { echo "imx95-gles2-analyze: $out/slices exists" >&2; exit 2; }
mkdir -p "$out/slices" || exit 2

py() { python3 -I "$@"; }
fail() { # n name text
	echo "stage $1 $2: FAIL $3" >&2
	exit 1
}
ok() { echo "stage $1 $2: ok${3:+ ($3)}"; }

# 1 rows -------------------------------------------------------------------
for d in "$@"; do
	tag=$(basename "$d" | sed -n 's/.*-imx95-gles2-\(S[0-9][ab]\{0,1\}\)-[0-9a-f]\{8\}$/\1/p')
	span=
	for e in $expects; do
		[ "${e%%=*}" = "$tag" ] && span=${e#*=}
	done
	if [ -n "$span" ]; then
		py "$here/imx95-rows-tsv.py" --tree "$tree" --expect "$span" --check "$d" \
			>"$out/rows-check.txt" 2>&1 || { cat "$out/rows-check.txt" >&2; fail 1 rows "$d: imx95-rows-tsv.py --check refuses it"; }
	else
		py "$here/imx95-rows-tsv.py" --tree "$tree" --check "$d" \
			>"$out/rows-check.txt" 2>&1 || { cat "$out/rows-check.txt" >&2; fail 1 rows "$d: imx95-rows-tsv.py --check refuses it"; }
	fi
done
if [ -n "$whole" ]; then
	py "$here/imx95-rows-tsv.py" --tree "$tree" --whole --check "$@" \
		>"$out/rows-check.txt" 2>&1 || { cat "$out/rows-check.txt" >&2; fail 1 rows "--whole: the sessions do not hold every row once"; }
fi
ok 1 rows "$# session(s)"

# 2 slice ------------------------------------------------------------------
slices=
for d in "$@"; do
	c=$out/slices/$(basename "$d")
	# meta.txt records the tree as given: the repository-relative name for the
	# default tree, run from the repository root, never an absolute path
	root=$(cd "$here/../.." && pwd)
	treearg=${tree#"$root"/}
	cdto=.
	[ "$treearg" != "$tree" ] && cdto=$root
	cp -R "$d" "$c" || fail 2 slice "cannot copy $d"
	if [ -n "$commit" ]; then
		(cd "$cdto" && py "$here/imx95-uart-slice.py" --tree "$treearg" --mustpass "$mustpass" --commit "$commit" "$c") \
			>"$c.slice.out" 2>&1
	else
		(cd "$cdto" && py "$here/imx95-uart-slice.py" --tree "$treearg" --mustpass "$mustpass" "$c") >"$c.slice.out" 2>&1
	fi || { cat "$c.slice.out" >&2; fail 2 slice "$d: imx95-uart-slice.py refuses it"; }
	py "$here/mali-slice-readme.py" --write "$c" >>"$c.slice.out" 2>&1 ||
		{ cat "$c.slice.out" >&2; fail 2 slice "$d: mali-slice-readme.py --write"; }
	py "$here/mali-slice-redact.py" "$c" >>"$c.slice.out" 2>&1 ||
		{ cat "$c.slice.out" >&2; fail 2 slice "$d: mali-slice-redact.py reports a hit (no text shown)"; }
	slices="$slices $c"
done
ok 2 slice

# 3 tally ------------------------------------------------------------------
# shellcheck disable=SC2086
py "$here/mali-deqp-tally.py" --check-cover $cover_opts "$mustpass" $slices >"$out/cover.txt" 2>&1 ||
	{ cat "$out/cover.txt" >&2; fail 3 tally "--check-cover over the sessions"; }
# shellcheck disable=SC2086
py "$here/mali-deqp-tally.py" --tally --no-git $slices >"$out/tally.txt" 2>"$out/tally.err" ||
	{ cat "$out/tally.err" >&2; fail 3 tally "--tally"; }
# shellcheck disable=SC2086
py "$here/mali-deqp-tally.py" --tally --per-case --markdown --no-git $slices >"$out/tally.md" 2>"$out/tally.err" ||
	{ cat "$out/tally.err" >&2; fail 3 tally "--tally --per-case --markdown"; }
ok 3 tally "$(sed -n 's/^TOTAL *\([0-9]*\).*/\1 cases/p' "$out/tally.txt" | sed -n 1p)"

# 4 triage, 5 family -------------------------------------------------------
# shellcheck disable=SC2086
py "$here/mali-fail-triage.py" $slices >"$out/triage.md" 2>"$out/triage.err" ||
	{ cat "$out/triage.err" >&2; fail 4 triage "mali-fail-triage.py"; }
ok 4 triage
# shellcheck disable=SC2086
py "$here/mali-family-tree.py" --mustpass "$mustpass" $slices >"$out/family.txt" 2>"$out/family.err" ||
	{ cat "$out/family.err" >&2; fail 5 family "mali-family-tree.py"; }
ok 5 family

# 6 diff -------------------------------------------------------------------
imx_args=
for s in $slices; do imx_args="$imx_args --imx95 $s"; done
t6_args=
for s in $t6; do t6_args="$t6_args --t6 $s"; done
# shellcheck disable=SC2086
py "$here/imx95-t6-diff.py" $imx_args $t6_args >"$out/diff.txt" 2>"$out/diff.err" ||
	{ cat "$out/diff.err" >&2; fail 6 diff "imx95-t6-diff.py"; }
# shellcheck disable=SC2086
py "$here/imx95-t6-diff.py" --markdown $imx_args $t6_args >"$out/diff.md" 2>"$out/diff.err" ||
	{ cat "$out/diff.err" >&2; fail 6 diff "imx95-t6-diff.py --markdown"; }
ok 6 diff

# 7 cause ------------------------------------------------------------------
# Join the results.tsv of the T6 slices and of the iMX95 slices on the case:
# a result beats Lost, otherwise the later one wins. Only cases both sides hold.
: >"$out/t6.results.tsv"
for s in $t6; do
	[ -f "$s/results.tsv" ] && cat "$s/results.tsv" >>"$out/t6.results.tsv"
done
: >"$out/imx95.results.tsv"
for s in $slices; do cat "$s/results.tsv" >>"$out/imx95.results.tsv"; done
{
	printf 'case\tt6_result\timx95_result\n'
	awk -F'\t' '
		function n(s) {
			if (s == "Lost") return "Missing"
			if (s == "ResourceError") return "InternalError"
			return s
		}
		$1 == "case" { next }
		NR == FNR { r = n($2); if (!($1 in t) || t[$1] == "Missing" || r != "Missing") t[$1] = r; next }
		{ r = n($2); if (!($1 in m)) order[++k] = $1
		  if (!($1 in m) || m[$1] == "Missing" || r != "Missing") m[$1] = r }
		END { for (i = 1; i <= k; i++) { c = order[i]; if (c in t) printf "%s\t%s\t%s\n", c, t[c], m[c] } }
	' "$out/t6.results.tsv" "$out/imx95.results.tsv"
} >"$out/cause-input.tsv" 2>"$out/cause.err" || { cat "$out/cause.err" >&2; fail 7 cause "joining the results"; }
rm -f "$out/t6.results.tsv" "$out/imx95.results.tsv"
py "$here/imx95-g310-cause.py" --summary "$out/cause-input.tsv" >"$out/cause.tsv" 2>"$out/cause.err" ||
	{ cat "$out/cause.err" >&2; fail 7 cause "imx95-g310-cause.py"; }
grep '^#' "$out/cause.tsv" >"$out/cause-summary.txt"
grep -v '^#' "$out/cause.tsv" | awk -F'\t' 'NR == 1 || $4 != "no-difference"' >"$out/cause-differences.tsv"
ok 7 cause "$(( $(grep -c . "$out/cause-differences.tsv") - 1 )) difference(s)"
exit 0
