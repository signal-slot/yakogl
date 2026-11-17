#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of imx8mp-gles3-acceptance.sh: a fake repository under
# $TMPDIR with stub python tools, one all-pass case and one case per check
# that must turn only that check to FAIL.
#
#   sh tools/cts/imx8mp-gles3-acceptance-selftest.sh
[ -n "${BASH_VERSION:-}" ] || exec bash "$0" "$@"
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
ACC="$HERE/imx8mp-gles3-acceptance.sh"
BASE=${TMPDIR:-}
[ -n "$BASE" ] && [ -d "$BASE" ] || { echo "selftest: TMPDIR must name a directory" >&2; exit 2; }
WORK=$(mktemp -d "$BASE/imx8mp-acc.XXXXXX") || exit 2
trap 'rm -rf "$WORK"' EXIT
BAD=0

# A stub is a python file that fails when the marker file named after it
# exists in the fake root.
stub() { # stub <name> <stdout text>
	cat >"$R/tools/cts/$1" <<PYEOF
import os, sys
root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
if os.path.exists(os.path.join(root, "fail-$1")):
    print("stub failure")
    sys.exit(1)
print("""$2""")
PYEOF
}

mkfix() { # mkfix <dir>: an all-pass fake repository
	R=$1
	mkdir -p "$R/tools/cts/log/2026-11-12-imx8mp-gles3-0123abcd-rows01-13" \
		"$R/tools/cts/log/2026-11-13-imx8mp-gles3-0123abcd-rows17-31" \
		"$R/tools/cts/log/2026-11-12-imx8mp-es2-0123abcd" "$R/docs/es3"
	stub imx8mp-gles3-merge.py "checked 2 inputs, 133 entries with a record: OK"
	stub imx8mp-gles3-report.py "drafts"
	stub check-imx8mp-gles3-tree.py "matched exactly once 42810
OK"
	printf '# SPDX-License-Identifier: x\nentry\n' >"$R/docs/es3/imx8mp-gles3-notpass.tsv"
	printf 'GC7000UL board run at 0123abcd\n' >"$R/docs/es3-cts-failures.md"
	printf 'slices rows01-13 and rows17-31\n' >"$R/docs/es3/board-recipe-imx8mp.md"
	i=0
	: >"$R/tools/cts/log/2026-11-12-imx8mp-es2-0123abcd/README.md"
	while [ $i -lt 29 ]; do
		echo "board.sh imx8mp e$i rc=0" >>"$R/tools/cts/log/2026-11-12-imx8mp-es2-0123abcd/README.md"
		i=$((i + 1))
	done
	echo "ok log" >"$R/tools/cts/log/2026-11-12-imx8mp-gles3-0123abcd-rows01-13/README.md"
}
# The stub merge prints the not-pass list as 'entry' for --notpass.
patch_notpass() {
	python3 -I - "$R/tools/cts/imx8mp-gles3-merge.py" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read().replace('print("""', 'print("entry" if "--notpass" in sys.argv else """', 1)
open(p, "w").write(s)
PYEOF
}

# expect <case> <expected exit> <expected FAIL list, space separated or ''>
run_case() {
	local name=$1 want_rc=$2 want_fail=$3 out rc got
	out=$(sh "$ACC" "$R" 2>"$WORK/stderr")
	rc=$?
	got=$(printf '%s\n' "$out" | sed -n 's/^\(c[0-9]\) FAIL.*/\1/p' | tr '\n' ' ' | sed 's/ $//')
	lines=$(printf '%s\n' "$out" | grep -Ec '^c[0-9] (ok|FAIL) ')
	if [ "$rc" = "$want_rc" ] && [ "$got" = "$want_fail" ] && [ "$lines" = 8 ] &&
		printf '%s\n' "$out" | grep -q '^summary: ' && ! grep -Eiq 'not found|syntax|unbound|: line [0-9]|Traceback' "$WORK/stderr"; then
		echo "ok   $name"
	else
		echo "FAIL $name: rc=$rc (want $want_rc), failing '$got' (want '$want_fail'), $lines lines"
		printf '%s\n' "$out"
		cat "$WORK/stderr"
		BAD=$((BAD + 1))
	fi
}

