#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx8mp-gles3-gen.py: (a) the default output is the committed imx8mp-gles3-tree.txt byte for byte,
# (b) a trial --split of one large entry gives more entries and the checker (every mustpass case matched by
# exactly one entry, length and entry limits) still passes on it. Host only; writes under $TMPDIR.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/gles3-gen-selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

python3 "$here/imx8mp-gles3-gen.py" > "$tmp/default.txt"
cmp "$tmp/default.txt" "$here/imx8mp-gles3-tree.txt" || fail "default output differs from the committed table"
echo "ok: default output is byte-identical to imx8mp-gles3-tree.txt"

split=functional.shaders.arrays=4
python3 "$here/imx8mp-gles3-gen.py" --split "$split" > "$tmp/split.txt"
n0=$(grep -vc '^#' "$tmp/default.txt") || true
n1=$(grep -vc '^#' "$tmp/split.txt") || true
[ "$n1" -gt "$n0" ] || fail "--split $split gave $n1 entries, not more than $n0"
python3 "$here/check-imx8mp-gles3-tree.py" -q --table "$tmp/split.txt" > "$tmp/check.txt" \
    || { cat "$tmp/check.txt" >&2; fail "checker rejects the split table"; }
echo "ok: --split $split: $n0 -> $n1 entries, checker passes ($(tail -1 "$tmp/check.txt"))"

if python3 "$here/imx8mp-gles3-gen.py" --split nosuch.group=2 > /dev/null 2>&1; then fail "unknown group accepted"; fi
echo "ok: unknown group rejected"
