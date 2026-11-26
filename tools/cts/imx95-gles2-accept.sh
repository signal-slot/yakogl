#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# imx95-gles2-accept.sh - walk plan #407's acceptance checks c1, c2 and c5
# (the Verdin iMX95 dEQP-GLES2 tree) over a repository.
#
#   tools/cts/imx95-gles2-accept.sh [--root DIR] [--base REV] [--main-ref REF]
#                                   [MUSTPASS]
#
#   --root DIR      the repository to check (default .)
#   --base REV      the commit the images were built from (default e63ab28e);
#                   its first 8 hex digits end the session directories' names
#   --main-ref REF  passed to mali-deqp-tally.py --check-cover: the ref the
#                   slices' commits must be ancestors of (default origin/main)
#   MUSTPASS        the mustpass list, relative to DIR or absolute (default
#                   tools/cts/gles2-mustpass.txt, the list the T6 tree's
#                   --check-cover takes)
#
# It prints one line per criterion, `c1: PASS <what held>` or
# `c1: FAIL <why>`, on stdout (the tools' own output goes to stderr), and
# exits 0 when all three pass, 1 when any fails, 2 on a usage error.
#
# c1  tools/cts/log/*-imx95-gles2-S*-<sha8>/ is six session directories, S4,
#     S1a, S1b, S2, S3a and S3b, each once and no other; each is a slice
#     (meta.txt, chunks.tsv, results.tsv) and
#     `mali-deqp-tally.py --check-cover MUSTPASS <the six>` exits 0; each
#     rows.tsv has one line per '# image' row of its session (S4 R01-R08,
#     S1a R01-R12, S1b R13-R24, S2 R01-R14, S3a R01-R15, S3b R16-R30: 76),
#     with an integer seconds and lifetime_s and a bbm value. When
#     imx95-rows-tsv.py is beside this script, its --check runs too.
# c2  docs/es2-cts-failures.md has a part whose heading names
#     `Verdin iMX95`, and that part's totals equal the tally of the slices.
#     With a mali-deqp-tally block in the part: the part alone, as a
#     document, passes `mali-deqp-tally.py --check-doc PART <the six>` (or
#     the block equals `--tally --per-case --markdown`). Without a block:
#     the part's `| <Status> | <N> |` rows equal `--tally`'s counts.
#     mali-deqp-tally.py has no option that names a part: --check-doc reads
#     the first block of the file, the T6's. So --check-doc on the whole
#     document (the gate's form, with --log) cannot check the iMX95 block;
#     this script never passes c2 on that run.
# c5  `tools/redact-diff-scan.py <base>..HEAD` reports nothing, and
#     `git diff <base>..HEAD` changes no file under third_party/,
#     driver/src/*/generated/, no driver/src/fw/*blob*, and none of
#     tools/board.sh, tools/board_selftest.sh, tools/cts/RESULTS.md,
#     tools/cts/STATUS.md, README.md and ROADMAP.md. That diff also holds
#     what --main-ref changed since the base; the FAIL line says how many of
#     the paths it names differ between the base and --main-ref as well.
#
# Only the tools' existing command lines are used; the tools are the ones
# beside this script, the checked tree is DIR.
# tools/cts/imx95-gles2-accept-selftest.sh shows one PASS and one FAIL of
# each criterion on a throwaway repository.
set -u

here=$(cd "$(dirname "$0")" && pwd)
TALLY="$here/mali-deqp-tally.py"
SCAN="$here/../redact-diff-scan.py"
ROWS="$here/imx95-rows-tsv.py"

root=.
base=e63ab28e
main_ref=origin/main
mustpass=

usage() {
	sed -n '5,22p' "$0" | sed 's/^# \{0,1\}//' >&2
	exit 2
}

while [ $# -gt 0 ]; do
	case $1 in
	--root) [ $# -ge 2 ] || usage; root=$2; shift 2 ;;
	--base) [ $# -ge 2 ] || usage; base=$2; shift 2 ;;
	--main-ref) [ $# -ge 2 ] || usage; main_ref=$2; shift 2 ;;
	-h | --help) usage ;;
	-*) echo "imx95-gles2-accept: unknown option $1" >&2; usage ;;
	*) [ -z "$mustpass" ] || usage; mustpass=$1; shift ;;
	esac
done
[ -n "$mustpass" ] || mustpass=tools/cts/gles2-mustpass.txt

