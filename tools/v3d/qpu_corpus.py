#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Collect the V3D 4.2 QPU instruction words of every program glsl2v3d.py
compiles into one deduplicated fixture, for the round-trip gate of
src/shaderc/online/v3d (roundtrip.rs).

It runs glsl2v3d.py with --json and --keep (or reads the output of an earlier
run with --from), takes the fs, vs and cs `code` arrays of every pair whose
status is ok, and writes each distinct 64-bit word once, sorted, one per line.

The header records what the words came from, and the Rust test pins its
numbers, so a corpus that grows or shrinks fails the pin instead of widening
the gate without anyone deciding it:

  # pairs: <ok> ok of <formed> formed
  # words: <n> with repeats (fs <a>, vs <b>, cs <c>)
  # unique: <u>

Nothing is skipped silently: an ok pair without an out.json, or with a stage
that has no `code`, fails the run. The known positive of glsl2v3d.py
(color_uv.vert + color_uv.frag, 12/31/22 instructions) is checked here too,
so a changed naming of the --keep directories cannot make the words vanish.

Usage: qpu_corpus.py [--device 42|71] [--rev N] [--compat-rev N] [--out FILE]
                     [--from DIR] [--jobs N]
  --from DIR  reuse DIR/pairs.json and DIR/keep/ from an earlier run
              (glsl2v3d.py --json DIR/pairs.json --keep DIR/keep)
  --device 71 compiles for the V3D 7.1 (Raspberry Pi 5) and writes
              qpu_corpus_71.txt next to the 4.2 file, which is not touched. The
              known positive's instruction counts are pinned for 4.2 only; for
              7.1 the header records the counts the run gave. --rev and
              --compat-rev set the hardware revision (v3d_shc; default 7 and 0,
              what Mesa's drm-shim reports): the words depend on it.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", ".."))
DEFAULT_OUT = {"42": os.path.join(HERE, "qpu_corpus.txt"),
               "71": os.path.join(HERE, "qpu_corpus_71.txt")}
STAGES = ("fs", "vs", "cs")

# The same pair and counts as glsl2v3d.py's KNOWN_POSITIVE, restated so that
# this script does not import the pairing machinery.
KNOWN_POSITIVE = ("color_uv.vert", "color_uv.frag")
KNOWN_POSITIVE_COUNTS = {"fs": 12, "vs": 31, "cs": 22}


def keep_dir(keep, vs, fs):
    """The directory glsl2v3d.py's run_pair() writes a pair's out.json to."""
    return os.path.join(keep, re.sub(r"[^\w.]", "_", vs) + "__" + re.sub(r"[^\w.]", "_", fs))


def build_id():
    try:
        return subprocess.run(["git", "-C", REPO, "describe", "--always", "--dirty"],
                              capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--device", choices=("42", "71"), default="42")
    ap.add_argument("--rev", type=int, help="7.1 hardware revision (default: v3d_shc's, 7)")
    ap.add_argument("--compat-rev", type=int, help="7.1 compat revision (default: v3d_shc's, 0)")
    ap.add_argument("--out")
    ap.add_argument("--from", dest="src", help="directory with pairs.json and keep/")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    args = ap.parse_args()
    out_path = args.out or DEFAULT_OUT[args.device]

    src = args.src
    if src is None:
        src = tempfile.mkdtemp(prefix="qpu_corpus-")
        cmd = [sys.executable, os.path.join(HERE, "glsl2v3d.py"),
               "--jobs", str(args.jobs), "--json", os.path.join(src, "pairs.json"),
               "--keep", os.path.join(src, "keep"), "--device", args.device]
        if args.rev is not None:
            cmd += ["--rev", str(args.rev)]
        if args.compat_rev is not None:
            cmd += ["--compat-rev", str(args.compat_rev)]
        r = subprocess.run(cmd)
        if r.returncode != 0:
            raise SystemExit("qpu_corpus: glsl2v3d.py exited %d" % r.returncode)
    with open(os.path.join(src, "pairs.json")) as f:
        js = json.load(f)
    if js.get("device", "42") != args.device:
        raise SystemExit("qpu_corpus: FAIL: %s holds a --device %s run, asked for %s"
                         % (src, js.get("device", "42"), args.device))
    pairs = js["pairs"]
    keep = os.path.join(src, "keep")

    ok = [p for p in pairs if p["status"] == "ok"]
    if not ok:
        raise SystemExit("qpu_corpus: FAIL: no pair compiled")
    words = set()
    per_stage = {st: 0 for st in STAGES}
    known = None
    rows = []  # per pair: vs, fs and the instruction count of each stage
    revs = set()  # (rev, compat_rev) the 7.1 runs named
    for p in ok:
        path = os.path.join(keep_dir(keep, p["vs"], p["fs"]), "out.json")
        if not os.path.isfile(path):
            raise SystemExit("qpu_corpus: FAIL: ok pair %s + %s has no %s" % (p["vs"], p["fs"], path))
        with open(path) as f:
            out = json.load(f)
        gpu = out.get("gpu") or {}
        if gpu.get("ver") != int(args.device):
            raise SystemExit("qpu_corpus: FAIL: %s + %s was compiled for ver %s, not %s"
                             % (p["vs"], p["fs"], gpu.get("ver"), args.device))
        revs.add((gpu.get("rev"), gpu.get("compat_rev")))
        counts = {}
        for st in STAGES:
            code = (out.get(st) or {}).get("code")
            if not code:
                raise SystemExit("qpu_corpus: FAIL: %s + %s: stage %s has no code"
                                 % (p["vs"], p["fs"], st))
            counts[st] = len(code)
            per_stage[st] += len(code)
            for w in code:
                v = int(w, 16)
                if not 0 <= v < 1 << 64:
                    raise SystemExit("qpu_corpus: FAIL: %s is not a 64-bit word" % w)
                words.add(v)
        rows.append((p["vs"], p["fs"], counts["fs"], counts["vs"], counts["cs"]))
        if (p["vs"], p["fs"]) == KNOWN_POSITIVE:
            known = counts
    if known is None or (args.device == "42" and known != KNOWN_POSITIVE_COUNTS):
        raise SystemExit("qpu_corpus: FAIL: known positive %s + %s read as %s (want %s)"
                         % (KNOWN_POSITIVE + (known, KNOWN_POSITIVE_COUNTS)))

    total = sum(per_stage.values())
    build = build_id()  # before the file is truncated, or a tracked corpus reads -dirty
    if len(revs) != 1:
        raise SystemExit("qpu_corpus: FAIL: the pairs name different revisions: %s" % sorted(revs, key=str))
    (rev, compat_rev), = revs
    with open(out_path, "w") as f:
        if args.device == "42":
            f.write("# V3D 4.2 QPU instruction words: every distinct word of the fs, vs and cs\n")
            f.write("# of every program tools/v3d/glsl2v3d.py compiles, sorted. Generated by\n")
            f.write("# tools/v3d/qpu_corpus.py; the round trip in src/shaderc/online/v3d pins\n")
            f.write("# the three counts below.\n")
        else:
            f.write("# V3D 7.1 QPU instruction words: every distinct word of the fs, vs and cs\n")
            f.write("# of every program tools/v3d/glsl2v3d.py compiles for V3D 7.1, sorted. The\n")
            f.write("# 7.1 twin of qpu_corpus.txt (same programs, v3d_shc --device 71). Test\n")
            f.write("# data for the 7.1 ISA work; nothing pins these counts yet.\n")
            f.write("# generator: tools/v3d/qpu_corpus.py --device 71%s%s\n"
                    % ("" if args.rev is None else " --rev %d" % args.rev,
                       "" if args.compat_rev is None else " --compat-rev %d" % args.compat_rev))
            f.write("# device: ver 71, rev %s, compat_rev %s (v3d_shc defaults are what Mesa's\n"
                    % (rev, compat_rev))
            f.write("# drm-shim reports; the Pi 5's real revision is not known)\n")
        f.write("# build: %s\n" % build)
        f.write("# v3d_shc: %s\n" % os.path.basename(js.get("v3d_shc", "unknown")))
        f.write("# pairs: %d ok of %d formed\n" % (len(ok), len(pairs)))
        f.write("# gl_test sites not read: %d (glsl2v3d.py KNOWN_UNRESOLVED)\n"
                % len(js.get("known_unresolved_sites", [])))
        f.write("# words: %d with repeats (fs %d, vs %d, cs %d)\n"
                % (total, per_stage["fs"], per_stage["vs"], per_stage["cs"]))
        f.write("# unique: %d\n" % len(words))
        for v in sorted(words):
            f.write("0x%016x\n" % v)
    # The instruction count of every program, one file per device, so that the
    # 7.1 count sits beside the 4.2 one.
    counts_path = (os.path.splitext(args.out)[0] + ".counts.tsv" if args.out
                   else os.path.join(HERE, "qpu_counts_%s.tsv" % args.device))
    with open(counts_path, "w") as f:
        f.write("# Instructions per program (a 64-bit word each), V3D %s, from the run that\n"
                % ("4.2" if args.device == "42" else "7.1"))
        f.write("# wrote %s (tools/v3d/qpu_corpus.py --device %s).\n"
                % (os.path.basename(out_path), args.device))
        f.write("# columns: vertex shader, fragment shader, fs, vs, cs\n")
        for row in sorted(rows):
            f.write("%s\t%s\t%d\t%d\t%d\n" % row)
    print("qpu_corpus: %d pairs ok of %d; %d words (fs %d, vs %d, cs %d); %d unique -> %s"
          % (len(ok), len(pairs), total, per_stage["fs"], per_stage["vs"], per_stage["cs"],
             len(words), out_path))


if __name__ == "__main__":
    main()
