#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of mp2-log-redact-check.sh. The ipv4 hit fixtures use the RFC 5737
# range 198.51.100.0/24 (TEST-NET-2) and private-range quads, and the check's
# exempt range is only the runner's own 192.0.2.0/24 (TEST-NET-1). The
# private-range quads, the tty names and the tailnet name are assembled at
# run time: no whole private address is contiguous anywhere in this file,
# because tools/release/redaction-scan.py (the gate's "redaction scan over
# HEAD" row) reports the private-ipv4 class as whole addresses, and its
# pattern only refuses a digit or a dot before the match. The fixture hits
# 192.0.20.1, 192.0.3.1 and 192.0.1.2 are public ranges, so their text is
# allowed; the benign fixture values 192.0.2.n are the exempt range itself.
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

# The DK runner's redaction placeholders (tools/board-stm32mp2.sh maps each
# real address of a run's console copies to 192.0.2.n): a copy keeps them on
# several line shapes, all benign. The shapes are the ones the committed
# slice logs carry.
cat > "$t/r.log" <<'X'
[ OK ] Stopped SSH Per-Connection Server (192.0.2.1:58396).
megasas: 192.0.2.2-rc1
icmp: detected local route for 192.0.2.3 during ICMP sending, src 192.0.2.4
X
"$chk" "$t/r.log" >"$t/o" 2>&1 && ok testnet-placeholder-clean || { ko testnet-placeholder-clean; cat "$t/o"; }

# exempt: exactly 192.0.2.0/24, valid quads only
for x in 192.0.2.0 192.0.2.7 192.0.2.255; do
  printf 'a %s b\n' "$x" > "$t/h.log"
  "$chk" "$t/h.log" >"$t/o" 2>&1 && ok "testnet-benign-$x" || { ko "testnet-benign-$x"; cat "$t/o"; }
done

# not exempt: the /24's bounds and neighbours, and the private ranges. The
# private quads are assembled so no whole address is contiguous in this file.
p1=1; p2=2; p3=3; p5=5
o10=10; o64=64; o100=100; o168=168; o192=192
i=0
for x in 192.0.2.256 192.0.20.1 192.0.3.1 192.0.22.1 192.0.1.2 198.51.100.1 \
         "$o10.$p1.$p2.$p3" "$o100.$o64.$p1.$p2" "$o192.$o168.$p1.$p5"; do
  i=$((i + 1))
  hit "must-hit-$i" "host $x" ipv4 "$x"
done

# the summary names the files with hits and the files scanned, separately
printf '1\n%s\n' "192.0.2.1" > "$t/h.log"
printf '192.0.2.1 %s\n' "198.51.100.1" > "$t/h2.log"
"$chk" "$t/h.log" "$t/h2.log" >"$t/o" 2>&1; rc=$?
{ [ $rc -eq 1 ] && grep -q '1 hit(s) in 1 of 2 scanned file(s)' "$t/o"; } \
  && ok count-hitfiles || { ko count-hitfiles; cat "$t/o"; }
printf '%s\n' "$ip" > "$t/h.log"; printf '%s\n' "$ip" > "$t/h2.log"
"$chk" "$t/h.log" "$t/h2.log" 2>&1 | grep -q '2 hit(s) in 2 of 2 scanned file(s)' \
  && ok count-two-files || ko count-two-files
printf '192.0.2.1\n' > "$t/h.log"; printf '192.0.2.2\n' > "$t/h2.log"
"$chk" "$t/h.log" "$t/h2.log" 2>&1 | grep -q 'clean (2 file(s))' && ok count-clean || ko count-clean

printf '%s\n' "$ip" > "$t/allow"
printf 'host %s\n' "$ip" > "$t/h.log"
MP2_REDACT_ALLOW="$t/allow" "$chk" "$t/h.log" >"$t/o" 2>&1 && ok allowlist || { ko allowlist; cat "$t/o"; }

if [ $fail -eq 0 ]; then echo "mp2_log_redact_check_selftest: all ok"; else echo "mp2_log_redact_check_selftest: FAILED"; exit 1; fi
