#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of imx8mp-gles3-acceptance.sh: a fake repository under
# $TMPDIR with stub python tools, one all-pass case and one case per check
# that must turn only that check to FAIL; Armadillo-X2 slices (alone, mixed with
# a Verdin slice, without their measured-on line) and, with the real merge
# tool, a mixed Verdin + X2 tree of 133 entries.
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
	out=$(sh "$ACC" ${ARGS:-} "$R" 2>"$WORK/stderr")
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

fresh c3p
cat >"$R/tools/cts/imx8mp-gles3-report.py" <<'PYEOF'
import sys
print("  problem: entry 7 has no record")
print("with the ROADMAP.md draft above")
sys.exit(1)
PYEOF
out=$(sh "$ACC" "$R" 2>/dev/null)
if printf '%s\n' "$out" | grep -q '^c3 FAIL .*report exit 1: entry 7 has no record$'; then
	echo "ok   c3 FAIL names report.py's problem line"
else
	echo "FAIL c3 problem line"; printf '%s\n' "$out" | grep '^c3'; BAD=$((BAD + 1))
fi

fresh c3b
mv "$R/tools/cts/log/2026-11-12-imx8mp-gles3-0123abcd-rows01-13" "$R/tools/cts/log/2026-11-imx8mp-gles3-0123abcd-rows01-13"
run_case "c3 no day in the rows01-13 name" 1 "c3"

fresh c3c
mv "$R/tools/cts/log/2026-11-12-imx8mp-gles3-0123abcd-rows01-13" "$R/tools/cts/log/2026-11-imx8mp-gles3-0123abcd-rows01-13"
printf '# slice\n\nRun 2026-11-12, 19:47-22:19 UTC (the board clock)\n' >"$R/tools/cts/log/2026-11-imx8mp-gles3-0123abcd-rows01-13/README.md"
run_case "c3 day from the README run window" 0 ""

fresh c4
printf 'the GC7000UL board run is pending, see 0123abcd\n' >"$R/docs/es3-cts-failures.md"
run_case "c4 pending" 1 "c4"

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

# two campaigns in one tree: the merge tool refuses a directory of the other commit
mk2() {
	mkdir -p "$R/tools/cts/log/2026-11-imx8mp-gles3-9bc82915-rows01-13"
	echo "commit 9bc82915" >"$R/tools/cts/log/2026-11-imx8mp-gles3-9bc82915-rows01-13/README.md"
	sed -i 's/^import os, sys$/import os, sys\nif any("9bc82915" in a for a in sys.argv):\n    print("two commits")\n    sys.exit(1)/' "$R/tools/cts/imx8mp-gles3-merge.py"
}
fresh two
mk2
run_case "two campaigns without --commit: c1 c2 fail" 1 "c1 c2"
ARGS="--commit 0123abcd"
run_case "--commit reads only that campaign's slices" 0 ""
ARGS="--commit 0123abcdef0123"
run_case "--commit takes a longer hash (cut to 8)" 0 ""
ARGS=
out=$(sh "$ACC" --commit xyz "$R" 2>&1); rc=$?
if [ "$rc" = 2 ]; then echo "ok   --commit with non-hex: usage error 2"; else echo "FAIL --commit non-hex: rc=$rc"; BAD=$((BAD + 1)); fi

# Armadillo-X2 slices (board.sh x2 verdict lines; rows by the owner's decision of 2026-11-27)
X2L='board.sh x2 e17 rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=record:DEQP resets=absent back_after=40s (kexec-dev: a development run, never a row)'
fresh x2
printf 'Run 2026-11-13 on the Armadillo-X2, measured on the Armadillo-X2 (tools/board.sh x2, crash-route kexec)\n%s\n' "$X2L" \
	>"$R/tools/cts/log/2026-11-13-imx8mp-gles3-0123abcd-rows17-31/README.md"
run_case "a Verdin and an X2 slice (mixed tree) pass" 0 ""
if sh "$ACC" "$R" 2>/dev/null | grep -q '^c1 ok .*(2 slice dirs, 1 measured on the Armadillo-X2)$'; then
	echo "ok   c1 counts the X2 slices"
else
	echo "FAIL c1 X2 count"; BAD=$((BAD + 1))
