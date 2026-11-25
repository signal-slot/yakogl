#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of mp2-log-redact-check.sh. Fixtures use RFC 5737 addresses for
# the benign cases; the hit cases use made-up values that are not real hosts.
set -u
d=$(cd "$(dirname "$0")" && pwd)
chk="$d/mp2-log-redact-check.sh"
t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
fail=0
ok() { echo "ok   $1"; }
ko() { echo "FAIL $1"; fail=1; }

grep -qF '([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|ttyUSB|ttyACM|tailnet|\.ts\.net' "$chk" \
  && ok c6-pattern-present || ko c6-pattern-present

cat > "$t/clean.log" <<'X'
Vivante GC7000 rev 6214, libGAL 6.4.9.1
firmware 2.12.0.1 built; spec 4.6.2.14; v1.4.6.2.555
board 192.0.2.7 and 198.51.100.9, 203.0.113.1 doc ranges
mask 255.255.255.0 any 0.0.0.0 lo 127.0.0.1
mac 00:00:00:00:00:00 bcast ff:ff:ff:ff:ff:ff
gl_test: all: 0 of 1 cases failed
X
"$chk" "$t/clean.log" >"$t/o" 2>&1 && ok clean-passes || { ko clean-passes; cat "$t/o"; }

hit() { # name text class full-match
  printf '%s\n' "$2" > "$t/h.log"
  "$chk" "$t/h.log" >"$t/o" 2>&1; rc=$?
  if [ $rc -eq 1 ] && grep -q "h.log:1: $3: " "$t/o"; then ok "$1"; else ko "$1"; cat "$t/o"; fi
  if grep -qF -- "$4" "$t/o"; then ko "$1-masked"; fi
}
hit ipv4 'host 10.20.30.40 up' ipv4 10.20.30.40
hit tty 'console /dev/ttyUSB3 opened' tty ttyUSB3
hit ttyacm 'opened /dev/ttyACM0' tty ttyACM0
hit tailnet 'peer dk.example.ts.net' tailnet dk.example.ts.net
hit tailnet2 'on tailnet foo' tailnet 'on tailnet'
hit mac 'eth0 02:11:22:33:44:55 up' mac 02:11:22:33:44:55
hit usbserial 'iSerial 3 SerialNumber: AB12CD34EF' usb-serial AB12CD34EF
hit byid 'usb-FTDI_Dual_RS232-HS-if00' usb-serial usb-FTDI_Dual

mkdir -p "$t/d/sub"; cp "$t/clean.log" "$t/d/sub/a.log"
printf 'x\n10.1.2.3\n' > "$t/d/b.log"
"$chk" "$t/d" >"$t/o" 2>&1; rc=$?
{ [ $rc -eq 1 ] && grep -q 'b.log:2: ipv4: 1\*\*\*' "$t/o"; } && ok dir-scan || { ko dir-scan; cat "$t/o"; }
"$chk" "$t/nonexistent" >/dev/null 2>&1; [ $? -eq 2 ] && ok missing-exit2 || ko missing-exit2

printf '10.20.30.40\n' > "$t/allow"
printf 'host 10.20.30.40\n' > "$t/h.log"
MP2_REDACT_ALLOW="$t/allow" "$chk" "$t/h.log" >/dev/null 2>&1 && ok allowlist || ko allowlist

if [ $fail -eq 0 ]; then echo "mp2_log_redact_check_selftest: all ok"; else echo "mp2_log_redact_check_selftest: FAILED"; exit 1; fi
