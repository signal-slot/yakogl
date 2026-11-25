#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   bash tools/cts/t6-uboot-deqp-compare-selftest.sh
#
# Host-only check of tools/cts/t6-uboot-deqp-compare.py on synthetic inputs,
# and (when present) on the real 8ff01055 slices as all three inputs.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/t6-uboot-deqp-compare.py
D=$(mktemp -d "${TMPDIR:-/tmp}/t6-udc.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
chk() { l=$1; shift; if "$@"; then echo "  ok    $l"; else echo "  FAIL  $l"; fail=1; fi; }
P=dEQP-GLES2.functional
run() { python3 -I "$TOOL" "$@" 2>&1; }
has() { printf '%s\n' "$1" | grep -qx -- "$2"; }
lacks() { ! printf '%s\n' "$1" | grep -q -- "$2"; }

# U-Boot log: a Pass, b Fail, c Pass, d Pass, e Fail then Pass; noise, CR, stamp
printf '12:00:01 deqp: Pass %s.a\r\ndeqp: Fail %s.b\ndeqp: Pass %s.c\ndeqp: Pass %s.d\nnoise\ndeqp: Fail %s.e\ndeqp: Pass %s.e\n' \
  $P $P $P $P $P $P >"$D/ub.log"
# kexec statuses: a Pass, b Pass, c Fail, d Pass, f Pass
printf 'Pass %s.a\nPass %s.b\nFail %s.c\nPass %s.d\nPass %s.f\n' $P $P $P $P $P >"$D/kx.txt"
# armbian results.tsv: a Pass, b Fail, c Fail, d NotSupported, z Pass
printf 'case\tstatus\tdetail\n%s.a\tPass\tPass\n%s.b\tFail\tx\n%s.c\tFail\tx\n%s.d\tNotSupported\tx\n%s.z\tPass\tPass\n' \
  $P $P $P $P $P >"$D/ar.tsv"

out=$(run --uboot "$D/ub.log" --kexec "$D/kx.txt" --armbian "$D/ar.tsv"); rc=$?
chk "exit 0" test $rc -eq 0
chk "a same" has "$out" "$P.a uboot=Pass kexec=Pass armbian=Pass same"
chk "b newly failing (kexec passes)" has "$out" "$P.b uboot=Fail kexec=Pass armbian=Fail newly failing"
chk "c newly passing" has "$out" "$P.c uboot=Pass kexec=Fail armbian=Fail newly passing"
chk "d same across Pass/NotSupported" has "$out" "$P.d uboot=Pass kexec=Pass armbian=NotSupported same"
chk "e last status wins, no baseline: missing" has "$out" "$P.e uboot=Pass kexec=- armbian=- missing"
chk "default scope is the U-Boot run" lacks "$out" "$P.z"
chk "summary" has "$out" "t6-uboot-deqp-compare: 5 cases, same 2, newly failing 1, newly passing 1, missing 1"
out=$(run --quiet --uboot "$D/ub.log" --kexec "$D/kx.txt" --armbian "$D/ar.tsv")
chk "--quiet drops same" lacks "$out" "$P.a "
out=$(run --all --uboot "$D/ub.log" --kexec "$D/kx.txt" --armbian "$D/ar.tsv")
chk "--all: f is missing" has "$out" "$P.f uboot=- kexec=Pass armbian=- missing"
chk "--all summary" has "$out" "t6-uboot-deqp-compare: 7 cases, same 2, newly failing 1, newly passing 1, missing 3"
printf '%s.a\n%s.q\n' $P $P >"$D/scope.txt"
out=$(run --scope "$D/scope.txt" --uboot "$D/ub.log" --kexec "$D/kx.txt" --armbian "$D/ar.tsv")
chk "--scope: q missing" has "$out" "$P.q uboot=- kexec=- armbian=- missing"

# bad input exits 2
run --uboot "$D/nope" --kexec "$D/kx.txt" --armbian "$D/ar.tsv" >/dev/null
chk "unreadable file: exit 2" test $? -eq 2
echo noise >"$D/empty.log"
run --uboot "$D/empty.log" --kexec "$D/kx.txt" --armbian "$D/ar.tsv" >/dev/null
chk "no outcome line: exit 2" test $? -eq 2
run --kexec "$D/kx.txt" >/dev/null
chk "usage error: exit 2" test $? -eq 2

# the real 8ff01055 slices as all three inputs: every case same
set -- "$HERE"/log/2026-11-24-t6-gles2-S?-8ff01055
if [ -f "$1/results.tsv" ]; then
  out=$(run --uboot "$@" --kexec "$@" --armbian "$@"); rc=$?
  chk "real 8ff01055: exit 0" test $rc -eq 0
  last=$(printf '%s\n' "$out" | tail -1)
  case $last in
    *", same "[1-9]*", newly failing 0, newly passing 0, missing 0") chk "real 8ff01055: all same" true ;;
    *) echo "  last line: $last"; chk "real 8ff01055: all same" false ;;
  esac
else
  echo "  skip  real 8ff01055 slices not in this tree"
fi
[ $fail -eq 0 ] && echo "t6-uboot-deqp-compare-selftest: ok"
exit $fail
