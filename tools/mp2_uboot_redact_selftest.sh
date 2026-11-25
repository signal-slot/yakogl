#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host selftest of tools/mp2_uboot_redact_check.py (plan #357, item
# uboot-redact-check), against generated fixtures only.
#
# Fixture values are synthetic: RFC 5737 documentation addresses
# (192.0.2.0/24, 198.51.100.0/24), RFC 3849 (2001:db8::/32), and zz* fake
# names. The values that tools/release/redaction-scan.py classes would
# report in a tracked file - the tailnet range 100.64/10, a link-local IPv6,
# a *.ts.net name - are written split with "" here, so this file holds none
# of them in full; the checker sees them joined at run time. Nothing here
# reaches the tree: the fixtures live under mktemp and the redaction scan
# row of the gate reads this file, so it must hold no finding.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
CHK=$HERE/mp2_uboot_redact_check.py
T=$(mktemp -d "${TMPDIR:-/tmp}/mp2redact.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
ok=0 fail=0
pass() { ok=$((ok + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; }
# env -u: a gate or operator host may export BOARD_* and NETBOOT_*; the
# plain cases must answer as if they were unset (check.sh runs the row
# through no_board_env, and a direct run must behave the same).
run() { env -u BOARD_HOST -u BOARD_UART -u BOARD_USER -u NETBOOT_SERVER \
        python3 -I "$CHK" "$@" >"$T/out" 2>"$T/err"; echo $?; }
runenv() { env BOARD_HOST=zzhost9 BOARD_USER=zzuser9 BOARD_UART=zzuart9 \
        NETBOOT_SERVER=zzsrv9zz python3 -I "$CHK" "$@" >"$T/out" 2>"$T/err"; echo $?; }

mkrec() { # mkrec DIR : a redacted record, the shape board-stm32mp2-uboot.sh writes
    local d=$1
    mkdir -p "$d"
    printf 'name=mp2_hello\nsha=8fe07a2a\nBOARD_UART=<BOARD_UART>\nhost=<board-host>\n' >"$d/meta.txt"
    {
        echo 'U-Boot 2023.10-stm32mp-r3.1 (Jan 01 2026 - 00:00:00 +0000)'
        echo 'ethaddr=<redacted>'
        echo 'serverip=<redacted>'
        echo 'ipaddr=<redacted>'
        echo 'gatewayip=<redacted>'
        echo 'netmask=<redacted>'
        echo 'hostname=<redacted>'
        echo 'dnsip1=<redacted>'
        echo 'serial#=<redacted>'
        echo 'board_id=<redacted>'
        echo '## Starting application at 0xa0000000 ...'
        echo 'loaded to 0xa0000000, ip 0.0.0.0, lo 127.0.0.1'
        echo 'sha256 e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'
        echo 'setenv autoload no; setenv serverip ${serverip}; setenv ipaddr $ipaddr'
        echo 'printenv serverip ipaddr; printenv ethaddr'
        echo 'console <BOARD_UART>, ssh <board-host>, no mac here'
    } >"$d/console.raw"
    cp "$d/console.raw" "$d/typed.log"
    printf 'ethaddr=<redacted>\n' >"$d/env-before.txt"
    printf 'ethaddr=<redacted>\n' >"$d/env-after.txt"
    # the redacted form of a kept env dump: NUL-separated, every value masked
    printf 'ethaddr=<redacted>\0serverip=<redacted>\0serial#=<redacted>\0' >"$d/env-before.bin"
    printf 'ethaddr=<redacted>\0serverip=<redacted>\0serial#=<redacted>\0' >"$d/env-after.bin"
}

# 1 a redacted record passes, masked env-* files included
mkrec "$T/clean"
[ "$(run "$T/clean")" = 0 ] && pass "redacted record passes" || { bad "redacted record passes"; cat "$T/out" "$T/err"; }

# 2 dirty records: one per kind (run on the record dir; the finding names
#   the file and line; the value itself is never echoed)
dirty() { # dirty NAME KIND LINE LEAK
    local n=$1 kind=$2 line=$3 d=$T/d-$1 rc ln
    mkrec "$d"
    printf '%s\n' "$line" >>"$d/console.raw"
    ln=$(wc -l <"$d/console.raw")
    rc=$(run "$d")
    if [ "$rc" = 1 ] && grep -q "^console.raw:$ln: $kind " "$T/out" && [ "$(wc -l <"$T/out")" = 1 ]; then
        pass "dirty $n -> $kind at console.raw:$ln"
    else
        bad "dirty $n -> $kind (rc=$rc)"; cat "$T/out" "$T/err"
    fi
    if grep -F -q -e "$4" "$T/out" 2>/dev/null; then bad "dirty $n echoes the value"; fi
}
dirty ipv4       ipv4       'DHCP: lease 192.0.2.17 granted' 192.0.2.17
dirty ipv4-port  ipv4       'tftp from 198.51.100.7:69 started' 198.51.100.7
dirty mac        mac        'Net: eth0: 02:00:5e:10:00:01 up' 02:00:5e
dirty mac-dash   mac        'link 02-00-5e-10-00-02 up' 02-00-5e
dirty tty        tty        'console on /dev/ttyACM9 at 115200' ttyACM9
dirty ttyusb     tty        'picocom /dev/ttyUSB7 -f cmd' ttyUSB7
dirty byid       tty        'uart /dev/serial/by-id/usb-FTDI_Dual_RS232-HS-if00-port0 ready' Dual_RS232
dirty usbname    usb-serial 'device usb-FTDI_Dual_RS232-HS-if01 on bus 2' Dual_RS232
dirty iserial    usb-serial 'iSerial                 3 AB12CD34EF' AB12CD34EF
dirty serialnum  usb-serial 'SerialNumber=ZX81Q9' ZX81Q9
dirty idserial   usb-serial 'ID_SERIAL_SHORT=AB12CD34EF' AB12CD34EF
# the split fixtures (a""b) need double quotes: the shell joins the two
# halves when it runs the line, and this file never holds the value whole
dirty tailnet    tailnet    "host zzdk9.tail1234.ts"".net up" "zzdk9.tail1234.ts"".net"
dirty tailnet-ip tailnet-ip "peer 100.100"".100.1 reachable" "100.100"".100.1"
dirty tailnet-lo tailnet-ip "peer 100.64"".0.9 reachable" "100.64"".0.9"
dirty ipv6       ipv6       'ping6 2001:db8::1234 unreachable' db8::1234
dirty ipv6-link  ipv6       "ping6 fe80""::1ff:fe23:4567:890a reply" "fe80""::1ff"
dirty ethaddr    env        'ethaddr=zz-not-redacted' zz-not-redacted
dirty eth1addr   env        'eth1addr=qq-not-redacted' qq-not-redacted
dirty serverip   env        'serverip=zzhostA' zzhostA
dirty ipaddr     env        'ipaddr: someval' someval
dirty serialnum2 env        'serial#=0042abcdef' 0042abcdef
dirty boardid    env        'board_id=zzboardid1' zzboardid1
dirty gatewayip  env        'gatewayip=198.51.100.1' 198.51.100.1
dirty netmask    env        'setenv netmask 255.255.255.0' 255.255.255.0
dirty dnsip      env        'dnsip1=zzdnsserver' zzdnsserver
dirty hostname   env        'hostname=zzboardname' zzboardname
dirty setenv     env        'setenv ethaddr zzmacvalue' zzmacvalue
dirty fwsetenv   env        'fw_setenv board_id zzboardid2' zzboardid2
dirty ssh        ssh        'ran: ssh -p 22 zzu9@zzdk9' zzu9@zzdk9
dirty scp        ssh        'scp a.bin zzu9@zzdk9:/tmp/x' zzu9@zzdk9

# 3 a kept raw env dump is refused: the runner keeps it out of the record
#   because it carries the MAC and the serial; NUL breaks every name=value
d=$T/dump; mkdir -p "$d"
printf 'ethaddr=02:00:5e:10:00:01\0serial#=0042abcdef\0' >"$d/env-before.bin"
rc=$(run "$d")
if [ "$rc" = 1 ] && grep -q '^env-before.bin:1: env ' "$T/out"; then
    pass "raw env dump refused (env-before.bin:1: env)"
else bad "raw env dump refused"; cat "$T/out" "$T/err"; fi
if grep -F -q -e 02:00:5e -e 0042abcdef "$T/out"; then bad "raw env dump echoed a value"; fi

# 4 file arguments: a diff of a record, masked lines pass, the rest counted
printf 'ok line\n+ethaddr=<redacted>\n-serverip=zzbad\nx 192.0.2.5 y 192.0.2.6\n' >"$T/f1.md"
rc=$(run "$T/f1.md")
if [ "$rc" = 1 ] && [ "$(wc -l <"$T/out")" = 3 ] && grep -q 'f1.md:3: env ' "$T/out" \
    && grep -q 'f1.md:4: ipv4 ' "$T/out"; then pass "file arg: three findings, file:line named"; else bad "file arg findings"; cat "$T/out" "$T/err"; fi

# 5 note text: memory addresses, digests, versions, loopback, 0.0.0.0,
#   masked values and prose mentions of the env names all pass
cat >"$T/note.md" <<'EOF'
# Board run
go 0xa0000000, DTB at 0xa8000000, 0xb0000000 end; U-Boot 2023.10-stm32mp-r3.1
Jan 01 2026 - 10:11:12 +0000; md: 9f000000: 00000000 00000000
sha256 3b7e72ae147d93d26d2c1ea6bd6a0b0b3b3b0b0a5b2f1a4a7e0a2c9f5d8e1b44
a0:b1 and aa:bb:cc is not a MAC; 1ff:fe23:4567:890a is not an IPv6 address
bind 0.0.0.0 and 127.0.0.1, ping6 ::1 from ::, U-Boot 2023.10.0
ethaddr=<redacted>, serverip=<NETBOOT_SERVER>, ssh <user>@<board-host>
printenv serverip ipaddr; prose about the netmask and gatewayip variables
EOF
[ "$(run "$T/note.md")" = 0 ] && pass "note text passes" || { bad "note text passes"; cat "$T/out" "$T/err"; }

# 6 allowlist: exact text and kind:text; the wrong kind does not allow
printf 'see 192.0.2.99 here\n' >"$T/al.md"
printf '# reviewed: documentation example\n192.0.2.99\n' >"$T/allow1"
printf 'ipv4:192.0.2.99\n' >"$T/allow2"
printf 'mac:192.0.2.99\n' >"$T/allow3"
[ "$(run "$T/al.md")" = 1 ] && pass "no allowlist: finding" || bad "no allowlist: finding"
[ "$(run --allow "$T/allow1" "$T/al.md")" = 0 ] && pass "--allow exact text" || bad "--allow exact text"
[ "$(run --allow "$T/allow2" "$T/al.md")" = 0 ] && pass "--allow kind:text" || bad "--allow kind:text"
[ "$(run --allow "$T/allow3" "$T/al.md")" = 1 ] && pass "--allow other kind does not allow" || bad "--allow other kind"

# 7 BOARD_* and NETBOOT_* values from the environment; unset passes; a value
#   inside a longer word is not a leak
printf 'connect zzhost9 as zzuser9 through zzuart9 to zzsrv9zz\n' >"$T/ev.txt"
rc=$(runenv "$T/ev.txt")
n=$(grep -c 'ev.txt:1: env-value ' "$T/out")
if [ "$rc" = 1 ] && [ "$n" = 4 ] && ! grep -F -q -e zzhost9 -e zzuser9 -e zzuart9 -e zzsrv9zz "$T/out"; then
    pass "env values flagged, four, masked"
else bad "env values flagged"; cat "$T/out" "$T/err"; fi
[ "$(run "$T/ev.txt")" = 0 ] && pass "no env set: clean" || bad "no env set: clean"
printf 'nothing: zzhost9es zzuser90 zzuart9x zzsrv9zza\n' >"$T/ev2.txt"
[ "$(runenv "$T/ev2.txt")" = 0 ] && pass "env value inside a longer word passes" || { bad "env value inside a longer word passes"; cat "$T/out" "$T/err"; }

# 8 usage and I/O errors
[ "$(run)" = 2 ] && pass "no path: exit 2" || bad "no path: exit 2"
[ "$(run "$T/missing")" = 2 ] && pass "missing path: exit 2" || bad "missing path: exit 2"
[ "$(run --bogus "$T/f1.md")" = 2 ] && pass "unknown option: exit 2" || bad "unknown option: exit 2"
[ "$(run --allow "$T/none" "$T/f1.md")" = 2 ] && pass "missing allowlist: exit 2" || bad "missing allowlist: exit 2"

# 9 CRLF and control bytes do not hide a finding or crash
# a NUL breaks the line it sits in (the env dumps), so the address lands on
# its own line and is found with the other bytes kept
printf 'a\r\nb \x00\x01 192.0.2.44\r\n' >"$T/crlf.raw"
rc=$(run "$T/crlf.raw")
[ "$rc" = 1 ] && grep -q 'crlf.raw:3: ipv4' "$T/out" && pass "CRLF and control bytes" || { bad "CRLF and control bytes"; cat "$T/out" "$T/err"; }

# 10 symlinks in a record are not followed
mkrec "$T/sym"
printf 'x 192.0.2.77\n' >"$T/outside.txt"
ln -s "$T/outside.txt" "$T/sym/link.txt"
[ "$(run "$T/sym")" = 0 ] && pass "symlink in record skipped" || bad "symlink in record skipped"

# 11 the current U-Boot boot note, when present (it quotes runs: it must
#    pass the same check every later quote of it must pass)
NOTE=$HERE/../docs/notes/stm32mp257f-dk-uboot-boot.md
if [ -f "$NOTE" ]; then
    rc=$(run "$NOTE")
    [ "$rc" = 0 ] && pass "current note is clean" || { bad "current note is clean"; cat "$T/out" "$T/err"; }
fi

echo "mp2_uboot_redact_selftest: $ok ok, $fail failed"
[ "$fail" = 0 ]
