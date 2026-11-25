#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host-only check of tools/cts/imx95-g310-cause.py: its own checks, then the
# command line (file and stdin, --summary, a refused line).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/imx95-g310-cause.py"
t=$(mktemp -d "${TMPDIR:-.}/imx95-g310-cause.XXXXXX") || exit 2
trap 'rm -rf "$t"' EXIT
python3 -I "$tool" --self-test | grep -q 'checks passed' || { echo "self-test"; exit 1; }
printf '# c\ndEQP-GLES2.a.b\tPass\tFail\ndEQP-GLES2.a.c\tPass\tPass\n' > "$t/in.tsv"
python3 -I "$tool" "$t/in.tsv" > "$t/o1"
python3 -I "$tool" < "$t/in.tsv" > "$t/o2"
cmp "$t/o1" "$t/o2" || { echo "file vs stdin"; exit 1; }
want=$(printf 'dEQP-GLES2.a.b\tPass\tFail\tg310-regress')
[ "$(sed -n 2p "$t/o1")" = "$want" ] || { echo "line"; exit 1; }
python3 -I "$tool" --summary "$t/in.tsv" | grep -q '^g310-regress.1.100.0%' || { echo "summary"; exit 1; }
printf 'x\tPass\n' > "$t/bad.tsv"
if python3 -I "$tool" "$t/bad.tsv" >/dev/null 2>&1; then echo "bad line accepted"; exit 1; fi
echo ok
