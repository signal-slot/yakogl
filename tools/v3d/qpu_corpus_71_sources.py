#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Write qpu_corpus_71_sources.tsv: the GLSL source and stage of every
program of qpu_corpus_71.txt.

qpu_corpus_71.txt holds only the distinct instruction words; the programs it
was made from (one vertex/fragment pair each, "# pairs: N ok of M formed" in
its header) are listed by name in qpu_counts_71.tsv, which qpu_corpus.py wrote
from the same run. A program's key is "<vs>+<fs>", the two paths relative to
tools/shaders. Each program yields three rows, one per stage Mesa compiled for
it: fs (the fragment shader's source), vs (the vertex shader's) and cs (the
coordinate shader, which Mesa derives from the vertex shader: the same source).

Columns: program, stage, source (a path relative to tools/shaders). Sorted,
one header line. Nothing about the 4.2 compile status is recorded here.

Usage: python3 -I tools/v3d/qpu_corpus_71_sources.py [--check]
  --check  compare with the file on disk instead of writing it
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SHADERS = os.path.normpath(os.path.join(HERE, "..", "shaders"))
CORPUS = os.path.join(HERE, "qpu_corpus_71.txt")
COUNTS = os.path.join(HERE, "qpu_counts_71.tsv")
OUT = os.path.join(HERE, "qpu_corpus_71_sources.tsv")
HEADER = "program\tstage\tsource\n"


def fail(msg):
    raise SystemExit("qpu_corpus_71_sources: FAIL: " + msg)


def main():
    with open(CORPUS) as f:
        m = re.search(r"^# pairs: (\d+) ok of \d+ formed$", f.read(), re.M)
    if not m:
        fail("no '# pairs:' header in qpu_corpus_71.txt")
    want = int(m.group(1))

    progs = set()
    rows = []
    with open(COUNTS) as f:
        for line in f:
            if line.startswith("#") or not line.strip():
                continue
            cols = line.rstrip("\n").split("\t")
            if len(cols) != 5:
                fail("bad row in qpu_counts_71.tsv: %r" % line)
            vs, fs = cols[0], cols[1]
            if not vs.endswith(".vert") or not fs.endswith(".frag"):
                fail("unexpected extensions: %s, %s" % (vs, fs))
            for p in (vs, fs):
                if not os.path.isfile(os.path.join(SHADERS, p)):
                    fail("%s is not under tools/shaders" % p)
            key = "%s+%s" % (vs, fs)
            if key in progs:
                fail("program listed twice: " + key)
            progs.add(key)
            rows += [(key, "cs", vs), (key, "fs", fs), (key, "vs", vs)]
    if len(progs) != want:
        fail("%d programs in qpu_counts_71.tsv, the corpus header says %d" % (len(progs), want))
    text = HEADER + "".join("%s\t%s\t%s\n" % r for r in sorted(rows))
    if "--check" in sys.argv[1:]:
        with open(OUT) as f:
            if f.read() != text:
                fail("%s is stale" % OUT)
        return
    with open(OUT, "w", newline="\n") as f:
        f.write(text)
    print("qpu_corpus_71_sources: %d programs, %d rows -> %s" % (len(progs), len(rows), OUT))


if __name__ == "__main__":
    main()
