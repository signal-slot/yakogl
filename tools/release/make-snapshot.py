#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""One-commit snapshot of a revision's tree (#1606 step 7, checklist line 8).

The public repository is a new one whose history is one fresh commit of the
cleaned tree (docs/release/checklist.md D3). This tool makes that commit:

    python3 -I tools/release/make-snapshot.py [--repo DIR] [--exclude-file F]
        [--author 'Name <email>'] [--date DATE] REV OUTDIR
    python3 -I tools/release/make-snapshot.py --self-test

REV is resolved to a commit of the repository DIR (default `.`). OUTDIR
must not exist or be empty; it becomes a new git repository whose branch
main holds one root commit (no parent) of REV's tree without the excluded
paths, checked out. Blob ids, modes (executable bits, symlinks) and paths of
every kept file are those of REV; a submodule gitlink that is kept is
refused. The source repository is only read: its working tree, index and
object store are not written (the filtered tree is built in OUTDIR).

The exclusion list is read from REV itself (`git show
REV:tools/release/snapshot-exclude`), or from the file --exclude-file names.
A missing default list is an empty one, and the tool says so. Format: `#`
comment lines and blank lines are ignored; every other line is one literal
path from the root of the tree (no globs, no `.` or `..`); a trailing `/`
names a directory and everything below it, no `/` a file or symlink. A
listed path that is not in REV's tree as that kind is stale, an error.

The author and the date come from --author and --date, else from the
environment (GIT_AUTHOR_NAME, GIT_AUTHOR_EMAIL, GIT_AUTHOR_DATE); the
committer is the same. They never come from a file or a git config: every
git command runs with GIT_CONFIG_NOSYSTEM=1 and GIT_CONFIG_GLOBAL=/dev/null,
and the new repository is made without a template. The commit message is
fixed text and does not name REV.

Output (stdout):

    source <full hash of REV>
    snapshot <full hash of the new commit>
    files <number of paths in the snapshot>
    exclusions: <what was excluded, or none and why>

The tool publishes nothing: it adds no remote to the new repository, sends
nothing off this machine and imports no networking module. Making the
public repository and publishing it are the owner's (checklist D7).

Exit status:
  0  the snapshot was made
  2  usage error, no author name, email or date, or a git error
  3  REV does not resolve to a commit
  4  the source repository has uncommitted changes to tracked files
  5  OUTDIR exists and is not an empty directory
  6  the exclusion list is malformed (or --exclude-file cannot be read)
  7  the exclusion list is stale (a listed path is not in REV's tree)
  8  REV's tree keeps a submodule gitlink
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile

EXCLUDE_IN_TREE = "tools/release/snapshot-exclude"
MESSAGE = ("YakoGL: OpenGL ES 2.0 and EGL for embedded GPUs, without an "
           "operating-system GPU driver\n"
           "\n"
           "The source tree of YakoGL as one snapshot commit.\n")
SELF_TEST_CHECKS = 38

(EXIT_OK, EXIT_USAGE, EXIT_REV, EXIT_DIRTY, EXIT_OUTDIR, EXIT_MALFORMED,
 EXIT_STALE, EXIT_GITLINK) = 0, 2, 3, 4, 5, 6, 7, 8

# Inherited variables that would point git at another repository, index,
# object store or config than the one each command names.
GIT_ENV_DROP = ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE",
                "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                "GIT_COMMON_DIR", "GIT_NAMESPACE", "GIT_CEILING_DIRECTORIES",
                "GIT_CONFIG", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG_COUNT",
                "GIT_TEMPLATE_DIR", "GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL",
                "GIT_COMMITTER_DATE", "GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL",
                "GIT_AUTHOR_DATE")


class SnapshotError(Exception):
    def __init__(self, code, msg):
        super().__init__(msg)
        self.code = code


def git_env(extra=None):
    env = {k: v for k, v in os.environ.items() if k not in GIT_ENV_DROP}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null",
               GIT_LITERAL_PATHSPECS="1", GIT_TERMINAL_PROMPT="0",
               LC_ALL="C")
    env.update(extra or {})
    return env


def git(repo, *args, data=None, env=None, ok=(0,)):
    p = subprocess.run(["git", "--no-optional-locks", "-C", repo, *args],
                       input=data, capture_output=True, env=env or git_env())
    if p.returncode not in ok:
        raise SnapshotError(EXIT_USAGE, "git %s: %s" % (
            args[0], p.stderr.decode(errors="replace").strip()))
    return p.stdout


