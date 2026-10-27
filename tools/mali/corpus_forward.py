#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Mechanical check that corpus-verdicts.txt moves forward only (plan #153, c6).

    python3 -I tools/mali/corpus_forward.py [--verbose] <BASE_REV> [<HEAD_REV>]

tools/mali/corpus-verdicts.txt has one line per shader, `<path>: ok` or
`<path>: <refusal text>`; `#` lines are comments. The file is read at BASE_REV,
at every commit of `git rev-list --reverse BASE..HEAD -- <file>` and at the
end (HEAD_REV, default the working tree). Between two consecutive states, a
path that was `ok` and is now anything else, or is gone, is an offence: one
line `rev path: ok -> new`, exit 1. Every commit must be green, so a
regression that a later commit fixes is still reported. Added paths, refusals
whose text changes and refusals that become ok are allowed (listed under
--verbose). A malformed line or an unreadable revision is exit 2.

No exemption exists, none is needed over the public range. Plan #154 (a)
expected to accept the removal of the tools/shaders/qt* lines, whose sources
the owner removed in c2538996 (#1509). That commit belongs to the history
before the repository started over (root 381b0f09, 2026-10-27): the root's
corpus-verdicts.txt has no qt* line, so 381b0f09..HEAD removes none, and the
range reports 0 regressions. A removal in a later range is an offence like any
other; do not add an allow-list for it.
"""
import subprocess
import sys

CORPUS = "tools/mali/corpus-verdicts.txt"
GONE = "(removed)"


class Error(Exception):
    pass


def git(*args):
    r = subprocess.run(["git", *args], capture_output=True, text=True)
    if r.returncode:
        raise Error("git %s: %s" % (" ".join(args), r.stderr.strip()))
    return r.stdout


def parse(text, where):
    verdicts = {}
    for n, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        path, sep, verdict = line.partition(": ")
        if not sep or not path or not verdict.strip() or path in verdicts:
            raise Error("%s:%d: malformed line: %r" % (where, n, line))
        verdicts[path] = verdict.strip()
    return verdicts


def at_rev(rev):
    r = subprocess.run(["git", "show", "%s:%s" % (rev, CORPUS)], capture_output=True, text=True)
    if r.returncode:
        # a revision without the file yet is an empty corpus; a bad revision is an error
        git("rev-parse", "--verify", rev + "^{commit}")
        return {}
    return parse(r.stdout, "%s:%s" % (rev[:12], CORPUS))


def count_ok(state):
    return sum(1 for v in state.values() if v == "ok")


def main(argv):
    verbose = "--verbose" in argv
    args = [a for a in argv if a != "--verbose"]
    if not 1 <= len(args) <= 2:
        print(__doc__)
        return 2
    base = args[0]
    head = args[1] if len(args) > 1 else None
    try:
        revs = git("rev-list", "--reverse", "%s..%s" % (base, head or "HEAD"), "--", CORPUS).split()
        states = [(base[:12], at_rev(base))]
        states += [(r[:12], at_rev(r)) for r in revs]
        if head:
            states.append((head[:12], at_rev(head)))
        else:
            with open(CORPUS) as f:
                states.append(("worktree", parse(f.read(), CORPUS)))
    except (Error, OSError) as e:
        print("corpus_forward: %s" % e, file=sys.stderr)
        return 2
    bad = 0
    prev = states[0][1]
    for name, cur in states[1:]:
        for path in sorted(set(prev) | set(cur)):
            old, new = prev.get(path), cur.get(path)
            if old == new:
                continue
            shown = new if new is not None else GONE
            if old == "ok":
                bad += 1
                print("%s %s: ok -> %s" % (name, path, shown))
            elif verbose:
                print("info %s %s: %s -> %s" % (name, path, old or "(new)", shown))
        prev = cur
    print("corpus-verdicts: %d commits, %d ok -> %d ok, %d regressions"
          % (len(revs), count_ok(states[0][1]), count_ok(states[-1][1]), bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
