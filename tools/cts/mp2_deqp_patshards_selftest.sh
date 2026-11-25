#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_deqp_patshards_selftest.sh
#
# Host-only check of tools/cts/mp2-deqp-patshards.py on a small generated
# mustpass list and on the real files: every case once, the excludes honoured,
# the budgets, the slot's real room (--slot minus 1: the NUL), refusals write
# nothing, two runs give the same bytes.
set -u

HERE=$(dirname "$0")
TOOL=$HERE/mp2-deqp-patshards.py
D=$(mktemp -d "${TMPDIR:-.}/mp2-patshards.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }
run() { python3 -I "$TOOL" "$@"; }

# fixture: groups a (4 cases), b (3), texture (x: 3, y: 3), c (1), skipped (not listed)
P=dEQP-GLES2.functional
{
  for i in 1 2 3 4; do echo "$P.a.c$i"; done
  for i in 1 2 3; do echo "$P.b.c$i"; done
  for i in 1 2 3; do echo "$P.texture.x.c$i"; done
  for i in 1 2 3; do echo "$P.texture.y.c$i"; done
  echo "$P.c.only"
  echo "$P.skipped.c1"
} >"$D/must.txt"
printf '%s\n' "$P.a.*" "$P.b.*" "$P.texture.*" "$P.c.*" >"$D/list.txt"
printf '%s\t%s\n' "$P.b.c2" "why" "$P.texture.y.*" "why" >"$D/excl.txt"
F=(--mustpass "$D/must.txt" --list "$D/list.txt" --exclude "$D/excl.txt")
# the case set the tool must cover: mustpass under the list, minus excludes
grep -v -e skipped -e 'b\.c2' -e 'texture\.y' "$D/must.txt" | sort >"$D/want"

# the cases a set of filter files runs, against the fixture's mustpass list
cover() {
  python3 -I - "$D/must.txt" "$@" <<'PY'
import sys
must = [l.strip() for l in open(sys.argv[1]) if l.strip()]
for fn in sys.argv[2:]:
    for p in open(fn).read().strip().split(","):
        for c in must:
            if c == p or (p.endswith("*") and c.startswith(p[:-1])):
                print(c)
PY
}
# column N of a manifest's data rows, largest value
maxcol() { tail -n +2 "$1" | cut -f"$2" | sort -n | tail -n 1; }

run "${F[@]}" --out "$D/o1" --budget-s 3600 >"$D/o1.log"
chk "10 cases, one shard" grep -q " 10 cases, 1 shards" "$D/o1.log"
chk "excluded case b.c2: b.c1 and b.c3 by name" grep -q "$P.b.c1,$P.b.c3" "$D/o1/filter-0000.txt"
chk "excluded group is in no filter" bash -c "! grep -q 'texture.y' '$D/o1/filter-0000.txt'"
chk "clean group is one prefix pattern" grep -q "$P.a.\*" "$D/o1/filter-0000.txt"
chk "excluded.txt lists the exclude names" test "$(cat "$D/o1/excluded.txt")" = "$P.b.c2
$P.texture.y.*"
chk "0 extra cases" grep -q ' 0 extra cases' "$D/o1.log"

# 150 ms a case, 0.75 s budget = 5 cases a shard
run "${F[@]}" --out "$D/o2" --budget-s 0.75 >/dev/null
chk "budget: no shard over 5 cases" test "$(maxcol "$D/o2/manifest.tsv" 2)" -le 5
chk "budget: several shards" test "$(tail -n +2 "$D/o2/manifest.tsv" | wc -l)" -ge 2
cover "$D"/o2/filter-*.txt | sort >"$D/got2"
chk "budget: every case exactly once" cmp -s "$D/got2" "$D/want"

# per-group override: texture cases at 1000 ms, budget 2.5 s
run "${F[@]}" --out "$D/o3" --budget-s 2.5 --group-ms 'texture.*=1000' >/dev/null
chk "group-ms: no shard over 2.5 s" test "$(maxcol "$D/o3/manifest.tsv" 3 | cut -d. -f1)" -le 2
# console bytes: 300 B a case, 1000 B console = 3 cases a shard
run "${F[@]}" --out "$D/o4" --console-bytes 1000 >/dev/null
chk "console: no shard over 1000 bytes" test "$(maxcol "$D/o4/manifest.tsv" 4)" -le 1000

# slot: the room is slot-1 (the NUL). One group a: its pattern is L bytes, its
# cases are longer. --slot L must refuse; --slot L+1 holds the pattern.
printf '%s\n' "$P.a.*" >"$D/lista.txt"
: >"$D/excla.txt"
FA=(--mustpass "$D/must.txt" --list "$D/lista.txt" --exclude "$D/excla.txt")
L=${#P}
L=$((L + 4))   # "<P>.a.*"
run "${FA[@]}" --out "$D/sa" --slot $((L + 1)) >"$D/sa.log" 2>&1
chk "slot L+1: filter of L bytes accepted" test "$(maxcol "$D/sa/manifest.tsv" 5)" = "$L"
chk "slot L+1: summary says of $L" grep -q "of $L," "$D/sa.log"
run "${FA[@]}" --out "$D/sb" --slot "$L" >/dev/null 2>&1
chk "slot L: filter of exactly --slot bytes refused" test "$?" = 1
chk "slot L: nothing written" bash -c "! [ -e '$D/sb' ]"
# a tight slot on the full fixture splits, never reaches --slot, covers each case once
run "${F[@]}" --out "$D/s3" --slot 200 >/dev/null
chk "slot 200: no filter of 200 bytes" test "$(maxcol "$D/s3/manifest.tsv" 5)" -lt 200
cover "$D"/s3/filter-*.txt | sort >"$D/got3"
chk "slot 200: every case exactly once" cmp -s "$D/got3" "$D/want"

# refusal writes nothing
run "${F[@]}" --out "$D/r" --case-ms 5000 --budget-s 1 >/dev/null 2>&1
chk "refusal: exit 1" test "$?" = 1
chk "refusal: nothing written" bash -c "! [ -e '$D/r' ]"

# real files: 17,144 cases (mp2-deqp-list-check.sh), 0 extra, deterministic
RA=(--group-ms 'texture.*=750' --group-bytes 'texture.*=1300')
run --out "$D/ra" "${RA[@]}" >"$D/ra.log"
run --out "$D/rb" "${RA[@]}" >/dev/null
chk "real: 17144 cases" grep -q ' 17144 cases' "$D/ra.log"
chk "real: 0 extra cases" grep -q ' 0 extra cases' "$D/ra.log"
chk "real: largest filter below 16383" test "$(maxcol "$D/ra/manifest.tsv" 5)" -lt 16383
chk "real: summary says of 16383" grep -q 'of 16383,' "$D/ra.log"
chk "real: deterministic" diff -r "$D/ra" "$D/rb"
chk "real: same count as mp2-deqp-list-check.sh" bash -c "sh '$HERE/mp2-deqp-list-check.sh' | grep -q ' 17144 mustpass'"

printf '%d ok, %d failed\n' "$pass" "$fail"
[ "$fail" = 0 ]
