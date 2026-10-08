#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/tree-diff-selftest.sh
#
# Host-only checks of tools/cts/tree-diff.py on the fixtures in
# tools/cts/testdata/tree-diff/. No board, no GPU, no network. They cover what
# the diff is for -- a regression, a fix, a case missing from B, an accepted
# NotSupported outcome -- the three per-case input forms the runners produce
# (deqp console, .qpa, statuses), bad input, --json, and the exit statuses 0,
# 1 and 2.
#
# This script does not wire itself into tools/check.sh or cts-selftest.sh:
# those files are where the other tools/cts selftests are discovered, and this
# item may not edit them. The handoff names the step line to add there.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
TD=$HERE/tree-diff.py
FIX=$HERE/testdata/tree-diff
n=0 fail=0
ok()  { n=$((n+1)); printf '  ok    %s\n' "$1"; }
bad() { n=$((n+1)); fail=$((fail+1)); printf '  FAIL  %s\n' "$1"; }
run() { out=$(python3 "$TD" "$@" 2>&1); rc=$?; }
want_rc() { [ "$rc" = "$1" ] && ok "exit $1: $2" || bad "exit for $2: wanted $1, got $rc"; }
want() { case $out in *"$1"*) ok "says: $1";; *) bad "missing: $1"; printf '%s\n' "$out" | sed 's/^/          | /';; esac; }
want_n() { c=$(printf '%s\n' "$out" | grep -cF -- "$1"); [ "$c" = "$2" ] && ok "says $2x: $1" || bad "wanted $2x '$1', got $c"; }
want_pair() { # a listed line naming this case with this status change
	if printf '%s\n' "$out" | grep -F -- "$1" | grep -qF -- "$2"; then ok "$1 $2"; else bad "no line with '$1' and '$2'"; fi
}

echo "== 1. console form: a regression, a fix, a case missing from B, exit 1"
# console-a -> console-b: regress_fail Pass->Fail, fixed_case Fail->Pass,
# missing_case Pass->(absent), accepted_ns Pass->NotSupported.
run --baseline "$FIX/console-a.txt" --new "$FIX/console-b.txt" --mustpass "$FIX/mustpass.txt"
want_rc 1 "console pair"
want_n "form deqp-console" 2
want "6 cases, 6 in scope, 0 outside both sides' case filters"
want "regressed (Pass in A -> anything else in B): 3"
want_pair dEQP-GLES2.functional.treediff.regress_fail "-> Fail"
want "fixed (not Pass in A -> Pass in B): 1"
want_pair dEQP-GLES2.functional.treediff.fixed_case "-> Pass"
want "still fail (Fail/Crash/Timeout/InternalError/ResourceError in A and in B): 0"
want "missing from B (a result in A, none in B): 1"
want_pair dEQP-GLES2.functional.treediff.missing_case "-> Missing"
want "verdict: FAIL (exit 1): 3 regression(s) outside the accepted list, 1 Fail/Crash/Timeout in B outside it"

echo "== 2. --accepted: the accepted regression is listed but out of the gate"
run --baseline "$FIX/console-a.txt" --new "$FIX/console-b.txt" --mustpass "$FIX/mustpass.txt" \
	--accepted "$FIX/accepted.txt"
want_rc 1 "console pair with --accepted"
want "accepted: 2 case(s) in scope match"
want_pair dEQP-GLES2.functional.treediff.accepted_ns "[accepted]"
want "verdict: FAIL (exit 1): 2 regression(s) outside the accepted list, 1 Fail/Crash/Timeout in B outside it"

echo "== 3. --accepted flips the verdict when the only regression is accepted"
# mustpass-acc.txt narrows the universe to same_pass + accepted_ns, so
# accepted_ns is the only regression and B has no Fail/Crash/Timeout.
run --baseline "$FIX/console-a.txt" --new "$FIX/console-b.txt" --mustpass "$FIX/mustpass-acc.txt"
want_rc 1 "narrow pair without --accepted"
want "verdict: FAIL (exit 1): 1 regression(s) outside the accepted list, 0 Fail/Crash/Timeout in B outside it"
run --baseline "$FIX/console-a.txt" --new "$FIX/console-b.txt" --mustpass "$FIX/mustpass-acc.txt" \
	--accepted "$FIX/accepted.txt"
