#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of tools/cts/am62p-gles3-acceptance.sh: fixture repositories under
# mktemp -d in $TMPDIR, each holding a copy of the tree generator, the
# mustpass list and the tree table, plus a not-pass TSV and a #1285 note made
# to pass or fail c1-c4. Last, the checked-in tree must give c1 FAIL (its TSV
# names two builds) and c3 PASS.
#
#   tools/cts/am62p-gles3-acceptance-selftest.sh
#
# Exit 0 when every case behaves, 1 otherwise.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
ACC="$HERE/am62p-gles3-acceptance.sh"
TOP=$(cd "$HERE/../.." && pwd)
WORK=$(mktemp -d "${TMPDIR:-$TOP/build}/am62p-acc-selftest.XXXXXX") || exit 1
trap 'rm -rf "$WORK"' EXIT
BAD=0
SHA=0123456789abcdef0123456789abcdef01234567
SHA8=${SHA:0:8}
N=$(grep -cv -e '^#' -e '^[[:space:]]*$' "$HERE/am62p-gles3-tree.txt")

fail() {
	echo "FAIL: $*" >&2
	BAD=$((BAD + 1))
}

# fixture <name>: a repository whose record passes c1-c4 at $SHA8
fixture() {
	local r="$WORK/$1"
	mkdir -p "$r/tools/cts" "$r/docs/es3" "$r/docs/notes"
	cp "$HERE/gen-am62p-gles3-tree.py" "$HERE/gles3-mustpass.txt" "$HERE/am62p-gles3-tree.txt" "$r/tools/cts/"
	printf '# x\n' >"$r/tools/cts/RESULTS.md"
	printf '# x\n' >"$r/tools/cts/STATUS.md"
	printf '# x\n' >"$r/ROADMAP.md"
	{
		printf '# dEQP-GLES3 on the TI AM62P: header\n'
		printf '# rows 0-%d: build %s, run 2026-11-30\n' $((N - 1)) "$SHA8"
		printf 'Fail\tdEQP-GLES3.functional.a.b\tmsg\n'
		printf 'NotSupported\tdEQP-GLES3.functional.a.c\tmsg\n'
	} >"$r/docs/es3/am62p-gles3-notpass.tsv"
	{
		printf '# note\n\n## 1. Earlier\n\nrow 30 resets 4 (old, outside the outcome section)\n\n'
		printf '## 9. Outcome (#1285)\n\nBuild %s, run 2026-11-30.\n\n' "$SHA8"
		printf '| row | case | resets | time |\n|---|---|---|---|\n'
		for i in $(seq 22 38); do printf '| %d | x | 0 | 12 s |\n' "$i"; done
		printf '\n## 10. After\n\nrow 31 resets 2\n'
	} >"$r/docs/notes/pvr-3d-mipmap-hang.md"
	git -C "$r" init -q
	git -C "$r" add -A
	git -C "$r" -c user.name=t -c user.email=t@t commit -q -m fixture
	printf '%s\n' "$r"
}

