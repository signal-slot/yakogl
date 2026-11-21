#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pr-c7-check.sh [ROOT]
#
# Mechanical check of acceptance c7 of the partial-render plan (issue #2):
#   1. the Kconfig entry YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER (driver/zephyr/Kconfig,
#      up to the next 'config' line) has no 'default y';
#   2. the status paragraph of the "Design: the firmware's partial render"
#      section of docs/notes/pvr-render-pool.md (from its "**Status" line to
#      the next line starting with "**" or a "# " heading) cites at least one
#      tools/cts/log/<dir>/ path;
#   3. every cited directory exists under ROOT.
# ROOT defaults to the repository root. Exit 0 on success, 1 on a failed part,
# 2 on a missing input file.
set -u

ROOT=${1:-$(cd "$(dirname "$0")/../.." && pwd)}
KC=$ROOT/driver/zephyr/Kconfig
NOTE=$ROOT/docs/notes/pvr-render-pool.md

for f in "$KC" "$NOTE"; do
    [ -f "$f" ] || { echo "pr-c7-check: missing $f" >&2; exit 2; }
done

entry=$(awk '
    /^config[ \t]+YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER[ \t]*$/ { on = 1; print; next }
    on && /^config[ \t]/ { exit }
    on { print }
' "$KC")
if [ -z "$entry" ]; then
    echo "pr-c7-check: FAIL: no config YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER in $KC" >&2
    exit 1
fi
if printf '%s\n' "$entry" | grep -Eq '^[[:space:]]*default[[:space:]]+y'; then
    echo "pr-c7-check: FAIL: the knob has 'default y' (must ship opt-in)" >&2
    exit 1
fi

para=$(awk '
    /^##+ Design: the firmware.s partial render/ { sec = 1; next }
    sec && /^## / { exit }
    sec && !on && /^\*\*Status/ { on = 1; print; next }
    on && (/^\*\*/ || /^#+ /) { exit }
    on { print }
' "$NOTE")
if [ -z "$para" ]; then
    echo "pr-c7-check: FAIL: no status paragraph in the design section of $NOTE" >&2
    exit 1
fi

dirs=$(printf '%s\n' "$para" | grep -Eo 'tools/cts/log/[A-Za-z0-9._-]+/' | sort -u)
if [ -z "$dirs" ]; then
    echo "pr-c7-check: FAIL: the status paragraph cites no tools/cts/log/<dir>/ record" >&2
    exit 1
fi

n=0
missing=0
while IFS= read -r d; do
    n=$((n + 1))
    if [ ! -d "$ROOT/$d" ]; then
        echo "pr-c7-check: FAIL: cited record $d does not exist" >&2
        missing=1
    fi
done <<EOF2
$dirs
EOF2
[ "$missing" = 0 ] || exit 1

echo "pr-c7-check: $n records cited, all present; knob default n"
