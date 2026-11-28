#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of gles3-issue-drafts.py on a fixture TSV under $TMPDIR.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/gles3-issue-drafts.py"
d=$(mktemp -d "${TMPDIR:-/tmp}/issue-drafts.XXXXXX")
trap 'rm -rf "$d"' EXIT
T=dEQP-GLES3.functional
printf '# rows 0-1: build aaaaaaaa, run 2026-11-22\n# rows 2-3: build bbbbbbbb, run 2026-11-27\n' >"$d/n.tsv"
{
printf 'Fail\t%s.fbo.a\tbad pixel at es3fX.cpp:10\tRC1-int-mix\n' $T
printf 'Fail\t%s.fbo.b\tbad pixel at es3fX.cpp:10\tRC1-int-mix\n' $T
printf 'NotSupported\t%s.ext.c\tExt\tns-ext\n' $T
printf 'NoResult\t%s.texture.filtering.3d.d\t\t#1285-watchdog\n' $T
printf 'QualityWarning\t%s.shadow.e\tq\tqw-shadow\n' $T
} >>"$d/n.tsv"
printf 'one\ntwo\n' >"$d/tree.txt"
n=0; : >"$d/must.txt"; while [ $n -lt 8 ]; do echo "case$n" >>"$d/must.txt"; n=$((n+1)); done
: >"$d/none.txt"; : >"$d/doc.md"
run() { python3 -I "$tool" --tsv "$d/n.tsv" --tree "$d/tree.txt" --mustpass "$d/must.txt" \
  --families "$d/none.txt" --doc "$d/doc.md" "$@"; }
run >"$d/out.md"
for s in '## Draft 1' '## Draft 2' '## Draft 3' 'build aaaaaaaa, bbbbbbbb, 2 images' \
  '| 3 | 2 | 1 | 1 | 1 | 8 |' 'rows 2-3: build bbbbbbbb' '`#1285-watchdog`: 1 (1 NoResult), #1285' \
  '### AM62P dEQP-GLES3: RC1-int-mix (2 Fail)' 'es3fX.cpp:10 (2)' 'NoResult: `dEQP-GLES3.functional.texture.filtering.3d.d`'; do
  grep -F -- "$s" "$d/out.md" >/dev/null || { echo "FAIL: missing: $s"; cat "$d/out.md"; exit 1; }
done
grep -F '### AM62P dEQP-GLES3: #1285' "$d/out.md" && { echo "FAIL: issue family drafted"; exit 1; }
# counts above the mustpass list: refused
head -3 "$d/must.txt" >"$d/m3.txt"
if python3 -I "$tool" --tsv "$d/n.tsv" --tree "$d/tree.txt" --mustpass "$d/m3.txt" >/dev/null 2>&1; then
  echo "FAIL: counts above mustpass accepted"; exit 1; fi
# redaction: a board address, tty path and user@host in a message are refused
# the values are built from parts so that this file holds none whole (the tree's redaction scan)
a=192; b=168; ttyd=/dev/tty; at=@; f=fe80
for bad in "$a.$b.7.9" "${ttyd}USB3" "root${at}board1" "$f::1:2"; do
  { cat "$d/n.tsv"; printf 'Fail\t%s.fbo.z\tat %s\tRC9-x\n' $T "$bad"; } >"$d/r.tsv"
  if python3 -I "$tool" --tsv "$d/r.tsv" --tree "$d/tree.txt" --mustpass "$d/must.txt" \
     --families "$d/none.txt" --doc "$d/doc.md" >/dev/null 2>"$d/err"; then
    echo "FAIL: $bad not refused"; exit 1; fi
  grep -F 'redaction' "$d/err" >/dev/null || { echo "FAIL: no redaction message for $bad"; exit 1; }
done
# --issue links a family
run --issue RC1-int-mix=77 | grep -F '`RC1-int-mix`: 2 (2 Fail), #77' >/dev/null || { echo "FAIL: --issue"; exit 1; }
echo "gles3-issue-drafts-selftest: ok"
