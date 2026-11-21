#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# frame16_selftest: compiles test/frame16_test.c (host test of src/frame16.h)
# with the host cc under -std=c11 -Wall -Wextra -Werror (plus AddressSanitizer
# and UBSan when the compiler has them) into build/rpi5_scanout16/, runs it,
# then feeds the console logs it wrote (built only from the formatters) to
# verdicts.sh: the good log must exit 0 (PASS), the log with a readback FAIL
# 3 and the log with a missing line 1. Last line:
# "frame16_selftest: N checks, 0 failed"; exit 0. No cc: a gap, exit 77.
cd "$(dirname "$0")/../../../.." || exit 1
cc=${CC:-cc}
out=build/rpi5_scanout16
dir=tools/probes/rpi_5/rpi5_scanout16
if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
mkdir -p "$out" || exit 1
san=""
if echo 'int main(void){return 0;}' > "$out/san_probe.c" &&
   "$cc" -fsanitize=address,undefined -o "$out/san_probe" "$out/san_probe.c" >/dev/null 2>&1 &&
   "$out/san_probe" >/dev/null 2>&1; then
  san="-fsanitize=address,undefined -fno-sanitize-recover=undefined"
fi
# shellcheck disable=SC2086
if ! "$cc" -std=c11 -Wall -Wextra -Werror $san -o "$out/frame16_test" \
    "$dir/test/frame16_test.c"; then
  echo "frame16_selftest: compile failed"
  exit 1
fi
rm -f "$out/good.log" "$out/readback_fail.log" "$out/missing.log"
res=$("$out/frame16_test" "$out")
rc=$?
printf '%s\n' "$res"
if [ $rc -ne 0 ]; then
  echo "frame16_selftest: FAILED"
  exit 1
fi
n=$(printf '%s\n' "$res" | tail -n 1 | sed -n 's/^frame16_test: \([0-9][0-9]*\) checks, 0 failed$/\1/p')
if [ -z "$n" ]; then
  echo "frame16_selftest: no result line from the test"
  exit 1
fi

bad=0
vchecks=0
verdict() { # <log> <expected exit> <expected text>
  vchecks=$((vchecks + 1))
  vout=$(bash "$dir/verdicts.sh" "$out/$1" 2>&1)
  vrc=$?
  if [ "$vrc" -ne "$2" ]; then
    echo "FAIL verdicts.sh $1: exit $vrc, wanted $2"
    printf '%s\n' "$vout"
    bad=$((bad + 1))
    return
  fi
  vchecks=$((vchecks + 1))
  if ! printf '%s\n' "$vout" | grep -q "$3"; then
    echo "FAIL verdicts.sh $1: no '$3' in:"
    printf '%s\n' "$vout"
    bad=$((bad + 1))
  fi
}
verdict good.log 0 '^verdicts: PASS'
verdict readback_fail.log 3 '^verdicts: FAIL (.*readback 2 FAIL'
verdict missing.log 1 '^verdicts: incomplete'
# the good log has all 11 lines, and q2 is answered 'no'
vchecks=$((vchecks + 1))
if ! bash "$dir/verdicts.sh" "$out/good.log" | grep -q '11 of 11 expected verdict lines' ; then
  echo "FAIL good.log: not 11 of 11 lines"
  bad=$((bad + 1))
fi
vchecks=$((vchecks + 1))
if ! bash "$dir/verdicts.sh" "$out/good.log" | grep -q '^q2: no '; then
  echo "FAIL good.log: q2 is not 'no'"
  bad=$((bad + 1))
fi
if [ "$bad" -ne 0 ]; then
  echo "frame16_selftest: $bad failed"
  exit 1
fi
echo "frame16_selftest: $((n + vchecks)) checks, 0 failed"
exit 0
