#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The corpus differential of the Mali compiler: every program of the corpus
through our compiler and through Mesa 26.2.1, one table per commit.

    python3 -I tools/mali/corpus_diff.py <out> [--deqp RUN] [--glmark2 DIR]
        [--shaders DIR]... [--pairs DIR]... [--jobs N] [--ref-cache DIR]
        [--no-mesa] [--stage all|gather|ref|measure|table]

--jobs N (default 8): Mesa draws and test processes at once.

The unit is a program (a vertex and a fragment shader), not one shader:
Mesa compiles a program, and a vertex shader's words depend on what its
fragment shader reads. Each program gets one verdict:

  equal    ours compiles it to Mesa's words and facts, in every region
           (built as glCompileShader and glLinkProgram build it: compile,
           the vertex shader again for the fragment shader, link)
  differ   ours compiles it to other words or facts; the detail lists
           each difference (`fs.fragment: 8 words ours, 7 reference`)
  refused  ours refuses it; the detail is the code generator's text, or
           the log when another part refused
  noref    ours compiles it and Mesa gave no reference (the detail says
           why: Mesa refused the program, or its draw failed)
  invalid  both refuse it, ours before the code generator (a dEQP negative
           case: the front end or the link rejects it, as Mesa does)
  error    the compare could not run (it panicked, or the process died
           in this program); the detail says which

The corpus (--stage gather, into <out>/pairs/ and <out>/corpus.tsv):

  --deqp RUN     the run directory of tools/mali/deqp-host-measure.sh
                 (its programs.tsv and programs/: every program of every
                 case; --cases tools/cts/gles2-mustpass.txt measures the
                 whole tree). The group of a program is the dEQP group
                 (`shaders.operator`) of most of its cases.
  --glmark2 DIR  the glmark2 corpus of tools/hostharness/glmark2/run.sh
                 (index.txt; never committed, it is GPL-3.0 text), paired
                 by the PROGRAMS table of tests/mali_glmark2_corpus.rs.
  --shaders DIR  a tree of single shaders (tools/shaders), paired per
                 directory: <stem>.vert with <stem>.frag; a fragment shader
                 without one with the directory's only vertex shader; any
                 other shader with a generated partner that writes or reads
                 each of its varyings (partner_for). A desktop GLSL source
                 (a `#version` other than 100 or 300 es) is skipped.
  --pairs DIR    <name>.vert beside <name>.frag, as they are.

A program is named by the first 16 hex digits of the SHA-256 of
"<vs>\\0<fs>" (each text ending in a newline), as deqp-host-measure.sh
names its programs, so one text pair is compiled once whatever its corpora.

