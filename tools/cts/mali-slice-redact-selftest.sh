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
# unreadable or missing path exits 2; the home-path rule (/home/<name>) and
# the login-name rule (--user NAME, defaulting from BOARD_USER) on slices
# built at run time, whose look-alikes are assembled from parts so that
# neither this file's text nor the tool's holds one (the way
# t6-redact-check.sh concatenates its own patterns). Exits 0 only if every
# check holds.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mali-slice-redact.py
DATA=$HERE/testdata/mali-slice-redact
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

# The login-name rule reads BOARD_USER from the environment; start without it
# so that every run below spells out the name it means (as
# tools/mp2_redact_check_selftest.sh does with env -u and env NAME=value).
unset BOARD_USER

work=$(mktemp -d "${TMPDIR:-/tmp}/mali-slice-redact.XXXXXX") || exit 1
trap 'chmod -R u+rwx "$work" 2>/dev/null; rm -rf "$work"' EXIT

python3 -I "$TOOL" --self-test >/dev/null 2>&1 && ok "--self-test" || bad "--self-test"

out=$(python3 -I "$TOOL" "$DATA/clean" 2>&1); rc=$?
if [ "$rc" = 0 ] && [ -z "$out" ]; then ok "clean fixture: exit 0, no output"; else bad "clean fixture: exit $rc: $out"; fi

out=$(python3 -I "$TOOL" "$DATA/dirty" 2>&1); rc=$?
[ "$rc" = 1 ] && ok "dirty fixture: exit 1" || bad "dirty fixture: exit $rc"
for k in ipv4 ipv6 mac tailnet userhost serial-path uart usb-serial; do
    printf '%s\n' "$out" | grep -Eq "^REDACT .+:[0-9]+: $k\$" && ok "dirty fixture: reports $k" || bad "dirty fixture: no $k"
done
[ "$(printf '%s\n' "$out" | grep -c '^REDACT ')" = 9 ] && ok "dirty fixture: 9 hits" || bad "dirty fixture: not 9 hits"
if printf '%s\n' "$out" | grep -Eq '192\.0\.2|2001:db8|00:00:5e|ts\.net|root@|by-id|ttyUSB|EX000001'; then
    bad "dirty fixture: a match was echoed"
else
    ok "dirty fixture: no match echoed"
fi

# A Zephyr Verdin iMX95 slice has no kmsg/: clean passes; the U-Boot
# identifiers of its console (ethaddr, serial#, eth1addr, serverip) are hits.
[ ! -d "$DATA/zephyr-clean/kmsg" ] && ok "zephyr fixture has no kmsg/" || bad "zephyr-clean has a kmsg/"
out=$(python3 -I "$TOOL" "$DATA/zephyr-clean" 2>&1); rc=$?
if [ "$rc" = 0 ] && [ -z "$out" ]; then ok "zephyr-clean: exit 0 (placeholders pass)"; else bad "zephyr-clean: exit $rc: $out"; fi
out=$(python3 -I "$TOOL" "$DATA/zephyr-dirty" 2>&1); rc=$?
[ "$rc" = 1 ] && ok "zephyr-dirty: exit 1" || bad "zephyr-dirty: exit $rc"
for k in mac ipv4 uboot-id; do
    printf '%s\n' "$out" | grep -Eq "^REDACT .+console.txt:[0-9]+: $k\$" && ok "zephyr-dirty: reports $k" || bad "zephyr-dirty: no $k"
done
[ "$(printf '%s\n' "$out" | grep -c 'uboot-id')" = 4 ] && ok "zephyr-dirty: 4 uboot-id lines" || bad "zephyr-dirty: not 4 uboot-id lines"
if printf '%s\n' "$out" | grep -Eq '192\.0\.2|00:00:5e|12345678|garbled'; then bad "zephyr-dirty: a match was echoed"; else ok "zephyr-dirty: no match echoed"; fi

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
    if [ "$rc" = 2 ] && printf '%s\n' "$err" | grep -q 'locked.txt'; then ok "unreadable file: exit 2, file named"; else bad "unreadable file: exit $rc: $err"; fi
fi

