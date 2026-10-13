#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""check_gl_test_blobs.py - every shader gl_test compiles has a row in the table.

The offline backend looks a shader up by the SHA-256 of the text the
application passed to glShaderSource. So a source that gl_test compiles but
that nobody regenerated the table for is not a compile error and not a link
error: it is a miss at draw time, on the board, reported as whatever that
case checks. That is how gl_test case 63 (matattr) came to be unservable on
the Vivante offline path -- tools/shaders/es100/matattr*.{vert,frag} were
added for it and the table was never rebuilt, so the four sources existed,
the case existed, and the blobs did not.

This script asks the question the runtime asks. It takes the exact strings
out of the generated gl_test headers (the bytes glShaderSource is handed),
hashes each one, and looks the digest up in the Rust table -- no list of
expected shaders to fall out of date, because the headers ARE the list.

Usage:
It asks the same question of the other direction too: glsl2usc.sh compiles
the source list's `src` lines (tools/shaders/precompiled.list), so every
source in that list must have a row in the PowerVR table. Absences there fall into two kinds and the check separates them, because
treating one as the other is how a real defect gets excused:

  staleness      the source compiles, nobody regenerated -- what this catches
  uncompilable   the compiler refuses the source; listed in
                 tools/shaders/known-uncompilable.txt with its provenance

Usage:
  python3 tools/vivante/check_gl_test_blobs.py           # both tables
  python3 tools/vivante/check_gl_test_blobs.py --table tests/reference/blobs.rs

Exit status is 1 if any source a generator lists has no row it should have.
"""
import argparse
import glob
import hashlib
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
HEADERS = (
    os.path.join(REPO, "driver", "tools", "pvrlab", "gl_test_shaders.h"),
    os.path.join(REPO, "driver", "tools", "pvrlab", "gl_test_shaders_es100.h"),
)

# `static const char NAME[] =\n\t"line\n"\n\t"line\n";`
DECL = re.compile(r'static const char (\w+)\[\] =\s*((?:\s*"(?:[^"\\]|\\.)*"\s*)+);')


def unescape(lit):
    """One C string literal's bytes, the way the compiler sees it."""
    out, i = [], 0
    while i < len(lit):
        c = lit[i]
        if c == "\\":
            i += 1
            out.append({"n": "\n", "t": "\t", "\\": "\\", '"': '"'}[lit[i]])
        else:
            out.append(c)
        i += 1
    return "".join(out)


def sources(path):
    """name -> the exact text glShaderSource receives."""
    text = open(path).read()
    out = {}
    for name, block in DECL.findall(text):
        parts = re.findall(r'"((?:[^"\\]|\\.)*)"', block)
        out[name] = "".join(unescape(p) for p in parts)
    return out


def table_digests(path):
    """Every sha256 in the table, as lowercase hex."""
    text = open(path).read()
    out = set()
    for row in re.findall(r"sha256: \[(.*?)\]", text, re.S):
        out.add("".join(re.findall(r"0x([0-9a-f]{2})", row)))
    return out


def empty_rows(path):
    """sha256 -> name, for rows whose program is all zeros.

    Its own class, reported and not failed. On the GC7000 an all-zero program
    is one `nop`, and every such row in the table is a PURE PASS-THROUGH whose
    every output is a direct copy of an input: the register wiring routes them
    and no ALU instruction is needed. The board agrees -- cases 1, 21 and 34
    draw with passthrough.vert, 10 and 40 with interp_color.frag, and eighteen
    with color_uv.vert, and they pass; a vertex shader writing no position
    could not. So these are listed, not flagged. They are listed at all because
    "the row is there" and "the row has a program" are different facts, and
    reading one as the other is how a placeholder would slip through.
    """
    text = open(path).read()
    arrays = dict(re.findall(r"static (CODE_\d+): \[u32; \d+\] = \[(.*?)\];", text, re.S))
    out = {}
    body = text[text.index("SHC_BLOB_TABLE") :]
    for row in re.findall(r"    ShcBlob \{(.*?)\n    \},", body, re.S):
        m = re.search(r"usc: (CODE_\d+)\.as_ptr\(\)", row)
        if not m:
            continue
        words = re.findall(r"0x([0-9a-f]+)", arrays.get(m.group(1), "1"))
        if words and all(int(w, 16) == 0 for w in words):
            sha = "".join(re.findall(r"0x([0-9a-f]{2})", re.search(r"sha256: \[(.*?)\]", row, re.S).group(1)))
            out[sha] = re.search(r'name: "(.*?)\\0"', row).group(1)
    return out


