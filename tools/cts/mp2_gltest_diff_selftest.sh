#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_gltest_diff_selftest.sh
#
# Host-only check of tools/cts/mp2-gltest-diff.py on synthetic gl_test logs in
# a temporary directory: every class (new, flaky, expected, stale, skipped,
# missing), both input forms, the exit codes, and the real baseline files.
# Nothing here is a real capture.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mp2-gltest-diff.py
D=$(mktemp -d "${TMPDIR:-/tmp}/mp2-gltest.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }

# synthetic baseline: set 1 2 3 4 5; 5 skipped on the i.MX8MP; 2 and 3 flaky
cat >"$D/lib.rs" <<'EOF'
pub static SET_ORDER: [i32; 5] = [
    1, 2, 3, 4, 5,
];
EOF
cat >"$D/RESULTS.md" <<'EOF'
| Board | glmark2-es2 | gl_test | GPU resets |
|---|---|---|---|
| i.MX8MP | 42 | 4 of 4 run, case 5 skipped (#4); cases 2 and 3 nondeterministic (#5) | 0 |
EOF
printf '10  # pending Q11: wide\n230 # pending Q11: wide\n' >"$D/exp.txt"
ARGS=(--results "$D/RESULTS.md" --set "$D/lib.rs" --expected "$D/exp.txt")
run() { python3 -I "$TOOL" "${ARGS[@]}" "$@" 2>&1; }

# clean: 1..4 pass, 5 skipped, 10 and 230 fail (expected)
cat >"$D/clean.log" <<'EOF'
13:00:01 gl_test: case 1 uniform: PASS
gl_test: case 2 b: PASS
gl_test: case 3 c: PASS
gl_test: case 4 d: PASS
gl_test: case 5 skipped: ES 3.00 source
gl_test: case 10 prims: FAIL
gl_test: case 230 flat_lines: FAIL
EOF
out=$(run "$D/clean.log"); rc=$?
chk "clean: exit 0" test $rc -eq 0
chk "clean: summary" grep -qx 'mp2-gltest-diff: new 0, flaky 0, expected 2, missing 0' <<<"$out"

# new failure 4, flaky 2, stale 230 passes, 10 TIMEOUT expected, 1 missing,
# 5 now runs (i.MX8MP only skip), 3 skipped by the DK? no: 3 PASS
cat >"$D/bad.log" <<'EOF'
gl_test: case 2 b: FAIL
gl_test: case 3 c: PASS
gl_test: case 4 d: FAIL
gl_test: case 5 e: PASS
gl_test: case 10 prims: TIMEOUT
gl_test: case 230 flat_lines: PASS
EOF
out=$(run "$D/bad.log"); rc=$?
chk "bad: summary" grep -qx 'mp2-gltest-diff: new 1, flaky 1, expected 1, missing 1' <<<"$out"
chk "bad: exit 1" test $rc -eq 1
chk "bad: new is 4" bash -c "grep -A1 'new failures (DK only): 1' <<<'$out' | grep -qx 4"
chk "bad: flaky is 2" bash -c "grep -A1 'flaky on the i.MX8MP: 1' <<<'$out' | grep -qx 2"
chk "bad: stale 230" bash -c "grep -A1 'stale): 1' <<<'$out' | grep -qx 230"
chk "bad: missing 1" bash -c "grep -A1 'no verdict: 1' <<<'$out' | grep -qx 1"
chk "bad: i.MX8MP-only skip 5" grep -qx '5 (i.MX8MP only)' <<<"$out"
run --report-only "$D/bad.log" >/dev/null; chk "bad: --report-only exits 0" test $? -eq 0

# result list form, DK-only skip, merge (last wins)
printf '1 PASS\n2 PASS\n3 PASS\n4 SKIP\n5 SKIP\n' >"$D/list.txt"
printf '1 FAIL\n' >"$D/list2.txt"
out=$(run "$D/list.txt"); rc=$?
chk "list: DK-only skip 4" grep -qx '4 (DK only)' <<<"$out"
chk "list: exit 0" test $rc -eq 0
out=$(run "$D/list.txt" "$D/list2.txt"); rc=$?
chk "merge: later input wins, 1 new" grep -q 'new 1,' <<<"$out"
chk "merge: exit 1" test $rc -eq 1

# a skipped-only log misses nothing it names; an empty log misses all 5
: >"$D/empty.log"
out=$(run "$D/empty.log"); rc=$?
chk "empty: missing 5" bash -c "grep -q 'missing 5\$' <<<'$out'"
chk "empty: exit 1" test $rc -eq 1

# a case that skips itself prints its own line (case 16's real wording): a
# verdict, not "missing"; a case with no line at all stays missing
cat >"$D/self.log" <<'EOF'
gl_test: case 1 a: PASS
gl_test: case 2 b: PASS
gl_test: case 3 c: PASS
gl_test: case 4 d: PASS
gl_test: case 5 msaa: 1x half done; SKIP the 4x half (no 4x EGL config offered)
EOF
out=$(run "$D/self.log"); rc=$?
chk "self-skip the i.MX8MP skips too: missing 0" grep -qx 'mp2-gltest-diff: new 0, flaky 0, expected 0, missing 0' <<<"$out"
chk "self-skip: exit 0" test $rc -eq 0
# the i.MX8MP skips 5; a self-skip of case 4 on the DK alone is a skip difference
sed 's/case 4 d: PASS/case 4 d: 1x half done; SKIP the 4x half/' "$D/self.log" >"$D/self4.log"
out=$(run "$D/self4.log")
chk "self-skip DK only: reported 4 (DK only)" grep -qx '4 (DK only)' <<<"$out"
chk "self-skip DK only: not missing" grep -q 'missing 0$' <<<"$out"
# no line at all for case 5: still missing
grep -v 'case 5' "$D/self.log" >"$D/noline.log"
out=$(run "$D/noline.log"); rc=$?
chk "no line at all: missing 1" grep -q 'missing 1$' <<<"$out"
chk "no line at all: exit 1" test $rc -eq 1
# a verdict line wins over a self-skip sub-line, in either order
printf 'gl_test: case 4 d: x; SKIP sub\ngl_test: case 4 d: FAIL\n' >"$D/w.log"
chk "verdict after sub-skip wins" grep -q 'new 1,' <<<"$(run "$D/w.log")"
printf 'gl_test: case 4 d: FAIL\ngl_test: case 4 d: x; SKIP sub\n' >"$D/w2.log"
chk "sub-skip after verdict ignored" grep -q 'new 1,' <<<"$(run "$D/w2.log")"

# error exits
run "$D/nonexistent" >/dev/null; chk "missing input exits 2" test $? -eq 2
python3 -I "$TOOL" --results "$D/lib.rs" "$D/clean.log" >/dev/null 2>&1
chk "bad RESULTS.md exits 2" test $? -eq 2

# the real files: expected list is the 7 wide-line cases; set and baseline parse
out=$(python3 -I "$TOOL" --report-only "$D/clean.log" 2>&1); rc=$?
chk "real baseline parses (exit 0 report-only)" test $rc -eq 0
chk "real baseline: 50 cases missing from a 1-case log" grep -q 'missing [0-9]*$' <<<"$out"
chk "real expected list has 7 cases" test "$(grep -c '^[0-9]' "$HERE/mp2-gltest-expected.txt")" -eq 7
chk "every expected line has a reason" test -z "$(grep '^[0-9]' "$HERE/mp2-gltest-expected.txt" | grep -v '#')"

echo "mp2_gltest_diff selftest: $pass ok, $fail failed"
[ "$fail" -eq 0 ]
