#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx95-redact-check.py on a synthetic Verdin iMX95 U-Boot console: a printenv
# listing, the Toradex banner, a typed setenv, Linux staging lines and the dEQP start line.
# Made-up values only: MACs from the locally administered range (02:..), RFC 5737 and
# 2001:db8:: addresses, fake serials. MACs, the UART and home paths, the host name and the
# token are built by concatenation so this file's own text stays clean under the scans.
# POSIX sh; scratch under $TMPDIR (else the current directory), removed on exit.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/imx95-redact-check.py"
t=$(mktemp -d "${TMPDIR:-.}/imx95-redact.XXXXXX") || exit 2
trap 'rm -rf "$t"' EXIT
fail=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*" >&2; fail=$((fail + 1)); }

m="02:5e:00:53"
mac1="$m:00:01"
mac2="$m:00:02"
mac3="02-5e-00""-53-00-03"
uart="/dev/""ttyUSB9"
home="/ho""me/someone/vi95.log"
som="verdin-imx95-""0a0b0c0d"
tok="tok""en=notarealvalue"
u="ro""ot"
mask="255"".0"
ver="vulkan-cts-1.4.6.2-555-g0123abcd"

mkdir "$t/raw"
# raw/console.log: one line per case; the expected --check output below names the lines.
{
	echo 'U-Boot 2024.04-7.1.0+git (Nov 01 2026 - 00:00:00 +0000)'           # 1 clean
	echo 'Model: Toradex 0000 Verdin iMX95 Hexa 8GB WB IT V1.0A, Serial# 99000001' # 2 serial
	echo 'Serial#: 99000001'                                                   # 3 serial
	echo '=> printenv'                                                         # 4 clean
	echo 'baudrate=115200'                                                     # 5 clean
	echo "eth1addr=$mac2"                                                      # 6 env
	echo "ethaddr=$mac1"                                                       # 7 env
	echo 'fdtfile=imx95-verdin-wifi-mallow.dtb'                                # 8 clean
	echo 'gatewayip=203.0.113.254'                                             # 9 env
	echo 'ipaddr=192.0.2.10'                                                   # 10 env
	echo "netmask=255.255.$mask"                                             # 11 env
	echo 'serial#=99000001'                                                    # 12 env
	echo 'serverip=198.51.100.20'                                              # 13 env
	echo 'Environment size: 1234/65532 bytes'                                  # 14 clean
	echo '=> setenv serverip 198.51.100.21'                                    # 15 env
	echo "Net:   eth0: enetc-0 [PRIME] MAC $mac3 via 192.0.2.11"               # 16 mac, addr
	echo "inet6 2001:db8::5/64 scope global"                                   # 17 addr6
	echo "$u@$som:~# md5sum /run/vi95.bin"                                   # 18 somhost
	echo "console $uart, log $home"                                            # 19 uart, home
	printf 'NUL\000 byte here\n'                                               # 20 nul
	echo "dEQP Core $ver starting.."                                           # 21 clean for --check (rewritten)
	echo "curl -s ?$tok"                                                       # 22 secret
	echo 'dcache off; icache flush; go 0xd0000000'                             # 23 clean
} > "$t/raw/console.log"
echo "ipaddr=192.0.2.77" > "$t/outside.log"
ln -s ../outside.log "$t/raw/link.log"
cp -R "$t/raw" "$t/red"

want_check="raw/console.log:2:serial
raw/console.log:3:serial
raw/console.log:6:env
raw/console.log:7:env
raw/console.log:9:env
raw/console.log:10:env
raw/console.log:11:env
raw/console.log:12:env
raw/console.log:13:env
raw/console.log:15:env
raw/console.log:16:mac
raw/console.log:16:addr
raw/console.log:17:addr6
raw/console.log:18:somhost
raw/console.log:19:uart
raw/console.log:19:home
raw/console.log:20:nul
raw/console.log:22:secret"

cd "$t"
rc=0; got=$(python3 -I "$tool" --check raw) || rc=$?
[ "$rc" -eq 1 ] && ok "--check exits 1 on the unredacted copy" || bad "--check rc=$rc on raw"
[ "$got" = "$want_check" ] && ok "--check names file:line:kind of every hit" ||
	{ bad "--check output differs"; printf '%s\n' "$got" >&2; }
for leak in 02:5e 02-5e 192.0.2 198.51 203.0.113 2001:db8 99000001 0a0b0c0d ttyUSB someone notareal; do
	case "$got" in *"$leak"*) bad "--check prints a value ($leak)" ;; esac
done
ok "--check prints no value"

