#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Acceptance checks c1-c8 of the i.MX8MP dEQP-GLES3 board campaign (issue
# #24), in one run. Read-only: it runs the record's checkers and greps the
# result documents, it writes nothing.
#
#   tools/cts/imx8mp-gles3-acceptance.sh [--date YYYY-MM-DD] [<repo root>]
#
# <repo root>  the repository whose record is checked (default: the one that
#              holds this script); the checkers are run from its tools/cts.
# --date       the campaign day for c3 (default: the date in the name of the
#              tools/cts/log/*-imx8mp-gles3-*-rows01-13 directory).
#
#   c1  imx8mp-gles3-merge.py --check over the slice directories
#       tools/cts/log/*-imx8mp-gles3-*-rows* exits 0 and prints
#       '133 entries with a record: OK'.
#   c2  imx8mp-gles3-merge.py --notpass over them equals
#       docs/es3/imx8mp-gles3-notpass.tsv without its '# SPDX' lines.
#   c3  imx8mp-gles3-report.py --date <day> over them exits 0, no PARTIAL.
#   c4  docs/es3-cts-failures.md has 'GC7000UL board run' and the first 8-hex
#       sha after it (that line or the next) is the slices' commit.
#   c5  docs/es3/board-recipe-imx8mp.md no longer says 'nothing has run on
#       the board yet' and names 'rows01-13'.
#   c6  the tools/cts/log/*-imx8mp-es2-*/README.md files hold at least 29
#       lines 'board.sh imx8mp e<N> rc='.
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
USAGE="usage: $0 [--date YYYY-MM-DD] [<repo root>]"
while [ $# -gt 0 ]; do
	case $1 in
	--date)
		[ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }
		DATE=$2
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
rows=("$LOG"/*-imx8mp-gles3-*-rows*)
es2=("$LOG"/*-imx8mp-es2-*)
gles3=("$LOG"/*-imx8mp-gles3-*)
r01=("$LOG"/*-imx8mp-gles3-*-rows01-13)
shopt -u nullglob
dirs=()
for d in "${rows[@]}"; do [ -d "$d" ] && dirs+=("$d"); done

# c1
if [ ${#dirs[@]} -eq 0 ]; then
	res 1 FAIL "no slice directory matches tools/cts/log/*-imx8mp-gles3-*-rows*"
else
	out=$(tool imx8mp-gles3-merge.py --check "${dirs[@]}" 2>&1)
	rc=$?
	if [ $rc -eq 0 ] && printf '%s\n' "$out" | grep -q '133 entries with a record: OK'; then
		res 1 ok "133 entries with a record: OK (${#dirs[@]} slice dirs)"
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
if [ ${#dirs[@]} -eq 0 ]; then
	res 3 FAIL "no slice directories to report"
elif [ -z "$DATE" ]; then
	res 3 FAIL "no campaign day: pass --date or name the rows01-13 directory <YYYY-MM-DD>-imx8mp-gles3-..."
else
	out=$(tool imx8mp-gles3-report.py --date "$DATE" "${dirs[@]}" 2>/dev/null)
	rc=$?
	# Exit code and stdout only: stderr noise is not a report line, and the
	# marker is looked for on every line, not the last.
	if [ $rc -ne 0 ]; then
		res 3 FAIL "report exit $rc"
	elif printf '%s\n' "$out" | grep -q PARTIAL; then
		res 3 FAIL "report says PARTIAL"
	else
		res 3 ok "report for $DATE: exit 0, no PARTIAL"
	fi
fi

# c4
f="$ROOT/docs/es3-cts-failures.md"
# Only what follows the phrase counts (earlier shas on the line are other
# work), and 'the GC7000UL board run is pending' is the state before the run.
c4_tail=
[ -f "$f" ] && c4_tail=$(grep -A1 'GC7000UL board run' "$f" | sed 's/.*GC7000UL board run//')
# The commit of the committed slices: the <sha8> of '<day>-imx8mp-gles3-<sha8>-rows...'.
c4_commit=$(for d in "${dirs[@]}"; do basename "$d" | sed -n 's/.*-imx8mp-gles3-\([0-9a-f]\{8\}\)-rows.*/\1/p'; done | sort -u)
c4_doc=$(printf '%s\n' "$c4_tail" | grep -v '^ is pending' |
	grep -Eo '(^|[^0-9a-zA-Z])[0-9a-f]{8}([^0-9a-zA-Z]|$)' | head -1 | tr -dc '0-9a-f')
if [ -z "$c4_tail" ]; then
	res 4 FAIL "docs/es3-cts-failures.md has no 'GC7000UL board run'"
elif [ -z "$c4_doc" ]; then
	res 4 FAIL "'GC7000UL board run' is pending or has no 8-hex sha on its line or the next"
elif [ ${#dirs[@]} -eq 0 ]; then
	res 4 ok "docs/es3-cts-failures.md: 'GC7000UL board run' at $c4_doc (no slices to compare, c1 fails)"
elif [ "$(printf '%s\n' "$c4_commit" | wc -l)" -ne 1 ] || [ -z "$c4_commit" ]; then
	res 4 FAIL "the slice directories name no single 8-hex commit (${c4_commit:-none})"
elif [ "$c4_doc" != "$c4_commit" ]; then
	res 4 FAIL "'GC7000UL board run' names $c4_doc, the committed slices are at $c4_commit"
else
	res 4 ok "docs/es3-cts-failures.md: 'GC7000UL board run' at $c4_commit, the slices' commit"
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
	k=$(grep -c 'board.sh imx8mp e[0-9]* rc=' "$d/README.md")
	n=$((n + k))
done
if [ "$n" -ge 29 ]; then
	res 6 ok "$n 'board.sh imx8mp e<N> rc=' lines in the es2 READMEs (>= 29)"
else
	res 6 FAIL "$n 'board.sh imx8mp e<N> rc=' lines in ${#es2[@]} es2 directories (need >= 29)"
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
