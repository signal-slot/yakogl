#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of imx8mp-gles3-x2class.py on synthetic slice fixtures built here (no board values):
# every class (clean, watchdog, early-stop, gpu-reset, unproven), the sequence byte, the expected counts.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-.}/x2class.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
tool="$here/imx8mp-gles3-x2class.py"
fail=0
ok() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }

# synthetic table: 10 entries of 100 cases each (a.N.*), a 10-case entry for kind 9 (b.*)
python3 -I - "$tmp" <<'PY'
import sys
d = sys.argv[1]
ents = ['t.a%d.*' % i for i in range(9)] + ['t.b.*']
open(d + '/table.txt', 'w').write('# synthetic\n' + '\n'.join(ents) + '\n')
cases = ['t.a%d.c%d' % (i, k) for i in range(9) for k in range(100)] + ['t.b.c%d' % k for k in range(10)]
open(d + '/must.txt', 'w').write('\n'.join(cases) + '\n')

def w(kind, low, a, b, c=0):
    return '%08x,%08x,%08x,%08x' % (low | kind << 8 | 0x1234 << 16, a, b, c)
def tally(ended_pass, fail, extra_begun=0, begun=None):
    ended = ended_pass + fail
    seq = (2 * ended + extra_begun - 64) % 256
    return w(8, seq, ended_pass | fail << 16, 0)
def line(n, verdict, rec, back, resets='absent'):
    return ('board.sh x2 e%02d rc=0 verdict=%s boot=kexec-dev DEV boot_verdict=cycled result=record:DEQP resets=%s '
            'record=%s back_after=%ss\n' % (n, verdict, resets, rec, back))
L = [
    line(0, 'done', tally(100, 0), 95),                         # clean
    line(1, 'done', tally(60, 3), 690),                         # watchdog: 37 short, back 690
    line(2, 'done', tally(50, 0), 300),                         # early-stop: 50 short, back 300
    line(3, 'done_with_resets', tally(100, 0), 120, '2'),       # gpu-reset by resets=
    line(4, 'done', tally(70, 0), 650),                         # unproven: back between 630 and 680
    line(5, 'done', tally(100, 0, 1), 100),                     # unproven: complete but seq odd (a case begun, no end)
    line(6, 'done', 'absent', 100),                             # unproven: no record
    line(7, 'done', tally(40, 0, 1), 700),                      # watchdog, begun-no-end noted
    line(8, 'done', tally(30, 0), 660),                         # gpu-reset by the console scan below (recoveries)
    line(9, 'done', w(9, 10, 0x55555, 0), 80),                  # kind 9: 10 cases, all Pass (2 bits = 1) clean
]
open(d + '/s1.md', 'w').write('# slice\n\n```\n' + ''.join(L) + '```\n')
import os
os.makedirs(d + '/s1'); os.rename(d + '/s1.md', d + '/s1/README.md')
os.makedirs(d + '/s2')
open(d + '/s2/README.md', 'w').write('# slice\n```\n' + line(1, 'done', tally(60, 3), 100) + line(1, 'done', tally(100, 0), 99) + '```\n')
os.makedirs(d + '/s3')
open(d + '/s3/README.md', 'w').write('# slice\n```\n' + line(2, 'done', tally(10, 0), 90) + line(2, 'done', tally(10, 0), 640)
                                    .replace('rc=0', 'rc=0') + '```\n')
os.makedirs(d + '/bad')
open(d + '/bad/README.md', 'w').write('no verdict lines here\n')
open(d + '/scan.txt', 'w').write(
    'row   2  10:00:00   600s  results   50  recoveries  0  !! NO FINISH (watchdog end)\n'
    'row   8  10:20:00   610s  results   30  recoveries  3  !! NO FINISH (hang)\n')
