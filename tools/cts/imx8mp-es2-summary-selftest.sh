#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-es2-summary-selftest.sh - selftest of imx8mp-es2-summary.py: a README written from made-up verdict
# lines (both slices) is accepted by imx8mp-es2-regress.py and the redact check, the c6 grep counts each entry
# once, and short hashes, missing/duplicate entries and an address are refused. Files go under $TMPDIR.
#
#   TMPDIR=<dir> sh tools/cts/imx8mp-es2-summary-selftest.sh
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
: "${TMPDIR:?set TMPDIR to a directory (files are never written to /tmp)}"
[ -d "$TMPDIR" ] || { echo "TMPDIR is not a directory" >&2; exit 2; }
case $(cd "$TMPDIR" && pwd -P) in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "TMPDIR must not be under /tmp" >&2; exit 2 ;; esac
T=$(mktemp -d "$TMPDIR/es2-summary.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
TOOL="$HERE/imx8mp-es2-summary.py"
BASE="$HERE/imx8mp-full-tree-29460f96.results.tsv"
C=05769bf20123456789abcdef0123456789abcdef
fails=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fails=$((fails + 1)); }

# made-up session output from the baseline's counts, with page lines and noise
python3 -I - "$BASE" "$T/session.txt" <<'PY'
import sys
out = []
for ln in open(sys.argv[1]):
    if ln.startswith("#"):
        continue
    f = ln.split("\t")
    i = int(f[0])
    begun, p, fl, ns, ot = (int(x) for x in f[2:7])
    out.append("board.sh: booting e%02d" % i)
    out.append("board.sh imx8mp e%02d rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled "
               "result=record:DEQP resets=absent record=594b08cc,%08x,%08x,%08x back_after=60s uptime=20s"
               % (i, (fl << 16) | p, (ot << 16) | ns, 0xffff0000 | begun))
    out.append("board.sh imx8mp e%02d-p0 rc=0 verdict=done record=594b0a00,00000000,00000000,00000000" % i)
open(sys.argv[2], "w").write("\n".join(out) + "\n")
PY

grep -E 'e(0[0-9]|1[0-4])[ -]' "$T/session.txt" > "$T/sa.txt"
grep -E 'e(1[5-9]|2[0-8])[ -]' "$T/session.txt" > "$T/sb.txt"
python3 -I "$TOOL" --commit $C --entries 0-14 --rows rows01-15 --out "$T/a.md" "$T/sa.txt" 2>&1 \
    && ok write-a || bad write-a
python3 -I "$TOOL" --commit $C --entries 15-28 --rows rows16-29 --out "$T/b.md" - < "$T/sb.txt" 2>&1 \
    && ok write-b-stdin || bad write-b-stdin
out=$(python3 -I "$HERE/imx8mp-es2-regress.py" "$T/a.md" "$T/b.md")
printf '%s\n' "$out" | grep -q 'es2 regress: 29/29 entries, 0 worse, 0 missing' && ok regress-accepts || bad regress-accepts
python3 -I "$HERE/imx8mp-gles3-redact.py" --check "$T/a.md" "$T/b.md" >/dev/null && ok redact || bad redact
n=$(cat "$T/a.md" "$T/b.md" | grep -c 'board.sh imx8mp e[0-9]* rc=')
[ "$n" = 29 ] && ok c6-grep-29 || bad "c6-grep ($n)"
grep -q "^commit $C\$" "$T/a.md" && ok commit-line || bad commit-line

refuse() {
    name=$1; shift
    if python3 -I "$TOOL" "$@" >"$T/o" 2>&1; then bad "$name (accepted)"; else ok "$name"; fi
}
refuse short-hash --commit 05769bf2 --entries 0-14 "$T/sa.txt"
refuse bad-range --commit $C --entries 0-30 "$T/session.txt"
refuse outside-range --commit $C --entries 0-5 "$T/session.txt"
grep -v 'e0[3-9]\|e[1-2][0-9]' "$T/session.txt" > "$T/m.txt"
refuse missing-entry --commit $C --entries 0-5 "$T/m.txt"
python3 -I "$TOOL" --commit $C --entries 0-5 "$T/m.txt" >/dev/null 2>&1; rc=$?
[ "$rc" = 2 ] && ok exit-2 || bad "exit-2 ($rc)"
refuse duplicate --commit $C --entries 0-28 "$T/session.txt" "$T/session.txt"
sed 's/uptime=20s/uptime=20s host=10.1.2.3/' "$T/session.txt" > "$T/ip.txt"
refuse address --commit $C --entries 0-28 "$T/ip.txt"
sed 's/record=[0-9a-f,]*//' "$T/session.txt" > "$T/nr.txt"
refuse no-record --commit $C --entries 0-28 "$T/nr.txt"

[ "$fails" = 0 ] && echo "imx8mp-es2-summary-selftest: all ok" || echo "imx8mp-es2-summary-selftest: $fails failed"
[ "$fails" = 0 ]
