#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of pi5-gltest-verdicts.sh (row N8 of docs/notes/rpi5-gate-rows.md):
# its own --selftest (the synthetic console
# tools/testdata/pi5-gltest-synthetic.console.txt gives six verdicts, 59 and 67
# refused by name; three mutants are rejected), the six stdout lines of a plain
# call on the fixture, a console without stamps and with CR line ends, a run
# and a record that disagree, and the usage errors (exit 2). The gate runs this
# file as a row of its own (tools/check_selftests.py). Host only; scratch under
# TMPDIR or the current directory, never /tmp.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/pi5-gltest-verdicts.sh
fix=$here/../testdata/pi5-gltest-synthetic.console.txt
scratch=${TMPDIR:-$PWD}
case "$scratch" in /tmp | /tmp/* | /var/tmp | /var/tmp/*)
  echo "refusing a scratch dir under /tmp; set TMPDIR" >&2
  exit 2
  ;;
esac
tmp=$(mktemp -d "$scratch/pi5-gltest-verdicts-selftest.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
export TMPDIR=$tmp
nfail=0 nok=0
ok() {
  echo "ok: $*"
  nok=$((nok + 1))
}
bad() {
  echo "FAIL: $*" >&2
  nfail=$((nfail + 1))
}

out=$(sh "$tool" --selftest 2>&1)
rc=$?
if [ "$rc" -eq 0 ] && grep -q '^selftest: fixture accepted, 3 mutants rejected' <<< "$out"; then
  ok "--selftest: the fixture accepted, 3 mutants rejected"
else
  bad "--selftest (exit $rc): $out"
fi

lost='an earlier job wrote, and the target has no depth/stencil buffer to keep it in'
want="pi5 gl_test: case 5 scissor: PASS
pi5 gl_test: case 58 clearscis: PASS
pi5 gl_test: case 59 clearmask: REFUSED (the draw reads stencil $lost)
pi5 gl_test: case 66 clearvp: PASS
pi5 gl_test: case 67 fbods: REFUSED (the draw reads depth $lost)
pi5 gl_test: case 102 maskclear: PASS"
out=$(sh "$tool" "$fix" 2> "$tmp/err")
rc=$?
if [ "$rc" -eq 0 ] && [ "$out" = "$want" ]; then
  ok "the fixture: the six lines, exit 0"
else
  bad "the fixture (exit $rc): $out"
fi

# a console with CR line ends and no time stamps reads the same
sed -e 's/^[0-9:]* //' -e 's/$/\r/' "$fix" > "$tmp/crlf.txt"
out=$(sh "$tool" "$tmp/crlf.txt" 2> "$tmp/err")
rc=$?
if [ "$rc" -eq 0 ] && [ "$out" = "$want" ]; then
  ok "no stamps, CR line ends: the same six lines"
else
  bad "no stamps, CR line ends (exit $rc): $out"
fi

# the run and the record disagree on a case: named, exit 1
sed 's/v3d_gl_test: case 5 PASS/v3d_gl_test: case 5 FAIL/' "$fix" > "$tmp/disagree.txt"
out=$(sh "$tool" "$tmp/disagree.txt" 2> "$tmp/err")
rc=$?
if [ "$rc" -eq 1 ] && grep -q '^pi5 gl_test: case 5: MISSING (the run says PASS, the record FAIL)$' <<< "$out"; then
  ok "run and record disagree on case 5: named, exit 1"
else
  bad "run and record disagree (exit $rc): $out"
fi

usage_case() {
  local what=$1 rc
  shift
  sh "$tool" "$@" > /dev/null 2>&1
  rc=$?
  if [ "$rc" -eq 2 ]; then ok "usage error, exit 2: $what"; else bad "$what: exit $rc, not 2"; fi
}
usage_case "no argument"
usage_case "an unknown flag" --no-such-flag
usage_case "two arguments" "$fix" "$fix"
usage_case "a file that is not there" "$tmp/no-such-file"

echo "pi5-gltest-verdicts-selftest: $nok ok, $nfail failed"
[ "$nfail" -eq 0 ]
