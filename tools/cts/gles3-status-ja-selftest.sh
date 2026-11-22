#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of gles3-status-ja.py on a synthetic TSV, tree and mustpass list.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
d=$(mktemp -d "${TMPDIR:-/tmp}/status-ja.XXXXXX")
trap 'rm -rf "$d"' EXIT
T=dEQP-GLES3.functional.a
for i in $(seq 1 20); do echo "$T.c$i"; done > "$d/must.txt"
printf '# images\nA\nB\n\n' > "$d/tree.txt"
{
  echo '# build abcdef12, run'
  printf 'Fail\t%s.c1\tm\tRC1-x\n' "$T"
  printf 'Fail\t%s.c2\tm\tRC1-x\n' "$T"
  printf 'Fail\t%s.c3\tm\tRC2-y\n' "$T"
  printf 'NotSupported\t%s.c4\tm\tNS-z\n' "$T"
  printf 'QualityWarning\t%s.c5\tm\tQW-w\n' "$T"
  printf 'NoResult\t%s.c6\tm\t#1285-watchdog\n' "$T"
} > "$d/ok.tsv"
run() { python3 "$here/gles3-status-ja.py" --tsv "$d/ok.tsv" --tree "$d/tree.txt" --mustpass "$d/must.txt" "$@"; }
out=$(run --date 11/22)
echo "$out"
chk() { echo "$out" | grep -qF -- "$1" || { echo "FAIL: $2"; exit 1; }; }
chk 'abcdef12、11/22、2 画像' commit-images
chk '20 件のうち 14 合格 / 失敗 3 / 未対応 1 / 品質警告 1 / 結果なし 1' counts
chk '| 14 | 3 | 1 | 1 | 1 | 20 |' table
chk 'RC1-x 2 件' families
# an unassigned row must fail
printf 'Fail\t%s.c7\tm\tunassigned\n' "$T" >> "$d/ok.tsv"
if run >/dev/null 2>&1; then echo "FAIL: unassigned accepted"; exit 1; fi
echo "gles3-status-ja selftest: ok"
