#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of gles3-status-section.py on a synthetic TSV, tree and mustpass list.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
d=$(mktemp -d "${TMPDIR:-/tmp}/status-section.XXXXXX")
trap 'rm -rf "$d"' EXIT
T=dEQP-GLES3.functional.a
for i in $(seq 1 20); do echo "$T.c$i"; done > "$d/must.txt"
printf '# images\nA\nB\n\n' > "$d/tree.txt"
{
  echo '# rows 0-0: build 1111111a, run 2026-11-20'
  echo '# rows 1-1: build abcdef12, run 2026-11-22/25'
  echo '# rerun 2222222b: fbo rows'
  printf 'Fail\t%s.c1\tm\tRC1-x\n' "$T"
  printf 'Fail\t%s.c2\tm\tRC1-x\n' "$T"
  printf 'NotSupported\t%s.c4\tm\tNS-z\n' "$T"
  printf 'QualityWarning\t%s.c5\tm\tQW-w\n' "$T"
  printf 'NoResult\t%s.c6\tm\t#1285-watchdog\n' "$T"
  printf 'NoResult\t%s.c7\tm\t#1285-watchdog\n' "$T"
} > "$d/ok.tsv"
run() { python3 -I "$here/gles3-status-section.py" --tsv "$d/ok.tsv" --tree "$d/tree.txt" \
  --mustpass "$d/must.txt" --expect-total 20 "$@"; }
out=$(run)
echo "$out"
chk() { echo "$out" | grep -qF -- "$1" || { echo "FAIL: $2"; exit 1; }; }
chk '全ツリー（abcdef12、2 画像）は必須 20 件のうち' commit-images
chk '合格 14 / 失敗 2 / 未対応 1 / 品質警告 1 / 結果なし 2（計 20）' counts
chk '行 0〜0 は 1111111a（11/20）' range
chk '再実行: 2222222b（fbo rows）' rerun
chk '大きい順に #1285-watchdog 2 件、RC1-x 2 件、NS-z 1 件' families
# the sum check: a wrong expected total exits 1
rc=0; run --expect-total 21 >/dev/null 2>&1 || rc=$?
[ "$rc" = 1 ] || { echo "FAIL: sum mismatch exit $rc"; exit 1; }
# a malformed header and a missing build commit exit 2
sed 's/^# rows 1-1.*/# rows 1-1: build, run soon/' "$d/ok.tsv" > "$d/bad.tsv"
rc=0; python3 -I "$here/gles3-status-section.py" --tsv "$d/bad.tsv" --tree "$d/tree.txt" \
  --mustpass "$d/must.txt" --expect-total 20 >/dev/null 2>&1 || rc=$?
[ "$rc" = 2 ] || { echo "FAIL: malformed header exit $rc"; exit 1; }
grep -v '^# rows' "$d/ok.tsv" > "$d/nocommit.tsv"
rc=0; python3 -I "$here/gles3-status-section.py" --tsv "$d/nocommit.tsv" --tree "$d/tree.txt" \
  --mustpass "$d/must.txt" --expect-total 20 >/dev/null 2>&1 || rc=$?
[ "$rc" = 2 ] || { echo "FAIL: missing commit exit $rc"; exit 1; }
echo "gles3-status-section selftest: ok"
