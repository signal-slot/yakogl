#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mali-slice-redact-selftest.sh
#
# Host-only check of tools/cts/mali-slice-redact.py: its own --self-test; the
# fixture slices in tools/cts/testdata/mali-slice-redact/ (clean/ exits 0,
# dirty/ exits 1 with one hit per kind and never echoes a match); the committed
# slices of tools/cts/testdata/mali-deqp/good/ (read-only) are clean; an
# unreadable or missing path exits 2. Exits 0 only if every check holds.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mali-slice-redact.py
DATA=$HERE/testdata/mali-slice-redact
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

work=$(mktemp -d "${TMPDIR:-/tmp}/mali-slice-redact.XXXXXX") || exit 1
trap 'chmod -R u+rwx "$work" 2>/dev/null; rm -rf "$work"' EXIT

python3 -I "$TOOL" --self-test >/dev/null 2>&1 && ok "--self-test" || bad "--self-test"

out=$(python3 -I "$TOOL" "$DATA/clean" 2>&1); rc=$?
if [ "$rc" = 0 ] && [ -z "$out" ]; then ok "clean fixture: exit 0, no output"; else bad "clean fixture: exit $rc: $out"; fi

out=$(python3 -I "$TOOL" "$DATA/dirty" 2>&1); rc=$?
[ "$rc" = 1 ] && ok "dirty fixture: exit 1" || bad "dirty fixture: exit $rc"
for k in ipv4 ipv6 mac tailnet userhost serial-path uart usb-serial; do
    grep -Eq "^REDACT .+:[0-9]+: $k\$" <<<"$out" && ok "dirty fixture: reports $k" || bad "dirty fixture: no $k"
done
[ "$(grep -c '^REDACT ' <<<"$out")" = 9 ] && ok "dirty fixture: 9 hits" || bad "dirty fixture: not 9 hits"
if grep -Eq '192\.0\.2|2001:db8|00:00:5e|ts\.net|root@|by-id|ttyUSB|EX000001' <<<"$out"; then
    bad "dirty fixture: a match was echoed"
else
    ok "dirty fixture: no match echoed"
fi

# The committed good slices (read-only) are clean.
out=$(python3 -I "$TOOL" "$HERE"/testdata/mali-deqp/good/* 2>&1); rc=$?
if [ "$rc" = 0 ] && [ -z "$out" ]; then ok "mali-deqp/good slices are clean"; else bad "mali-deqp/good: exit $rc: $out"; fi

# Several slices at once: the dirty one decides the exit code.
python3 -I "$TOOL" "$DATA/clean" "$DATA/dirty" >/dev/null 2>&1; rc=$?
[ "$rc" = 1 ] && ok "clean + dirty: exit 1" || bad "clean + dirty: exit $rc"

# Unreadable paths exit 2 and name the file.
python3 -I "$TOOL" "$work/missing" >/dev/null 2>&1; rc=$?
[ "$rc" = 2 ] && ok "missing path: exit 2" || bad "missing path: exit $rc"
mkdir "$work/s"; echo "ok" > "$work/s/meta.txt"; echo "ok" > "$work/s/locked.txt"
chmod 000 "$work/s/locked.txt"
if [ -r "$work/s/locked.txt" ]; then
    ok "unreadable file: skipped (running as a user that reads everything)"
else
    err=$(python3 -I "$TOOL" "$work/s" 2>&1 >/dev/null); rc=$?
    if [ "$rc" = 2 ] && grep -q 'locked.txt' <<<"$err"; then ok "unreadable file: exit 2, file named"; else bad "unreadable file: exit $rc: $err"; fi
fi

if [ "$fail" = 0 ]; then echo "mali-slice-redact-selftest: ok"; exit 0; fi
echo "mali-slice-redact-selftest: $fail failed"
exit 1
