#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of mp2-log-redact-check.sh. Every address in a fixture is in an
# RFC 5737 documentation range or a private/CGNAT example (10.1.2.3, 100.64.0.5,
# 192.168.1.5, 192.0.20.1: obviously fake); the hit strings are assembled at
# run time, so this file holds no literal tty or tailnet name.
set -u
d=$(cd "$(dirname "$0")" && pwd)
chk="$d/mp2-log-redact-check.sh"
t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
fail=0
ok() { echo "ok   $1"; }
ko() { echo "FAIL $1"; fail=1; }

c6='([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|ttyUSB|ttyACM|tailnet|\.ts\.net'  # redact-scan: ok
grep -qF -- "$c6" "$chk" && ok c6-pattern-present || ko c6-pattern-present

cat > "$t/clean.log" <<'X'
Vivante GC7000 rev 6214, libGAL 6.4.9.1
firmware 2.12.0.1 built; spec 4.6.2.14; v1.4.6.2.555
mask 255.255.255.0 any 0.0.0.0 lo 127.0.0.1  # redact-scan: ok
mac 00:00:00:00:00:00 bcast ff:ff:ff:ff:ff:ff
gl_test: all: 0 of 1 cases failed
X
"$chk" "$t/clean.log" >"$t/o" 2>&1 && ok clean-passes || { ko clean-passes; cat "$t/o"; }

hit() { # name text class unmasked-part
  printf '%s\n' "$2" > "$t/h.log"
  "$chk" "$t/h.log" >"$t/o" 2>&1; rc=$?
  if [ $rc -eq 1 ] && grep -q "h.log:1: $3: " "$t/o"; then ok "$1"; else ko "$1"; cat "$t/o"; fi
  if grep -qF -- "$4" "$t/o"; then ko "$1-masked"; fi
}
ip=198.51.100.44
dev=/dev/tty
hit ipv4 "host $ip up" ipv4 "$ip"
hit tty "console ${dev}USB3 opened" tty USB3
hit ttyacm "opened ${dev}ACM0" tty ACM0
hit tailnet "peer dk.example.ts"".net" tailnet dk.example
hit tailnet2 "on tail""net foo" tailnet 'on tailnet'
hit mac 'eth0 02:11:22:33:44:55 up' mac 02:11:22:33:44:55  # redact-scan: ok
hit usbserial 'iSerial 3 SerialNumber: AB12CD34EF' usb-serial AB12CD34EF
hit byid 'usb-FTDI_Dual_RS232-HS-if00' usb-serial usb-FTDI_Dual

mkdir -p "$t/d/sub"; cp "$t/clean.log" "$t/d/sub/a.log"
printf 'x\n%s\n' "203.0.113.9" > "$t/d/b.log"
"$chk" "$t/d" >"$t/o" 2>&1; rc=$?
{ [ $rc -eq 1 ] && grep -q 'b.log:2: ipv4: 2\*\*\*' "$t/o"; } && ok dir-scan || { ko dir-scan; cat "$t/o"; }
"$chk" "$t/nonexistent" >/dev/null 2>&1; [ $? -eq 2 ] && ok missing-exit2 || ko missing-exit2

printf '%s\n' "$ip" > "$t/allow"
printf 'host %s\n' "$ip" > "$t/h.log"
MP2_REDACT_ALLOW="$t/allow" "$chk" "$t/h.log" >/dev/null 2>&1 && ok allowlist || ko allowlist

# 192.0.2.0/24 is the runner's placeholder range: benign, and only that range
printf 'a 192.0.2.7 b\n' > "$t/p.log"
"$chk" "$t/p.log" >"$t/o" 2>&1 && ok testnet-passes || { ko testnet-passes; cat "$t/o"; }
printf 'a 192.0.2.7 b 10.1.2.3\n' > "$t/p.log"
"$chk" "$t/p.log" >"$t/o" 2>&1; rc=$?
{ [ $rc -eq 1 ] && grep -q ': 1 hit(s) in 1 file(s)' "$t/o"; } && ok testnet-plus-10 || { ko testnet-plus-10; cat "$t/o"; }
for a in 192.0.20.1 192.0.3.1 192.168.1.5 100.64.0.5 10.1.2.3; do
  printf 'a %s b\n' "$a" > "$t/p.log"
  "$chk" "$t/p.log" >"$t/o" 2>&1; rc=$?
  { [ $rc -eq 1 ] && grep -q 'p.log:1: ipv4: ' "$t/o"; } && ok "hit-$a" || { ko "hit-$a"; cat "$t/o"; }
done
mkdir -p "$t/two"
printf 'x 10.1.2.3\ny 10.1.2.4 z 10.1.2.5\n' > "$t/two/a.log"
printf 'x 10.1.2.6\n' > "$t/two/b.log"
printf 'clean\n' > "$t/two/c.log"
"$chk" "$t/two" >"$t/o" 2>&1; rc=$?
{ [ $rc -eq 1 ] && grep -q ': 4 hit(s) in 2 file(s) (3 scanned)' "$t/o"; } && ok file-count-unique || { ko file-count-unique; cat "$t/o"; }

if [ $fail -eq 0 ]; then echo "mp2_log_redact_check_selftest: all ok"; else echo "mp2_log_redact_check_selftest: FAILED"; exit 1; fi
