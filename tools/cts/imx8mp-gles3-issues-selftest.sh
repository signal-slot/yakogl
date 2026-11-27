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

# no-record / unexplained causes: stderr WARNING with the case count, exit 0; --strict exits 1; stdout unchanged.
printf 'board-only\tunexplained-ubo-no-record\te5\tFail\tdEQP-GLES3.u1\nboard-only\tunexplained-ubo-no-record\te5\tFail\tdEQP-GLES3.u2\nboard-only\tx-no-record\te6\tFail\tdEQP-GLES3.u3\nboard-only\tunexplained-no-record\te6\tFail\tdEQP-GLES3.u4\nboard-only\tc1\te1\tFail\tdEQP-GLES3.a.k\nhost-predicted\tunexplained-zz\te1\tFail\tdEQP-GLES3.p\n' > "$tmp/nr.tsv"
nr() { python3 -I "$tool" "$tmp/nr.tsv" --causes "$tmp/causes.tsv" "$@"; }
nr > "$tmp/nr.out" 2> "$tmp/nr.err" || fail "warning changed the exit status"
grep -q 'WARNING: cause unexplained-ubo-no-record has 2 case(s): triage must give' "$tmp/nr.err" || fail "no-record warning"
grep -q 'WARNING: cause x-no-record has 1 case' "$tmp/nr.err" || fail "-no-record suffix warning"
grep -q 'WARNING: cause unexplained-no-record has 1 case' "$tmp/nr.err" || fail "unexplained warning"
[ "$(grep -c WARNING "$tmp/nr.err")" = 3 ] || fail "warning count (other verdicts must not warn)"
if nr --strict >/dev/null 2>&1; then fail "--strict exit 0 with a no-record cause"; fi
nr --strict > "$tmp/nr2.out" 2>/dev/null || true; cmp -s "$tmp/nr2.out" "$tmp/nr.out" || fail "--strict changed stdout"
[ -z "$(run 2>&1 >/dev/null)" ] || fail "warning without a no-record cause"
run --strict >/dev/null || fail "--strict failed without a no-record cause"
echo "ok: no-record warning, --strict"

# Armadillo-X2 slices (--slices): the draft names the X2, the 8-hex commit and the slice directories; Verdin
# slices keep the plain text. board_of() comes from imx8mp-gles3-merge.py (three-word rows, begun '-').
x2d="$tmp/20261127-imx8mp-gles3-x2-c29a5ffe-rows01-13"; vd="$tmp/20261105-imx8mp-gles3-rows01-13"
mkdir "$x2d" "$vd"
printf '0\tdEQP-GLES3.a.*\t-\t10\t1\t0\t0\t-\tdEQP-GLES3.a.x1\n' > "$x2d/results.tsv"
printf 'slice\n' > "$x2d/README.md"
printf '0\tdEQP-GLES3.a.*\t11\t10\t1\t0\t0\t0\tdEQP-GLES3.a.x1\n' > "$vd/results.tsv"
printf 'slice\n' > "$vd/README.md"
run --commit c29a5ffe84f7d891fc84fe8b151b9eefedbb351d --slices "$x2d" > "$tmp/x2.md"
[ "$(grep -c 'measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL) with tools/board.sh x2' "$tmp/x2.md")" = 3 ] || fail "x2 line per draft"
grep -q 'Campaign commit: c29a5ffe$' "$tmp/x2.md" || fail "x2 8-hex commit"
grep -q 'Records: 20261127-imx8mp-gles3-x2-c29a5ffe-rows01-13$' "$tmp/x2.md" || fail "x2 records"
run --commit abcd1234 --slices "$vd" > "$tmp/vd.md"
run --commit abcd1234 > "$tmp/nos.md"
cmp -s "$tmp/vd.md" "$tmp/nos.md" || fail "Verdin slices changed the text"
if grep -q Armadillo "$tmp/vd.md"; then fail "Verdin draft names the X2"; fi
if run --slices "$x2d" >/dev/null 2>&1; then fail "x2 draft without a commit"; fi
echo "ok: Armadillo-X2 drafts, Verdin text unchanged"

# Hang / abort / WDOG1 event ids: an event title, cause id, glob, entries, back-end area and a record
# placeholder; --events drafts ids without cases from --causes.
printf 'gc-gpu-hang-fbo-srgb-blend\tdEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.*\tGPU hang on entries 13-15 (predicted, 05769bf2): no result (src/vivante/draw.rs:10)\ngc-wdog1-filtering-3d\tdEQP-GLES3.functional.texture.filtering.3d.*\tWDOG1 reset (600 s) on entries 58-66 (predicted)\n' > "$tmp/ev-causes.tsv"
printf 'board-only\tgc-gpu-hang-fbo-srgb-blend\te13\tNO FINISH\tdEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.r8.0\n' > "$tmp/ev.tsv"
python3 -I "$tool" "$tmp/ev.tsv" --causes "$tmp/ev-causes.tsv" --commit abcd1234 --events > "$tmp/ev.md"
grep -q '^## GC7000UL: GPU hang on entries 13-15 (cause gc-gpu-hang-fbo-srgb-blend)$' "$tmp/ev.md" || fail "hang title"
grep -q 'Cause id: gc-gpu-hang-fbo-srgb-blend' "$tmp/ev.md" || fail "hang cause id"
grep -q 'Glob: `dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.\*`' "$tmp/ev.md" || fail "hang glob"
grep -q 'Table entries: e13$' "$tmp/ev.md" || fail "hang entries"
grep -q 'Vivante back-end area: src/vivante/draw.rs' "$tmp/ev.md" || fail "hang area"
grep -q 'Record: <slice record> at <commit>' "$tmp/ev.md" || fail "record placeholder"
grep -q '^## GC7000UL: WDOG1 reset (cause gc-wdog1-filtering-3d)$' "$tmp/ev.md" || fail "wdog1 title"
grep -q 'Table entries: see the slice README' "$tmp/ev.md" || fail "wdog1 without cases"
grep -q 'Vivante back-end area: src/vivante, driver$' "$tmp/ev.md" || fail "default area"
if python3 -I "$tool" "$tmp/ev.tsv" --causes "$tmp/ev-causes.tsv" | grep -q wdog1; then fail "--events off drafts an id without cases"; fi
echo "ok: hang and WDOG1 event drafts"
