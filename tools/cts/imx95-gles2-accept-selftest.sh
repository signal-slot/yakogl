#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# imx95-gles2-accept-selftest.sh - tools/cts/imx95-gles2-accept.sh on a
# throwaway git repository in $TMPDIR: a fake tree of six Verdin iMX95
# session slices (76 rows, one case each), a mustpass list and a
# docs/es2-cts-failures.md, showing PASS and FAIL of c1, c2 and c5.
#
#   sh tools/cts/imx95-gles2-accept-selftest.sh
#
# The serial path of the c5 redaction case is made up and put together at
# run time, so this file holds no such literal.
set -u
here=$(cd "$(dirname "$0")" && pwd)
A="$here/imx95-gles2-accept.sh"
TALLY="$here/mali-deqp-tally.py"
T=$(mktemp -d "${TMPDIR:-.}/imx95-gles2-accept-selftest.XXXXXX") || exit 2
T=$(cd "$T" && pwd)
trap 'rm -rf "$T"' EXIT HUP INT TERM
R="$T/repo"

# Every git call below, and every one the script and its tools make, runs
# with a configuration of this test's own: none of the host's (system,
# global, a HOME of this test's), no repository named by the caller's
# environment, and the throwaway repository trusted wherever TMPDIR lies
# (safe.directory: a scratch tree whose owner git does not take for the
# caller's makes every git call exit 128, and the script then exits 2).
# Messages are the C locale's.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_COMMON_DIR \
	GIT_NAMESPACE GIT_CEILING_DIRECTORIES GIT_TEMPLATE_DIR GIT_CONFIG_PARAMETERS
mkdir -p "$T/home"
HOME="$T/home"
LC_ALL=C
GIT_CONFIG_NOSYSTEM=1
GIT_CONFIG_GLOBAL=/dev/null
GIT_CONFIG_COUNT=1
GIT_CONFIG_KEY_0=safe.directory
GIT_CONFIG_VALUE_0='*'
export HOME LC_ALL GIT_CONFIG_NOSYSTEM GIT_CONFIG_GLOBAL GIT_CONFIG_COUNT GIT_CONFIG_KEY_0 GIT_CONFIG_VALUE_0

fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
g() { git -C "$R" -c user.name=selftest -c user.email=selftest@example.invalid -c commit.gpgsign=false "$@"; }

mkdir -p "$R/docs" "$R/tools/cts/log"
g init -q
g symbolic-ref HEAD refs/heads/main
printf '# fake\n' >"$R/README.md"
printf '# ES 2.0 CTS failure inventory\n\n## M. Mali-G610 (NanoPC-T6)\n\nNothing here.\n' >"$R/docs/es2-cts-failures.md"
if [ -f "$here/mali-gles2-tree.txt" ]; then
	grep '^# image' "$here/mali-gles2-tree.txt" >"$R/tools/cts/mali-gles2-tree.txt"
fi
g add -A
g commit -q -m base
BASE=$(g rev-parse HEAD)
SHA8=$(printf %s "$BASE" | cut -c1-8)
L="$R/tools/cts/log"

# One slice per session: one case per row, the first row's case Fail.
slice() { # session prefix first last
	d="$L/2026-11-25-imx95-gles2-$1-$SHA8"
	mkdir -p "$d"
	printf 'commit=%s\nsession=%s\nrows=%s-%s\ntree=tools/cts/mali-gles2-tree.txt\nboot_id_before=-\nboot_id_after=-\nkernel=zephyr\nlib=yakogl-mali\nstarted=2026-11-25T01:00:00Z\nended=2026-11-25T02:00:00Z\nboard=verdin-imx95\n' \
		"$BASE" "$1" "$3" "$4" >"$d/meta.txt"
	printf 'row\tlabel\tcases\tstatus\tseconds\tkmsg\tresets\tssh_after\tboot_same\texit_line\n' >"$d/chunks.tsv"
	printf 'case\tstatus\tdetail\n' >"$d/results.tsv"
	printf 'row\tlabel\tcases\tstatus\tseconds\tlifetime_s\tbbm\texit_line\n' >"$d/rows.tsv"
	i=$3
	while [ "$i" -le "$4" ]; do
		lab=$(printf '%s%02d' "$2" "$i")
		st=Pass
		[ "$i" -eq "$3" ] && st=Fail
		printf '%s\t%s\t1\tdone\t40\t0\t0\tyes\tyes\t-\n' "$i" "$lab" >>"$d/chunks.tsv"
		printf 'dEQP-GLES2.fake.%s\t%s\t%s\n' "$lab" "$st" "$st" >>"$d/results.tsv"
		printf '%s\t%s\t1\tdone\t40\t360\tmatch\tdeqp: OK (rc=0)\n' "$i" "$lab" >>"$d/rows.tsv"
		echo "dEQP-GLES2.fake.$lab" >>"$R/tools/cts/gles2-mustpass.txt"
		i=$((i + 1))
	done
}
slice S4 T6S4R 1 8
slice S1a T6S1R 1 12
slice S1b T6S1R 13 24
slice S2 T6S2R 1 14
slice S3a T6S3R 1 15
slice S3b T6S3R 16 30
S4="$L/2026-11-25-imx95-gles2-S4-$SHA8"
S3B="$L/2026-11-25-imx95-gles2-S3b-$SHA8"

