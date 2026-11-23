#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Verdict matrix: every env-reading mp2s helper on each canonical DK
# u-boot-env fixture of tools/testdata/mp2s-env/ (pristine, installed,
# counting1, counting2, left0, one-copy-bad, both-bad), against the verdicts
# docs/notes/stm32mp257f-dk-standalone.md requires (sections 3, 5, 7, 8).
# No board, no network; helpers and fixtures are only read.
#
#   tools/mp2s_fixture_matrix_selftest.sh
#
# Prints 'mp2s_fixture_matrix_selftest: N checks, F failed'; exit 0 iff F=0.
# A failure is a contradiction between a helper and the note: file an issue,
# do not edit the helper here.
set -u

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
fx="$here/testdata/mp2s-env"
py() { python3 -I "$@"; }
L=(--redundant --size 0x2000)

tmp=$(mktemp -d "${TMPDIR:-/tmp}/mp2s-fxm.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT

n=0; f=0
# chk NAME WANT_RC REGEX CMD...   (rc 0 = zero, nz = any nonzero)
chk() {
  local name=$1 want=$2 re=$3; shift 3
  local out rc
  out=$("$@" 2>&1); rc=$?
  n=$((n + 1))
  local ok=1
  case $want in
    0) [ "$rc" -eq 0 ] || ok=0 ;;
    nz) [ "$rc" -ne 0 ] || ok=0 ;;
  esac
  printf '%s\n' "$out" | grep -Eq -- "$re" || ok=0
  if [ $ok -eq 0 ]; then
    f=$((f + 1))
    printf 'FAIL %s (rc=%s, want %s, pattern /%s/)\n%s\n' "$name" "$rc" "$want" "$re" "$out" | sed 's/^/  /'
  fi
}
img() { echo "$fx/$1.bin"; }

for x in pristine installed counting1 counting2 left0 one-copy-bad both-bad; do
  [ -f "$(img $x)" ] || { echo "missing fixture $x" >&2; exit 2; }
done

# --- uboot_env.py check (fixtures README; section 8: a bad copy is named)
for x in pristine installed counting1 counting2 left0; do
  chk "env check $x" 0 'ok +copy 1.*' py "$here/uboot_env.py" "${L[@]}" check "$(img $x)"
done
# section 8: one bad copy -> refused, the bad copy is named, the other is ok
chk "env check one-copy-bad names copy 1" nz 'BAD copy 1' py "$here/uboot_env.py" "${L[@]}" check "$(img one-copy-bad)"
chk "env check one-copy-bad copy 2 ok" nz 'ok +copy 2' py "$here/uboot_env.py" "${L[@]}" check "$(img one-copy-bad)"
# section 8: both bad -> unusable (U-Boot would save compiled-in defaults)
chk "env check both-bad: 2 bad copies" nz '2 bad copy' py "$here/uboot_env.py" "${L[@]}" check "$(img both-bad)"
# a bad pair is not printable (section 8: no guessing which copy)
chk "env print both-bad refused" nz 'refused|bad' py "$here/uboot_env.py" "${L[@]}" print "$(img both-bad)"
# section 3: pristine has no mp2s_* variable, the counting images do
chk "env print pristine: no mp2s_left" 0 '^boot_prefixes=/ /boot/$' py "$here/uboot_env.py" "${L[@]}" print "$(img pristine)"
pr=$(py "$here/uboot_env.py" "${L[@]}" print "$(img pristine)" 2>&1 | grep -c '^mp2s_')
n=$((n + 1)); [ "$pr" -eq 0 ] || { f=$((f + 1)); echo "FAIL pristine prints $pr mp2s_ variables"; }
chk "env print counting2: mp2s_left=2 (s3)" 0 '^mp2s_left=2$' py "$here/uboot_env.py" "${L[@]}" print "$(img counting2)"
chk "env print counting1: mp2s_left=1 (s3)" 0 '^mp2s_left=1$' py "$here/uboot_env.py" "${L[@]}" print "$(img counting1)"
chk "env print left0: mp2s_left=0 (s3)" 0 '^mp2s_left=0$' py "$here/uboot_env.py" "${L[@]}" print "$(img left0)"
chk "env print counting1: boot_prefixes in front (s3)" 0 '^boot_prefixes=/yakogl/standalone/ / /boot/$' py "$here/uboot_env.py" "${L[@]}" print "$(img counting1)"

