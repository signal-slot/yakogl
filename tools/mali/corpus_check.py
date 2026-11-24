#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The Mali compiler against Mesa 26.2.1 over whole corpora: the progress meter.

    python3 -I tools/mali/corpus_check.py run [--select S]... [--out ROWS] [--cache DIR]
            [--jobs N] [--sample N [--seed K]] [--deqp DIR] [--deqp-cases SPEC] [--cts DIR]
            [--glmark2 DIR]
    python3 -I tools/mali/corpus_check.py summary ROWS [--board-refused LIST]
    python3 -I tools/mali/corpus_check.py diff OLD_ROWS NEW_ROWS

`run` measures every program of the selection (S, repeatable, default all):

  deqp       every dEQP-GLES2 must-pass case (tools/cts/gles2-mustpass.txt):
             the programs deqp-gles2 compiles on this host against the Mali
             back end's library (tools/mali/deqp-host-measure.sh --cases),
             read from the run's .qpa; a case whose first program is refused
             shows only the programs dEQP made before it stopped. The run
             covers the whole list; --deqp-cases keeps `slices` (the cases
             the board refused, summary --board-refused) or a list's cases
  testdata   every tools/mali/testdata/<name>.vert beside a <name>.frag,
             against its committed <name>.expect.json under the rule
             `either` (tools/mali/route.sh measure); one without is drawn
  glmark2    the programs of glmark2's 33 default scenes
             (tests/mali_glmark2_corpus.rs PROGRAMS over the corpus of
             tools/hostharness/glmark2/run.sh: --glmark2, GLMARK2_CORPUS, or
             build/glmark2-host/shaders)

