#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Acceptance checks c1-c8 of the i.MX8MP dEQP-GLES3 board campaign (issue
# #24), in one run. Read-only: it runs the record's checkers and greps the
# result documents, it writes nothing.
#
#   tools/cts/imx8mp-gles3-acceptance.sh [--date YYYY-MM-DD] [--commit [x2-]HASH8] [<repo root>]
#
# <repo root>  the repository whose record is checked (default: the one that
#              holds this script); the checkers are run from its tools/cts.
# --date       the campaign day for c3 (default: the date in the name of the
#              tools/cts/log/*-imx8mp-gles3-*-rows01-13 directory, else the
#              date of the 'Run YYYY-MM-DD, ...' line of its README.md).
# --commit     the campaign commit (8 hex or more, or x2-<hex> for the X2 slices only): c1-c3 read only the slices (Verdin <hash8> and X2 x2-<hash8> directories both; "slice <dir>: measured on ..." lines say the board)
#              tools/cts/log/*-imx8mp-gles3-<hash8>*-rows*, as the recipe's
#              `ls -d ...-<hash8>-rows*`. Without it every slice directory is
#              read, so a tree holding two campaigns (9bc82915 and 05769bf2
#              slices) fails c1 with 'different commits' until one is retired.
#
#   c1  imx8mp-gles3-merge.py --check over the slice directories
#       tools/cts/log/*-imx8mp-gles3-*-rows* exits 0 and prints
#       '133 entries with a record: OK'. A slice may be the Verdin's
#       (board.sh imx8mp lines) or the Armadillo-X2's (board.sh x2 lines,
#       kexec-dev: rows of this campaign by the owner's decision of
#       2026-11-27), and a tree may mix both where merge accepts it; a slice
#       README with board.sh x2 lines must also say 'measured on the
#       Armadillo-X2' (the owner's rule: each record says so), else c1 FAILs.
#   c2  imx8mp-gles3-merge.py --notpass over them equals
#       docs/es3/imx8mp-gles3-notpass.tsv without its '# SPDX' lines.
#   c3  imx8mp-gles3-report.py --date <day> over them exits 0, no PARTIAL.
#   c4  docs/es3-cts-failures.md has 'GC7000UL board run' and an 8-hex sha
#       on that line or the next.
#   c5  docs/es3/board-recipe-imx8mp.md no longer says 'nothing has run on
#       the board yet' and names 'rows01-13'.
#   c6  the tools/cts/log/*-imx8mp-es2-*/README.md files hold at least 29
#       lines 'board.sh imx8mp e<N> rc=' or 'board.sh x2 e<N> rc='.
#   c7  check-imx8mp-gles3-tree.py -q prints 'matched exactly once 42810'
#       and OK.
#   c8  no /dev/tty(USB|ACM), IPv4 address or login (a user name, '@',
#       a host name with a domain) in tools/cts/log/*-imx8mp-gles3-* and
#       *-imx8mp-es2-*.
#
# One line per check on stdout, 'c<N> ok|FAIL <detail>', then a summary.
# Exit 0 only when all pass, 1 otherwise, 2 on a usage error.
# Self-test: tools/cts/imx8mp-gles3-acceptance-selftest.sh.
[ -n "${BASH_VERSION:-}" ] || exec bash "$0" "$@"
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
DATE=
USAGE="usage: $0 [--date YYYY-MM-DD] [--commit [x2-]HASH8] [<repo root>]"
COMMIT=
BOARDONLY=
while [ $# -gt 0 ]; do
	case $1 in
	--date)
		[ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
		DATE=$2
		shift 2
		;;
	--commit)
		[ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
		case $2 in
		x2-*) BOARDONLY=x2; set -- "$1" "${2#x2-}" "${@:3}" ;;
		esac
		case $2 in
		[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]*) ;;
		*) echo "imx8mp-gles3-acceptance: --commit wants 8 or more hex digits: $2" >&2; exit 2 ;;
		esac
		COMMIT=${2:0:8}
		shift 2
		;;
	-h | --help)
		sed -n '4,/^\[ -n/p' "$0" | sed 's/^# \{0,1\}//; /^\[ -n/d'
		exit 0
		;;
	-*)
		echo "$USAGE" >&2
		exit 2
		;;
	*)
		ROOT=$1
		shift
		;;
	esac
done
[ -d "$ROOT" ] || { echo "imx8mp-gles3-acceptance: no such directory: $ROOT" >&2; exit 2; }
ROOT=$(cd "$ROOT" && pwd)
CTS="$ROOT/tools/cts"
LOG="$CTS/log"
FAILS=0
TOTAL=0

res() { # res <n> <ok|FAIL> <detail>
	TOTAL=$((TOTAL + 1))
	[ "$2" = ok ] || FAILS=$((FAILS + 1))
	printf 'c%s %s %s\n' "$1" "$2" "$3"
}
tool() { # tool <script> args... : python3 -I of a tool under the root
	local t=$1
	shift
	python3 -I "$CTS/$t" "$@"
}

