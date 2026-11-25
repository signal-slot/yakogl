#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Usage: t6-uboot-log-selftest.sh
# Selftest of t6-uboot-log.sh: a clean record is copied (redactor applied: a UART path and an address
# become <uart> and <addr>), and a record with each secret kind the redactor does not remove (rfc2217
# URL, tty path in a form it misses, MAC, tailnet IP, UUID, serial number) is refused with nothing
# left behind. An existing destination is never overwritten. Samples are built by concatenation.
# Host only; scratch under ${TMPDIR:-$PWD}, never /tmp.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/t6-uboot-log.sh
case "${TMPDIR:-$PWD}" in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "refusing a scratch dir under /tmp; set TMPDIR" >&2; exit 2 ;; esac
tmp=$(mktemp -d "${TMPDIR:-$PWD}/t6-uboot-log-selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
nfail=0 nok=0
ok() { echo "ok: $*"; nok=$((nok + 1)); }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }
export T6_UBOOT_LOG_ROOT=$tmp/log

url='rfc22'"17://host.example:7000"
tty_redactable='/dev/'"ttyUSB0"
tty_missed='/DEV/'"TTYUSB0"
mac='aa:bb:cc:'"dd:ee:ff"
tail_missed='host_100.'"64.1.2"
tail_plain='100.'"100.5.6"
uuid='123e4567-e89b'"-12d3-a456-426614174000"
serial='Serial Number'": 0123456789"

mkrec() { # $1 dir
    rm -rf "$1"; mkdir -p "$1/sub"
    printf 'U-Boot 2024.01\nT6 verdict: PASS\n' > "$1/console.txt"
    printf 'prompt-go rc=0\n' > "$1/sub/prompt-go.log"
    printf '\0\1\2binary\n' > "$1/blob.bin"
}
run() { bash "$tool" "$@" >"$tmp/out" 2>&1; }

# clean + redactable
mkrec "$tmp/r1"
printf 'port %s addr %s\n' "$tty_redactable" "$tail_plain" >> "$tmp/r1/console.txt"
d=$tmp/log/2026-11-25-t6-uboot-cold-0123abcd
if run "$tmp/r1" 2026-11-25 cold 0123abcd; then ok "clean record copied"; else bad "clean record refused: $(cat "$tmp/out")"; fi
if [ -f "$d/console.txt" ] && [ -f "$d/sub/prompt-go.log" ] && [ ! -e "$d/blob.bin" ]; then ok "text copied, binary skipped"; else bad "layout wrong"; fi
if grep -q '<uart>' "$d/console.txt" && grep -q '<addr>' "$d/console.txt" && grep -q 'verdict: PASS' "$d/console.txt"; then ok "redactor applied, verdict kept"; else bad "redaction: $(cat "$d/console.txt")"; fi
# no overwrite
if run "$tmp/r1" 2026-11-25 cold 0123abcd; then bad "existing destination overwritten"; else ok "existing destination refused"; fi

# one refusal per secret kind
i=0
for kind in URL TTY MAC TAILNET UUID SERIAL; do
    case $kind in
        URL) s=$url ;; TTY) s=$tty_missed ;; MAC) s=$mac ;; TAILNET) s=$tail_missed ;; UUID) s=$uuid ;; SERIAL) s=$serial ;;
    esac
    i=$((i + 1))
    mkrec "$tmp/s$i"; printf 'line %s end\n' "$s" >> "$tmp/s$i/sub/prompt-go.log"
    if run "$tmp/s$i" 2026-11-25 "bad$i" 0123abcd; then bad "$kind: not refused"; else ok "$kind: refused"; fi
    if [ ! -e "$tmp/log/2026-11-25-t6-uboot-bad$i-0123abcd" ] && ! ls -d "$tmp"/log/.t6-uboot-log.* >/dev/null 2>&1; then
        ok "$kind: nothing left behind"; else bad "$kind: leftovers"; fi
done

# usage errors
bash "$tool" >/dev/null 2>&1; [ $? -eq 2 ] && ok "no args exits 2" || bad "no args"
run "$tmp/r1" 2026-11-25 'bad/name' 0123abcd; [ $? -eq 2 ] && ok "bad name exits 2" || bad "bad name"
run "$tmp/nonexist" 2026-11-25 x 0123abcd; [ $? -eq 2 ] && ok "missing record exits 2" || bad "missing record"

echo "t6-uboot-log selftest: $nok ok, $nfail failed"
[ $nfail -eq 0 ]
