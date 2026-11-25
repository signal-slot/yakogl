#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2_uboot_redact_selftest.sh - tools/mp2_uboot_redact_check.py on DK U-Boot
# run records generated here (plan #357, item uboot-redact-check).
#
#   tools/mp2_uboot_redact_selftest.sh
#
# No board, no network, python3 standard library only. One redacted record
# (the shape tools/board-stm32mp2-uboot.sh's redact_run leaves: <redacted>
# env values, addresses as 192.0.2.N) must pass; then one dirty copy per
# pattern must be refused, naming the file and line, with the value masked.
# Fixture values come from RFC 5737 (198.51.100.0/24, 203.0.113.0/24),
# locally administered MACs and made-up names; the private-range, tailnet,
# MAC, tty and login@host values are assembled at run time, so this file
# holds none of them literally. Last line: "mp2_uboot_redact_selftest: N ok,
# 0 failed".
set -u
here=$(cd "$(dirname "$0")" && pwd)
C=$here/mp2_uboot_redact_check.py
T=$(mktemp -d "${TMPDIR:-$here}/mp2_uboot_redact_selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
unset BOARD_HOST BOARD_UART BOARD_USER NETBOOT_SERVER
ok=0
bad=0
pass() { ok=$((ok + 1)); echo "ok   $1"; }
fail() { bad=$((bad + 1)); echo "FAIL $1"; [ -n "${2-}" ] && sed 's/^/     /' "$2"; }
chk() { python3 -I "$C" "$@" > "$T/out" 2> "$T/err"; }

# ---- values, built from parts ----------------------------------------------------------
D=/dev
COLON=:
mac() { printf '02%s00%s5e%s10%s20%s%s' "$1" "$1" "$1" "$1" "$1" "$2"; }   # locally administered
V4_DOC1=198.51.100.23
V4_DOC2=203.0.113.5
V4_PRIV=$(printf '%d.%d.%d.%d' 10 20 30 40)
V4_TAIL=$(printf '%d.%d.%d.%d' 100 $((64 + 37)) 7 9)
TSNAME=mp2bench.example-tailnet.ts$(printf '.%s' net)
LOGIN=$(printf '%s@%s' ro ot)
V6=$(printf '%s:%s::%s' fd12 3456 9)

# ---- the redacted record ---------------------------------------------------------------
REC=$T/rec/20261125-1200-mp2_hello-0123abcd
mkrec() {
    local r=$1
    mkdir -p "$r"
    cat > "$r/meta.txt" <<'EOF'
board=stm32mp257f-dk
boot=uboot-mmc
image=mp2_hello
image_sha256=3f9a0c4e5b6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3e4f5a6b
env_dev_before=mmcblk1p3
env_dev_after=mmcblk1p3
env_before_bin_sha256=0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9
env_diff=empty
board_host=<board-host>
uart=<BOARD_UART>
EOF
    {
        printf 'NOTICE:  BL2: v2.10-stm32mp2-r1.0\r\n'
        printf 'U-Boot 2023.10-stm32mp-r1 (Oct 01 2026 - 00:00:00 +0000)\r\n'
        printf '\033[0mModel: STMicroelectronics STM32MP257F-DK\r\n'
        printf 'In:    serial@400e0000\r\nOut:   serial@400e0000\r\n'
        printf 'Net:   eth0: ethernet@482c0000\r\n\0\0'
        printf 'ethact=ethernet@8000000port@1\r\n'
        printf 'Hit any key to stop autoboot:  0 \r\nSTM32MP> '
        printf 'setenv mp2z_server 192.0.2.1; tftpboot ${scriptaddr} 192.0.2.1:yakogl/mp2_hello.scr\r\n'
        printf 'DHCP client bound to address 192.0.2.2 (12 ms)\r\n'
        printf 'mp2z: go 0xa0000000 (memory@a0000000, 0x00000000a0000000)\r\n'
        printf '*** Booting Zephyr OS build v4.1.0 ***\r\nmp2_hello: PASS: hello\r\n'
        printf 'listening on 0.0.0.0, loopback 127.0.0.1, Linux 6.6.48, 1.2.3.4.5\r\n'
        printf '[    1.2] 400e0000.serial: ttySTM0 at MMIO 0x400e0000\r\n'
        printf 'eth0 link 00:00:00:00:00:00\r\n'
    } > "$r/console.raw"
    printf '0.001 U-Boot 2023.10-stm32mp-r1\n12.500 mp2_hello: PASS: hello\n' > "$r/console.ts"
    printf 'typed ok: 1 line on <BOARD_UART>, server <NETBOOT_SERVER>, ssh <board-host>\n' > "$r/typed.log"
    cat > "$r/env-before.txt" <<'EOF'
board_id=<redacted>
bootcmd=run bootcmd_stm32mp
ethaddr=<redacted>
ipaddr=<redacted>
scriptaddr=0x90100000
serial#=<redacted>
serverip=<redacted>
EOF
    cp "$r/env-before.txt" "$r/env-after.txt"
    : > "$r/env.diff"
    printf '%s  /boot/yakogl/mp2_hello.bin\n' \
        3f9a0c4e5b6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3e4f5a6b > "$r/boot-sha256.txt"
    printf 'verdict: PASS (mp2_hello: PASS: hello), env unchanged\n' > "$r/verdict.txt"
    printf 'meta.txt console.raw console.ts typed.log env-before.txt env-after.txt verdict.txt\n' > "$r/INDEX.txt"
}
mkrec "$REC"

if chk "$REC" && [ ! -s "$T/out" ]; then pass "redacted record passes"; else fail "redacted record passes" "$T/out"; fi
if chk "$REC/console.raw" "$REC/env-before.txt"; then pass "redacted files given one by one pass"; else fail "redacted files given one by one pass" "$T/out"; fi
if python3 -I "$C" - < "$REC/console.raw" > "$T/out" 2>&1; then pass "stdin (-) of the redacted console passes"; else fail "stdin (-) of the redacted console passes" "$T/out"; fi

# ---- one dirty record per pattern --------------------------------------------------------
# dirty NAME FILE KIND LINE-TEXT VALUE: a copy of the record with LINE-TEXT
# appended to FILE must be refused (exit 1) with "FILE:<its line>: KIND:",
# and VALUE must not appear in the output.
n=0
dirty() {
    local name=$1 file=$2 kind=$3 text=$4 value=$5 d line
    n=$((n + 1))
    d=$T/dirty$n
    mkrec "$d"
    printf '%s\n' "$text" >> "$d/$file"
    line=$(wc -l < "$d/$file")
    chk "$d"
    local rc=$?
    if [ "$rc" = 1 ] && grep -qF "$d/$file:$line: $kind: " "$T/out" && ! grep -qF -- "$value" "$T/out" "$T/err"; then
        pass "refused: $name ($file:$line, $kind, masked)"
    else
        echo "     rc=$rc want 1, want '$file:$line: $kind:'" >> "$T/out"
        fail "refused: $name" "$T/out"
    fi
}
dirty "IPv4, RFC 5737 198.51.100/24 (not redact_run's 192.0.2/24)" console.raw ipv4 "Using ethernet@482c0000 device, TFTP from server $V4_DOC1" "$V4_DOC1"
dirty "IPv4, RFC 5737 203.0.113/24" console.ts ipv4 "13.000 DHCP client bound to address $V4_DOC2" "$V4_DOC2"
dirty "IPv4, private range" meta.txt ipv4 "server=$V4_PRIV" "$V4_PRIV"
dirty "tailnet address (100.64/10)" typed.log tailnet "ssh to $V4_TAIL" "$V4_TAIL"
dirty "tailnet name (*.ts.net)" verdict.txt tailnet "host $TSNAME" "$TSNAME"
dirty "MAC, ':'" console.raw mac "eth0: link up, $(mac "$COLON" 01)" "$(mac "$COLON" 01)"
dirty "MAC, '-'" INDEX.txt mac "adapter $(mac - 02)" "$(mac - 02)"
dirty "IPv6 (ULA)" console.raw ipv6 "inet6 $V6/64" "$V6"
dirty "/dev/ttyUSB*" typed.log tty "opened $D/tty""USB0 at 115200" "tty""USB0"
dirty "/dev/tty* (ACM)" meta.txt tty "uart=$D/tty""ACM3" "tty""ACM3"
dirty "/dev/serial/by-id link" typed.log tty "uart $D/serial/by-id/usb-STMicroelectronics_STLINK-V3_004C1A2B3C4D-if02" "1A2B3C4D"
dirty "USB serial (iSerial)" typed.log serial "  iSerial                 3 004C1A2B3C4D" "004C1A2B3C4D"
dirty "USB serial (SerialNumber)" meta.txt serial "SerialNumber: 0671FF3833" "0671FF3833"
dirty "USB serial (ID_SERIAL_SHORT)" meta.txt serial "ID_SERIAL_SHORT=A9F3K2Q7" "A9F3K2Q7"
dirty "ethaddr= value" env-before.txt env "ethaddr=$(mac "$COLON" 03)" "$(mac "$COLON" 03)"
dirty "serverip= value" env-after.txt env "serverip=$V4_DOC1" "$V4_DOC1"
dirty "ipaddr= value" env-before.txt env "ipaddr=$V4_DOC2" "$V4_DOC2"
dirty "serial#= value" env-before.txt env "serial#=002B00253232511639353236" "002B00253232511639353236"
dirty "board_id= value" env-after.txt env "board_id=0x00000AB3" "0x00000AB3"
dirty "hostname= value (env.diff '>' line)" env.diff env "> hostname=mp2bench" "mp2bench"
dirty "ssh login@host" typed.log login "ssh $LOGIN@mp2bench 'dd if=x'" "$LOGIN@mp2bench"
dirty "ssh:// login@host" meta.txt login "board_host=ssh://$LOGIN@mp2bench" "$LOGIN@mp2bench"
dirty "shell prompt login@host" console.raw login "$LOGIN@mp2bench:~# reboot" "$LOGIN@mp2bench"
dirty "LAN host name (.local)" typed.log lanhost "server mp2bench.local" "mp2bench.local"

# the values of the board variables, when set
d=$T/envval
mkrec "$d"
printf 'console on bridgebox9:4711\n' >> "$d/typed.log"
if BOARD_UART=bridgebox9:4711 python3 -I "$C" "$d" > "$T/out" 2>&1; then
    fail "refused: BOARD_UART's value" "$T/out"
elif grep -qF "typed.log:2: envval: " "$T/out" && ! grep -qF bridgebox9 "$T/out"; then
    pass "refused: BOARD_UART's value (envval, masked)"
else
    fail "refused: BOARD_UART's value" "$T/out"
fi
if chk "$d"; then pass "the same line passes with BOARD_UART unset"; else fail "the same line passes with BOARD_UART unset" "$T/out"; fi

# line numbers count '\n' lines of the raw capture (NULs, '\r', escapes removed)
d=$T/lines
mkdir -p "$d"
printf 'a\r\n\0b\r\n\033[1mc\033[0m\r\nfrom %s\r\n' "$V4_DOC1" > "$d/console.raw"
if ! chk "$d" && grep -qF "$d/console.raw:4: ipv4: 1***[13]" "$T/out"; then
    pass "raw capture: line 4 of a NUL/CR/ANSI file, masked as 1***[13]"
else
    fail "raw capture: line 4 of a NUL/CR/ANSI file" "$T/out"
fi

# ---- the allowlist -----------------------------------------------------------------------
d=$T/allow
mkrec "$d"
printf 'getty on %s/tty%s\n' "$D" STM0 >> "$d/console.raw"
cat > "$T/allow.ok" <<'EOF'
# reviewed: the DK's own Linux console device, not a host UART path
tty | */console.raw | /dev/ttySTM0 | the board's own console, ST's image
EOF
cat > "$T/allow.glob" <<'EOF'
tty | */typed.log | /dev/ttySTM0 | wrong file
EOF
cat > "$T/allow.part" <<'EOF'
tty | */console.raw | /dev/tty | not the whole match
EOF
printf 'tty | */console.raw | /dev/ttySTM0\n' > "$T/allow.bad"
printf 'nosuchkind | * | x | y\n' > "$T/allow.kind"
if ! chk "$d" && grep -qF ": tty: /***" "$T/out"; then pass "without --allow the board console path is refused"; else fail "without --allow the board console path is refused" "$T/out"; fi
if chk --allow "$T/allow.ok" "$d" && grep -qF "1 finding(s) dropped" "$T/err"; then pass "--allow: a reviewed rule drops it, counted on stderr"; else fail "--allow: a reviewed rule drops it" "$T/err"; fi
chk --allow "$T/allow.glob" "$d"
if [ $? = 1 ]; then pass "--allow: a rule for another file does not drop it"; else fail "--allow: a rule for another file does not drop it" "$T/out"; fi
chk --allow "$T/allow.part" "$d"
if [ $? = 1 ]; then pass "--allow: the regex must match the whole hit"; else fail "--allow: the regex must match the whole hit" "$T/out"; fi
chk --allow "$T/allow.bad" "$d"
if [ $? = 2 ] && grep -qF "allow.bad:1:" "$T/err"; then pass "--allow: a rule without a reason stops with exit 2 and its line"; else fail "--allow: a rule without a reason" "$T/err"; fi
chk --allow "$T/allow.kind" "$d"
if [ $? = 2 ]; then pass "--allow: an unknown kind stops with exit 2"; else fail "--allow: an unknown kind" "$T/err"; fi
chk --allow "$T/no-such-allow" "$d"
if [ $? = 2 ]; then pass "--allow: a missing allowlist stops with exit 2"; else fail "--allow: a missing allowlist" "$T/err"; fi

# ---- usage and the tree --------------------------------------------------------------------
chk
if [ $? = 2 ]; then pass "no path: exit 2"; else fail "no path: exit 2" "$T/err"; fi
chk "$T/no-such-record"
if [ $? = 2 ]; then pass "a missing path: exit 2"; else fail "a missing path: exit 2" "$T/err"; fi
d=$T/link
mkrec "$d"
printf 'from %s\n' "$V4_DOC1" > "$T/outside.txt"
ln -s "$T/outside.txt" "$d/outside.txt"
if chk "$d" && grep -qF "not followed (symlink)" "$T/err"; then pass "a symlink in the record is not followed, and said so"; else fail "a symlink in the record is not followed" "$T/err"; fi
mkdir -p "$d/sub"
printf 'from %s\n' "$V4_DOC2" > "$d/sub/extra.txt"
chk "$d"
if [ $? = 1 ] && grep -qF "sub/extra.txt:1: ipv4:" "$T/out"; then pass "a subdirectory of the record is read"; else fail "a subdirectory of the record is read" "$T/out"; fi

echo "mp2_uboot_redact_selftest: $ok ok, $bad failed"
[ "$bad" = 0 ]
