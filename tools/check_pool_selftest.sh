#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Pins tools/check.sh's row pool (--jobs). The same rows give the same log,
# line for line, with one job and with four: the order is the script's, a
# warning and a failure are red, a gap is counted and is not a pass. And a
# row that reads what an earlier row writes (`uses w:NAME` / `r:NAME`) waits
# for it; the control without the `uses` lines shows the reader fails when
# it does not wait, so the pass above is the lane's. And the compiler slots
# (the jobserver) bound the rows at once: with one slot two rows that would
# overlap do not, and the control with many slots shows the probe sees an
# overlap when there is one.
set -u
cd "$(dirname "$0")/.." || exit 2
fails=0
checks=0
mkdir -p build/check
dir=$(mktemp -d build/check/pool-selftest.XXXXXX) || exit 2
trap 'rm -rf "$dir"' EXIT
expect() {
  local label=$1 pattern=$2 out=$3
  checks=$((checks + 1))
  printf '%s\n' "$out" | grep -qE "$pattern" || {
    echo "FAIL $label: no line matches '$pattern'"
    printf '%s\n' "$out" | sed 's/^/    /' | tail -15
    fails=$((fails + 1))
  }
}
demo() {
  rm -f "$dir/lane" "$dir"/slot.*
  env -u YAKOGL_CHECK_JOBS -u MAKEFLAGS -u CARGO_MAKEFLAGS tools/check.sh --jobs "$1" --pool-demo "$dir" ${2-} 2>&1 |
    sed 's/\x1b\[[0-9;]*m//g'
}
# The log without the times, which differ between runs.
untimed() { printf '%s\n' "$1" | sed -E 's/ \[[0-9]+s\]$//; s/ in [0-9]+s;/;/; /compiler slot/d'; }
one=$(demo 1)
four=$(demo 4)
for jobs in 1 4; do
  out=$one
  [ "$jobs" = 4 ] && out=$four
  expect "jobs $jobs: the counts" '^check.sh: 5 ok, 2 failed, 1 gap\(s\)' "$out"
  expect "jobs $jobs: a warning is red" '^FAIL  demo    warns: 1 warning\(s\)' "$out"
  expect "jobs $jobs: a failure is red, its output under it" '^demo output$' "$out"
  expect "jobs $jobs: the reader waited for the writer" '^ok    demo    reads the lane.s file' "$out"
  expect "jobs $jobs: the gate is red" '^check.sh: fix the above' "$out"
done
checks=$((checks + 1))
if [ "$(untimed "$one" | grep -v whole.output)" != "$(untimed "$four" | grep -v whole.output)" ]; then
  echo "FAIL the log with four jobs is not the log with one:"
  diff <(untimed "$one") <(untimed "$four") | head -20
  fails=$((fails + 1))
fi
out=$(demo 4 nolanes)
expect "control: without the lane, the reader does not wait" '^FAIL  demo    reads the lane.s file' "$out"
out=$(YAKOGL_DEMO_AT_ONCE=1 YAKOGL_CHECK_MEM_PER_COMPILER_MB=999999999 demo 4)
expect "one slot: the slot line" '^check.sh: 1 compiler slot\(s\)' "$out"
expect "one slot: slot a ran alone" '^ok    demo    slot a' "$out"
expect "one slot: slot b ran alone" '^ok    demo    slot b' "$out"
out=$(YAKOGL_DEMO_AT_ONCE=1 YAKOGL_CHECK_MEM_PER_COMPILER_MB=1 demo 4)
expect "control: with many slots the probe sees two rows at once" '^2 at once$' "$out"
echo "check_pool_selftest: $checks checks run (planned $checks), $fails failed"
[ "$fails" -eq 0 ]
