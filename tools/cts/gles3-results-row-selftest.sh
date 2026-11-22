#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of gles3-results-row.py: a small TSV and tree, the generated rows
# spliced into a minimal RESULTS.md, accepted by check-gles3-results.py.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
gen="$here/gles3-results-row.py"
chk="$here/check-gles3-results.py"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/gles3-results-row.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "gles3-results-row-selftest: FAIL: $*" >&2; exit 1; }

for i in 1 2 3 4 5 6 7 8 9 10; do echo "dEQP-GLES3.a.b.c$i"; done >"$tmp/mustpass.txt"
printf '# tree\n# image A: x\ndEQP-GLES3.a.*\ndEQP-GLES3.b.*\n\ndEQP-GLES3.c.*\n' >"$tmp/tree.txt"
printf '# SPDX\n# dEQP-GLES3 build abcdef12, run 2026-11-20/21\n# rerun 0123abcd images 2-3\nFail\tx.1\nFail\tx.2\nNotSupported\tx.3\nQualityWarning\tx.5\nNoResult\tx.4\n' >"$tmp/np.tsv"

python3 -I "$gen" "$tmp/np.tsv" "$tmp/tree.txt" --mustpass "$tmp/mustpass.txt" >"$tmp/rows.md" \
  || fail "generator failed"
grep -q '^| 5 | 2 | 1 | 1 | 1 |$' "$tmp/rows.md" || fail "wrong counts: $(cat "$tmp/rows.md")"
grep -q '`abcdef12`, 2026-11-21, 3 images' "$tmp/rows.md" || fail "wrong commit/date/images"
grep -q 'Rerun on `0123abcd`: images 2-3' "$tmp/rows.md" || fail "rerun commit missing"

{
  printf '## Other\n\ntext\n\n## ES 3.0: dEQP-GLES3 (stage 4)\n\nMustpass list: 10 cases.\n\n'
  cat "$tmp/rows.md"
  printf '\n**AM62P, gl_test ES 3.0 cases** on `cd738b1a`: 99 images.\n\n## Next\n'
} >"$tmp/RESULTS.md"
python3 -I "$chk" --results "$tmp/RESULTS.md" --tree "$tmp/tree.txt" \
  --mustpass "$tmp/mustpass.txt" --tsv "$tmp/np.tsv" || fail "check-gles3-results rejected the rows"

# No build commit anywhere: exit 2; --commit rescues it.
printf 'Fail\tx.1\n' >"$tmp/bare.tsv"
rc=0
python3 -I "$gen" "$tmp/bare.tsv" "$tmp/tree.txt" --mustpass "$tmp/mustpass.txt" 2>/dev/null >&2 || rc=$?
[ "$rc" = 2 ] || fail "missing commit gave exit $rc"
python3 -I "$gen" "$tmp/bare.tsv" "$tmp/tree.txt" --mustpass "$tmp/mustpass.txt" --commit 1234567 \
  | grep -q '^| 9 | 1 | 0 | 0 | 0 |$' || fail "--commit run"
echo "gles3-results-row-selftest: ok"
