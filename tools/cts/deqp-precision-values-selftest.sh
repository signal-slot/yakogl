#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of deqp-precision-values.py on hand-made fixtures in dEQP's real
# message format (tools/cts/testdata/deqp-precision-values/): a Pass, a
# QualityWarning with an out-of-interval value, a Fail with a NaN, a
# truncated log and a board console log (qpa: / qpa+ echo).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/deqp-precision-values.py"
fx="$here/testdata/deqp-precision-values"
fail() { echo "deqp-precision-values-selftest: FAIL: $*" >&2; exit 1; }

# has FILE-ARG TEXT...: the tool's output on the fixture holds each TEXT line
has() {
  local f=$1 out; shift
  out=$(python3 -I "$tool" "$fx/$f") || fail "$f: exit status $?"
  for t in "$@"; do
    printf '%s\n' "$out" | grep -qF -- "$t" || fail "$f: output lacks: $t"
  done
}
lacks() {
  local f=$1 t=$2 out
  out=$(python3 -I "$tool" "$fx/$f") || fail "$f: exit status $?"
  if printf '%s\n' "$out" | grep -qF -- "$t"; then fail "$f: output has: $t"; fi
}

# QualityWarning: a non-NaN value outside the interval
has qw.qpa "outcome: QualityWarning" "in0 = vec2(0x1.000000p-130, -0x1.800000p1)" \
  "out0: 1 of 2 element(s) out of bound" \
  "out0[0] got 0.0 expected [0x1.000000p0, 0x1.000000p0]  OUT"
lacks qw.qpa "out0[1]"

# Fail with a NaN, in modf's second output too
has nan-fail.qpa "outcome: Fail" "out0[1] got NaN expected [-0x1.000000p-1, -0x1.000000p-1]  NaN" \
  "out1: 0 of 2 element(s) out of bound"

# truncated: whole sample kept, the cut case marked
has truncated.qpa "precision.tanh.highp_fragment.scalar" "TRUNCATED" "outcome: none (no Result)" \
  "1 not comparable" "sample 1: Fail (cut)"

# console echo, a line split across qpa+
has console.log "outcome: QualityWarning" "out0 got 0.0 expected [0x1.000000p0, 0x1.000000p0]  OUT"

# --all-elements lists the elements that are in bound as well
out=$(python3 -I "$tool" --all-elements "$fx/nan-fail.qpa")
printf '%s\n' "$out" | grep -qF "out1[0] got 0x1.000000p1 expected [0x1.000000p1, 0x1.000000p1]  ok" \
  || fail "--all-elements lacks the in-bound element"

# a Pass is not listed: exit 2 and a message when nothing is left
if python3 -I "$tool" "$fx/pass.qpa" >/dev/null 2>"${TMPDIR:-/tmp}/dpv.$$"; then
  rm -f "${TMPDIR:-/tmp}/dpv.$$"; fail "pass.qpa: expected exit 2"
fi
rm -f "${TMPDIR:-/tmp}/dpv.$$"
out=$(python3 -I "$tool" --include-pass "$fx/pass.qpa")
printf '%s\n' "$out" | grep -qF "outcome: Pass" || fail "--include-pass lacks the Pass case"

# an unreadable file is exit 2
if python3 -I "$tool" "$fx/no-such.qpa" 2>/dev/null; then fail "missing file: expected exit 2"; fi

echo "deqp-precision-values-selftest: ok"
