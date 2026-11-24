#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/probes/rpi_5/rpi5_triangle/verdicts_selftest.sh
#
# Feeds verdicts.sh inline rpi5_triangle console logs (the lines of
# src/main.c, as quoted in README.md) and checks the last line and exit:
#   good, ours  (exit 0, PASS, either shader name)
#   failimg     the image's FAIL line (exit 3)
#   steprc      M5 render/bin/caches_clean rc non-zero, end says PASS (exit 3)
#   noident     identity line missing, end says PASS (exit 3)
#   readback    wrong pixels or guard bytes, end says PASS (exit 3)
#   mismatch    build says mesa, end says ours (exit 3)
#   truncated   no end line (exit 1)  crlf  CR ends and a prefix (exit 0)
#   usage       no argument, unreadable file (exit 2)
# Never exits 77. Prints "verdicts_selftest: N checks, M failed".
set -u

here=$(cd "$(dirname "$0")" && pwd)
V="$here/verdicts.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/rpi5_triangle_vt.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT INT TERM
checks=0
failed=0

# body <shaders>: the console of a good run (README, 2026-11-20)
body() {
    cat <<X
v3d_triangle: build 0c58c457, shaders $1 (FS 9, VS 23, CS 15 instructions)
v3d_triangle: cache: CONFIG_CACHE_MANAGEMENT=1 CONFIG_DCACHE=1 sys_cache_data_flush_and_invd_range, data line 64 bytes
v3d_triangle: rpi_5, V3D 7.1, target 64x64 raster RGBA8888, watchdog armed at boot
M1 mailbox: 0
M1 before power-on: PM_GRAFX 0x00001040
M1 power_on: 0, PM_GRAFX 0x00001040, SMS REE_CS 0x00000000 TEE_CS 0x00000050
M1 V3D clock: rate 0 (500000000 Hz)
[00:00:00.051,000] <inf> gpu_v3d: V3D 7.1: 1 core(s), 12 QPUs, VPM 65536 bytes, PA 36 / VA 36 bits, MMU yes
M1 identity: HUB_IDENT0..3 0x42554856 0x00081117 0x00001900 0x00020a10
M1 identity: V3D 7.1, 1 core(s), 12 QPUs, VPM 65536 bytes, PA 36 / VA 36 bits, MMU yes: 0
M2 page table: PA 0x280000, 4096 entries, scratch PA 0x27f000, VA 0x00100000..0x0016a000 mapped, 108 pages used
M2 mmu_init: 0
M5 before bin: MISCCFG 0x00000006 UIFCFG 0x00000045 ERR_STAT 0x00001000 FDBGS 0x00000007 core INT_STS 0x00000000 hub INT_STS 0x00000000
M5 bin: 0, CT0CA 0x0010007e CT0EA 0x0010007e (end 0x0010007e), BPCA 0x00108000 BPCS 0x00001000, overflow blocks used 0
M5 render: 0, CT1CA 0x00101060 (end 0x00101060), ERR_STAT 0x00001000
M5 caches_clean: 0
M5 vertices (window): (8,8) (56,8) (32,56), clear 00 00 ff ff, tol 3, margin 1 px
M5 pixel centre (32,24): 51 56 58 ff
M5 pixel near v0 (12,10): e0 11 0d ff
M5 pixel near v1 (51,10): 11 e0 0d ff
M5 pixel near v2 (32,50): 0c 11 e2 ff
M5 pixel corner (1,62): 00 00 ff ff
M5 read-back: 0, 1012 inside (0 wrong, max diff 1), 2796 outside (0 wrong), 288 edge (0 wrong), guard rows 0 of 512 bytes changed
v3d_triangle: PASS (64x64 read back, 0 of 4096 px wrong: 1012 inside, 2796 outside, 288 edge, tol 3; shaders $1)
X
}

# run <name> <exit> <last-line-substring> <logfile>
run() {
    name=$1
    want=$2
    pat=$3
    log=$4
    checks=$((checks + 2))
    out=$(sh "$V" "$log" 2>&1)
    rc=$?
    last=$(printf '%s\n' "$out" | tail -n 1)
    if [ "$rc" -ne "$want" ]; then
        failed=$((failed + 1))
        echo "FAIL: $name: exit $rc, wanted $want"
    fi
    case "$last" in
    *"$pat"*) ;;
    *)
        failed=$((failed + 1))
        echo "FAIL: $name: last line '$last' lacks '$pat'"
        ;;
    esac
}

body mesa >"$tmp/good"
run good 0 "verdicts: PASS (M1 to M5 clean, shaders mesa" "$tmp/good"

body ours >"$tmp/ours"
run ours 0 "shaders ours" "$tmp/ours"

body ours | sed 's/^v3d_triangle: PASS (.*; shaders ours)/v3d_triangle: FAIL (M5 render: timeout; shaders ours)/' >"$tmp/failimg"
run failimg 3 "image says FAIL" "$tmp/failimg"

body mesa | sed 's/^M5 render: 0,/M5 render: -110,/' >"$tmp/steprc"
run steprc 3 "m5 FAIL" "$tmp/steprc"

body mesa | sed 's/^M5 bin: 0,/M5 bin: -5,/' >"$tmp/binrc"
run binrc 3 "m5 FAIL" "$tmp/binrc"

body mesa | sed 's/^M5 caches_clean: 0$/M5 caches_clean: -22/' >"$tmp/clean"
run clean 3 "m5 FAIL" "$tmp/clean"

body ours | grep -v '^M1 identity: V3D ' >"$tmp/noident"
run noident 3 "m1 missing" "$tmp/noident"

body ours | sed 's/V3D 7.1, 1 core/V3D 4.2, 1 core/' >"$tmp/wrongid"
run wrongid 3 "m1 FAIL" "$tmp/wrongid"

body mesa | sed 's/1012 inside (0 wrong/1012 inside (7 wrong/' >"$tmp/readback"
run readback 3 "pixels FAIL" "$tmp/readback"

body mesa | sed 's/guard rows 0 of/guard rows 8 of/' >"$tmp/guard"
run guard 3 "pixels FAIL" "$tmp/guard"

body mesa | grep -v '^M5 pixel ' >"$tmp/nopix"
run nopix 3 "pixels missing" "$tmp/nopix"

body mesa | sed 's/^\(v3d_triangle: PASS .*\); shaders mesa)/\1; shaders ours)/' >"$tmp/mismatch"
run mismatch 3 "build shaders mesa, end line ours" "$tmp/mismatch"

body mesa | sed 's/shaders mesa/shaders other/' >"$tmp/badname"
run badname 3 "verdicts: FAIL" "$tmp/badname"

body mesa | grep -v '^v3d_triangle: PASS' >"$tmp/truncated"
run truncated 1 "no end line" "$tmp/truncated"

body ours | sed 's/^/[00:00:01.000,000] /; s/$/\r/' >"$tmp/crlf"
run crlf 0 "verdicts: PASS" "$tmp/crlf"

checks=$((checks + 1))
sh "$V" >/dev/null 2>&1
[ $? -eq 2 ] || { failed=$((failed + 1)); echo "FAIL: usage: no argument"; }
checks=$((checks + 1))
sh "$V" "$tmp/none" >/dev/null 2>&1
[ $? -eq 2 ] || { failed=$((failed + 1)); echo "FAIL: usage: unreadable file"; }

echo "verdicts_selftest: $checks checks, $failed failed"
[ "$failed" -eq 0 ]
