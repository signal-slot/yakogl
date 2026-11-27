#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of tools/cts/mip3-deqp-diff.py on the synthetic fixtures in
# tools/cts/testdata/mip3-deqp-diff/: no difference, a new failure, a new pass.
set -u
here=$(cd "$(dirname "$0")" && pwd)
d=$here/testdata/mip3-deqp-diff
tool=$here/mip3-deqp-diff.py
fail=0
check() { # name want-exit want-last-line extra-grep args...
  local name=$1 want=$2 last=$3 grepx=$4; shift 4
  local out rc
  out=$(python3 -I "$tool" "$@" 2>&1); rc=$?
  if [ "$rc" != "$want" ]; then echo "FAIL $name: exit $rc, want $want"; fail=1; return; fi
  if [ "$(printf '%s\n' "$out" | tail -n 1)" != "$last" ]; then
    echo "FAIL $name: last line: $(printf '%s\n' "$out" | tail -n 1)"; fail=1; return; fi
  if [ -n "$grepx" ] && ! printf '%s\n' "$out" | grep -q -- "$grepx"; then
    echo "FAIL $name: no '$grepx'"; fail=1; return; fi
  echo "ok   $name"
}
check same 0 "mipmap.2d: 5 cases, 0 differ (new failures 0, new passes 0, other 0)" "" "$d/a.txt" "$d/same.txt"
check newfail 1 "mipmap.2d: 5 cases, 1 differ (new failures 1, new passes 0, other 0)" "generate.rgba8888_nearest | Pass | Fail" "$d/a.txt" "$d/e-newfail.txt"
check newpass 0 "mipmap.2d: 5 cases, 1 differ (new failures 0, new passes 1, other 0)" "DIFFERENCE OUTSIDE generate" "$d/a.txt" "$d/e-newpass.txt"
check all 0 "mipmap.2d: 5 cases, 0 differ (new failures 0, new passes 0, other 0)" "basic.a | Pass | Pass" --all "$d/a.txt" "$d/same.txt"
check prefix 0 "mipmap.2d: 1 cases, 0 differ (new failures 0, new passes 0, other 0)" "" --prefix dEQP-GLES2.functional.texture.size. "$d/a.txt" "$d/same.txt"
[ $fail = 0 ] && echo "mip3-deqp-diff selftest: ok"
exit $fail