Per program, our side is the integration tests' measure
(tests/mali_ref/mod.rs `measure_list`: glCompileShader of each stage, then
glLinkProgram's two steps, compared by `differences`: words, work_reg_count,
preload, FAU words, the varying program and its offset). Mesa's side is a
drm-shim draw (tools/mali/drm_shim_ref.sh run --uniform-values, then
pan_ref_diff.py extract --draw), the reference tools/mali/deqp_ref.sh makes.
One row per (program, stage) goes to ROWS (default <cache>/rows-<commit>.tsv):

  key group stage ours ours_text mesa verdict class detail program source

  ours      compiled | refused        mesa  ok | fail | - (not drawn)
  verdict   EQUAL | DIFFER (compiled, both stages compared) | REFUSED (this
            stage, by its compile or the link step its log names) | BLOCKED
            (compiled, the program refused at the other stage) | REJECTED
            (refused, not by the code generator, and Mesa refuses the program
            too: dEQP's negative cases) | NOREF
            (compiled, Mesa drew nothing) | ERROR (the comparison could not
            read the reference) | CRASH (the measure's process died) | SINGLE
            (a lone shader of a dEQP case: compiled, no program to compare)
            | NOSHADER / NOTRUN (a dEQP case with no program, or not run)
  class     the reason class of a refusal (docs/notes/mali-compiler-road-to-es2.md
            section 1.1), or the kind of the first difference (words, fau,
            work_reg_count, preload, varyings, padding)
  program   the first 16 hex digits of the program's content hash; source,
            of this stage's shader text: the distinct counts of the summary

Caches under --cache (default $YAKOGL_MALI_CORPUS_CACHE, else
~/.cache/yakogl-gate/mali-corpus; never in the tree): the sources by content
hash, Mesa's references by content hash and the recipe (the hashes of
gl_ref.py, pan_ref_diff.py and drm_shim_ref.sh), our rows by the tree key of
the compiler (src/shaderc, src/mali, src/util, tests/mali_ref, Cargo.*) and
the program, and the dEQP run by the tree key of the library. A rerun on an
unchanged compiler measures nothing again; a compiler change remeasures our
side only, a minute or two for the whole corpus.

`summary` prints the tables (counts by verdict, by refusal text, by reason
class, by difference class, by dEQP group; dEQP cases by their worst program;
distinct programs and shader sources behind the counts). With
--board-refused, the dEQP case tables again for those cases: `slices` is the
cases the board refused to compile in the 37305608 slices (the population of
deqp-host-measure.sh and of the road study), else a case list file.

`diff` prints what moved between two row files per (key, stage) and per dEQP
case, and lists every regression: a row EQUAL in OLD that is DIFFER or
REFUSED in NEW. Exit 1 when there is one, 0 otherwise, 2 on a malformed file.

Environment: MESA_SHIM_WORK (drm_shim_ref.sh's work dir, default
$CONNECTED_DEPS_DIR/mesa-shim), DEQP_WORK or --cts (the VK-GL-CTS checkout
deqp-host-measure.sh builds deqp-gles2 from).
"""
import argparse
import collections
import concurrent.futures
import hashlib
import html
import json
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
FEATURES = "host-test,gpu-mali,shaderc-online,no-shader-table"
ROWS_HEADER = "# corpus_check rows v1"
COLUMNS = ("key", "group", "stage", "ours", "ours_text", "mesa", "verdict", "class", "detail", "program", "source")
VERDICTS = ("EQUAL", "DIFFER", "REFUSED", "REJECTED", "BLOCKED", "NOREF", "ERROR", "CRASH", "SINGLE", "NOSHADER",
            "NOTRUN")
# a dEQP case's outcome is its worst program-stage row, in this order
CASE_ORDER = ("REFUSED", "CRASH", "ERROR", "DIFFER", "NOREF", "EQUAL", "REJECTED", "SINGLE", "NOSHADER", "NOTRUN")
COMPILER_PATHS = ("src/shaderc", "src/mali", "src/util", "tests/mali_ref", "tests/mali_ref_selftest.rs",
                  "Cargo.toml", "Cargo.lock")
LIBRARY_PATHS = ("src", "capi", "include", "Cargo.toml", "Cargo.lock", "tools/mali/deqp-host-measure.sh")
RECIPE_FILES = ("gl_ref.py", "pan_ref_diff.py", "drm_shim_ref.sh")
# a partner for a lone shader of a dEQP case (shader_api and the like): the
# program exists only so that the lone stage's own compile is measured
LONE_VS = "attribute highp vec4 a_position;\nvoid main() { gl_Position = a_position; }\n"
LONE_FS = "void main() { gl_FragColor = vec4(1.0); }\n"

# ---- the reason classes of the code generator's refusals -------------------
# docs/notes/mali-compiler-road-to-es2.md section 1.1 (the variants of
# MaliError) and 2.1 (the link part of "order" and Defect(IrIndex) apart).
CLASSES = collections.OrderedDict([
    ("missing operation", "Arithmetic TexKind UniformIndexed FragOutput NoColour IndexedLocal IfForm "
                          "ConvertWiden TexVertex SpecialInput TexArrayElement Interpolation NoPosition "
                          "LinkedVertexNotLowered"),
    ("reference missing", "ColourForm FsOperandForm TexCoord VaryingForm TexLanes ColourPair TexCoordShared "
                          "ColourZeroHalf CompareForm"),
    ("type domain", "IntValue UniformType UniformArray IntColour FsUniform"),
    ("order (algebraic, fold)", "ArithConstant ConstantFold Rebalance AlgebraicForm NegZeroShared "
                                "FusedMultiplyAdd AttributeNumbering"),
    ("order (link)", "VaryingNotFromAttribute VaryingDuplicate PositionLane"),
    ("control flow", "LoopForm ControlFlow LoopVertex DiscardForm GuardMediump LoopMediump"),
    ("precision", "Fp16Inexact FragCoordMediump FsVaryingLoad MixedPrecision UniformPrecision TexHighp"),
    ("deliberate guard", "BlendVariantNotLowered Invariant"),
    ("registers and size", "VaryingLocation TooManyValues BlockTableFull TexCount FragmentTooLarge UboRowsFull "
                           "MovedTooLarge Spill FauFull TooManyLocals TooManyConstants InstrTableFull "
                           "ValueTableFull UboFallback TooManyUniforms OutOfScratch"),
])
CLASS_OF = {v: c for c, vs in CLASSES.items() for v in vs.split()}
REFUSED_TEXT = "online shaderc: the code generator refused this"


def message_table(mod_rs):
    """{message text: variant} of `MaliError::message` in mod.rs's source."""
    try:
        with open(mod_rs, encoding="utf-8") as f:
            src = f.read()
    except OSError:
        return {}
    m = re.search(r"pub const fn message\(self\) -> &'static str \{(.*?)\n    \}\n", src, re.S)
    body = m.group(1) if m else ""
    out = {}
    for v, text in re.findall(r'MaliError::(\w+) => \{?\s*"((?:[^"\\]|\\.)*)"', body):
        out[text.replace('\\"', '"')] = v
    return out


def short_text(log):
    """The code generator's message of a refusal log (`MaliError::message`,
    tests/mali_ref `message`), else the log after its step prefix."""
    log = re.sub(r"^(vs|fs|vs_linked|link) \(-?\d+\): ", "", log)
    m = re.search(r"refused this (?:vertex|fragment) shader: Mali\((.*)\)\s*$", log)
    return m.group(1) if m else log


def variant_of(text, table):
    """The MaliError variant of a refusal text, `Defect(<kind>)`, or ''."""
    m = re.search(r"back end defect: an invariant of the pipeline did not hold \((\w+)", text)
    if m:
        return "Defect(%s)" % m.group(1)
    for msg, v in table.items():
        if msg and msg in text:
            return v
    return ""


def class_of_refusal(text, table):
    if text.startswith("panic:"):
        return "defect"
    v = variant_of(text, table)
    if v == "Defect(IrIndex)":
        return "type domain, through a defect"
    if v.startswith("Defect("):
        return "defect"
    if v:
        return CLASS_OF.get(v, "unclassed:" + v)
    return "not the code generator"


def class_of_difference(detail):
    """The kind of a first difference of tests/mali_ref `differences`."""
    for pat, cls in ((r"work_reg_count ours", "work_reg_count"), (r"preload ours", "preload"),
                     (r"FAU words ours", "fau"), (r"secondary_offset|varying program", "varyings"),
                     (r"words with padding", "padding"), (r"words ours, \d+ reference", "words")):
        if re.search(pat, detail):
            return cls
    return "other"


# ---- the dEQP .qpa: per case its programs ----------------------------------
# tools/mali/deqp-host-measure.sh's reading: glu::ShaderProgram's
# <ShaderProgram> with <VertexShader>/<FragmentShader>, or the
# <Section Name="Vertex Shader Info"> form of attribute_location, shader_api.
RE_CASE = re.compile(r"^#beginTestCaseResult (\S+)", re.M)
RE_UNIT = re.compile(
    r'<ShaderProgram\b[^>]*>\s*(?:<InfoLog>(?P<plog>.*?)</InfoLog>)?(?P<p>)'
    r'|<(?P<a>Vertex|Fragment)Shader\b[^>]*>(?P<ab>.*?)</(?P=a)Shader>'
    r'|(?P<e></ShaderProgram>)'
    r'|<Section Name="(?P<b>Vertex|Fragment) Shader Info"[^>]*>\s*<KernelSource>(?P<bsrc>.*?)</KernelSource>'
    r'|<Section Name="Program Link Info"[^>]*>(?P<d>)',
    re.S)
RE_SOURCE = re.compile(r"<ShaderSource>(.*?)</ShaderSource>", re.S)


def programs_of(block):
    """[(vs, fs), ...] of one case's .qpa block, in order; a lone shader has
    '' for the other stage."""
    programs = []
    cur = {"vs": "", "fs": ""}

    def flush():
        if cur["vs"] or cur["fs"]:
            programs.append((cur["vs"], cur["fs"]))
        cur["vs"] = cur["fs"] = ""
    for u in RE_UNIT.finditer(block):
        if u.group("p") is not None or u.group("e") or u.group("d") is not None:
            flush()
        elif u.group("a"):
            st = "vs" if u.group("a") == "Vertex" else "fs"
            ms = RE_SOURCE.search(u.group("ab"))
            if cur[st]:
                flush()
            cur[st] = html.unescape(ms.group(1)) if ms else ""
        elif u.group("b"):
            st = "vs" if u.group("b") == "Vertex" else "fs"
            if cur[st]:
                flush()
            cur[st] = html.unescape(u.group("bsrc"))
    flush()
    return programs


def read_qpa(text):
    """{case: [(vs, fs), ...]} of one .qpa."""
    parts = RE_CASE.split(text)
    return {parts[i]: programs_of(parts[i + 1]) for i in range(1, len(parts), 2)}


def deqp_group(case):
    return ".".join(case.split(".")[2:4])


# ---- glmark2's programs -----------------------------------------------------
def glmark2_programs(test_rs):
    """[(scene number, entry, [vs4+fs4, ...]), ...] of tests/mali_glmark2_corpus.rs."""
    with open(test_rs, encoding="utf-8") as f:
        src = f.read()
    ent = re.search(r"const ENTRIES: \[&str; (\d+)\] = \[(.*?)\];", src, re.S)
    prog = re.search(r"const PROGRAMS: \[&\[&str\]; (\d+)\] = \[(.*?)\n\];", src, re.S)
    if not ent or not prog:
        raise ValueError("%s: no ENTRIES or PROGRAMS table" % test_rs)
    entries = re.findall(r'"([^"]*)"', ent.group(2))
    lists = [re.findall(r'"([0-9a-f]{4}\+[0-9a-f]{4})"', x) for x in re.findall(r"&\[(.*?)\]", prog.group(2), re.S)]
    if len(entries) != int(ent.group(1)) or len(lists) != len(entries):
        raise ValueError("%s: %d entries, %d program lists" % (test_rs, len(entries), len(lists)))
    return [(i, e, l) for i, (e, l) in enumerate(zip(entries, lists))]


def glmark2_files(corpus):
    """{4 hex digits: path} of a glmark2 corpus directory (its index.txt)."""
    out = {}
    with open(os.path.join(corpus, "index.txt"), encoding="utf-8") as f:
        for line in f:
            if line.startswith("#") or ":" not in line:
                continue
            name = line.split(":", 1)[0]
            m = re.search(r"-([0-9a-f]{16})\.(vert|frag)\.glsl$", name)
            if m:
                out[(m.group(1)[:4], m.group(2))] = os.path.join(corpus, name)
    return out


# ---- keys -------------------------------------------------------------------
def sha(*parts):
    h = hashlib.sha256()
    for p in parts:
        h.update(p.encode("utf-8", "surrogateescape") if isinstance(p, str) else p)
        h.update(b"\0")
    return h.hexdigest()


def program_key(vs, fs):
    return sha("program", vs, fs)


def tree_key(repo, paths):
    """The content of `paths` in the work tree, tracked and untracked
    (ignored files aside): the same before and after a commit of it."""
    files = subprocess.run(["git", "-C", repo, "ls-files", "-co", "--exclude-standard", "-z", "--", *paths],
                           capture_output=True, check=True).stdout.split(b"\0")
    files = sorted(f for f in files if f and os.path.isfile(os.path.join(repo, f.decode())))
    blobs = subprocess.run(["git", "-C", repo, "hash-object", "--stdin-paths"], input=b"\n".join(files) + b"\n",
                           capture_output=True, check=True).stdout
    return sha(b"\n".join(files), blobs)[:16]


def describe(repo):
    r = subprocess.run(["git", "-C", repo, "describe", "--always", "--dirty"], capture_output=True, text=True)
    return r.stdout.strip() or "unknown"


def recipe_key():
    parts = []
    for n in RECIPE_FILES:
        with open(os.path.join(HERE, n), "rb") as f:
            parts.append(f.read())
    return sha("mesa-26.2.1", *parts)[:16]


# ---- the rows file ------------------------------------------------------------
def read_rows(path):
    """(header lines, [row dict, ...]) of a rows file; ValueError on a bad line."""
    head, rows = [], []
    with open(path, encoding="utf-8", errors="replace") as f:
        for n, line in enumerate(f, 1):
            line = line.rstrip("\n")
            if not line:
                continue
            if line.startswith("#"):
                head.append(line)
                continue
            parts = line.split("\t")
            if len(parts) != len(COLUMNS) or parts[6] not in VERDICTS:
                raise ValueError("%s:%d: not a corpus_check row: %r" % (path, n, line[:120]))
            rows.append(dict(zip(COLUMNS, parts)))
    if not head or not head[0].startswith(ROWS_HEADER):
        raise ValueError("%s: no '%s' header" % (path, ROWS_HEADER))
    return head, rows


def flat(s):
    return re.sub(r"[\t\r\n]", " ", s or "") or "-"


def write_rows(path, head, rows):
    with open(path + ".tmp", "w", encoding="utf-8") as f:
        for h in head:
            f.write(h + "\n")
        for r in rows:
            f.write("\t".join(flat(r[c]) for c in COLUMNS) + "\n")
    os.replace(path + ".tmp", path)


def stage_rows(key, group, measure, mesa, prog, table, lone="", sources=("-", "-")):
    """The two (or, for a lone shader, one) rows of a program from its
    measure fields (tests/mali_ref `measure_row` after the key)."""
    _, _, _, vv, vd, fv, fd = measure
    out = []
    for stage, v, d, src in (("vs", vv, vd, sources[0]), ("fs", fv, fd, sources[1])):
        if lone and stage != lone:
            continue
        ours = "refused" if v in ("REFUSED", "CRASH") else "compiled"
        text = short_text(d) if v in ("REFUSED", "CRASH") else "-"
        if v == "REFUSED":
            cls = class_of_refusal(text, table)
        elif v == "DIFFER":
            cls = class_of_difference(d)
        elif v == "ERROR":
            cls = "harness"
        else:
            cls = "-"
        detail = d
        if v == "REFUSED":
            detail = variant_of(text, table) or "-"
        if v == "REFUSED" and cls == "not the code generator" and mesa == "fail":
            v = "REJECTED"  # the front end or the link refuses, and so does Mesa (a negative case)
        if lone and v != "REFUSED" and v != "CRASH":
            v, cls, detail, m = "SINGLE", "-", "-", "-"
        else:
            m = mesa
        out.append(dict(key=key, group=group, stage=stage, ours=ours, ours_text=text, mesa=m,
                        verdict=v, cls_=cls, detail=detail, program=prog[:16], source=src[:16]))
    for r in out:
        r["class"] = r.pop("cls_")
    return out


# ---- the summary -------------------------------------------------------------
def case_outcomes(rows):
    """{case: (outcome, first refusal text or '')} of the dEQP rows."""
    by = collections.OrderedDict()
    for r in rows:
        if r["group"].startswith(("testdata", "glmark2")):
            continue
        case = r["key"].split("#")[0]
        by.setdefault(case, []).append(r)
    out = collections.OrderedDict()
    for case, rs in by.items():
        verdicts = [r["verdict"] for r in rs]
        outcome = min(verdicts, key=lambda v: CASE_ORDER.index(v) if v in CASE_ORDER else 99)
        first = next((r for r in rs if r["verdict"] == "REFUSED"), None)
        out[case] = (outcome, first)
    return out


def table(title, counter, total=None, n=None):
    lines = ["", "| %s | Count |" % title, "|---|--:|"]
    items = counter.most_common(n)
    for k, v in items:
        lines.append("| %s | %d |" % (k, v))
    if n and len(counter) > n:
        lines.append("| (%d more) | %d |" % (len(counter) - n, sum(v for k, v in counter.most_common()[n:])))
    if total is not None:
        lines.append("| **total** | **%d** |" % total)
    return lines


def summary(rows, board=None):
    out = []
    sel = collections.Counter("deqp" if not r["group"].startswith(("testdata", "glmark2")) else
                              r["group"].split(".")[0] for r in rows)
    out.append("Rows: %s." % ", ".join("%s %d" % kv for kv in sorted(sel.items())))
    for name in ("deqp", "testdata", "glmark2"):
        rs = [r for r in rows if (r["group"].split(".")[0] if name != "deqp" else
                                  ("deqp" if not r["group"].startswith(("testdata", "glmark2")) else ""))
              == name]
        if not rs:
            continue
        out.append("")
        out.append("### %s: program stages" % name)
        out.append("")
        out.append("| Verdict | vs | fs | distinct programs | distinct sources |")
        out.append("|---|--:|--:|--:|--:|")
        for v in VERDICTS:
            vs = [r for r in rs if r["verdict"] == v and r["stage"] == "vs"]
            fs = [r for r in rs if r["verdict"] == v and r["stage"] == "fs"]
            if not vs and not fs:
                continue
            progs = len({r["program"] for r in vs + fs})
            srcs = len({r["source"] for r in vs + fs})
            out.append("| %s | %d | %d | %d | %d |" % (v, len(vs), len(fs), progs, srcs))
        prog = collections.defaultdict(set)
        for r in rs:
            prog[r["program"]].add(r["verdict"])
        out.append("")
        out.append("Distinct programs: %d (both stages EQUAL: %d; a stage DIFFER: %d; a stage REFUSED: %d)." % (
            len(prog), sum(1 for v in prog.values() if v == {"EQUAL"}),
            sum(1 for v in prog.values() if "DIFFER" in v), sum(1 for v in prog.values() if "REFUSED" in v)))
        out += table("%s: difference class (stage rows)" % name,
                     collections.Counter(r["class"] for r in rs if r["verdict"] == "DIFFER"))
        out += table("%s: refusal reason class (stage rows)" % name,
                     collections.Counter(r["class"] for r in rs if r["verdict"] == "REFUSED"))
        out += table("%s: refusal text (stage rows, first 25)" % name,
                     collections.Counter("%s %s" % (r["stage"], r["ours_text"][:110])
                                         for r in rs if r["verdict"] == "REFUSED"), n=25)
    cases = case_outcomes(rows)
    if cases:
        out += case_tables("dEQP-GLES2 must-pass cases", cases, rows)
        if board is not None:
            out += case_tables("the board-refused cases", collections.OrderedDict(
                (c, o) for c, o in cases.items() if c in board), rows)
    return "\n".join(out) + "\n"


def case_tables(title, cases, rows):
    out = ["", "### %s: by the case's worst program" % title, ""]
    oc = collections.Counter(o for o, _ in cases.values())
    compiled = sum(oc[v] for v in ("EQUAL", "DIFFER", "NOREF", "ERROR"))
    out.append("%d cases: %s. Compiled (every program): %d, of which EQUAL %d (%.1f %%), DIFFER %d (%.1f %%)." % (
        len(cases), ", ".join("%s %d" % (v, oc[v]) for v in CASE_ORDER if oc[v]), compiled,
        oc["EQUAL"], 100.0 * oc["EQUAL"] / max(compiled, 1), oc["DIFFER"], 100.0 * oc["DIFFER"] / max(compiled, 1)))
    first = [f for o, f in cases.values() if o == "REFUSED" and f]
    out += table("%s: first refusal, reason class" % title, collections.Counter(f["class"] for f in first))
    out += table("%s: first refusal, text (first 25)" % title,
                 collections.Counter("%s %s" % (f["stage"], f["ours_text"][:110]) for f in first), n=25)
    # distinct shader sources behind the refused cases: the refusing stage's text
    out.append("")
    out.append("Distinct refusing shaders behind the %d refused cases: %d." % (
        len(first), len({f["source"] for f in first})))
    out.append("")
    out.append("| dEQP group | cases | EQUAL | DIFFER | REFUSED | distinct refusing shaders | other |")
    out.append("|---|--:|--:|--:|--:|--:|--:|")
    grp = collections.defaultdict(collections.Counter)
    srcs = collections.defaultdict(set)
    for c, (o, f) in cases.items():
        grp[deqp_group(c)][o] += 1
        if o == "REFUSED" and f:
            srcs[deqp_group(c)].add(f["source"])
    for g, c in sorted(grp.items(), key=lambda kv: -sum(kv[1].values())):
        n = sum(c.values())
        out.append("| %s | %d | %d | %d | %d | %d | %d |" % (
            g, n, c["EQUAL"], c["DIFFER"], c["REFUSED"], len(srcs[g]),
            n - c["EQUAL"] - c["DIFFER"] - c["REFUSED"]))
    return out


# ---- the diff ----------------------------------------------------------------
REGRESSION = {("EQUAL", "DIFFER"), ("EQUAL", "REFUSED")}


def diff(old, new):
    """(text, regressions) between two row lists."""
    a = {(r["key"], r["stage"]): r for r in old}
    b = {(r["key"], r["stage"]): r for r in new}
    moves = collections.Counter()
    regress = []
    for k in sorted(set(a) & set(b)):
        va, vb = a[k]["verdict"], b[k]["verdict"]
        if va != vb:
            moves[(va, vb)] += 1
            if (va, vb) in REGRESSION:
                regress.append("%s %s: %s -> %s: %s" % (k[0], k[1], va, vb,
                                                        b[k]["ours_text"] if vb == "REFUSED" else b[k]["detail"]))
    out = ["rows: old %d, new %d, in both %d, only old %d, only new %d" % (
        len(a), len(b), len(set(a) & set(b)), len(set(a) - set(b)), len(set(b) - set(a)))]
    for (va, vb), n in sorted(moves.items(), key=lambda kv: -kv[1]):
        out.append("%-9s -> %-9s %6d" % (va, vb, n))
    named = [("REFUSED", "EQUAL"), ("REFUSED", "DIFFER"), ("EQUAL", "DIFFER")]
    out.append("refused -> equal %d, refused -> differ %d, equal -> differ %d (regression), equal -> refused %d"
               " (regression)" % (moves[named[0]], moves[named[1]], moves[named[2]], moves[("EQUAL", "REFUSED")]))
    ca, cb = case_outcomes(old), case_outcomes(new)
    if ca or cb:
        cm = collections.Counter((ca[c][0], cb[c][0]) for c in set(ca) & set(cb) if ca[c][0] != cb[c][0])
        out.append("dEQP cases: %d in both; moved:" % len(set(ca) & set(cb)))
        for (va, vb), n in sorted(cm.items(), key=lambda kv: -kv[1]):
            out.append("  case %-9s -> %-9s %6d" % (va, vb, n))
    out.append("regressions: %d" % len(regress))
    out += ["regression\t" + r for r in regress]
    return "\n".join(out) + "\n", regress


# ---- run ----------------------------------------------------------------------
def log(msg):
    print("corpus_check: " + msg, file=sys.stderr, flush=True)


def has_struct_uniform(*srcs):
    """A uniform of a struct type. gl_ref.py --uniform-values sets its
    members and tests/mali_ref reads them (`differences_with`); a reference
    of such a program is kept under its own name (`<p>-m`), since one drawn
    before that rule had every uniform 0."""
    builtin = re.compile(r"^(float|int|bool|[ib]?vec[234]|mat[234]|sampler2D|samplerCube)$")
    for s in srcs:
        s = re.sub(r"//[^\n]*|/\*.*?\*/", " ", s, flags=re.S)
        for ty in re.findall(r"\buniform\s+(?:(?:lowp|mediump|highp)\s+)?(\w+)", s):
            if ty != "struct" and not builtin.match(ty):
                return True
        if re.search(r"\buniform\s+struct\b", s):
            return True
    return False


class Run:
    def __init__(self, args):
        self.args = args
        self.cache = os.path.abspath(os.path.expanduser(args.cache))
        os.makedirs(self.cache, exist_ok=True)
        self.recipe = recipe_key()
        self.table = message_table(os.path.join(REPO, "src/shaderc/online/mali/mod.rs"))
        self.items = []  # (key, group, vs, fs, json path or None, rule, lone stage or '')
        self.cases_without = []  # (case, verdict)

    # -- the corpora --
    def add(self, key, group, vs, fs, json_path=None, rule="values", lone=""):
        self.items.append((key, group, vs, fs, json_path, rule, lone))

    def select_testdata(self):
        d = os.path.join(HERE, "testdata")
        for n in sorted(f[:-5] for f in os.listdir(d) if f.endswith(".vert")):
            if not os.path.isfile(os.path.join(d, n + ".frag")):
                continue
            rd = lambda e: open(os.path.join(d, n + e), encoding="utf-8").read()
            j = os.path.join(d, n + ".expect.json")
            self.add("testdata:" + n, "testdata", rd(".vert"), rd(".frag"),
                     j if os.path.isfile(j) else None, "either" if os.path.isfile(j) else "values")

    def select_glmark2(self):
        corpus = self.args.glmark2 or os.environ.get("GLMARK2_CORPUS") or os.path.join(REPO, "build/glmark2-host/shaders")
        if not os.path.isfile(os.path.join(corpus, "index.txt")):
            raise SystemExit("corpus_check: no glmark2 corpus at %s (tools/hostharness/glmark2/run.sh makes it; "
                             "--glmark2 or GLMARK2_CORPUS names another)" % corpus)
        files = glmark2_files(corpus)
        for i, entry, progs in glmark2_programs(os.path.join(REPO, "tests/mali_glmark2_corpus.rs")):
            for p in progs:
                v, f = p.split("+")
                if (v, "vert") not in files or (f, "frag") not in files:
                    raise SystemExit("corpus_check: %s: %s is not in %s" % (entry, p, corpus))
                rd = lambda path: open(path, encoding="utf-8").read()
                self.add("glmark2:%d:%s" % (i, p), "glmark2." + entry.split(":")[0],
                         rd(files[(v, "vert")]), rd(files[(f, "frag")]))

    def deqp_dir(self):
        if self.args.deqp:
            return self.args.deqp
        lib = tree_key(REPO, LIBRARY_PATHS)
        d = os.path.join(self.cache, "deqp", lib)
        if os.path.isfile(os.path.join(d, "done")):
            log("dEQP: the run of library %s (%s)" % (lib, d))
            return d
        cmd = [os.path.join(HERE, "deqp-host-measure.sh"), "--out", d, "--cases",
               os.path.join(REPO, "tools/cts/gles2-mustpass.txt"), "--jobs", str(self.args.jobs)]
        if self.args.cts:
            cmd += ["--cts", self.args.cts]
        t = time.time()
        for stage in ("build", "run"):
            log("dEQP: %s" % " ".join(cmd + ["--stage", stage]))
            subprocess.run(cmd + ["--stage", stage], check=True, stdout=sys.stderr)
        open(os.path.join(d, "done"), "w").close()
        log("dEQP: run in %.0f s" % (time.time() - t))
        return d

    def select_deqp(self):
        d = self.deqp_dir()
        cases = {}
        for q in sorted(f for f in os.listdir(d) if f.startswith("c_") and f.endswith(".qpa")):
            with open(os.path.join(d, q), encoding="utf-8", errors="replace") as f:
                for c, progs in read_qpa(f.read()).items():
                    cases.setdefault(c, progs)
        with open(os.path.join(d, "cases.txt")) as f:
            order = [line.strip() for line in f if line.strip()]
        if self.args.deqp_cases:
            keep = board_refused(self.args.deqp_cases)
            order = [c for c in order if c in keep]
        for c in order:
            if c not in cases:
                self.cases_without.append((c, "NOTRUN"))
                continue
            if not cases[c]:
                self.cases_without.append((c, "NOSHADER"))
                continue
            for i, (vs, fs) in enumerate(cases[c]):
                if vs and fs:
                    self.add("%s#%d" % (c, i), deqp_group(c), vs, fs)
                elif vs:
                    self.add("%s#%d" % (c, i), deqp_group(c), vs, LONE_FS, lone="vs")
                else:
                    self.add("%s#%d" % (c, i), deqp_group(c), LONE_VS, fs, lone="fs")

    def sample(self):
        """--sample N: N of the selection's cases (a dEQP case with all its
        programs), seeded."""
        units = collections.OrderedDict()
        for it in self.items:
            units.setdefault(it[0].split("#")[0], []).append(it)
        for c, v in self.cases_without:
            units.setdefault(c, []).append(None)
        keys = sorted(units)
        pick = set(random.Random(self.args.seed).sample(keys, min(self.args.sample, len(keys))))
        self.items = [it for k in keys if k in pick for it in units[k] if it]
        self.cases_without = [(c, v) for c, v in self.cases_without if c in pick]

    # -- the sources --
    def source_paths(self, vs, fs):
        p = program_key(vs, fs)
        d = os.path.join(self.cache, "src", p[:2])
        os.makedirs(d, exist_ok=True)
        out = []
        for ext, s in ((".vert", vs), (".frag", fs)):
            path = os.path.join(d, p + ext)
            if not os.path.isfile(path):
                with open(path + ".tmp", "w", encoding="utf-8") as f:
                    f.write(s)
                os.replace(path + ".tmp", path)
            out.append(path)
        return p, out[0], out[1]

    # -- Mesa's side --
    def reference(self, p, vs_path, fs_path, vs, fs):
        """(json path or None, 'ok' | 'fail') of the program, made once."""
        d = os.path.join(self.cache, "ref", self.recipe, p[:2])
        name = p + "-m" if has_struct_uniform(vs, fs) else p
        j, fail = os.path.join(d, name + ".expect.json"), os.path.join(d, name + ".fail")
        if os.path.isfile(j):
            return j, "ok"
        if os.path.isfile(fail):
            return None, "fail"
        os.makedirs(d, exist_ok=True)
        work = os.environ.get("MESA_SHIM_WORK") or os.path.join(os.environ.get("CONNECTED_DEPS_DIR", ""), "mesa-shim")
        with tempfile.TemporaryDirectory(prefix="corpus_check.") as t:
            cmd = [os.path.join(HERE, "drm_shim_ref.sh"), "run", work, vs_path, fs_path, os.path.join(t, "r"),
                   "--prim", "triangles", "--uniform-values"]
            try:
                r = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
                ok = r.returncode == 0
                try:
                    with open(os.path.join(t, "r.stderr"), errors="replace") as f:
                        why = [line for line in f.read().splitlines() if line.startswith("gl_ref:")][:1]
                except OSError:
                    why = []
                why = why or (r.stderr.strip().splitlines() or ["exit %d" % r.returncode])[-2:]
            except subprocess.TimeoutExpired:
                ok, why = False, ["timeout"]
            if ok:
                r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "pan_ref_diff.py"), "extract",
                                    os.path.join(t, "r.trace"), "--draw"], capture_output=True, text=True)
                ok = r.returncode == 0 and '"fs.fragment"' in r.stdout and '"vs.position"' in r.stdout
                why = (r.stderr.strip().splitlines() or ["extract: no vs.position and fs.fragment"])[-2:]
            if ok:
                with open(j + ".tmp", "w") as f:
                    f.write(r.stdout)
                os.replace(j + ".tmp", j)
                return j, "ok"
            if why == ["timeout"]:
                return None, "fail"  # a loaded host: not kept, the next run draws again
            with open(fail, "w") as f:
                f.write(" / ".join(why) + "\n")
            return None, "fail"

    # -- our side --
    def test_binary(self):
        r = subprocess.run(["cargo", "test", "-q", "--no-default-features", "--features", FEATURES, "--test",
                            "mali_ref_selftest", "--no-run", "--message-format=json"],
                           cwd=REPO, capture_output=True, text=True)
        if r.returncode != 0:
            sys.stderr.write(r.stderr[-3000:])
            raise SystemExit("corpus_check: the measure's test does not build")
        exe = None
        for line in r.stdout.splitlines():
            try:
                m = json.loads(line)
            except ValueError:
                continue
            if m.get("reason") == "compiler-artifact" and m.get("target", {}).get("name") == "mali_ref_selftest" \
                    and m.get("executable"):
                exe = m["executable"]
        if not exe:
            raise SystemExit("corpus_check: cargo named no mali_ref_selftest executable")
        return exe

    def measure(self, exe, lines):
        """{key: measure fields} of list lines, through processes of up to
        200 lines each; a program whose process dies is CRASH, and the
        rest of its batch goes on in a fresh process."""
        out = {}

        def batch(chunk):
            got = {}
            while chunk:
                with tempfile.NamedTemporaryFile("w", suffix=".list", delete=False) as f:
                    f.write("".join(l + "\n" for l in chunk))
                try:
                    r = subprocess.run([exe, "--exact", "the_measure_reads_a_list", "--nocapture", "--test-threads",
                                        "1", "-q"], cwd=REPO, capture_output=True, text=True, timeout=1800,
                                       env=dict(os.environ, YAKOGL_MALI_MEASURE_LIST=f.name))
                    stdout, rc = r.stdout, r.returncode
                except subprocess.TimeoutExpired as e:
                    stdout, rc = (e.stdout or b"").decode(errors="replace") if isinstance(e.stdout, bytes) else (e.stdout or ""), -9
                os.unlink(f.name)
                begun = None
                for line in stdout.splitlines():
                    if line.startswith("begin\t"):
                        begun = line[6:]
                    elif line.startswith("measure\t"):
                        f2 = line.split("\t")[1:]
                        got[f2[0]] = f2[1:]
                        begun = None
                done = set(got)
                rest = [l for l in chunk if l.split("\t")[0] not in done]
                if rc == 0 or not rest:
                    break
                dead = begun or rest[0].split("\t")[0]
                text = "process exit %d" % rc
                got[dead] = ["REFUSED", "-", text, "CRASH", text, "CRASH", text]
                chunk = [l for l in rest if l.split("\t")[0] != dead]
            return got
        chunks = [lines[i:i + 200] for i in range(0, len(lines), 200)]
        with concurrent.futures.ThreadPoolExecutor(self.args.jobs) as ex:
            for got in ex.map(batch, chunks):
                out.update(got)
        return out

    def go(self):
        t0 = time.time()
        sels = self.args.select or ["deqp", "testdata", "glmark2"]
        for s in sels:
            getattr(self, "select_" + s.replace("-", "_"))()
        if self.args.sample:
            self.sample()
        tree = tree_key(REPO, COMPILER_PATHS)
        commit = describe(REPO)
        log("%d programs (%d distinct), %d cases without one; compiler tree %s at %s" % (
            len(self.items), len({program_key(i[2], i[3]) for i in self.items}), len(self.cases_without), tree, commit))
        # the sources, and Mesa's references
        prog = {}
        for key, group, vs, fs, jp, rule, lone in self.items:
            p, vp, fp = self.source_paths(vs, fs)
            prog[key] = (p, vp, fp)
        want = {}
        ours_known = self.load_ours(tree)
        for key, group, vs, fs, jp, rule, lone in self.items:
            p = prog[key][0]
            if jp or lone or p in want:
                continue
            want[p] = (prog[key][1], prog[key][2], vs, fs)
        t1 = time.time()
        refs = {}
        with concurrent.futures.ThreadPoolExecutor(self.args.jobs) as ex:
            futs = {ex.submit(self.reference, p, *w): p for p, w in want.items()}
            for n, fu in enumerate(concurrent.futures.as_completed(futs), 1):
                refs[futs[fu]] = fu.result()
                if n % 1000 == 0:
                    log("references: %d of %d" % (n, len(futs)))
        log("references: %d programs in %.0f s (%d drawn by Mesa, %d failed)" % (
            len(refs), time.time() - t1, sum(1 for v in refs.values() if v[1] == "ok"),
            sum(1 for v in refs.values() if v[1] == "fail")))
        # our side: what the cache lacks
        todo, keys = {}, {}
        for key, group, vs, fs, jp, rule, lone in self.items:
            p, vp, fp = prog[key]
            j = jp or (None if lone else refs.get(p, (None, "-"))[0])
            ok = self.ours_key(p, j, rule)
            keys[key] = ok
            if ok not in ours_known:
                todo[ok] = "\t".join([ok, vp, fp, j or "-", rule])
        lines = list(todo.values())
        t2 = time.time()
        if lines:
            exe = self.test_binary()
            log("ours: %d programs to measure (%d cached), test built in %.0f s" % (
                len(lines), len(ours_known), time.time() - t2))
            t3 = time.time()
            got = self.measure(exe, lines)
            self.save_ours(tree, got)
            ours_known.update(got)
            log("ours: measured in %.0f s" % (time.time() - t3))
        else:
            log("ours: every program cached for tree %s" % tree)
        rows = []
        for key, group, vs, fs, jp, rule, lone in self.items:
            p = prog[key][0]
            mesa = "ok" if jp else ("-" if lone else refs.get(p, (None, "-"))[1])
            rows += stage_rows(key, group, ours_known[keys[key]], mesa, p, self.table, lone,
                               (sha("source", vs), sha("source", fs)))
        for c, v in self.cases_without:
            rows.append(dict(key=c, group=deqp_group(c), stage="-", ours="-", ours_text="-", mesa="-", verdict=v,
                             **{"class": "-"}, detail="-", program="-", source="-"))
        out = self.args.out or os.path.join(self.cache, "rows-%s.tsv" % commit)
        head = ["%s tree=%s commit=%s recipe=%s select=%s%s" % (
            ROWS_HEADER, tree, commit, self.recipe, ",".join(sels),
            " sample=%d seed=%d" % (self.args.sample, self.args.seed) if self.args.sample else ""),
            "# " + "\t".join(COLUMNS)]
        write_rows(out, head, rows)
        log("%d rows in %s; wall time %.0f s" % (len(rows), out, time.time() - t0))
        sys.stdout.write(summary(rows))

    def ours_key(self, p, j, rule):
        jh = "-"
        if j:
            with open(j, "rb") as f:
                jh = sha(f.read())
        return sha("ours", p, jh, rule)[:32]

    def ours_file(self, tree):
        return os.path.join(self.cache, "ours", tree + ".tsv")

    def load_ours(self, tree):
        out = {}
        try:
            with open(self.ours_file(tree), encoding="utf-8") as f:
                for line in f:
                    parts = line.rstrip("\n").split("\t")
                    if len(parts) == 8:
                        out[parts[0]] = parts[1:]
        except OSError:
            pass
        return out

    def save_ours(self, tree, got):
        os.makedirs(os.path.dirname(self.ours_file(tree)), exist_ok=True)
        with open(self.ours_file(tree), "a", encoding="utf-8") as f:
            for k, v in got.items():
                f.write("\t".join([k] + v) + "\n")