# gl_test sources in a gc7000 directory that etna_shc refuses, each with the
# message it gives (tools/vivante/README.md, "The 168 refused sources"). Only
# these are gl_test sources; the other 165 refusals are not in the headers.
# Matched by the file's SHA-256, so an edited source is checked afresh, and a
# listed source that gains a row fails the check until it is dropped here.
LAYOUT = ("layout check: a run-time uniform index reads block dword 4 or 1 "
          "where Mesa's offsets point at 16 or 4")
GC7000_REFUSED = {
    "es100/uniidx_a.frag": LAYOUT,
    "es100/uniidx_b.frag": LAYOUT,
    "es100/uniidx_c.frag": LAYOUT,
}


def refused_digests():
    """sha256 of each GC7000_REFUSED file -> its path under tools/shaders."""
    out = {}
    for rel in GC7000_REFUSED:
        path = os.path.join(REPO, "tools", "shaders", rel)
        if os.path.exists(path):
            out[hashlib.sha256(open(path, "rb").read()).hexdigest()] = rel
        else:
            print("    REFUSED BUT ABSENT  tools/shaders/%s -- drop it from GC7000_REFUSED" % rel)
    return out


def list_roles():
    """dir -> its role in tools/shaders/precompiled.list (`dir NAME ROLE`)."""
    out = {}
    for line in open(os.path.join(REPO, "tools", "shaders", "precompiled.list")):
        f = line.split("#", 1)[0].split()
        if len(f) == 3 and f[0] == "dir":
            out[f[1] if f[1] != "." else ""] = f[2]
    return out


def shader_dirs():
    """sha256 of a file under tools/shaders -> the directories holding it."""
    root = os.path.join(REPO, "tools", "shaders")
    out = {}
    for d, _, files in os.walk(root):
        rel = os.path.relpath(d, root)
        rel = "" if rel == "." else rel
        for f in files:
            if f.endswith((".vert", ".frag")):
                digest = hashlib.sha256(open(os.path.join(d, f), "rb").read()).hexdigest()
                out.setdefault(digest, set()).add(rel)
    return out


def glsl2usc_inputs():
    """The sources glsl2usc.sh compiles: the `src` lines of the source list
    (tools/shaders/precompiled.list; a glob in a file name expands in byte
    order), as tools/precompile/src/list.rs reads them for it."""
    shaders = os.path.join(REPO, "tools", "shaders")
    out = []
    for line in open(os.path.join(shaders, "precompiled.list")):
        f = line.split("#", 1)[0].split()
        if len(f) != 2 or f[0] != "src":
            continue
        if "*" in f[1] or "?" in f[1]:
            found = [g for g in glob.glob(os.path.join(shaders, f[1])) if os.path.isfile(g)]
            out.extend(sorted(os.path.relpath(g, REPO) for g in found))
        else:
            out.append("tools/shaders/" + f[1])
    return out


def known_uncompilable():
    path = os.path.join(REPO, "tools", "shaders", "known-uncompilable.txt")
    if not os.path.exists(path):
        return None
    return {
        ln.strip()
        for ln in open(path)
        if ln.strip() and not ln.startswith("#")
    }


