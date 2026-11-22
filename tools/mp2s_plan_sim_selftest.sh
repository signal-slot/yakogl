#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_plan_sim_selftest.sh - the environment edit of tools/mp2s_env_plan.py
# drives the product script in tools/mp2s_script_sim.py (plan #362, item
# plan-sim-selftest). No board, no QEMU.
#
#   tools/mp2s_plan_sim_selftest.sh
#
# For N=1..9: start from tools/testdata/mp2s-env/pristine.bin, apply
# `mp2s_env_plan.py default --count N`, hand the changed variables to the
# simulator (as its start environment, copy B) and run N+1 boots: boots 1..N
# print "boots left N-1".."0" and start the image, boot N+1 is stock (Linux, no
# image), and the final env has no mp2s_* variable. Also: default 3, one boot,
# then the nodefault edit (on the env that boot leaves: mp2s_left=2), then the
# next boot is stock and the variables are the pristine ones. bootcmd,
# bootcount and altbootcmd keep their value (or absence) throughout.
#
# A tools/check.sh row waits for plan #341, so it is run by hand.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/.." && pwd)
PLAN="$here/mp2s_env_plan.py"
SIM="$here/mp2s_script_sim.py"
UE="$here/uboot_env.py"
CMD=$root/driver/tools/mp2-standalone/mp2-standalone.cmd
PRISTINE="$here/testdata/mp2s-env/pristine.bin"
LAY=(--redundant --size 0x2000)
T=$(mktemp -d "${TMPDIR:-$here}/mp2s-plan-sim-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
pass=0
fail=0
ok() { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; [ -f "$T/out" ] && sed 's/^/     /' "$T/out"; return 0; }
check() { # check DESC CMD...: ok when CMD succeeds
  local d=$1; shift
  if "$@"; then ok "$d"; else bad "$d"; fi
}
has() { grep -qxF -- "$1" "$T/out"; }
count() { grep -cxF -- "$1" "$T/out" || true; }

penv() { python3 -I "$UE" "${LAY[@]}" print "$1"; }
penv "$PRISTINE" > "$T/stock.env"

# sets_of BEFORE.env AFTER.env: NUL-separated sim options turning BEFORE into AFTER.
sets_of() {
  python3 -I - "$1" "$2" <<'EOF'
import sys
def rd(p):
    d = {}
    for l in open(p, "rb").read().split(b"\n"):
        if l:
            k, _, v = l.partition(b"=")
            d[k] = v
    return d
a, b = rd(sys.argv[1]), rd(sys.argv[2])
o = sys.stdout.buffer
for k in sorted(b):
    if a.get(k) != b[k]:
        o.write(b"--set\0" + k + b"=" + b[k] + b"\0")
for k in sorted(a):
    if k not in b:
        o.write(b"--unset\0" + k + b"\0")
EOF
}

# prot ENV: the protected variables, as lines (absent ones print nothing).
prot() { grep -E '^(bootcmd|bootcount|altbootcmd)=' "$1" || true; }
prot "$T/stock.env" > "$T/prot.ref"

# plan_default N OUT: apply the default edit; env text in OUT.env.
plan_default() {
  python3 -I "$PLAN" default "$PRISTINE" "$T/$2.bin" --count "$1" --record "$T/rec" "${LAY[@]}" > "$T/plan.out" 2>&1
  penv "$T/$2.bin" > "$T/$2.env"
}

sim_edit() { # sim_edit EDITED.env [sim options]: run the sim from stock with the edit as copy B
  local e=$1; shift
  mapfile -d '' -t S < <(sets_of "$T/stock.env" "$e")
  rc=0
  python3 "$SIM" run "$CMD" --env "$T/stock.env" --left none "${S[@]}" "$@" > "$T/out" 2>&1 || rc=$?
}

for n in 1 2 3 4 5 6 7 8 9; do
  plan_default "$n" "d$n"
  sim_edit "$T/d$n.env" --max-boots $((n + 1)) --check-restored
  good=1
  [ "$rc" -eq 0 ] || good=0
  # boots 1..N: "boots left" N-1 .. 0 and the image started
  for ((i = 1; i <= n; i++)); do
    left=$((n - i))
    [ "$(count "mp2s: default /yakogl/standalone/default.bin, boots left $left")" -eq 1 ] || good=0
    grep -qxF "sim: boot $i: the image runs, then resets the SoC" "$T/out" || good=0
  done
  # boot N+1 is stock: no script run, Linux, no image
  grep -qxF "sim: boot $((n + 1)): prefix /: ST's extlinux, booting Linux (OpenSTLinux)" "$T/out" || good=0
  [ "$(grep -c 'the image runs' "$T/out" || true)" -eq "$n" ] || good=0
  [ "$(grep -c 'boots left' "$T/out" || true)" -eq "$n" ] || good=0
  grep -q "^sim: result: Linux at boot $((n + 1)), " "$T/out" || good=0
  grep -qxF 'sim: env: boot_prefixes "/ /boot/", no mp2s_*' "$T/out" || good=0
  grep -qxF 'sim: env: other variables changed: none' "$T/out" || good=0
  # the plan's own env: mp2s_left is N, bootcmd/bootcount/altbootcmd untouched
  grep -qxF "mp2s_left=$n" "$T/d$n.env" || good=0
  prot "$T/d$n.env" | cmp -s - "$T/prot.ref" || good=0
  if [ "$good" -eq 1 ]; then ok "default $n: boots left up to $((n - 1)) down to 0 with the image, boot $((n + 1)) stock, env restored"; else bad "default $n: plan and simulator disagree"; fi
done

# default 3, one boot, then nodefault: the next boot is stock.
plan_default 3 d3b
sim_edit "$T/d3b.env" --max-boots 1
has "mp2s: default /yakogl/standalone/default.bin, boots left 2" && grep -qxF 'sim: boot 1: the image runs, then resets the SoC' "$T/out" \
  && ok "default 3: boot 1 shows boots left 2 and starts the image" || bad "default 3: boot 1 shows boots left 2 and starts the image"
# the env that boot leaves: the script's decrement (mp2s_left=2), as saved
python3 -I "$UE" "${LAY[@]}" set "$T/d3b.bin" "$T/after1.bin" mp2s_left=2 > /dev/null
python3 -I "$PLAN" nodefault "$T/after1.bin" "$T/nodef.bin" --record "$T/rec" "${LAY[@]}" > "$T/out" 2>&1 && nrc=0 || nrc=$?
penv "$T/nodef.bin" > "$T/nodef.env"
check "default 3, one boot, nodefault: plan exits 0" test "$nrc" -eq 0
check "nodefault: the env is the pristine one, variable for variable" cmp -s "$T/nodef.env" "$T/stock.env"
check "nodefault: bootcmd, bootcount, altbootcmd unchanged" bash -c 'prot() { grep -E "^(bootcmd|bootcount|altbootcmd)=" "$1" || true; }; prot "$1" | cmp -s - "$2"' _ "$T/nodef.env" "$T/prot.ref"
sim_edit "$T/nodef.env" --max-boots 3
if [ "$rc" -eq 0 ] && grep -qxF "sim: boot 1: prefix /: ST's extlinux, booting Linux (OpenSTLinux)" "$T/out" \
   && [ "$(count 'sim: boot 1: the image runs, then resets the SoC')" -eq 0 ] && ! grep -q 'boots left' "$T/out" \
   && grep -q '^sim: result: Linux at boot 1, ' "$T/out"; then
  ok "after nodefault the next boot is stock (Linux at boot 1, no image, no count)"
else bad "after nodefault the next boot is stock"; fi

# the pristine image itself is never written
check "pristine.bin untouched (git)" git -C "$root" diff --quiet -- tools/testdata/mp2s-env/pristine.bin

echo "mp2s_plan_sim_selftest: $((pass + fail)) checks, $fail failed"
[ "$fail" -eq 0 ]
