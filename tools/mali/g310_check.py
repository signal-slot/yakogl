#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check the committed deqp_*.expect.json references on the G310v2 and G610.

    tools/mali/g310_check.sh [--since REV] [--build-dir DIR] [--scratch DIR] [REF...]

REF is a glob or a name (deqp_arith_x, deqp_arith_x.expect.json or a path);
default tools/mali/testdata/deqp_*.expect.json. --since REV keeps the
references whose files (.expect.json, .vert, .frag) were added or changed
since REV (git diff REV, plus untracked files).

For each reference: the pair <name>.vert/.frag is drawn on the Mesa shim with
--gpu g610 under the first option set whose `pan_ref_diff.py extract --draw`
(with --attributes where the committed file has attributes) equals the
committed file (the trace name aside); the same set is drawn with --gpu
g310v2 and the two extracts are compared with `pan_ref_diff.py diff-ref`.
One line per reference: IDENTICAL, DIFFERS <fields> or NO-OPTION-SET (no
pair, or no option set reproduces the committed reference: skipped).
Summary: "N identical, M differ, K skipped"; exit 1 if any differ.

The shim: <build dir>/inst, default $CONNECTED_DEPS_DIR/mesa-pan-shim, built
with drm_shim_ref.sh build from the tarball of third_party/mesa-26.2.1 when
absent. Needs CONNECTED_DEPS_DIR or --build-dir.
"""
import glob
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
DATA = os.path.join(HERE, "testdata")
# first match wins, the order of the sweep in docs/notes/mali-g310-vs-g610.md
OPTION_SETS = [
    ["--uniform-values"],
    [],
    ["--prim", "points", "--uniform-values"],
    ["--prim", "points"],
]
FIELDS = ("words", "fau", "work_reg_count", "preload", "va")
STATUSES = ("IDENTICAL", "DIFFERS", "NO-OPTION-SET")


def strip(ref):
    """A reference without the field that names its trace file."""
    return {k: v for k, v in ref.items() if k != "trace"}


def pick_option_set(extract_for, committed):
    """The first option set whose extract equals the committed reference.

    extract_for(opts) returns the extract (dict) or None when the draw or the
    extract failed. Returns the option list, or None when no set matches.
    """
    want = strip(committed)
    for opts in OPTION_SETS:
        got = extract_for(opts)
        if got is not None and strip(got) == want:
            return opts
    return None


def diff_fields(a, b):
    """Names of the fields in which two extracts differ ([] when identical)."""
    out = []
    ra, rb = a.get("regions", {}), b.get("regions", {})
    for name in sorted(set(ra) | set(rb)):
        if name not in ra or name not in rb:
            out.append("%s(region)" % name)
            continue
        for f in FIELDS:
            if ra[name].get(f) != rb[name].get(f):
                out.append("%s.%s" % (name, f))
    for key in sorted((set(a) | set(b)) - {"regions", "trace"}):
        if a.get(key) != b.get(key):
            out.append(key)
    return out


DIFFREF = re.compile(r"pan_ref_diff: (\d+) region\(s\) compared, (\d+) differ or missing")


def parse_diffref(text):
    """(regions compared, differing) from diff-ref's last summary line, or None."""
    m = None
    for m in DIFFREF.finditer(text):
        pass
    return (int(m.group(1)), int(m.group(2))) if m else None


SUMMARY = re.compile(r"^(\d+) identical, (\d+) differ, (\d+) skipped$", re.M)


def parse_summary(text):
    """(identical, differ, skipped) from a run's summary line, or None."""
    m = SUMMARY.search(text)
    return tuple(int(x) for x in m.groups()) if m else None


def format_summary(results):
    """results: {name: (status, detail)} -> the summary line."""
    def n(s):
        return sum(1 for v in results.values() if v[0] == s)
    return "%d identical, %d differ, %d skipped" % (n("IDENTICAL"), n("DIFFERS"), n("NO-OPTION-SET"))


def format_line(name, status, detail):
    return "%s %s" % (status, name) + ("" if status == "IDENTICAL" or not detail else " " + detail)


def select(args, since=None, changed=()):
    """Reference paths for the command line: globs or names, then --since."""
    found = set()
    for p in args or [os.path.join(DATA, "deqp_*.expect.json")]:
        for c in (p, os.path.join(DATA, p), os.path.join(DATA, p + ".expect.json")):
            found.update(glob.glob(c))
    found = {f for f in found if f.endswith(".expect.json")}
    if since is not None:
        keep = {os.path.abspath(os.path.join(ROOT, c)) for c in changed}
        found = {f for f in found if os.path.abspath(f) in keep}
    return sorted(found)


def git_changed(rev):
    def git(*a):
        return subprocess.run(["git", "-C", ROOT] + list(a), check=True,
                              capture_output=True, text=True).stdout.split("\n")
    names = git("diff", "--name-only", "--diff-filter=AMR", rev, "--", "tools/mali/testdata")
    names += git("ls-files", "--others", "--exclude-standard", "tools/mali/testdata")
    # a changed shader changes its reference's draw, so .vert/.frag count too
    return sorted({re.sub(r"\.(expect\.json|vert|frag)$", ".expect.json", n)
                   for n in names if n.endswith((".expect.json", ".vert", ".frag"))})


def shim_work(build_dir):
    if build_dir:
        return os.path.abspath(build_dir)
    deps = os.environ.get("CONNECTED_DEPS_DIR")
    if not deps:
        sys.exit("g310_check: set CONNECTED_DEPS_DIR or pass --build-dir")
    return os.path.join(deps, "mesa-pan-shim")


