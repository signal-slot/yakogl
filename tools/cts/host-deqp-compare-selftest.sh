#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-check for host-deqp-run.sh --compare (tools/cts/README.md 2b).
#
#   tools/cts/host-deqp-compare-selftest.sh      # host only, builds nothing
#
# Three tiny run directories of one chunk each (the compare reads c_*.qpa),
# written inline below:
#
#   old/    the <Result StatusCode= lines unindented, the shape the anchored
#           pattern always matched (as in testdata/gles3-notpass/logs/a.qpa)
#   new/    the same three cases, the Result lines indented the way dEQP
#           writes them (a space per nesting level, as in
#           testdata/by-message.qpa; bravo's with a tab to pin that any
#           leading whitespace parses, not just a space), and bravo changing
#           Fail -> Pass -- the change is only visible if BOTH shapes parse,
#           so this check is red for the pre-2026-11-12 anchored parser: it
#           read new/ as all-Incomplete and printed "0 case(s) changed".
#   nores/  a chunk that began its cases but wrote no Result line (a run cut
#           short): zero parsed results, which --compare must refuse loudly
#           instead of reporting "no changes".
#
# Checks: (1) --compare-only=old,new exits 0 and reports the one status
# change, and the totals count the indented statuses; (2) a nores side on
# either end exits non-zero with a message and prints no comparison. The
# end-to-end --compare= flag runs the same compare_runs function after a
# run, so these fixtures exercise it without dEQP or a library build.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
RUN=$HERE/host-deqp-run.sh
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

n_ok=0 n_bad=0
ok()  { echo "  ok $*"; n_ok=$((n_ok + 1)); }
bad() { echo "  FAIL $*"; n_bad=$((n_bad + 1)); }

mkdir -p "$TMP/old" "$TMP/new" "$TMP/nores"

cat > "$TMP/old/c_000.qpa" <<'EOF'
#startSuite Nov 12 2026 12:00:00
#startGroup.default gles3
#beginTestCaseResult dEQP-selftest.alpha
<TestCaseResult CasePath="dEQP-selftest.alpha" CaseType="SelfValidate" Version="0.3.4">
<Result StatusCode="Pass">detail for alpha</Result>
</TestCaseResult>
#endTestCaseResult
#beginTestCaseResult dEQP-selftest.bravo
<TestCaseResult CasePath="dEQP-selftest.bravo" CaseType="SelfValidate" Version="0.3.4">
<Result StatusCode="Fail">detail for bravo</Result>
</TestCaseResult>
#endTestCaseResult
#beginTestCaseResult dEQP-selftest.charlie
<TestCaseResult CasePath="dEQP-selftest.charlie" CaseType="SelfValidate" Version="0.3.4">
<Result StatusCode="NotSupported">detail for charlie</Result>
</TestCaseResult>
#endTestCaseResult
#terminateTestCaseResult Passed
EOF

# The same log as dEQP writes it: every Result line indented (alpha and
# charlie with a space, bravo with a tab), and bravo now passing.
printf '%s\n' \
    '#startSuite Nov 12 2026 12:00:00' \
    '#startGroup.default gles3' \
    '#beginTestCaseResult dEQP-selftest.alpha' \
    '<TestCaseResult CasePath="dEQP-selftest.alpha" CaseType="SelfValidate" Version="0.3.4">' \
    ' <Result StatusCode="Pass">detail for alpha</Result>' \
    '</TestCaseResult>' \
    '#endTestCaseResult' \
    '#beginTestCaseResult dEQP-selftest.bravo' \
    '<TestCaseResult CasePath="dEQP-selftest.bravo" CaseType="SelfValidate" Version="0.3.4">' \
    "$(printf '\t')<Result StatusCode=\"Pass\">detail for bravo</Result>" \
    '</TestCaseResult>' \
    '#endTestCaseResult' \
    '#beginTestCaseResult dEQP-selftest.charlie' \
    '<TestCaseResult CasePath="dEQP-selftest.charlie" CaseType="SelfValidate" Version="0.3.4">' \
    ' <Result StatusCode="NotSupported">detail for charlie</Result>' \
    '</TestCaseResult>' \
    '#endTestCaseResult' \
    '#terminateTestCaseResult Passed' \
    > "$TMP/new/c_000.qpa"