Mesa's words (--stage ref): tools/mali/deqp_ref.sh --instrument shim per
program (the drm-shim draw with --uniform-values, the audit's method,
docs/notes/mali-deqp-compiled-audit.md section 2), kept in --ref-cache
(default build/mali-corpus-ref/<digest>/, the digest of the reference
tools and Mesa's UPSTREAM: a reference does not depend on our commit, so
the next commit makes only the new programs'). The shim is MESA_SHIM_WORK,
else $CONNECTED_DEPS_DIR/mesa-shim (deqp_ref.sh builds it when absent).
--no-mesa makes none (a program without a cached reference is noref).

Ours (--stage measure): tests/mali_corpus_diff.rs in N processes, each on
a share of the programs, into <out>/measure.tsv. Its compare is
tests/mali_ref's (the deqp_* tests' and the audit's), whose uniform words
follow gl_ref.py's for every GL type; tools/mali/route.sh's compare reads
every uniform as a float and no struct member, so an int or struct uniform
would differ there by its FAU words alone. A process that dies is read as
dying in the program after its last line (error), and the rest of its
share runs in a new process.

The table (--stage table): <out>/table.tsv, one row per corpus and
program: corpus, group, program, uses (dEQP cases, glmark2 scenes, or 1),
verdict, name (an example case or the file paths), detail. Sorted, so two
runs of one commit give the same bytes. <out>/summary.md: the counts per
verdict per corpus (programs and uses), per dEQP group, the refusal texts
ranked, and where the differing programs first differ. The last line of
the output is the summary line.

Standard library only (and cargo).
"""
import argparse
import collections
import concurrent.futures
import hashlib
import os
import re
import shutil
import subprocess
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
GLMARK2_TEST = os.path.join(ROOT, "tests", "mali_glmark2_corpus.rs")
# the files a reference depends on: a change to one makes a new cache
REF_TOOLS = ("tools/mali/deqp_ref.sh", "tools/mali/drm_shim_ref.sh", "tools/mali/gl_ref.py",
             "tools/mali/pan_ref_diff.py", "third_party/mesa-26.2.1/UPSTREAM")
VERDICTS = ("equal", "differ", "refused", "noref", "invalid", "error")
FEATURES = "host-test,gpu-mali,shaderc-online,no-shader-table"
CORPORA = ("deqp", "glmark2", "shaders", "pairs")


# ---- names -----------------------------------------------------------------

def nl(text):
    return text if text.endswith("\n") else text + "\n"


def program_key(vs, fs):
    """The program's name: deqp-host-measure.sh's (16 hex digits)."""
    return hashlib.sha256((nl(vs) + "\0" + nl(fs)).encode()).hexdigest()[:16]


def deqp_group(case):
    """`dEQP-GLES2.functional.shaders.operator.x` -> `shaders.operator`."""
    return ".".join(case.split(".")[2:4])


# ---- partners for single shaders --------------------------------------------

def strip_comments(text):
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def glsl_version(text):
    """'100', '300 es', another version text, or '100' when none."""
    m = re.search(r"^\s*#\s*version\s+([^\n]*)", strip_comments(text), re.M)
    return " ".join(m.group(1).split()) if m else "100"


PRECISION = ("highp", "mediump", "lowp")
QUAL = r"(?:(?:flat|smooth|centroid|invariant|highp|mediump|lowp)\s+)*"
# a varying declaration at global scope: qualifiers, the storage word, more
# qualifiers, the type, then the names (`a`, `b[2]`)
RE_VARYING = re.compile(r"^\s*(" + QUAL + r")(varying|in|out)\s+(" + QUAL + r")(\w+)\s+([^;]+);", re.M)
RE_NAME = re.compile(r"^\s*(\w+)\s*(?:\[\s*(\d+)\s*\])?\s*$")
# a value of the type from one vec4 attribute `a`, and the type made a vec4
FROM_VEC4 = {
    "float": "{a}.x", "vec2": "{a}.xy", "vec3": "{a}.xyz", "vec4": "{a}",
    "int": "int({a}.x)", "ivec2": "ivec2({a}.xy)", "ivec3": "ivec3({a}.xyz)", "ivec4": "ivec4({a})",
    "uint": "uint({a}.x)", "uvec2": "uvec2({a}.xy)", "uvec3": "uvec3({a}.xyz)", "uvec4": "uvec4({a})",
    "mat2": "mat2({a})", "mat3": "mat3({a}.x)", "mat4": "mat4({a}.x)",
}
TO_VEC4 = {
    "float": "vec4({v})", "vec2": "vec4({v}, 0.0, 0.0)", "vec3": "vec4({v}, 0.0)", "vec4": "{v}",
    "int": "vec4(float({v}))", "ivec2": "vec4(vec2({v}), 0.0, 0.0)", "ivec3": "vec4(vec3({v}), 0.0)",
    "ivec4": "vec4({v})", "uint": "vec4(float({v}))", "uvec2": "vec4(vec2({v}), 0.0, 0.0)",
    "uvec3": "vec4(vec3({v}), 0.0)", "uvec4": "vec4({v})",
    "mat2": "vec4({v}[0], {v}[1])", "mat3": "vec4({v}[0] + {v}[1] + {v}[2], 0.0)",
    "mat4": "{v}[0] + {v}[1] + {v}[2] + {v}[3]",
}


def varyings(text, stage):
    """[(leading qualifiers, precision, type, [(name, size or None)])] of
    the shader's varyings (centroid dropped):
    `varying`, or in ES 3.00 the vertex shader's `out` / the fragment
    shader's `in`. None when one is of a type partner_for cannot make."""
    es3 = glsl_version(text) == "300 es"
    want = ("out" if stage == "vert" else "in") if es3 else "varying"
    out = []
    for m in RE_VARYING.finditer(strip_comments(text)):
        if m.group(2) != want:
            continue
        typ = m.group(4)
        if typ not in FROM_VEC4:
            return None
        names = []
        for n in m.group(5).split(","):
            nm = RE_NAME.match(n)
            if not nm:
                return None
            names.append((nm.group(1), int(nm.group(2)) if nm.group(2) else None))
        quals = (m.group(1) + " " + m.group(3)).split()
        # the order GLSL ES wants: interpolation and invariant before the
        # storage word, the precision after it
        lead = [q for q in quals if q not in PRECISION and q != "centroid"]
        prec = [q for q in quals if q in PRECISION]
        out.append((lead, prec, typ, names))
    return out


def partner_for(text, stage):
    """A partner for a single shader of `stage` ('vert' or 'frag'): a
    fragment shader that reads every varying the vertex shader writes and
    sums them into its colour, or a vertex shader that writes every varying
    the fragment shader reads, each from its own attribute (so no two are
    equal and none is a constant Mesa could fold across the stages). None
    when a varying is of a type it cannot make."""
    vary = varyings(text, stage)
    if vary is None:
        return None
    es3 = glsl_version(text) == "300 es"
    head = "#version 300 es\n" if es3 else "#version 100\n"
    lines, body = [], []
    elems = [(q, p, t, n, s) for q, p, t, names in vary for n, s in names]
    def decl(q, p, t, n, s, word):
        return " ".join(q + [word] + p + [t, n + ("[%d]" % s if s else "")]) + ";"
    if stage == "frag":
        attr = "in" if es3 else "attribute"
        lines.append("%s vec4 yk_pos;" % attr)
        k = 0
        for q, p, t, n, s in elems:
            lines.append(decl(q, p, t, n, s, "out" if es3 else "varying"))
            for i in range(s or 1):
                lines.append("%s vec4 yk_a%d;" % (attr, k))
                body.append("    %s%s = %s;" % (n, "[%d]" % i if s else "", FROM_VEC4[t].format(a="yk_a%d" % k)))
                k += 1
        body.append("    gl_Position = yk_pos;")
    else:
        lines.append("precision highp float;")
        for q, p, t, n, s in elems:
            lines.append(decl(q, p, t, n, s, "in" if es3 else "varying"))
        if es3:
            lines.append("out vec4 yk_color;")
        body.append("    vec4 c = vec4(0.0);")
        for _q, _p, t, n, s in elems:
            for i in range(s or 1):
                body.append("    c += %s;" % TO_VEC4[t].format(v=n + ("[%d]" % i if s else "")))
        body.append("    %s = c;" % ("yk_color" if es3 else "gl_FragColor"))
    return head + "\n".join(lines) + "\nvoid main()\n{\n" + "\n".join(body) + "\n}\n"


# ---- the corpora -------------------------------------------------------------

class Corpus:
    """The programs: {key: (vs, fs)} and the rows {(corpus, key): info}."""

    def __init__(self):
        self.texts = {}
        self.rows = {}
        self.skipped = collections.Counter()

    def add(self, corpus, vs, fs, group, uses, name):
        key = program_key(vs, fs)
        self.texts[key] = (nl(vs), nl(fs))
        r = self.rows.get((corpus, key))
        if r is None:
            self.rows[(corpus, key)] = {"group": group, "uses": uses, "name": name}
        else:
            r["uses"] += uses
        return key


def read(path):
    with open(path, errors="replace") as f:
        return f.read()


def gather_deqp(corpus, run):
    tsv = os.path.join(run, "programs.tsv")
    if not os.path.exists(tsv):
        raise SystemExit("corpus_diff: %s has no programs.tsv (run tools/mali/deqp-host-measure.sh "
                         "of this tree, its extract stage writes it)" % run)
    cases = collections.OrderedDict()
    with open(tsv) as f:
        next(f)
        for line in f:
            key, case = line.rstrip("\n").split("\t")
            cases.setdefault(key, []).append(case)
    for key, cs in cases.items():
        base = os.path.join(run, "programs", key)
        vs, fs = read(base + ".vert"), read(base + ".frag")
        if program_key(vs, fs) != key:
            raise SystemExit("corpus_diff: %s.vert/.frag do not hash to their name" % base)
        groups = collections.Counter(deqp_group(c) for c in cs)
        top = max(groups.values())
        group = next(deqp_group(c) for c in cs if groups[deqp_group(c)] == top)
        corpus.add("deqp", vs, fs, group, len(cs), cs[0])


def glmark2_programs(path=GLMARK2_TEST):
    """(ENTRIES, PROGRAMS) of tests/mali_glmark2_corpus.rs: the entries in
    order and per entry its `<vs>+<fs>` programs by four hex digits."""
    text = read(path)
    ent = re.search(r"const ENTRIES: \[&str; \d+\] = \[(.*?)\];", text, re.S)
    prog = re.search(r"const PROGRAMS: \[&\[&str\]; \d+\] = \[(.*?)\n\];", text, re.S)
    if not ent or not prog:
        raise SystemExit("corpus_diff: no ENTRIES or PROGRAMS table in %s" % path)
    entries = re.findall(r'"([^"]*)"', ent.group(1))
    programs = [re.findall(r'"([0-9a-f]{4}\+[0-9a-f]{4})"', p) for p in re.findall(r"&\[(.*?)\]", prog.group(1), re.S)]
    if len(entries) != len(programs):
        raise SystemExit("corpus_diff: %s: %d entries, %d program lists" % (path, len(entries), len(programs)))
    return entries, programs


def gather_glmark2(corpus, d):
    entries, programs = glmark2_programs()
    files = collections.defaultdict(list)
    for line in read(os.path.join(d, "index.txt")).splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        f, es = line.split(": ", 1)
        for e in es.split("; "):
            files[e].append(f)
    for e, progs in zip(entries, programs):
        def find(stage, h, e=e):
            of = [f for f in files[e] if (".vert." in f) == (stage == "vs")
                  and f.rsplit("-", 1)[-1].startswith(h)]
            if len(of) != 1:
                raise SystemExit("corpus_diff: glmark2 entry %s: %d %s files with hash %s" % (e, len(of), stage, h))
            return of[0]
        for p in progs:
            vh, fh = p.split("+")
            v, f = find("vs", vh), find("fs", fh)
            corpus.add("glmark2", read(os.path.join(d, v)), read(os.path.join(d, f)),
                       e.split(":")[0], 1, "%s %s" % (e, p))


def gather_shaders(corpus, top):
    top = os.path.normpath(top)
    rel = os.path.relpath(top, ROOT)
    name_of = (lambda p: os.path.join(rel, os.path.relpath(p, top))) if not rel.startswith("..") else (lambda p: p)
    for d, _dirs, names in sorted(os.walk(top)):
        _dirs.sort()
        shaders = {}
        for n in sorted(names):
            stem, ext = os.path.splitext(n)
            if ext not in (".vert", ".frag"):
                continue
            p = os.path.join(d, n)
            text = read(p)
            v = glsl_version(text)
            if v not in ("100", "300 es"):
                corpus.skipped["shaders: desktop GLSL (#version %s)" % v] += 1
                continue
            shaders[(stem, ext[1:])] = (p, text)
        verts = [k for k in shaders if k[1] == "vert"]
        used = set()
        group = os.path.relpath(d, top) if d != top else "."
        for (stem, st) in sorted(shaders):
            if st != "vert" or (stem, "frag") not in shaders or \
                    glsl_version(shaders[(stem, "vert")][1]) != glsl_version(shaders[(stem, "frag")][1]):
                continue
            (vp, vt), (fp, ft) = shaders[(stem, "vert")], shaders[(stem, "frag")]
            corpus.add("shaders", vt, ft, group, 1, "%s + %s" % (name_of(vp), name_of(fp)))
            used |= {(stem, "vert"), (stem, "frag")}
        # the fragment shaders first: one paired with the directory's only
        # vertex shader uses it
        for k in sorted(shaders, key=lambda k: (k[1] == "vert", k)):
            if k in used:
                continue
            p, t = shaders[k]
            if k[1] == "frag" and len(verts) == 1 and glsl_version(shaders[verts[0]][1]) == glsl_version(t):
                vp, vt = shaders[verts[0]]
                corpus.add("shaders", vt, t, group, 1, "%s + %s" % (name_of(vp), name_of(p)))
                used.add(verts[0])
                continue
            partner = partner_for(t, k[1])
            if partner is None:
                corpus.skipped["shaders: a varying of a type the partner cannot make"] += 1
                continue
            vs, fs = (t, partner) if k[1] == "vert" else (partner, t)
            corpus.add("shaders", vs, fs, group, 1, "%s + partner" % name_of(p))


def gather_pairs(corpus, d):
    for n in sorted(os.listdir(d)):
        if n.endswith(".vert") and os.path.exists(os.path.join(d, n[:-5] + ".frag")):
            base = os.path.join(d, n[:-5])
            corpus.add("pairs", read(base + ".vert"), read(base + ".frag"), os.path.basename(os.path.normpath(d)),
                       1, os.path.basename(base))


def write_corpus(corpus, out):
    pd = os.path.join(out, "pairs")
    if os.path.isdir(pd):
        shutil.rmtree(pd)
    os.makedirs(pd)
    for key, (vs, fs) in corpus.texts.items():
        for ext, t in ((".vert", vs), (".frag", fs)):
            with open(os.path.join(pd, key + ext), "w") as f:
                f.write(t)
    with open(os.path.join(out, "corpus.tsv"), "w") as f:
        f.write("corpus\tgroup\tprogram\tuses\tname\n")
        for (c, key), r in sorted(corpus.rows.items(), key=lambda kv: (CORPORA.index(kv[0][0]), kv[1]["group"], kv[0][1])):
            f.write("%s\t%s\t%s\t%d\t%s\n" % (c, r["group"], key, r["uses"], r["name"]))
    with open(os.path.join(out, "skipped.tsv"), "w") as f:
        f.write("reason\tshaders\n")
        for reason, n in sorted(corpus.skipped.items()):
            f.write("%s\t%d\n" % (reason, n))


# ---- Mesa's words --------------------------------------------------------------

def ref_cache_dir(given):
    if given:
        return given
    h = hashlib.sha256()
    for p in REF_TOOLS:
        with open(os.path.join(ROOT, p), "rb") as f:
            h.update(f.read())
    return os.path.join(ROOT, "build", "mali-corpus-ref", h.hexdigest()[:12])


def flat(s):
    return " ".join(s.split())


def ref_failure(stderr, rc):
    """One line on why deqp_ref.sh made no reference: gl_ref.py's own
    message, a GLSL error, or Mesa's abort; directories dropped."""
    lines = [l.strip() for l in stderr.splitlines() if l.strip() and not l.startswith("drm_shim_ref: the draw failed")]
    if re.search(r"Aborted|core dumped", stderr):
        # the draw's process died (an assertion or the IR validator of
        # Mesa's debug build; its text is in the draw's stdout, not kept)
        return "Mesa aborted (the draw's process died)"
    why = [l for l in lines if re.search(r"^gl_ref:|error|Traceback", l)]
    why = [re.sub(r"(?:/[^\s/]+)+/", "", l) for l in why[-3:]]
    return flat(" / ".join(why))[:400] or "deqp_ref.sh exit %d" % rc


def make_ref(key, pairs, cache, tmp):
    """Mesa's reference of one program into the cache: <key>.expect.json,
    or <key>.noref with the reason. A program whose uniforms gl_ref.py
    cannot set is drawn again with every uniform 0, and <key>.zero-uniforms
    marks it so (tests/mali_corpus_diff.rs compares it with every uniform
    word 0)."""
    prefix = os.path.join(tmp, key)
    env = dict(os.environ, TMPDIR=tmp, LC_ALL="C")
    cmd = [os.path.join(HERE, "deqp_ref.sh"), os.path.join(pairs, key + ".vert"),
           os.path.join(pairs, key + ".frag"), prefix, "--instrument", "shim"]
    r = subprocess.run(cmd, env=env, capture_output=True, text=True, errors="replace")
    zero = r.returncode != 0 and "--uniform-values does not set" in r.stderr
    if zero:
        r = subprocess.run(cmd + ["--zero-uniforms"], env=env, capture_output=True, text=True, errors="replace")
    if r.returncode == 0 and os.path.exists(prefix + ".expect.json"):
        if zero:
            open(os.path.join(cache, key + ".zero-uniforms"), "w").close()
        shutil.move(prefix + ".expect.json", os.path.join(cache, key + ".expect.json"))
    else:
        with open(os.path.join(cache, key + ".noref"), "w") as f:
            f.write(ref_failure(r.stderr, r.returncode) + "\n")
    for ext in (".vert", ".frag", ".expect.json"):
        if os.path.exists(prefix + ext):
            os.remove(prefix + ext)


def stage_ref(out, cache, jobs, mesa):
    pairs = os.path.join(out, "pairs")
    keys = sorted(n[:-5] for n in os.listdir(pairs) if n.endswith(".vert"))
    os.makedirs(cache, exist_ok=True)
    todo = [k for k in keys if not os.path.exists(os.path.join(cache, k + ".expect.json"))
            and not os.path.exists(os.path.join(cache, k + ".noref"))]
    print("corpus_diff: %d programs, %d references cached in %s, %d to make%s"
          % (len(keys), len(keys) - len(todo), cache, len(todo), "" if mesa else " (--no-mesa: none made)"))
    if mesa and todo:
        tmp = tempfile.mkdtemp(prefix="corpus_diff.", dir=out)
        try:
            with concurrent.futures.ThreadPoolExecutor(jobs) as ex:
                for _ in ex.map(lambda k: make_ref(k, pairs, cache, tmp), todo):
                    pass
        finally:
            shutil.rmtree(tmp, ignore_errors=True)
    # the references beside the pairs, as tests/mali_corpus_diff.rs reads them
    n = 0
    for k in keys:
        for ext in (".expect.json", ".zero-uniforms"):
            dst = os.path.join(pairs, k + ext)
            if os.path.exists(dst):
                os.remove(dst)
            if os.path.exists(os.path.join(cache, k + ext)):
                shutil.copyfile(os.path.join(cache, k + ext), dst)
        n += os.path.exists(os.path.join(pairs, k + ".expect.json"))
    print("corpus_diff: %d of %d programs have Mesa's reference" % (n, len(keys)))


# ---- ours ----------------------------------------------------------------------------

def cargo_test(run=True):
    """The command of tests/mali_corpus_diff.rs (one process)."""
    cmd = ["cargo", "test", "-q", "--no-default-features", "--features", FEATURES, "--test", "mali_corpus_diff"]
    return cmd + (["--", "--nocapture", "--test-threads", "1"] if run else ["--no-run"])


def measure_shares(pairs, names, jobs, tmp):
    """{name: (WORD, detail)} of tests/mali_corpus_diff.rs over names, in
    `jobs` processes at once. A process that dies leaves the rest of its
    share: the program after its last line is an ERROR, the rest run again."""
    results = {}
    pending = list(names)
    rnd = 0
    while pending:
        shares = [pending[k::jobs] for k in range(jobs) if pending[k::jobs]]
        procs = []
        for k, share in enumerate(shares):
            lst = os.path.join(tmp, "list.%d.%d" % (rnd, k))
            with open(lst, "w") as f:
                f.write("\n".join(share) + "\n")
            env = dict(os.environ, YAKOGL_MALI_CORPUS_DIR=pairs, YAKOGL_MALI_CORPUS_LIST=lst)
            o = open(os.path.join(tmp, "out.%d.%d" % (rnd, k)), "w+")
            e = open(os.path.join(tmp, "err.%d.%d" % (rnd, k)), "w+")
            procs.append((share, subprocess.Popen(cargo_test(), cwd=ROOT, env=env, stdout=o, stderr=e), o, e))
        pending = []
        for share, p, o, e in procs:
            rc = p.wait()
            o.seek(0)
            for line in o:
                f = line.rstrip("\n").split("\t", 3)
                if len(f) == 4 and f[0] == "corpus":
                    results[f[1]] = (f[2], f[3])
            left = [n for n in share if n not in results]
            e.seek(0)
            err = e.read()
            o.close()
            e.close()
            if left:
                tail = flat(" / ".join(err.splitlines()[-2:]))[:200]
                results[left[0]] = ("ERROR", "the process died in this program (exit %d): %s" % (rc, tail))
                pending += left[1:]
            elif rc != 0:
                raise SystemExit("corpus_diff: tests/mali_corpus_diff.rs exit %d:\n%s" % (rc, err[-2000:]))
        rnd += 1
    return results


def stage_measure(out, jobs):
    pairs = os.path.join(out, "pairs")
    names = sorted(n[:-5] for n in os.listdir(pairs) if n.endswith(".vert"))
    # one build, then the shares at once
    if subprocess.run(cargo_test(run=False), cwd=ROOT).returncode != 0:
        raise SystemExit("corpus_diff: tests/mali_corpus_diff.rs does not build")
    tmp = tempfile.mkdtemp(prefix="corpus_diff.", dir=out)
    try:
        results = measure_shares(pairs, names, jobs, tmp)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    with open(os.path.join(out, "measure.tsv"), "w") as f:
        for n in names:
            f.write("%s\t%s\t%s\n" % (n, results[n][0], results[n][1]))
    print("corpus_diff: %d programs measured: %s" % (len(names), ", ".join(
        "%s %d" % kv for kv in sorted(collections.Counter(w for w, _ in results.values()).items()))))


# ---- the table -----------------------------------------------------------------------

def first_difference(detail):
    """The kind of a DIFFER detail's first difference: its region and what
    differs, numbers as N (`fs.fragment: N words ours, N reference`)."""
    first = detail.split("; ")[0]
    first = re.sub(r" ours \[.*", " ours [...]", first)
    return re.sub(r"0x[0-9a-f]+|\d+", "N", first)[:100]


def verdict_of(measured, noref):
    """(verdict, detail) of one program from its measure.tsv word and
    detail and Mesa's reason for no reference (None: Mesa has one)."""
    word, detail = measured
    if word == "REFUSED":
        # the Mali back end's refusal (every MaliError text names Valhall) is
        # ours whatever Mesa did; one before it (the front end, the link),
        # with Mesa's, is an invalid program
        if noref is None or "Valhall" in detail:
            return "refused", detail if noref is None else "%s | Mesa: %s" % (detail, noref)
        return "invalid", "%s | Mesa: %s" % (detail, noref)
    if word == "EQUAL":
        return "equal", "-"
    if word == "DIFFER":
        return "differ", detail
    if word == "NOREF":
        return "noref", noref if noref is not None else "no reference made"
    if word == "ERROR":
        return "error", detail
    raise SystemExit("corpus_diff: measure.tsv word %r" % word)


def stage_table(out, cache):
    measured = {}
    with open(os.path.join(out, "measure.tsv")) as f:
        for line in f:
            p = line.rstrip("\n").split("\t", 2)
            if len(p) == 3:
                measured[p[0]] = (p[1], p[2])
    rows = []
    with open(os.path.join(out, "corpus.tsv")) as f:
        next(f)
        for line in f:
            c, g, key, uses, name = line.rstrip("\n").split("\t", 4)
            if key not in measured:
                raise SystemExit("corpus_diff: program %s has no measure.tsv row" % key)
            nr = os.path.join(cache, key + ".noref")
            noref = read(nr).strip() if os.path.exists(nr) else None
            if noref is None and not os.path.exists(os.path.join(out, "pairs", key + ".expect.json")):
                noref = "no reference made"
            v, detail = verdict_of(measured[key], noref)
            rows.append((c, g, key, int(uses), v, name, flat(detail)))
    rows.sort(key=lambda r: (CORPORA.index(r[0]), r[1], r[2]))
    with open(os.path.join(out, "table.tsv"), "w") as f:
        f.write("corpus\tgroup\tprogram\tuses\tverdict\tname\tdetail\n")
        for r in rows:
            f.write("%s\t%s\t%s\t%d\t%s\t%s\t%s\n" % r)
    summary = summarise(rows, out)
    with open(os.path.join(out, "summary.md"), "w") as f:
        f.write(summary)
    line = summary_line(rows)
    print(line)
    return rows


def counts(rows):
    p, u = collections.Counter(), collections.Counter()
    for r in rows:
        p[r[4]] += 1
        u[r[4]] += r[3]
    return p, u


def summary_line(rows):
    p, _ = counts(rows)
    keys = sorted({r[2] for r in rows})
    return "corpus_diff: %d rows, %d programs: %s" % (
        len(rows), len(keys), ", ".join("%s %d" % (v, p[v]) for v in VERDICTS))


def summarise(rows, out):
    s = ["# The Mali corpus differential", ""]
    commit = os.path.join(out, "commit")
    if os.path.exists(commit):
        s += ["```", read(commit).strip(), "```", ""]
    s += ["One row per corpus and program (`table.tsv`). Uses: dEQP cases, glmark2 entries, or 1.", "",
          "| Corpus | Programs | " + " | ".join(VERDICTS) + " | Uses | " + " | ".join("%s (uses)" % v for v in VERDICTS) + " |",
          "|---|--:|" + "--:|" * len(VERDICTS) + "--:|" + "--:|" * len(VERDICTS)]
    for c in CORPORA + ("all",):
        rs = [r for r in rows if c in (r[0], "all")]
        if not rs:
            continue
        p, u = counts(rs)
        s.append("| %s | %d | %s | %d | %s |" % (c, len(rs), " | ".join(str(p[v]) for v in VERDICTS),
                                                sum(u.values()), " | ".join(str(u[v]) for v in VERDICTS)))
    deqp = [r for r in rows if r[0] == "deqp"]
    if deqp:
        s += ["", "## Per dEQP group (programs; cases in brackets)", "",
              "| Group | Programs | " + " | ".join(VERDICTS) + " |", "|---|--:|" + "--:|" * len(VERDICTS)]
        groups = collections.defaultdict(list)
        for r in deqp:
            groups[r[1]].append(r)
        for g in sorted(groups, key=lambda g: (-len(groups[g]), g)):
            p, u = counts(groups[g])
            s.append("| %s | %d | %s |" % (g, len(groups[g]), " | ".join("%d (%d)" % (p[v], u[v]) for v in VERDICTS)))
    ref = collections.Counter()
    refu = collections.Counter()
    for r in rows:
        if r[4] == "refused":
            t = r[6][:300]
            ref[t] += 1
            refu[t] += r[3]
    if ref:
        s += ["", "## Refusal texts (rows; uses in brackets)", ""]
        s += ["- %d (%d): %s" % (n, refu[t], t) for t, n in sorted(ref.items(), key=lambda kv: (-kv[1], kv[0]))]
    diff = collections.Counter()
    for r in rows:
        if r[4] == "differ":
            diff[first_difference(r[6])] += 1
    if diff:
        s += ["", "## Where the differing programs first differ (rows)", ""]
        s += ["- %s: %d" % (k, n) for k, n in sorted(diff.items(), key=lambda kv: (-kv[1], kv[0]))]
    nore = collections.Counter(re.sub(r"[0-9a-f]{8,}|\d+", "N", r[6])[:120] for r in rows if r[4] == "noref")
    if nore:
        s += ["", "## No reference from Mesa (rows)", ""]
        s += ["- %d: %s" % (n, t) for t, n in sorted(nore.items(), key=lambda kv: (-kv[1], kv[0]))]
    sk = os.path.join(out, "skipped.tsv")
    if os.path.exists(sk):
        lines = read(sk).splitlines()[1:]
        if lines:
            s += ["", "## Skipped single shaders", ""] + ["- %s: %s" % tuple(l.split("\t")) for l in lines]
    s += ["", summary_line(rows), ""]
    return "\n".join(s)


# ---- main ------------------------------------------------------------------------------

def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("out")
    ap.add_argument("--deqp")
    ap.add_argument("--glmark2")
    ap.add_argument("--shaders", action="append", default=[])
    ap.add_argument("--pairs", action="append", default=[])
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--ref-cache")
    ap.add_argument("--no-mesa", action="store_true")
    ap.add_argument("--stage", default="all", choices=("all", "gather", "ref", "measure", "table"))
    a = ap.parse_args(argv)
    if a.jobs < 1:
        ap.error("--jobs must be 1 or more")
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    cache = os.path.abspath(ref_cache_dir(a.ref_cache))
    if a.stage in ("all", "gather"):
        if not (a.deqp or a.glmark2 or a.shaders or a.pairs):
            ap.error("gather needs a corpus: --deqp, --glmark2, --shaders or --pairs")
        corpus = Corpus()
        if a.deqp:
            gather_deqp(corpus, a.deqp)
        if a.glmark2:
            gather_glmark2(corpus, a.glmark2)
        for d in a.shaders:
            gather_shaders(corpus, d)
        for d in a.pairs:
            gather_pairs(corpus, d)
        write_corpus(corpus, out)
        commit = subprocess.run(["git", "-C", ROOT, "describe", "--always", "--dirty", "--abbrev=12"],
                                capture_output=True, text=True).stdout.strip() or "unknown"
        with open(os.path.join(out, "commit"), "w") as f:
            f.write("tip=%s\nref-cache=%s\ncorpora=%s\n" % (
                commit, os.path.basename(cache),
                " ".join(x for x in ("deqp" if a.deqp else "", "glmark2" if a.glmark2 else "") if x)
                + "".join(" shaders:%s" % os.path.basename(os.path.normpath(d)) for d in a.shaders)
                + "".join(" pairs:%s" % os.path.basename(os.path.normpath(d)) for d in a.pairs)))
        print("corpus_diff: %d programs in %d rows (%s)" % (
            len(corpus.texts), len(corpus.rows),
            ", ".join("%s %d" % (c, sum(1 for k in corpus.rows if k[0] == c)) for c in CORPORA
                      if any(k[0] == c for k in corpus.rows))))
    if a.stage in ("all", "ref"):
        stage_ref(out, cache, a.jobs, not a.no_mesa)
    if a.stage in ("all", "measure"):
        stage_measure(out, a.jobs)
    if a.stage in ("all", "table"):
        stage_table(out, cache)


if __name__ == "__main__":
    main()
