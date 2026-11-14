#!/usr/bin/env python3
"""Check the DK note's "## Board run" section for acceptance c5's evidence.

    python3 -I tools/mp2_uboot_note_check.py [NOTE.md]
    python3 -I tools/mp2_uboot_note_check.py --self-test

NOTE.md defaults to docs/notes/stm32mp257f-dk-uboot-boot.md. The section runs
from the "## Board run" heading to the next "## " heading or the end. Checks:

  verdict     a runner verdict line (as board-stm32mp2.sh prints it:
              "<tag> <board> <name> rc=<rc> verdict=<v> boot=<route> row|blind|DEV ...")
              with verdict=done, rc=0 and the word "row"; never DEV, never
              another verdict. Every verdict line in the section must be good.
  fw_printenv "fw_printenv diff: empty" (the heading tools/probes/
              stm32mp257f_dk/env_read/summarize.py prints; "N variable(s)" fails)
  /boot       "/boot files outside yakogl/: unchanged" (summarize.py's heading;
              "N differ" fails, and so does "RESULT: DIFFERENT")

Output: one line per check, then "note-check: N ok, M failed". Exit 0 when all
pass; 1 when one fails; 2 with "note-check: not run yet" when the section still
holds a placeholder (<...> or TODO(board-run)) or has no content; 2 also for
usage errors and an unreadable file. A missing section fails every check (1).
"""
import os
import re
import sys

DEFAULT_NOTE = os.path.join(os.path.dirname(os.path.abspath(__file__)), os.pardir,
                            "docs", "notes", "stm32mp257f-dk-uboot-boot.md")
HEAD_RX = re.compile(r"^##[ \t]+Board run[ \t]*$", re.M)
# summarize.py prints its own "## fw_printenv diff" and "## /boot files" headings
NEXT_RX = re.compile(r"^##[ \t]+(?!fw_printenv diff\b|/boot files\b)\S", re.M)
PLACEHOLDER_RX = re.compile(r"<[^<>\s][^<>\n]*>|TODO\(board-run\)")
VERDICT_RX = re.compile(r"\brc=(-?\d+) verdict=(\S+)(.*)$")
ENV_RX = re.compile(r"fw_printenv diff\W[^\n]*?\b(empty|\d+ variable\(s\))")
BOOT_RX = re.compile(r"/boot files outside yakogl/\W[^\n]*?\b(unchanged|\d+ differ)")
RESULT_RX = re.compile(r"^\W*RESULT: (IDENTICAL|DIFFERENT[^\n]*)", re.M)


def section(text):
    m = HEAD_RX.search(text)
    if not m:
        return None
    rest = text[m.end():]
    n = NEXT_RX.search(rest)
    return rest[:n.start()] if n else rest


def check_verdict(sec):
    lines = [l for l in sec.splitlines() if VERDICT_RX.search(l)]
    if not lines:
        return False, "no runner verdict line (... rc=<rc> verdict=<v> boot=<route> row ...)"
    bad = []
    for l in lines:
        m = VERDICT_RX.search(l)
        rc, v, rest = m.group(1), m.group(2), m.group(3)
        toks = re.split(r"[\s`*]+", rest)
        why = []
        if v != "done":
            why.append("verdict=%s" % v)
        if rc != "0":
            why.append("rc=%s" % rc)
        if "DEV" in toks or "dev" in toks:
            why.append("a DEV run")
        elif "row" not in toks:
            why.append("not a row")
        if why:
            bad.append(", ".join(why))
    if bad:
        return False, "; ".join(bad)
    return True, "verdict=done row (%d line%s)" % (len(lines), "" if len(lines) == 1 else "s")


def check_env(sec):
    found = ENV_RX.findall(sec)
    if not found:
        return False, "no 'fw_printenv diff: empty' line"
    if any(f != "empty" for f in found):
        return False, "fw_printenv diff is not empty (%s)" % ", ".join(f for f in found if f != "empty")
    return True, "fw_printenv diff empty"


def check_boot(sec):
    found = BOOT_RX.findall(sec)
    if not found:
        return False, "no '/boot files outside yakogl/: unchanged' line"
    if any(f != "unchanged" for f in found):
        return False, "/boot stock files changed (%s)" % ", ".join(f for f in found if f != "unchanged")
    res = [r for r in RESULT_RX.findall(sec) if r.startswith("DIFFERENT")]
    if res:
        return False, "the env_read summary says RESULT: %s" % res[0]
    return True, "/boot stock files unchanged"


