#!/usr/bin/env python3
"""Check that a runner transcript prints each undo before its change.

Usage: mp2s_undo_order.py [TRANSCRIPT]     (no argument or "-": stdin)

Line formats.  Section 7 of docs/notes/stm32mp257f-dk-standalone.md fixes the
undo/change *text* only loosely ("UNDO: ..." banners), not one command per
line, so this tool defines the machine-checked lines here.  The runner's
dry-run must print them exactly so (any other line is ignored):

    undo: <command>
    change: <command>

<command> is one shell command, shell-quoted.  Recognised commands:

  fw_setenv [opts] VAR [VALUE...]   object = env variable VAR
  cp|install|mv [opts] SRC... DST   object = DST (the staged file)
  tee [opts] DST                    object = DST
  rm [opts] PATH...                 object = each PATH

Inverses (the undo line must be seen BEFORE the change line, same object):

  change fw_setenv VAR VALUE   undo fw_setenv VAR (no value: delete),
                               or fw_setenv VAR OLD with OLD != VALUE
  change fw_setenv VAR         undo fw_setenv VAR VALUE (restore a value)
  change cp/install/tee/mv -> DST
                               undo rm DST, or cp/mv/install SAVED DST with
                               SAVED != DST (restore from the saved copy)
  change rm PATH               undo cp/mv/install SAVED PATH, SAVED != PATH

An undo stays valid for later changes of the same object until a newer undo
of that object replaces it.  Any change or undo line naming bootcmd,
bootcount or altbootcmd is rejected.

Exit: 0 all good; 1 a violation ("line N: why" on stderr, N is the offending
line); 2 usage error or unreadable input.
"""
import shlex
import sys

PROTECTED = {"bootcmd", "bootcount", "altbootcmd"}
FILE_CMDS = {"cp", "install", "mv", "tee"}


class Bad(Exception):
    pass


def parse(words):
    """Return (kind, objects, detail) for a split command."""
    words = list(words)
    while words and words[0] in ("sudo", "env"):
        words.pop(0)
    if not words:
        raise Bad("empty command")
    cmd = words[0].rsplit("/", 1)[-1]
    args = [w for w in words[1:] if not w.startswith("-") or w == "-"]
    if cmd == "fw_setenv":
        if not args:
            raise Bad("fw_setenv without a variable")
        return ("env", [args[0]], args[1:] or None)
    if cmd == "tee":
        if not args:
            raise Bad("tee without a path")
        return ("put", [args[-1]], [])
    if cmd in FILE_CMDS:
        if len(args) < 2:
            raise Bad("%s needs a source and a destination" % cmd)
        return ("put", [args[-1]], args[:-1])
    if cmd == "rm":
        if not args:
            raise Bad("rm without a path")
        return ("rm", args, None)
    raise Bad("unknown command %r (not fw_setenv/cp/install/mv/tee/rm)" % cmd)


def restores(undo, obj):
    uk, _, ud = undo
    return uk == "put" and bool(ud) and obj not in ud


def inverts(change, undo, obj):
    ck, _, cd = change
    uk, _, ud = undo
    if ck == "env":
        if uk != "env":
            return False
        if cd is None:
            return ud is not None
        return ud is None or ud != cd
    if ck == "put":
        return uk == "rm" or restores(undo, obj)
    if ck == "rm":
        return restores(undo, obj)
    return False


def check(lines):
    """Return None if fine, else (line number, reason)."""
    seen = {}  # object -> latest undo
    for n, raw in enumerate(lines, 1):
        s = raw.strip()
        tag = next((t for t in ("undo:", "change:") if s.startswith(t)), None)
        if tag is None:
            continue
        what = tag[:-1]
        try:
            words = shlex.split(s[len(tag):])
        except ValueError as e:
            return n, "%s: unparsable command: %s" % (what, e)
        for w in words:
            if w.split("=")[0] in PROTECTED:
                return n, "%s touches %s" % (what, w.split("=")[0])
        try:
            p = parse(words)
        except Bad as e:
            return n, "%s: %s" % (what, e)
        if what == "undo":
            for o in p[1]:
                seen[o] = p
            continue
        for o in p[1]:
            u = seen.get(o)
            if u is None:
                return n, "change of %s has no earlier undo" % o
            if not inverts(p, u, o):
                return n, "the earlier undo of %s does not invert this change" % o
    return None


def main(argv):
    if len(argv) > 2 or (len(argv) == 2 and argv[1] in ("-h", "--help")):
        sys.stderr.write(__doc__)
        return 2
    try:
        if len(argv) == 1 or argv[1] == "-":
            lines = sys.stdin.read().splitlines()
        else:
            with open(argv[1], encoding="utf-8", errors="replace") as f:
                lines = f.read().splitlines()
    except OSError as e:
        sys.stderr.write("mp2s_undo_order: %s\n" % e)
        return 2
    r = check(lines)
    if r:
        sys.stderr.write("mp2s_undo_order: line %d: %s\n" % r)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