# --- inputs -----------------------------------------------------------------

def identity(author, date):
    """{GIT_AUTHOR_*, GIT_COMMITTER_*} from the arguments or the environment."""
    name = os.environ.get("GIT_AUTHOR_NAME", "")
    email = os.environ.get("GIT_AUTHOR_EMAIL", "")
    if author is not None:
        m = re.fullmatch(r"\s*([^<>]*?)\s*<([^<>]*)>\s*", author)
        if not m:
            raise SnapshotError(EXIT_USAGE,
                                "--author must read 'Name <email>'")
        name, email = m.group(1), m.group(2)
    when = date if date is not None else os.environ.get("GIT_AUTHOR_DATE", "")
    missing = [what for what, v in (("name", name), ("email", email),
                                    ("date", when)) if not v.strip()]
    if missing:
        raise SnapshotError(EXIT_USAGE, (
            "no author %s: give --author 'Name <email>' and --date, or set "
            "GIT_AUTHOR_NAME, GIT_AUTHOR_EMAIL and GIT_AUTHOR_DATE "
            "(never read from a git config)") % ", ".join(missing))
    return {"GIT_AUTHOR_NAME": name, "GIT_AUTHOR_EMAIL": email,
            "GIT_AUTHOR_DATE": when, "GIT_COMMITTER_NAME": name,
            "GIT_COMMITTER_EMAIL": email, "GIT_COMMITTER_DATE": when}


def resolve(repo, rev):
    out = git(repo, "rev-parse", "--verify", "--quiet", "--end-of-options",
              rev + "^{commit}", ok=(0, 1))
    full = out.decode().strip()
    if not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", full):
        raise SnapshotError(EXIT_REV,
                            "%s does not resolve to a commit" % rev)
    return full


def check_clean(repo):
    if git(repo, "rev-parse", "--is-bare-repository").strip() == b"true":
        return
    out = git(repo, "status", "--porcelain", "--untracked-files=no")
    if out.strip():
        raise SnapshotError(EXIT_DIRTY, (
            "the source repository has uncommitted changes to tracked "
            "files:\n%s") % out.decode(errors="replace").rstrip())


def check_outdir(outdir):
    if os.path.lexists(outdir) and not (os.path.isdir(outdir)
                                        and not os.path.islink(outdir)
                                        and not os.listdir(outdir)):
        raise SnapshotError(EXIT_OUTDIR,
                            "%s exists and is not an empty directory" % outdir)


def tree_entries(repo, commit):
    """[(mode, type, oid, path)] of the commit's tree, recursive."""
    out = git(repo, "ls-tree", "-r", "-z", "--full-tree", commit)
    entries = []
    for entry in out.decode("utf-8", "surrogateescape").split("\0"):
        if entry:
            meta, path = entry.split("\t", 1)
            mode, typ, oid = meta.split()
            entries.append((mode, typ, oid, path))
    return entries


def read_exclusions(repo, commit, exclude_file):
    """(text, where) of the exclusion list; text None when the default is absent."""
    if exclude_file is not None:
        try:
            with open(exclude_file, "rb") as f:
                return f.read(), exclude_file
        except OSError as e:
            raise SnapshotError(EXIT_MALFORMED,
                                "cannot read %s: %s" % (exclude_file, e.strerror))
    out = git(repo, "ls-tree", "-z", "--full-tree", commit, "--",
              EXCLUDE_IN_TREE)
    if not out:
        return None, EXCLUDE_IN_TREE
    if not out.startswith(b"100"):
        raise SnapshotError(EXIT_MALFORMED,
                            "%s in the revision is not a file" % EXCLUDE_IN_TREE)
    return git(repo, "cat-file", "blob", "%s:%s" % (commit, EXCLUDE_IN_TREE)), \
        EXCLUDE_IN_TREE


def parse_exclusions(data, where):
    """[(path, is_dir)] from the list's bytes; malformed lines are an error."""
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        raise SnapshotError(EXIT_MALFORMED, "%s is not UTF-8" % where)
    rules, seen = [], set()
    for n, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        is_dir = line.endswith("/")
        path = line[:-1] if is_dir else line
        parts = path.split("/")
        why = None
        if line.startswith("/"):
            why = "is absolute (paths are relative to the root)"
        elif any(c in path for c in "*?[]\\"):
            why = "has a glob or backslash (paths are literal)"
        elif any(p in ("", ".", "..") for p in parts):
            why = "has an empty, `.` or `..` component"
        elif path in seen:
            why = "repeats an earlier line"
        if why:
            raise SnapshotError(EXIT_MALFORMED,
                                "%s:%d: the path %s" % (where, n, why))
        seen.add(path)
        rules.append((path, is_dir))
    return rules