# --- mp2s_guard.py (section 7: one-shot vs standalone are mutually exclusive)
g() { py "$here/mp2s_guard.py" --for "$1" --env-image "$(img "$2")" "${L[@]}"; }
chk "guard oneshot pristine allows" 0 'allow' g oneshot pristine
for x in installed counting1 counting2 left0; do
  chk "guard oneshot $x refuses" nz 'REFUSE' g oneshot "$x"
done
chk "guard standalone pristine allows" 0 'allow' g standalone pristine
for x in installed counting1 counting2 left0; do
  chk "guard standalone $x refuses (a second default)" nz 'REFUSE' g standalone "$x"
done
# section 8: an unreadable env is never "allowed"
for m in oneshot standalone; do
  chk "guard $m both-bad errors" nz 'error' g "$m" both-bad
done
chk "guard oneshot one-copy-bad is not an allow" nz 'REFUSE|error' g oneshot one-copy-bad

# --- mp2s_env_capacity.py (section 3: the default's edits fit, both copies)
cap() { py "$here/mp2s_env_capacity.py" --image "$(img "$1")" --size 0x2000 mp2s_ver=1 mp2s_left=2 mp2s_prefixes='/ /boot/'; }
chk "capacity pristine fits" 0 'FITS' cap pristine
chk "capacity counting1 fits (a decrement)" 0 'FITS' py "$here/mp2s_env_capacity.py" --image "$(img counting1)" --size 0x2000 mp2s_left=0
chk "capacity both-bad refused" nz 'refused' cap both-bad

# --- mp2s_env_diff.py (section 5: restored / NOT restored / default set)
d() { py "$here/mp2s_env_diff.py" --image "${L[@]}" "$@"; }
chk "diff pristine/pristine restored" 0 'mp2s-env: restored' d "$(img pristine)" "$(img pristine)"
for x in installed counting1 counting2 left0; do
  chk "diff pristine/$x NOT restored" nz 'NOT restored' d "$(img pristine)" "$(img $x)"
done
chk "diff pristine/counting2 --expect-default 2 (s3)" 0 'default set' d --expect-default 2 "$(img pristine)" "$(img counting2)"
chk "diff pristine/counting1 --expect-default 2 fails" nz 'mp2s-env' d --expect-default 2 "$(img pristine)" "$(img counting1)"
chk "diff pristine/both-bad cannot read (s8)" nz 'cannot read' d "$(img pristine)" "$(img both-bad)"
chk "diff both-bad/pristine cannot read (s8)" nz 'cannot read' d "$(img both-bad)" "$(img pristine)"
# printenv text form (what the runner reads from the board)
for x in pristine counting1; do
  py "$here/uboot_env.py" "${L[@]}" print "$(img $x)" > "$tmp/$x.txt" 2>/dev/null
done
chk "diff text pristine/pristine restored" 0 'restored' py "$here/mp2s_env_diff.py" "$tmp/pristine.txt" "$tmp/pristine.txt"
chk "diff text pristine/counting1 NOT restored" nz 'NOT restored' py "$here/mp2s_env_diff.py" "$tmp/pristine.txt" "$tmp/counting1.txt"

# --- mp2s_env_plan.py verify (sections 3, 5, 7)
v() { py "$here/mp2s_env_plan.py" verify "$(img "$1")" "$(img "$2")" --expect "$3" "${L[@]}"; }
chk "plan verify pristine->counting2 default" 0 'verify default: ok' v pristine counting2 default
chk "plan verify counting2->pristine nodefault" 0 'verify nodefault: ok' v counting2 pristine nodefault
chk "plan verify counting1->pristine nodefault" 0 'verify nodefault: ok' v counting1 pristine nodefault
chk "plan verify left0->pristine nodefault" 0 'verify nodefault: ok' v left0 pristine nodefault
chk "plan verify pristine->pristine default refused" nz 'refused' v pristine pristine default
chk "plan verify pristine->both-bad refused (s8)" nz 'refused' v pristine both-bad default
chk "plan verify both-bad->pristine refused (s8)" nz 'refused' v both-bad pristine nodefault

# --- plan default on each fixture (section 7: refused unless pristine and both CRCs valid)
pd() { py "$here/mp2s_env_plan.py" default "$(img "$1")" "$tmp/out.bin" --count 2 --record "$tmp/rec" "${L[@]}"; }
chk "plan default pristine ok" 0 'wrote copy 2' pd pristine
for x in installed counting2 both-bad; do
  chk "plan default $x refused" nz 'refused' pd "$x"
done

echo "mp2s_fixture_matrix_selftest: $n checks, $f failed"
[ "$f" -eq 0 ]
