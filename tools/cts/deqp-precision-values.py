#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""List the per-element values of the not-Pass dEQP precision cases.

    tools/cts/deqp-precision-values.py [--all-elements] [--max-samples N]
                                       [--case SUBSTR] [--include-pass] log...

Reads dEQP TestLog text: a .qpa file, or the text a board prints on its
console (driver/samples/zephyr/deqp, CONFIG_DEQP_ECHO_QPA: every qpa line is
printed as "qpa: <line>", a line longer than the sample's buffer continues on
"qpa+ <text>", which is appended to the line before it; a leading "HH:MM:SS "
time stamp, NULs and CRs are dropped, the sample's own "deqp: ..." lines are
ignored). The form is recognised by its "qpa: " lines.

For every case under builtin_functions.precision.* and
builtin_functions.common.modf.* whose result is not Pass (--include-pass: all;
--case SUBSTR: only names containing SUBSTR) it prints the case, its outcome
and, for each logged sample, the sample's own outcome, the input values and,
per element of each output, the value got and the expected interval. An
element is marked OUT when the value is not in the interval, NaN when it is a
NaN the interval does not allow; by default only those elements are listed
(--all-elements: every element). Nothing here says why a value is out.

The message format is dEQP's, VK-GL-CTS at 93bca01 (the commit
tools/cts/deps.toml pins), not guessed:

  modules/glshared/glsBuiltinPrecisionTests.cpp, PrecisionCase::testStatement
  (the source the AM62P notpass TSV cites as glsBuiltinPrecisionTests.cpp):
  per sample, a message only for a sample that is not Passed (at most 100 per
  case; a "(Skipped N messages.)" message follows), built as
      "<Passed|QualityWarning|Fail> sample:\\n"
      "\\t<inK name> = <valueToString(in)>\\n"          for each input
      "\\t<outK name> = <valueToString(out)>\\n"
      "\\tExpected range: <intervalToString(interval)>\\n"   per output
  (the second output, out1, is the Void of an ordinary function and the
  in/out parameter of modf). The case ends with "All N inputs passed." or
  "K/N inputs failed or had quality warnings.". A sample inside the warning
  band but outside the strict interval is "QualityWarning", outside both is
  "Fail". The same function in that file holds the Traits::doPrintValue and
  doPrintIVal printers: a vector or matrix prints as "<type>(<element>, ...)"
  (a column per element for a matrix), its interval as "(<iv>, ...)"; an int
  interval as "[lo, hi]"; a bool one as "{false}", "{true}" or both.

  framework/common/tcuFloatFormat.cpp, FloatFormat::floatToHex and
  intervalToHex: a float is "NaN", "+inf", "-inf", "0.0" (any zero, the
  sign is not printed) or "[-]0x1.<6 hex digits>p<exp>"; an interval is
  "{}" (empty), "{ NaN }", "{ x }", "{ NaN, x }", "<any>", or
  "[lo, hi]" with "{ NaN } | " in front when it also allows NaN.

I read both functions through a summarising fetch of the pinned sources, which
gave no line numbers: the file and function are cited, the lines are not. The selftest (tools/cts/deqp-precision-values-selftest.sh) runs hand-made
fixtures in this format; try a real log before relying on a detail this
header could not see.

