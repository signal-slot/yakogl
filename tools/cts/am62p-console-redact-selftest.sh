#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of console-redact.py on synthetic AM62P console text (fake values only: documentation
# addresses, fake MACs, a made-up home and UART path): U-Boot banner and env print, Zephyr boot,
# dEQP, gl_test and board.sh/tftp lines. Each testdata/am62p-console-redact/<name>.in is redacted
# through stdin and compared with <name>.want; every output is clean under the acceptance grep and
# carries no MAC, the second pass changes nothing, and the in-place mode agrees with the stream mode.
# With --update the .want files are rewritten from the tool (review the diff before committing).
# Host only; scratch under ${TMPDIR:-$PWD}, never /tmp.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/console-redact.py
data=$here/testdata/am62p-console-redact
case "${TMPDIR:-$PWD}" in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "refusing a scratch dir under /tmp; set TMPDIR" >&2; exit 2 ;; esac
tmp=$(mktemp -d "${TMPDIR:-$PWD}/am62p-console-redact.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
nfail=0 nok=0
ok() { echo "ok: $*"; nok=$((nok + 1)); }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }

acc='\b([0-9]{1,3}\.){3}[0-9]{1,3}\b|/dev/''tty|/home''/'
mac='([0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}'

names=(uboot zephyr deqp gltest board)

if [ "${1:-}" = --update ]; then
    for n in "${names[@]}"; do python3 -I "$tool" - < "$data/$n.in" > "$data/$n.want"; done
    echo "updated ${#names[@]} .want files"
    exit 0
fi

for n in "${names[@]}"; do
    in=$data/$n.in want=$data/$n.want
    if [ ! -f "$in" ] || [ ! -f "$want" ]; then bad "$n: fixture missing"; continue; fi
    # the inputs must carry something to redact (else the fixture proves nothing)
    if grep -Eq -- "$acc|$mac" "$in"; then ok "$n: input carries redactable text"; else bad "$n: input has nothing to redact"; fi
    python3 -I "$tool" - < "$in" > "$tmp/$n.out"; rc=$?
    if [ $rc -eq 0 ] && cmp -s "$tmp/$n.out" "$want"; then ok "$n: redacted output equals expected"
    else bad "$n: output differs (rc=$rc)"; diff "$want" "$tmp/$n.out" >&2; fi
    if grep -Eq -- "$acc|$mac" "$tmp/$n.out"; then bad "$n: output still has an address, UART, home or MAC"; else ok "$n: output clean"; fi
    python3 -I "$tool" - < "$tmp/$n.out" | cmp -s - "$tmp/$n.out" && ok "$n: idempotent" || bad "$n: second pass changed the output"
    if [ "$(wc -l < "$in")" = "$(wc -l < "$tmp/$n.out")" ]; then ok "$n: line count kept"; else bad "$n: line count changed"; fi
    # in-place mode gives the same bytes
    mkdir -p "$tmp/ip"; cp "$in" "$tmp/ip/$n.txt"
    python3 -I "$tool" "$tmp/ip/$n.txt" >/dev/null 2>&1
    if cmp -s "$tmp/ip/$n.txt" "$want"; then ok "$n: in-place equals stream"; else bad "$n: in-place differs"; fi
done

# the spots that matter, by name: each is one rule on one AM62P line shape
chk() { # $1 want-file $2 fixed string that must be present
    if grep -qF -- "$2" "$data/$1.want"; then ok "$1: has '$2'"; else bad "$1: lacks '$2'"; fi
}
chk uboot 'ethaddr=<mac>'
chk uboot 'ipaddr=<addr>'
chk uboot 'serverip=<addr>'
chk uboot 'bootfile=<home>'
chk uboot 'TFTP from server <addr>; our IP address is <addr>'
chk uboot 'Hit any key to stop autoboot:  0'
chk zephyr 'MAC <mac> link up'
chk zephyr 'BVNC <addr> fw ready'
chk zephyr 'rti_arm: fed for 150 s'
chk deqp 'vulkan-cts-<1.4.6-2>-555-g'
chk deqp 'Writing <home>'
chk deqp 'dEQP-GLES3.functional.texture.filtering.3d.formats.rgba8_linear_mipmap_linear'
chk gltest 'gl_test: PASS case 196'
chk board 'capture <uart> '
chk board 'probe <uart>'
chk board 'stage=<home>'
chk board 'peer=<addr>:59643'
chk board 'mac <mac> and 12:34:56 time and 1.4.6 stay'

# this file carries nothing the acceptance grep matches
if grep -nE -- "$acc" "$0" >/dev/null; then bad "this selftest trips the acceptance grep"; else ok "selftest itself clean"; fi

echo "am62p-console-redact selftest: $nok ok, $nfail failed"
[ $nfail -eq 0 ]
