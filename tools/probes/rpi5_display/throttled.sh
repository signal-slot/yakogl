#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi5_display/throttled.sh [file...]    (stdin if no file)
#
# Decodes get_throttled values of the Pi 5 display session records. Two
# input forms, found anywhere in a line (other lines are ignored):
#
#   throttled=0x50005                  'vcgencmd get_throttled' under Linux
#   throttled=0x%08x uv=... ...        the Zephyr probe's console line
#
# Per value one line:
#
#   throttled 0x%08x: now=<list|none> seen=<list|none> verdict=<v>
#
# Bits (public Raspberry Pi documentation of 'vcgencmd get_throttled',
# https://www.raspberrypi.com/documentation/computers/os.html#vcgencmd):
#
#   now:  0 under-voltage  1 arm-freq-capped  2 throttled  3 soft-temp-limit
#   seen: 16 under-voltage 17 arm-freq-capped 18 throttled 19 soft-temp-limit
#         (occurred since boot)
#
# verdict: weak-supply if under-voltage is now or seen; throttled if any
# other bit is set; else ok.
#
# Exit: 0 every value ok; 1 any value not ok; 2 unparsable input (a
# 'throttled=' that is not followed by 0x and hex digits, an unreadable
# file, or no value at all).
set -u
export LC_ALL=C

names=(under-voltage arm-freq-capped throttled soft-temp-limit)

list() {  # list <value> <shift>
    local out="" i
    for i in 0 1 2 3; do
        if (( ($1 >> ($2 + i)) & 1 )); then
            out="$out${out:+,}${names[i]}"
        fi
    done
    printf '%s' "${out:-none}"
}

rc=0
n=0
scan() {
    local line rest tok v verdict
    while IFS= read -r line || [ -n "$line" ]; do
        case "$line" in *throttled=*) ;; *) continue ;; esac
        rest=${line#*throttled=}
        tok=${rest%%[!0-9a-fA-FxX]*}
        if [[ ! $tok =~ ^0[xX][0-9a-fA-F]{1,8}$ ]]; then
            echo "throttled: unparsable value: '${rest%% *}'" >&2
            rc=2
            continue
        fi
        v=$((tok))
        n=$((n + 1))
        if (( v & 0x10001 )); then
            verdict=weak-supply
        elif (( v & 0xe000e )); then
            verdict=throttled
        else
            verdict=ok
        fi
        printf 'throttled 0x%08x: now=%s seen=%s verdict=%s\n' \
            "$v" "$(list "$v" 0)" "$(list "$v" 16)" "$verdict"
        if [ "$verdict" != ok ] && [ "$rc" -ne 2 ]; then rc=1; fi
    done
}

if [ $# -eq 0 ]; then
    scan
else
    for f in "$@"; do
        if [ ! -r "$f" ]; then
            echo "throttled: cannot read $f" >&2
            rc=2
            continue
        fi
        scan < "$f"
    done
fi
if [ "$n" -eq 0 ] && [ "$rc" -ne 2 ]; then
    echo "throttled: no throttled= value in the input" >&2
    rc=2
fi
exit "$rc"
