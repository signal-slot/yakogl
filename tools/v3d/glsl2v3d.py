#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compile YakoGL's shader corpus (tools/shaders) for the Broadcom V3D 4.2 with
v3d_shc, one GL PROGRAM (a vertex shader and a fragment shader) at a time.

v3d_shc compiles the FS, then the VS keyed by the FS's inputs, then the CS (the
same VS for binning), so a stage is not a unit here; a pair is. The SPIR-V is
made exactly as tools/vivante/glsl2vivante.py makes it: this script imports its
translate_es100_gl / translate_es300_gl / translate_450_gl and es100's stage_of
and es_version, and runs `glslangValidator -G`.

Pairing rule. A fragment shader gets its REAL partners when the tree records
any, and only then the fallback:

  1. gl_test programs. Every program gl_test.c links, read out of its text:
     link_program(VS_SYM, FS_SYM) calls, and for the sites that link through a
     variable (glAttachShader, `p ? a : b`, an array, a `const char *vs_src`
     parameter) the gl_test_*_vs_src / *_fs_src symbols the function refers to
     that no direct call accounts for, the parameter resolved at its call
     sites. Symbols map to files through ENTRIES in
     driver/tools/pvrlab/gen_gl_test_shaders.py, and each program also gets its
     GL_TEST_ES100 form through that file's ES100_TWINS (the ES 1.00 header
     swaps the sources and keeps the symbols, so the same call links the twins).
     A call whose other half is a buffer the case fills with GLSL while it runs
     (`static char src[2048]`) names no program, and the source beside it is
     used up there: offering it to another call of the same function would
     invent a program gl_test never links.
     A site this cannot resolve fails the run; it is never dropped.
  2. The file-name convention of the directories that document one:
     `<name>.vert` + `<name>.frag`, and `<name>.batchable.vert` + `<name>.frag`
     (shaders/qt/README.md; the same convention holds for internal/clear.* and
     internal/fbload.*, which the backends draw with, and for qt3d's
     qtappshaders_N, one shadergen material each). Applied in every directory.
  3. Fallback, only for an FS that neither rule pairs: every VS in the FS's
     own directory and in the top level whose output names cover all of the
     FS's user varying inputs (v3d_shc links varyings by name, built-ins
     excluded, so this is the rule the tool itself applies). The names are
     read from the translated text; for every source glslang accepts they are
     checked against the SPIR-V (spirv-dis), and a disagreement fails the run.

An FS with no partner under any rule is listed by name and is neither a pass
nor a fail. A VS that ends up in no pair is listed the same way. A pair that
includes a source in shaders/known-uncompilable.txt is compiled like any other
and marked as such.

