#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pi4-session-tally.sh <session directory>
#
# Writes <LABEL>.tally.txt for every <LABEL>.console.txt of a Pi 4 dEQP-GLES3
# session directory (tools/cts/log/<date>-pi4-gles3-<sha>-rowsNN-MM/), with
# deqp-uart-tally.py run from inside that directory on relative names only,
# so no worker path reaches a tally (tools/release/redaction-scan.py).
# A console whose label is not an '# image <LABEL>:' line of
# pi4-gles3-tree.txt is refused before anything is written. Prints one line
# per image. Touches no file other than the tallies.
# Environment: PI4_TREE, PI4_MUSTPASS (defaults: the files next to this script).
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
TREE=${PI4_TREE:-$HERE/pi4-gles3-tree.txt}
MUSTPASS=${PI4_MUSTPASS:-$HERE/gles3-mustpass.txt}
TALLY=$HERE/deqp-uart-tally.py

[ $# -eq 1 ] && [ -d "$1" ] || { echo "usage: $0 <session directory>" >&2; exit 2; }
cd "$1" || exit 2

shopt -s nullglob
consoles=(*.console.txt)
[ ${#consoles[@]} -gt 0 ] || { echo "pi4-session-tally: no *.console.txt here" >&2; exit 1; }

# refuse first, so a bad label leaves the directory as it was
for c in "${consoles[@]}"; do
    label=${c%.console.txt}
    grep -Eq "^#[[:space:]]*image[[:space:]]+${label}[[:space:]]*:" "$TREE" || {
        echo "pi4-session-tally: label '$label' ($c) is not an '# image <LABEL>:' line of pi4-gles3-tree.txt" >&2
        exit 1
    }
done

# a relative --mustpass keeps the worker's directory out of any echo
MP=$(python3 -c 'import os,sys; print(os.path.relpath(sys.argv[1]))' "$MUSTPASS")
tmp=$(mktemp "${TMPDIR:-/tmp}/pi4-session-tally.XXXXXX") || exit 2
trap 'rm -f "$tmp"' EXIT
rc=0
for c in "${consoles[@]}"; do
    label=${c%.console.txt}
    out=$label.tally.txt
    python3 "$TALLY" --label "$label" --mustpass "$MP" "$c" > "$tmp" 2>&1
    # exit 1 is "the image has failing cases", a result; more is an error
    if [ $? -gt 1 ] || [ ! -s "$tmp" ]; then
        echo "pi4-session-tally: $label: deqp-uart-tally.py failed" >&2
        rc=1
        continue
    fi
    {
        echo "# SPDX-License-Identifier: Apache-2.0"
        echo "# SPDX-FileCopyrightText: 2026 Signal Slot, Inc."
        echo "# Pi 4 dEQP-GLES3 image $label: deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt $c"
        cat "$tmp"
    } > "$out"
    if grep -q 'complete=1' "$out" &&
        grep -Eq 'deqp: summary: ([0-9]+) cases begun, \1 with a result, 0 without' "$out"; then
        read -r p f n o <<< "$(awk '$1 == "TOTAL" { print $2, $3, $4, $5 }' "$out")"
        echo "$label: ${p:-?} pass, ${f:-?} fail, ${n:-?} notsupported, ${o:-?} other"
    else
        echo "$label: aborted"
    fi
done
exit $rc
