#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check the one-commit Pi 4 dEQP-GLES3 session record.

Usage:
  tools/cts/check-pi4-gles3-log.py --commit <full sha> \
      [--tree tools/cts/pi4-gles3-tree.txt] [session-dir ...]
  tools/cts/check-pi4-gles3-log.py --self-test

The session directories (default: the glob
tools/cts/log/*-pi4-gles3-<sha8>-*) are what the board sessions write,
tools/cts/log/<YYYY-MM>-pi4-gles3-<sha8>-rowsAA-BB/. Exit 1 when:
  a) an image label of the tree ('# image <LABEL>:' lines) has no
     <LABEL>.tally.txt in any directory (missing);
  b) a label has tallies in more than one directory (duplicated);
  c) a README.md has no line starting 'commit: <sha>' (or 'commit <sha>'),
     or names another commit;
  d) a tally has no complete deqp summary line (complete=1 and
     'summary: N cases begun, N with a result, 0 without'): aborted, unless
     it is a cut (below);
  e) a file holds an absolute home path (/home/, /Users/), an IPv4
     address, a .ts.net name or a /dev/tty path.
A cut is not a FAIL (an 'info: cut' line): an aborted image whose
console and tally show (a) that every case of the label in the CURRENT tree
began and the last case begun is the abort trigger (a ResourceError,
InternalError, Crash or Timeout line), and (b) that each case the old
console's filter never began is covered by another label of the tree that has
its own tally. The cases come from the tree globs expanded against
tools/cts/gles3-mustpass.txt (--mustpass). Anything else stays FAIL.
On success prints '<N> images, <N> with a tally, 0 missing, 0 duplicated,
commit <sha8>'.
"""
import argparse
import contextlib
import glob
import io
import os
import re
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
# gles3-notpass.py --pi4 wants 'commit: <sha>'; 'commit <sha>' is accepted too.
# Only line starts count, so prose such as 'fixed in commit <sha>' does not.
COMMIT_LINE = re.compile(r"^commit:?[ \t]+([0-9a-f]{7,40})\b", re.M | re.I)
REDACT = re.compile(
    r"/home/|/Users/|(?<![\w.-])(?:\d{1,3}\.){3}\d{1,3}(?![\w.])|\.ts\.net|/dev/tty")
SUMMARY = re.compile(
    r"deqp: summary: (\d+) cases begun, (\d+) with a result, (\d+) without")


def labels_of(tree, counts=None):
    """Image labels of the tree; counts (a dict), if given, gets the case
    count of each label from its '# image <LABEL>: <N>' header."""
    out = []
    with open(tree, encoding="utf-8") as f:
        for line in f:
            m = re.match(r"#\s*image\s+([^\s:]+)\s*:\s*(\d+)?", line)
            if m and m.group(1) not in out:
                out.append(m.group(1))
                if counts is not None and m.group(2):
                    counts[m.group(1)] = int(m.group(2))
    return out


def complete(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        text = f.read()
    if "complete=1" not in text:
        return False
    m = SUMMARY.search(text)
    return bool(m) and m.group(3) == "0" and m.group(1) == m.group(2)


def refused_one_case(path):
    """An image that ended with its own end line (a summary of 1 case
    begun, 1 with a result, 0 without) but complete=0: a one-case image
    (the tree header says 1 case) whose only case refused. No case of it is left without a result."""
    with open(path, encoding="utf-8", errors="replace") as f:
        text = f.read()
    m = SUMMARY.search(text)
    return bool(m) and (m.group(1), m.group(2), m.group(3)) == ("1", "1", "0")


ABORT = ("InternalError", "ResourceError", "Crash", "Timeout")
CASE_LINE = re.compile(r"^Test case '(\S+)'\.\.")
V3D_CASE = re.compile(r"^v3d_deqp: case #\d+ \S .*?(\S+)\s*$")
STATUS_LINE = re.compile(
    r"^(?:deqp: )?(Pass|Fail|NotSupported|QualityWarning|CompatibilityWarning|"
    r"InternalError|ResourceError|Crash|Timeout) (dEQP-\S+)")
FILTER_LINE = re.compile(r"^deqp: case filter '([^']*)'")


def pattern_re(pat):
    return re.compile(".*".join(re.escape(p) for p in pat.split("*")), re.S)


def expand(filt, mustpass):
    """Case names of a comma separated filter: '*' globs expand against the
    mustpass list (in its order), other names stand for themselves."""
    out, seen = [], set()
    for ent in filt.split(","):
        ent = ent.strip()
        if not ent:
            continue
        if "*" in ent:
            r = pattern_re(ent)
            names = [n for n in mustpass if r.fullmatch(n)]
        else:
            names = [ent]
        for n in names:
            if n not in seen:
                seen.add(n)
                out.append(n)
    return out


def tree_cases(tree):
    """{label: filter text}: the case lines under each '# image' header."""
    out, cur = {}, None
    with open(tree, encoding="utf-8") as f:
        for line in f:
            m = re.match(r"#\s*image\s+([^\s:]+)\s*:", line)
            if m:
                cur = m.group(1)
                out.setdefault(cur, [])
            elif cur and line.strip() and not line.lstrip().startswith("#"):
                out[cur].append(line.strip())
    return {k: ",".join(v) for k, v in out.items()}


def read_mustpass(path):
    with open(path, encoding="utf-8") as f:
        return [l.strip() for l in f if l.strip() and not l.startswith("#")]


def cut_verdict(lab, tallies, mustpass, expansions, where):
    """(ok, text) for an aborted label: 'cut' when conditions (a) and (b) of
    the module doc hold, else the reason it is not one."""
    mine = expansions.get(lab)
    if not mine:
        return False, "no case list for it in the tree"
    tally = tallies[0]
    con = tally[: -len(".tally.txt")] + ".console.txt"
    begun, status, last = [], {}, None
    flt = None
    texts = []
    for p in (con, tally):
        if os.path.isfile(p):
            with open(p, encoding="utf-8", errors="replace") as f:
                texts.append(f.read())
    if not texts:
        return False, "no console or tally"
    for text in texts:
        for line in text.splitlines():
            line = line.rstrip("\r")
            m = CASE_LINE.match(line) or V3D_CASE.match(line)
            if m and m.group(1) not in begun:
                begun.append(m.group(1))
            m = STATUS_LINE.match(line)
            if m:
                if m.group(2) not in begun:
                    begun.append(m.group(2))
                status[m.group(2)] = m.group(1)
            m = FILTER_LINE.match(line)
            if m and flt is None:
                flt = m.group(1)
    nb = set(begun)
    notbegun = [c for c in mine if c not in nb]
    if notbegun:
        return False, "%d case(s) of the tree label never began (first %s)" % (
            len(notbegun), notbegun[0])
    mine_set = set(mine)
    inside = [c for c in begun if c in mine_set]
    trigger = inside[-1] if inside else None
    if trigger is None or begun[-1] != trigger or status.get(trigger) not in ABORT:
        return False, "the last case begun is not an abort trigger"
    if trigger != mine[-1]:
        return False, "the abort trigger is not the last case of the tree label"
    if flt is None:
        return False, "no 'deqp: case filter' line in the console"
    old = expand(flt, mustpass)
    rest = [c for c in old if c not in nb]
    owner = {}
    for other, cs in expansions.items():
        if other != lab and where.get(other):
            for c in cs:
                owner.setdefault(c, other)
    unc = [c for c in rest if c not in owner]
    if unc:
        return False, "%d never-begun case(s) have no label with a tally (first %s)" % (
            len(unc), unc[0])
    subs = []
    for c in rest:
        if owner[c] not in subs:
            subs.append(owner[c])
    return True, "cut after abort trigger %s (%d begun, %d moved to %s)" % (
        trigger, len(begun), len(rest), " ".join(subs) if subs else "no label")


def check(sha, tree, dirs, mustpass=None):
    """Check the record; print the verdict, return the exit status."""
    dirs = [d for d in dirs if os.path.isdir(d)]
    problems = []
    if not dirs:
        problems.append("no session directory")

    counts = {}
    labels = labels_of(tree, counts)
    if not labels:
        problems.append("%s: no '# image <LABEL>:' lines" % tree)
    where = {}
    for d in dirs:
        for p in sorted(glob.glob(os.path.join(d, "*.tally.txt"))):
            lab = os.path.basename(p)[: -len(".tally.txt")]
            where.setdefault(lab, []).append(p)

        readme = os.path.join(d, "README.md")
        if not os.path.isfile(readme):
            problems.append("%s: no README.md (no commit line)" % d)
        else:
            with open(readme, encoding="utf-8", errors="replace") as f:
                shas = COMMIT_LINE.findall(f.read())
            if not shas:
                problems.append("%s: README.md has no 'commit: <sha>' line" % readme)
            for s in shas:
                s = s.lower()
                if not (sha.startswith(s) or s.startswith(sha)):
                    problems.append("%s: README.md names commit %s, not %s"
                                    % (readme, s, sha[:8]))

        for root, _, files in os.walk(d):
            for fn in sorted(files):
                p = os.path.join(root, fn)
                with open(p, "rb") as f:
                    data = f.read().decode("utf-8", errors="replace")
                for n, line in enumerate(data.splitlines(), 1):
                    m = REDACT.search(line)
                    if m:
                        problems.append("%s:%d: private detail %r" % (p, n, m.group(0)))
                        break

    missing, aborted, dup, info = [], [], [], []
    for lab in labels:
        ps = where.get(lab, [])
        if not ps:
            missing.append(lab)
            continue
        if len(ps) > 1:
            dup.append(lab)
            problems.append("%s: tallies in %d directories: %s" % (lab, len(ps), ", ".join(ps)))
        if any(complete(p) for p in ps):
            continue
        if counts.get(lab) == 1 and any(refused_one_case(p) for p in ps):
            info.append("%s: refused (one-case image)" % lab)
            continue
        aborted.append(lab)
    if aborted:
        try:
            mp = read_mustpass(mustpass or os.path.join(HERE, "gles3-mustpass.txt"))
        except OSError:
            mp = []
        filt = tree_cases(tree)
        exps = {k: expand(v, mp) for k, v in filt.items()}
        for lab in list(aborted):
            ok, why = cut_verdict(lab, where[lab], mp, exps, where)
            if ok:
                aborted.remove(lab)
                info.append("%s: %s" % (lab, why))
    if missing:
        problems.append("missing (%d): %s" % (len(missing), " ".join(missing)))
    if aborted:
        problems.append("aborted, no complete deqp summary (%d): %s"
                        % (len(aborted), " ".join(aborted)))
    extra = []
    for lab in sorted(set(where) - set(labels)):
        subs = [l for l in labels if l.startswith(lab) and l != lab]
        if subs:
            info.append("superseded by re-split: %s -> %s" % (lab, " ".join(subs)))
        else:
            extra.append(lab)
    for line in info:
        print("info:", line)
    if extra:
        problems.append("tallies not in the tree: %s" % " ".join(extra))

    have = len(labels) - len(missing)
    if problems:
        for p in problems:
            print("FAIL:", p)
        print("%d images, %d with a tally, %d missing, %d duplicated, %d aborted, commit %s"
              % (len(labels), have, len(missing), len(dup), len(aborted), sha[:8]))
        return 1
    print("%d images, %d with a tally, 0 missing, 0 duplicated, commit %s"
          % (len(labels), have, sha[:8]))
    return 0


def self_test():
    """A synthetic record: a complete one passes, a missing tally and a
    mixed commit fail."""
    good = "a" * 40
    other = "b" * 40
    tally = ("deqp: complete=1\n"
             "deqp: summary: 3 cases begun, 3 with a result, 0 without\n")
    fails = []
    cases = []

    def run(tree, dirs):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(good, tree, dirs)
        return rc, buf.getvalue()

    def expect(name, ok):
        cases.append(name)
        print("%s %s" % ("ok  " if ok else "FAIL", name))
        if not ok:
            fails.append(name)

    with tempfile.TemporaryDirectory() as td:
        tree = os.path.join(td, "tree.txt")
        with open(tree, "w", encoding="utf-8") as f:
            f.write("# image A1: dEQP-GLES3.a\n# image B2: dEQP-GLES3.b\n"
                    "# image C3: dEQP-GLES3.c\n")

        def session(name, commit, labels, readme=None, texts=None):
            d = os.path.join(td, name)
            os.makedirs(d)
            with open(os.path.join(d, "README.md"), "w", encoding="utf-8") as f:
                f.write(readme if readme is not None else "commit: %s\n" % commit)
            for lab in labels:
                with open(os.path.join(d, lab + ".tally.txt"), "w",
                          encoding="utf-8") as f:
                    f.write((texts or {}).get(lab, tally))
            return d

        d1 = session("s1-rows1-2", good, ["A1", "B2"])
        d2 = session("s1-rows3-3", good, ["C3"])
        rc, out = run(tree, [d1, d2])
        expect("complete record passes",
               rc == 0 and "3 images, 3 with a tally" in out)

        os.remove(os.path.join(d2, "C3.tally.txt"))
        rc, out = run(tree, [d1, d2])
        expect("missing tally fails and names the label",
               rc == 1 and "missing (1): C3" in out)

        d3 = session("s2-rows3-3", other, ["C3"])
        rc, out = run(tree, [d1, d3])
        expect("session with another commit fails",
               rc == 1 and "names commit " + other in out)

        # re-split / refused rules, on their own tree
        tree2 = os.path.join(td, "tree2.txt")
        with open(tree2, "w", encoding="utf-8") as f:
            f.write("# image S1a: 3\n# image S1b: 2\n# image R1: 1\n"
                    "# image M1: 7\n")
        refused = ("deqp: complete=0\n"
                   "deqp: summary: 1 cases begun, 1 with a result, 0 without\n")
        multi = ("deqp: complete=0\n"
                 "deqp: summary: 5 cases begun, 5 with a result, 0 without\n")

        def run2(labels, texts=None, tag="x"):
            d = session("t2-" + tag, good, labels, None, texts)
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = check(good, tree2, [d])
            return rc, buf.getvalue()

        base = ["S1a", "S1b", "R1", "M1"]
        rc, out = run2(base + ["S1"], {"R1": refused}, "a")
        expect("superseded tally ok, one-case refused ok",
               rc == 0 and "superseded by re-split: S1 -> S1a S1b" in out
               and "R1: refused (one-case image)" in out)
        rc, out = run2(base + ["Zzz"], {"R1": refused}, "b")
        expect("unrelated extra tally still fails",
               rc == 1 and "tallies not in the tree: Zzz" in out)
        rc, out = run2(base, {"M1": multi, "R1": refused}, "c")
        expect("multi-case complete=0 still aborted",
               rc == 1 and "aborted, no complete deqp summary (1): M1" in out)
        rc, out = run2(base, {"R1": multi.replace("complete=0", "complete=0")}, "d")
        expect("multi-case image without complete summary fails",
               rc == 1 and "R1" in out)

        rc, out = run2(base, {"M1": refused}, "e")
        expect("one case begun in a multi-case image is aborted, not refused",
               rc == 1 and "aborted, no complete deqp summary (1): M1" in out)

        # cut rules: label K keeps the cases up to the trigger, K2 the rest
        tree3 = os.path.join(td, "tree3.txt")
        mp3 = os.path.join(td, "mp3.txt")
        names = ["dEQP-X.k.c%d" % i for i in range(1, 7)]
        with open(mp3, "w", encoding="utf-8") as f:
            f.write("\n".join(names) + "\n")
        with open(tree3, "w", encoding="utf-8") as f:
            f.write("# image K: 3\n%s\n# image K2: 3\ndEQP-X.k.c4,dEQP-X.k.c5,"
                    "dEQP-X.k.c6\n" % ",".join(names[:3]))
        abort = ("deqp: complete=0\n"
                 "deqp: summary: 3 cases begun, 3 with a result, 0 without\n")
        con = ("deqp: case filter 'dEQP-X.k.*'\n"
               + "".join("Test case '%s'..\n  Pass (Pass)\n" % c for c in names[:2])
               + "Test case '%s'..\ndeqp: ResourceError %s\n" % (names[2], names[2]))

        def run3(labels, console, tag, con_text=con):
            d = session("t3-" + tag, good, labels, None,
                        {"K": abort, "K2": tally})
            if console:
                with open(os.path.join(d, "K.console.txt"), "w",
                          encoding="utf-8") as f:
                    f.write(con_text)
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = check(good, tree3, [d], mp3)
            return rc, buf.getvalue()

        rc, out = run3(["K", "K2"], True, "ok")
        expect("cut: abort at the last case, remainder label has a tally, ok",
               rc == 0 and "info: K: cut after abort trigger" in out
               and "moved to K2" in out)
        rc, out = run3(["K"], True, "notally")
        expect("cut: remainder label without a tally fails",
               rc == 1 and "missing (1): K2" in out)
        d = session("t3-nolabel", good, ["K", "K2"], None, {"K": abort, "K2": tally})
        with open(os.path.join(d, "K.console.txt"), "w", encoding="utf-8") as f:
            f.write(con)
        with open(tree3, "w", encoding="utf-8") as f:
            f.write("# image K: 3\n%s\n# image K2: 2\ndEQP-X.k.c4,dEQP-X.k.c5\n"
                    % ",".join(names[:3]))
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(good, tree3, [d], mp3)
        expect("cut: a never-begun case no label covers fails",
               rc == 1 and "aborted, no complete deqp summary (1): K" in buf.getvalue())
        with open(tree3, "w", encoding="utf-8") as f:
            f.write("# image K: 4\n%s\n# image K2: 2\ndEQP-X.k.c5,dEQP-X.k.c6\n"
                    % ",".join(names[:4]))
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(good, tree3, [d], mp3)
        expect("cut: a tree case that never began fails",
               rc == 1 and "aborted, no complete deqp summary (1): K" in buf.getvalue())
        rc, out = run3(["K", "K2"], False, "nocon")
        expect("cut: no console fails", rc == 1 and "(1): K" in out)

        def readme_case(name, tag, text, want_ok):
            d = session("r-" + tag, good, ["C3"], text)
            rc, out = run(tree, [d1, d])
            expect(name, (rc == 0) == want_ok)

        readme_case("'commit: <sha>' line passes", "colon",
                    "commit: %s\n" % good, True)
        readme_case("'commit <sha>' line passes", "plain",
                    "commit %s\n" % good, True)
        readme_case("prose 'see commit <sha>' only fails", "prose",
                    "see commit %s\n" % good, False)
        readme_case("prose sha of another commit beside a commit: line passes",
                    "both", "fixed in commit %s\ncommit: %s\n" % (other, good),
                    True)

    print("check-pi4-gles3-log self-test: %d of %d cases ok"
          % (len(cases) - len(fails), len(cases)))
    return 1 if fails else 0


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tree", default=os.path.join(HERE, "pi4-gles3-tree.txt"))
    ap.add_argument("--mustpass", default=os.path.join(HERE, "gles3-mustpass.txt"))
    ap.add_argument("--commit", help="full sha of the record")
    ap.add_argument("--self-test", action="store_true",
                    help="check the checker on a synthetic record")
    ap.add_argument("dirs", nargs="*", help="session directories")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.commit:
        ap.error("--commit is required")
    sha = a.commit.lower()
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        ap.error("--commit needs a full 40-digit sha")
    dirs = a.dirs or sorted(
        glob.glob(os.path.join(HERE, "log", "*-pi4-gles3-%s-*" % sha[:8])))
    return check(sha, a.tree, dirs, a.mustpass)


if __name__ == "__main__":
    sys.exit(main())