os.makedirs(d + '/s4')
open(d + '/s4/README.md', 'w').write('# slice\n```\n' + line(2, 'done', tally(50, 0), 300) + '```\n')
PY
T="--table $tmp/table.txt --mustpass $tmp/must.txt"
out=$(python3 -I "$tool" $T "$tmp/s1")
cls() { printf '%s\n' "$out" | awk -F'\t' -v n="$1" '$1==n {print $7}'; }
col() { printf '%s\n' "$out" | awk -F'\t' -v n="$1" -v c="$2" '$1==n {print $c}'; }

ok "clean (e0)" '[ "$(cls 0)" = clean ]'
ok "watchdog (e1): shortfall 37, back_after 690" '[ "$(cls 1)" = watchdog ] && [ "$(col 1 4)" = 37 ] && [ "$(col 1 5)" = 690 ] && [ "$(col 1 3)" = 63 ]'
ok "early-stop (e2)" '[ "$(cls 2)" = early-stop ] && [ "$(col 2 4)" = 50 ]'
ok "gpu-reset from resets= (e3)" '[ "$(cls 3)" = gpu-reset ]'
ok "unproven: back_after between the limits (e4)" '[ "$(cls 4)" = unproven ]'
ok "unproven: complete count, sequence byte says begun-no-end (e5)" '[ "$(cls 5)" = unproven ] && [ "$(col 5 6)" = begun-no-end ]'
ok "unproven: no record (e6)" '[ "$(cls 6)" = unproven ] && [ "$(col 6 6)" = n/a ]'
ok "watchdog with a begun-no-end case (e7)" '[ "$(cls 7)" = watchdog ] && [ "$(col 7 6)" = begun-no-end ]'
ok "kind 9 record (<=32 cases) is clean (e9)" '[ "$(cls 9)" = clean ] && [ "$(col 9 2)" = 10 ] && [ "$(col 9 3)" = 10 ]'
ok "sequence verdict ok on a clean end" '[ "$(col 0 6)" = ok ]'
ok "a reason is the last column" '[ -n "$(col 0 8)" ] && [ "$(printf "%s\n" "$out" | head -1 | awk -F"\t" "{print NF}")" = 8 ]'

out=$(python3 -I "$tool" $T --scan "$tmp/scan.txt" "$tmp/s1")
ok "console scan: recoveries > 0 makes e8 gpu-reset" '[ "$(cls 8)" = gpu-reset ]'
out=$(python3 -I "$tool" $T --scan "$tmp/scan.txt" "$tmp/s4")
ok "console scan: watchdog end with 50 short and a short back_after is a watchdog" '[ "$(cls 2)" = watchdog ]'
out=$(python3 -I "$tool" $T "$tmp/s2")
ok "an entry twice: the last line counts" '[ "$(printf "%s\n" "$out" | wc -l)" = 1 ] && [ "$(cls 1)" = clean ]'

python3 -I "$tool" $T "$tmp/bad" >/dev/null 2>&1 && rc=0 || rc=$?
ok "unreadable input (no verdict line) exits 2" '[ "$rc" = 2 ]'
python3 -I "$tool" $T "$tmp/nothere" >/dev/null 2>&1 && rc=0 || rc=$?
ok "unreadable input (no directory) exits 2" '[ "$rc" = 2 ]'
python3 -I "$tool" $T "$tmp/s1" >/dev/null 2>&1 && rc=0 || rc=$?
ok "an unproven or non-clean entry still exits 0" '[ "$rc" = 0 ]'

sum=$(python3 -I "$tool" --expected | awk -F'\t' '$1=="sum" {print $2}')
n=$(python3 -I "$tool" --expected | grep -c '^[0-9]')
ok "expected begun over the 133 entries sums to 42810" '[ "$sum" = 42810 ] && [ "$n" = 133 ]'
python3 -I "$tool" --help >/dev/null 2>&1 && rc=0 || rc=$?
ok "--help works" '[ "$rc" = 0 ]'

[ "$fail" = 0 ] && echo "x2class selftest: all ok"
exit "$fail"
