#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mali-fail-triage-selftest.sh
#
# Host-only check of tools/cts/mali-fail-triage.py on the fixtures in
# tools/cts/testdata/mali-fail-triage/: two slices (a forecast refusal, its
# sibling variants and a message-only one, an unforecast refusal, a fault chunk
# with loop cases, plain Fails, a lost chunk). The printed draft must equal
# expected.md byte for byte, carry no address or user@host, name each NEW row
# in a draft issue, and --self-test must pass. A slice that cannot be read
# exits 2. Exits 0 only if every comparison matches.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mali-fail-triage.py
DATA=$HERE/testdata/mali-fail-triage
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

run() { python3 -I "$TOOL" --forecast "$DATA/forecast.md" --mali-src "$DATA/mali-src.rs" "$@"; }

got=$(run "$DATA"/slices/* 2>&1)
if [ "$got" = "$(cat "$DATA/expected.md")" ]; then ok "output equals expected.md"; else bad "output differs from expected.md"; fi

if grep -Eq '([0-9]{1,3}\.){3}[0-9]{1,3}|[A-Za-z0-9_.-]+@[A-Za-z0-9_.-]+' <<<"$got"; then bad "draft carries an address or user@host"; else ok "no address or user@host"; fi

new=$(grep -c '\*\*NEW\*\*' <<<"$got")
issues=$(grep -c '^### Draft issue:' <<<"$got")
[ "$new" -gt 0 ] && [ "$new" = "$issues" ] && ok "one draft issue per NEW row ($new)" || bad "NEW rows $new, draft issues $issues"

for pat in 'refused MixedPrecision .*yes' 'refused CompareForm .*yes' 'made-up form is not lowered yet .*yes' \
           'hang-risk: fragment loop .*yes' 'fault chunk: mmu_faults .*NEW' 'refused blend_equation_separate .*NEW'; do
    grep -Eq "$pat" <<<"$got" && ok "row: $pat" || bad "no row: $pat"
done

python3 -I "$TOOL" --forecast "$DATA/forecast.md" "$DATA/nope" >/dev/null 2>&1; rc=$?
[ "$rc" = 2 ] && ok "unreadable slice exits 2" || bad "unreadable slice exited $rc"

python3 -I "$TOOL" --self-test >/dev/null 2>&1; rc=$?
[ "$rc" = 0 ] && ok "--self-test" || bad "--self-test exited $rc"

if [ "$fail" = 0 ]; then echo "mali-fail-triage-selftest: ok"; exit 0; fi
echo "mali-fail-triage-selftest: $fail failed"
exit 1