# The scratch directory, made before the cd below and named in full: TMPDIR
# may be relative to the caller's directory.
tmp=$(mktemp -d "${TMPDIR:-.}/imx95-gles2-accept.XXXXXX") && tmp=$(cd "$tmp" && pwd) || exit 2
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

cd "$root" 2>/dev/null || { echo "imx95-gles2-accept: $root: not a directory" >&2; exit 2; }
# DIR is the top of a work tree, not a directory somewhere inside one: every
# path below is relative to it (a directory under the gate's build/ is inside
# this repository's work tree and is no repository of its own).
top=$(git rev-parse --show-toplevel 2>/dev/null) && [ -n "$top" ] && [ "$(cd "$top" && pwd -P)" = "$(pwd -P)" ] ||
	{ echo "imx95-gles2-accept: $root: not the top of a git work tree" >&2; exit 2; }

rc=0
why=
add() { why="${why:+$why; }$*"; }
verdict() { # criterion, what held
	if [ -n "$why" ]; then
		echo "$1: FAIL $why"
		rc=1
	else
		echo "$1: PASS $2"
	fi
	why=
}

# The sessions and the '# image' rows each one runs (plan #407).
SESSIONS="S4 S1a S1b S2 S3a S3b"
session_rows() { # session -> "<label prefix> <first> <last>"
	case $1 in
	S4) echo "T6S4R 1 8" ;;
	S1a) echo "T6S1R 1 12" ;;
	S1b) echo "T6S1R 13 24" ;;
	S2) echo "T6S2R 1 14" ;;
	S3a) echo "T6S3R 1 15" ;;
	S3b) echo "T6S3R 16 30" ;;
	esac
}

sha8=
if full=$(git rev-parse --verify -q "$base^{commit}" 2>/dev/null); then
	sha8=$(printf %s "$full" | cut -c1-8)
fi

