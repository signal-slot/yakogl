#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of mp2-log-redact-check.sh. Hit addresses are in the RFC 5737 range
# 198.51.100.0/24 (not exempt); private-range addresses and the tty and
# tailnet names are assembled at run time, so this file holds none literally.
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
printf 'x\n%s\n' "198.51.100.9" > "$t/d/b.log"
"$chk" "$t/d" >"$t/o" 2>&1; rc=$?
{ [ $rc -eq 1 ] && grep -q 'b.log:2: ipv4: 1\*\*\*' "$t/o"; } && ok dir-scan || { ko dir-scan; cat "$t/o"; }
"$chk" "$t/nonexistent" >/dev/null 2>&1; [ $? -eq 2 ] && ok missing-exit2 || ko missing-exit2

# the runner's placeholders 192.0.2.0/24 are benign, and only that /24
printf 'a 192.0.2.7 b 192.0.2.255 c 192.0.2.0\n' > "$t/h.log"
"$chk" "$t/h.log" >/dev/null 2>&1 && ok rfc5737-benign || ko rfc5737-benign
p10=10; p100=100; p192=192
q=192.0
for x in $q.20.1 $q.3.1 $q.22.1 $q.1.2 198.51.100.1 \
         "$p10.1.2.3" "$p100.64.1.2" "$p192.168.1.5"; do
  printf 'host %s\n' "$x" > "$t/h.log"
  "$chk" "$t/h.log" >/dev/null 2>&1; [ $? -eq 1 ] && ok "hit-${x%%.*}.${x#*.}" || ko "hit-$x"
done
printf '1\n192.0.2.1\n' > "$t/h.log"; printf '192.0.2.1 198.51.100.1\n' > "$t/h2.log"
"$chk" "$t/h.log" "$t/h2.log" >"$t/o" 2>&1
grep -q '1 hit(s) in 1 of 2 scanned' "$t/o" && ok count-files-with-hits || { ko count-files-with-hits; cat "$t/o"; }

printf '%s\n' "$ip" > "$t/allow"
printf 'host %s\n' "$ip" > "$t/h.log"
MP2_REDACT_ALLOW="$t/allow" "$chk" "$t/h.log" >/dev/null 2>&1 && ok allowlist || ko allowlist

if [ $fail -eq 0 ]; then echo "mp2_log_redact_check_selftest: all ok"; else echo "mp2_log_redact_check_selftest: FAILED"; exit 1; fi
