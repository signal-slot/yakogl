#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/cts-selftest.sh
#
# Host-only checks of the tools/cts parsers. No board, no GPU, no network.
# Each check says what it feeds in and what it expects, so a failure can be
# read rather than guessed at.
#
# Checks 1-3 tested the wedge detectors of the Linux dEQP runner
# (cts-board-run.sh's fence-timeout pattern and exit-status escalation,
# cts-run.sh's one-case-failure recovery). Those scripts drove dEQP over
# /dev/pvrgpu and /dev/gcgpu, whose kernel modules are retired
# (driver/tools/pvrlab/README.md, driver/tools/gclab/README.md), so the
# checks went with them; they are in the history at 49058ba4. Check 16 keeps
# its number so old logs still read.
set -u

HERE=$(dirname "$0")
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

echo
# ---- 16: cts-refusals.py parses the message format with a token ------------
#
# The format changed under this tool once (fc88b41 added `'<tok>' :`) and the
# tool went silently EMPTY rather than failing, because dEQP escapes the
# token's quotes as `&apos;` and the pattern stopped at `&`. A sweep run on
# that would have cost 40 minutes of board time and produced one nameless row.
# This uses the fixture in testdata/ rather than the `check` helper above,
# which compares a decide() verdict and takes different arguments.
echo
echo "== 16. cts-refusals.py on the tokened format (testdata fixture)"
rout=$(python3 "$HERE/cts-refusals.py" --cases 1 "$HERE/testdata" 2>&1)
for want in "assignment to this is not lowered yet" \
            "the code generator refused this fragment shader: Select(OutOfScratch)" \
            "on cases dEQP FAILED (real gaps)        : 4" \
            "on cases dEQP NOT SUPPORTED (excused)   : 1" \
            "on cases dEQP PASSED (correct refusals) : 1" \
            "Usc(Record(MatrixAttrib { index: 0 }))" \
            "no implicit conversion in GLSL ES 1.00"; do
    case "$rout" in
        *"$want"*) ok   "refusals: reports \"$want\"" ;;
        *)         bad  "refusals: MISSING \"$want\"" ;;
    esac
done
# the per-construct split: one message, two tokens
if printf '%s\n' "$rout" | grep -qE '^ +1 +\.$' &&
   printf '%s\n' "$rout" | grep -qE '^ +1 +\[\]$'; then
    ok  "refusals: splits one message by construct (. and [])"
else
    bad "refusals: did NOT split by construct -- the token was lost again"
fi

if [ "$fail" -eq 0 ]; then
    echo "== all checks passed"
else
    echo "== $fail check(s) FAILED" >&2
fi
[ "$fail" -eq 0 ]