BOARD_SLICES = ["tools/cts/log/2026-11-15-t6-gles2-S%d-37305608" % k for k in (1, 2, 3, 4)]


def board_refused(spec):
    """The case set of --board-refused: `slices` is every case the board
    refused to compile in BOARD_SLICES (deqp-host-measure.sh's default
    population, the study's 10,597); else a file whose lines begin with a
    case name (a case list, or deqp-host-measure.sh's board.tsv)."""
    if spec == "slices":
        out = set()
        for d in BOARD_SLICES:
            with open(os.path.join(REPO, d, "refusals.tsv"), encoding="utf-8", errors="replace") as f:
                for line in f:
                    case, _, text = line.rstrip("\n").partition("\t")
                    if "online shaderc: the code generator refused" in text:
                        out.add(case)
        return out
    with open(spec, encoding="utf-8") as f:
        return {line.split("\t")[0].strip() for line in f if line.strip()}


def main(argv):
    ap = argparse.ArgumentParser(prog="corpus_check.py", description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--select", action="append", choices=["deqp", "testdata", "glmark2"])
    r.add_argument("--out")
    r.add_argument("--cache", default=os.environ.get("YAKOGL_MALI_CORPUS_CACHE", "~/.cache/yakogl-gate/mali-corpus"))
    r.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) - 1))
    r.add_argument("--sample", type=int, default=0)
    r.add_argument("--seed", type=int, default=387)
    r.add_argument("--deqp", help="a deqp-host-measure.sh output directory to read instead of running dEQP")
    r.add_argument("--deqp-cases", metavar="SPEC",
                   help="only these dEQP cases: `slices` (the board-refused cases) or a case list file")
    r.add_argument("--cts")
    r.add_argument("--glmark2")
    s = sub.add_parser("summary")
    s.add_argument("rows")
    s.add_argument("--board-refused", help="`slices` or a case list: the dEQP table again for these cases")
    d = sub.add_parser("diff")
    d.add_argument("old")
    d.add_argument("new")
    a = ap.parse_args(argv[1:])
    if a.cmd == "run":
        Run(a).go()
        return 0
    try:
        if a.cmd == "summary":
            _, rows = read_rows(a.rows)
            board = board_refused(a.board_refused) if a.board_refused else None
            sys.stdout.write(summary(rows, board))
            return 0
        _, old = read_rows(a.old)
        _, new = read_rows(a.new)
    except (OSError, ValueError) as e:
        print("corpus_check: %s" % e, file=sys.stderr)
        return 2
    text, regress = diff(old, new)
    sys.stdout.write(text)
    return 1 if regress else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