Usage: glsl2v3d.py [--jobs N] [--json results.json] [--keep DIR]
"""
import argparse
import ast
import collections
import concurrent.futures
import json
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", ".."))
SHADERS = os.path.join(REPO, "tools", "shaders")
GL_TEST_C = os.path.join(REPO, "driver", "tools", "pvrlab", "gl_test.c")
GEN_GL_TEST = os.path.join(REPO, "driver", "tools", "pvrlab", "gen_gl_test_shaders.py")
KNOWN_BAD = os.path.join(SHADERS, "known-uncompilable.txt")

sys.path.insert(0, os.path.join(REPO, "tools", "vivante"))
import glsl2vivante as gv  # noqa: E402
from glsl2vivante import u  # noqa: E402  (es100: stage_of, es_version)

V3D_SHC = os.environ.get("V3D_SHC", os.path.expanduser(
    "~/pvr-lab/mesa-build/mesa-26.2.1/build-v3d/src/broadcom/v3d_shc"))
GLSLANG = gv.GLSLANG
SPIRV_DIS = os.environ.get("SPIRV_DIS", "spirv-dis")
STAGES = ("fs", "vs", "cs")

# The known positive: this pair must be formed and must compile to exactly
# these instruction counts, or nothing else this script prints is believed.
KNOWN_POSITIVE = ("color_uv.vert", "color_uv.frag")
KNOWN_POSITIVE_COUNTS = {"fs": 12, "vs": 31, "cs": 22}


# --------------------------------------------------------------------------
# The corpus
# --------------------------------------------------------------------------

def corpus():
    """Every .vert/.frag under tools/shaders, as paths relative to it."""
    out = []
    for d, _, fs in os.walk(SHADERS):
        for f in fs:
            if f.endswith((".vert", ".frag")):
                out.append(os.path.relpath(os.path.join(d, f), SHADERS))
    return sorted(out)


def known_bad():
    out = set()
    for line in open(KNOWN_BAD):
        line = line.strip()
        if line and not line.startswith("#"):
            out.add(line)
    return out


# --------------------------------------------------------------------------
# Rule 1: gl_test programs
# --------------------------------------------------------------------------

SYM = r"gl_test_(?:\w+_)?[vf]s_src"


def gen_gl_test_tables():
    """ENTRIES and ES100_TWINS from gen_gl_test_shaders.py, read with ast
    (importing it would regenerate the headers)."""
    tree = ast.parse(open(GEN_GL_TEST).read())
    found = {}
    for node in tree.body:
        if isinstance(node, ast.Assign) and len(node.targets) == 1 \
                and isinstance(node.targets[0], ast.Name) \
                and node.targets[0].id in ("ENTRIES", "ES100_TWINS"):
            found[node.targets[0].id] = ast.literal_eval(node.value)
    sym2file = {e[0]: e[1] for e in found["ENTRIES"]}
    return sym2file, found["ES100_TWINS"]


def c_functions(text):
    """(name, params, body) for each top-level C function: a chunk ending at a
    '}' in column 0, named by the last `name(...)` line in column 0 before a
    '{' in column 0."""
    out = []
    for chunk in re.split(r"(?<=\n\})\n", text):
        m = None
        for m in re.finditer(r"^(?:static\s+)?[A-Za-z_][\w \t\*]*?\b(\w+)\s*\(([^;{]*?)\)\s*\n\{",
                             chunk, re.M):
            pass
        if m:
            out.append((m.group(1), m.group(2), chunk[m.start():]))
    return out


def link_calls(body):
    """The argument lists of the body's link_program / glLinkProgram calls.
    Parentheses are balanced, so a call spanning lines or holding a ternary
    reads as one call; a mention in a comment gives no arguments."""
    out = []
    for m in re.finditer(r"\blink_program\s*\(|\bglLinkProgram\s*\(", body):
        i, depth = m.end(), 1
        while i < len(body) and depth:
            if body[i] == "(":
                depth += 1
            elif body[i] == ")":
                depth -= 1
            i += 1
        args, depth, cur = [], 0, ""
        for c in body[m.end():i - 1]:
            if c == "(":
                depth += 1
            elif c == ")":
                depth -= 1
            if c == "," and not depth:
                args.append(cur)
                cur = ""
            else:
                cur += c
        out.append([a.strip() for a in args + [cur] if a.strip()])
    return out


def runtime_buffers(body):
    """Names the function fills with GLSL while it runs: `static char src[2048]`.
    A link call with one of them as an argument links a program whose other half
    exists only after the case runs, so no pair can name that program."""
    return set(re.findall(r"\bchar\s+(\w+)\s*\[", body))


def program_rows(body):
    """The rows of a table of programs: an initializer row naming both halves,
    `{ "rsg's pair", gl_test_f3_vs_src, gl_test_f3_fs_src },`. gl_test links
    such a table with link_program(t[p].vs, t[p].fs), so its rows are its
    programs; pairing the table's vertex shaders with its fragment shaders
    instead names programs the table does not hold."""
    out = []
    for m in re.finditer(r"\{[^{}]*\},", body):
        syms = re.findall(r"\b%s\b" % SYM, m.group(0))
        vs = [s for s in syms if s.endswith("_vs_src")]
        fs = [s for s in syms if s.endswith("_fs_src")]
        out += [(v, f) for v in vs for f in fs]
    return out


def gl_test_pairs():
    """[(vs_file, fs_file, how)] and a list of sites that could not be resolved.

    A program is named by the call that links it. Each argument of a link call
    is one of:
      - a source symbol: the pair, when both arguments are one;
      - a variable or a table entry (fs[p], progs[p].fs): the sources the
        function hands it, paired within the function;
      - a parameter: the parameter's values at the call sites;
      - a buffer the case fills at run time (runtime_buffers): no pair, and the
        source beside it is used up by that call -- offering it to another call
        of the same function invents a program gl_test never links, and an
        invented program is a build failure, not a spare row.
    A table that holds both halves names its programs by its rows
    (program_rows), which the same rule asks for before pairing across it.
    """
    sym2file, twins = gen_gl_test_tables()
    text = open(GL_TEST_C).read()
    sym_pairs = []          # (vs_sym, fs_sym, how)
    unresolved = []
    for name, params, body in c_functions(text):
        if name == "link_program":
            continue
        calls = link_calls(body)
        if not calls:
            continue
        buffers = runtime_buffers(body)
        direct, runtime = [], []
        for a in calls:
            ids = [i for x in a for i in re.findall(r"\b\w+\b", x)]
            named = [i for i in ids if re.fullmatch(SYM, i)]
            built = [i for i in ids if i in buffers]
            if len(a) == 2 and len(named) == 2:
                direct.append((named[0], named[1]))
            elif built:
                runtime.append((named, built))
        rows = program_rows(body)
        for vs, fs in direct:
            sym_pairs.append((vs, fs, "gl_test.c %s" % name))
        for vs, fs in rows:
            sym_pairs.append((vs, fs, "gl_test.c %s (a table of programs)" % name))
        used = {s for p in direct + rows for s in p}
        for named, built in runtime:
            used.update(named)
        if runtime:
            unresolved.append("%s: links %d, direct %d, half built at run time: %s"
                              % (name, len(calls), len(direct),
                                 "; ".join("%s with %s"
                                           % (", ".join(n) or "no named half", b[0])
                                           for n, b in runtime)))
        if len(direct) == len(calls):
            continue
        syms = set(re.findall(r"\b%s\b" % SYM, body))
        left = {"vs": sorted(s for s in syms - used if s.endswith("_vs_src")),
                "fs": sorted(s for s in syms - used if s.endswith("_fs_src"))}
        # a source passed in as a parameter: take it from the call sites
        for side in ("vs", "fs"):
            if left[side] or not re.search(r"const\s+char\s*\*\s*\w*%s\w*" % side, params):
                continue
            for call in re.finditer(r"\b%s\s*\(([^;]*?)\)\s*(?:[;!=<>)]|$)" % re.escape(name),
                                    text, re.M):
                left[side].extend(s for s in re.findall(r"\b%s\b" % SYM, call.group(1))
                                  if s.endswith("_%s_src" % side))
        if bool(left["vs"]) != bool(left["fs"]):
            unresolved.append("%s: links %d, direct %d, left over vs=%s fs=%s"
                              % (name, len(calls), len(direct), left["vs"], left["fs"]))
            continue
        for vs in left["vs"]:
            for fs in left["fs"]:
                sym_pairs.append((vs, fs, "gl_test.c %s (by variable)" % name))
    pairs = []
    for vs, fs, how in sym_pairs:
        missing = [s for s in (vs, fs) if s not in sym2file]
        if missing:
            unresolved.append("%s: symbol not in gen_gl_test_shaders.py ENTRIES: %s"
                              % (how, missing))
            continue
        vf, ff = sym2file[vs], sym2file[fs]
        pairs.append((vf, ff, how))
        tv, tf = twins.get(vf, vf), twins.get(ff, ff)
        if (tv, tf) != (vf, ff):
            pairs.append((tv, tf, how + ", GL_TEST_ES100 twins"))
    return pairs, unresolved


# --------------------------------------------------------------------------
# Rule 2: the file-name convention
# --------------------------------------------------------------------------

def name_pairs(files):
    fset = set(files)
    out = []
    for f in files:
        if not f.endswith(".frag"):
            continue
        base = f[:-len(".frag")]
        for vs in (base + ".vert", base + ".batchable.vert"):
            if vs in fset:
                out.append((vs, f, "name convention"))
    return out


# --------------------------------------------------------------------------
# SPIR-V and varying names
# --------------------------------------------------------------------------

def translate(rel):
    """The GLSL 4.60 text glsl2vivante.compile_one would hand glslang."""
    path = os.path.join(SHADERS, rel)
    stage = u.stage_of(path)
    text = open(path).read()
    version = u.es_version(text)
    if version == 100:
        return gv.translate_es100_gl(text, stage)
    if version in (300, 310):
        return gv.translate_es300_gl(text, stage)
    return gv.translate_450_gl(text)


# layout() and the interpolation qualifiers come in either order: the ES 3.00
# rewrite writes "flat layout(location = N) in ..." (es300/flatlast.frag).
IO_DECL = re.compile(r"^\s*(?:layout\s*\([^)]*\)\s*|"
                     r"(?:invariant|flat|smooth|noperspective|centroid)\s+)*"
                     r"(in|out)\s+(?:(?:highp|mediump|lowp)\s+)?\w+\s+(\w+)\s*(?:\[[^\]]*\])?\s*;\s*$",
                     re.S)


def text_io(text):
    """Top-level user `in` and `out` names of translated text."""
    ins, outs = set(), set()
    for top, kind, chunk in u.es100_statements(u.strip_comments(text)):
        if not top or kind != "stmt":
            continue
        m = IO_DECL.match(chunk)
        if m:
            (ins if m.group(1) == "in" else outs).add(m.group(2))
    return ins, outs


def spirv_io(spv):
    """User (non-BuiltIn) Input and Output variable names of a SPIR-V module."""
    r = subprocess.run([SPIRV_DIS, "--raw-id", spv], capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit("glsl2v3d: spirv-dis failed on %s: %s" % (spv, r.stderr))
    names, builtin, pointee, vars_, builtin_blocks = {}, set(), {}, [], set()
    for line in r.stdout.splitlines():
        m = re.match(r'\s*OpName %(\d+) "([^"]*)"', line)
        if m:
            names[m.group(1)] = m.group(2)
        m = re.match(r"\s*OpDecorate %(\d+) BuiltIn", line)
        if m:
            builtin.add(m.group(1))
        m = re.match(r"\s*OpMemberDecorate %(\d+) \d+ BuiltIn", line)
        if m:
            builtin_blocks.add(m.group(1))
        m = re.match(r"\s*%(\d+) = OpTypePointer (?:Input|Output) %(\d+)", line)
        if m:
            pointee[m.group(1)] = m.group(2)
        m = re.match(r"\s*%(\d+) = OpVariable %(\d+) (Input|Output)", line)
        if m:
            vars_.append((m.group(1), m.group(2), m.group(3)))
    ins, outs = set(), set()
    for vid, ptr, sc in vars_:
        if vid in builtin or pointee.get(ptr) in builtin_blocks:
            continue
        (ins if sc == "Input" else outs).add(names.get(vid, "<unnamed %%%s>" % vid))
    return ins, outs


def prepare(rel, work):
    """Translate and assemble one source. Returns a dict with spv (or None),
    error (the full glslang/translator message, or None), and the in/out
    names from the text and from the SPIR-V (None when not known)."""
    stage = u.stage_of(rel)
    rec = {"rel": rel, "stage": stage, "spv": None, "error": None,
           "ins": None, "outs": None, "spirv_ins": None, "spirv_outs": None}
    try:
        text = translate(rel)
    except SystemExit as e:
        rec["error"] = "translate: %s" % e
        return rec
    rec["ins"], rec["outs"] = text_io(text)
    stem = re.sub(r"[^\w.]", "_", rel)
    src = os.path.join(work, stem + (".vert" if stage == "vs" else ".frag"))
    spv = os.path.join(work, stem + ".spv")
    with open(src, "w") as f:
        f.write(text)
    r = subprocess.run([GLSLANG, "-G", src, "-o", spv], capture_output=True, text=True)
    if r.returncode != 0:
        tag = "<%s>" % stage
        msg = (r.stdout + r.stderr).replace(src, tag)
        lines = [ln for ln in msg.splitlines() if ln.strip() and ln.strip() != tag]
        rec["error"] = "\n".join(lines) if lines else "(glslang exit %d, no output)" % r.returncode
        return rec
    rec["spv"] = spv
    rec["spirv_ins"], rec["spirv_outs"] = spirv_io(spv)
    return rec


# --------------------------------------------------------------------------
# One program through v3d_shc
# --------------------------------------------------------------------------

DISASM = re.compile(r"^(--- \w+: \d+ instructions$|\s*\d+ 0x[0-9a-f]{16} )")


def run_pair(vs, fs, recs, keep):
    rv, rf = recs[vs], recs[fs]
    res = {"vs": vs, "fs": fs, "status": None, "stage": None, "message": None, "stats": None}
    if rv["spv"] is None:
        res.update(status="fail", stage="glslang vs", message=rv["error"])
        return res
    if rf["spv"] is None:
        res.update(status="fail", stage="glslang fs", message=rf["error"])
        return res
    r = subprocess.run([V3D_SHC, rv["spv"], rf["spv"]], capture_output=True, text=True)
    stderr = r.stderr.replace(rv["spv"], "<vs.spv>").replace(rf["spv"], "<fs.spv>")
    err = "\n".join(ln for ln in stderr.splitlines() if not DISASM.match(ln))
    if keep:
        d = os.path.join(keep, re.sub(r"[^\w.]", "_", vs) + "__" + re.sub(r"[^\w.]", "_", fs))
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "out.json"), "w") as f:
            f.write(r.stdout)
        with open(os.path.join(d, "disasm.txt"), "w") as f:
            f.write(r.stderr)
    if r.returncode != 0:
        if r.returncode < 0:
            stage = "v3d_shc killed by signal %d" % -r.returncode
        elif "link: FS input" in err:
            stage = "link"
        elif "v3d_compile FS failed" in err:
            stage = "v3d FS"
        elif "v3d_compile VS failed" in err:
            stage = "v3d VS"
        elif "v3d_compile CS failed" in err:
            stage = "v3d CS"
        elif "<vs.spv>: spirv_to_nir failed" in err:
            stage = "spirv_to_nir vs"
        elif "<fs.spv>: spirv_to_nir failed" in err:
            stage = "spirv_to_nir fs"
        else:
            stage = "v3d_shc exit %d" % r.returncode
        res.update(status="fail", stage=stage,
                   message=err if err else "(no stderr besides disassembly)")
        return res
    try:
        js = json.loads(r.stdout)
    except ValueError as e:
        res.update(status="fail", stage="v3d_shc output",
                   message="exit 0 but stdout is not JSON: %s" % e)
        return res
    res["status"] = "ok"
    res["stats"] = {}
    for st in STAGES:
        s = js.get(st)
        res["stats"][st] = None if s is None else {
            "instructions": len(s["code"]) if "code" in s else None,
            "threads": s.get("threads"),
            "spill_size": s.get("spill_size"),
        }
    return res


# --------------------------------------------------------------------------

def stat(r, st, key):
    s = (r["stats"] or {}).get(st)
    v = None if s is None else s.get(key)
    return "absent" if v is None else str(v)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    ap.add_argument("--json", help="write every pair's record here")
    ap.add_argument("--keep", help="keep each pair's out.json and disasm.txt under this directory")
    args = ap.parse_args()

    if not os.path.isfile(V3D_SHC):
        raise SystemExit("glsl2v3d: %s not found (set V3D_SHC)" % V3D_SHC)

    files = corpus()
    fset = set(files)
    bad = known_bad()
    fss = [f for f in files if f.endswith(".frag")]
    vss = [f for f in files if f.endswith(".vert")]

    # ---- pairing, rules 1 and 2
    real = []
    gl_pairs, unresolved = gl_test_pairs()
    for vs, fs, how in gl_pairs:
        for f in (vs, fs):
            if f not in fset:
                unresolved.append("%s names %s, which is not in tools/shaders" % (how, f))
        if vs in fset and fs in fset:
            real.append((vs, fs, how))
    real += name_pairs(files)
    if unresolved:
        print("gl_test pairing could not resolve:")
        for x in unresolved:
            print("  " + x)
        raise SystemExit("glsl2v3d: FAIL: a gl_test program site is unresolved")

    work = tempfile.mkdtemp(prefix="glsl2v3d-")
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as ex:
        recs = dict(zip(files, ex.map(lambda f: prepare(f, work), files)))

    # the varying-name instrument: a known positive, then the SPIR-V
    if recs[KNOWN_POSITIVE[1]]["ins"] != {"v_color", "v_uv"} \
            or recs[KNOWN_POSITIVE[0]]["outs"] != {"v_color", "v_uv"}:
        raise SystemExit("glsl2v3d: FAIL: known positive varyings read as %s / %s"
                         % (recs[KNOWN_POSITIVE[0]]["outs"], recs[KNOWN_POSITIVE[1]]["ins"]))
    checked, mismatch = 0, []
    for f, r in sorted(recs.items()):
        if r["spv"] is None:
            continue
        checked += 1
        key = "ins" if r["stage"] == "fs" else "outs"
        if r[key] != r["spirv_" + key]:
            mismatch.append("%s: text %s, SPIR-V %s"
                            % (f, sorted(r[key]), sorted(r["spirv_" + key])))
    if mismatch:
        print("varying names from the text disagree with the SPIR-V:")
        for x in mismatch:
            print("  " + x)
        raise SystemExit("glsl2v3d: FAIL: the pairing instrument is wrong for %d of %d sources"
                         % (len(mismatch), checked))

    # ---- rule 3, the fallback
    pairs = {}
    for vs, fs, how in real:
        pairs.setdefault((vs, fs), [])
        if how not in pairs[(vs, fs)]:
            pairs[(vs, fs)].append(how)
    paired_fs = {fs for _, fs in pairs}
    fs_unreadable = []
    for fs in fss:
        if fs in paired_fs:
            continue
        ins = recs[fs]["ins"]
        if ins is None:
            fs_unreadable.append((fs, recs[fs]["error"]))
            continue
        d = os.path.dirname(fs)
        for vs in vss:
            outs = recs[vs]["outs"]
            if os.path.dirname(vs) in (d, "") and outs is not None and ins <= outs:
                pairs.setdefault((vs, fs), []).append("varying cover")
    paired_fs = {fs for _, fs in pairs}
    no_partner = sorted(fs for fs in fss if fs not in paired_fs
                        and fs not in {f for f, _ in fs_unreadable})
    unpaired_vs = sorted(set(vss) - {v for v, _ in pairs})

    if KNOWN_POSITIVE not in pairs:
        raise SystemExit("glsl2v3d: FAIL: known positive %s + %s was not paired" % KNOWN_POSITIVE)

    # ---- compile
    order = sorted(pairs)
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as ex:
        results = list(ex.map(lambda p: run_pair(p[0], p[1], recs, args.keep), order))
    for (vs, fs), r in zip(order, results):
        r["how"] = pairs[(vs, fs)]
        r["known_uncompilable"] = sorted(x for x in (vs, fs) if x in bad)

    if not order or len(results) != len(order) \
            or any(r["status"] not in ("ok", "fail") for r in results):
        raise SystemExit("glsl2v3d: FAIL: formed %d pairs, processed %d"
                         % (len(order), len(results)))

    kpr = results[order.index(KNOWN_POSITIVE)]
    got = {st: (kpr["stats"] or {}).get(st, {}).get("instructions") for st in STAGES} \
        if kpr["stats"] else None
    if kpr["status"] != "ok" or got != KNOWN_POSITIVE_COUNTS:
        raise SystemExit("glsl2v3d: FAIL: known positive %s + %s: %s %s (want %s)"
                         % (KNOWN_POSITIVE + (kpr["status"], kpr["message"] or got,
                                              KNOWN_POSITIVE_COUNTS)))

    # ---- report
    print("build: v3d_shc %s" % V3D_SHC)
    for r in results:
        name = "%s + %s" % (r["vs"], r["fs"])
        kb = "  [known-uncompilable: %s]" % ", ".join(r["known_uncompilable"]) \
            if r["known_uncompilable"] else ""
        if r["status"] == "ok":
            print("ok   %-66s %s" % (name, "  ".join(
                "%s %s/%s/%s" % (st.upper(), stat(r, st, "instructions"), stat(r, st, "threads"),
                                 stat(r, st, "spill_size")) for st in STAGES)) + kb)
        else:
            print("FAIL %-66s %s%s" % (name, r["stage"], kb))
    print("(ok rows: instructions/threads/spill_size per stage)")

    fails = [r for r in results if r["status"] == "fail"]
    hist = {}
    for r in fails:
        hist.setdefault((r["stage"], r["message"]), []).append(r)
    print()
    print("=== failures keyed by stage and whole message: %d distinct ===" % len(hist))
    for (stage, msg), rs in sorted(hist.items(), key=lambda kv: (-len(kv[1]), kv[0])):
        kb = sum(1 for r in rs if r["known_uncompilable"])
        print("--- %d x [%s]%s  e.g. %s + %s" % (
            len(rs), stage, " (%d with a known-uncompilable source)" % kb if kb else "",
            rs[0]["vs"], rs[0]["fs"]))
        for ln in msg.splitlines():
            print("    " + ln)

    # A second view, derived from the first and labelled as such: the same
    # messages with every digit run masked, so that rows differing only in an
    # SSA index or a line number can be read together. The rows above remain
    # the measurement; this never replaces them.
    masked = {}
    for (stage, msg), rs in hist.items():
        masked.setdefault((stage, re.sub(r"\d+", "N", msg)), []).append(rs)
    print()
    print("=== derived: the rows above with digit runs masked: %d groups ===" % len(masked))
    for (stage, msg), groups in sorted(masked.items(), key=lambda kv: -sum(map(len, kv[1]))):
        rs = [r for g in groups for r in g]
        side = {"glslang vs": "vs", "glslang fs": "fs"}.get(stage)
        srcs = len({r[side] for r in rs}) if side else None
        print("--- %d pairs, %d distinct messages%s [%s]  e.g. %s + %s" % (
            len(rs), len(groups),
            ", %d distinct %s sources" % (srcs, side.upper()) if side else "",
            stage, rs[0]["vs"], rs[0]["fs"]))
        for ln in msg.splitlines():
            print("    " + ln)

    def rule_of(r):
        return "gl_test" if r["how"][0].startswith("gl_test") else r["how"][0]
    rule = collections.Counter(rule_of(r) for r in results)
    rule_ok = collections.Counter(rule_of(r) for r in results if r["status"] == "ok")
    stages = collections.Counter(r["stage"] for r in fails)
    ok = len(results) - len(fails)
    print()
    print("=== summary ===")
    print("corpus: %d sources (%d FS, %d VS); varying names checked against SPIR-V for %d"
          % (len(files), len(fss), len(vss), checked))
    print("pairs formed: %d (by first rule, ok/formed: %s)" % (
        len(order), ", ".join("%s %d/%d" % (k, rule_ok[k], n) for k, n in sorted(rule.items()))))
    print("pairs processed: %d" % len(results))
    print("pairs ok: %d" % ok)
    print("pairs failed: %d (%s)" % (len(fails), ", ".join(
        "%s %d" % kv for kv in sorted(stages.items()))))
    print("FS with no VS partner under any rule: %d" % len(no_partner))
    for f in no_partner:
        print("    " + f + ("  [known-uncompilable]" if f in bad else ""))
    print("FS whose inputs could not be read (translator refused; not paired): %d"
          % len(fs_unreadable))
    for f, e in fs_unreadable:
        print("    %s: %s" % (f, e))
    print("VS in no pair: %d" % len(unpaired_vs))
    for f in unpaired_vs:
        print("    " + f + ("  [known-uncompilable]" if f in bad else ""))

    if args.json:
        with open(args.json, "w") as f:
            json.dump({"v3d_shc": V3D_SHC, "pairs": results, "no_partner_fs": no_partner,
                       "fs_inputs_unreadable": fs_unreadable, "unpaired_vs": unpaired_vs},
                      f, indent=1)


if __name__ == "__main__":
    main()
