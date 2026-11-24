#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi_5/rpi5_ident/verdicts.sh <console.log>
#
# Reads the console log of the rpi5_ident image (src/main.c) and prints one
# line per check, then the verdict:
#
#   rpi5_ident verdict: <NAME>: ok|FAIL (<read> / expected <board value>)
#       NAME is each of HUB_IDENT0..3, CORE_IDENT0..2, MMU_DEBUG_INFO (the
#       words of the README's D2 run, equal to Linux's v3d_ident), then
#       `decode` (the line `V3D 7.1, 1 core(s), 12 QPUs, VPM 65536 bytes,
#       PA 36 / VA 36 bits, MMU yes`) and `end` (the image's `done` or `FAIL`
#       line)
#   rpi5_ident verdict: PASS | FAIL (<reasons>) | no end line
#
# Lines other than the above (build, fallback, boot loader) are ignored. A
# missing value line is a FAIL of its check (a read that faulted ends the
# log there). `rpi5_ident: FAIL` anywhere is a FAIL.
#
# Exit status: 0 done and every check ok; 1 no end line (truncated log);
# 3 FAIL (a wrong or missing word, or the image's FAIL line); 2 usage.
set -u

if [ $# -ne 1 ] || [ ! -r "$1" ]; then
    echo "usage: verdicts.sh <console.log>" >&2
    exit 2
fi
log=$1
bad=0
reasons=""

# value_of <NAME>: the last `rpi5_ident: NAME 0x...` word, or empty
value_of() {
    tr -d '\r' <"$log" | sed -n "s/.*rpi5_ident: $1 \(0x[0-9a-fA-F]*\).*/\1/p" | tail -n 1 | tr 'A-F' 'a-f'
}

check_word() {
    name=$1
    want=$2
    got=$(value_of "$name")
    if [ "$got" = "$want" ]; then
        echo "rpi5_ident verdict: $name: ok ($got)"
    else
        echo "rpi5_ident verdict: $name: FAIL (${got:-missing} / expected $want)"
        bad=1
        reasons="$reasons $name"
    fi
}

check_word HUB_IDENT0 0x42554856
check_word HUB_IDENT1 0x00081117
check_word HUB_IDENT2 0x00001900
check_word HUB_IDENT3 0x00020a10
check_word CORE_IDENT0 0x07443356
check_word CORE_IDENT1 0x81001431
check_word CORE_IDENT2 0xc0078101
check_word MMU_DEBUG_INFO 0x20804664

want_dec="V3D 7.1, 1 core(s), 12 QPUs, VPM 65536 bytes, PA 36 / VA 36 bits, MMU yes"
dec=$(tr -d '\r' <"$log" | sed -n 's/.*rpi5_ident: \(V3D [0-9].*MMU [a-z]*\).*/\1/p' | tail -n 1)
if [ "$dec" = "$want_dec" ]; then
    echo "rpi5_ident verdict: decode: ok ($dec)"
else
    echo "rpi5_ident verdict: decode: FAIL (${dec:-missing} / expected $want_dec)"
    bad=1
    reasons="$reasons decode"
fi

done=$(tr -d '\r' <"$log" | grep -c 'rpi5_ident: done')
failed=$(tr -d '\r' <"$log" | grep -c 'rpi5_ident: FAIL')
if [ "$failed" -gt 0 ]; then
    echo "rpi5_ident verdict: end: FAIL (the image printed FAIL)"
    bad=1
    reasons="$reasons image-FAIL"
elif [ "$done" -gt 0 ]; then
    echo "rpi5_ident verdict: end: ok (rpi5_ident: done)"
else
    echo "rpi5_ident verdict: end: missing"
fi

if [ "$failed" -gt 0 ] || { [ "$bad" -ne 0 ] && [ "$done" -gt 0 ]; }; then
    echo "rpi5_ident verdict: FAIL ($(echo $reasons))"
    exit 3
fi
if [ "$done" -eq 0 ]; then
    echo "rpi5_ident verdict: no end line"
    exit 1
fi
echo "rpi5_ident verdict: PASS"
exit 0