# expect <name> <want exit> <grep -E pattern on stdout>... -- <acceptance args>
expect() {
	local name=$1 want=$2
	shift 2
	local pats=()
	while [ $# -gt 0 ] && [ "$1" != -- ]; do
		pats+=("$1")
		shift
	done
	shift
	local out rc
	out=$(bash "$ACC" "$@" 2>"$WORK/stderr")
	rc=$?
	[ "$rc" -eq "$want" ] || fail "$name: exit $rc, want $want; stdout: $out"
	local p
	for p in "${pats[@]}"; do
		printf '%s\n' "$out" | grep -Eq "$p" || fail "$name: stdout lacks /$p/: $out"
	done
}

# --- pass -------------------------------------------------------------------
R=$(fixture pass)
expect pass 0 '^c1 PASS ' '^c2 PASS ' '^c3 PASS ' '^c4 PASS ' '^c5 PASS \(informational\)' -- --root "$R"
expect pass-sha 0 '^c1 PASS .*'"$SHA8" '^c4 PASS ' -- --root "$R" --sha "$SHA"
expect pass-sha8 0 '^c1 PASS ' -- --root "$R" --sha "$SHA8"

# re-split rows: 22 split into 22a/22b (one range each), prose outcome lines
R=$(fixture resplit)
T="$R/docs/es3/am62p-gles3-notpass.tsv"
sed -i "s/^# rows 0-.*/# rows 0-21: build $SHA8, run 2026-11-30\n# rows 22a-22a: build $SHA8, run 2026-12-01\n# rows 22b-22b: build $SHA8, run 2026-12-01\n# rows 23-$((N - 1)): build $SHA8, run 2026-11-30/" "$T"
M="$R/docs/notes/pvr-3d-mipmap-hang.md"
sed -i '/^| 22 |/d' "$M"
sed -i 's/^## 10. After$/- row 22a: 7 cases, resets 0\n- row 22b: 8 cases, 0 resets\n\n&/' "$M"
expect resplit 0 '^c1 PASS ' '^c4 PASS ' -- --root "$R" --sha "$SHA"

# the header gles3-notpass.py --splice --resplit writes: one range over the base
# rows, the re-split rows named in a parenthesis
R=$(fixture resplit-head)
T="$R/docs/es3/am62p-gles3-notpass.tsv"
sed -i "s/^# rows 0-.*/# rows 0-$((N - 1)) (21 re-split as 21a-21b, 30 re-split as 30a-30c): build $SHA8, run 2026-11-30\/12-01/" "$T"
expect resplit-head 0 '^c1 PASS .*'"$SHA8" -- --root "$R" --sha "$SHA"

# a re-split made in the generator: MAX_CASES 1900 gives the tree table one more
# row (N+1) and the splice writes 'rows 0-<N-1> (R re-split as Ra-Rb)'
regen() { # regen <fixture>: the fixture's generator re-split, its table regenerated
	sed -i 's/^MAX_CASES = 2000/MAX_CASES = 1900/' "$1/tools/cts/gen-am62p-gles3-tree.py"
	python3 -I "$1/tools/cts/gen-am62p-gles3-tree.py" >/dev/null || fail "regen $1: generator failed"
	N2=$(grep -cv -e '^#' -e '^[[:space:]]*$' "$1/tools/cts/am62p-gles3-tree.txt")
	[ "$N2" -eq $((N + 1)) ] || fail "regen $1: $N2 tree rows, want $((N + 1))"
}
R=$(fixture gen-resplit)
regen "$R"
T="$R/docs/es3/am62p-gles3-notpass.tsv"
sed -i "s/^# rows 0-.*/# rows 0-$((N - 1)) (21 re-split as 21a-21b): build $SHA8, run 2026-11-30\/12-01/" "$T"
expect gen-resplit 0 '^c1 PASS .*'"$SHA8" '^c3 PASS ' -- --root "$R" --sha "$SHA"

R=$(fixture gen-resplit-missing)
regen "$R"
sed -i "s/^# rows 0-.*/# rows 0-$((N - 2)) (21 re-split as 21a-21b): build $SHA8, run 2026-11-30/" "$R/docs/es3/am62p-gles3-notpass.tsv"
expect gen-resplit-missing-row 1 "^c1 FAIL .*rows $((N - 1)) of" -- --root "$R"

R=$(fixture gen-resplit-two)
regen "$R"
sed -i "s/^# rows 0-.*/# rows 0-30 (21 re-split as 21a-21b): build $SHA8, run 2026-11-30\n# rows 31-$((N - 1)): build ad2b5ed, run 2026-10-14/" \
	"$R/docs/es3/am62p-gles3-notpass.tsv"
expect gen-resplit-two-builds 1 '^c1 FAIL .*2 builds' -- --root "$R"

R=$(fixture gen-resplit-note)
regen "$R"
sed -i "s/^# rows 0-.*/# rows 0-$((N - 1)): build $SHA8, run 2026-11-30/" "$R/docs/es3/am62p-gles3-notpass.tsv"
expect gen-resplit-no-note 1 "^c1 FAIL .*rows $N of" -- --root "$R"

# --- c1 fail ----------------------------------------------------------------
R=$(fixture c1-two)
sed -i "s/^# rows 0-.*/# rows 0-10: build b25997e4, run 2026-11-22\n# rows 11-$((N - 1)): build ad2b5ed, run 2026-10-13\/14/" \
	"$R/docs/es3/am62p-gles3-notpass.tsv"
expect c1-two-builds 1 '^c1 FAIL .*2 builds' '^c2 PASS ' '^c3 PASS ' '^c4 FAIL ' -- --root "$R"
expect c1-two-builds-sha 1 '^c1 FAIL .*2 builds' '^c4 PASS ' -- --root "$R" --sha "$SHA"

R=$(fixture c1-resplit-two)
sed -i "s/^# rows 0-.*/# rows 0-30 (21 re-split as 21a-21b): build $SHA8, run 2026-11-30\n# rows 31-$((N - 1)): build ad2b5ed, run 2026-10-14/" \
	"$R/docs/es3/am62p-gles3-notpass.tsv"
expect c1-resplit-two-builds 1 '^c1 FAIL .*2 builds' -- --root "$R"

R=$(fixture c1-resplit-gap)
sed -i "s/^# rows 0-.*/# rows 0-20 (21 re-split as 21a-21b): build $SHA8, run 2026-11-30\n# rows 22-$((N - 1)): build $SHA8, run 2026-11-30/" \
	"$R/docs/es3/am62p-gles3-notpass.tsv"
expect c1-resplit-missing-row 1 '^c1 FAIL .*rows 21 of' -- --root "$R"

R=$(fixture c1-gap)
sed -i "s/^# rows 0-.*/# rows 0-20: build $SHA8, run 2026-11-30\n# rows 22-$((N - 1)): build $SHA8, run 2026-11-30/" \
	"$R/docs/es3/am62p-gles3-notpass.tsv"
expect c1-gap 1 '^c1 FAIL .*rows 21 of' -- --root "$R"

R=$(fixture c1-short)
sed -i "s/^# rows 0-.*/# rows 0-$((N - 2)): build $SHA8, run 2026-11-30/" "$R/docs/es3/am62p-gles3-notpass.tsv"
expect c1-short 1 "^c1 FAIL .*rows $((N - 1)) of" -- --root "$R"

R=$(fixture c1-beyond)
sed -i "s/^# rows 0-.*/# rows 0-$N: build $SHA8, run 2026-11-30/" "$R/docs/es3/am62p-gles3-notpass.tsv"
expect c1-beyond 1 '^c1 FAIL .*beyond' -- --root "$R"

R=$(fixture c1-twice)
sed -i "s/^# rows 0-.*/# rows 0-30: build $SHA8, run 2026-11-30\n# rows 30-$((N - 1)): build $SHA8, run 2026-12-01/" \
	"$R/docs/es3/am62p-gles3-notpass.tsv"
expect c1-twice 1 '^c1 FAIL .*rows 30 are named by more than one' -- --root "$R"

R=$(fixture c1-noheader)
sed -i '/^# rows /d' "$R/docs/es3/am62p-gles3-notpass.tsv"
expect c1-noheader 1 '^c1 FAIL .*no .# rows' -- --root "$R"

R=$(fixture c1-sha)
expect c1-wrong-sha 1 '^c1 FAIL .*not fedcba98' '^c4 FAIL .*fedcba98' -- --root "$R" --sha fedcba9876543210fedcba9876543210fedcba98

# --- c2 fail ----------------------------------------------------------------
R=$(fixture c2)
printf 'NoResult\tdEQP-GLES3.functional.texture.filtering.3d.formats.rgba8_linear_mipmap_linear\t\n' \
	>>"$R/docs/es3/am62p-gles3-notpass.tsv"
expect c2-noresult 1 '^c1 PASS ' '^c2 FAIL 1 NoResult' -- --root "$R"

# --- c3 fail ----------------------------------------------------------------
R=$(fixture c3)
sed -i '$d' "$R/tools/cts/am62p-gles3-tree.txt"
expect c3-table 1 '^c3 FAIL --check exit' -- --root "$R"
R=$(fixture c3-gen)
rm "$R/tools/cts/gen-am62p-gles3-tree.py"
expect c3-nogen 1 '^c3 FAIL .*does not exist' -- --root "$R"

# --- c4 fail ----------------------------------------------------------------
R=$(fixture c4-nosec)
sed -i 's/^## 9. Outcome (#1285)$/## 9. Results/' "$R/docs/notes/pvr-3d-mipmap-hang.md"
expect c4-no-section 1 '^c4 FAIL .*no outcome section' -- --root "$R"

R=$(fixture c4-reset)
sed -i 's/^| 30 | x | 0 |/| 30 | x | 1 |/' "$R/docs/notes/pvr-3d-mipmap-hang.md"
expect c4-reset 1 '^c4 FAIL .*resets not 0: 30 \(1\)' -- --root "$R"

R=$(fixture c4-missing)
sed -i '/^| 38 |/d' "$R/docs/notes/pvr-3d-mipmap-hang.md"
expect c4-missing-row 1 '^c4 FAIL .*rows 38 have no resets' -- --root "$R"

R=$(fixture c4-nosha)
sed -i "s/^Build $SHA8, run/Build of the day, run/" "$R/docs/notes/pvr-3d-mipmap-hang.md"
expect c4-no-sha 1 "^c4 FAIL .*does not name build $SHA8" -- --root "$R"

# --- c5 informational only --------------------------------------------------
R=$(fixture c5)
printf 'edited\n' >>"$R/ROADMAP.md"
expect c5-edited 0 '^c5 FAIL \(informational\)' -- --root "$R"
expect c5-base 0 '^c5 FAIL \(informational\) no such revision' -- --root "$R" --base no-such-rev

# --- usage ------------------------------------------------------------------
expect usage-opt 2 -- --bogus
expect usage-arg 2 -- --sha
expect usage-hex 2 -- --sha xyz1234
expect usage-short 2 -- --sha abc
expect usage-root 2 -- --root "$WORK/no-such-dir"

# --- the checked-in tree: two builds today ----------------------------------
# c3 must pass on the checked-in tree. c1 must say 'N builds' while the TSV's
# header names more than one build (today: ad2b5ed and b25997e4); once the
# splice leaves one build the case only asks c1 not to report several.
CUR_TSV="$TOP/docs/es3/am62p-gles3-notpass.tsv"
CUR_NB=$(sed -n 's/^# rows [0-9a-z]*-[0-9a-z]*: build \([0-9a-fA-F]*\), run .*/\1/p' "$CUR_TSV" |
	tr 'A-F' 'a-f' | sort -u | wc -l)
if [ "$CUR_NB" -gt 1 ]; then
	expect current-tree 1 "^c1 FAIL .*$CUR_NB builds" '^c3 PASS ' -- --root "$TOP"
else
	out=$(bash "$ACC" --root "$TOP" 2>/dev/null)
	printf '%s\n' "$out" | grep -q '^c3 PASS ' || fail "current-tree: c3 does not pass: $out"
	printf '%s\n' "$out" | grep -Eq '^c1 FAIL .*[0-9]+ builds' && fail "current-tree: c1 names several builds: $out"
fi

if [ $BAD -eq 0 ]; then
	echo "am62p-gles3-acceptance self-test ok"
	exit 0
fi
echo "am62p-gles3-acceptance self-test: $BAD failure(s)" >&2
exit 1