def check_powervr_inputs(have):
    """Every source glsl2usc.sh lists has a PowerVR row, or a recorded reason."""
    excused = known_uncompilable()
    if excused is None:
        print("tools/shaders/known-uncompilable.txt is missing; cannot tell a stale "
              "table from a source the compiler refuses", file=sys.stderr)
        return 2
    rc = 0
    stale, gone = [], []
    for rel in glsl2usc_inputs():
        full = os.path.join(REPO, rel)
        short = os.path.relpath(full, os.path.join(REPO, "tools", "shaders"))
        if not os.path.exists(full):
            print("    LISTED BUT ABSENT  %s" % rel)
            rc = 1
            continue
        digest = hashlib.sha256(open(full, "rb").read()).hexdigest()
        if digest in have:
            if short in excused:
                gone.append(short)
        elif short not in excused:
            stale.append(short)
    print("glsl2usc.sh inputs           %3d sources, %d stale, %d excused"
          % (len(glsl2usc_inputs()), len(stale), len(excused)))
    for n in stale:
        print("    NO ROW (stale table)  %s" % n)
    for n in gone:
        print("    NOW COMPILES          %s -- drop it from known-uncompilable.txt" % n)
    return 1 if (stale or gone) else rc


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--table", default=os.path.join(REPO, "tests", "reference", "vivante_blobs.rs"))
    args = ap.parse_args()

    have = table_digests(args.table)
    blank = empty_rows(args.table)
    if not have:
        print("no sha256 rows parsed out of %s -- the check would pass vacuously"
              % os.path.relpath(args.table, REPO), file=sys.stderr)
        return 2
    print("table %s: %d rows" % (os.path.relpath(args.table, REPO), len(have)))

    rc = 0
    roles, dirs_of = list_roles(), shader_dirs()
    refused = refused_digests()
    if len(refused) != len(GC7000_REFUSED):
        rc = 1
    for d, rel in sorted(refused.items(), key=lambda kv: kv[1]):
        if d in have:
            print("    NOW HAS A ROW  tools/shaders/%s -- drop it from GC7000_REFUSED" % rel)
            rc = 1
    for h in HEADERS:
        if not os.path.exists(h):
            print("MISSING HEADER %s" % os.path.relpath(h, REPO))
            rc = 1
            continue
        srcs = sources(h)
        if not srcs:
            print("no shaders parsed out of %s" % os.path.relpath(h, REPO), file=sys.stderr)
            return 2
        digests = {n: hashlib.sha256(t.encode()).hexdigest() for n, t in srcs.items()}
        absent = sorted(n for n, d in digests.items() if d not in have)
        # A source that lives only in a directory the list gives no gc7000 role
        # (gltest/, offarm/) has no GC7000 row by the list's own decision. That
        # is an open item or a PowerVR-only series, not a table that lags the
        # sources: reported on its own line, and not a failure.
        missing, by_list, by_compiler = [], [], []
        for n in absent:
            dirs = dirs_of.get(digests[n], set())
            if digests[n] in refused:
                by_compiler.append((n, refused[digests[n]]))
            elif dirs and not any(roles.get(x) == "gc7000" for x in dirs):
                by_list.append((n, sorted(dirs)))
            else:
                missing.append(n)
        empties = sorted(n for n, d in digests.items() if d in blank)
        print("%-32s %3d shaders, %d without a row, %d with an empty program, "
              "%d left out by the source list, %d refused by the compiler"
              % (os.path.basename(h), len(srcs), len(missing), len(empties), len(by_list),
                 len(by_compiler)))
        for n in missing:
            print("    NO ROW  %s" % n)
        per_dir = {}
        for n, dirs in by_list:
            per_dir.setdefault(", ".join(dirs), []).append(n)
        for d, names in sorted(per_dir.items()):
            print("    LISTED OUT  %3d from tools/shaders/%s/ (no gc7000 role), e.g. %s"
                  % (len(names), d, names[0]))
        for n, rel in by_compiler:
            print("    REFUSED %s  (tools/shaders/%s: %s)" % (n, rel, GC7000_REFUSED[rel]))
        for n in empties:
            print("    EMPTY   %s  (pass-through: one nop, no ALU work needed)" % n)
        if missing:
            rc = 1

    # The PowerVR arm reads its own table, whichever one was asked for above.
    pvr = os.path.join(REPO, "tests", "reference", "blobs.rs")
    if os.path.exists(pvr):
        rc = max(rc, check_powervr_inputs(table_digests(pvr)))
    return rc


if __name__ == "__main__":
    sys.exit(main())
