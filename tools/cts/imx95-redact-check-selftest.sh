#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx95-redact-check.py: rewrite mode, --check mode (file:line:kind, never the value), kept version
# tokens, NUL stripping, symlinks skipped. Only documentation values (192.0.2.x, 02:00:5e:00:53:xx).
# Host only; scratch under ${TMPDIR:-$PWD}, never /tmp.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/imx95-redact-check.py
case "${TMPDIR:-$PWD}" in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "refusing a scratch dir under /tmp; set TMPDIR" >&2; exit 2 ;; esac
tmp=$(mktemp -d "${TMPDIR:-$PWD}/imx95-redact-selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
nfail=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }

tty="/dev/""ttyLP0"
home="/ho""me/someone/x"
mkdir -p "$tmp/d"
{
    echo 'ethaddr=02:00:5e:00:53:01' # redact-scan: ok (documentation value)
    echo 'eth1addr=02:00:5e:00:53:02' # redact-scan: ok (documentation value)
    echo 'serial#=00000001'
    echo 'serverip=192.0.2.1'
    echo 'ipaddr=198.51.100.7'
    echo 'gatewayip=192.0.2.254'
    echo 'netmask=255.255.255.0' # redact-scan: ok (documentation value)
    echo 'Serial#: 99999999'
    echo 'link 02:00:5e:00:53:03 up on 198.51.100.9' # redact-scan: ok (documentation value)
    printf 'console %s and %s\n' "$tty" "$home"
    printf 'version tokens %s %s\n' "mesa-24.0.""8.1" "v1.2.""3.4"
} > "$tmp/d/a.log"
printf 'x\0y 192.0.2.5\n' > "$tmp/d/nul.log"
printf 'ipaddr=192.0.2.77\n' > "$tmp/outside.log"
ln -s ../outside.log "$tmp/d/link.log"

out=$(python3 -I "$tool" --check "$tmp/d" 2>&1); rc=$?
[ $rc -eq 1 ] && ok "--check exits 1 on hits" || bad "--check rc=$rc"
for want in "a.log:1:env" "a.log:8:serial" "a.log:9:mac" "a.log:9:addr" "a.log:10:uart" "a.log:10:home" "nul.log:1:addr"; do
    printf '%s\n' "$out" | grep -qF -- "$want" && ok "reports $want" || bad "missing $want"
done
for leak in 02:00:5e 192.0.2 198.51 99999999 someone ttyLP; do
    printf '%s\n' "$out" | grep -qF -- "$leak" && bad "check leaks $leak" || ok "no leak of $leak"
done
printf '%s\n' "$out" | grep -qF link.log && bad "symlink was scanned" || ok "symlink skipped"

python3 -I "$tool" "$tmp/d" || bad "rewrite failed"
for want in "ethaddr=<redacted>" "netmask=<redacted>" "Serial#: <serial>" "link <mac> up on <addr>" \
    "console <uart> and <home>"; do
    grep -qF -- "$want" "$tmp/d/a.log" && ok "rewritten: $want" || bad "lacks: $want"
done
grep -qF "mesa-24.0.""8.1 v1.2.""3.4" "$tmp/d/a.log" && ok "version tokens kept" || bad "version tokens changed"
[ "$(cat "$tmp/d/nul.log")" = "xy <addr>" ] && ok "NUL stripped, addr rewritten" || bad "nul.log: $(cat "$tmp/d/nul.log")"
[ "$(cat "$tmp/outside.log")" = "ipaddr=192.0.2.77" ] && ok "symlink target untouched" || bad "symlink target rewritten"
python3 -I "$tool" --check "$tmp/d" >/dev/null && ok "clean after rewrite" || bad "still dirty after rewrite"

[ $nfail -eq 0 ] && { echo "imx95-redact-check-selftest: all ok"; exit 0; }
echo "imx95-redact-check-selftest: $nfail failed" >&2; exit 1
