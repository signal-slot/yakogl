#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of tools/cts/imx95-redact-check.py (fake values only).
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/imx95-redact-check.py"
d=$(mktemp -d "${TMPDIR:-/tmp}/imx95rc.XXXXXX") || exit 2
trap 'rm -rf "$d"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
run() { python3 -I "$tool" "$@"; }

mkdir "$d/log"
printf 'ethaddr=02:00:5e:00:53:01\neth1addr=02:00:5e:00:53:02\nserial#=0123456\nserverip=192.0.2.1\nipaddr=192.0.2.2\ngatewayip=192.0.2.254\nnetmask=255.255.255.0\nSerial#: 12345678\nlink 02-00-5e-00-53-03 on 198.51.100.7\nopen /dev/ttyUSB3 in /home/someone/x\nmesa-24.0.8.1 v1.2.3.4 fdt\n' >"$d/log/a.log"
# NUL noise splits the values; they must still be caught and removed.
printf 'ethaddr=02:00:\0005e:00:53:01\nip 192.0\0.2.9\nSerial#: 99\0999\n' >"$d/log/nul.log"
cp "$d/log/a.log" "$d/orig"
ln -s "$d/log/a.log" "$d/log/link.log"

run --check "$d/log" >"$d/out"; rc=$?
[ $rc -eq 1 ] && ok "check exits 1" || bad "check exit $rc"
grep -q 'nul.log:1:env' "$d/out" && grep -q 'nul.log:2:ipv4' "$d/out" \
  && grep -q 'nul.log:3:serial' "$d/out" && ok "NUL files are checked" || bad "NUL file skipped"
grep -q 'link.log' "$d/out" && bad "symlink scanned" || ok "symlink skipped"
grep -qE '02:00|192\.0|0123456|someone' "$d/out" && bad "value echoed" || ok "no value echoed"

run "$d/log"; rc=$?
[ $rc -eq 0 ] && ok "rewrite exits 0" || bad "rewrite exit $rc"
run --check "$d/log" >/dev/null && ok "clean after rewrite" || bad "still findings"
grep -q 'ethaddr=<redacted>' "$d/log/a.log" && grep -q 'Serial#: <serial>' "$d/log/a.log" \
  && grep -q 'netmask=<redacted>' "$d/log/a.log" && grep -q 'link <mac> on <addr>' "$d/log/a.log" \
  && grep -q 'open <uart> in <home>' "$d/log/a.log" && ok "replacements" || bad "replacements"
grep -q "mesa-24.0.8.1 v1.2.3.4" "$d/log/a.log" && ok "version tokens kept" || bad "version tokens"
[ "$(tr -d '\0' <"$d/log/nul.log" | wc -c)" -eq "$(wc -c <"$d/log/nul.log")" ] \
  && grep -q 'ethaddr=<redacted>' "$d/log/nul.log" && ok "NUL rewritten" || bad "NUL rewrite"
cmp -s "$d/log/a.log" "$d/orig" && bad "target untouched?" ; [ -L "$d/log/link.log" ] && ok "symlink kept" || bad "symlink replaced"
run --check "$d/nope" 2>/dev/null; [ $? -eq 2 ] && ok "missing path exits 2" || bad "missing path"
[ $fail -eq 0 ] && echo "imx95-redact-check selftest: all ok" || echo "imx95-redact-check selftest: FAILED"
exit $fail