doc_head() {
	printf '# ES 2.0 CTS failure inventory\n\n## M. Mali-G610 (NanoPC-T6)\n\nNothing here.\n\n## N. Mali-G310 (Verdin iMX95): the whole tree\n\nThe block:\n\n'
}
doc_tail() { printf '\n## O. Something after\n\n| Pass | 1 |\n'; }
{
	doc_head
	(cd "$R" && python3 -I "$TALLY" --tally --markdown tools/cts/log/*-imx95-gles2-S*-"$SHA8")
	doc_tail
} >"$R/docs/es2-cts-failures.md"
g add -A
g commit -q -m sessions
GOOD=$(g rev-parse HEAD)
fresh() { g reset -q --hard "$GOOD" && g clean -q -fd; }

# run: the three lines of the last run are in $out, its exit code in $rc
run() {
	out=$(sh "$A" --root "$R" --base "$BASE" --main-ref main "$@" 2>"$T/err")
	rc=$?
}
# expect <name> <exit code> <c1> <c2> <c5> [text that must be in the output]
expect() {
	e=
	[ "$rc" -eq "$2" ] || e="exit $rc, want $2"
	[ "$(printf '%s\n' "$out" | grep -c '^c[125]: ')" -eq 3 ] || e="$e; not three criterion lines"
	[ "$(printf '%s\n' "$out" | grep -c .)" -eq 3 ] || e="$e; extra lines on stdout"
	for c in "c1 $3" "c2 $4" "c5 $5"; do
		printf '%s\n' "$out" | grep -q "^${c%% *}: ${c##* } " || e="$e; ${c%% *} is not ${c##* }"
	done
	if [ $# -ge 6 ]; then
		printf '%s\n' "$out" | grep -qF -e "$6" || e="$e; no \"$6\""
	fi
	if [ -z "$e" ]; then
		ok "$1"
	else
		bad "$1: $e"
		printf '%s\n' "$out" | sed 's/^/    /'
		sed 's/^/    | /' "$T/err"
	fi
}

run
expect "the good tree: all three pass" 0 PASS PASS PASS "status rows equal --tally"
run tools/cts/gles2-mustpass.txt
expect "the mustpass list as an argument" 0 PASS PASS PASS
(cd "$R" && out=$(sh "$A" --base "$BASE" --main-ref main 2>/dev/null) && [ "$(printf '%s\n' "$out" | grep -c ': PASS ')" -eq 3 ]) &&
	ok "the root defaults to ." || bad "the root defaults to ."
(cd "$T" && out=$(TMPDIR=. sh "$A" --root repo --base "$BASE" --main-ref main 2>/dev/null) && [ "$(printf '%s\n' "$out" | grep -c ': PASS ')" -eq 3 ]) &&
	ok "a relative root and a relative TMPDIR" || bad "a relative root and a relative TMPDIR"
[ -z "$(find "$T" -name 'imx95-gles2-accept.*' -prune -print)" ] && ok "no scratch directory is left" || bad "a scratch directory is left"

# c1
rm -rf "$S3B"
run
expect "c1: a session directory missing (c2 has no slices to tally)" 1 FAIL FAIL PASS "imx95-gles2-S3b-$SHA8/ (want 1)"
fresh
mv "$S3B" "$L/2026-11-25-imx95-gles2-S5-$SHA8"
run
expect "c1: a session that is none of the six" 1 FAIL FAIL PASS "none of the six"
fresh
cp -R "$S4" "$L/2026-11-26-imx95-gles2-S4-$SHA8"
run
expect "c1: a session twice" 1 FAIL FAIL PASS "2 directories"
fresh
sed '$d' "$R/tools/cts/gles2-mustpass.txt" >"$T/mp" && { cat "$T/mp"; echo dEQP-GLES2.fake.extra; } >"$R/tools/cts/gles2-mustpass.txt"
run
expect "c1: --check-cover fails (a mustpass case without a result)" 1 FAIL PASS PASS "--check-cover exits 1"
fresh
run no-such-mustpass.txt
expect "c1: the mustpass list is not a file" 1 FAIL PASS PASS "not a file"
run --main-ref no-such-ref
expect "c1: the slices' commit is not an ancestor of --main-ref" 1 FAIL PASS PASS "--check-cover exits 1"
rm "$S4/rows.tsv"
run
expect "c1: no rows.tsv" 1 FAIL PASS PASS "has no rows.tsv"
fresh
sed '$d' "$S4/rows.tsv" >"$T/rows" && cp "$T/rows" "$S4/rows.tsv"
run
expect "c1: rows.tsv lacks a row" 1 FAIL PASS PASS "T6S4R08"
# the span guard of imx95-rows-tsv.py --check refuses the same truncation
expect "c1: rows.tsv lacks a row: the guard of --check refuses it too" 1 FAIL PASS PASS "imx95-rows-tsv.py --check exits 1"
fresh
# a split table (S3b keeps session-wide numbers 16..30) without its last row
sed '$d' "$S3B/rows.tsv" >"$T/rows" && cp "$T/rows" "$S3B/rows.tsv"
run
expect "c1: S3b table lacks T6S3R30" 1 FAIL PASS PASS "imx95-rows-tsv.py --check exits 1"
fresh
# a duplicate row of S3b
tail -n 1 "$S3B/rows.tsv" >>"$S3B/rows.tsv"
run
expect "c1: S3b table repeats a row" 1 FAIL PASS PASS "imx95-rows-tsv.py --check exits 1"
fresh
awk -F '\t' -v OFS='\t' 'NR == 3 { $5 = "-" } { print }' "$S4/rows.tsv" >"$T/rows" && cp "$T/rows" "$S4/rows.tsv"
run
expect "c1: a row without seconds" 1 FAIL PASS PASS 'seconds is "-"'
fresh
cut -f1-5,7- "$S4/rows.tsv" >"$T/rows" && cp "$T/rows" "$S4/rows.tsv"
run
expect "c1: rows.tsv without the lifetime_s column" 1 FAIL PASS PASS "lifetime_s"
fresh
awk -F '\t' -v OFS='\t' 'NR == 2 { $7 = "" } { print }' "$S4/rows.tsv" >"$T/rows" && cp "$T/rows" "$S4/rows.tsv"
run
expect "c1: a row without bbm" 1 FAIL PASS PASS "bbm"
fresh

# c2
sed 's/^| Pass | 70 |$/| Pass | 71 |/' "$R/docs/es2-cts-failures.md" >"$T/doc"
cmp -s "$T/doc" "$R/docs/es2-cts-failures.md" && bad "fixture: the block has no '| Pass | 70 |' row"
cp "$T/doc" "$R/docs/es2-cts-failures.md"
run
expect "c2: a stale block" 1 PASS FAIL PASS "differ from --tally"
printf '%s\n' "$out" | grep -qF -e "$T" && bad "c2: the verdict names the scratch directory"
fresh
printf '# ES 2.0 CTS failure inventory\n\n## M. Mali-G610 (NanoPC-T6)\n\nVerdin iMX95 is only named in the text.\n' >"$R/docs/es2-cts-failures.md"
run
expect "c2: no Verdin iMX95 part" 1 PASS FAIL PASS "no heading naming Verdin iMX95"
{
	doc_head
	printf '| Status | Cases |\n| --- | ---: |\n| Pass | 70 |\n| Fail | 6 |\n| **Total** | **76** |\n'
	doc_tail
} >"$R/docs/es2-cts-failures.md"
run
expect "c2: status rows without a block, equal to --tally" 0 PASS PASS PASS "--check-doc does not apply"
{
	doc_head
	printf '| Status | Cases |\n| --- | ---: |\n| Pass | 69 |\n| Fail | 7 |\n| **Total** | **76** |\n'
	doc_tail
} >"$R/docs/es2-cts-failures.md"
run
expect "c2: status rows that differ from --tally" 1 PASS FAIL PASS "differ from --tally"
{
	doc_head
	printf 'No table yet.\n'
	doc_tail
} >"$R/docs/es2-cts-failures.md"
run
expect "c2: a part with nothing to compare (the next part's rows are not read)" 1 PASS FAIL PASS "neither"
rm "$R/docs/es2-cts-failures.md"
run
expect "c2: no document" 1 PASS FAIL PASS "is not a file"
fresh

# c5
echo "more" >>"$R/README.md"
g commit -q -am readme
run
expect "c5: README.md changed (on --main-ref too: said)" 1 PASS PASS FAIL "1 path(s) it must not: README.md (1 of them also differ between $BASE and main)"
run --main-ref "$GOOD"
expect "c5: README.md changed by the branch alone" 1 PASS PASS FAIL "1 path(s) it must not: README.md"
printf '%s\n' "$out" | grep -qF -e "also differ" && bad "c5: a path --main-ref did not change is charged to it"
fresh
mkdir -p "$R/third_party/x" "$R/driver/src/v3d/generated" "$R/driver/src/fw" "$R/tools/cts"
for f in third_party/x/a.h driver/src/v3d/generated/a.rs driver/src/fw/pvr_fw_blob.h tools/board.sh \
	tools/board_selftest.sh tools/cts/RESULTS.md tools/cts/STATUS.md ROADMAP.md; do
	echo x >"$R/$f"
done
g add -A -f
g commit -q -m excluded
run
expect "c5: every excluded path is counted" 1 PASS PASS FAIL "8 path(s)"
fresh
mkdir -p "$R/driver/src/fw" "$R/docs/notes"
echo x >"$R/driver/src/fw/loader.c"
echo x >"$R/docs/notes/README.md"
echo x >"$R/tools/board.sh.md"
g add -A
g commit -q -m allowed
run
expect "c5: paths that only look like excluded ones" 0 PASS PASS PASS
fresh
TTY="/dev/tty""USB7"
echo "console on $TTY" >"$R/tools/cts/log/note.txt"
g add -A
g commit -q -m leak
run
expect "c5: redact-diff-scan.py reports a hit" 1 PASS PASS FAIL "reports 1 hit(s), first: tools/cts/log/note.txt:1: tty: "
printf '%s\n' "$out" | grep -qF -e "$TTY" && bad "c5: the hit is printed in full"
fresh

# the base, and usage
out=$(sh "$A" --root "$R" --base 0000000000000000000000000000000000000001 --main-ref main 2>/dev/null)
rc=$?
expect "a base commit that is not in the repository" 1 FAIL FAIL FAIL "is not in this repository"
sh "$A" --root "$T/nope" >/dev/null 2>&1
[ $? -eq 2 ] && ok "a root that is no directory: exit 2" || bad "a root that is no directory: exit 2"
sh "$A" --root "$R" --frob >/dev/null 2>&1
[ $? -eq 2 ] && ok "an unknown option: exit 2" || bad "an unknown option: exit 2"
mkdir "$T/plain"
sh "$A" --root "$T/plain" >/dev/null 2>&1
[ $? -eq 2 ] && ok "a root that is no git repository: exit 2" || bad "a root that is no git repository: exit 2"
# The gate's TMPDIR is under this repository's build/: a directory inside a
# work tree is not its top either.
sh "$A" --root "$R/docs" >/dev/null 2>&1
[ $? -eq 2 ] && ok "a root inside a work tree, not its top: exit 2" || bad "a root inside a work tree, not its top: exit 2"

if [ "$fail" -eq 0 ]; then
	echo "imx95-gles2-accept-selftest: ok"
else
	# What ran, for a log read without the host (the gate keeps a failed
	# row's whole output).
	echo "imx95-gles2-accept-selftest: FAILED"
	echo "    TMPDIR=${TMPDIR:-} scratch=$T"
	echo "    git: $(git --version 2>&1 | sed -n '1p'); python3: $(python3 -I --version 2>&1 | sed -n '1p')"
	echo "    sh: $(command -v sh) -> $(readlink -f "$(command -v sh)" 2>/dev/null); awk: $(command -v awk) -> $(readlink -f "$(command -v awk)" 2>/dev/null)"
	g status --short 2>&1 | sed -n '1,20p' | sed 's/^/    git status: /'
fi
exit "$fail"
