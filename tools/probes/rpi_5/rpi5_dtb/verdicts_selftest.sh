#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/probes/rpi_5/rpi5_dtb/verdicts_selftest.sh
#
# Feeds verdicts.sh canned rpi5_dtb console logs (the line format of
# src/main.c): good (exit 0), crlf with a timestamp prefix (0), FAIL line
# (3), truncated (1), a missing GPU node with `done` (3), a blob that does not
# walk (3), usage (2). Prints one line per failed check and a last line
# "verdicts_selftest: N checks, M failed"; exits 0 when none failed.
set -u

here=$(cd "$(dirname "$0")" && pwd)
V="$here/verdicts.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/rpi5_dtb_verdicts.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT INT TERM

checks=0
failed=0

head_lines() {
    echo "rpi5_dtb: build test"
    echo "rpi5_dtb: x0 0x2efec600, EL2, entry 0x200000, mpidr 0x0"
    echo "rpi5_dtb: mapped PA 0x2efec000..0x2f000000 at VA 0x1000"
    echo "rpi5_dtb: totalsize 80201, walk rc $1: 322 nodes, 900 props, depth 6, path 60"
}
gpu_node() {
    echo "rpi5_dtb: node /axi/v3d@1002000"
    echo "rpi5_dtb:   compatible brcm,2712-v3d"
    echo "rpi5_dtb:   reg 01002000 00004000"
}
hvs_node() {
    echo "rpi5_dtb: node /axi/hvs@107c580000"
    echo "rpi5_dtb:   compatible brcm,bcm2712-hvs"
}

mk_good() { head_lines 0; gpu_node; hvs_node; echo "rpi5_dtb: done"; }
mk_crlf() { head_lines 0; gpu_node; echo "rpi5_dtb: done"; }
mk_fail() { head_lines 0; echo "rpi5_dtb: FAIL: no GPU node in the device tree"; }
mk_trunc() { head_lines 0; gpu_node; }
mk_nogpu() { head_lines 0; hvs_node; echo "rpi5_dtb: done"; }
mk_nowalk() { head_lines -1; echo "rpi5_dtb: FAIL: the blob does not walk"; }
mk_nomagic() { echo "rpi5_dtb: FAIL: no FDT magic at x0 (0x00000000)"; }

mk_good >"$tmp/good"
mk_crlf | sed 's/^/[ 1.234] /' | sed 's/$/\r/' >"$tmp/crlf"
mk_fail >"$tmp/fail"
mk_trunc >"$tmp/trunc"
mk_nogpu >"$tmp/nogpu"
mk_nowalk >"$tmp/nowalk"
mk_nomagic >"$tmp/nomagic"

# run <name> <log> <want exit> <want line>...
run() {
    name=$1 log=$2 want=$3
    shift 3
    out=$(sh "$V" "$log" 2>&1)
    rc=$?
    checks=$((checks + 1))
    if [ "$rc" != "$want" ]; then
        failed=$((failed + 1))
        echo "FAIL: $name: exit $rc, want $want"
    fi
    for l in "$@"; do
        checks=$((checks + 1))
        if ! printf '%s\n' "$out" | grep -qxF "rpi5_dtb verdict: $l"; then
            failed=$((failed + 1))
            echo "FAIL: $name: line missing: $l"
        fi
    done
}

run good "$tmp/good" 0 "boot: ok" "walk: ok" "nodes: 2" "gpu: yes" "end: done" "PASS"
run crlf "$tmp/crlf" 0 "boot: ok" "nodes: 1" "gpu: yes" "PASS"
run fail "$tmp/fail" 3 "nodes: 0" "gpu: no" "end: FAIL: no GPU node in the device tree" \
    "FAIL (FAIL: no GPU node in the device tree)"
run truncated "$tmp/trunc" 1 "gpu: yes" "end: missing" "FAIL (no end line)"
run nogpu "$tmp/nogpu" 3 "nodes: 1" "gpu: no" "end: done" "FAIL (no v3d node)"
run nowalk "$tmp/nowalk" 3 "walk: FAIL" "FAIL (FAIL: the blob does not walk)"
run nomagic "$tmp/nomagic" 3 "boot: missing" "walk: missing"
run empty /dev/null 1 "boot: missing" "end: missing"

checks=$((checks + 1))
sh "$V" >/dev/null 2>&1
if [ $? -ne 2 ]; then
    failed=$((failed + 1))
    echo "FAIL: usage: no argument not exit 2"
fi
checks=$((checks + 1))
sh "$V" "$tmp/none" >/dev/null 2>&1
if [ $? -ne 2 ]; then
    failed=$((failed + 1))
    echo "FAIL: usage: unreadable file not exit 2"
fi

echo "verdicts_selftest: $checks checks, $failed failed"
[ "$failed" -eq 0 ]
