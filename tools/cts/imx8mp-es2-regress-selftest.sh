#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-es2-regress-selftest.sh - selftest of imx8mp-es2-regress.py: the real baseline against itself (exit 0),
# synthetic READMEs built from the baseline's counts (same, split over two files, better, WORSE by a failure,
# by a missing pass, by a reset, missing entry, page lines ignored). Files go under $TMPDIR (never /tmp).
#
#   TMPDIR=<dir> sh tools/cts/imx8mp-es2-regress-selftest.sh
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
: "${TMPDIR:?set TMPDIR to a directory (synthetic files are never written to /tmp)}"
[ -d "$TMPDIR" ] || { echo "TMPDIR is not a directory" >&2; exit 2; }
case $(cd "$TMPDIR" && pwd -P) in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "TMPDIR must not be under /tmp" >&2; exit 2 ;; esac
T=$(mktemp -d "$TMPDIR/es2-regress.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
TOOL="$HERE/imx8mp-es2-regress.py"
BASE="$HERE/imx8mp-full-tree-29460f96.results.tsv"
fails=0

# gen OUT FIRST LAST [MODE]: verdict lines of the baseline's entries FIRST..LAST (MODE: ok, fail3 = three more failures
# in entry 3, pass3 = one pass fewer in entry 3, reset3 = resets=1 in entry 3, better14 = entry 14 passes all)
gen() {
    python3 -I - "$BASE" "$1" "$2" "$3" "${4:-ok}" <<'PY'
import sys
base, out, a, b, mode = sys.argv[1:]
lines = []
for ln in open(base):
    if ln.startswith("#"):
        continue
    f = ln.split("\t")
    i = int(f[0])
    if not int(a) <= i <= int(b):
        continue
    begun, p, fl, ns, ot = (int(x) for x in f[2:7])
    resets = "absent"
    if i == 3 and mode == "fail3":
        p, fl = p - 3, fl + 3
    if i == 3 and mode == "pass3":
        p, begun = p - 1, begun
    if i == 3 and mode == "reset3":
        resets = "1"
    if i == 14 and mode == "better14":
        p, ns, ot = p + 14, 0, 0
    w1 = (fl << 16) | p
    w2 = (ot << 16) | ns
    w3 = 0xffff0000 | begun
    lines.append("board.sh imx8mp e%02d rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled "
                 "result=record:DEQP resets=%s record=594b08cc,%08x,%08x,%08x back_after=60s uptime=20s"
                 % (i, resets, w1, w2, w3))
    lines.append("board.sh imx8mp e%02d-p0 rc=0 verdict=done record=594b0a00,00000000,00000000,00000000" % i)
open(out, "w").write("# synthetic\n" + "\n".join(lines) + "\n")
PY
}

# expect NAME EXIT PATTERN ARGS...: run the tool, check the exit status and that the output matches PATTERN
expect() {
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

# the real baseline against itself, as a tsv and as a directory holding it as results.tsv
expect real-baseline 0 'es2 regress: 29/29 entries, 0 worse, 0 missing' "$BASE"
mkdir "$T/dir"
cp "$BASE" "$T/dir/results.tsv"
expect real-baseline-dir 0 'es2 regress: 29/29 entries, 0 worse, 0 missing' "$T/dir"

# the baseline's counts as verdict lines
gen "$T/all.md" 0 28
expect same 0 'es2 regress: 29/29 entries, 0 worse, 0 missing' "$T/all.md"
out=$(python3 -I "$TOOL" "$T/all.md" | grep -c ' same ')
[ "$out" = 29 ] && echo "ok   same-lines" || { echo "FAIL same-lines ($out)"; fails=$((fails + 1)); }

# two slices as two files (es2a, es2b) and as two directories
gen "$T/a.md" 0 14
gen "$T/b.md" 15 28
expect two-files 0 'es2 regress: 29/29 entries, 0 worse, 0 missing' "$T/a.md" "$T/b.md"
mkdir "$T/da" "$T/db"
cp "$T/a.md" "$T/da/README.md"
cp "$T/b.md" "$T/db/README.md"
expect two-dirs 0 'es2 regress: 29/29 entries, 0 worse, 0 missing' "$T/da" "$T/db"

# one slice only: 14 entries missing
expect missing 1 'es2 regress: 15/29 entries, 0 worse, 14 missing' "$T/a.md"

# worse and better
gen "$T/f.md" 0 28 fail3
expect worse-fail 1 'e03 WORSE' "$T/f.md"
gen "$T/p.md" 0 28 pass3
expect worse-pass 1 'e03 WORSE' "$T/p.md"
gen "$T/r.md" 0 28 reset3
expect worse-reset 1 'e03 WORSE.*not clean' "$T/r.md"
gen "$T/g.md" 0 28 better14
expect better 0 'e14 better' "$T/g.md"

# bad input
expect no-args 2 'baseline' --help
expect bad-path 2 'es2 regress:' "$T/nonexistent"

[ "$fails" = 0 ] && echo "imx8mp-es2-regress-selftest: all ok" || echo "imx8mp-es2-regress-selftest: $fails failed"
[ "$fails" = 0 ]