fi
fresh x2b
printf '%s\n' "$X2L" >"$R/tools/cts/log/2026-11-13-imx8mp-gles3-0123abcd-rows17-31/README.md"
run_case "an X2 slice that does not say it was measured on the X2 fails c1" 1 "c1"
if sh "$ACC" "$R" 2>/dev/null | grep -q "^c1 FAIL X2 slice README without 'measured on the Armadillo-X2': 2026-11-13-imx8mp-gles3-0123abcd-rows17-31$"; then
	echo "ok   c1 names the unmarked X2 slice"
else
	echo "FAIL c1 unmarked X2 slice name"; BAD=$((BAD + 1))
fi
fresh x2c
sed -i 's/^board\.sh imx8mp /board.sh x2 /' "$R/tools/cts/log/2026-11-12-imx8mp-es2-0123abcd/README.md"
run_case "c6 counts board.sh x2 lines of an es2 record" 0 ""

# The real merge tool on a mixed tree: a Verdin slice (entries 0-66) and an X2 slice (entries 67-132, '-' in
# begun and noresult, board.sh x2 lines) of one commit give c1 ok
fresh real
rm -r "$R"/tools/cts/log/*-imx8mp-gles3-*
cp "$HERE/imx8mp-gles3-merge.py" "$HERE/imx8mp-gles3-tree.txt" "$HERE/gles3-mustpass.txt" "$R/tools/cts/"
V="$R/tools/cts/log/2026-11-16-imx8mp-gles3-0123abcd-rows01-67"
X="$R/tools/cts/log/2026-11-16-imx8mp-gles3-0123abcd-rows68-133"
mkdir -p "$V" "$X"
python3 -I - "$HERE/imx8mp-gles3-tree.txt" "$V" "$X" <<'PYEOF'
import sys
table = [l.rstrip('\n') for l in open(sys.argv[1]) if l.strip() and not l.lstrip().startswith('#')]
c = '0123abcd' + '0' * 32
for d, lo, hi, board in ((sys.argv[2], 0, 67, 'imx8mp'), (sys.argv[3], 67, 133, 'x2')):
    with open(d + '/README.md', 'w') as f:
        f.write('commit %s\n' % c)
        if board == 'x2':
            f.write('measured on the Armadillo-X2 (tools/board.sh x2, crash-route kexec)\n')
        for i in range(lo, hi):
            if board == 'x2':
                f.write('board.sh x2 e%02d rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=record:DEQP '
                        'resets=absent back_after=40s (kexec-dev: a development run, never a row)\n' % i)
            else:
                f.write('board.sh imx8mp e%02d rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled '
                        'result=record:DEQP resets=absent back_after=90s uptime=25s\n' % i)
    with open(d + '/results.tsv', 'w') as f:
        f.write('# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names\n')
        for i, e in enumerate(table):
            if not lo <= i < hi:
                f.write('%d\t%s\t-\t-\t-\t-\t-\t-\t-\n' % (i, e))
            elif board == 'x2':
                f.write('%d\t%s\t-\t1\t0\t0\t0\t-\t-\n' % (i, e))
            else:
                f.write('%d\t%s\t1\t1\t0\t0\t0\t0\t-\n' % (i, e))
PYEOF
out=$(sh "$ACC" "$R" 2>/dev/null)
if printf '%s\n' "$out" | grep -q '^c1 ok 133 entries with a record: OK (2 slice dirs, 1 measured on the Armadillo-X2)$'; then
	echo "ok   the real merge tool accepts a mixed Verdin + X2 tree (c1)"
else
	echo "FAIL real merge on a mixed tree"; printf '%s\n' "$out"; BAD=$((BAD + 1))
fi
sed -i '/measured on the Armadillo-X2/d' "$X/README.md"
if sh "$ACC" "$R" 2>/dev/null | grep -q "^c1 FAIL X2 slice README without 'measured on the Armadillo-X2': ${X##*/}$"; then
	echo "ok   the same tree without the X2 line fails c1"
else
	echo "FAIL real tree without the X2 line"; BAD=$((BAD + 1))
fi

if [ $BAD -eq 0 ]; then
	echo "imx8mp-gles3-acceptance-selftest: all cases passed"
else
	echo "imx8mp-gles3-acceptance-selftest: $BAD case(s) failed"
	exit 1
fi