CHECKS = (("verdict", check_verdict), ("fw_printenv", check_env), ("/boot sha256", check_boot))


def run(text, out=print):
    """Return the exit code."""
    sec = section(text)
    if sec is None:
        for name, _ in CHECKS:
            out("FAILED %-12s no '## Board run' section" % name)
        out("note-check: 0 ok, %d failed" % len(CHECKS))
        return 1
    # the section's own instructions quote the shape of the lines: code spans
    # and the italic template paragraph are not evidence, but they do hold
    # placeholders; any placeholder means the run has not been written.
    if PLACEHOLDER_RX.search(sec) or not sec.strip():
        out("note-check: not run yet")
        return 2
    ok = failed = 0
    for name, fn in CHECKS:
        good, msg = fn(sec)
        out("%-6s %-12s %s" % ("ok" if good else "FAILED", name, msg))
        ok, failed = ok + good, failed + (not good)
    out("note-check: %d ok, %d failed" % (ok, failed))
    return 0 if not failed else 1


# ---- self test ---------------------------------------------------------------
VLINE = "board-stm32mp2-uboot.sh stm32mp257f-dk mp2_hello rc=0 verdict=done boot=uboot-sd row boot_verdict=done result=pass:mp2_hello back_after=41s"


def note(verdict=VLINE, env="empty", boot="unchanged", extra=""):
    return ("# Note\n\n## 11. Index\n\n| a | b |\n\n## Board run\n\n"
            "Run of 2026-11-20, image mp2_hello.\n\n1. Verdict:\n\n```\n%s\n```\n\n"
            "## fw_printenv diff: %s\n\n"
            "### u-boot-env image\n\n"
            "## /boot files outside yakogl/: %s\n\n%s"
            "RESULT: %s\n\n## Appendix\n\nafter the section <not> counted\n"
            % (verdict, env, boot, extra,
               "IDENTICAL" if env == "empty" and boot == "unchanged" else "DIFFERENT (1)"))


def self_test():
    cases = [
        ("pass", note(), 0, "2 ok" if False else "3 ok, 0 failed"),
        ("pass, section ends at EOF", note().split("## Appendix")[0], 0, "3 ok, 0 failed"),
        ("DEV verdict", note(VLINE.replace(" row ", " DEV ")), 1, "a DEV run"),
        ("verdict not done", note(VLINE.replace("verdict=done", "verdict=hang")), 1, "verdict=hang"),
        ("rc not 0", note(VLINE.replace("rc=0", "rc=4")), 1, "rc=4"),
        ("blind run", note(VLINE.replace(" row ", " blind ")), 1, "not a row"),
        ("no verdict line", note("(none)"), 1, "no runner verdict line"),
        ("non-empty diff", note(env="2 variable(s)"), 1, "fw_printenv diff is not empty"),
        ("changed sha256", note(boot="1 differ"), 1, "/boot stock files changed"),
        ("RESULT DIFFERENT only", note(extra="RESULT: DIFFERENT (1)\n"), 1, "RESULT: DIFFERENT"),
        ("placeholder <...>", note(VLINE.replace("rc=0", "rc=<rc>")), 2, "not run yet"),
        ("placeholder TODO", note(extra="TODO(board-run) the diff\n"), 2, "not run yet"),
        ("empty section", "# N\n\n## Board run\n\n", 2, "not run yet"),
        ("missing section", "# N\n\n## Other\n\ntext\n", 1, "no '## Board run' section"),
    ]
    fails = 0
    for name, text, want_rc, want_msg in cases:
        got = []
        rc = run(text, got.append)
        good = rc == want_rc and any(want_msg in l for l in got)
        if want_rc == 1 and name != "missing section":
            good = good and got[-1].endswith("failed") and not got[-1].endswith(" 0 failed")
        print("%s self-test: %s (exit %d)" % ("ok  " if good else "FAIL", name, rc))
        if not good:
            fails += 1
            for l in got:
                print("      " + l)
    print("self-test: %d cases, %d failed" % (len(cases), fails))
    return 1 if fails else 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if len(argv) > 1 or (argv and argv[0].startswith("-")):
        print(__doc__, file=sys.stderr)
        return 2
    path = argv[0] if argv else DEFAULT_NOTE
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError as e:
        print("note-check: cannot read %s: %s" % (os.path.basename(path), e.strerror), file=sys.stderr)
        return 2
    return run(text)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
