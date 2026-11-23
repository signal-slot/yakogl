#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host selftest of tools/mp2_uboot_redact_check.py. Fixtures are generated here
# (RFC 5737 / 3849 documentation values only); nothing is committed.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
CHK=$HERE/mp2_uboot_redact_check.py
T=$(mktemp -d "${TMPDIR:-/tmp}/mp2redact.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
ok=0 fail=0
pass() { ok=$((ok + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; }
run() { python3 -I "$CHK" "$@" >"$T/out" 2>"$T/err"; echo $?; }

mkrec() { # mkrec DIR : a redacted record
    local d=$1
    mkdir -p "$d"
    printf 'name=mp2_hello\nsha=8fe07a2a\nBOARD_UART=<BOARD_UART>\nhost=<board-host>\n' >"$d/meta.txt"
    {
        echo 'U-Boot 2023.10-stm32mp-r3.1 (Jan 01 2026 - 00:00:00 +0000)'
        echo 'ethaddr=<redacted>'
        echo 'serverip=<redacted>'
        echo 'ipaddr=<redacted>'
        echo 'serial#=<redacted>'
        echo 'board_id=<redacted>'
        echo '## Starting application at 0xa0000000 ...'
        echo 'loaded to 0xa0000000, ip 0.0.0.0, lo 127.0.0.1'
        echo 'sha256 e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'
        echo 'setenv serverip ${serverip}; setenv ipaddr $ipaddr; setenv ethaddr'
        echo 'console <BOARD_UART>, ssh <board-host>, no mac here'
    } >"$d/console.raw"
    cp "$d/console.raw" "$d/typed.log"
    printf 'ethaddr=<redacted>\n' >"$d/env-before.txt"
    printf 'ethaddr=<redacted>\n' >"$d/env-after.txt"
}

# 1 clean record passes
mkrec "$T/clean"
[ "$(run "$T/clean")" = 0 ] && pass "redacted record passes" || { bad "redacted record passes"; cat "$T/out" "$T/err"; }

# 2 dirty records: one per pattern (record dir, finding named file:line)
dirty() { # dirty NAME KIND LINE
    local n=$1 kind=$2 line=$3 d=$T/d-$1 rc
    mkrec "$d"
    printf '%s\n' "$line" >>"$d/console.raw"
    local ln
    ln=$(wc -l <"$d/console.raw")
    rc=$(run "$d")
    if [ "$rc" = 1 ] && grep -q "^console.raw:$ln: $kind " "$T/out" && [ "$(wc -l <"$T/out")" = 1 ]; then
        pass "dirty $n -> $kind at console.raw:$ln"
    else
        bad "dirty $n -> $kind (rc=$rc)"; cat "$T/out" "$T/err"
    fi
    # the value itself is never echoed
    if grep -q -e "$4" "$T/out" 2>/dev/null; then bad "dirty $n echoes the value"; fi
}
dirty ipv4       ipv4       'DHCP: lease 192.0.2.17 granted' 192.0.2.17
dirty ipv4-ver   ipv4       'tftp from 198.51.100.7:69' 198.51.100.7
dirty mac        mac        'Net: eth0: 02:00:5e:10:00:01 up' 02:00:5e
dirty mac-dash   mac        'link 02-00-5e-10-00-02' 02-00-5e
dirty tty        tty        'console on /dev/ttyACM9 at 115200' ttyACM9
dirty ttyusb     tty        'picocom /dev/ttyUSB7' ttyUSB7
dirty byid       tty        'uart /dev/serial/by-id/usb-Doc_Bridge_0123456789AB-if00' 0123456789AB
dirty usbname    usb-serial 'device usb-Doc_Bridge_0123456789AB-if01 seen' 0123456789AB
dirty iserial    usb-serial 'iSerial 3 A1B2C3D4E5F6' A1B2C3D4E5F6
dirty serialnum  usb-serial 'SerialNumber=ZZ99YY88XX77' ZZ99YY88XX77
dirty tailnet    tailnet    'host dk-lab.tail1234.ts.net up' tail1234
dirty tailnet-ip tailnet-ip 'peer 100.100.100.1' 100.100.100.1
dirty tailnet-lo tailnet-ip 'peer 100.64.0.9' 100.64.0.9
dirty ethaddr    env        'ethaddr=zz-not-redacted' zz-not-redacted
dirty eth1addr   env        'eth1addr=qq-not-redacted' qq-not-redacted
dirty serverip   env        'serverip=hostA' hostA
dirty ipaddr     env        'ipaddr: someval' someval
dirty serialnum2 env        'serial#=0042abcdef' 0042abcdef
dirty boardid    env        'board_id=0x1234QQ' 0x1234QQ
dirty setenv     env        'setenv ethaddr secretmacvalue' secretmacvalue
dirty ssh        ssh        'ran: ssh -p 22 operator@labhost' operator@labhost
dirty scp        ssh        'scp a.bin tester@labhost:/x' tester@labhost

# 3 file arguments, diff prefix kept, several findings counted
printf 'ok line\n+ethaddr=<redacted>\n-serverip=bad\nx 192.0.2.5 y 192.0.2.6\n' >"$T/f1.md"
rc=$(run "$T/f1.md")
if [ "$rc" = 1 ] && [ "$(wc -l <"$T/out")" = 3 ] && grep -q '^.*f1.md:3: env ' "$T/out" \
    && grep -q 'f1.md:4: ipv4 ' "$T/out"; then pass "file arg: three findings, file:line named"; else bad "file arg findings"; cat "$T/out"; fi

# 4 note text: memory addresses, digests, versions, loopback, 0.0.0.0 pass
cat >"$T/note.md" <<'EOF'
# Board run
go 0xa0000000, DTB at 0xa8000000, 0xb0000000 end; U-Boot 2023.10.0
sha256 3b7e72ae147d93d26d2c1ea6bd6a0b0b3b3b0b0a5b2f1a4a7e0a2c9f5d8e1b44
a0:b1 and aa:bb:cc is not a MAC; bind 0.0.0.0 and 127.0.0.1
EOF
[ "$(run "$T/note.md")" = 0 ] && pass "memory addresses, digests, versions pass" || { bad "note text passes"; cat "$T/out"; }

# 5 allowlist: exact text and kind:text; wrong kind does not allow
printf 'see 192.0.2.99 here\n' >"$T/al.md"
printf '# reviewed: documentation example\n192.0.2.99\n' >"$T/allow1"
printf 'ipv4:192.0.2.99\n' >"$T/allow2"
printf 'mac:192.0.2.99\n' >"$T/allow3"
[ "$(run "$T/al.md")" = 1 ] && pass "no allowlist: finding" || bad "no allowlist: finding"
[ "$(run --allow "$T/allow1" "$T/al.md")" = 0 ] && pass "--allow exact text" || bad "--allow exact text"
[ "$(run --allow "$T/allow2" "$T/al.md")" = 0 ] && pass "--allow kind:text" || bad "--allow kind:text"
[ "$(run --allow "$T/allow3" "$T/al.md")" = 1 ] && pass "--allow other kind does not allow" || bad "--allow other kind"

# 6 BOARD_* values from the environment
printf 'connect to labbox-17 now\n' >"$T/ev.txt"
rc=$(BOARD_HOST=labbox-17 python3 -I "$CHK" "$T/ev.txt" >"$T/out" 2>"$T/err"; echo $?)
if [ "$rc" = 1 ] && grep -q 'ev.txt:1: env-value' "$T/out"; then pass "BOARD_HOST value flagged"; else bad "BOARD_HOST value"; fi
[ "$(run "$T/ev.txt")" = 0 ] && pass "no BOARD_HOST: clean" || bad "no BOARD_HOST: clean"

# 7 usage and I/O errors
[ "$(run)" = 2 ] && pass "no path: exit 2" || bad "no path: exit 2"
[ "$(run "$T/missing")" = 2 ] && pass "missing path: exit 2" || bad "missing path: exit 2"
[ "$(run --bogus "$T/f1.md")" = 2 ] && pass "unknown option: exit 2" || bad "unknown option: exit 2"
[ "$(run --allow "$T/none" "$T/f1.md")" = 2 ] && pass "missing allowlist: exit 2" || bad "missing allowlist: exit 2"

# 8 CRLF console and a binary byte do not hide a finding or crash
printf 'a\r\nb \x00\x01 192.0.2.44\r\n' >"$T/crlf.raw"
rc=$(run "$T/crlf.raw")
[ "$rc" = 1 ] && grep -q 'crlf.raw:2: ipv4' "$T/out" && pass "CRLF and binary bytes" || bad "CRLF and binary bytes"

# 9 symlinks in a record are not followed
mkrec "$T/sym"
printf 'x 192.0.2.77\n' >"$T/outside.txt"
ln -s "$T/outside.txt" "$T/sym/link.txt"
[ "$(run "$T/sym")" = 0 ] && pass "symlink in record skipped" || bad "symlink in record skipped"

# 10 the current note, when present
NOTE=$HERE/../docs/notes/stm32mp257f-dk-uboot-boot.md
if [ -f "$NOTE" ]; then
    rc=$(run "$NOTE")
    [ "$rc" = 0 ] && pass "current note is clean" || { bad "current note is clean"; cat "$T/out"; }
fi

echo "mp2_uboot_redact_selftest: $ok ok, $fail failed"
[ "$fail" = 0 ]
