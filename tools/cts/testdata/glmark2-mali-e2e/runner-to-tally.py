#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The bridge of tools/cts/glmark2-mali-e2e-selftest.sh: a run directory of
tools/cts/mali-glmark2-run.sh in the layout tools/cts/glmark2-mali-tally.py reads.

  python3 -I runner-to-tally.py <runner outdir> <runner stdout> <glmark2-host-ref.sh --list> \
      <libGLESv2.so.2> <board> <tally rundir>

The runner writes meta.txt, scenes.tsv, log/<n>.out|err and kmsg/<n>.log|class;
the tally reads env.txt and <NN>-<slug>.log|kmsg|status. This test fixture
maps one onto the other and fills what the runner does not write from its
inputs (README.md beside it, "The layout gap"):
  env.txt     commit= and kernel= from meta.txt, date= from its started=,
              board= from the argument, pair_sha256= the sha256 of the
              build's libGLESv2.so.2 (the run directory has none)
  .log        log/<n>.out then log/<n>.err
  .kmsg       tools/cts/mali-kmsg.py --json over kmsg/<n>.log, then
              kmsg/<n>.class and kmsg/<n>.log (the tally takes the JSON:
              the .class line has cs_faults=, which its line pattern lacks)
  .status     exit= the "rc" of the runner's "scene <n>: ..." stdout line (the
              run directory has none), timeout=1 for the verdict hang
The slug is the --list slug of the scene's info string ([name] and its
options sorted, as glmark2 prints it). A not-run scene gets no file.
Exit 1 when the runner's output does not map (a scene not in the list, a
missing file, no rc).
"""

import hashlib
import os
import re
import subprocess
import sys

KMSG = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "mali-kmsg.py")


def die(msg):
    sys.stderr.write("runner-to-tally: %s\n" % msg)
    sys.exit(1)


def read(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError as e:
        die("%s: %s" % (path, e.strerror))


def info_of(scene):
    name, _, opts = scene.partition(":")
    return "[%s] %s" % (name, ":".join(sorted(opts.split(":"))) if opts else "<default>")


def main(outdir, stdout, listing, lib, board, dst):
    meta = dict(l.split("=", 1) for l in read(os.path.join(outdir, "meta.txt")).splitlines() if "=" in l)
    slugs = {}
    for line in read(listing).splitlines():
        slug, _, info = line.partition("\t")
        slugs[info] = slug
    rcs = dict(re.findall(r"^mali-glmark2-run: scene (\d+): [\w-]+, .*, rc (\S+)$", read(stdout), re.M))
    with open(lib, "rb") as f:
        pair = hashlib.sha256(f.read()).hexdigest()
    os.makedirs(dst)
    with open(os.path.join(dst, "env.txt"), "w") as f:
        f.write("commit=%s\nboard=%s\nkernel=%s\npair_sha256=%s\ndate=%s\n"
                % (meta.get("commit", ""), board, meta.get("kernel", ""), pair, meta.get("started", "")[:10]))
    rows = read(os.path.join(outdir, "scenes.tsv")).splitlines()[1:]
    for n, row in enumerate(rows, 1):
        scene, _, verdict = row.split("\t")[:3]
        info = info_of(scene)
        if info not in slugs:
            die("scene %d %s (%s) is not in the --list" % (n, scene, info))
        if verdict == "not-run":
            continue
        if str(n) not in rcs or rcs[str(n)] == "-":
            die("scene %d has no rc in the runner's output" % n)
        base = os.path.join(dst, "%02d-%s" % (n, slugs[info]))
        with open(base + ".log", "w") as f:
            f.write(read(os.path.join(outdir, "log", "%d.out" % n)) + read(os.path.join(outdir, "log", "%d.err" % n)))
        klog = os.path.join(outdir, "kmsg", "%d.log" % n)
        js = subprocess.run([sys.executable, "-I", KMSG, "--json", klog], capture_output=True, text=True)
        if js.returncode != 0 or not js.stdout.startswith("{"):
            die("mali-kmsg.py --json %s: exit %d" % (klog, js.returncode))
        with open(base + ".kmsg", "w") as f:
            f.write(js.stdout + read(os.path.join(outdir, "kmsg", "%d.class" % n)) + read(klog))
        with open(base + ".status", "w") as f:
            f.write("exit=%s\ntimeout=%d\n" % (rcs[str(n)], verdict == "hang"))


if __name__ == "__main__":
    if len(sys.argv) != 7:
        die("usage: runner-to-tally.py <outdir> <stdout> <list> <libGLESv2.so.2> <board> <dst>")
    main(*sys.argv[1:])
