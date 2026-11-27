#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx8mp-gles3-redact.py: --check reports file:line and pattern name without the matched text and
# exits 1, a clean tree exits 0 with 'redact: <n> files clean', --fix rewrites to <tty>/<addr>/<login>, the
# patterns equal the acceptance grep's, and the committed tools/cts/log/*-imx8mp-* directories are clean.
# Samples are built by concatenation. Host only; scratch under ${TMPDIR:-$PWD}, never /tmp.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/imx8mp-gles3-redact.py
case "${TMPDIR:-$PWD}" in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "refusing a scratch dir under /tmp; set TMPDIR" >&2; exit 2 ;; esac
tmp=$(mktemp -d "${TMPDIR:-$PWD}/gles3-redact-selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
nfail=0 nok=0
ok() { echo "ok: $*"; nok=$((nok + 1)); }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }

tty="/dev/""ttyUSB0"
acm="/dev/""ttyACM1"
addr="10.1.2"".3"
addrp="192.168.0"".7:2222"
login="abc""@""host.example"

mkdir -p "$tmp/clean/sub" "$tmp/dirty"
printf 'Pass 12\nFail 0\nversion 1.2 and /dev/null\n' > "$tmp/clean/a.txt"
printf 'ok\n' > "$tmp/clean/sub/b.txt"
printf '\0\1\2%s\n' "$addr" > "$tmp/clean/bin.dat"
printf 'line1\nport %s here\nx\n' "$tty" > "$tmp/dirty/t.txt"
printf '%s\nconnect %s and %s\n' "$acm" "$addr" "$addrp" > "$tmp/dirty/u.txt"
printf 'mail %s ok\n' "$login" > "$tmp/dirty/l.txt"

out=$(python3 -I "$tool" --check "$tmp/clean" 2>&1); rc=$?
if [ $rc -eq 0 ] && [ "$out" = "redact: 2 files clean" ]; then ok "clean tree, binary skipped"; else bad "clean: rc=$rc out=$out"; fi

out=$(python3 -I "$tool" --check "$tmp/dirty" 2>&1); rc=$?
if [ $rc -eq 1 ]; then ok "dirty tree exits 1"; else bad "dirty rc=$rc"; fi
for want in "t.txt:2: tty" "u.txt:1: tty" "u.txt:2: addr" "l.txt:1: login"; do
    if printf '%s\n' "$out" | grep -qF -- "$want"; then ok "reports $want"; else bad "missing $want in: $out"; fi
done
for leak in "$tty" "$acm" "$addr" "192.168" "abc@" "host.exa"; do
    if printf '%s\n' "$out" | grep -qF -- "$leak"; then bad "output leaks matched text"; else ok "no leak"; fi
done

python3 -I "$tool" --fix "$tmp/dirty" >/dev/null 2>&1; rc=$?
if [ $rc -eq 0 ]; then ok "--fix exits 0"; else bad "--fix rc=$rc"; fi
cat "$tmp/dirty/t.txt" "$tmp/dirty/u.txt" "$tmp/dirty/l.txt" > "$tmp/all"
for want in "port <tty> here" "connect <addr> and <addr>" "mail <login> ok"; do
    if grep -qF -- "$want" "$tmp/all"; then ok "fixed: $want"; else bad "fix lacks: $want"; fi
done
if python3 -I "$tool" --check "$tmp/dirty" >/dev/null 2>&1; then ok "clean after --fix"; else bad "still dirty after --fix"; fi