def apply_exclusions(entries, rules, where):
    """The kept entries; a rule matching nothing of its kind is stale."""
    paths = {e[3] for e in entries}
    for path, is_dir in rules:
        if is_dir:
            if not any(p.startswith(path + "/") for p in paths):
                raise SnapshotError(EXIT_STALE, "%s: %s/ is not a directory "
                                    "of the revision's tree" % (where, path))
        elif path not in paths:
            raise SnapshotError(EXIT_STALE, "%s: %s is not a file of the "
                                "revision's tree" % (where, path))
    files = {p for p, d in rules if not d}
    dirs = tuple(p + "/" for p, d in rules if d)
    return [e for e in entries
            if e[3] not in files and not e[3].startswith(dirs or ("\0",))]


# --- snapshot ---------------------------------------------------------------

def build(repo, commit, kept, outdir, ident):
    """Make OUTDIR's repository and its one commit; return the commit id."""
    out = os.path.abspath(outdir)
    os.makedirs(out, exist_ok=True)
    git(out, "init", "-q", "--template=", "-b", "main", ".")
    oids = sorted({e[2] for e in kept})
    if oids:
        pack = subprocess.run(
            ["git", "-C", repo, "pack-objects", "-q", "--stdout"],
            input=("\n".join(oids) + "\n").encode(), capture_output=True,
            env=git_env())
        if pack.returncode != 0:
            raise SnapshotError(EXIT_USAGE, "git pack-objects: %s" %
                                pack.stderr.decode(errors="replace").strip())
        git(out, "index-pack", "--stdin", data=pack.stdout)
    info = b"".join(("%s %s\t%s\0" % (mode, oid, path)).encode(
        "utf-8", "surrogateescape") for mode, _, oid, path in kept)
    git(out, "update-index", "-z", "--index-info", data=info)
    tree = git(out, "write-tree").decode().strip()
    snap = git(out, "commit-tree", tree, data=MESSAGE.encode(),
               env=git_env(ident)).decode().strip()
    git(out, "update-ref", "refs/heads/main", snap, "")
    git(out, "reset", "-q", "--hard", "main")
    return snap


def run(repo, rev, outdir, exclude_file, author, date):
    ident = identity(author, date)
    commit = resolve(repo, rev)
    check_clean(repo)
    check_outdir(outdir)
    entries = tree_entries(repo, commit)
    data, where = read_exclusions(repo, commit, exclude_file)
    rules = parse_exclusions(data, where) if data is not None else []
    kept = apply_exclusions(entries, rules, where)
    links = [e[3] for e in kept if e[1] == "commit"]
    if links:
        raise SnapshotError(EXIT_GITLINK, "the tree keeps submodule "
                            "gitlink(s), not supported: %s" % ", ".join(links))
    made = not os.path.lexists(outdir)
    try:
        snap = build(repo, commit, kept, outdir, ident)
    except BaseException:
        if made:
            shutil.rmtree(outdir, ignore_errors=True)
        elif os.path.isdir(outdir):
            for name in os.listdir(outdir):
                full = os.path.join(outdir, name)
                if os.path.isdir(full) and not os.path.islink(full):
                    shutil.rmtree(full, ignore_errors=True)
                else:
                    os.unlink(full)
        raise
    print("source %s" % commit)
    print("snapshot %s" % snap)
    print("files %d" % len(kept))
    if data is None:
        print("exclusions: none (%s is not in the revision)" % where)
    elif not rules:
        print("exclusions: none (%s lists no path)" % where)
    else:
        print("exclusions: %d path(s) left out by %d rule(s) of %s"
              % (len(entries) - len(kept), len(rules), where))
    return EXIT_OK


# --- self-test --------------------------------------------------------------

