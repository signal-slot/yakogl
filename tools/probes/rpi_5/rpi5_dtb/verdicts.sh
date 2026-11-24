#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi_5/rpi5_dtb/verdicts.sh <console.log>
#
# Reads the console log of the rpi5_dtb image (src/main.c) and prints, in this
# order, one line each:
#
#   rpi5_dtb verdict: boot: <ok|missing>   the `x0 0x.., EL.., entry ..` line
#   rpi5_dtb verdict: walk: <ok|FAIL|missing>
#                                          the `totalsize N, walk rc 0: ...` line
#   rpi5_dtb verdict: nodes: <n>           `node <path>` groups printed
#   rpi5_dtb verdict: gpu: <yes|no>        a node whose compatible names v3d
#   rpi5_dtb verdict: end: <done|FAIL: ...|missing>
#   rpi5_dtb verdict: PASS | FAIL (<reason>)
#
# Exit status: 0 `done` and every check passed, 1 no end line (truncated log),
# 2 usage, 3 a FAIL line or a failed check.
set -u

if [ $# -ne 1 ] || [ ! -r "$1" ]; then
    echo "usage: verdicts.sh <console.log>" >&2
    exit 2
fi

# CR and anything before `rpi5_dtb:` (a timestamp prefix) are dropped.
lines=$(tr -d '\r' <"$1" | sed -n 's/^.*\(rpi5_dtb: .*\)$/\1/p')

boot=missing
case "$lines" in
*"rpi5_dtb: x0 0x"*"EL"*"entry 0x"*) boot=ok ;;
esac

walk=missing
wl=$(printf '%s\n' "$lines" | grep 'rpi5_dtb: totalsize .*walk rc ' | tail -n 1)
if [ -n "$wl" ]; then
    case "$wl" in
    *"walk rc 0:"*) walk=ok ;;
    *) walk=FAIL ;;
    esac
fi

nodes=$(printf '%s\n' "$lines" | grep -c '^rpi5_dtb: node ')
gpu=no
if printf '%s\n' "$lines" | grep '^rpi5_dtb:   compatible ' | grep -q 'v3d'; then
    gpu=yes
fi

end=missing
if printf '%s\n' "$lines" | grep -q '^rpi5_dtb: done$'; then
    end=done
else
    f=$(printf '%s\n' "$lines" | grep '^rpi5_dtb: FAIL' | head -n 1)
    if [ -n "$f" ]; then
        end=$(printf '%s\n' "$f" | sed 's/^rpi5_dtb: //')
    fi
fi

echo "rpi5_dtb verdict: boot: $boot"
echo "rpi5_dtb verdict: walk: $walk"
echo "rpi5_dtb verdict: nodes: $nodes"
echo "rpi5_dtb verdict: gpu: $gpu"
echo "rpi5_dtb verdict: end: $end"

if [ "$end" = missing ]; then
    echo "rpi5_dtb verdict: FAIL (no end line)"
    exit 1
fi
reason=
if [ "$end" != done ]; then
    reason=$end
elif [ "$boot" != ok ]; then
    reason="no boot record line"
elif [ "$walk" != ok ]; then
    reason="the blob did not walk"
elif [ "$nodes" -eq 0 ]; then
    reason="no node printed"
elif [ "$gpu" != yes ]; then
    reason="no v3d node"
fi
if [ -n "$reason" ]; then
    echo "rpi5_dtb verdict: FAIL ($reason)"
    exit 3
fi
echo "rpi5_dtb verdict: PASS"
exit 0