# the patterns are the acceptance grep's: byte-identical text, same verdict on samples
acc='/dev/tty(USB|ACM)|([0-9]{1,3}\.){3}[0-9]{1,3}(:[0-9]+)?|[A-Za-z0-9._-]+@[A-Za-z0-9-]+\.[A-Za-z]'
pyacc=$(python3 -I -c '
import importlib.util, sys
s = importlib.util.spec_from_file_location("r", sys.argv[1])
m = importlib.util.module_from_spec(s)
s.loader.exec_module(m)
print("|".join(p for _, p, _ in m.PATTERNS))' "$tool")
if [ "$pyacc" = "$acc" ]; then ok "patterns byte-identical to the acceptance grep"; else bad "pattern text differs"; fi
mkdir -p "$tmp/m"   # (a dotless a@b is flagged by the X2 login pattern, beyond the grep, so it is not a sample here)
for s in "$tty" "$acm" "$addr" "$addrp" "$login" "1.2.3" "x/dev/ttyS0"; do
    printf '%s\n' "$s" > "$tmp/m/s.txt"
    grep -EIq -- "$acc" "$tmp/m/s.txt"; g=$?
    python3 -I "$tool" --check "$tmp/m" >/dev/null 2>&1; p=$?
    if [ $((g + p)) -eq 1 ]; then ok "grep and tool agree"; else bad "grep/tool differ (g=$g p=$p)"; fi
done

# Armadillo-X2 shapes: each is flagged under its pattern name, none leaks into the output, --fix cleans them
# and legitimate X2 content is not flagged. Samples are concatenated so this file holds no real value.
mkdir -p "$tmp/x2" "$tmp/x2ok"
printf 'ssh root''@''armadillo-x2 up\n'                      > "$tmp/x2/login.txt"
printf 'host x2-lab.tail1a2b3c.ts''.net ok\n'                > "$tmp/x2/tailnet.txt"
printf 'peer 100.64.''12.34 up\n'                            > "$tmp/x2/cgnat.txt"
printf 'ip fd7a:115c:''a1e0::1234 up\n'                      > "$tmp/x2/ipv6.txt"
printf 'ip 2001:db8:0:0:''0:0:0:1 up\n'                      > "$tmp/x2/ipv6b.txt"
printf 'ether 00:11:22:''33:44:55 up\n'                      > "$tmp/x2/mac.txt"
printf 'console /dev/serial/by-id/usb-FTDI_FT232R_A50285BI-if00-port0\n' > "$tmp/x2/byid.txt"
printf 'usb-FTDI_FT232R_A50285BI-''if00-port0\n'             > "$tmp/x2/usbname.txt"
printf 'iSerial 3 A50285BI\n'                                > "$tmp/x2/iserial.txt"
printf 'ID_SERIAL_SHORT=A50285BI\n'                          > "$tmp/x2/idserial.txt"
printf 'port rfc2217:''//console-host:4001\n'                > "$tmp/x2/rfc.txt"
printf 'uart /dev/''ttymxc1 opened\n'                            > "$tmp/x2/mxc.txt"
printf 'UNDO: podman start demo''-app-7\n'                    > "$tmp/x2/undo.txt"
printf 'undo=podman stop -t 5 demo''-app-7\n'                 > "$tmp/x2/stop.txt"
printf 'verdict=ok containers_stopped=demo''-a,demo-b\n'      > "$tmp/x2/cstop.txt"
printf 'app_back=demo''-app-7 running again after the jump\n' > "$tmp/x2/back.txt"
printf 'demohost''-x2:~# uname\n'                             > "$tmp/x2/prompt.txt"
printf 'demohost''-x2 login: \n'                              > "$tmp/x2/getty.txt"
out=$(python3 -I "$tool" --check "$tmp/x2" 2>&1); rc=$?
if [ $rc -eq 1 ]; then ok "X2 tree exits 1"; else bad "X2 rc=$rc"; fi
for want in "login.txt:1: login" "tailnet.txt:1: tailnet" "cgnat.txt:1: addr" "ipv6.txt:1: ipv6" "ipv6b.txt:1: ipv6" \
            "mac.txt:1: mac" "byid.txt:1: tty" "usbname.txt:1: usbserial" "iserial.txt:1: usbserial" \
            "idserial.txt:1: usbserial" "rfc.txt:1: url" "mxc.txt:1: ttymxc" "undo.txt:1: container" \
            "stop.txt:1: container" "cstop.txt:1: container" "back.txt:1: container" "prompt.txt:1: prompt" \
            "getty.txt:1: prompt"; do
    if printf '%s\n' "$out" | grep -qF -- "$want"; then ok "X2 reports $want"; else bad "X2 missing $want"; fi
done
for leak in armadillo-x2 tail1a2b3c 100.64 fd7a 2001:db8 00:11:22 A50285BI console-host ttymxc1 demo-app demo-a demohost; do
    if printf '%s\n' "$out" | grep -qF -- "$leak"; then bad "X2 output leaks $leak"; else ok "X2 no leak of $leak"; fi
done
python3 -I "$tool" --fix "$tmp/x2" >/dev/null 2>&1
if python3 -I "$tool" --check "$tmp/x2" >/dev/null 2>&1; then ok "X2 clean after --fix"; else bad "X2 still dirty after --fix"; fi
if grep -rqE 'armadillo-x2|tail1a2b3c|A50285BI|console-host|fd7a|00:11:22|demo-app|demohost|ttymxc1' "$tmp/x2"; then bad "X2 --fix left a tail"; else ok "X2 --fix left no tail"; fi
cat > "$tmp/x2ok/a.txt" <<'EOT'
staging in /tmp/yakogl-kexec, window 0x60000000 size 0x10000000
record words 0xdeadbeef 00000000 ffffffff
sha256 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
time 12:34:56 std::vector dEQP-GLES3.functional.shaders.x
console=ttymxc1,115200 root=/dev/mmcblk2p2
UNDO: podman start <name>   # one line per name listed
app_back not read: Linux did not come back
Last login: Mon on ttyS0
EOT
if python3 -I "$tool" --check "$tmp/x2ok" >/dev/null 2>&1; then ok "legitimate X2 content not flagged"; else bad "legitimate X2 content flagged"; fi

# the c8 grep finds nothing in a --fix output of the X2 samples
if grep -rEIn '/dev/tty(USB|ACM)|([0-9]{1,3}\.){3}[0-9]{1,3}(:[0-9]+)?|[A-Za-z0-9._-]+@[A-Za-z0-9-]+\.[A-Za-z]' "$tmp/x2" >/dev/null; then
    bad "c8 grep finds a hit after --fix of X2 samples"; else ok "c8 grep clean after --fix of X2 samples"; fi

# committed i.MX8MP logs, if any
set -- "$here"/log/*-imx8mp-*
if [ -e "$1" ]; then
    if python3 -I "$tool" --check "$@" >/dev/null; then ok "committed i.MX8MP logs clean"; else bad "committed i.MX8MP logs have hits"; fi
else
    ok "no committed i.MX8MP logs yet"
fi

echo "redact selftest: $nok ok, $nfail failed"
[ $nfail -eq 0 ]