def have_shim(work):
    inst = os.path.join(work, "inst")
    dirs = [os.path.join(inst, "lib64")]
    if os.path.isdir(os.path.join(inst, "lib")):
        dirs += [os.path.join(inst, "lib", x) for x in os.listdir(os.path.join(inst, "lib"))]
    return any(os.path.isfile(os.path.join(d, "libpanfrost_noop_drm_shim.so")) for d in dirs)


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def tarball(work):
    upstream = open(os.path.join(ROOT, "third_party/mesa-26.2.1/UPSTREAM")).read()
    sha = re.search(r"^tarball-sha256: *(\S+)", upstream, re.M).group(1)
    url = re.search(r"^tarball: *(\S+)", upstream, re.M).group(1)
    deps = os.environ.get("CONNECTED_DEPS_DIR", "")
    for c in (os.path.join(deps, "mesa-tarball", "mesa-26.2.1.tar.xz"),
              os.path.join(deps, "downloads", "mesa-26.2.1.tar.xz")):
        if deps and os.path.isfile(c) and sha256(c) == sha:
            return c
    dst = os.path.join(work, "mesa-26.2.1.tar.xz")
    os.makedirs(work, exist_ok=True)
    urllib.request.urlretrieve(url, dst + ".part")
    if sha256(dst + ".part") != sha:
        sys.exit("g310_check: tarball sha256 differs from UPSTREAM")
    os.rename(dst + ".part", dst)
    return dst


def ensure_shim(work):
    if not have_shim(work):
        print("g310_check: building the shim in %s (tens of minutes)" % work, file=sys.stderr)
        subprocess.run(["bash", os.path.join(HERE, "drm_shim_ref.sh"), "build", tarball(work), work],
                       check=True, stdout=sys.stderr)
    return work


def pair_of(ref):
    base = ref[:-len(".expect.json")]
    v, f = base + ".vert", base + ".frag"
    return (v, f) if os.path.isfile(v) and os.path.isfile(f) else None


def draw(work, vs, fs, out, gpu, opts):
    """Draw once; the trace path or None."""
    r = subprocess.run(["bash", os.path.join(HERE, "drm_shim_ref.sh"), "run", work, vs, fs, out,
                        "--gpu", gpu] + opts, capture_output=True, text=True)
    return out + ".trace" if r.returncode == 0 else None


def extract(trace, attributes):
    cmd = [sys.executable, "-I", os.path.join(HERE, "pan_ref_diff.py"), "extract", trace, "--draw"]
    if attributes:
        cmd.append("--attributes")
    r = subprocess.run(cmd, capture_output=True, text=True)
    return json.loads(r.stdout) if r.returncode == 0 else None


def check_one(work, ref, tmp):
    """(status, detail) for one reference."""
    pair = pair_of(ref)
    if pair is None:
        return "NO-OPTION-SET", "no .vert/.frag pair"
    with open(ref) as f:
        committed = json.load(f)
    attrs = "attributes" in committed
    count = [0]

    def extract_for(opts, gpu="g610"):
        count[0] += 1
        t = draw(work, pair[0], pair[1], os.path.join(tmp, "d%d" % count[0]), gpu, opts)
        return extract(t, attrs) if t else None

    opts = pick_option_set(extract_for, committed)
    if opts is None:
        return "NO-OPTION-SET", "no option set reproduces the G610 reference"
    a = extract_for(opts)
    b = extract_for(opts, "g310v2")
    if a is None or b is None:
        return "DIFFERS", "g310v2 draw failed"
    fa, fb = os.path.join(tmp, "a.json"), os.path.join(tmp, "b.json")
    for path, ex in ((fa, a), (fb, b)):
        with open(path, "w") as f:
            json.dump(ex, f)
    r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "pan_ref_diff.py"), "diff-ref", fa, fb],
                       capture_output=True, text=True)
    parsed = parse_diffref(r.stdout)
    fields = diff_fields(a, b)
    if r.returncode == 0 and parsed and parsed[1] == 0 and not fields:
        return "IDENTICAL", " ".join(opts)
    return "DIFFERS", " ".join(fields) or "diff-ref"


def parse_args(argv):
    """(since, build_dir, keep, refs) or None for a usage error."""
    vals = {"--since": None, "--build-dir": None, "--scratch": None}
    args = []
    i = 0
    while i < len(argv):
        if argv[i] in vals:
            if i + 1 >= len(argv):
                return None
            vals[argv[i]] = argv[i + 1]
            i += 2
        else:
            args.append(argv[i])
            i += 1
    return vals["--since"], vals["--build-dir"], vals["--scratch"], args


def main(argv):
    if "-h" in argv or "--help" in argv:
        print(__doc__)
        return 0
    parsed = parse_args(argv)
    if parsed is None:
        print(__doc__, file=sys.stderr)
        return 2
    since, build_dir, keep, args = parsed
    refs = select(args, since, git_changed(since) if since else ())
    if not refs:
        print("g310_check: no reference selected")
        print(format_summary({}))
        return 0
    work = ensure_shim(shim_work(build_dir))
    results = {}
    for ref in refs:
        name = os.path.basename(ref)[:-len(".expect.json")]
        with tempfile.TemporaryDirectory(prefix="g310_check.", dir=keep) as tmp:
            try:
                results[name] = check_one(work, ref, tmp)
            except (OSError, ValueError, subprocess.SubprocessError) as e:
                results[name] = ("NO-OPTION-SET", "error: %s" % e)
        print(format_line(name, *results[name]), flush=True)
    print(format_summary(results))
    return 1 if any(v[0] == "DIFFERS" for v in results.values()) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
