#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx8mp-gles3-issues.py on synthetic compare output: grouping by cause, sort by count, the
# verdict filter, --examples, entry order, unknown causes, bad input. Host only; writes under $TMPDIR.
[ -n "${BASH_VERSION:-}" ] || exec bash "$0" "$@"
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:?TMPDIR must be set}/gles3-issues-selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
tool="$here/imx8mp-gles3-issues.py"
printf 'c1\tdEQP-GLES3.a.*\tAlpha thing\nc2\tdEQP-GLES3.b.*\tBeta thing\n' > "$tmp/causes.tsv"
cat > "$tmp/in.tsv" <<'IN'
board-only	c1	e12	Fail	dEQP-GLES3.a.x1
board-only	c1	e3	Crash	dEQP-GLES3.a.x2
board-only	c2	e7	Fail	dEQP-GLES3.b.y1
board-only	c2	e7	Fail	dEQP-GLES3.b.y2
board-only	c2	e8	Fail	dEQP-GLES3.b.y3
board-only	unclassified	e9	Fail	dEQP-GLES3.z.q
host-predicted	c1	e1	Fail	dEQP-GLES3.a.pred
host-other	c3	e2	Fail	dEQP-GLES3.o
IN
run() { python3 -I "$tool" "$tmp/in.tsv" --causes "$tmp/causes.tsv" "$@"; }

run --commit abcd1234 > "$tmp/out.md"
grep -q '^## GC7000UL: Beta thing: 3 dEQP-GLES3 cases fail on the board only$' "$tmp/out.md" || fail "beta title"
grep -q '^## GC7000UL: Alpha thing: 2 dEQP-GLES3 cases fail on the board only$' "$tmp/out.md" || fail "alpha title"
grep -q '^## GC7000UL: unclassified failures: 1 ' "$tmp/out.md" || fail "unclassified title"
b=$(grep -n 'Beta thing' "$tmp/out.md" | head -1 | cut -d: -f1)
a=$(grep -n 'Alpha thing' "$tmp/out.md" | head -1 | cut -d: -f1)
[ "$b" -lt "$a" ] || fail "not sorted by count"
grep -q 'Table entries: e3, e12$' "$tmp/out.md" || fail "entries not numerically sorted"
grep -q 'Table entries: e7, e8$' "$tmp/out.md" || fail "beta entries"
grep -q 'Campaign commit: abcd1234' "$tmp/out.md" || fail "commit"
grep -q 'docs/es3/board-recipe-imx8mp.md' "$tmp/out.md" || fail "reproduce line"
if grep -q 'pred\|dEQP-GLES3\.o' "$tmp/out.md"; then fail "other verdicts leaked"; fi
echo "ok: board-only blocks, sorted, entries, commit"

n=$(run --examples 1 | grep -c '^  - `' || true)
[ "$n" = 3 ] || fail "--examples 1 should print one example per cause (3), got $n"
if run --examples 0 | grep -q Examples; then fail "--examples 0 prints examples"; fi
run --verdict host-predicted | grep -q 'Alpha thing: 1 dEQP-GLES3 cases fail on the board only' || fail "--verdict"
echo "ok: --examples, --verdict"

printf 'board-only\tc9\te1\tFail\tdEQP-GLES3.n\n' > "$tmp/u.tsv"
python3 -I "$tool" "$tmp/u.tsv" --causes "$tmp/causes.tsv" | grep -q '^## GC7000UL: c9: 1 ' || fail "unknown cause id"
printf 'bad line\n' > "$tmp/bad.tsv"
if python3 -I "$tool" "$tmp/bad.tsv" --causes "$tmp/causes.tsv" 2>/dev/null; then fail "bad input accepted"; fi
: > "$tmp/empty.tsv"
[ -z "$(python3 -I "$tool" "$tmp/empty.tsv" --causes "$tmp/causes.tsv")" ] || fail "empty input prints"
echo "ok: unknown cause, bad input, empty input"