want_rc 0 "narrow pair with --accepted"
want "verdict: PASS (exit 0): no Fail/Crash/Timeout in B outside the accepted list, no regression outside it"

echo "== 4. statuses form (deqp-uart-tally --statuses output): fixes only, exit 0"
run --baseline "$FIX/statuses-a.txt" --new "$FIX/statuses-b.txt" --mustpass "$FIX/mustpass-clean.txt"
want_rc 0 "statuses pair"
want_n "form statuses" 2
want "fixed (not Pass in A -> Pass in B): 2"
want_pair dEQP-GLES2.functional.treediff.new_pass "-> Pass" # NoResult in A -> Pass in B
want "regressed (Pass in A -> anything else in B): 0"

echo "== 5. qpa form: #terminateTestCaseResult Crash gates, exit 1"
run --baseline "$FIX/a.qpa" --new "$FIX/b.qpa" --mustpass "$FIX/mustpass-qpa.txt"
want_rc 1 "qpa pair"
want_n "form qpa" 2
want "fixed (not Pass in A -> Pass in B): 1"
want_pair dEQP-GLES2.functional.treediff.qpa_crash "-> Crash"
want "verdict: FAIL (exit 1): 1 regression(s) outside the accepted list, 1 Fail/Crash/Timeout in B outside it"

echo "== 5b. directory input: a tree directory's *.qpa files, last file wins"
run --baseline "$FIX/a.qpa" --new "$FIX" --mustpass "$FIX/mustpass-qpa.txt"
want_rc 1 "qpa directory input"
want "2 file(s), form qpa"
want_pair dEQP-GLES2.functional.treediff.qpa_crash "-> Crash"

echo "== 6. bad input: exit 2"
run --baseline "$FIX/bad-input.txt" --new "$FIX/console-b.txt"
want_rc 2 "a file with no recognized per-case form"
want "no recognized per-case form"
run --baseline "$FIX/no-such-file.txt" --new "$FIX/console-b.txt"
want_rc 2 "a missing file"
want "no such file"

echo "== 7. --json: the machine-readable form"
json=$(python3 "$TD" --baseline "$FIX/console-a.txt" --new "$FIX/console-b.txt" \
	--mustpass "$FIX/mustpass.txt" --json 2>&1); rc=$?
want_rc 1 "--json exit status"
if printf '%s\n' "$json" | python3 -c 'import json,sys
d = json.loads(sys.stdin.read())
assert d["counts"]["A"]["Pass"] == 4 and d["counts"]["A"]["Missing"] == 0
assert d["counts"]["B"]["Pass"] == 2 and d["counts"]["B"]["NotSupported"] == 2
assert d["counts"]["B"]["Missing"] == 1
assert len(d["regressed"]) == 3 and len(d["fixed"]) == 1 and len(d["missing_from_b"]) == 1
assert d["gate"]["regressions_outside_accepted"] == 3 and d["exit"] == 1'; then
	ok "--json counts, lists and gate"
else
	bad "--json counts, lists and gate"
fi

echo "== 8. no --mustpass: the universe is the union of the two sets"
run --baseline "$FIX/console-a.txt" --new "$FIX/console-b.txt"
want_rc 1 "console pair without --mustpass"
want "scope: the union of the cases seen in A and B: 6 cases"

echo "== 9. committed .qpa records (testdata/refusals-*.qpa): a real per-case diff"
run --baseline "$HERE/testdata/refusals-text-and-notsupported.qpa" \
	--new "$HERE/testdata/refusals-new-format.qpa"
want_rc 1 "committed refusals pair"
want_n "form qpa" 2
want "fixed (not Pass in A -> Pass in B): 1" # shaders.neg, Missing in A -> Pass in B
want "missing from B (a result in A, none in B): 2" # mat4 and the NotSupported loop case
want "verdict: FAIL (exit 1): 0 regression(s) outside the accepted list, 3 Fail/Crash/Timeout in B outside it"

echo "tree-diff-selftest: $n checks, $fail failed"
[ "$fail" = 0 ] || exit 1
