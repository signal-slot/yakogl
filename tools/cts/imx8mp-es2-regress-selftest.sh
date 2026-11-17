#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-es2-regress-selftest.sh - selftest of imx8mp-es2-regress.py: the real baseline against itself (exit 0),
# synthetic READMEs built from the baseline's counts (same, split over two files, better, WORSE by a failure,
# by a missing pass, by a reset, missing entry, page lines ignored) and record words the tool must refuse (exit 2:
# w2 halves swapped, wrong kind, counts above begun or the case total, too few words). Files go under $TMPDIR (never /tmp).
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
# in entry 3, pass3 = one pass fewer in entry 3, reset3 = resets=1 in entry 3, better14 = entry 14 passes all, worse14 = entry 14
# one pass fewer and one other more; encodings
# the tool must refuse: swap14 = entry 14's w2 halves swapped, swap22 = entry 22's, kind3 = entry 3's w0 of kind 12,
# name3 = entry 3's result=record:APP, over3 = entry 3's begun one below its counts, total3 = entry 3's begun above its
# case total, short3 = entry 3's record of two words; stale3 = entry 3 not clean with a PROGRESS record, read as WORSE)
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
    resets, rc, verdict, kname, w0 = "absent", 0, "done", "DEQP", 0x594b08cc
    if i == 3 and mode == "fail3":
        p, fl = p - 3, fl + 3
    if i == 3 and mode == "pass3":
        p, begun = p - 1, begun
    if i == 3 and mode == "reset3":
        resets = "1"
    if i == 14 and mode == "better14":
        p, ns, ot = p + 14, 0, 0
    if i == 14 and mode == "worse14":
        p, ot = p - 1, ot + 1
    w1 = (fl << 16) | p
    w2 = (ot << 16) | ns
    if (i, mode) in ((14, "swap14"), (22, "swap22")):
        w2 = (ns << 16) | ot
    if i == 3 and mode == "kind3":
        w0 = 0x594b0ccc
    if i == 3 and mode == "name3":
        kname = "APP"
    if i == 3 and mode == "over3":
        begun -= 1
    if i == 3 and mode == "total3":
        begun += 1
    w3 = 0xffff0000 | begun
    rec = "%08x,%08x,%08x,%08x" % (w0, w1, w2, w3)
    if i == 3 and mode == "short3":
        rec = "%08x,%08x" % (w0, w1)
    if i == 3 and mode == "stale3":
        rc, verdict, kname, rec = 4, "no_record", "PROGRESS", "594b0500,00000301,00004e20,00000002"
    lines.append("board.sh imx8mp e%02d rc=%d verdict=%s boot=uboot-mmc row boot_verdict=cycled "
                 "result=record:%s resets=%s record=%s back_after=60s uptime=20s"
                 % (i, rc, verdict, kname, resets, rec))
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
gen "$T/x.md" 0 28 worse14
expect worse-other 1 'e14 WORSE passed 98->97 bad 14->15' "$T/x.md"

# an encoding the tool cannot read stops it (exit 2, the entry and the reason), never better or WORSE: e14 (other 14)
# read with w2 swapped would be 'better', e22 (notsup 9) 'WORSE'
gen "$T/s14.md" 0 28 swap14
expect swapped-w2-e14 2 'e14: record=.*w2 (notsup | other << 16) reads 14,0 where the baseline has 0,14: halves swapped' "$T/s14.md"
gen "$T/s22.md" 0 28 swap22
expect swapped-w2-e22 2 'e22: record=.*halves swapped, unknown record encoding' "$T/s22.md"
gen "$T/k.md" 0 28 kind3
expect bad-kind 2 'e03: record=.*w0 is kind 12, not a DEQP record' "$T/k.md"
gen "$T/n.md" 0 28 name3
expect bad-kind-name 2 'e03: record=.*result=record:APP' "$T/n.md"
gen "$T/o.md" 0 28 over3
expect over-begun 2 'e03: record=.*exceeds cases begun 993' "$T/o.md"
gen "$T/t.md" 0 28 total3
expect over-total 2 "e03: record=.*cases begun 995 exceeds the entry's 994 cases" "$T/t.md"
gen "$T/w.md" 0 28 short3
expect two-words 2 'e03: record=594b08cc,.* neither .absent. nor three or four hex words' "$T/w.md"
# a failed row's stale record (not DEQP) is no tally: the entry is WORSE, not refused
gen "$T/st.md" 0 28 stale3
expect stale-record 1 'e03 WORSE passed 994->0 bad 0->994 (row not clean)' "$T/st.md"

# bad input
expect no-args 2 'baseline' --help
expect bad-path 2 'es2 regress:' "$T/nonexistent"

[ "$fails" = 0 ] && echo "imx8mp-es2-regress-selftest: all ok" || echo "imx8mp-es2-regress-selftest: $fails failed"
[ "$fails" = 0 ]