# The home-path rule: a /home/<name> reports; a <redacted> name and a bare
# "/home/" pass, as do the U-Boot placeholders; a relative "home/" component
# (under a word character) is not a home path. The sample is assembled from
# parts at run time.
home_prefix='/ho''me'
mkdir -p "$work/hp" "$work/uc"
{
    printf 'ls %s/root/logs/sweep.log\n' "$home_prefix"
    printf 'see %s/<redacted>/tail and a bare %s/ name pass\n' "$home_prefix" "$home_prefix"
    printf 'relative tools/ho''me/root/notes.txt is not a home path\n'
} > "$work/hp/a.txt"
out=$(python3 -I "$TOOL" "$work/hp" 2>&1); rc=$?
if [ "$rc" = 1 ] && [ "$(printf '%s\n' "$out" | grep -c '^REDACT ')" = 1 ] \
   && printf '%s\n' "$out" | grep -Eq '^REDACT .+a\.txt:1: home-path$' \
   && ! printf '%s\n' "$out" | grep -q 'root/logs'; then
    ok "home-path: the home directory reports; <redacted>, bare and relative home pass, nothing echoed"
else
    bad "home-path: exit $rc: $out"
fi

# The login-name rule: --user NAME searches the name as a whole word; inside
# a user@host pair it reports beside userhost; the name is never echoed.
zzuser=zz''user9
{
    printf 'login as %s on the board\n' "$zzuser"
    printf 'ssh %s@board jump\n' "$zzuser"
    printf 'plain kernel line: panthor fff00000.gpu: GPU reset\n'
} > "$work/uc/b.txt"
out=$(python3 -I "$TOOL" --user "$zzuser" "$work/uc" 2>&1); rc=$?
if [ "$rc" = 1 ] && [ "$(printf '%s\n' "$out" | grep -c '^REDACT ')" = 3 ] \
   && printf '%s\n' "$out" | grep -Eq '^REDACT .+b\.txt:1: user$' \
   && printf '%s\n' "$out" | grep -Eq '^REDACT .+b\.txt:2: user$' \
   && printf '%s\n' "$out" | grep -Eq '^REDACT .+b\.txt:2: userhost$' \
   && ! printf '%s\n' "$out" | grep -q "$zzuser"; then
    ok "--user NAME: the login reports as a word, beside userhost in user@host"
else
    bad "--user NAME: exit $rc: $out"
fi

out=$(env BOARD_USER="$zzuser" python3 -I "$TOOL" "$work/uc" 2>&1); rc=$?
if [ "$rc" = 1 ] && [ "$(printf '%s\n' "$out" | grep -c '^REDACT ')" = 3 ] \
   && printf '%s\n' "$out" | grep -Eq ': user$'; then
    ok "BOARD_USER: the login name defaults from the environment"
else
    bad "BOARD_USER default: exit $rc: $out"
fi

out=$(env BOARD_USER=zz''other9 python3 -I "$TOOL" "$work/uc" 2>&1); rc=$?
if [ "$rc" = 1 ] && [ "$(printf '%s\n' "$out" | grep -c '^REDACT ')" = 1 ] \
   && printf '%s\n' "$out" | grep -Eq ': userhost$' && ! printf '%s\n' "$out" | grep -Eq ': user$'; then
    ok "BOARD_USER holding a name the slice does not: only userhost reports"
else
    bad "BOARD_USER mismatch: exit $rc: $out"
fi

out=$(env BOARD_USER=zz''other9 python3 -I "$TOOL" --user "$zzuser" "$work/uc" 2>&1); rc=$?
if [ "$rc" = 1 ] && [ "$(printf '%s\n' "$out" | grep -c '^REDACT ')" = 3 ]; then
    ok "--user NAME overrides BOARD_USER"
else
    bad "--user override: exit $rc: $out"
fi

python3 -I "$TOOL" --user >/dev/null 2>&1; rc=$?
[ "$rc" = 2 ] && ok "--user without a name: usage, exit 2" || bad "--user without a name: exit $rc"

# Neither this file's own text nor the tool's holds a home path the scan
# would report: the samples above exist only at run time.
if grep -Eq "/home/[A-Za-z0-9]" "$TOOL" "$0"; then
    bad "the tool or this file holds a home path in its own text"
else
    ok "the tool and this file hold no home path in their own text"
fi

if [ "$fail" = 0 ]; then echo "mali-slice-redact-selftest: ok"; exit 0; fi
echo "mali-slice-redact-selftest: $fail failed"
exit 1
