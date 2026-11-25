#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tally one panthor-gltest sweep, and diff it per case against another.

  python3 -I tools/cts/mali-sweep-tally.py [--markdown] <verdicts.txt>
  python3 -I tools/cts/mali-sweep-tally.py [--markdown] --against OLD <verdicts.txt>

<verdicts.txt> is one panthor-gltest --sweep run (for example
tools/cts/log/2026-11-t6-sweep-1a22bd38/sweep-1.verdicts.txt): lines
"panthor-gltest: verdict case N <name>: <VERDICT> <text>" and one closing
"panthor-gltest: N case(s) run: CLASS n, CLASS n, ..." summary. Other lines
(comments, blank lines) are skipped.

The tally is per verdict class and is checked against the summary line: a
different count, a missing or doubled summary, a doubled case number or a
case count that is not the summary's N is an error (exit 1). With --against
the per-case diff is printed as well: the cases whose verdict, name or text
changed, the cases only in one file, and the per-class deltas. With
--markdown the same is printed as markdown tables for a record README.
Both files are checked; the exit status is 1 if either is inconsistent.
"""
import re
import sys

VERDICT = re.compile(r"^panthor-gltest: verdict case (\d+) (.*?): ([A-Z][A-Z-]*)(?: (.*))?$")
SUMMARY = re.compile(r"^panthor-gltest: (\d+) case\(s\) run:(.*)$")


def parse(path):
    """Return (cases {n: (name, verdict, text)}, summary (total, {class: n}) or None, errors)."""
    cases, summary, errors = {}, None, []
    with open(path, encoding="utf-8") as f:
        for ln, line in enumerate(f, 1):
            line = line.rstrip("\n")
            m = VERDICT.match(line)
            if m:
                n = int(m.group(1))
                if n in cases:
                    errors.append(path + ":" + str(ln) + ": case " + str(n) + " given twice")
                cases[n] = (m.group(2), m.group(3), (m.group(4) or "").strip())
                continue
            m = SUMMARY.match(line)
            if m:
                if summary is not None:
                    errors.append(path + ":" + str(ln) + ": a second summary line")
                    continue
                counts = {}
                for part in m.group(2).split(","):
                    part = part.strip()
                    if not part:
                        continue
                    pm = re.fullmatch(r"([A-Z][A-Z-]*) (\d+)", part)
                    if not pm:
                        errors.append(path + ":" + str(ln) + ": unreadable summary part '" + part + "'")
                        continue
                    counts[pm.group(1)] = int(pm.group(2))
                summary = (int(m.group(1)), counts)
            elif line.startswith("panthor-gltest: verdict"):
                errors.append(path + ":" + str(ln) + ": unreadable verdict line")
    return cases, summary, errors


def tally(cases):
    t = {}
    for _, v, _ in cases.values():
        t[v] = t.get(v, 0) + 1
    return t


def check(path, cases, summary, errors):
    t = tally(cases)
    if summary is None:
        errors.append(path + ": no summary line")
        return t
    total, counts = summary
    if total != len(cases):
        errors.append("%s: summary says %d case(s), %d verdict line(s)" % (path, total, len(cases)))
    if sum(counts.values()) != total:
        errors.append("%s: summary classes add up to %d, not %d" % (path, sum(counts.values()), total))
    for c in sorted(set(t) | set(counts)):
        if t.get(c, 0) != counts.get(c, 0):
            errors.append("%s: %s: %d verdict line(s), summary says %d" % (path, c, t.get(c, 0), counts.get(c, 0)))
    return t


def table(md, header, rows):
    out = []
    if md:
        out.append("| " + " | ".join(header) + " |")
        out.append("|" + "|".join("---" for _ in header) + "|")
        for r in rows:
            out.append("| " + " | ".join(str(c).replace("|", "\\|") for c in r) + " |")
    else:
        out.append("\t".join(header))
        for r in rows:
            out.append("\t".join(str(c) for c in r))
    return out


def main(argv):
    md, against, paths, i = False, None, [], 1
    while i < len(argv):
        a = argv[i]
        if a == "--markdown":
            md = True
        elif a == "--against":
            i += 1
            if i >= len(argv):
                print("mali-sweep-tally: --against needs a file", file=sys.stderr)
                return 2
            against = argv[i]
        elif a in ("-h", "--help"):
            print(__doc__)
            return 0
        elif a.startswith("-"):
            print("mali-sweep-tally: unknown option " + a, file=sys.stderr)
            return 2
        else:
            paths.append(a)
        i += 1
    if len(paths) != 1:
        print("usage: mali-sweep-tally.py [--markdown] [--against OLD] <verdicts.txt>", file=sys.stderr)
        return 2
    new_path = paths[0]
    try:
        new, nsum, nerr = parse(new_path)
        old, osum, oerr = (None, None, [])
        if against:
            old, osum, oerr = parse(against)
    except OSError as e:
        print("mali-sweep-tally: " + str(e), file=sys.stderr)
        return 2
    nt = check(new_path, new, nsum, nerr)
    ot = check(against, old, osum, oerr) if against else None
    out = []
    h = "### " if md else ""
    out.append("%sTally: %s (%d case(s))" % (h, new_path, len(new)))
    rows = [(c, nt[c]) for c in sorted(nt)]
    out += table(md, ["verdict", "cases"], rows)
    out.append("panthor-gltest: %d case(s) run: %s" % (len(new), ", ".join("%s %d" % r for r in rows)))
    if against:
        out.append("")
        out.append("%sPer-class deltas: %s -> %s" % (h, against, new_path))
        drows = []
        for c in sorted(set(nt) | set(ot)):
            drows.append((c, ot.get(c, 0), nt.get(c, 0), "%+d" % (nt.get(c, 0) - ot.get(c, 0))))
        out += table(md, ["verdict", "old", "new", "delta"], drows)
        changed = []
        for n in sorted(set(new) & set(old)):
            (on, ov, ox), (nn, nv, nx) = old[n], new[n]
            if (on, ov, ox) != (nn, nv, nx):
                name = on if on == nn else on + " -> " + nn
                text = "" if ox == nx else (ox or "(none)") + " -> " + (nx or "(none)")
                changed.append((n, name, ov if ov == nv else ov + " -> " + nv, text))
        out.append("")
        out.append("%sChanged cases: %d" % (h, len(changed)))
        out += table(md, ["case", "name", "verdict", "text"], changed)
        added = [(n,) + new[n] for n in sorted(set(new) - set(old))]
        gone = [(n,) + old[n] for n in sorted(set(old) - set(new))]
        out.append("")
        out.append("%sNew cases (only in %s): %d" % (h, new_path, len(added)))
        out += table(md, ["case", "name", "verdict", "text"], added)
        out.append("")
        out.append("%sRemoved cases (only in %s): %d" % (h, against, len(gone)))
        out += table(md, ["case", "name", "verdict", "text"], gone)
    print("\n".join(out))
    errors = nerr + oerr
    for e in errors:
        print("mali-sweep-tally: ERROR: " + e, file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