def self_test():
    """Snapshot a throwaway fixture repository; 0 when every check holds."""
    failures, ran = [], [0]

    def check(desc, ok):
        ran[0] += 1
        print("%s  %s" % ("ok  " if ok else "FAIL", desc))
        if not ok:
            failures.append(desc)

    tmp = tempfile.mkdtemp(prefix="make-snapshot-")
    try:
        home = os.path.join(tmp, "home")
        os.makedirs(home)
        # A host identity in a global config and the source's own config:
        # neither may reach the snapshot.
        with open(os.path.join(home, ".gitconfig"), "w") as f:
            f.write("[user]\n\tname = host-global\n"
                    "\temail = host-global@example.invalid\n")
        base = {k: v for k, v in os.environ.items() if k not in GIT_ENV_DROP}
        base.update(HOME=home, XDG_CONFIG_HOME=os.path.join(home, ".config"))
        fixture_env = dict(base, GIT_CONFIG_NOSYSTEM="1",
                           GIT_AUTHOR_NAME="f", GIT_AUTHOR_EMAIL="f@example.invalid",
                           GIT_COMMITTER_NAME="f",
                           GIT_COMMITTER_EMAIL="f@example.invalid")

        def sh(repo, *args):
            return subprocess.run(["git", "-C", repo, *args], env=fixture_env,
                                  check=True, capture_output=True).stdout

        def write(repo, path, text, mode=None):
            full = os.path.join(repo, path)
            os.makedirs(os.path.dirname(full), exist_ok=True)
            with open(full, "w", encoding="utf-8", newline="\n") as f:
                f.write(text)
            if mode is not None:
                os.chmod(full, mode)

        src = os.path.join(tmp, "src")
        os.makedirs(src)
        sh(src, "init", "-q", "-b", "main")
        sh(src, "config", "user.name", "host-local")
        sh(src, "config", "user.email", "host-local@example.invalid")
        write(src, "README", "fixture\n")
        write(src, "a/b/c/deep.txt", "deep\n")
        sh(src, "add", "-A")
        sh(src, "commit", "-q", "-m", "one")
        first = sh(src, "rev-parse", "HEAD").decode().strip()
        write(src, "run.sh", "#!/bin/sh\necho run\n", 0o755)
        os.symlink("run.sh", os.path.join(src, "link"))
        write(src, "internal/notes.md", "internal\n")
        write(src, "internal/deeper/more.md", "more\n")
        write(src, "logs/one.log", "log\n")
        write(src, "a/b/keep.txt", "keep\n")
        write(src, "a/b/drop.txt", "drop\n")
        write(src, EXCLUDE_IN_TREE, "\n".join([
            "# fixture exclusions", "", "internal/", "a/b/drop.txt",
            "  logs/  ", ""]))
        sh(src, "add", "-A")
        sh(src, "commit", "-q", "-m", "two")
        second = sh(src, "rev-parse", "HEAD").decode().strip()
        excluded = ("internal/", "a/b/drop.txt", "logs/")

        def is_excluded(path):
            return any(path == x or (x.endswith("/") and path.startswith(x))
                       for x in excluded)

        def tool(*args, env=None):
            p = subprocess.run([sys.executable, "-I", os.path.abspath(__file__),
                                "--repo", src, *args], cwd=tmp,
                               env=env or base, capture_output=True, text=True)
            return p.returncode, p.stdout, p.stderr

        def lstree(repo, rev):
            return sorted(git(repo, "ls-tree", "-r", "-z", "--full-tree", rev)
                          .split(b"\0"))

        index = os.path.join(src, ".git", "index")
        with open(index, "rb") as f:
            index_before = f.read()
        objects_before = git(src, "count-objects", "-v")
        ident = ["--author", "Snap Shot <snap@example.invalid>",
                 "--date", "2026-11-13T00:00:00+00:00"]

        out = os.path.join(tmp, "out")
        code, stdout, stderr = tool(*ident, "main", out)
        check("a snapshot of the second commit exits 0 (%d%s)"
              % (code, ": " + stderr.strip() if code else ""), code == 0)
        lines = stdout.splitlines()
        check("the first line names the source commit in full",
              lines[:1] == ["source " + second])
        snap = git(out, "rev-parse", "HEAD").decode().strip() \
            if os.path.isdir(os.path.join(out, ".git")) else ""
        check("the second line names the snapshot commit in full",
              bool(snap) and lines[1:2] == ["snapshot " + snap])
        check("main is the checked-out branch",
              git(out, "symbolic-ref", "HEAD").strip() == b"refs/heads/main")
        check("the snapshot has exactly one commit",
              git(out, "rev-list", "--all", "--count").strip() == b"1")
        check("the commit has no parent",
              git(out, "rev-list", "--parents", "-n1", "HEAD").split() == [snap.encode()])
        kept_src = [e for e in lstree(src, second)
                    if e and not is_excluded(e.decode().split("\t", 1)[1])]
        check("the snapshot's ls-tree -r is the source's minus the exclusions "
              "(paths, modes, blob ids)", [e for e in lstree(out, "HEAD") if e] == kept_src)
        check("the third line counts the kept paths",
              lines[2:3] == ["files %d" % len(kept_src)])
        check("the exclusions line counts the dropped paths",
              lines[3:4] == ["exclusions: 4 path(s) left out by 3 rule(s) of "
                             + EXCLUDE_IN_TREE])
        names = git(out, "ls-tree", "-r", "--name-only", "--full-tree", "HEAD") \
            .decode().splitlines()
        check("the excluded files and directories are not in the tree",
              not any(is_excluded(n) for n in names))
        check("the excluded paths are not in the working tree",
              not any(os.path.lexists(os.path.join(out, p)) for p in
                      ("internal", "logs", "a/b/drop.txt")))
        modes = {e.decode().split("\t", 1)[1]: e.split()[0] for e in kept_src}
        check("the executable keeps mode 100755 and the symlink 120000",
              modes.get("run.sh") == b"100755" and modes.get("link") == b"120000")
        check("the working tree holds the nested file's bytes",
              open(os.path.join(out, "a/b/c/deep.txt"), "rb").read() == b"deep\n")
        check("the working tree holds the symlink as a link",
              os.path.islink(os.path.join(out, "link"))
              and os.readlink(os.path.join(out, "link")) == "run.sh")
        check("the working tree's executable is executable",
              os.access(os.path.join(out, "run.sh"), os.X_OK))
        check("the snapshot's working tree is clean",
              git(out, "status", "--porcelain").strip() == b"")
        who = git(out, "log", "-1", "--date=raw",
                  "--format=%an|%ae|%ad|%cn|%ce|%cd").decode().strip()
        check("author and committer are the ones given, with the date given "
              "(%s)" % who,
              who == "Snap Shot|snap@example.invalid|1794528000 +0000|"
                     "Snap Shot|snap@example.invalid|1794528000 +0000")
        raw = git(out, "cat-file", "commit", "HEAD").decode()
        check("no host identity (global or source config) is in the commit",
              "host-" not in raw)
        check("the message is the fixed text and names no source hash",
              raw.split("\n\n", 1)[1] == MESSAGE and second not in raw)
        check("the new repository's config has no remote",
              git(out, "config", "--file", os.path.join(out, ".git", "config"),
                  "--get-regexp", r"^remote\.", ok=(0, 1)).strip() == b"")
        check("the new repository has no hooks (no template)",
              not os.listdir(os.path.join(out, ".git", "hooks"))
              if os.path.isdir(os.path.join(out, ".git", "hooks")) else True)
        with open(index, "rb") as f:
            index_after = f.read()
        check("the source's index and object store are not written",
              index_after == index_before
              and git(src, "count-objects", "-v") == objects_before)

        # The environment gives the identity when no argument does.
        env_ident = dict(base, GIT_AUTHOR_NAME="Env Name",
                         GIT_AUTHOR_EMAIL="env@example.invalid",
                         GIT_AUTHOR_DATE="2026-01-02T03:04:05+09:00")
        out2 = os.path.join(tmp, "out2")
        code, stdout, _ = tool("main", out2, env=env_ident)
        who = git(out2, "log", "-1", "--date=raw",
                  "--format=%an|%ae|%ad|%cn|%cd").decode().strip() if code == 0 else ""
        check("without --author/--date the environment's identity is used",
              who == "Env Name|env@example.invalid|1767290645 +0900|"
                     "Env Name|1767290645 +0900")
        code, _, stderr = tool("main", os.path.join(tmp, "out3"))
        check("no name, email or date (only a git config) is refused (exit 2)",
              code == EXIT_USAGE and "no author" in stderr
              and not os.path.lexists(os.path.join(tmp, "out3")))

        # A revision without the default list: empty list, said so, and the
        # whole tree is kept (the same tree id).
        out4 = os.path.join(tmp, "out4")
        code, stdout, _ = tool(*ident, first, out4)
        check("a revision without %s keeps every path and says so"
              % EXCLUDE_IN_TREE,
              code == 0 and "exclusions: none (%s is not in the revision)"
              % EXCLUDE_IN_TREE in stdout
              and git(out4, "rev-parse", "HEAD^{tree}")
              == git(src, "rev-parse", first + "^{tree}"))

        # --exclude-file replaces the list in the revision.
        listing = os.path.join(tmp, "list")
        with open(listing, "w") as f:
            f.write("link\na/\n")
        out5 = os.path.join(tmp, "out5")
        code, stdout, _ = tool(*ident, "--exclude-file", listing, second, out5)
        names = git(out5, "ls-tree", "-r", "--name-only", "--full-tree", "HEAD") \
            .decode().splitlines() if code == 0 else []
        check("--exclude-file replaces the revision's list",
              code == 0 and "link" not in names
              and not any(n.startswith("a/") for n in names)
              and "internal/notes.md" in names)

        def refused(desc, want, args, outdir):
            code, _, stderr = tool(*ident, *args, outdir)
            check("%s is refused (exit %d, got %d)" % (desc, want, code),
                  code == want and stderr.strip() != "")

        refused("a missing revision", EXIT_REV, ["no-such-rev"],
                os.path.join(tmp, "r1"))
        refused("a tree id as the revision", EXIT_REV, [second + "^{tree}"],
                os.path.join(tmp, "r2"))
        busy = os.path.join(tmp, "busy")
        write(busy, "keep-me", "x\n")
        refused("a non-empty outdir", EXIT_OUTDIR, ["main"], busy)
        check("the non-empty outdir is left as it was",
              os.listdir(busy) == ["keep-me"])
        for desc, text, want in (
                ("a stale file exclusion", "gone.txt\n", EXIT_STALE),
                ("a stale directory exclusion", "gone/\n", EXIT_STALE),
                ("a file listed as a directory", "run.sh/\n", EXIT_STALE),
                ("a glob in the exclusion list", "logs/*.log\n", EXIT_MALFORMED),
                ("a `..` in the exclusion list", "a/../README\n",
                 EXIT_MALFORMED)):
            with open(listing, "w") as f:
                f.write(text)
            refused(desc, want, ["--exclude-file", listing, "main"],
                    os.path.join(tmp, "r3"))
        write(src, "README", "changed\n")
        refused("a dirty source", EXIT_DIRTY, ["main"], os.path.join(tmp, "r4"))
        check("a refused run makes no outdir",
              not any(os.path.lexists(os.path.join(tmp, r))
                      for r in ("r1", "r2", "r3", "r4")))
        sh(src, "checkout", "-q", "--", "README")
        os.makedirs(os.path.join(src, "sub"))
        sh(src, "update-index", "--add", "--cacheinfo",
           "160000,%s,sub" % first)
        sh(src, "commit", "-q", "-m", "gitlink")
        refused("a kept submodule gitlink", EXIT_GITLINK, ["main"],
                os.path.join(tmp, "r5"))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    if ran[0] != SELF_TEST_CHECKS:
        failures.append("ran %d checks, the script pins %d"
                        % (ran[0], SELF_TEST_CHECKS))
        print("FAIL  " + failures[-1])
    print("%d check(s) failed" % len(failures) if failures else "self-test passed")
    return 1 if failures else 0


