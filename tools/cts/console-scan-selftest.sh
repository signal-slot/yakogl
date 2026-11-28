#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of console-scan.sh: a synthetic slice of one clean row, one HWR
# row and one watchdog row, in $TMPDIR. No board, no addresses.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
scan=$here/console-scan.sh
w=$(mktemp -d "${TMPDIR:-/tmp}/cscan-st.XXXXXX")
trap 'rm -rf "$w"' EXIT
sha=0123abcd
s=$w/log/2026-11-01-am62p-gles3-$sha-rows01-03
mkdir -p "$s"
cat >"$w/am62p-gles3-tree.txt" <<'T'
# image A: x
dEQP-GLES3.functional.a.*
# image B: x
dEQP-GLES3.functional.b.*
# image C: x
dEQP-GLES3.functional.c.*,dEQP-GLES3.functional.d.*
T
printf '00:00:01 Test case x\n00:00:02 deqp: summary\n' >"$s/A.console.txt"
printf '00:00:01 Test case x\n00:00:09 render: HWR during fence 12 (counter 0 -> 1)\n' >"$s/B.console.txt"
printf '00:00:01 Test case x\n00:09:59 rti_arm: feeding bound of 600 s over after 600 feeds; the RTI resets the SoC\n' >"$s/C.console.txt"
{
  echo "board.sh am62p A rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=x resets=0"
  echo "board.sh am62p B rc=0 verdict=done_with_resets boot=uboot-mmc row boot_verdict=done result=x resets=1"
  echo "board.sh am62p C rc=0 verdict=no_reset boot=uboot-mmc row boot_verdict=no_reset result=none resets=absent"
} >"$s/verdicts.txt"

fail=0
check() { # check NAME WANT_RC TREE
  local out rc=0
  out=$(CONSOLE_SCAN_LOGDIR=$w/log "$scan" "$3" "$sha") || rc=$?
  if [ "$rc" != "$2" ]; then echo "FAIL $1: rc $rc, want $2"; fail=1; fi
  printf '%s\n' "$out" >"$w/$1.out"
}
# the three rows together: flagged, rc 1
check all 1 "$w/am62p-gles3-tree.txt"
grep -q '^A .*console=yes hwr=0 fence=0 wdog=0 verdict=done$' "$w/all.out" || { echo "FAIL: clean row line"; fail=1; }
grep -q '^B .*hwr=1 fence=0 wdog=0 verdict=done_with_resets  FLAG$' "$w/all.out" || { echo "FAIL: hwr row line"; fail=1; }
grep -q '^C .*hwr=0 fence=0 wdog=1 verdict=no_reset  FLAG$' "$w/all.out" || { echo "FAIL: watchdog row line"; fail=1; }
grep -q 'rows 3  flagged 2' "$w/all.out" || { echo "FAIL: summary"; fail=1; }
# each fixture alone
for l in A B C; do
  printf '# image %s: x\ndEQP-GLES3.functional.x.*\n' "$l" >"$w/one-$l-gles3-tree.txt"
  # stem "one-X-gles3" -> set "one-X-gles3"; reuse the slice via a symlinked name
  mkdir -p "$w/log2"
  ln -sfn "$s" "$w/log2/2026-11-01-one-$l-gles3-$sha"
done
for l in A B C; do
  want=1; [ "$l" = A ] && want=0
  rc=0
  CONSOLE_SCAN_LOGDIR=$w/log2 "$scan" "$w/one-$l-gles3-tree.txt" "$sha" >"$w/one-$l.out" || rc=$?
  [ "$rc" = "$want" ] || { echo "FAIL: row $l alone rc $rc, want $want"; fail=1; }
done
if [ "$fail" = 0 ]; then echo "console-scan-selftest: OK"; exit 0; fi
echo "console-scan-selftest: FAIL"
exit 1
