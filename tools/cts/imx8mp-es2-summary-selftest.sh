#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-es2-summary-selftest.sh - selftest of imx8mp-es2-summary.py: the real baseline (exit 0), synthetic verdict lines
# of board.sh imx8mp (Verdin) and board.sh x2 (Armadillo-X2, fake data): the board named in the header, the X2 line
# only for an X2 record, mixed boards, one entry missing (exit 1) and a bad path (exit 2). Files go under $TMPDIR.
#
#   TMPDIR=<dir> sh tools/cts/imx8mp-es2-summary-selftest.sh
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
: "${TMPDIR:?set TMPDIR to a directory (synthetic files are never written to /tmp)}"
[ -d "$TMPDIR" ] || { echo "TMPDIR is not a directory" >&2; exit 2; }
case $(cd "$TMPDIR" && pwd -P) in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "TMPDIR must not be under /tmp" >&2; exit 2 ;; esac
T=$(mktemp -d "$TMPDIR/es2-summary.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
TOOL="$HERE/imx8mp-es2-summary.py"
BASE="$HERE/imx8mp-full-tree-29460f96.results.tsv"
fails=0

# gen OUT FIRST LAST BOARD: the baseline's entries FIRST..LAST as clean verdict lines of board.sh BOARD
gen() {
    python3 -I - "$BASE" "$1" "$2" "$3" "$4" <<'PY'
import sys
base, out, a, b, bn = sys.argv[1:]
lines = []
for ln in open(base):
    if ln.startswith("#"):
        continue
    f = ln.split("\t")
    i = int(f[0])
    if not int(a) <= i <= int(b):
        continue
    begun, p, fl, ns, ot = (int(x) for x in f[2:7])
    rec = "%08x,%08x,%08x,%08x" % (0x594b08cc, (fl << 16) | p, (ot << 16) | ns, 0xffff0000 | begun)
    lines.append("board.sh %s e%02d rc=0 verdict=done boot=uboot-mmc row result=record:DEQP resets=absent "
                 "record=%s back_after=60s" % (bn, i, rec))
open(out, "w").write("\n".join(lines) + "\n")
PY
}

expect() { # NAME EXIT PATTERN ARGS...
    name=$1 want=$2 pat=$3
    shift 3
    out=$(python3 -I "$TOOL" "$@" 2>&1)
    rc=$?
    if [ "$rc" = "$want" ] && printf '%s\n' "$out" | grep -q -e "$pat"; then
        echo "ok   $name"
    else
        echo "FAIL $name (exit $rc, want $want, pattern '$pat')"
        printf '%s\n' "$out" | tail -3
        fails=$((fails + 1))
    fi
}

expect real-baseline 0 'es2 summary: 29/29 entries, passed [0-9]*, failed' "$BASE"
gen "$T/v.md" 0 28 imx8mp
expect verdin 0 '^es2 summary: board Verdin i.MX8MP (baseline' "$T/v.md"
out=$(python3 -I "$TOOL" "$T/v.md" | grep -c 'measured on the Armadillo-X2')
[ "$out" = 0 ] && echo "ok   verdin-no-x2-line" || { echo "FAIL verdin-no-x2-line"; fails=$((fails + 1)); }
gen "$T/x.md" 0 28 x2
expect x2 0 '^es2 summary: board Armadillo-X2 (baseline' "$T/x.md"
expect x2-line 0 'es2 summary: measured on the Armadillo-X2' "$T/x.md"
a=$(python3 -I "$TOOL" "$T/v.md" | sed -n 2p)
b=$(python3 -I "$TOOL" "$T/x.md" | sed -n 2p)
[ "$a" = "$b" ] && echo "ok   same-totals" || { echo "FAIL same-totals"; fails=$((fails + 1)); }
gen "$T/a.md" 0 14 imx8mp
gen "$T/b.md" 15 28 x2
expect mixed 0 'board Armadillo-X2, Verdin i.MX8MP' "$T/a.md" "$T/b.md"
expect missing 1 'es2 summary: 15/29 entries' "$T/a.md"
expect bad-path 2 'es2 summary:' "$T/nonexistent"
expect help 2 'baseline' --help

[ "$fails" = 0 ] && echo "imx8mp-es2-summary-selftest: all ok" || echo "imx8mp-es2-summary-selftest: $fails failed"
[ "$fails" = 0 ]
