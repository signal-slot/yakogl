#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Write tools/v3d/qpu_corpus_71_sources.tsv: the GLSL source of each stage of
every program in qpu_corpus_71.txt, for the V3D 7.1 back end's corpus
comparison (plan #365).

A program of the 7.1 corpus is a pair (vertex shader, fragment shader) that
glsl2v3d.py formed and v3d_shc compiled; qpu_counts_71.tsv lists them, by
their paths under tools/shaders, with Mesa's fs, vs and cs instruction counts.
This table gives each of the three stages its own row: the file it is
compiled from (the fs from the fragment shader; the vs and the cs, the
coordinate shader v3d_shc compiles from the same source for binning, from the
vertex shader), the GLSL dialect glsl2v3d.py's translate() reads it as, and
Mesa's instruction count for that stage.

  dialect  es100: GLSL ES 1.00 (also version-less legacy text);
           es300, es310: GLSL ES 3.x; gl: any other text (translate_450_gl).

Needs no Mesa build, no glslang and no v3d_shc: the table is derived from
qpu_counts_71.tsv and the sources alone. It refuses (and writes nothing) when
a source is missing, a name is not a .vert/.frag of its stage, a program is
listed twice, or the program count is not the `# pairs: N ok` of
qpu_corpus_71.txt.

Usage: qpu_corpus_71_sources.py [--check] [--out FILE]
  --check  write nothing; exit 1 when the table is not what this writes
           (tools/v3d/check71.d/sources71.sh runs it so)
"""
import argparse
import collections
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", ".."))
SHADERS = os.path.join(REPO, "tools", "shaders")
CORPUS = os.path.join(HERE, "qpu_corpus_71.txt")
COUNTS = os.path.join(HERE, "qpu_counts_71.tsv")
OUT = os.path.join(HERE, "qpu_corpus_71_sources.tsv")

sys.path.insert(0, os.path.join(REPO, "tools", "vivante"))
import es100 as u  # noqa: E402  (stage_of, es_version: what glsl2v3d.translate() uses)

STAGES = ("fs", "vs", "cs")


def fail(msg):
    raise SystemExit("qpu_corpus_71_sources: FAIL: %s" % msg)


def corpus_pairs():
    """The `# pairs: N ok of M formed` count of qpu_corpus_71.txt."""
    with open(CORPUS) as f:
        for line in f:
            m = re.match(r"# pairs: (\d+) ok of \d+ formed$", line.rstrip("\n"))
            if m:
                return int(m.group(1))
            if not line.startswith("#"):
                break
    fail("%s has no `# pairs:` line" % os.path.relpath(CORPUS, REPO))


def programs():
    """(vs, fs, {stage: count}) for every row of qpu_counts_71.tsv."""
    out, seen = [], set()
    with open(COUNTS) as f:
        for n, line in enumerate(f, 1):
            if line.startswith("#") or not line.strip():
                continue
            c = line.rstrip("\n").split("\t")
            if len(c) != 5 or not all(x.isdigit() for x in c[2:]):
                fail("qpu_counts_71.tsv:%d: malformed row %r" % (n, line))
            if (c[0], c[1]) in seen:
                fail("qpu_counts_71.tsv:%d: %s + %s listed twice" % (n, c[0], c[1]))
            seen.add((c[0], c[1]))
            out.append((c[0], c[1], dict(zip(STAGES, map(int, c[2:])))))
    return out


def dialect(rel):
    path = os.path.join(SHADERS, rel)
    if not os.path.isfile(path):
        fail("%s is not a file under tools/shaders" % rel)
    with open(path) as f:
        v = u.es_version(f.read())
    return {100: "es100", 300: "es300", 310: "es310", None: "gl"}[v]


def table():
    progs = programs()
    want = corpus_pairs()
    if len(progs) != want:
        fail("qpu_counts_71.tsv has %d programs, qpu_corpus_71.txt says %d" % (len(progs), want))
    dialects = {}
    rows = []
    for vs, fs, counts in progs:
        for rel, st in ((vs, "vs"), (fs, "fs")):
            try:
                got = u.stage_of(rel)
            except SystemExit:
                got = None
            if got != st:
                fail("%s + %s: %s is not a %s source" % (vs, fs, rel, "vertex" if st == "vs" else "fragment"))
            if rel not in dialects:
                dialects[rel] = dialect(rel)
        for st in STAGES:
            src = fs if st == "fs" else vs
            rows.append((vs, fs, st, src, dialects[src], counts[st]))
    by_dialect = collections.Counter(dialects.values())
    lines = [
        "# The GLSL source of each stage of every program in qpu_corpus_71.txt (one",
        "# row per stage; programs and counts from qpu_counts_71.tsv). The fs is",
        "# compiled from the fragment shader, the vs and the cs (the coordinate",
        "# shader, for binning) from the vertex shader. Paths are under tools/shaders.",
        "# dialect: as glsl2v3d.py's translate() reads the source (es100, es300,",
        "# es310, gl). instructions: Mesa 26.2.1's count for that stage at 7.1.",
        "# generator: tools/v3d/qpu_corpus_71_sources.py",
        "# programs: %d (the `pairs` of qpu_corpus_71.txt)" % len(progs),
        "# rows: %d (fs %d, vs %d, cs %d)" % ((len(rows),) + tuple(len(progs) for _ in STAGES)),
        "# sources: %d (vs %d, fs %d; %s)" % (
            len(dialects), len({p[0] for p in progs}), len({p[1] for p in progs}),
            ", ".join("%s %d" % (d, by_dialect[d]) for d in sorted(by_dialect))),
        "# columns: vertex shader, fragment shader, stage, source, dialect, instructions",
    ]
    lines += ["%s\t%s\t%s\t%s\t%s\t%d" % r for r in rows]
    return "\n".join(lines) + "\n", len(rows)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--out", default=OUT)
    args = ap.parse_args()
    text, nrows = table()
    if args.check:
        try:
            with open(args.out) as f:
                have = f.read()
        except OSError as e:
            fail("cannot read %s: %s" % (args.out, e))
        if have != text:
            a, b = have.splitlines(), text.splitlines()
            i = next((k for k in range(min(len(a), len(b))) if a[k] != b[k]), min(len(a), len(b)))
            fail("%s is stale at line %d (have %r, want %r); rerun tools/v3d/qpu_corpus_71_sources.py"
                 % (os.path.relpath(args.out, REPO), i + 1,
                    a[i] if i < len(a) else "<end>", b[i] if i < len(b) else "<end>"))
        print("qpu_corpus_71_sources: %s is current (%d rows)" % (os.path.relpath(args.out, REPO), nrows))
        return
    with open(args.out, "w") as f:
        f.write(text)
    print("qpu_corpus_71_sources: wrote %s" % os.path.relpath(args.out, REPO))


if __name__ == "__main__":
    main()
