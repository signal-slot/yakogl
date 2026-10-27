#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/tree-diff-selftest.sh
#
# Host-only checks of tools/cts/tree-diff.py on the fixtures in
# tools/cts/testdata/tree-diff/ (README.md there says what each holds): a
# regression, a fix, a case missing from B, an accepted NotSupported case,
# each input form, a directory, and bad input. Each check names the exit
# status it expects (0 clean, 1 regression or Fail/Crash/Timeout, 2 bad input)
# and the report lines that must be there. The qpa fixtures are named *-qpa.txt,
# not *.qpa: cts-selftest.sh check 16 runs cts-refusals.py over every .qpa
# under testdata/ and pins its counts.
set -u

HERE=$(dirname "$0")
FIX=$HERE/testdata/tree-diff
TD=$HERE/tree-diff.py
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

# check <label> <want rc> <args...> -- <wanted line>...: run the tool, compare
# its exit status, and look for each wanted line in its stdout+stderr
check() {
    local label=$1 want=$2 args=() out rc w miss=
    shift 2
    while [ $# -gt 0 ] && [ "$1" != -- ]; do args+=("$1"); shift; done
    [ $# -gt 0 ] && shift
    out=$(python3 -I "$TD" "${args[@]}" 2>&1)
    rc=$?
    for w in "$@"; do
        printf '%s\n' "$out" | grep -qF -- "$w" || miss+="${miss:+; }MISSING \"$w\""
    done
    if [ "$rc" -eq "$want" ] && [ -z "$miss" ]; then
        ok "$label (exit $rc)"
    else
        bad "$label: exit $rc, want $want${miss:+; $miss}"
        printf '%s\n' "$out" | sed 's/^/        | /'
    fi
}

echo "== 1. deqp-console pair (epoch and HH:MM:SS time stamps): regression, fix, missing"
check "console pair against the mustpass" 1 \
    --baseline "$FIX/console-a.txt" --new "$FIX/console-b.txt" --mustpass "$FIX/mustpass.txt" -- \
    "A (baseline): 1 file(s), form deqp-console" \
    "B (new run): 1 file(s), form deqp-console" \
    "8 cases, 7 in A's or B's scope, 1 outside both" \
    "1 case(s) with a result are not in the mustpass, ignored" \
    "  A         4    2            1              0                    0     0       0       0" \
    "  B         2    2            2              0                    0     0       0       1" \
    "regressed (Pass in A, not Pass in B): 3" \
    "Pass                 -> Fail                 dEQP-GLES2.functional.treediff.regress_fail" \
    "Pass                 -> Missing              dEQP-GLES2.functional.treediff.missing_case" \
    "Pass                 -> NotSupported         dEQP-GLES2.functional.treediff.accepted_ns" \
    "fixed (not Pass in A, Pass in B): 1" \
    "Fail                 -> Pass                 dEQP-GLES2.functional.treediff.fixed_case" \
    "still fail (Fail/Crash/Timeout/InternalError/ResourceError in A and B): 1" \
    "Fail                 -> Fail                 dEQP-GLES2.functional.treediff.still_fail" \
    "missing from B (a result in A, none in B): 1" \
    "verdict: FAIL (exit 1): 2 Fail/Crash/Timeout in B and 3 regression(s)"
check "console pair, no --mustpass: the union of the cases" 1 \
    --baseline "$FIX/console-a.txt" --new "$FIX/console-b.txt" -- \
    "scope: no --mustpass, the union of the cases in A and B: 8 cases"
check "console pair + --accepted: the accepted regression is marked, the rest still gate" 1 \
    --baseline "$FIX/console-a.txt" --new "$FIX/console-b.txt" --mustpass "$FIX/mustpass.txt" \
    --accepted "$FIX/accepted.txt" -- \
    "dEQP-GLES2.functional.treediff.accepted_ns [accepted]" \
    "accepted: 2 case(s) in scope match" \
    "verdict: FAIL (exit 1): 2 Fail/Crash/Timeout in B and 2 regression(s)"

echo "== 2. statuses pair (NoResult, Lost): an accepted NotSupported case"
check "statuses pair without --accepted: Pass -> NotSupported regresses" 1 \
    --baseline "$FIX/statuses-a.txt" --new "$FIX/statuses-b.txt" --mustpass "$FIX/mustpass.txt" -- \
    "form statuses" \
    "8 cases, 5 in A's or B's scope, 3 outside both" \
    "(1 of Missing lost in capture)" \
    "regressed (Pass in A, not Pass in B): 1" \
    "missing from B (a result in A, none in B): 0" \
    "verdict: FAIL (exit 1): 0 Fail/Crash/Timeout in B and 1 regression(s)"
check "statuses pair + --accepted: clean" 0 \
    --baseline "$FIX/statuses-a.txt" --new "$FIX/statuses-b.txt" --mustpass "$FIX/mustpass.txt" \
    --accepted "$FIX/accepted.txt" -- \
    "Pass                 -> NotSupported         dEQP-GLES2.functional.treediff.accepted_ns [accepted]" \
    "fixed (not Pass in A, Pass in B): 1" \
    "verdict: PASS (exit 0)"

echo "== 3. qpa pair: #terminateTestCaseResult Crash, a case never closed"
check "qpa pair" 1 \
    --baseline "$FIX/a-qpa.txt" --new "$FIX/b-qpa.txt" --mustpass "$FIX/mustpass.txt" -- \
    "form qpa" \
    "8 cases, 8 in A's or B's scope, 0 outside both" \
    "Pass                 -> Crash                dEQP-GLES2.functional.treediff.regress_fail" \
    "Pass                 -> Missing              dEQP-GLES2.functional.treediff.missing_case" \
    "Fail                 -> Pass                 dEQP-GLES2.functional.treediff.fixed_case" \
    "verdict: FAIL (exit 1): 1 Fail/Crash/Timeout in B and 2 regression(s)"

echo "== 4. directories: a tree's */console.txt and a host run's *.qpa"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/a/run-1" "$tmp/b/run-1" "$tmp/empty"
cp "$FIX/console-a.txt" "$tmp/a/run-1/console.txt"
cp "$FIX/console-b.txt" "$tmp/b/run-1/console.txt"
cp "$FIX/b-qpa.txt" "$tmp/b/c_001.qpa"
check "directory input, two forms on one side" 1 \
    --baseline "$tmp/a" --new "$tmp/b" --mustpass "$FIX/mustpass.txt" -- \
    "A (baseline): 1 file(s), form deqp-console" \
    "B (new run): 2 file(s), form deqp-console+qpa" \
    "$tmp/b/c_001.qpa"

echo "== 5. --json"
jout=$(python3 -I "$TD" --baseline "$FIX/console-a.txt" --new "$FIX/console-b.txt" \
    --mustpass "$FIX/mustpass.txt" --accepted "$FIX/accepted.txt" --json 2>&1)
jrc=$?
jcheck=$(printf '%s\n' "$jout" | python3 -I -c '
import json, sys
d = json.load(sys.stdin)
want = [(d["a"]["form"], "deqp-console"), (d["counts"]["A"]["Pass"], 4), (d["counts"]["B"]["Missing"], 1),
        ([e["case"].rsplit(".", 1)[1] for e in d["regressed"]], ["regress_fail", "missing_case", "accepted_ns"]),
        ([e["accepted"] for e in d["regressed"]], [False, False, True]),
        ([e["case"].rsplit(".", 1)[1] for e in d["fixed"]], ["fixed_case"]),
        ([e["case"].rsplit(".", 1)[1] for e in d["still_fail"]], ["still_fail"]),
        ([e["b"] for e in d["missing_from_b"]], ["Missing"]),
        (d["gate"], {"fail_crash_timeout_in_b_outside_accepted": 2, "regressions_outside_accepted": 2}),
        (d["exit"], 1)]
bad = [(g, w) for g, w in want if g != w]
print("ok" if not bad else "got %r" % bad)' 2>&1)
if [ "$jrc" -eq 1 ] && [ "$jcheck" = ok ]; then
    ok "--json: forms, counts, lists, accepted flags and gate (exit $jrc)"
else
    bad "--json: exit $jrc, want 1; $jcheck"
fi

echo "== 6. bad input: exit 2"
check "a file in no per-case form" 2 --baseline "$FIX/bad-input.txt" --new "$FIX/console-b.txt" -- \
    "no per-case form found"
check "the per-row selector form (results.tsv, TALLY)" 2 \
    --baseline "$FIX/results-tsv.txt" --new "$FIX/console-b.txt" -- "is per row, not per case"
check "an unknown StatusCode in a .qpa" 2 --baseline "$FIX/bad-status-qpa.txt" --new "$FIX/console-b.txt" -- \
    "unknown status 'Bogus'"
check "a missing file" 2 --baseline "$FIX/no-such-file.txt" --new "$FIX/console-b.txt" -- \
    "no such file or directory"
check "a directory without result files" 2 --baseline "$tmp/empty" --new "$FIX/console-b.txt" -- \
    "holds no console.txt, */console.txt or *.qpa"
check "an empty --mustpass" 2 --baseline "$FIX/console-a.txt" --new "$FIX/console-b.txt" \
    --mustpass "$FIX/empty-mustpass.txt" -- "names no case"
check "no --new (usage error)" 2 --baseline "$FIX/console-a.txt" -- "required"

echo "== 7. --help and tools/cts/es2-accepted.txt"
check "--help names the inputs, the options and the exit codes" 0 --help -- \
    "deqp-console" "qpa" "statuses" "--mustpass FILE" "--accepted FILE" "--json" \
    "exit status:" "  0  B has no Fail" "  1  otherwise" "  2  bad input"
# every pattern of the accepted list names a mustpass case, and each carries a reason
acc=$(python3 -I - "$HERE/es2-accepted.txt" "$HERE/gles2-mustpass.txt" <<'EOF'
import fnmatch, sys
mp = [l.strip() for l in open(sys.argv[2]) if l.strip()]
n = 0
for l in open(sys.argv[1]):
    if not l.strip() or l.lstrip().startswith("#"):
        continue
    pat, _, why = l.partition("#")
    if not why.strip() or not any(fnmatch.fnmatchcase(c, pat.strip()) for c in mp):
        sys.exit("no reason or no mustpass case: %s" % l.strip())
    n += 1
print(n)
EOF
)
if [ "$acc" = 34 ]; then
    ok "es2-accepted.txt: 34 patterns, each with a reason, each a gles2-mustpass.txt case"
else
    bad "es2-accepted.txt: $acc"
fi

echo
if [ "$fail" -eq 0 ]; then
    echo "== all $pass checks passed"
else
    echo "== $fail check(s) FAILED ($pass passed)" >&2
fi
[ "$fail" -eq 0 ]
