#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_plan_model_selftest.sh - the env edit of tools/mp2s_env_plan.py equals
# the fault model's default_vars/nodefault_vars (tools/mp2s_model.py), for
# N=1..9, on tools/testdata/mp2s-env/pristine.bin. No board, no QEMU.
#
#   tools/mp2s_plan_model_selftest.sh
#
# Per N: default N, then nodefault. Each result, read with tools/uboot_env.py,
# must equal the model's variable set exactly (so the changed mp2s_* variables
# and their values agree), and bootcmd, bootcount, altbootcmd and
# upgrade_available keep their value (or absence). After nodefault the
# variables are the pristine ones.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PLAN="$here/mp2s_env_plan.py"
UE="$here/uboot_env.py"
PRISTINE="$here/testdata/mp2s-env/pristine.bin"
LAY=(--redundant --size 0x2000)
T=$(mktemp -d "${TMPDIR:-$here}/mp2s-plan-model-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT

python3 -I "$UE" "${LAY[@]}" print "$PRISTINE" > "$T/pristine.env"

# cmp_model KIND N BEFORE.env AFTER.env: one line "name<TAB>status<TAB>detail" per check.
cmp_model() {
  python3 -I - "$here" "$@" <<'PY'
import sys
sys.path.insert(0, sys.argv[1] + "/")
import mp2s_model as m
kind, n, bp, ap = sys.argv[2], int(sys.argv[3]), sys.argv[4], sys.argv[5]
def rd(p):
    d = {}
    for l in open(p, "rb").read().decode("latin-1").split("\n"):
        if l:
            k, _, v = l.partition("=")
            d[k] = v
    return d
b, a = rd(bp), rd(ap)
want = m.default_vars(b, n) if kind == "default" else m.nodefault_vars(b)
bad = ["%s: env_plan=%r model=%r" % (k, a.get(k), want.get(k))
       for k in sorted(set(a) | set(want)) if a.get(k) != want.get(k)]
print("model\t%s\t%s" % ("ok" if not bad else "FAIL", "; ".join(bad)))
for k in ("bootcmd", "bootcount", "altbootcmd", "upgrade_available"):
    print("%s\t%s\t%r" % (k, "ok" if a.get(k) == b.get(k) else "FAIL", (b.get(k), a.get(k))))
PY
}

pass=0
fail=0
report() { # report DESC: reads cmp_model lines on stdin
  local d=$1 name st detail
  while IFS=$'\t' read -r name st detail; do
    if [ "$st" = ok ]; then
      pass=$((pass + 1))
    else
      fail=$((fail + 1))
      echo "FAIL $d $name: $detail"
    fi
  done
}

for n in 1 2 3 4 5 6 7 8 9; do
  if ! python3 -I "$PLAN" default "$PRISTINE" "$T/d.bin" --count "$n" --record "$T/rec" "${LAY[@]}" > "$T/plan.out" 2>&1; then
    fail=$((fail + 1)); echo "FAIL default $n: env_plan refused"; sed 's/^/     /' "$T/plan.out"; continue
  fi
  python3 -I "$UE" "${LAY[@]}" print "$T/d.bin" > "$T/d.env"
  cmp_model default "$n" "$T/pristine.env" "$T/d.env" | report "default $n"
  if ! python3 -I "$PLAN" nodefault "$T/d.bin" "$T/u.bin" --record "$T/rec" "${LAY[@]}" > "$T/plan.out" 2>&1; then
    fail=$((fail + 1)); echo "FAIL nodefault after $n: env_plan refused"; sed 's/^/     /' "$T/plan.out"; continue
  fi
  python3 -I "$UE" "${LAY[@]}" print "$T/u.bin" > "$T/u.env"
  cmp_model nodefault "$n" "$T/d.env" "$T/u.env" | report "nodefault after $n"
  if cmp -s <(sort "$T/pristine.env") <(sort "$T/u.env"); then
    pass=$((pass + 1))
  else
    fail=$((fail + 1)); echo "FAIL nodefault after $n: not back to pristine"
  fi
done

echo "mp2s_plan_model_selftest: $((pass + fail)) checks, $fail failed"
[ "$fail" -eq 0 ]
