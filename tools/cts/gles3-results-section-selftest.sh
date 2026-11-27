#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of gles3-results-section.py on a synthetic TSV, tree and mustpass
# list; the output is also fed to check-gles3-results.py when it exists.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
d=$(mktemp -d "${TMPDIR:-/tmp}/results-section.XXXXXX")
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
run() { python3 "$here/gles3-results-section.py" --tsv "$d/ok.tsv" --tree "$d/tree.txt" --mustpass "$d/must.txt" "$@"; }
out=$(run)
echo "$out"
chk() { echo "$out" | grep -qF -- "$1" || { echo "FAIL: $2"; exit 1; }; }
chk '**AM62P, whole tree** on `abcdef12`, 2026-11-25, 2 images:' commit-date-images
chk '| 14 | 2 | 1 | 1 | 2 |' table
chk '2 without a result, by cause: #1285-watchdog: 2.' noresult
chk 'Rows 0-0 of the tree: build `1111111a`, run 2026-11-20.' range
chk 'Rerun on `2222222b`: fbo rows' rerun
# the checker accepts it inside a RESULTS.md-shaped section
if [ -f "$here/check-gles3-results.py" ]; then
  { echo '## ES 3.0: dEQP-GLES3 (stage 4)'; echo; echo "$out"; } > "$d/RESULTS.md"
  python3 "$here/check-gles3-results.py" --results "$d/RESULTS.md" --tree "$d/tree.txt" \
    --mustpass "$d/must.txt" --tsv "$d/ok.tsv" || { echo "FAIL: check-gles3-results"; exit 1; }
fi
# a NoResult row without a cause fails; --commit overrides the header
printf 'NoResult\t%s.c8\t\t\n' "$T" >> "$d/ok.tsv"
if run >/dev/null 2>&1; then echo "FAIL: causeless NoResult accepted"; exit 1; fi
sed -i '$d' "$d/ok.tsv"
run --commit 9999999 | grep -qF 'on `9999999`' || { echo "FAIL: --commit"; exit 1; }
printf 'Bogus\t%s.c9\tm\tx\n' "$T" >> "$d/ok.tsv"
if run >/dev/null 2>&1; then echo "FAIL: unknown outcome accepted"; exit 1; fi
echo "gles3-results-section selftest: ok"
