#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""One i.MX8MP glmark2 record directory to a ready row of the by-eye note.

    tools/cts/imx8mp-glmark2-judge.py <record dir> <llvmpipe reference dir>
        [--scene TEXT] [--out DIR] [--repo DIR] [--main REF]
    tools/cts/imx8mp-glmark2-judge.py --self-test

<record dir> is records/imx8mp/<YYYYMMDD-HHMM>-<name>-<sha8>/ (tools/board.sh
run through tools/cts/imx8mp-glmark2-dumps.sh); <reference dir> is the output
of tools/cts/glmark2-host-ref.sh (<slug>.png per scene). The scene text
(`[build] use-vbo=false`) is looked up from <name> (gm01-build, ...) in
`imx8mp-glmark2-dumps.sh scenes`, or given with --scene.

What it does (it never decides a verdict, that is for a person looking):

  1. decodes the record's console-live.txt (console.txt without it) with
     glmark2-fb.py into <out>/<name>.judge.png;
  2. compares that picture with <reference>/<slug>.png with glmark2-picdiff.py
     (<out>/<name>.picdiff.png) and writes <out>/<name>.sidebyside.png with
     glmark2-sidebyside.py;
  3. finds the record's commit (first meta.txt's `commit=<40-hex>` line, then
     a `commit` file; the name's sha8 is not looked at then) and prints whether it is an ancestor of --main
     (default origin/main): yes, no or unknown. A record whose sha8 is its
     commit: the full sha from INDEX.txt or another small text file of the
     record, else git rev-parse in --repo. A record as tools/board.sh names it
     (sha8 = the image's sha256, meta.txt image_sha256): the commit is the
     image's YAKOGL_SAMPLE_TREE=<sha8> (meta.txt image_build, INDEX.txt) or the
     console's `build tree=<sha8>` line; a -dirty build is named. Such a row's
     record sha8 is not its commit, which the verdicts checker's rule (c)
     refuses: the helper says so;
  4. prints the table row of docs/notes/imx8mp-glmark2-pictures-by-eye.md with
     the verdict `not yet looked at` (the placeholder; replace it by looking)
     and the opening of the scene's `- **[scene] args**` entry with the crc and
     the picdiff result filled in.

--out defaults to the record directory. Exit 0: decoded and compared; 1: the
dump is damaged (no row: rerun the board, never patch pixels); 2: bad input.
Every line it quotes is stripped of addresses, host names, UART paths and
serials. Stdlib only.
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
DUMPS = os.path.join(HERE, "imx8mp-glmark2-dumps.sh")
PLACEHOLDER = "not yet looked at"
RECDIR = re.compile(r"^([0-9]{8}-[0-9]{4})-(.+)-([0-9a-f]{8})$")

# text that must never leave the tool: IPv4, host:port, MAC, user@host, device
# paths, serial numbers, host names with a domain suffix
_LEAK = [
    re.compile(r"\b\d{1,3}(?:\.\d{1,3}){3}(?::\d+)?\b"),
    re.compile(r"\b(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}\b"),
    re.compile(r"\b[\w.-]+@[\w.-]+\b"),
    re.compile(r"/dev/\S+"),
    re.compile(r"\b[\w-]+(?:\.[\w-]+)*\.(?:local|lan|net|com|org|io|internal|home|"
               r"localdomain|example)(?::\d+)?\b", re.I),
    re.compile(r"\b(?:serial|ser|sn|uuid)\b\s*[:=#]?\s*[\w-]{4,}", re.I),
    re.compile(r"\b(?:tcp|telnet|rfc2217|socket|ssh)://\S+", re.I),
]


def scrub(text):
    """Strip addresses, host names, UART paths and serials from quoted text."""
    out = []
    for line in text.splitlines():
        for rx in _LEAK:
            line = rx.sub("<redacted>", line)
        # any other absolute path: its last component only
        line = re.sub(r"(?<![\w.<])/(?:[\w.+-]+/)+([\w.+-]+)", r"\1", line)
        out.append(line)
    return "\n".join(out)


def slug(info):
    """glmark2-host-ref.sh's slug rule."""
    return re.sub(r"^-+|-+$", "", re.sub(r"[^a-z0-9]+", "-", info.lower()))


def sh(cmd, cwd=None):
    r = subprocess.run(cmd, capture_output=True, text=True, cwd=cwd)
    return r.returncode, (r.stdout + r.stderr)


def scene_of_label(label):
    rc, out = sh(["bash", DUMPS, "scenes"])
    if rc:
        return None
    for line in out.splitlines():
        if line.startswith(label + " "):
            rest = line.split(" ", 1)[1]
            return rest.rpartition(" -b ")[0] or rest
    return None


def _small_texts(rec):
    """The record's small text files other than the console, INDEX.txt first."""
    names = ["INDEX.txt", "meta.txt"] + sorted(
        n for n in os.listdir(rec) if n not in ("INDEX.txt", "meta.txt") and
        not n.startswith("console") and n.endswith((".txt", ".meta", ".env", ".json")))
    for n in names:
        p = os.path.join(rec, n)
        if os.path.isfile(p) and os.path.getsize(p) <= 1 << 20:
            with open(p, errors="replace") as f:
                yield f.read()


def _named_commit(rec):
    """The 40-hex commit the record states: meta.txt `commit=`, else a `commit` file."""
    for n, rx in (("meta.txt", r"^commit=([0-9a-f]{40})\s*$"),
                  ("commit", r"^\s*([0-9a-f]{40})\s*$")):
        p = os.path.join(rec, n)
        if os.path.isfile(p) and os.path.getsize(p) <= 1 << 20:
            with open(p, errors="replace") as f:
                m = re.search(rx, f.read(), re.M)
            if m:
                return m.group(1)
    return None


def record_commit(rec, sha8, log, repo):
    """(commit8 or None, full sha or None, dirty, named_by_image).

    tools/board.sh names a record by the first 8 digits of the image's sha256
    (meta.txt image_sha256); the commit of such a record is the image's build
    id, YAKOGL_SAMPLE_TREE=<sha8>[-dirty] (meta.txt image_build, INDEX.txt) or
    the console's `build tree=<sha8>` line. A record whose sha8 is not an
    image sha256 is taken as named by its commit."""
    texts = list(_small_texts(rec))
    by_image = any(re.search(r"^image_sha256=%s[0-9a-f]{56}\s*$" % sha8, t, re.M)
                   for t in texts)
    commit, dirty = None, False
    named = _named_commit(rec)
    if named:
        return named[:8], named, False, by_image
    if by_image:
        with open(log, errors="replace") as f:
            con = f.read(1 << 20)
        for t, rx in [(t, r"YAKOGL_SAMPLE_TREE=([0-9a-f]{7,40})(-dirty)?") for t in texts] + [
                (con, r": build tree=([0-9a-f]{7,40})(-dirty)?")]:
            m = re.search(rx, t)
            if m:
                commit, dirty = m.group(1), bool(m.group(2))
                break
        if not commit:
            return None, None, False, True
    else:
        commit = sha8
        pat = re.compile(r"\b(%s[0-9a-f]{32})\b" % sha8)
        for t in texts:
            m = pat.search(t)
            if m:
                return sha8, m.group(1), False, False
    rc, out = sh(["git", "-C", repo, "rev-parse", "--verify", "--quiet",
                  commit + "^{commit}"])
    full = out.strip() if rc == 0 and re.fullmatch(r"[0-9a-f]{40}", out.strip()) else None
    return (full or commit)[:8], full, dirty, by_image


def ancestor(sha, repo, main):
    """yes / no / unknown"""
    if not sha:
        return "unknown (no full commit found)"
    rc, _ = sh(["git", "-C", repo, "rev-parse", "--verify", "--quiet", main])
    if rc:
        return "unknown (%s not found in the repository)" % main
    rc, _ = sh(["git", "-C", repo, "cat-file", "-e", sha + "^{commit}"])
    if rc:
        return "unknown (commit not in the repository)"
    rc, _ = sh(["git", "-C", repo, "merge-base", "--is-ancestor", sha, main])
    return "yes" if rc == 0 else "no"


def judge(rec, ref, scene, out, repo, main):
    rec = rec.rstrip("/")
    dname = os.path.basename(rec)
    m = RECDIR.match(dname)
    if not m or not os.path.isdir(rec) or not os.path.isdir(ref):
        print("judge: the record directory must be <YYYYMMDD-HHMM>-<name>-<sha8> "
              "and both directories must exist", file=sys.stderr)
        return 2
    _, label, sha8 = m.groups()
    scene = scene or scene_of_label(label)
    if not scene:
        print("judge: no scene for '%s' (pass --scene TEXT)" % label, file=sys.stderr)
        return 2
    log = next((os.path.join(rec, n) for n in ("console-live.txt", "console.txt")
                if os.path.isfile(os.path.join(rec, n))), None)
    refpng = os.path.join(ref, slug(scene) + ".png")
    if not log or not os.path.isfile(refpng):
        print("judge: no console log in the record, or no %s.png in the reference"
              % slug(scene), file=sys.stderr)
        return 2
    out = out or rec
    os.makedirs(out, exist_ok=True)
    png = os.path.join(out, label + ".judge.png")
    name = re.match(r"\[([^\]]+)\]", scene)
    cmd = [sys.executable, "-I", os.path.join(HERE, "glmark2-fb.py"), log, png]
    if name:
        cmd += ["--scene", name.group(1)]
    rc, dec = sh(cmd)
    print("== decode (glmark2-fb.py)")
    print(scrub(dec).rstrip())
    if rc == 2:
        return 2
    if rc:
        if os.path.exists(png):
            os.remove(png)  # no picture of a damaged dump is left to be judged
        print("== damaged dump: rerun the board, never patch pixels; no row")
        return 1
    crc = re.search(r"frame crc32 ([0-9a-f]{8}) agrees", dec)
    value = "picture value agrees" in dec
    size = re.search(r"(\d+x\d+), \d+ of \d+ rows good", dec)
    diffpng = os.path.join(out, label + ".picdiff.png")
    rc, pd = sh([sys.executable, "-I", os.path.join(HERE, "glmark2-picdiff.py"),
                 png, refpng, diffpng])
    print("== picdiff against %s.png (glmark2-picdiff.py)" % slug(scene))
    print(scrub(pd).rstrip())
    if rc:
        return 2
    sbs = os.path.join(out, label + ".sidebyside.png")
    rc, sb = sh([sys.executable, "-I", os.path.join(HERE, "glmark2-sidebyside.py"),
                 png, refpng, sbs, "--label-board", "i.MX8MP " + label,
                 "--label-ref", "llvmpipe"])
    print("== side by side: %s (%s)" % (os.path.basename(sbs), "written" if rc == 0
          else scrub(sb).strip()))
    commit, full, dirty, by_image = record_commit(rec, sha8, log, repo)
    anc = ancestor(full, repo, main)
    print("== commit %s%s%s: ancestor of %s: %s" % (
        commit or "unknown", " (" + full + ")" if full else "",
        ", built from a dirty tree" if dirty else "", main, anc))
    if by_image:
        print("== note: the record's sha8 %s is the image's sha256 (tools/board.sh), "
              "not the commit%s" % (sha8, "" if commit else
                                    "; no YAKOGL_SAMPLE_TREE or build line gives the commit"))
        if commit and commit != sha8:
            print("== note: imx8mp-glmark2-verdicts.py rule (c) wants the record's sha8 "
                  "to be the commit: it refuses this row once the verdict is filled in")
    if dirty:
        print("== note: a dirty build is not an origin/main commit: rebuild and rerun")
    recpath = "records/imx8mp/%s/" % dname
    print("== row for docs/notes/imx8mp-glmark2-pictures-by-eye.md "
          "(replace the verdict by looking)")
    print("| %s | %s | %s | dump | %s |" % (scene, commit or "-", recpath, PLACEHOLDER))
    print("== opening of the entry")
    print("- **%s** (%s, %s): dump %s, frame crc32 %s%s; picdiff against llvmpipe: %s. "
          "Seen: <by eye>." % (
              scene, commit or "-", recpath, size.group(1) if size else "?",
              crc.group(1) if crc else "?", ", picture value agrees" if value else "",
              scrub(pd).strip().splitlines()[-1] if pd.strip() else "?"))
    return 0


def self_test():
    import shutil
    import tempfile
    here = os.path.join(HERE, "testdata", "imx8mp-glmark2-judge")
    tmp = tempfile.mkdtemp(prefix="judge-selftest.", dir=os.environ.get("TMPDIR"))
    bad = 0

    def expect(what, ok):
        nonlocal bad
        print("  %s  %s" % ("ok  " if ok else "FAIL", what))
        bad += not ok

    try:
        sha = "0123456789abcdef0123456789abcdef01234567"
        sha8 = sha[:8]
        # a repository with origin/main holding the commit
        repo = os.path.join(tmp, "repo")
        os.makedirs(repo)
        env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t",
                   GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
        for c in (["init", "-q"], ["commit", "-q", "--allow-empty", "-m", "a"]):
            subprocess.run(["git", "-C", repo] + c, env=env, capture_output=True)
        head = subprocess.run(["git", "-C", repo, "rev-parse", "HEAD"],
                              capture_output=True, text=True).stdout.strip()
        subprocess.run(["git", "-C", repo, "update-ref", "refs/remotes/origin/main",
                        head])
        subprocess.run(["git", "-C", repo, "commit", "-q", "--allow-empty", "-m", "b"],
                       env=env, capture_output=True)
        side = subprocess.run(["git", "-C", repo, "rev-parse", "HEAD"],
                              capture_output=True, text=True).stdout.strip()
        dname = "20261120-1530-gm01-build-" + head[:8]
        rec = os.path.join(tmp, "records", "imx8mp", dname)
        shutil.copytree(os.path.join(here, "record"), rec)
        with open(os.path.join(rec, "INDEX.txt"), "a") as f:
            f.write("image build: zephyr " + head + "\n")
        ref = os.path.join(here, "reference")
        scene = "[build] use-vbo=false"

        def run(*a):
            r = subprocess.run([sys.executable, "-I", os.path.abspath(__file__)] + list(a),
                               capture_output=True, text=True)
            return r.returncode, r.stdout + r.stderr

        out1 = os.path.join(tmp, "out")
        rc, o = run(rec, ref, "--repo", repo, "--out", out1)
        expect("clean record: exit 0", rc == 0)
        row = next((x for x in o.splitlines() if x.startswith("| [")), "")
        want = "| %s | %s | records/imx8mp/%s/ | dump | %s |" % (
            scene, head[:8], dname, PLACEHOLDER)
        expect("row has the exact format", row == want)
        expect("scene looked up from the label (gm01-build)", scene in row)
        expect("ancestor of origin/main: yes", "ancestor of origin/main: yes" in o)
        expect("opening of the entry with crc and picdiff",
              any(x.startswith("- **%s**" % scene) and "frame crc32 " in x and
                  "mean |diff|" in x for x in o.splitlines()))
        for n in ("gm01-build.judge.png", "gm01-build.picdiff.png",
                  "gm01-build.sidebyside.png"):
            expect("wrote " + n, os.path.getsize(os.path.join(out1, n)) > 0)
        # no leak: the log has an address, a UART path and a serial
        leaks = ("192.0.2.", "ttyUSB", "bridge.example", "SN12345678", tmp)
        expect("no address, UART path, serial or host in the output",
               not any(s in o for s in leaks))
        expect("redaction is exercised (log carries them)",
               all(s in open(os.path.join(rec, "console-live.txt")).read()
                   for s in leaks[:4]))
        # the verdicts parser accepts the row, with and without a verdict
        vp = os.path.join(HERE, "imx8mp-glmark2-verdicts.py")
        allscenes = [scene_of_label(l.split()[0]) for l in
                     sh(["bash", DUMPS, "scenes"])[1].splitlines() if l.strip()]
        for verdict in (PLACEHOLDER, "right"):
            note = os.path.join(tmp, "note.md")
            with open(note, "w") as f:
                f.write("## Verdicts\n\n| scene | commit | record | route | verdict |\n"
                        "|---|---|---|---|---|\n")
                f.write(row.replace(PLACEHOLDER, verdict) + "\n")
                for s in allscenes[1:]:
                    f.write("| %s | - | - | - | %s |\n" % (s, PLACEHOLDER))
            r = subprocess.run([sys.executable, "-I", vp, note],
                               capture_output=True, text=True)
            filled = 0 if verdict == PLACEHOLDER else 1
            expect("verdicts.py parser accepts the row with '%s' (%s)" % (
                verdict, r.stdout.strip()),
                   r.returncode == 0 and "ok: 33 rows, %d filled" % filled in r.stdout)
        # a commit that is not on origin/main
        rec2 = os.path.join(tmp, "records", "imx8mp", "20261120-1531-gm01-build-" + side[:8])
        shutil.copytree(os.path.join(here, "record"), rec2)
        with open(os.path.join(rec2, "INDEX.txt"), "a") as f:
            f.write("image build: zephyr " + side + "\n")
        rc, o = run(rec2, ref, "--repo", repo, "--out", os.path.join(tmp, "out2"))
        expect("commit past origin/main: ancestor no", "ancestor of origin/main: no" in o)
        # all 33 rows in the helper's format, filled: --complete accepts them
        note = os.path.join(tmp, "note-complete.md")
        with open(note, "w") as f:
            f.write("## Verdicts\n\n| scene | commit | record | route | verdict |\n"
                    "|---|---|---|---|---|\n")
            for s in allscenes:
                f.write(row.replace(scene, s).replace(PLACEHOLDER, "right") + "\n")
        r = subprocess.run([sys.executable, "-I", vp, note, "--complete"],
                           capture_output=True, text=True)
        expect("verdicts.py --complete accepts 33 rows of the helper's format (%s)"
               % r.stdout.strip(),
               r.returncode == 0 and "ok: 33 rows, 33 filled" in r.stdout)
        # a record as tools/board.sh names it: sha8 = the image's sha256, the
        # commit in meta.txt's image_build (and never in the directory name)
        img = "feedc0de" + "5" * 56
        for k, (tree, where) in enumerate(((head[:8], "meta"), (head[:8] + "-dirty", "meta"),
                                           (head[:8], "console"), (None, None))):
            recb = os.path.join(tmp, "records", "imx8mp",
                                "20261120-154%d-gm01-build-feedc0de" % k)
            shutil.copytree(os.path.join(here, "record"), recb)
            with open(os.path.join(recb, "meta.txt"), "w") as f:
                f.write("board=imx8mp\nimage_sha256=%s\n" % img)
                if where == "meta":
                    f.write("image_build=YAKOGL_SAMPLE_TABLE=6c5b8a70 "
                            "YAKOGL_SAMPLE_TREE=%s\n" % tree)
            if where == "console":
                with open(os.path.join(recb, "console-live.txt")) as f:
                    c = f.read()
                with open(os.path.join(recb, "console-live.txt"), "w") as f:
                    f.write("sample: build tree=%s table=6c5b8a70\n%s" % (tree, c))
            rc, o = run(recb, ref, "--repo", repo, "--out", os.path.join(tmp, "outb%d" % k))
            rowb = next((x for x in o.splitlines() if x.startswith("| [")), "")
            what = "image-named record, commit %s in %s" % (tree, where)
            if tree is None:
                expect(what + ": commit cell '-', ancestor unknown",
                       rc == 0 and "| - | records/imx8mp/" in rowb and
                       "ancestor of origin/main: unknown" in o)
                continue
            expect(what + ": the row's commit is the build's, not the image sha8",
                   rc == 0 and rowb.startswith("| %s | %s | " % (scene, head[:8])) and
                   "feedc0de/ | dump |" in rowb and
                   "ancestor of origin/main: yes" in o and "is the image's sha256" in o and
                   "rule (c)" in o)
            expect(what + ": dirty %s" % ("named" if "dirty" in tree else "not named"),
                   ("dirty" in tree) == ("built from a dirty tree" in o))
        # meta.txt commit= wins over the name's sha8 (image sha256)
        for k, where in enumerate(("meta.txt", "commit")):
            recc = os.path.join(tmp, "records", "imx8mp",
                                "20261120-155%d-gm01-build-feedc0de" % k)
            shutil.copytree(os.path.join(here, "record"), recc)
            with open(os.path.join(recc, "meta.txt"), "w") as f:
                f.write("board=imx8mp\nimage_sha256=%s\n" % img)
                if where == "meta.txt":
                    f.write("commit=%s\n" % head)
            if where == "commit":
                with open(os.path.join(recc, "commit"), "w") as f:
                    f.write(head + "\n")
            rc, o = run(recc, ref, "--repo", repo, "--main", "origin/main",
                        "--out", os.path.join(tmp, "outc%d" % k))
            rowc = next((x for x in o.splitlines() if x.startswith("| [")), "")
            expect("sha8 differs from %s commit: commit cell from it, path keeps the name"
                   % where, rc == 0 and rowc == "| %s | %s | records/imx8mp/%s/ | dump | %s |" % (
                       scene, head[:8], os.path.basename(recc), PLACEHOLDER) and
                   "ancestor of origin/main: yes" in o)
        # a damaged dump: exit 1, no row, a note to rerun
        rec3 = os.path.join(tmp, "records", "imx8mp", "20261120-1532-gm01-build-" + head[:8])
        shutil.copytree(os.path.join(here, "record"), rec3)
        p = os.path.join(rec3, "console-live.txt")
        with open(p) as f:
            lines = f.read().splitlines()
        i = next(k for k, x in enumerate(lines) if re.search(r"glmark2-fb: 3 ", x))
        del lines[i]
        with open(p, "w") as f:
            f.write("\n".join(lines) + "\n")
        rc, o = run(rec3, ref, "--repo", repo, "--out", os.path.join(tmp, "out3"))
        expect("damaged dump leaves no judge.png",
               not os.path.exists(os.path.join(tmp, "out3", "gm01-build.judge.png")))
        expect("damaged dump: exit 1, rerun, no row",
               rc == 1 and "rerun" in o and not any(x.startswith("| [") for x in o.splitlines()))
        # bad input
        rc, _ = run(os.path.join(tmp, "nodir"), ref)
        expect("bad record directory: exit 2", rc == 2)
        # scrub on its own
        ip = ".".join(("10", "1", "2", "3"))
        home = "/" + "home/"
        s = scrub("x " + ip + ":5000 /dev/ttyACM0 a@b.c host.lan serial: ABCD1234 " +
                  home + "u/r/f.png mean 0.03 ab12cd34")
        expect("scrub keeps numbers and hashes",
               "0.03" in s and "ab12cd34" in s and "f.png" in s and not any(
                   k in s for k in (ip, "ttyACM", "a@b", "host.lan", "ABCD1234",
                                    home)))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    if bad:
        print("== %d judge self-test case(s) FAILED" % bad, file=sys.stderr)
        return 1
    print("== imx8mp-glmark2-judge self-test: all cases passed")
    return 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    args, opt, pos = list(argv), {"--scene": None, "--out": None,
                                  "--repo": ROOT, "--main": "origin/main"}, []
    while args:
        a = args.pop(0)
        if a in opt and args:
            opt[a] = args.pop(0)
        elif a.startswith("-"):
            print(__doc__, file=sys.stderr)
            return 2
        else:
            pos.append(a)
    if len(pos) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    return judge(pos[0], pos[1], opt["--scene"], opt["--out"], opt["--repo"],
                 opt["--main"])


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
