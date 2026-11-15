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
login="ab""c@host.exa""mple"

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
mkdir -p "$tmp/m"
for s in "$tty" "$acm" "$addr" "$addrp" "$login" "1.2.3" "a@b" "x/dev/ttyS0"; do
    printf '%s\n' "$s" > "$tmp/m/s.txt"
    grep -EIq -- "$acc" "$tmp/m/s.txt"; g=$?
    python3 -I "$tool" --check "$tmp/m" >/dev/null 2>&1; p=$?
    if [ $((g + p)) -eq 1 ]; then ok "grep and tool agree"; else bad "grep/tool differ (g=$g p=$p)"; fi
done

# committed i.MX8MP logs, if any
set -- "$here"/log/*-imx8mp-*
if [ -e "$1" ]; then
    if python3 -I "$tool" --check "$@" >/dev/null; then ok "committed i.MX8MP logs clean"; else bad "committed i.MX8MP logs have hits"; fi
else
    ok "no committed i.MX8MP logs yet"
fi

echo "redact selftest: $nok ok, $nfail failed"
[ $nfail -eq 0 ]