fresh() { # fresh <case>
	mkfix "$WORK/$1"
	patch_notpass
}

fresh all
run_case "all pass" 0 ""

fresh c1
touch "$R/fail-imx8mp-gles3-merge.py"
run_case "c1 and c2 fail when the merge tool fails" 1 "c1 c2"

fresh c1b
rm -r "$R"/tools/cts/log/*-imx8mp-gles3-*-rows*
run_case "no slice directory fails c1 c2 c3 (c6 8 stay)" 1 "c1 c2 c3"

fresh c2
echo "other" >"$R/docs/es3/imx8mp-gles3-notpass.tsv"
run_case "c2 differing not-pass list" 1 "c2"

fresh c3
touch "$R/fail-imx8mp-gles3-report.py"
run_case "c3 report fails" 1 "c3"

fresh c3b
mv "$R/tools/cts/log/2026-11-12-imx8mp-gles3-0123abcd-rows01-13" "$R/tools/cts/log/2026-11-imx8mp-gles3-0123abcd-rows01-13"
run_case "c3 no day in the rows01-13 name" 1 "c3"

fresh c4
printf 'the GC7000UL board run is pending, see 0123abcd\n' >"$R/docs/es3-cts-failures.md"
run_case "c4 pending" 1 "c4"

fresh c4b
printf 'the GC7000UL board run at 05769bf2\n' >"$R/docs/es3-cts-failures.md"
run_case "c4 stale commit" 1 "c4"

fresh c3c
# noise before and after the drafts, exit 0, no PARTIAL: ok
printf 'import sys\nprint("warning: a stray line")\nprint("drafts")\nprint("note: done", file=sys.stderr)\nprint("trailer")\n' >"$R/tools/cts/imx8mp-gles3-report.py"
run_case "c3 noise around the report is not a failure" 0 ""

fresh c3d
# PARTIAL in the middle of the output, a clean last line, exit 0: FAIL
printf 'print("*** PARTIAL: merge failed ***")\nprint("drafts")\nprint("ok")\n' >"$R/tools/cts/imx8mp-gles3-report.py"
run_case "c3 PARTIAL above a clean last line" 1 "c3"

fresh c5
echo "nothing has run on the board yet; rows01-13" >"$R/docs/es3/board-recipe-imx8mp.md"
run_case "c5 recipe still says nothing has run" 1 "c5"

fresh c6
sed -i '1d' "$R/tools/cts/log/2026-11-12-imx8mp-es2-0123abcd/README.md"
run_case "c6 28 lines" 1 "c6"

fresh c7
touch "$R/fail-check-imx8mp-gles3-tree.py"
run_case "c7 tree check fails" 1 "c7"

fresh c8
echo "uart on /dev/ttyUSB0" >"$R/tools/cts/log/2026-11-12-imx8mp-gles3-0123abcd-rows01-13/e00.console.txt"
run_case "c8 device path" 1 "c8"

fresh c8b
# The tree holds no private address literally (tests/redaction_scan.rs scans
# this file too): the fixture address is joined at run time.
printf 'host %s.%s.%s.%s:22\n' 192 168 1 5 >"$R/tools/cts/log/2026-11-12-imx8mp-es2-0123abcd/x.txt"
run_case "c8 address in an es2 log" 1 "c8"

fresh empty
rm -r "$R"/tools/cts/log/*
run_case "empty log tree: clean FAIL, c7 ok" 1 "c1 c2 c3 c6"

if [ $BAD -eq 0 ]; then
	echo "imx8mp-gles3-acceptance-selftest: all cases passed"
else
	echo "imx8mp-gles3-acceptance-selftest: $BAD case(s) failed"
	exit 1
fi
