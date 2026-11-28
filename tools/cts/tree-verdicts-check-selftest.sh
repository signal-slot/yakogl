#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of tree-verdicts-check.sh: exact, tagged and sliced labels, no_reset.
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
work=$(mktemp -d "${TMPDIR:-/var/tmp}/tvcst.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/cts/log"
cp "$here/tree-verdicts-check.sh" "$work/cts/"
chk=$work/cts/tree-verdicts-check.sh
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
tree=$work/x1-gles3-tree.txt
{ echo "# image named:"; echo "dEQP-GLES3.a.*"; for i in 1 2 3; do echo "dEQP-GLES3.f$i.*"; done; } >"$tree"
# 4 rows: named, img2, img3, img4 (idx counts every filter row)
mk() { # dir label verdict
  mkdir -p "$work/cts/log/$1"
  echo "board.sh x1 $2 rc=0 verdict=$3 resets=0" >>"$work/cts/log/$1/console.log"
}
run() { "$chk" "$tree" aaaaaaaa; }
d=d-x1-gles3-aaaaaaaa

# (a) exact labels
rm -rf "$work/cts/log"/*
for l in named img2 img3 img4; do mk $d "$l" done; done
out=$(run); rc=$?
if [ $rc -eq 0 ]; then ok "exact labels"; else bad "exact labels"; echo "$out"; fi

# (b) tagged labels
rm -rf "$work/cts/log"/*
for l in named img2-t img3-t img4-t; do mk $d "$l" done; done
out=$(run); rc=$?
if [ $rc -eq 0 ]; then ok "tagged labels"; else bad "tagged labels"; echo "$out"; fi

# (c) slice past row 1; img2 is not satisfied by img20-t; empty tag works
rm -rf "$work/cts/log"/*
mk $d named done
mk $d img20-t done
mk $d img3- done
mk $d img4-t done
out=$(run); rc=$?
if [ $rc -ne 0 ] && printf '%s\n' "$out" | grep -q '^img2 missing' &&
  printf '%s\n' "$out" | grep -q '^img3 done' && printf '%s\n' "$out" | grep -q '^img4 done'; then
  ok "img2 not matched by img20-t"
else bad "img2 vs img20-t"; echo "$out"; fi
mk $d img2-t done
mk $d img2-u done
out=$(run); rc=$?
if [ $rc -eq 0 ] && printf '%s\n' "$out" | grep -q '^img2 done  (2 lines, last counts)'; then
  ok "slice with tags, several lines"
else bad "slice with tags"; echo "$out"; fi

# (d) no_reset makes the exit non-zero
rm -rf "$work/cts/log"/*
for l in named img2-t img4-t; do mk $d "$l" done; done
mk $d img3-t no_reset
out=$(run); rc=$?
if [ $rc -ne 0 ] && printf '%s\n' "$out" | grep -q '^img3 no_reset'; then ok "no_reset fails"
else bad "no_reset"; echo "$out"; fi

if [ $fail -eq 0 ]; then echo "tree-verdicts-check-selftest: OK"; fi
exit $fail
