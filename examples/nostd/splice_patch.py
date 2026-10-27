#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Regenerate the sections of changed files in one of this directory's patches.

    splice_patch.py <patch> <base-dir> <tree-dir> <file> [<file> ...]

<base-dir> is the unpatched source (upstream Slint at the README's commit,
or the crates.io femtovg/glow), <tree-dir> the patched tree, and each <file>
a path relative to both. Every section of the patch that names one of the
files is replaced by a fresh `diff -uN` of <base-dir>/<file> against
<tree-dir>/<file> (labelled a/<file> and b/<file>); every other section, and
the order of the sections, is kept byte for byte. A full `diff -ruN`
would reorder slint-nostd.patch (its sections are not in diff's order) and
rewrite every timestamp of the femtovg patch, so a one-file change would
show as a rewrite of the whole file.

Both formats of this directory are read: sections that start at a
"diff -ruN ..." line (femtovg, glow; the line is kept) and sections that
start at "--- a/<file>" (slint). A file with no section yet is refused: add
it by hand or regenerate the whole patch. The result goes to <patch>.new;
check it by applying it to a fresh <base-dir> copy and comparing with
<tree-dir> (README.md), then move it over <patch>.
"""
import subprocess
import sys


def sections_of(text):
    lines = text.splitlines(keepends=True)
    ruN = bool(lines) and lines[0].startswith("diff -ruN ")
    if ruN:
        starts = [i for i, l in enumerate(lines) if l.startswith("diff -ruN ")]
    else:
        starts = [i for i, l in enumerate(lines)
                  if l.startswith("--- a/") and i + 1 < len(lines) and lines[i + 1].startswith("+++ b/")]
    if not starts or starts[0] != 0:
        sys.exit("splice_patch.py: text before the first section, or no section")
    out = []
    for k, s in enumerate(starts):
        e = starts[k + 1] if k + 1 < len(starts) else len(lines)
        plus = next(l for l in lines[s:e] if l.startswith("+++ b/"))
        name = plus[len("+++ b/"):].split("\t")[0].rstrip("\n")
        header = lines[s] if ruN else ""
        out.append((name, header, "".join(lines[s:e])))
    if "".join(sec for _, _, sec in out) != text:
        sys.exit("splice_patch.py: the sections do not rebuild the patch")
    return out


def main():
    if len(sys.argv) < 5:
        sys.exit(__doc__)
    patch, base, tree, files = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:]
    sections = sections_of(open(patch).read())
    missing = set(files) - {name for name, _, _ in sections}
    if missing:
        sys.exit("splice_patch.py: no section for " + ", ".join(sorted(missing)))
    new = []
    for name, header, sec in sections:
        if name not in files:
            new.append(sec)
            continue
        r = subprocess.run(["diff", "-uN", "--label", "a/" + name, "--label", "b/" + name,
                            base + "/" + name, tree + "/" + name],
                           capture_output=True, text=True, env={"LC_ALL": "C"})
        if r.returncode != 1:
            sys.exit("splice_patch.py: diff of %s returned %d (%s)" % (name, r.returncode, r.stderr.strip()))
        new.append(header + r.stdout)
    open(patch + ".new", "w").write("".join(new))
    print("splice_patch.py: %d sections, %d regenerated (%s) -> %s.new"
          % (len(sections), len(files), ", ".join(files), patch))


if __name__ == "__main__":
    main()
