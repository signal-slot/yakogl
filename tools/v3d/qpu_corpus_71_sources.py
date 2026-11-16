#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Write tools/v3d/qpu_corpus_71_sources.tsv: the GLSL source each stage of
every program of tools/v3d/qpu_corpus_71.txt is compiled from, for the 7.1
corpus comparison (corpus71) of the V3D 7.1 back end (plan #365, issue #7).

qpu_corpus_71.txt holds words, not programs. Its header names the programs
(`# pairs: N ok of M formed`, and the fs, vs and cs word counts), and
qpu_counts_71.tsv -- written by the same qpu_corpus.py --device 71 run --
lists them by name: the vertex and fragment source of every pair, as
tools/v3d/glsl2v3d.py sees them under tools/shaders. A program of the corpus
is a pair: glsl2v3d.py's header quotes the rule -- "v3d_shc compiles the FS,
then the VS keyed by the FS's inputs, then the CS (the same VS for binning),
so a stage is not a unit here; a pair is." This table unfolds each pair into
its three stage programs, each row carrying the source that stage is compiled
from (fs from the fragment shader; vs and cs, the coordinate shader, from the
vertex shader) under the program key `vs + fs` -- the form
tests/v3d71_verify_corpus.rs names its programs by.

  columns:  program  stage  source
  order:    by program (byte order of the key), then the stages in the order
            v3d_shc compiles them (fs, vs, cs)

Needs no Mesa build, no glslang and no v3d_shc: everything comes from the
corpus's own header, qpu_counts_71.tsv and the sources. It refuses (and
writes nothing) when the counts table is malformed or lists a pair twice,
its program count is not the corpus's `# pairs: N ok`, its per-stage counts
do not add up to the corpus's `# words: ... (fs .., vs .., cs ..)`, a source
is not a file under tools/shaders, or a name is not a .vert/.frag of its
stage. It records no 4.2 compile status: corpus71 measures what our back end
compiles at 7.1, and that moves. tests/v3d71_corpus_sources.rs pins the
table's shape and its counts.

Usage: qpu_corpus_71_sources.py [--out FILE]
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", ".."))
SHADERS = os.path.join(REPO, "tools", "shaders")
CORPUS = os.path.join(HERE, "qpu_corpus_71.txt")
COUNTS = os.path.join(HERE, "qpu_counts_71.tsv")
OUT = os.path.join(HERE, "qpu_corpus_71_sources.tsv")

# The stages, in the order v3d_shc compiles them, and the source each takes.
STAGES = ("fs", "vs", "cs")
STAGE_EXT = {"fs": ".frag", "vs": ".vert", "cs": ".vert"}


def fail(msg):
    raise SystemExit("qpu_corpus_71_sources: FAIL: %s" % msg)


def corpus_header():
    """(pairs, (fs, vs, cs) words with repeats) from qpu_corpus_71.txt."""
    pairs = words = None
    with open(CORPUS) as f:
        for line in f:
            if not line.startswith("#"):
                break
            m = re.match(r"# pairs: (\d+) ok of \d+ formed$", line.rstrip("\n"))
            if m:
                pairs = int(m.group(1))
            m = re.match(r"# words: \d+ with repeats \(fs (\d+), vs (\d+), cs (\d+)\)$",
                         line.rstrip("\n"))
            if m:
                words = tuple(map(int, m.groups()))
    if pairs is None:
        fail("%s has no `# pairs: N ok of M formed` line" % os.path.relpath(CORPUS, REPO))
    if words is None:
        fail("%s has no `# words: N with repeats (fs .., vs .., cs ..)` line"
             % os.path.relpath(CORPUS, REPO))
    return pairs, words