def main():
    ap = argparse.ArgumentParser(
        description=__doc__.split("\n\n", 1)[0],
        epilog=__doc__.split("\n\n", 1)[1],
        formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("rev", nargs="?", help="revision whose tree is snapshot")
    ap.add_argument("outdir", nargs="?",
                    help="new repository's directory (absent or empty)")
    ap.add_argument("--repo", default=".",
                    help="source repository (default: the current directory)")
    ap.add_argument("--exclude-file",
                    help="exclusion list (default: %s read from REV)"
                         % EXCLUDE_IN_TREE)
    ap.add_argument("--author", help="'Name <email>' of the author and "
                                     "committer (default: GIT_AUTHOR_NAME, "
                                     "GIT_AUTHOR_EMAIL)")
    ap.add_argument("--date", help="author and commit date, any form git "
                                   "takes (default: GIT_AUTHOR_DATE)")
    ap.add_argument("--self-test", action="store_true",
                    help="snapshot a throwaway fixture repository and check "
                         "the tree, the commit and the refusals")
    try:
        args = ap.parse_args()
    except SystemExit as e:
        return EXIT_USAGE if e.code else EXIT_OK
    if args.self_test:
        if args.rev or args.outdir:
            print("make-snapshot: --self-test takes no other argument",
                  file=sys.stderr)
            return EXIT_USAGE
        return self_test()
    if not args.rev or not args.outdir:
        print("make-snapshot: give REV and OUTDIR", file=sys.stderr)
        return EXIT_USAGE
    try:
        return run(args.repo, args.rev, args.outdir, args.exclude_file,
                   args.author, args.date)
    except SnapshotError as e:
        print("make-snapshot: %s" % e, file=sys.stderr)
        return e.code


if __name__ == "__main__":
    sys.exit(main())
