#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Draft cause table and issue list from NanoPC-T6 dEQP-GLES2 slices.

  python3 -I tools/cts/mali-fail-triage.py [--forecast NOTE] [--mali-src FILE]...
                                           SLICE...
  python3 -I tools/cts/mali-fail-triage.py --self-test

SLICE is a directory of the format tools/cts/mali-deqp-tally.py reads (its
docstring: meta.txt, chunks.tsv, results.tsv, kmsg/<label>.txt). The readers,
the rerun rule and the exit line split are that tool's, imported from it.

Prints markdown: one row per cause for every non-Pass case, and after the
table a draft issue (title and body) for each cause marked NEW. A case has
exactly one cause, the first that applies:

  1. a back end refusal: "refused <name>" in the detail, a MaliError or
     src/mali message in the detail, or first_refused= / "refused <name>" of
     the chunk's exit line. Row "refused <name>".
  2. a fragment loop of shaders.loops with a run-time bound in a hang, fault
     or crash chunk, or in a chunk whose kernel lines show a job timeout or a
     fault: the hang-risk of the forecast note's section 3.
  3. attribute_location / default_vertex_attrib with 'mat' in the name that
     fails: MATRIX_ATTRIB.
  4. Crash, Timeout, InternalError, ResourceError or a Lost (chunk-crash,
     chunk-hang) in a crash, hang or fault chunk: the chunk's kernel class
     (mali-kmsg.py: mmu_faults, fw_faults, gpu_faults, cs_faults, timeouts,
     resets, other_panthor, or none), row "<status> chunk: <class>".
  5. Lost (board-lost, not-run): "not run", no cause (the cases are rerun).
  6. NotSupported, QualityWarning, CompatibilityWarning: not a Fail, no cause.
  7. any other: "unexplained <status> in <group at depth 3>" (the three
     components after dEQP-GLES2 above the case).