cat > "$TMP/nores/c_000.qpa" <<'EOF'
#startSuite Nov 12 2026 12:00:00
#startGroup.default gles3
#beginTestCaseResult dEQP-selftest.alpha
<TestCaseResult CasePath="dEQP-selftest.alpha" CaseType="SelfValidate" Version="0.3.4">
</TestCaseResult>
#endTestCaseResult
#beginTestCaseResult dEQP-selftest.bravo
<TestCaseResult CasePath="dEQP-selftest.bravo" CaseType="SelfValidate" Version="0.3.4">
</TestCaseResult>
#endTestCaseResult
#terminateTestCaseResult InternalError
EOF

# The fixtures are the two shapes the check claims (guards against the
# inline logs above drifting without any check noticing).
if grep -q '^<Result StatusCode=' "$TMP/old/c_000.qpa" \
   && grep -q '^ <Result StatusCode=' "$TMP/new/c_000.qpa" \
   && grep -q "$(printf '^\t<Result StatusCode=')" "$TMP/new/c_000.qpa" \
   && ! grep -q '<Result StatusCode=' "$TMP/nores/c_000.qpa"; then
    ok "fixtures: unindented, indented (space and tab) and result-free logs as intended"
else
    bad "fixtures: not the shapes the compare needs to see"
fi

# 1. old vs new: exit 0, the one status change, totals after it.
out=$(bash "$RUN" --compare-only="$TMP/old,$TMP/new" 2>&1); rc=$?
if [ "$rc" -eq 0 ] \
   && printf '%s\n' "$out" | grep -qF 'dEQP-selftest.bravo: Fail -> Pass' \
   && printf '%s\n' "$out" | grep -qF '1 case(s) changed'; then
    ok "--compare-only reports the Fail -> Pass between the two shapes"
else
    bad "--compare-only old,new (rc $rc): $(printf '%s\n' "$out" | head -4 | paste -sd';' -)"
fi
# The totals must count the indented side's statuses (Pass: 1 old, 2 new) --
# the anchored parser saw only Incomplete there.
if printf '%s\n' "$out" | grep -qE 'Pass +1 +2'; then
    ok "totals read the indented results: Pass 1 old, 2 new"
else
    bad "totals do not count the indented Pass values: $(printf '%s\n' "$out" | grep Pass | paste -sd';' -)"
fi

# 2. A side with zero parsed results is an error naming the counts, with no
# comparison printed (this is the silent "no changes" the control run hit).
for side in "$TMP/old,$TMP/nores" "$TMP/nores,$TMP/new"; do
    out=$(bash "$RUN" --compare-only="$side" 2>&1); rc=$?
    # The message names both counts: "parses to <n> case result(s), <other>
    # to <m>;" -- the zero side's count is followed by " case result" or by
    # the ';' that ends the line.
    if [ "$rc" -ne 0 ] && printf '%s\n' "$out" | grep -qE 'to 0(;| case result)' \
       && ! printf '%s\n' "$out" | grep -q 'case(s) changed'; then
        ok "empty side refused, exit $rc: ${side//$TMP\//}"
    else
        bad "empty side accepted (rc $rc): ${side//$TMP\//}: $(printf '%s\n' "$out" | head -3 | paste -sd';' -)"
    fi
done

if [ "$n_bad" -gt 0 ]; then
    echo "host-deqp-compare-selftest: $n_ok ok, $n_bad FAILED"
    exit 1
fi
echo "host-deqp-compare-selftest: $n_ok checks passed"
