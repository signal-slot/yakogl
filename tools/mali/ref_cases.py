#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Run the control cases through pan_shc and through Mesa's GL driver on
drm-shim, and check each program region against the result the case list
records.

    PAN_SHC=<pan_shc> tools/mali/ref_cases.py <drm_shim_ref work dir> <out dir> [<cases>]
    tools/mali/ref_cases.py --lint [<cases>]

<cases> defaults to tools/mali/testdata/cases.txt: one case per line,
`name vs fs prim region=result ...`, where result is `equal` or
`differs:<cause>:<n>:<sha256>` (README-pan_shc.md names each cause). `equal`
means the words are byte-equal AND the facts agree (work_reg_count, preload,
the FAU constants, secondary_offset). `differs` pins the words only: <n> is the
number of differing instruction words and <sha256> the first 12 hex digits of
pan_ref_diff's digest of those words on both sides, so a different difference
fails as a closed gap does; the facts of a `differs` region are not pinned. For
every case the script runs glsl2pan.py (pan_shc), drm_shim_ref.sh run (the
driver) and pan_ref_diff.py diff --draw, and reads each region's verdict from
pan_ref_diff's own lines. A case passes when the set of regions compared is
the set the line names and each verdict is the recorded one, so a gap that
closes (a `differs` that turns `equal`) fails too and the line is updated.
Prints one line per region and a total; exits 0 only when every case of the
list ran and passed.

--lint only parses the list (every case file exists, every result is well
formed) and needs neither a Mesa build nor pan_shc.
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
VERDICT = re.compile(
    r"^(\S+) vs (\S+): (?:byte-equal|(\d+) of \d+/\d+ instructions differ \(words sha256 ([0-9a-f]{12})\))")
FACTS = re.compile(r"^(\S+) facts: (equal|differ)")
RESULT = re.compile(r"^(?:equal|differs:[a-z0-9-]+:\d+:[0-9a-f]{12})$")
MISSING = re.compile(r"^(\S+): (?:pan_shc has no such region|the reference has no shader)")


def run_case(work, out, name, vs, fs, prim):
    base = os.path.join(out, name)
    tdir = os.path.join(HERE, "testdata")
    vs, fs = os.path.join(tdir, vs), os.path.join(tdir, fs)
    with open(base + ".json", "w") as so, open(base + ".disasm", "w") as se:
        r = subprocess.run([sys.executable, os.path.join(HERE, "glsl2pan.py"), vs, fs,
                            "--prim", prim], stdout=so, stderr=se)
    if r.returncode != 0:
        return None, "pan_shc failed (%s.disasm)" % base
    r = subprocess.run([os.path.join(HERE, "drm_shim_ref.sh"), "run", work, vs, fs, base,
                        "--prim", prim], capture_output=True, text=True)
    if r.returncode != 0:
        return None, "drm_shim_ref failed: %s" % r.stderr.strip().splitlines()[-1:]
    r = subprocess.run([sys.executable, os.path.join(HERE, "pan_ref_diff.py"), "diff",
                        base + ".json", base + ".disasm", base + ".trace", "--draw"],
                       capture_output=True, text=True)
    with open(base + ".diff", "w") as f:
        f.write(r.stdout + r.stderr)
    got = {}
    for line in r.stdout.splitlines():
        m = VERDICT.match(line)
        if m:
            got.setdefault(m.group(1), {}).update(
                words="equal" if m.group(3) is None else "differs", n=m.group(3), digest=m.group(4))
            continue
        m = FACTS.match(line)
        if m:
            got.setdefault(m.group(1), {})["facts"] = m.group(2)
            continue
        m = MISSING.match(line)
        if m:
            got[m.group(1)] = {"words": "missing"}
    return got, None


def verdict(g):
    """The result string of a region as the case list writes it, with the cause left out."""
    if g is None:
        return "(not compared)"
    if g["words"] == "equal":
        return "equal" if g.get("facts") == "equal" else "equal-words, facts %s" % g.get("facts", "absent")
    if g["words"] == "differs":
        return "differs:%s:%s" % (g["n"], g["digest"])
    return g["words"]


def matches(g, recorded):
    """recorded is `equal` or `differs:<cause>:<n>:<sha256>`."""
    if recorded.startswith("differs:"):
        return verdict(g) == "differs:" + recorded.split(":", 2)[2]
    return verdict(g) == recorded


def read_cases(path):
    """[(name, vs, fs, prim, {region: result})]; SystemExit on a line that is not well formed."""
    cases = []
    for lineno, line in enumerate(open(path), 1):
        fields = line.split("#", 1)[0].split()
        if not fields:
            continue
        if len(fields) < 5:
            raise SystemExit("%s:%d: name vs fs prim region=result ... expected" % (path, lineno))
        name, vs, fs, prim, *want = fields
        results = {}
        for w in want:
            region, _, result = w.partition("=")
            if not RESULT.match(result):
                raise SystemExit("%s:%d: %s: result %r is neither `equal` nor "
                                 "`differs:<cause>:<n>:<sha256>`" % (path, lineno, region, result))
            results[region] = result
        for f in (vs, fs):
            if not os.path.exists(os.path.join(HERE, "testdata", f)):
                raise SystemExit("%s:%d: testdata/%s does not exist" % (path, lineno, f))
        cases.append((name, vs, fs, prim, results))
    return cases


def main(argv):
    if argv[:1] == ["--lint"] and len(argv) <= 2:
        path = argv[1] if len(argv) == 2 else os.path.join(HERE, "testdata", "cases.txt")
        cases = read_cases(path)
        n = sum(len(c[4]) for c in cases)
        print("ref_cases --lint: %d case(s), %d region(s) (list %s)" % (len(cases), n, path))
        return 0 if cases and n else 1
    if len(argv) not in (2, 3):
        print(__doc__, file=sys.stderr)
        return 2
    work, out = argv[0], argv[1]
    cases_path = argv[2] if len(argv) == 3 else os.path.join(HERE, "testdata", "cases.txt")
    os.makedirs(out, exist_ok=True)
    cases = read_cases(cases_path)
    passed = regions = 0
    for name, vs, fs, prim, want in cases:
        got, err = run_case(work, out, name, vs, fs, prim)
        if err:
            print("FAIL %s: %s" % (name, err))
            continue
        ok = set(got) == set(want)
        for region in sorted(set(got) | set(want)):
            w = want.get(region, "(not in the list)")
            match = region in want and region in got and matches(got[region], w)
            ok &= match
            regions += 1
            print("%s %s %s: %s (recorded %s)"
                  % ("ok  " if match else "FAIL", name, region, verdict(got.get(region)), w))
        passed += ok
    print("ref_cases: %d of %d cases as recorded, %d region(s) (list %s)"
          % (passed, len(cases), regions, cases_path))
    return 0 if cases and passed == len(cases) else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
