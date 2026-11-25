#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of mali-sweep-tally.py: the 1a22bd38 record's tally, the diff of
# 2026-11-t6-sweep336 against it, a broken summary, and identical files.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/mali-sweep-tally.py"
old="$here/log/2026-11-t6-sweep-1a22bd38/sweep-1.verdicts.txt"
mid="$here/log/2026-11-t6-sweep336/sweep-1.verdicts.txt"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mali-sweep-tally.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "mali-sweep-tally-selftest: FAIL: $*" >&2; exit 1; }
run() { python3 -I "$tool" "$@"; }
tab=$(printf '\t')

out=$(run "$old") || fail "1a22bd38 record rejected"
want="panthor-gltest: 256 case(s) run: FAIL 3, NO-CASE 73, PASS 165, REFUSED 6, SKIP 9"
printf "%s\n" "$out" | grep -qxF "$want" || fail "tally is not '$want'"

out=$(run --against "$old" "$mid") || fail "diff of sweep336 against 1a22bd38 failed"
printf "%s\n" "$out" | grep -q '^Changed cases: [1-9]' || fail "sweep336 shows no changed case"
printf "%s\n" "$out" | grep -qx "NO-CASE${tab}73${tab}80${tab}+7" || fail "NO-CASE delta is not 73 -> 80"
out=$(run --markdown --against "$old" "$mid") || fail "markdown diff failed"
printf "%s\n" "$out" | grep -qF '| verdict | old | new | delta |' || fail "markdown delta table missing"

sed 's/^\(panthor-gltest: 256 case(s) run:\) FAIL 3,/\1 FAIL 4,/' "$old" >"$tmp/broken.txt"
if run "$tmp/broken.txt" >/dev/null 2>&1; then fail "a broken summary was accepted"; fi
grep -v 'case(s) run' "$old" >"$tmp/nosum.txt"
if run "$tmp/nosum.txt" >/dev/null 2>&1; then fail "a missing summary was accepted"; fi

cp "$old" "$tmp/same.txt"
out=$(run --against "$old" "$tmp/same.txt") || fail "identical files failed"
printf "%s\n" "$out" | grep -q '^Changed cases: 0' || fail "identical files show changed cases"
printf "%s\n" "$out" | grep -q '^New cases.*: 0$' || fail "identical files show new cases"
printf "%s\n" "$out" | grep -q '^Removed cases.*: 0$' || fail "identical files show removed cases"
if printf "%s\n" "$out" | grep -q "${tab}[+-][1-9]"; then fail "identical files show a class delta"; fi
echo "mali-sweep-tally-selftest: ok"
