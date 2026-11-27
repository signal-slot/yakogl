#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# t6-redact-check-selftest.sh - tools/cts/t6-redact-check.sh without a board
# (plan #408, item t6-redact-check).
#
#   bash tools/cts/t6-redact-check-selftest.sh
#
# Synthetic positives (made-up values only: the RFC 5737 test-net ranges,
# the RFC 3849 documentation prefix, locally administered MACs, made-up
# names) are each flagged with their kind and never echoed (a kernel boot
# id line is the one UUID that passes: random per boot); a U-Boot/Zephyr/
# Armbian console sample (load addresses, hex device unit names, sha256
# sums, version strings, timestamps, masked placeholders, loopback)
# produces no finding. Partly redacted tailnet spellings (the first two
# octets of the 100.64/10 range, dots and all, with the rest replaced) are
# refused, the way the plan's acceptance grep c3 refuses them; every c3
# alternative the grep flags, the tool flags too. An unlistable directory
# and an unreadable file (chmod 000) count as findings of that path, so a
# run that could not look cannot report clean (skipped when run as root,
# which reads anything). The environment values and the REDACT_EXTRA file
# are refused and never printed; the exit codes are 0 clean, 1 finding,
# 2 usage. No sample token appears whole in this file: samples are built
# from pieces at run time, so this file and the tool pass the acceptance
# grep c3 and the tool itself (checked at the end, like
# tools/cts/console-redact-selftest.sh does for its acceptance grep).
#
# Host only; scratch under ${TMPDIR:-$PWD}, never /tmp.
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tool="$here/t6-redact-check.sh"
self="$here/t6-redact-check-selftest.sh"
case "${TMPDIR:-$PWD}" in /tmp | /tmp/* | /var/tmp | /var/tmp/*)
  echo "refusing a scratch dir under /tmp; set TMPDIR" >&2
  exit 2
  ;;
esac
T=$(mktemp -d "${TMPDIR:-$PWD}/t6-redact-check.XXXXXX")
trap 'chmod -R u+rwX "$T" 2>/dev/null; rm -rf "$T"' EXIT HUP INT TERM
fails=0
fail() {
  echo "FAIL: $*" >&2
  fails=$((fails + 1))
}

# env -u: the caller's own board environment must not leak into the cases
run() {
  env -u BOARD_HOST -u BOARD_USER -u BOARD_UART -u T6_UART -u T6_HOST \
    -u REDACT_EXTRA bash "$tool" "$@"
}

# samples built from pieces (no token whole in this file)
BR='rfc2217'"://"
UA='/dev/''tty''USB0'
ACM='/dev/''tty''ACM3'
UBARE='/dev/''tty''USB'
MAC='02:00:5e:''10:00:01'
MACD='00-11-22-''33-44-55'
TN='100'""'.64.0.9'
TN_PART='100'""'.64.1.x'
TN_PART2='100'""'.101.x.x'
TN_SUB='1100'""'.75.9'
TSNET='zznano.example'"".ts.net
QUAD='192.0'""'.2.7'
QUAD2='198.51.100'""'.7'
V6='2001:db8'""'::2'
V6F='2001:db8:0:0:0:0'""':0:2'
UUID='12345678-1234'""'-1234-1234-123456789abc'
SER1='iSerial                 3 AB'""'12CD34EF'
SER2='SerialNumber'""'=ZX81Q9'
SER3='Serial Number: zz'""'serial1'
HOSTP='root'""'@zznano:~#'
HOSTM='ops'""'@zzmail'
HOSTU='serial-getty'""'@ttyS2.service'
SERPATH='/dev/ser'""'ial/by-id/usb-FTDI_Dual_RS232-HS-if00-port0'
BIDLINE='before time=2026-11-25T10:11:12Z boot_id=a33b88ec-e886-410e'""'-b70d-45ec4b55ac5d dmesg_lines=918'
UUIDDISK='uuid_disk='

pos() { # pos <name> <kind> <line> <fragment that must not be echoed>
  printf '%s\n' "$3" > "$T/p.txt"
  local out rc=0
  out=$(run "$T/p.txt") || rc=$?
  [ "$rc" = 1 ] || fail "$1: exit $rc, want 1"
  printf '%s\n' "$out" | grep -Eq ": $2\$" || fail "$1: no $2 finding: $out"
  case "$out" in *"$4"*) fail "$1: match echoed: $out" ;; esac
}
pos rfc2217 rfc2217 "console $BR/z:2001/0" "$BR"
pos uart uart "console $UA 115200" "$UA"
pos uart-acm uart "console on $ACM" "$ACM"
pos uart-bare uart "mounted at $UBARE with no digits" "$UBARE"
pos mac mac "ether $MAC txqueuelen 1000" "$MAC"
pos mac-dash mac-dash "ethaddr $MACD" "$MACD"
pos tailnet tailnet "peer $TN up" "$TN"
pos tailnet-part tailnet "peer $TN_PART (redacted host part)" "$TN_PART"
pos tailnet-part2 tailnet "host $TN_PART2 up" "$TN_PART2"
pos tailnet-sub tailnet "clock 100 $TN_SUB ticks" "$TN_SUB"
pos tailnet-name tailnet-name "ssh $TSNET" ".ts.net"
pos ipv4 ipv4 "link up at $QUAD" "$QUAD"
pos ipv4-scheme ipv4 "tftp://$QUAD2/boot, then done." "$QUAD2"
pos ipv6 ipv6-colons "inet6 $V6/64 scope global" "$V6"
pos ipv6-full ipv6-full "addr $V6F" "0:0:0:0:0:2"
pos uuid uuid "uuid $UUID" "$UUID"
pos uuid-disk uuid "$UUIDDISK$UUID" "$UUID"
neg() { # neg <name> <line>: no finding, exit 0
  printf '%s\n' "$2" > "$T/n.txt"
  local out rc=0
  out=$(run "$T/n.txt") || rc=$?
  [ "$rc" = 0 ] && [ -z "$out" ] || fail "$1: flagged: $out"
}
neg boot-id "$BIDLINE"
neg boot-id-prose "- **Boot id:** $UUID before and after (no reboot)."
UBSER='serial''#=0123456789abcdef'
pos uboot-id uboot-id "$UBSER" "0123456789abcdef"
pos uboot-id-colon uboot-id "serial""#: 0123456789abcdef" "0123456789abcdef"
neg uboot-id-redacted "serial""#=<serial>"
neg uboot-id-empty "serial""#="
pos serial-lsusb usb-serial "$SER1" "12CD34EF"
pos serial-equals usb-serial "$SER2" "ZX81Q9"
pos serial-colon usb-serial "$SER3" "zzserial1"
pos userhost userhost "$HOSTP" "zznano"
pos userhost-mail userhost "write to $HOSTM today" "zzmail"
pos userhost-unit userhost "unit $HOSTU (fail safe, as in mali-slice-redact)" "ttyS2.service"
pos serial-path serial-path "ls $SERPATH" "by-id"

# two kinds on one line: each reported once, neither echoed
printf 'link %s at %s\n' "$MAC" "$QUAD" > "$T/p.txt"
out=$(run "$T/p.txt")
rc=$?
if [ "$rc" = 1 ] && [ "$(printf '%s\n' "$out" | grep -c ': mac$')" = 1 ] &&
  [ "$(printf '%s\n' "$out" | grep -c ': ipv4$')" = 1 ]; then :; else
  fail "two kinds on one line: rc $rc out $out"
fi

# binary file: the plan's acceptance grep scans binaries too, so do we
printf 'x\0y %s\n' "$QUAD" > "$T/b.dat"
out=$(run "$T/b.dat")
rc=$?
if [ "$rc" = 1 ]; then
  case "$out" in (*":1: ipv4") : ;; (*) fail "binary: $out" ;; esac
else
  fail "binary not flagged: rc $rc"
fi
case "$out" in (*"$QUAD"*) fail "binary match echoed" ;; esac

# directory scan: nested and hidden files reached, clean dirs pass
mkdir -p "$T/d/sub" "$T/empty"
printf 'ok line\n' > "$T/d/a.txt"
printf 'peer %s\n' "$TN" > "$T/d/sub/b.txt"
printf 'console %s\n' "$UA" > "$T/d/.hidden"
out=$(run "$T/d")
rc=$?
[ "$rc" = 1 ] || fail "dir scan: rc $rc out $out"
case "$out" in *"sub/b.txt:1: tailnet"*) : ;; (*) fail "dir scan: $out" ;; esac
case "$out" in *".hidden:1: uart"*) : ;; (*) fail "hidden file skipped: $out" ;; esac
case "$out" in (*"$TN"* | *"$UA"*) fail "dir scan echoed a match" ;; esac
out=$(run "$T/empty")
[ $? = 0 ] || fail "empty dir not clean"

# the clean console sample: U-Boot, Zephyr and Armbian lines that must pass
cat > "$T/uboot.txt" <<'X'
U-Boot 2017.09-armbian (Nov 12 2026 - 10:11:12 +0000)
Model: FriendlyElec NanoPC-T6 (RK3588)
Hit any key to stop autoboot:  0
DRAM:  4 GiB
MMC:   dwmmc@fe310000: 1 (SD), dwmmc@fe2b0000: 0
## Loading kernel from FIT Image at 0x01000000
   Loading kernel from 0x01000000 to 0x11000000
clk_summary: aclk 1000000000 hclk 200000000 pclk 100000000
watchdog0@fedc0000 armed, timeout 60 s
panthor fff00000.gpu: Mali-G610 r0p0
uuid_gpt_disk=
ethaddr masked to <mac>
prompt masked to root@<host> for the record
## Starting application at 0x1000000 ...
*** Booting Zephyr OS build v4.0.0-7-g1234567890ab ***
uart:~$ yk-record: run over, uptime 412 s
dEQP Core vulkan-cts-1.4.6.2-555-g93bca01861b0e3ef starting..
mesa-24.0.8.1 and v1.2.3.4 and 1.2.3.4.5 and 300.1.2.3 and BVNC 36.53.104.796
commit 8ff010556066248dca5e7a6fbb5ca6c8e2c8e01
1f3870be274f6c49b3e31a0c6728957f  Image
loopback 127.0.0.1 any 0.0.0.0 and :: and ::1
time 12:34:56.789 and 2026-11-25T10:11:12Z and std::string
pci 0000:00:00.0 class code 0x060400
range note: the tailnet is 100.64/10
masked: <addr> <mac> <uart> <host> <redacted:ipv4> <redacted:uuid>
Rockchip# mmc list
dwmmc@fe2b0000
X
printf '%s\n' "$BIDLINE" >> "$T/uboot.txt"
out=$(run "$T/uboot.txt")
if [ $? != 0 ]; then
  echo "stdout: $out" >&2
  fail "U-Boot sample flagged"
fi

# fail closed: an unlistable directory and an unreadable file are findings
if [ "$(id -u)" = 0 ]; then
  echo "note: running as root, the chmod 000 cases skipped"
else
  mkdir -p "$T/lock/sub"
  printf 'peer %s\n' "$TN" > "$T/lock/sub/secret.txt"
  printf 'ok\n' > "$T/lock/visible.txt"
  chmod 000 "$T/lock/sub"
  out=$(run "$T/lock")
  rc=$?
  [ "$rc" = 1 ] || fail "locked dir: rc $rc out $out"
  case "$out" in (*":0: unreadable") : ;; (*) fail "locked dir kind: $out" ;; esac
  case "$out" in (*"$TN"*) fail "locked dir leaked its content" ;; esac
  chmod 755 "$T/lock/sub"

  printf 'peer %s\n' "$QUAD" > "$T/nr.txt"
  chmod 000 "$T/nr.txt"
  out=$(run "$T/nr.txt")
  rc=$?
  [ "$rc" = 1 ] || fail "unreadable file: rc $rc"
  case "$out" in (*":0: unreadable") : ;; (*) fail "unreadable file kind: $out" ;; esac
  case "$out" in (*"$QUAD"*) fail "unreadable file leaked its content" ;; esac
  chmod 644 "$T/nr.txt"

  mkdir "$T/toplocked"
  chmod 000 "$T/toplocked"
  out=$(run "$T/toplocked")
  [ $? = 1 ] || fail "unlistable top directory not refused"
  chmod 755 "$T/toplocked"
fi

# environment values: refused by the caller's own environment, never printed
printf 'on %s via %s\n' zzhost9 zzser9 > "$T/e1.txt"
out=$(env -u BOARD_USER -u T6_UART -u T6_HOST -u REDACT_EXTRA \
  BOARD_HOST=zzhost9 BOARD_UART=zzser9 bash "$tool" "$T/e1.txt")
rc=$?
[ "$rc" = 1 ] || fail "env values: rc $rc out $out"
case "$out" in (*env:BOARD_HOST*) : ;; (*) fail "env:BOARD_HOST not flagged: $out" ;; esac
case "$out" in (*env:BOARD_UART*) : ;; (*) fail "env:BOARD_UART not flagged: $out" ;; esac
case "$out" in (*zzhost9* | *zzser9*) fail "env value echoed: $out" ;; esac
# a value of under three characters is not searched (would flag everything)
printf 'plain zz line\n' > "$T/e2.txt"
out=$(env -u BOARD_UART -u BOARD_USER -u T6_UART -u T6_HOST -u REDACT_EXTRA \
  BOARD_HOST=zz bash "$tool" "$T/e2.txt")
[ $? = 0 ] || fail "short env value searched: $out"

# REDACT_EXTRA: fixed strings from a file, blank lines ignored
printf 'zzextra9\n\n' > "$T/extra.pats"
printf 'note zzextra9 here\n' > "$T/x1.txt"
out=$(env REDACT_EXTRA="$T/extra.pats" bash "$tool" "$T/x1.txt")
rc=$?
[ "$rc" = 1 ] || fail "extra: rc $rc"
case "$out" in (*": extra") : ;; (*) fail "extra kind: $out" ;; esac
case "$out" in (*zzextra9*) fail "extra value echoed" ;; esac
printf 'plain\n' > "$T/x2.txt"
env REDACT_EXTRA="$T/extra.pats" bash "$tool" "$T/x2.txt" >/dev/null
[ $? = 0 ] || fail "the blank line in the extra file matched everything"
env REDACT_EXTRA="$T/nothere" bash "$tool" "$T/x2.txt" >/dev/null 2>&1
[ $? = 2 ] || fail "REDACT_EXTRA naming no file is not a usage error"

# exit codes and refusals
run >/dev/null 2>&1
[ $? = 2 ] || fail "no arguments is not usage"
run "$T/nonexistent" >/dev/null 2>&1
[ $? = 2 ] || fail "missing path is not usage"
ln -sf "$T/d/a.txt" "$T/link.txt"
run "$T/link.txt" >/dev/null 2>&1
[ $? = 2 ] || fail "a symbolic link argument is not refused"
out=$(run -h)
[ $? = 0 ] || fail "--help does not print the usage"

# the plan's acceptance grep c3: everything it flags, the tool flags too
p1='rfc2217'"://"
p2='/dev/''tty(USB|ACM)'
c3="$p1|$p2|([0-9a-f]{2}:){5}[0-9a-f]{2}|100\.(6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\."
for s in "$BR""z:2001/0" "$UA" "$ACM" "$UBARE" "$MAC" "$TN" "$TN_PART" "$TN_PART2" "$TN_SUB"; do
  printf '%s\n' "$s" > "$T/c3.txt"
  if grep -Eiq -- "$c3" "$T/c3.txt"; then
    if run "$T/c3.txt" >/dev/null 2>&1; then
      fail "the acceptance grep flags a sample the tool passes"
    fi
  else
    fail "the sample no longer trips the acceptance grep"
  fi
done

# the two files pass their own rules and the acceptance grep c3
for f in "$tool" "$self"; do
  if grep -Eiq -- "$c3" "$f"; then
    fail "a source file trips the acceptance grep"
  fi
  out=$(run "$f")
  if [ $? != 0 ]; then
    echo "stdout: $out" >&2
    fail "the tool flags one of its own two files"
  fi
done

if [ "$fails" -ne 0 ]; then
  echo "t6-redact-check selftest: $fails failure(s)" >&2
  exit 1
fi
echo "t6-redact-check selftest: ok"