def programs():
    """(vs, fs, {stage: count}) for every row of qpu_counts_71.tsv."""
    out, seen = [], set()
    with open(COUNTS) as f:
        for n, line in enumerate(f, 1):
            if line.startswith("#") or not line.strip():
                continue
            c = line.rstrip("\n").split("\t")
            if len(c) != 5 or not all(x.isdigit() for x in c[2:]):
                fail("%s:%d: malformed row %r" % (os.path.relpath(COUNTS, REPO), n,
                                                  line.rstrip("\n")))
            vs, fs = c[0], c[1]
            for rel, st in ((vs, "vs"), (fs, "fs")):
                if not rel.endswith(STAGE_EXT[st]):
                    fail("%s:%d: %s is not a %s source"
                         % (os.path.relpath(COUNTS, REPO), n, rel, STAGE_EXT[st]))
            if (vs, fs) in seen:
                fail("%s:%d: %s + %s listed twice"
                     % (os.path.relpath(COUNTS, REPO), n, vs, fs))
            seen.add((vs, fs))
            out.append((vs, fs, dict(zip(STAGES, map(int, c[2:])))))
    return out


def main():
    ap = argparse.ArgumentParser(
        description="Derive tools/v3d/qpu_corpus_71_sources.tsv from "
                    "qpu_corpus_71.txt's header and qpu_counts_71.tsv.")
    ap.add_argument("--out", help="write here instead (default: the tracked table)")
    args = ap.parse_args()
    out_path = args.out or OUT

    pairs, words = corpus_header()
    progs = programs()
    if len(progs) != pairs:
        fail("qpu_counts_71.tsv lists %d programs, qpu_corpus_71.txt says %d"
             % (len(progs), pairs))
    for st, want in zip(STAGES, words):
        got = sum(p[2][st] for p in progs)
        if got != want:
            fail("qpu_counts_71.tsv's %s counts add to %d, qpu_corpus_71.txt says %d"
                 % (st, got, want))
    for rel in sorted({p[0] for p in progs} | {p[1] for p in progs}):
        if not os.path.isfile(os.path.join(SHADERS, rel)):
            fail("%s is not a file under tools/shaders" % rel)

    rows = []
    for vs, fs, _ in progs:
        key = "%s + %s" % (vs, fs)
        for st in STAGES:
            rows.append((key, st, fs if st == "fs" else vs))
    rows.sort(key=lambda r: (r[0], STAGES.index(r[1])))
    n_vsrc = len({r[2] for r in rows if r[1] != "fs"})
    n_fsrc = len({r[2] for r in rows if r[1] == "fs"})

    with open(out_path, "w") as f:
        f.write("# SPDX-License-Identifier: Apache-2.0\n")
        f.write("# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n")
        f.write("# tools/v3d/qpu_corpus_71_sources.tsv: the GLSL source each stage of\n")
        f.write("# every program of tools/v3d/qpu_corpus_71.txt is compiled from, one row\n")
        f.write("# per fs, vs and cs program, keyed by the `vs + fs` pair it belongs to.\n")
        f.write("# Generated by tools/v3d/qpu_corpus_71_sources.py from qpu_corpus_71.txt's\n")
        f.write("# header, qpu_counts_71.tsv and tools/shaders; do not edit.\n")
        f.write("# programs: %d pairs (qpu_corpus_71.txt `# pairs:`), rows: %d (fs, vs and\n"
                % (pairs, len(rows)))
        f.write("# cs per program), sources: %d (%d vertex, %d fragment).\n"
                % (n_vsrc + n_fsrc, n_vsrc, n_fsrc))
        f.write("# columns: program (`<vertex> + <fragment>`), stage, source (under tools/shaders)\n")
        f.write("# order: by program, then fs, vs, cs (the order v3d_shc compiles them in)\n")
        for key, st, src in rows:
            f.write("%s\t%s\t%s\n" % (key, st, src))
    print("qpu_corpus_71_sources: %d programs, %d rows, %d sources (%d vertex, %d fragment) -> %s"
          % (pairs, len(rows), n_vsrc + n_fsrc, n_vsrc, n_fsrc, out_path))


if __name__ == "__main__":
    sys.exit(main())
