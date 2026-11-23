#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest: every env-reading mp2s helper gives the designed verdict on each
# canonical env fixture of tools/testdata/mp2s-env/ (README there). The
# verdicts are those docs/notes/stm32mp257f-dk-standalone.md requires
# (sections 3, 5, 7 and 8). No board, no network; helpers and fixtures are
# only read. Prints 'mp2s_fixture_matrix_selftest: N checks, F failed'.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
FIX=$HERE/testdata/mp2s-env
L=(--redundant --size 0x2000)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/mp2s-fixmatrix.XXXXXX") || exit 2
trap 'rm -rf "$WORK"' EXIT

N=0
F=0
OUT=
RC=0
# run LABEL CMD... : sets OUT (stdout+stderr) and RC
run() {
  OUT=$("$@" 2>&1)
  RC=$?
}
pass() { N=$((N + 1)); }
fail() {
  N=$((N + 1))
  F=$((F + 1))
  echo "FAIL: $1 (rc=$RC): $(printf '%s' "$OUT" | tail -3 | tr '\n' '|')" >&2
}
# expect_rc LABEL WANT CMD...
expect_rc() {
  local label=$1 want=$2
  shift 2
  run "$@"
  if [ "$RC" -eq "$want" ]; then pass; else fail "$label: want rc $want"; fi
}
# expect_out LABEL WANT_RC REGEX CMD...
expect_out() {
  local label=$1 want=$2 re=$3
  shift 3
  run "$@"
  if [ "$RC" -eq "$want" ] && printf '%s' "$OUT" | grep -Eq -- "$re"; then pass; else fail "$label: want rc $want and /$re/"; fi
}
env_py() { python3 -I "$HERE/uboot_env.py" "${L[@]}" "$@"; }
guard() { python3 -I "$HERE/mp2s_guard.py" --for "$1" --env-image "$FIX/$2.bin" "${L[@]}"; }
cap() { python3 -I "$HERE/mp2s_env_capacity.py" --image "$FIX/$1.bin" --size 0x2000 "${@:2}"; }

for f in pristine installed counting2 counting1 left0 one-copy-bad both-bad; do
  [ -f "$FIX/$f.bin" ] || { echo "missing fixture $f.bin" >&2; exit 2; }
done

# --- uboot_env.py: check (fixture README; note section 8: a bad copy is
# named, U-Boot boots from the valid one; both bad = defaults would be saved)
for f in pristine installed counting2 counting1 left0; do
  expect_rc "uboot_env check $f valid" 0 env_py check "$FIX/$f.bin"
done
# section 8: one bad copy is not fatal to the check's verdict? read it, pin it
run env_py check "$FIX/one-copy-bad.bin"
RC1=$RC
run env_py check "$FIX/both-bad.bin"
RC2=$RC
if [ "$RC2" -ne 0 ] && printf '%s' "$OUT" | grep -qi 'bad\|crc\|invalid'; then pass; else fail "uboot_env check both-bad names bad copies"; fi
run env_py check "$FIX/one-copy-bad.bin"
if printf '%s' "$OUT" | grep -qi 'bad\|crc\|invalid\|copy'; then pass; else fail "uboot_env check one-copy-bad names the bad copy"; fi
# print: a pristine image has ST's boot_prefixes and no mp2s_* (section 3)
expect_out "uboot_env print pristine boot_prefixes" 0 '^boot_prefixes=/ /boot/$' env_py print "$FIX/pristine.bin"
run env_py print "$FIX/pristine.bin"
if printf '%s' "$OUT" | grep -q '^mp2s_'; then fail "pristine has no mp2s_*"; else pass; fi
# counting1 boots from copy 2 with mp2s_left=1; left0 from copy 1 with 0 (README)
expect_out "uboot_env print counting1 mp2s_left=1" 0 '^mp2s_left=1$' env_py print "$FIX/counting1.bin"
expect_out "uboot_env print counting2 mp2s_left=2" 0 '^mp2s_left=2$' env_py print "$FIX/counting2.bin"
expect_out "uboot_env print left0 mp2s_left=0" 0 '^mp2s_left=0$' env_py print "$FIX/left0.bin"
expect_out "uboot_env print counting1 prefix armed" 0 '^boot_prefixes=/yakogl/standalone/ / /boot/$' env_py print "$FIX/counting1.bin"
# section 8: both-bad is unusable, never pristine
run env_py print "$FIX/both-bad.bin"
if [ "$RC" -ne 0 ]; then pass; else fail "uboot_env print both-bad refused"; fi

# --- mp2s_guard.py (section 7 'Runner verbs', section 5)
# one-shot: refused (3) while any mp2s_* is set
for f in installed counting2 counting1 left0; do
  expect_rc "guard oneshot refuses $f" 3 guard oneshot $f
done
expect_rc "guard oneshot allows pristine" 0 guard oneshot pristine
# standalone: refused while boot_prefixes carries /yakogl/standalone/ (section 5)
expect_rc "guard standalone allows pristine" 0 guard standalone pristine
for f in installed counting2 counting1 left0; do
  expect_rc "guard standalone refuses armed prefix $f" 3 guard standalone $f