# The session directories, as "$@" (names hold no blank: they are matched
# against a fixed pattern below).
set --
have=
if [ -n "$sha8" ]; then
	for d in tools/cts/log/*-imx95-gles2-S*-"$sha8"/; do
		[ -d "$d" ] || continue
		d=${d%/}
		set -- "$@" "$d"
		s=${d##*-imx95-gles2-}
		have="$have ${s%-"$sha8"}"
	done
fi
slices_ok=1 # the six session directories are there, each once

# ---- c1 ------------------------------------------------------------------
if [ -z "$sha8" ]; then
	add "the base commit $base is not in this repository"
	slices_ok=0
elif [ $# -eq 0 ]; then
	add "no directory tools/cts/log/*-imx95-gles2-S*-$sha8/ (want six: $SESSIONS)"
	slices_ok=0
else
	for s in $SESSIONS; do
		n=0
		for h in $have; do [ "$h" = "$s" ] && n=$((n + 1)); done
		[ "$n" -eq 1 ] || { add "$n directories tools/cts/log/*-imx95-gles2-$s-$sha8/ (want 1)"; slices_ok=0; }
	done
	for h in $have; do
		case " $SESSIONS " in
		*" $h "*) ;;
		*) add "a session directory that is none of the six: *-imx95-gles2-$h-$sha8"; slices_ok=0 ;;
		esac
	done
fi
if [ $# -gt 0 ]; then
	if [ ! -f "$mustpass" ]; then
		add "the mustpass list $mustpass is not a file"
	else
		python3 -I "$TALLY" --check-cover --main-ref "$main_ref" "$mustpass" "$@" >"$tmp/cover.out" 2>&1
		r=$?
		cat "$tmp/cover.out" >&2
		[ "$r" -eq 0 ] ||
			add "mali-deqp-tally.py --check-cover exits $r ($(grep -c . "$tmp/cover.out") line(s), first: $(sed -n '1p' "$tmp/cover.out" | cut -c1-160))"
	fi
	for d in "$@"; do
		s=${d##*-imx95-gles2-}
		s=${s%-"$sha8"}
		want=$(session_rows "$s")
		if [ ! -f "$d/rows.tsv" ]; then
			add "$d has no rows.tsv"
			continue
		fi
		[ -n "$want" ] || continue # not one of the six: named above
		read -r pfx first last <<EOF
$want
EOF
		e=$(awk -F '\t' -v pfx="$pfx" -v first="$first" -v last="$last" '
			NR == 1 {
				for (i = 1; i <= NF; i++) col[$i] = i
				n = split("label seconds lifetime_s bbm", need, " ")
				for (i = 1; i <= n; i++)
					if (!(need[i] in col)) { printf "no %s column; ", need[i]; bad = 1 }
				if (bad) exit
				next
			}
			{
				lab = $(col["label"])
				if (seen[lab]++) printf "%s twice; ", lab
				if ($(col["seconds"]) !~ /^[0-9]+$/) printf "%s: seconds is \"%s\"; ", lab, $(col["seconds"])
				if ($(col["lifetime_s"]) !~ /^[0-9]+$/) printf "%s: lifetime_s is \"%s\"; ", lab, $(col["lifetime_s"])
				if ($(col["bbm"]) == "") printf "%s: bbm is empty; ", lab
			}
			END {
				if (bad) exit
				if (NR == 0) { printf "empty; "; exit }
				for (i = first; i <= last; i++) {
					lab = sprintf("%s%02d", pfx, i)
					if (!(lab in seen)) miss = miss " " lab
					delete seen[lab]
				}
				if (miss != "") printf "no line for%s; ", miss
				for (lab in seen) printf "a line for %s, not a row of the session; ", lab
			}' "$d/rows.tsv")
		[ -z "$e" ] || add "$d/rows.tsv: ${e%; }"
	done
	if [ -f "$ROWS" ]; then
		# The session tag of each directory name gives its span; with all
		# six there, --whole wants every '# image' row exactly once.
		whole=
		[ "$slices_ok" -eq 1 ] && whole=--whole
		if [ -f tools/cts/mali-gles2-tree.txt ]; then
			python3 -I "$ROWS" --tree tools/cts/mali-gles2-tree.txt $whole --check "$@" >"$tmp/rows.out" 2>&1
		else
			python3 -I "$ROWS" $whole --check "$@" >"$tmp/rows.out" 2>&1
		fi
		r=$?
		cat "$tmp/rows.out" >&2
		[ "$r" -eq 0 ] || add "imx95-rows-tsv.py --check exits $r"
	fi
fi
verdict c1 "six session directories of $sha8, --check-cover of $mustpass exits 0, every rows.tsv has its rows with seconds, lifetime_s and bbm"

# ---- c2 ------------------------------------------------------------------
doc=docs/es2-cts-failures.md
BEGIN='<!-- mali-deqp-tally begin imx95 -->'
END='<!-- mali-deqp-tally end imx95 -->'
how=
if [ ! -f "$doc" ]; then
	add "$doc is not a file"
else
	# The part: from the first heading naming Verdin iMX95 to the next
	# heading of its level or above.
	awk '
		/^#+ / {
			lvl = length($1)
			if (inpart && lvl <= plvl) exit
			if (!inpart && index($0, "Verdin iMX95")) { inpart = 1; plvl = lvl }
		}
		inpart { print }' "$doc" >"$tmp/part.md"
	if [ ! -s "$tmp/part.md" ]; then
		add "$doc has no heading naming Verdin iMX95"
	elif [ "$slices_ok" -ne 1 ]; then
		add "$doc has a Verdin iMX95 part, but the six session slices to tally are not all there (see c1)"
	elif ! python3 -I "$TALLY" --tally "$@" >"$tmp/tally.txt" 2>"$tmp/tally.err"; then
		cat "$tmp/tally.err" >&2
		add "mali-deqp-tally.py --tally over the six slices fails: $(sed -n '1p' "$tmp/tally.err" | cut -c1-160)"
	elif grep -qF -e "$BEGIN" "$tmp/part.md"; then
		python3 -I "$TALLY" --check-doc "$tmp/part.md" "$@" >"$tmp/doc.out" 2>&1
		r=$?
		if [ "$r" -eq 0 ]; then
			how="the part's mali-deqp-tally block passes --check-doc (run on the part alone)"
		else
			awk -v b="$BEGIN" -v e="$END" '
				index($0, b) { on = 1 }
				on { print }
				on && index($0, e) { exit }' "$tmp/part.md" >"$tmp/block.md"
			python3 -I "$TALLY" --tally --per-case --markdown "$@" >"$tmp/percase.md" 2>/dev/null
			if cmp -s "$tmp/block.md" "$tmp/percase.md"; then
				how="the part's mali-deqp-tally block equals --tally --per-case --markdown (--check-doc has no --per-case form)"
			else
				cat "$tmp/doc.out" >&2
				# The tool's own line names the scratch copy of the
				# part: it goes to stderr, not into the verdict.
				add "the Verdin iMX95 part's mali-deqp-tally block differs from --tally --markdown of the six slices (--check-doc on the part alone exits $r)"
			fi
		fi
	else
		# No block: the part's two-cell status rows against --tally.
		awk '$1 != "slices:" && $1 != "cause" && NF == 2 { print ($1 == "TOTAL" ? "Total" : $1) "\t" $2 }' \
			"$tmp/tally.txt" | sort >"$tmp/want.tsv"
		awk -F '|' '
			NF == 4 {
				a = $2; b = $3
				gsub(/[ *`,]/, "", a); gsub(/[ *`,]/, "", b)
				if (a ~ /^(Pass|Fail|NotSupported|QualityWarning|CompatibilityWarning|InternalError|ResourceError|Crash|Timeout|Lost|Total|TOTAL)$/ && b ~ /^[0-9]+$/)
					print (a == "TOTAL" ? "Total" : a) "\t" b
			}' "$tmp/part.md" | sort -u >"$tmp/got.tsv"
		if [ ! -s "$tmp/got.tsv" ]; then
			add "the Verdin iMX95 part has neither a mali-deqp-tally block nor '| <Status> | <N> |' rows to compare with --tally"
		elif cmp -s "$tmp/want.tsv" "$tmp/got.tsv"; then
			how="the part's status rows equal --tally ($(tr '\t\n' '  ' <"$tmp/want.tsv" | sed 's/ $//')); it has no mali-deqp-tally block, so --check-doc does not apply"
		else
			add "the Verdin iMX95 part's totals ($(tr '\t\n' '  ' <"$tmp/got.tsv" | sed 's/ $//')) differ from --tally ($(tr '\t\n' '  ' <"$tmp/want.tsv" | sed 's/ $//'))"
		fi
	fi
fi
verdict c2 "$how"

# ---- c5 ------------------------------------------------------------------
if [ -z "$sha8" ]; then
	add "the base commit $base is not in this repository"
else
	# The hits are the tool's stdout, one `path:line: class: masked` line
	# each; its stderr holds its own summary and is not counted.
	python3 -I "$SCAN" "$base..HEAD" >"$tmp/scan.out" 2>"$tmp/scan.err"
	r=$?
	case $r in
	0) ;;
	1)
		cat "$tmp/scan.out" "$tmp/scan.err" >&2
		add "redact-diff-scan.py $base..HEAD reports $(grep -c . "$tmp/scan.out") hit(s), first: $(sed -n '1p' "$tmp/scan.out" | cut -c1-120)"
		;;
	*)
		cat "$tmp/scan.out" "$tmp/scan.err" >&2
		add "redact-diff-scan.py $base..HEAD exits $r (the range could not be read)"
		;;
	esac
	# The paths `git diff --stat` lists, in full (--stat shortens long ones).
	if git diff --no-renames --name-only "$base..HEAD" >"$tmp/names.txt" 2>"$tmp/names.err"; then
		grep -E '^(third_party/|driver/src/[^/]+/generated/|driver/src/fw/[^/]*blob|tools/board\.sh$|tools/board_selftest\.sh$|tools/cts/RESULTS\.md$|tools/cts/STATUS\.md$|README\.md$|ROADMAP\.md$)' \
			"$tmp/names.txt" >"$tmp/forbidden.txt"
		if [ -s "$tmp/forbidden.txt" ]; then
			n=$(grep -c . "$tmp/forbidden.txt")
			# The criterion's diff holds what --main-ref itself changed
			# since the base as well: say how many of the paths that is,
			# so the reader can tell the branch's from main's. It stays
			# a FAIL: the criterion names this diff.
			also=
			if git diff --no-renames --name-only "$base..$main_ref" >"$tmp/main-names.txt" 2>/dev/null; then
				k=$(grep -cxF -f "$tmp/forbidden.txt" "$tmp/main-names.txt")
				[ "$k" -eq 0 ] || also=" ($k of them also differ between $base and $main_ref)"
			fi
			add "git diff $base..HEAD changes $n path(s) it must not: $(sed -n '1,6p' "$tmp/forbidden.txt" | tr '\n' ' ' | sed 's/ $//')$([ "$n" -gt 6 ] && echo ' ...')$also"
		fi
	else
		cat "$tmp/names.err" >&2
		add "git diff $base..HEAD fails"
	fi
fi
verdict c5 "redact-diff-scan.py $base..HEAD reports nothing and the diff ($(grep -c . "$tmp/names.txt" 2>/dev/null) path(s)) changes no excluded path"

exit $rc
