#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# counts71: qpu_counts_71.tsv is the per-program table of qpu_corpus_71.txt, as
# tools/v3d/qpu_corpus.py --device 71 writes both: the header of the corpus
# (pairs, words with repeats per stage, unique) equals the rows of the table,
# and the corpus holds that many distinct 64-bit words, sorted. Host only, no
# Mesa input. Convention: tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1
mkdir -p build/v3d71/counts71 || exit 1
exec python3 -I - tools/v3d/qpu_corpus_71.txt tools/v3d/qpu_counts_71.tsv <<'PY'
import re
import sys

corpus, counts = sys.argv[1:3]
bad = []
hdr = {}
words = []
for line in open(corpus):
    line = line.rstrip("\n")
    if line.startswith("#"):
        m = re.match(r"# pairs: (\d+) ok of (\d+) formed$", line)
        if m:
            hdr["pairs"] = int(m.group(1))
        m = re.match(r"# words: (\d+) with repeats \(fs (\d+), vs (\d+), cs (\d+)\)$", line)
        if m:
            hdr["words"], hdr["fs"], hdr["vs"], hdr["cs"] = map(int, m.groups())
        m = re.match(r"# unique: (\d+)$", line)
        if m:
            hdr["unique"] = int(m.group(1))
    elif re.fullmatch(r"0x[0-9a-f]{16}", line):
        words.append(int(line, 16))
    else:
        bad.append("corpus: unexpected line %r" % line)
for k in ("pairs", "words", "fs", "vs", "cs", "unique"):
    if k not in hdr:
        bad.append("corpus: header has no %s" % k)

rows = []
seen = set()
for line in open(counts):
    if line.startswith("#") or not line.strip():
        continue
    f = line.rstrip("\n").split("\t")
    if len(f) != 5 or not all(x.isdigit() for x in f[2:]):
        bad.append("counts: malformed row %r" % line)
        continue
    if (f[0], f[1]) in seen:
        bad.append("counts: duplicate row %s + %s" % (f[0], f[1]))
    seen.add((f[0], f[1]))
    rows.append(list(map(int, f[2:])))

if not bad:
    tot = [sum(r[i] for r in rows) for i in range(3)]
    for name, got, want in (("rows", len(rows), hdr["pairs"]), ("fs", tot[0], hdr["fs"]),
                            ("vs", tot[1], hdr["vs"]), ("cs", tot[2], hdr["cs"]),
                            ("words", sum(tot), hdr["words"]),
                            ("unique words", len(words), hdr["unique"])):
        if got != want:
            bad.append("%s: table/corpus has %d, header says %d" % (name, got, want))
    if words != sorted(set(words)):
        bad.append("corpus words are not sorted and distinct")
for b in bad:
    print("FAIL:", b)
if bad:
    sys.exit(1)
print("counts71: %d programs, %d words (%d unique) agree, 0 failed" % (len(rows), hdr["words"], len(words)))
PY