A case with no Result (the log ends in it, or the next case begins first), or
whose message text is cut, is printed as TRUNCATED with the samples that were
whole. Exit status 0 once the logs were read, 2 on an unreadable file or
no case to report.
"""
import argparse
import html
import re
import sys

CASE_RE = re.compile(r"\.builtin_functions\.(precision|common\.modf)\.")
BEGIN = re.compile(r"^#beginTestCaseResult\s+(\S+)")
END = re.compile(r"^#(end|terminate)TestCaseResult")
RESULT = re.compile(r'<Result\s+StatusCode="([^"]*)"')
STAMP = re.compile(r"^\d\d:\d\d:\d\d ")
SAMPLE_HEAD = re.compile(r"^(Passed|QualityWarning|Fail) sample:$")


def read_lines(path):
    """The qpa lines of one log, console echo undone."""
    with open(path, encoding="utf-8", errors="replace") as f:
        raw = [STAMP.sub("", ln.replace("\0", "").rstrip("\r\n")) for ln in f]
    if not any(ln.startswith("qpa: ") for ln in raw):
        return raw
    out = []
    for ln in raw:
        if ln.startswith("qpa: "):
            out.append(ln[5:])
        elif ln.startswith("qpa+ ") and out:
            out[-1] += ln[5:]
    return out


def split_top(s):
    """Split on the commas outside (), [] and {}."""
    parts, depth, cur = [], 0, ""
    for c in s:
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        if c == "," and depth == 0:
            parts.append(cur.strip())
            cur = ""
        else:
            cur += c
    if cur.strip() or parts:
        parts.append(cur.strip())
    return parts


def parse_tree(s):
    """"vec2(a, b)" / "(a, b)" -> [a, b]; anything else is an atom string."""
    s = s.strip()
    m = re.match(r"^(\w*)\((.*)\)$", s, re.S)
    if m and (m.group(1) or s.startswith("(")):
        return [parse_tree(p) for p in split_top(m.group(2))]
    return s


def to_float(tok):
    tok = tok.strip()
    if tok in ("NaN", "nan"):
        return float("nan")
    if tok in ("+inf", "inf"):
        return float("inf")
    if tok == "-inf":
        return float("-inf")
    if tok in ("true", "false"):
        return tok
    try:
        return float.fromhex(tok) if "0x" in tok.lower() else float(tok)
    except ValueError:
        return None


def contains(iv, got):
    """True/False, or None when either text is not understood."""
    iv = iv.strip()
    g = to_float(got)
    if g is None:
        return None
    isnan = isinstance(g, float) and g != g
    if iv == "<any>":
        return True
    if iv == "{}":
        return False
    nan_ok = False
    if iv.startswith("{ NaN } | "):
        nan_ok, iv = True, iv[len("{ NaN } | "):]
    if iv == "{ NaN }":
        return isnan
    m = re.match(r"^\{ (NaN, )?(.*) \}$", iv)
    if m:
        if isnan:
            return bool(m.group(1))
        p = to_float(m.group(2))
        return None if p is None else p == g
    m = re.match(r"^\[(.*), (.*)\]$", iv)
    if m:
        lo, hi = to_float(m.group(1)), to_float(m.group(2))
        if lo is None or hi is None or isinstance(g, str):
            return None
        return nan_ok if isnan else lo <= g <= hi
    return None


def elements(got, exp, path=""):
    """Yield (path, got atom, expected atom) over two trees; a shape that
    differs yields the raw pair once."""
    if isinstance(got, list) and isinstance(exp, list) and len(got) == len(exp):
        for i, (g, e) in enumerate(zip(got, exp)):
            yield from elements(g, e, "%s[%d]" % (path, i))
    elif isinstance(got, str) and isinstance(exp, str):
        yield path, got, exp
    else:
        yield path, repr(got), repr(exp)


class Sample:
    def __init__(self, outcome):
        self.outcome = outcome
        self.inputs = []    # (name, value text)
        self.outputs = []   # (name, value text, interval text)
        self.cut = False


def parse_text(text, cut):
    """The samples of one <Text> body."""
    lines = text.split("\n")
    if not lines or not SAMPLE_HEAD.match(lines[0].rstrip()):
        return None
    s = Sample(SAMPLE_HEAD.match(lines[0].rstrip()).group(1))
    pending = None
    for ln in lines[1:]:
        if not ln.startswith("\t"):
            continue
        body = ln[1:]
        if body.startswith("Expected range: "):
            if pending is not None:
                s.outputs.append((pending[0], pending[1], body[len("Expected range: "):]))
                pending = None
            else:
                s.cut = True
            continue
        m = re.match(r"^(\w+) = (.*)$", body)
        if not m:
            s.cut = True
            continue
        if m.group(1).startswith("out"):
            pending = (m.group(1), m.group(2))
        else:
            s.inputs.append((m.group(1), m.group(2)))
    if pending is not None or cut:
        s.cut = True
    return s


class Case:
    def __init__(self, name):
        self.name = name
        self.status = None
        self.samples = []
        self.truncated = False
        self.note = ""


def read_cases(path):
    cases, cur = [], None
    text, intext = None, False

    def close_text(cut):
        nonlocal text, intext
        if cur is not None and text is not None:
            s = parse_text(html.unescape(text), cut)
            if s:
                cur.samples.append(s)
            if cut:
                cur.truncated = True
        text, intext = None, False

    def finish():
        nonlocal cur
        if cur is not None:
            if cur.status is None:
                cur.truncated = True
            cases.append(cur)
        cur = None

    for ln in read_lines(path):
        if intext:
            if "</Text>" in ln:
                text += "\n" + ln.split("</Text>", 1)[0]
                close_text(False)
            elif BEGIN.match(ln) or END.match(ln):
                close_text(True)
            else:
                text += "\n" + ln
                continue
            if not (BEGIN.match(ln) or END.match(ln)):
                continue
        m = BEGIN.match(ln)
        if m:
            finish()
            cur = Case(m.group(1))
            continue
        if cur is None:
            continue
        if END.match(ln):
            finish()
            continue
        r = RESULT.search(ln)
        if r:
            cur.status = r.group(1)
        if "<Text>" in ln:
            body = ln.split("<Text>", 1)[1]
            if "</Text>" in body:
                text = body.split("</Text>", 1)[0]
                close_text(False)
            else:
                text, intext = body, True
    if intext:
        close_text(True)
    finish()
    return cases


def report(case, all_elements, max_samples, out):
    out.write("case %s\n  outcome: %s\n" % (
        case.name, case.status if case.status else "none (no Result)"))
    if case.truncated:
        out.write("  TRUNCATED: the log ends before this case's result or message is whole\n")
    shown = case.samples if max_samples is None else case.samples[:max_samples]
    for n, s in enumerate(shown, 1):
        out.write("  sample %d: %s%s\n" % (n, s.outcome, " (cut)" if s.cut else ""))
        for name, val in s.inputs:
            out.write("    %s = %s\n" % (name, val))
        for name, val, iv in s.outputs:
            ge, ee = parse_tree(val), parse_tree(iv)
            bad = total = unk = 0
            lines = []
            for p, g, e in elements(ge, ee):
                total += 1
                ok = contains(e, g)
                gf = to_float(g)
                if ok is False:
                    bad += 1
                    tag = "NaN" if isinstance(gf, float) and gf != gf else "OUT"
                elif ok is None:
                    tag = "?"
                    unk += 1
                else:
                    tag = "ok"
                if all_elements or tag != "ok":
                    lines.append("    %s%s got %s expected %s  %s" % (name, p, g, e, tag))
            out.write("    %s: %d of %d element(s) out of bound%s\n" % (name, bad, total, ", %d not comparable" % unk if unk else ""))
            for ln in lines:
                out.write(ln + "\n")
    if len(case.samples) > len(shown):
        out.write("  (%d more sample(s) not shown)\n" % (len(case.samples) - len(shown)))
    if not case.samples and case.status not in (None, "Pass"):
        out.write("  no per-sample message in the log\n")


def main(argv):
    ap = argparse.ArgumentParser(description="per-element values of not-Pass dEQP precision cases")
    ap.add_argument("logs", nargs="+")
    ap.add_argument("--all-elements", action="store_true")
    ap.add_argument("--max-samples", type=int)
    ap.add_argument("--case", default="")
    ap.add_argument("--include-pass", action="store_true")
    a = ap.parse_args(argv)
    n = 0
    for path in a.logs:
        try:
            cases = read_cases(path)
        except OSError as e:
            sys.stderr.write("deqp-precision-values: %s\n" % e)
            return 2
        for c in cases:
            if not CASE_RE.search(c.name) or a.case not in c.name:
                continue
            if c.status == "Pass" and not c.truncated and not a.include_pass:
                continue
            n += 1
            report(c, a.all_elements, a.max_samples, sys.stdout)
    if n == 0:
        sys.stderr.write("deqp-precision-values: no precision/modf case that is not Pass\n")
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