python3 -I "$tool" --fix red || bad "rewrite failed"
{
	echo 'U-Boot 2024.04-7.1.0+git (Nov 01 2026 - 00:00:00 +0000)'
	echo 'Model: Toradex 0000 Verdin iMX95 Hexa 8GB WB IT V1.0A, Serial# <serial>'
	echo 'Serial#: <serial>'
	echo '=> printenv'
	echo 'baudrate=115200'
	echo 'eth1addr=<redacted>'
	echo 'ethaddr=<redacted>'
	echo 'fdtfile=imx95-verdin-wifi-mallow.dtb'
	echo 'gatewayip=<redacted>'
	echo 'ipaddr=<redacted>'
	echo 'netmask=<redacted>'
	echo 'serial#=<redacted>'
	echo 'serverip=<redacted>'
	echo 'Environment size: 1234/65532 bytes'
	echo '=> setenv serverip <redacted>'
	echo 'Net:   eth0: enetc-0 [PRIME] MAC <mac> via <addr>'
	echo 'inet6 <addr6>/64 scope global'
	echo "$u@verdin-imx95-<serial>:~# md5sum /run/vi95.bin"
	echo 'console <uart> log <home>'
	echo 'NUL byte here'
	echo 'dEQP Core vulkan-cts-<1.4.6-2>-555-g0123abcd starting..'
	echo 'curl -s ?token=<redacted>'
	echo 'dcache off; icache flush; go 0xd0000000'
} > want.log
# the UART rule takes the path's trailing ',' with it (console-redact.py's \S*)
if cmp -s want.log red/console.log; then ok "the printenv capture is rewritten as expected"; else
	bad "rewrite differs"; diff want.log red/console.log >&2 || :; fi
grep -qF 'vulkan-cts-<1.4.6-2>-555-g0123abcd' red/console.log &&
	ok "the dEQP version token survives in readable form" || bad "version token lost"

rc=0; got=$(python3 -I "$tool" --check red) || rc=$?
[ "$rc" -eq 0 ] && [ -z "$got" ] && ok "--check is clean after the rewrite" || bad "--check rc=$rc after rewrite: $got"
rc=0; python3 -I "$tool" --check raw >/dev/null || rc=$?
[ "$rc" -eq 1 ] && ok "the unredacted copy still fails --check" || bad "raw rc=$rc"
rc=0; python3 -I "$tool" --check raw/console.log >/dev/null || rc=$?
[ "$rc" -eq 1 ] && ok "--check takes a file as well" || bad "file rc=$rc"

# a log whose only quad is the raw dEQP token (as the S1a consoles are) passes --check
mkdir tok
echo "dEQP Core $ver (0x0123abcd) starting.." > tok/T6S1R01.log
rc=0; got=$(python3 -I "$tool" --check tok) || rc=$?
[ "$rc" -eq 0 ] && [ -z "$got" ] && ok "a raw dEQP version token is not a hit" || bad "raw token rc=$rc: $got"

cp red/console.log again.log
python3 -I "$tool" --fix red
cmp -s again.log red/console.log && ok "a second rewrite changes nothing" || bad "not idempotent"
[ "$(cat outside.log)" = "ipaddr=192.0.2.77" ] && ok "a symlink is not followed" || bad "symlink target rewritten"

# a bare call reports, exits 2 and leaves the dirty file unchanged
cp raw/console.log bare.log
rc=0; got=$(python3 -I "$tool" bare.log 2>/dev/null) || rc=$?
[ "$rc" -eq 2 ] && cmp -s raw/console.log bare.log && ok "a bare call exits 2 and leaves the file unchanged" ||
	bad "bare call rc=$rc or file changed"
case "$got" in *bare.log:2:serial*) ok "a bare call prints the hits" ;; *) bad "bare call prints no hits" ;; esac
rc=0; python3 -I "$tool" --fix >/dev/null 2>&1 || rc=$?
[ "$rc" -eq 2 ] && ok "--fix without a path is a usage error" || bad "--fix no path rc=$rc"
rc=0; python3 -I "$tool" --check >/dev/null 2>&1 || rc=$?
[ "$rc" -eq 2 ] && ok "no path is a usage error" || bad "no path rc=$rc"
rc=0; python3 -I "$tool" --check missing >/dev/null 2>&1 || rc=$?
[ "$rc" -eq 2 ] && ok "a missing path exits 2" || bad "missing path rc=$rc"

[ "$fail" -eq 0 ] && { echo "imx95-redact-check-selftest: all ok"; exit 0; }
echo "imx95-redact-check-selftest: $fail failed" >&2
exit 1