done
# both-bad: no usable environment, neither verdict may be an allow
for m in oneshot standalone; do
  run guard $m both-bad
  if [ "$RC" -ne 0 ]; then pass; else fail "guard $m both-bad not allowed"; fi
done
# one-copy-bad: U-Boot boots copy 2 (mp2s_left=1): one-shot refused if readable
run guard oneshot one-copy-bad
if [ "$RC" -ne 0 ]; then pass; else fail "guard oneshot one-copy-bad not allowed"; fi

# --- mp2s_env_capacity.py (section 3 'The variables' / section 8)
expect_out "capacity ok on pristine" 0 'ok' cap pristine mp2s_ver=1 mp2s_left=2 mp2s_prefixes="/ /boot/"
expect_out "capacity ok on counting1 (update in place)" 0 'ok' cap counting1 mp2s_left=0
run cap both-bad mp2s_left=2
if [ "$RC" -ne 0 ]; then pass; else fail "capacity refuses both-bad"; fi
# an oversize edit does not fit
BIG=$(head -c 9000 /dev/zero | tr '\0' x)
run cap pristine "mp2s_big=$BIG"
if [ "$RC" -ne 0 ]; then pass; else fail "capacity refuses an 9000-byte value"; fi

# --- mp2s_env_diff.py (section 5 'The undo', section 7): printenv text
pp() { env_py print "$FIX/$1.bin" >"$WORK/$1.txt" 2>/dev/null; }
for f in pristine installed counting2 counting1 left0; do pp "$f"; done
diff_txt() { python3 -I "$HERE/mp2s_env_diff.py" "$WORK/$1.txt" "$WORK/$2.txt" "${@:3}"; }
expect_out "diff pristine/pristine restored" 0 '^mp2s-env: restored' diff_txt pristine pristine
for f in installed counting2 counting1 left0; do
  expect_out "diff pristine/$f NOT restored" 1 '^mp2s-env: NOT restored' diff_txt pristine $f
done
# mid-run state (section 3): the default set, count 2 and 1
expect_out "diff pristine/counting2 default set" 0 '^mp2s-env: default set' diff_txt pristine counting2 --expect-default 2
expect_out "diff pristine/counting1 default set 1" 0 '^mp2s-env: default set' diff_txt pristine counting1 --expect-default 1
expect_out "diff pristine/counting1 not count 2" 1 '^mp2s-env: NOT default set' diff_txt pristine counting1 --expect-default 2
# raw-image form agrees (--image)
expect_out "diff --image pristine/pristine restored" 0 '^mp2s-env: restored' \
  python3 -I "$HERE/mp2s_env_diff.py" --image "${L[@]}" "$FIX/pristine.bin" "$FIX/pristine.bin"
expect_out "diff --image pristine/counting1 NOT restored" 1 '^mp2s-env: NOT restored' \
  python3 -I "$HERE/mp2s_env_diff.py" --image "${L[@]}" "$FIX/pristine.bin" "$FIX/counting1.bin"

# --- mp2s_env_plan.py (sections 3, 5, 7): verify and the edits
plan() { python3 -I "$HERE/mp2s_env_plan.py" "$@"; }
expect_rc "plan default 2 on pristine" 0 plan default "$FIX/pristine.bin" "$WORK/d2.bin" --count 2 --record "$WORK/rec" "${L[@]}"
expect_rc "plan verify pristine->default" 0 plan verify "$FIX/pristine.bin" "$WORK/d2.bin" --expect default "${L[@]}"
expect_rc "plan nodefault on its own default" 0 plan nodefault "$WORK/d2.bin" "$WORK/nd.bin" --record "$WORK/rec" "${L[@]}"
expect_rc "plan verify default->nodefault" 0 plan verify "$WORK/d2.bin" "$WORK/nd.bin" --expect nodefault "${L[@]}"
# a default over a default is refused (section 3: mp2s_* absent before default)
for f in installed counting2 counting1 left0; do
  expect_rc "plan default refused on $f" 2 plan default "$FIX/$f.bin" "$WORK/x-$f.bin" --count 2 --record "$WORK/r-$f" "${L[@]}"
done
# both-bad: no environment to edit (section 8)
run plan default "$FIX/both-bad.bin" "$WORK/xb.bin" --count 2 --record "$WORK/rb" "${L[@]}"
if [ "$RC" -ne 0 ] && [ ! -e "$WORK/xb.bin" ]; then pass; else fail "plan default refused on both-bad, no output"; fi
# nodefault on a pristine image is a no-op: OUT is IMAGE's bytes
expect_rc "plan nodefault on pristine no-op" 0 plan nodefault "$FIX/pristine.bin" "$WORK/np.bin" --record "$WORK/rnp" "${L[@]}"
if cmp -s "$FIX/pristine.bin" "$WORK/np.bin"; then pass; else fail "plan nodefault pristine leaves bytes"; fi

echo "mp2s_fixture_matrix_selftest: $N checks, $F failed"
[ "$F" -eq 0 ]
