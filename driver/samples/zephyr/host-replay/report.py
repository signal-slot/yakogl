#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Summarise a sample's host replay: one log per GPU, what each one found.

usage: report.py <out dir> <gpu>...

Reads <out>/<gpu>.log (the replay program's stdout and stderr in one stream:
its own "replay:" lines and the library's lines, in order) and prints, per
GPU, the GL errors with their entry points, the refused draws, the
incomplete-texture lines, the unresolved GL names, and the lines nobody
expected. Exit status 1 when any GPU found anything, or its log has no
summary line (the run did not finish).
"""
import collections
import os
import re
import sys

# the first match names a line; classes as tools/hostharness/glmark2/report.py
CLASSES = [
    # an allocation refused (the V3D's carve-out, src/osal/linux.c, as the
    # board prints it), or the backend's line about one it could not make
    ("GPU memory", re.compile(r"out of GPU memory|no GPU memory")),
    # the V3D's tiled-copy budget high-water and its sampler-key variants
    # ("N built, M refused"; before "refused draw", which "0 refused (" matches)
    ("tiled copy", re.compile(r"^yakogl/v3d: (tiled copies high-water |a stale tiled copy )")),
    ("sampler variant", re.compile(r"^yakogl/v3d: sampler variants ")),
    ("GL error",re.compile(r"^GL error 0x[0-9a-f]+ in \S+")),
    ("refused draw", re.compile(r"draw (was )?refused|the draw is refused|; refused\b|refused \(")),
    ("incomplete texture", re.compile(r"incomplete")),
    ("unresolved GL name", re.compile(r"^replay: unresolved ")),
    ("frame error", re.compile(r"^replay: frame \d+ .*: rc (?!0 )|^replay: frame \d+ .* glerror 0x(?!0 )")),
    ("panic", re.compile(r"panicked|panic:", re.I)),
    ("dropped", re.compile(r"dropped|did not complete")),
    # glGetUniformLocation of a name the program lacks: -1, no GL error
    # (ES 2.0 2.10.4); femtovg asks for optional uniforms
    ("no such uniform", re.compile(r"has no uniform '")),
]
# listed, not findings: glow resolves every GL name (desktop ones too) at
# context creation and panics only on a call to an unresolved one
INFO = ("unresolved GL name", "no such uniform", "tiled copy", "sampler variant")
NOISE = re.compile(
    r"^\s|^$|^replay: (frame|summary|GL_RENDERER|shader store|window)|^osal\(linux\): heap |^pvr_backend: init ok"
    r"|^slint: |^cluster: |^glcount: "
    # the V3D's carve-out and backend start-up lines, as
    # tools/hostharness/glmark2/report.py takes them
    r"|^osal\(linux\): v3d (carve-out|pending frees)"
    r"|^yakogl/v3d: (depth and stencil (apart|in one buffer)|job arenas for |two job arena sets"
    r"|clear program built|job arenas high-water at frame |binner tile pool high-water at frame \d+: not read)")
SUMMARY = re.compile(r"^replay: summary frames=(\d+) failed=(\d+) glerrors=(\d+) refused=(\d+) unresolved=(\d+)$")


def normalise(line):
    line = re.sub(r"\b(texture|framebuffer|renderbuffer|buffer|program|shader) \d+", r"\1 <n>", line)
    return re.sub(r"\b0x[0-9a-f]{5,}\b", "0x<addr>", line)


def read(path):
    found = collections.defaultdict(collections.Counter)
    summary = None
    for line in open(path, errors="replace").read().splitlines():
        m = SUMMARY.match(line)
        if m:
            summary = tuple(int(x) for x in m.groups())
            continue
        for name, rx in CLASSES:
            if rx.search(line):
                found[name][normalise(line)] += 1
                break
        else:
            if not NOISE.search(line):
                found["other"][normalise(line)] += 1
    return found, summary


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    out, gpus = argv[1], argv[2:]
    bad = False
    for g in gpus:
        path = os.path.join(out, g + ".log")
        found, summary = read(path)
        print("## " + g)
        if summary is None:
            print("- the run did not finish (no summary line); see %s" % path)
            bad = True
        else:
            print("- frames %d, failed %d, frames ending with a GL error %d, draws refused %d, "
                  "unresolved names %d" % summary)
            bad = bad or any(summary[1:4])
        for name in ("GL error", "refused draw", "incomplete texture", "GPU memory", "frame error", "panic",
                     "dropped", "other", "tiled copy", "sampler variant", "no such uniform", "unresolved GL name"):
            rows = found.get(name)
            if not rows:
                print("- %s: none" % name)
                continue
            bad = bad or name not in INFO
            print("- %s%s: %d lines, %d distinct" % (name, " (informational)" if name in INFO else "",
                                                    sum(rows.values()), len(rows)))
            for text, n in rows.most_common(5 if name in INFO else 20):
                print("  - %dx `%s`" % (n, text))
        print()
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
