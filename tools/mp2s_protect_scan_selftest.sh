#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_protect_scan_selftest.sh - gate-visible selftest of mp2s_protect_scan.py.
#
#   tools/mp2s_protect_scan_selftest.sh
#
# 1. the scan's built-in --self-test; 2. the scan on the real tree (no
# argument) must be clean, so a later change that writes a protected item
# turns the gate red; 3. black-box fixtures in $TMPDIR: exit 3 on a write of
# bootcmd / bootcount / the eMMC / fip-a, exit 0 on text, the allow marker
# (the scan's real one is `protect-scan: allow`), exit 2 on usage errors.
# No board, no network. Prints "N checks, M failed"; exit 1 if any failed.
set -u
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SCAN="$HERE/mp2s_protect_scan.py"
T=$(mktemp -d "${TMPDIR:-/tmp}/mp2s-protect-st.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
N=0; F=0

# expect NAME WANT_RC [args...]: runs the scan, compares the exit code.
expect() {
    local name=$1 want=$2 rc; shift 2
    python3 -I "$SCAN" "$@" >"$T/out" 2>"$T/err"; rc=$?
    N=$((N + 1))
    if [ "$rc" = "$want" ]; then
        echo "ok   $name"
    else
        F=$((F + 1)); echo "FAIL $name: exit $rc, want $want"
        sed 's/^/     /' "$T/err" | head -5
    fi
}
fx() { printf '%s\n' "$2" >"$T/$1"; }   # fx FILE LINE

expect "built-in --self-test" 0 --self-test
expect "real tree is clean" 0

fx hit_setenv.cmd   'setenv bootcmd run foo'
fx hit_fwsetenv.sh  'fw_setenv bootcount 0'
fx hit_altboot.sh   'fw_setenv altbootcmd x'
fx hit_emmc.sh      'dd if=x.bin of=/dev/mmcblk0 bs=1M'
fx hit_emmc_p.sh    'cp x /dev/mmcblk0p3'
fx hit_mmcdev.cmd   'mmc dev 0'
fx hit_fipa.sh      'dd if=x.bin of=/dev/disk/by-partlabel/fip-a'
fx hit_fsbla.sh     'sgdisk --delete=1 fsbla1'
fx hit_nv.sh        'printf "bootcmd=x\n" | fw_setenv -s -'
for f in hit_setenv.cmd hit_fwsetenv.sh hit_altboot.sh hit_emmc.sh hit_emmc_p.sh \
         hit_mmcdev.cmd hit_fipa.sh hit_fsbla.sh hit_nv.sh; do
    expect "hit: $f" 3 "$T/$f"
done

# text and reads are not writes
fx ok_comment.sh   '# setenv bootcmd x'
fx ok_echo.sh      'echo "never setenv bootcmd"'
fx ok_read.sh      'fw_printenv bootcount'
fx ok_otherpart.sh 'dd if=x of=/dev/disk/by-partlabel/bootfs'
fx ok_otherenv.cmd 'setenv mp2s_boots_left 1'
for f in ok_comment.sh ok_echo.sh ok_read.sh ok_otherpart.sh ok_otherenv.cmd; do
    expect "clean: $f" 0 "$T/$f"
done

# the allow marker: exact, and near misses that must not suppress
fx mk_exact.sh  'fw_setenv bootcount 0 # protect-scan: allow'
fx mk_pre.sh    'fw_setenv bootcount 0 # mp2s-protect: read-only'
fx mk_case.sh   'fw_setenv bootcount 0 # Protect-Scan: Allow'
fx mk_nospace.sh 'fw_setenv bootcount 0 # protect-scan:allow'
fx mk_word.sh   'fw_setenv bootcount 0 # protect scan: allow'
expect "marker exact suppresses" 0 "$T/mk_exact.sh"
for f in mk_pre.sh mk_case.sh mk_nospace.sh mk_word.sh; do
    expect "marker near-miss still hit: $f" 3 "$T/$f"
done

# usage
expect "usage: unknown option" 2 --bogus
expect "usage: missing file" 2 "$T/does-not-exist"

echo "$N checks, $F failed"
[ "$F" = 0 ]
