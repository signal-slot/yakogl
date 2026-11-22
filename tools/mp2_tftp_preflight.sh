#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Can this host serve the DK's TFTP route (driver/tools/uboot-mp2/mp2-net.cmd,
# tools/tftp_oneshot.py) on UDP port 69 without root? ST's U-Boot ignores
# tftpdstp, so the server must own port 69 (docs/notes/stm32mp257f-dk-uboot-boot.md).
#
#   tools/mp2_tftp_preflight.sh
#
# Reports: the net.ipv4.ip_unprivileged_port_start value; a test bind of UDP 69
# by an unprivileged `python3 -I` socket (released at once); a one-file
# tools/tftp_oneshot.py start/serve/stop on port 69 to a localhost client.
# Last line: "mp2_tftp_preflight: ready" (exit 0) or
# "mp2_tftp_preflight: not ready: <why>" (exit 1).
# Never changes sysctl or the firewall, never calls sudo. The firewall cannot
# be read without root: the host's UDP 69 and ephemeral data ports must be
# open for the board (not checked here).
# Test overrides (selftest): MP2_PREFLIGHT_SYSCTL=<number|unreadable>,
# MP2_PREFLIGHT_BIND=ok|fail, MP2_PREFLIGHT_SERVE=ok|fail.
set -u
here=$(cd "$(dirname "$0")" && pwd)
port=69
why=()
say() { echo "mp2_tftp_preflight: info: $*"; }

# 1. sysctl
if [ -n "${MP2_PREFLIGHT_SYSCTL:-}" ]; then
  sc=$MP2_PREFLIGHT_SYSCTL
else
  sc=$(cat /proc/sys/net/ipv4/ip_unprivileged_port_start 2>/dev/null || echo unreadable)
fi
say "net.ipv4.ip_unprivileged_port_start=$sc"
case $sc in
  ''|*[!0-9]*) why+=("sysctl unreadable") ;;
  *) [ "$sc" -le $port ] || why+=("net.ipv4.ip_unprivileged_port_start=$sc is above $port") ;;
esac

# 2. test bind
case "${MP2_PREFLIGHT_BIND:-real}" in
  ok) bind=ok ;;
  fail) bind=fail ;;
  *) if python3 -I -c '
import socket
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.bind(("0.0.0.0", 69))
s.close()' 2>/dev/null; then bind=ok; else bind=fail; fi ;;
esac
say "unprivileged bind of UDP $port: $bind"
[ "$bind" = ok ] || why+=("unprivileged bind of UDP $port fails")

# 3. oneshot server start/serve/stop (only when the bind works)
if [ "$bind" = ok ]; then
  case "${MP2_PREFLIGHT_SERVE:-real}" in
    ok) serve=ok ;;
    fail) serve=fail ;;
    *)
      tmp=$(mktemp -d "${TMPDIR:-/tmp}/mp2pf.XXXXXX")
      printf 'mp2 preflight dummy\n' > "$tmp/dummy"
      mkdir "$tmp/root"; mv "$tmp/dummy" "$tmp/root/dummy"
      python3 -I "$here/tftp_oneshot.py" --root "$tmp/root" --bind 127.0.0.1 --port $port \
        >"$tmp/server.log" 2>&1 &
      spid=$!
      sleep 1
      cat > "$tmp/client.py" <<'PY'
import socket, sys
want = b"mp2 preflight dummy\n"
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.settimeout(3)
s.sendto(b"\0\1dummy\0octet\0", ("127.0.0.1", 69))
got = b""
while True:
    d, a = s.recvfrom(1024)
    if d[:2] != b"\0\3":
        sys.exit(1)
    got += d[4:]
    s.sendto(b"\0\4" + d[2:4], a)
    if len(d) < 516:
        break
sys.exit(0 if got == want else 1)
PY
      if python3 -I "$tmp/client.py" 2>/dev/null; then serve=ok; else serve=fail; fi
      kill "$spid" 2>/dev/null
      wait "$spid" 2>/dev/null
      rm -rf "$tmp"
      ;;
  esac
  say "tftp_oneshot.py serve on port $port to localhost: $serve"
  [ "$serve" = ok ] || why+=("tftp_oneshot.py could not serve on port $port")
else
  say "tftp_oneshot.py serve: skipped (no bind)"
fi
say "firewall not checked (needs root): UDP $port and the ephemeral data ports must pass"

if [ ${#why[@]} -eq 0 ]; then
  echo "mp2_tftp_preflight: ready"
  exit 0
fi
reason=$(printf '%s; ' "${why[@]}")
echo "mp2_tftp_preflight: not ready: ${reason%; }"
exit 1