shopt -s nullglob
# both shapes: Verdin '<sha8>' and Armadillo-X2 'x2-<sha8>' ('--commit x2-<sha8>': X2 only)
rows=()
[ "$BOARDONLY" = x2 ] || rows+=("$LOG"/*-imx8mp-gles3-"$COMMIT"*-rows*)
rows+=("$LOG"/*-imx8mp-gles3-x2-"$COMMIT"*-rows*)
es2=("$LOG"/*-imx8mp-es2-*)
gles3=("$LOG"/*-imx8mp-gles3-*)
r01=("$LOG"/*-imx8mp-gles3-"$COMMIT"*-rows01-13)
shopt -u nullglob
dirs=()
for d in "${rows[@]}"; do
	[ -d "$d" ] || continue
	case " ${dirs[*]:-} " in *" $d "*) continue ;; esac
	dirs+=("$d")
done
# which board each slice was measured on: x2- in the name, or board.sh x2 lines in the README
for d in "${dirs[@]}"; do
	b=$(basename "$d")
	case $b in
	*-imx8mp-gles3-x2-*) echo "slice $b: measured on the Armadillo-X2" ;;
	*)
		if [ -f "$d/README.md" ] && grep -q 'board\.sh x2 e[0-9]* rc=' "$d/README.md"; then
			echo "slice $b: measured on the Armadillo-X2"
		else
			echo "slice $b: measured on the Verdin"
		fi
		;;
	esac
done

# c1
# the Armadillo-X2 slices: board.sh x2 verdict lines in the README; each must say it was measured on the X2
nx2=0 unmarked=
for d in "${dirs[@]}"; do
	case $(basename "$d") in
	*-imx8mp-gles3-x2-*) ;;
	*) [ -f "$d/README.md" ] && grep -q 'board\.sh x2 e[0-9]* rc=' "$d/README.md" || continue ;;
	esac
	nx2=$((nx2 + 1))
	[ -f "$d/README.md" ] && grep -q 'measured on the Armadillo-X2' "$d/README.md" || unmarked="$unmarked $(basename "$d")"
done
if [ ${#dirs[@]} -eq 0 ]; then
	res 1 FAIL "no slice directory matches tools/cts/log/*-imx8mp-gles3-*-rows*"
elif [ -n "$unmarked" ]; then
	res 1 FAIL "X2 slice README without 'measured on the Armadillo-X2':$unmarked"
else
	out=$(tool imx8mp-gles3-merge.py --check "${dirs[@]}" 2>&1)
	rc=$?
	if [ $rc -eq 0 ] && printf '%s\n' "$out" | grep -q '133 entries with a record: OK'; then
		res 1 ok "133 entries with a record: OK (${#dirs[@]} slice dirs, $nx2 measured on the Armadillo-X2)"
	else
		res 1 FAIL "merge --check exit $rc: $(printf '%s\n' "$out" | tail -1)"
	fi
fi

# c2
want="$ROOT/docs/es3/imx8mp-gles3-notpass.tsv"
if [ ${#dirs[@]} -eq 0 ]; then
	res 2 FAIL "no slice directories to merge"
elif [ ! -f "$want" ]; then
	res 2 FAIL "docs/es3/imx8mp-gles3-notpass.tsv does not exist"
else
	got=$(tool imx8mp-gles3-merge.py --notpass "${dirs[@]}" 2>/dev/null)
	rc=$?
	if [ $rc -ne 0 ]; then
		res 2 FAIL "merge --notpass exit $rc"
	else
		n=$(diff <(printf '%s\n' "$got") <(grep -v '^# SPDX' "$want") | grep -c '^[<>]')
		if [ "$n" -eq 0 ]; then
			res 2 ok "not-pass list equals docs/es3/imx8mp-gles3-notpass.tsv"
		else
			res 2 FAIL "not-pass list differs from docs/es3/imx8mp-gles3-notpass.tsv in $n line(s)"
		fi
	fi
fi

# c3
if [ -z "$DATE" ]; then
	for d in "${r01[@]}"; do
		b=$(basename "$d")
		case $b in
		[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]-*)
			DATE=$(printf '%s' "$b" | cut -c1-10)
			break
			;;
		esac
	done
fi
# a YYYY-MM directory name: the run window line of its README.md, "Run
# YYYY-MM-DD, hh:mm-hh:mm UTC", in the board's clock
if [ -z "$DATE" ]; then
	for d in "${r01[@]}"; do
		[ -f "$d/README.md" ] || continue
		DATE=$(grep -m1 -oE '^Run [0-9]{4}-[0-9]{2}-[0-9]{2}' "$d/README.md" | cut -c5-14)
		[ -n "$DATE" ] && break
	done
fi
if [ ${#dirs[@]} -eq 0 ]; then
	res 3 FAIL "no slice directories to report"
elif [ -z "$DATE" ]; then
	res 3 FAIL "no campaign day: pass --date, name the rows01-13 directory <YYYY-MM-DD>-imx8mp-gles3-... or give its README.md a 'Run <YYYY-MM-DD>, ...' line"
else
	out=$(tool imx8mp-gles3-report.py --date "$DATE" "${dirs[@]}" 2>&1)
	rc=$?
	if [ $rc -ne 0 ]; then
		why=$(printf '%s\n' "$out" | sed -n 's/^  problem: //p' | head -1)
		res 3 FAIL "report exit $rc: ${why:-$(printf '%s\n' "$out" | tail -1)}"
	elif printf '%s\n' "$out" | grep -q PARTIAL; then
		res 3 FAIL "report says PARTIAL"
	else
		res 3 ok "report for $DATE: no PARTIAL"
	fi
fi

# c4
f="$ROOT/docs/es3-cts-failures.md"
# Only what follows the phrase counts (earlier shas on the line are other
# work), and 'the GC7000UL board run is pending' is the state before the run.
c4_tail=
[ -f "$f" ] && c4_tail=$(grep -A1 'GC7000UL board run' "$f" | sed 's/.*GC7000UL board run//')
if printf '%s\n' "$c4_tail" | grep -v '^ is pending' |
	grep -Eq '(^|[^0-9a-zA-Z])[0-9a-f]{8}([^0-9a-zA-Z]|$)'; then
	res 4 ok "docs/es3-cts-failures.md: 'GC7000UL board run' with an 8-hex sha"
elif [ -n "$c4_tail" ]; then
	res 4 FAIL "'GC7000UL board run' is pending or has no 8-hex sha on its line or the next"
else
	res 4 FAIL "docs/es3-cts-failures.md has no 'GC7000UL board run'"
fi

# c5
f="$ROOT/docs/es3/board-recipe-imx8mp.md"
if [ ! -f "$f" ]; then
	res 5 FAIL "docs/es3/board-recipe-imx8mp.md does not exist"
elif grep -q 'nothing has run on the board yet' "$f"; then
	res 5 FAIL "recipe still says 'nothing has run on the board yet'"
elif ! grep -q 'rows01-13' "$f"; then
	res 5 FAIL "recipe does not name 'rows01-13'"
else
	res 5 ok "recipe names rows01-13 and no longer says nothing has run"
fi

# c6
n=0
for d in "${es2[@]}"; do
	[ -f "$d/README.md" ] || continue
	k=$(grep -Ec 'board\.sh (imx8mp|x2) e[0-9]* rc=' "$d/README.md")
	n=$((n + k))
done
if [ "$n" -ge 29 ]; then
	res 6 ok "$n 'board.sh imx8mp|x2 e<N> rc=' lines in the es2 READMEs (>= 29)"
else
	res 6 FAIL "$n 'board.sh imx8mp|x2 e<N> rc=' lines in ${#es2[@]} es2 directories (need >= 29)"
fi

# c7
out=$(tool check-imx8mp-gles3-tree.py -q 2>&1)
rc=$?
if [ $rc -eq 0 ] && printf '%s\n' "$out" | grep -q 'matched exactly once 42810' &&
	printf '%s\n' "$out" | grep -Eq '(^|[^A-Za-z])OK([^A-Za-z]|$)'; then
	res 7 ok "matched exactly once 42810"
else
	res 7 FAIL "check-imx8mp-gles3-tree.py -q exit $rc: $(printf '%s\n' "$out" | tail -1)"
fi

# c8
if [ ${#gles3[@]} -eq 0 ] && [ ${#es2[@]} -eq 0 ]; then
	res 8 ok "no imx8mp log directory to scan"
else
	hits=$(grep -rEIn -e '/dev/tty(USB|ACM)' \
		-e '([0-9]{1,3}\.){3}[0-9]{1,3}(:[0-9]+)?' \
		-e '[A-Za-z0-9._-]+@[A-Za-z0-9-]+\.[A-Za-z]' \
		-- "${gles3[@]}" "${es2[@]}" 2>/dev/null)
	if [ -z "$hits" ]; then
		res 8 ok "no device path, address or login in the imx8mp logs"
	else
		printf '%s\n' "$hits" | head -5 | cut -c1-200 >&2
		res 8 FAIL "$(printf '%s\n' "$hits" | wc -l) line(s) match (first: $(printf '%s\n' "$hits" | head -1 | cut -d: -f1,2))"
	fi
fi

echo "summary: $((TOTAL - FAILS)) ok, $FAILS FAIL of $TOTAL"
[ $FAILS -eq 0 ]