Forecast column: a cause is known when docs/notes/mali-gles2-forecast.md
names it. Known are the cause keys and variant names of its section 2 table,
every CamelCase name in backticks in the prose of section 2, every MaliError
variant and `pub const` name of the --mali-src files (default
src/shaderc/online/mali/mod.rs and src/mali/*.rs), and any message ending
"is not lowered yet" or starting "mali frame:" or "mali:" (section 4 rule 1);
owner as the table says (plan #309 for the refusals). A cause no rule knows
is NEW and gets a draft issue. Text taken from slices has IP addresses and
user@host removed.

Exit 0, or 2 for a slice or note that cannot be read.
"""
import collections
import glob
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
FIXTURES = os.path.join(HERE, "testdata", "mali-fail-triage")
DEFAULT_FORECAST = os.path.join(ROOT, "docs", "notes", "mali-gles2-forecast.md")
FORECAST_NAME = "docs/notes/mali-gles2-forecast.md"
DEFAULT_SRC = [os.path.join(ROOT, "src", "shaderc", "online", "mali", "mod.rs")] + \
    sorted(glob.glob(os.path.join(ROOT, "src", "mali", "*.rs")))
KMSG_ORDER = ("mmu_faults", "fw_faults", "gpu_faults", "cs_faults", "timeouts",
              "resets", "other_panthor")
CHUNK_CAUSE_STATUS = ("Crash", "Timeout", "InternalError", "ResourceError")
NOT_FAIL = ("NotSupported", "QualityWarning", "CompatibilityWarning")
BAD_CHUNK = ("crash", "hang", "fault")
LOOP_RE = re.compile(r"shaders\.loops\..*(?:dynamic|uniform)_iterations.*_fragment")
MSG_RE = re.compile(r"(Valhall: .*?is not lowered yet|mali frame: .*|mali: .*)")
FIRST_REFUSED = re.compile(r"first_refused=(.*?)(?:\s+gpu_faults=\d+.*)?\s*$")
IPV4 = re.compile(r"\b\d{1,3}(?:\.\d{1,3}){3}(?::\d+)?\b")
USERHOST = re.compile(r"\b[\w.-]+@[\w.-]+\b")
MAX_EXAMPLES = 3


class Fatal(Exception):
    pass


def load_module(name, filename):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, filename))
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


tally = load_module("mali_deqp_tally", "mali-deqp-tally.py")
kmsg = load_module("mali_kmsg", "mali-kmsg.py")


def scrub(text):
    return USERHOST.sub("<redacted>", IPV4.sub("<redacted>", text))


def key(name):
    return re.sub(r"[^a-z0-9]", "", name.lower())


def read_text(path):
    try:
        with open(path, encoding="utf-8") as f:
            return f.read()
    except OSError as e:
        raise Fatal("%s: %s" % (path, e.strerror))


class Forecast:
    """What the forecast note and the back end sources already name."""

    def __init__(self, note_text, src_texts):
        self.names = {}     # key(name) -> (canonical name, owner)
        self.messages = []  # (message, canonical name, owner)
        sec = re.search(r"(?ms)^## 2\..*?(?=^## 3\.)", note_text)
        if not sec:
            raise Fatal("forecast note has no section 2")
        for ln in sec.group(0).splitlines():
            if ln.startswith("|"):
                self._row(ln)
                continue
            for n in re.findall(r"`([A-Z][A-Za-z0-9]+)`", ln):
                self.names.setdefault(key(n), (n, "plan #309"))
        for text in src_texts:
            m = re.search(r"(?ms)enum MaliError \{(.*?)^\}", text)
            if m:
                for n in re.findall(r"(?m)^\s{4}([A-Z][A-Za-z0-9]*)\b", m.group(1)):
                    self.names.setdefault(key(n), (n, "plan #309"))
            for n in re.findall(r"pub const ([A-Z][A-Z0-9_]+): &str", text):
                self.names.setdefault(key(n), (n, "plan #309"))

    def _row(self, line):
        cells = [c.strip() for c in re.split(r"(?<!\\)\|", line.strip().strip("|"))]
        if len(cells) < 5 or not cells[0].startswith("`"):
            return
        toks = [t.replace("MaliError::", "") for t in re.findall(r"`([A-Za-z_:]+)`", cells[0])]
        if not toks or not re.fullmatch(r"[A-Z][A-Z0-9_]+", toks[0]):
            return
        owner = cells[-1]
        canon = toks[0]
        for t in toks + re.findall(r"`(NOT_BUILT_[A-Z_]+)`", cells[1]):
            self.names[key(t)] = (canon, owner)
        m = re.match(r"`((?:[^`\\]|\\.)*)`", cells[1])
        if m:
            self.messages.append((m.group(1).replace("\\`", "`"), canon, owner))

    def lookup_name(self, name):
        return self.names.get(key(name))

    def lookup_message(self, msg):
        for text, canon, owner in self.messages:
            if text[:60] in msg or msg[:60] in text:
                return canon, owner
        if msg.endswith("is not lowered yet") or msg.startswith(("mali frame:", "mali:")):
            return None, "plan #309"
        return None


def refusal(detail, exit_line):
    """('name', text) or ('message', text) or None."""
    m = tally.REFUSED.search(detail)
    if m:
        return "name", m.group(1)
    m = MSG_RE.search(detail)
    if m:
        return "message", m.group(1).strip()
    m = FIRST_REFUSED.search(exit_line)
    if m and m.group(1) not in ("", "-"):
        v = m.group(1).strip()
        mm = MSG_RE.search(v)
        return ("message", mm.group(1).strip()) if mm else ("name", v)
    m = tally.REFUSED.search(exit_line)
    if m:
        return "name", m.group(1)
    return None


def group_of(case):
    parts = case.split(".")
    if parts and parts[0].startswith("dEQP-"):
        parts = parts[1:]
    parts = parts[:-1] or parts
    return ".".join(parts[:3])


def chunk_kmsg(s, label, row):
    path = os.path.join(s["path"], "kmsg", label + ".txt")
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            r = kmsg.classify(f.read().splitlines())
        return r, any(r[k] for k in KMSG_ORDER)
    except OSError:
        pass
    r = {k: 0 for k in KMSG_ORDER}
    r["first"] = "-"
    if row[6].isdigit() and int(row[6]):
        r["resets"] = int(row[6])
    elif row[5].isdigit() and int(row[5]):
        r["other_panthor"] = int(row[5])
    return r, any(r[k] for k in KMSG_ORDER)


def kmsg_class(r):
    for k in KMSG_ORDER:
        if r[k]:
            return k
    return "none"


def chunk_of_results(s):
    """Per result index: (chunk row fields, label) or None."""
    out = [None] * len(s["results"])
    counts = [int(f[2]) if f[2].isdigit() else 0 for _n, f, _w in s["chunks"]]
    if sum(counts) != len(out):
        return out
    pos = 0
    for (_n, f, _w), c in zip(s["chunks"], counts):
        for i in range(pos, pos + c):
            out[i] = f
        pos += c
    return out


class Cause:
    def __init__(self, label, kind, known, owner):
        self.label = label
        self.kind = kind
        self.known = known
        self.owner = owner
        self.cases = 0
        self.examples = []
        self.slices = set()
        self.commits = set()
        self.kernels = set()
        self.details = collections.Counter()
        self.first_kmsg = ""
        self.status = collections.Counter()


def triage(slices, fc):
    ordered, superseded, _dups, _errs = tally.resolve(slices)
    causes = {}

    def add(label, kind, known, owner, s, case, status, detail, first=""):
        c = causes.get(label)
        if c is None:
            c = causes[label] = Cause(label, kind, known, owner)
        c.cases += 1
        if len(c.examples) < MAX_EXAMPLES:
            c.examples.append(case)
        c.slices.add(s["name"])
        c.commits.add(s["meta"].get("commit", "")[:8])
        c.kernels.add(s["meta"].get("kernel", ""))
        c.status[status] += 1
        c.details[tally.normalize(detail)] += 1
        if first and not c.first_kmsg:
            c.first_kmsg = first

    for s in ordered:
        chunk_rows = chunk_of_results(s)
        km = {}
        for i, (_ln, f, _w) in enumerate(s["results"]):
            if i in superseded[s["name"]] or f[1] == "Pass":
                continue
            case, status, detail = f
            row = chunk_rows[i]
            exit_line = "" if row is None or row[9] == "-" else row[9]
            cstatus = row[3] if row else "done"
            if row and row[1] not in km:
                km[row[1]] = chunk_kmsg(s, row[1], row)
            r, noisy = km.get(row[1], (None, False)) if row else (None, False)
            first = r["first"] if r and r["first"] != "-" else ""
            ref = refusal(detail, exit_line)
            if ref:
                kind, text = ref
                if kind == "name":
                    hit = fc.lookup_name(text)
                    label = "refused " + (hit[0] if hit else text)
                    add(label, "refusal", bool(hit), hit[1] if hit else "", s, case,
                        status, detail)
                else:
                    hit = fc.lookup_message(text)
                    if hit and hit[0]:
                        label = "refused " + hit[0]
                    else:
                        label = "refused: " + (text if len(text) <= 70 else text[:67] + "...")
                    add(label, "refusal", bool(hit), hit[1] if hit else "", s, case,
                        status, detail)
                continue
            if LOOP_RE.search(case) and (cstatus in ("hang", "fault") or
                                         (r and (r["timeouts"] or r["mmu_faults"]
                                                 or r["gpu_faults"] or r["cs_faults"]
                                                 or r["fw_faults"] or r["resets"]))):
                add("hang-risk: fragment loop with a run-time bound", "loop", True,
                    "forecast section 3 (loop work)", s, case, status, detail, first)
                continue
            g = group_of(case)
            if status == "Fail" and re.match(r"functional\.(attribute_location|default_vertex_attrib)\.", g + ".") \
                    and "mat" in case.split(".")[-1]:
                hit = fc.lookup_name("MATRIX_ATTRIB")
                add("MATRIX_ATTRIB", "matrix", True, hit[1] if hit else "see forecast",
                    s, case, status, detail)
                continue
            lost_chunk = status == "Lost" and detail.strip() in ("chunk-crash", "chunk-hang")
            if cstatus in BAD_CHUNK and (status in CHUNK_CAUSE_STATUS or lost_chunk):
                add("%s chunk: %s" % (cstatus, kmsg_class(r) if r else "none"), "kmsg",
                    False, "", s, case, status, detail, first)
                continue
            if status == "Lost":
                add("not run: %s" % (detail.strip() or "-"), "notrun", True,
                    "none (rerun the cases)", s, case, status, detail)
                continue
            if status in NOT_FAIL:
                add("%s in %s" % (status, g), "notfail", True,
                    "none (not a Fail)", s, case, status, detail)
                continue
            add("unexplained %s in %s" % (status, g), "fail", False, "", s, case,
                status, detail)
    return causes


def cell(text):
    return text.replace("|", "\\|")


def render(slices, causes):
    out = ["# Mali dEQP-GLES2 failure triage (draft)", ""]
    out.append("Slices: " + ", ".join("`%s`" % s["name"] for s in
                                      sorted(slices, key=lambda x: x["name"])) + ".")
    out.append("Forecast: `%s`." % FORECAST_NAME)
    out += ["", "| Cause | Cases | Examples | Slices | Forecast | Owner |",
            "| --- | ---: | --- | --- | --- | --- |"]
    ordered = sorted(causes.values(), key=lambda c: (-c.cases, c.label))
    for c in ordered:
        new = c.kind in ("kmsg", "fail", "refusal") and not c.known
        out.append("| %s | %d | %s | %s | %s | %s |" % (
            cell(c.label), c.cases,
            ", ".join("`%s`" % scrub(x) for x in c.examples),
            ", ".join("`%s`" % n for n in sorted(c.slices)),
            "**NEW**" if new else ("yes" if c.known else "no"),
            cell(c.owner if c.known else "-")))
    out += ["", "Non-Pass cases: %d." % sum(c.cases for c in ordered), ""]
    news = [c for c in ordered if c.kind in ("kmsg", "fail", "refusal") and not c.known]
    out.append("## Draft issues")
    out.append("")
    if not news:
        out += ["No NEW cause.", ""]
    for c in news:
        out += draft_issue(c)
    return "\n".join(out)


def plural(n):
    return "%d case%s" % (n, "" if n == 1 else "s")


def draft_issue(c):
    if c.kind == "refusal":
        title = "Mali dEQP-GLES2: refusal %s is not in the forecast (%s)" % (
            c.label.split(" ", 1)[1], plural(c.cases))
        how = "The back end refused the draw or the link with a name or message " \
              "the forecast note does not list."
    elif c.kind == "kmsg":
        title = "Mali dEQP-GLES2: %s (%s)" % (c.label, plural(c.cases))
        how = "The chunk ended in %s and its kernel lines are of class %s." % (
            c.label.split(" chunk: ")[0], c.label.split(": ")[-1])
    else:
        title = "Mali dEQP-GLES2: %s (%s)" % (c.label, plural(c.cases))
        how = "A non-Pass with no refusal message, outside the forecast's causes."
    body = ["### Draft issue: %s" % scrub(title), "",
            "Title: %s" % scrub(title), "", "Body:", "",
            "- Cause: %s" % scrub(c.label),
            "- Cases: %d (%s)" % (c.cases, ", ".join(
                "%s %d" % (k, v) for k, v in sorted(c.status.items()))),
            "- Examples: " + ", ".join("`%s`" % scrub(x) for x in c.examples),
            "- Slices: " + ", ".join("`%s`" % n for n in sorted(c.slices)),
            "- Commits: " + ", ".join("`%s`" % x for x in sorted(c.commits)),
            "- Kernels: " + ", ".join(sorted(k for k in c.kernels if k)),
            "- Detail (numbers and names stripped): " +
            "; ".join("`%s` x%d" % (scrub(d), n) for d, n in c.details.most_common(3)),
            "- How found: " + how]
    if c.first_kmsg:
        body.append("- First kernel line: `%s`" % scrub(c.first_kmsg))
    body += ["- Forecast: not named in `%s`; add it to section 2 when the issue is open."
             % FORECAST_NAME, ""]
    return body


def run(argv):
    forecast = DEFAULT_FORECAST
    src = None
    args = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in ("--forecast", "--mali-src"):
            i += 1
            if i >= len(argv):
                raise Fatal("%s needs a file" % a)
            if a == "--forecast":
                forecast = argv[i]
            else:
                src = (src or []) + [argv[i]]
        elif a.startswith("--"):
            raise Fatal("unknown option " + a)
        else:
            args.append(a)
        i += 1
    if not args:
        return None
    srcs = []
    for p in (src if src is not None else DEFAULT_SRC):
        try:
            srcs.append(read_text(p))
        except Fatal:
            if src is not None:
                raise
    fc = Forecast(read_text(forecast), srcs)
    try:
        slices = [tally.load_slice(p) for p in args]
    except tally.Fatal as e:
        raise Fatal(str(e))
    return render(slices, triage(slices, fc)) + "\n", slices


def self_test():
    n_ok = n_bad = 0

    def chk(label, cond):
        nonlocal n_ok, n_bad
        if cond:
            n_ok += 1
            print("  ok    " + label)
        else:
            n_bad += 1
            print("  FAIL  " + label)

    fx = FIXTURES
    base = ["--forecast", os.path.join(fx, "forecast.md"), "--mali-src",
            os.path.join(fx, "mali-src.rs")]
    sl = sorted(os.path.join(fx, "slices", d) for d in os.listdir(os.path.join(fx, "slices")))
    out, _s = run(base + sl)
    exp = read_text(os.path.join(fx, "expected.md"))
    chk("fixture output equals expected.md", out == exp)
    chk("forecast refusal known (ARITH, owner plan #309)",
        "| refused ARITH | 3 |" in out and "| plan #309 |" in out)
    chk("sibling variant in the note's prose is known",
        re.search(r"\| refused MixedPrecision \|.*\| yes \| plan #309 \|", out) is not None)
    chk("MaliError variant of the source is known",
        re.search(r"\| refused CompareForm \|.*\| yes \|", out) is not None)
    chk("message-only refusal is known",
        re.search(r"\| refused: Valhall: a made-up form is not lowered yet \|.*\| yes \|", out)
        is not None)
    chk("frame message maps to its table row",
        re.search(r"\| refused BLEND_SHADER \|.*\| yes \|", out) is not None)
    chk("exit line first_refused= is read",
        re.search(r"\| refused TEX_KIND \|", out) is not None)
    chk("unforecast refusal is NEW",
        re.search(r"\| refused blend_equation_separate \|.*\*\*NEW\*\*", out) is not None)
    chk("loop case in a fault chunk is the hang-risk, not NEW",
        re.search(r"\| hang-risk: fragment loop with a run-time bound \| 2 \|.*\| yes \|", out)
        is not None)
    chk("fault chunk row carries the kmsg class and is NEW",
        re.search(r"\| fault chunk: mmu_faults \|.*\*\*NEW\*\*", out) is not None)
    chk("plain Fail names its group at depth 3",
        "unexplained Fail in functional.fragment_ops.blend" in out)
    chk("MATRIX_ATTRIB is known", re.search(r"\| MATRIX_ATTRIB \|.*\| yes \|", out) is not None)
    chk("one draft issue per NEW row",
        out.count("### Draft issue:") == out.count("**NEW**"))
    chk("no address or user@host in the draft",
        not IPV4.search(out) and not USERHOST.search(out))
    chk("scrub removes an address and a login",
        scrub("ssh root@nano at 10.1.2.3:22") == "ssh <redacted> at <redacted>")
    good = sorted(os.path.join(tally.FIXTURES, "good", d)
                  for d in os.listdir(os.path.join(tally.FIXTURES, "good")))
    out2, s2 = run(base + good)
    by_status, _bc = tally.tally(s2)
    want = sum(n for st, n in by_status.items() if st != "Pass")
    got = 0
    for ln in out2.splitlines():
        cells = re.split(r"(?<!\\) \| ", ln)
        if ln.startswith("| ") and len(cells) == 6 and cells[1].isdigit():
            got += int(cells[1])
    chk("mali-deqp/good: table sum equals the tally's non-Pass (%d)" % want, got == want)
    out3, _s = run(good)
    chk("default forecast note and sources load", "## Draft issues" in out3)
    print("%d checks passed" % n_ok if not n_bad else
          "%d checks passed, %d FAILED" % (n_ok, n_bad))
    return 1 if n_bad else 0


def main(argv):
    try:
        if argv == ["--self-test"]:
            return self_test()
        r = run(argv)
    except Fatal as e:
        print("mali-fail-triage: %s" % e, file=sys.stderr)
        return 2
    if r is None:
        print(__doc__, file=sys.stderr)
        return 2
    sys.stdout.write(r[0])
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
