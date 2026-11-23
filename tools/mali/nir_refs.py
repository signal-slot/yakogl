#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Mesa's NIR for each committed reference pair, at two points, as SHA-256.

    tools/mali/nir_refs.sh [--build-dir DIR] [--texts DIR] [--jobs N] [--scratch DIR] [NAME...]
    tools/mali/nir_refs.sh normalise < dump.txt

NAME is a reference name (fbload_fbload) or a glob of names (deqp_arith_*);
default every tools/mali/testdata/<name>.expect.json with a <name>.vert and a
<name>.frag (the set tests/mali_ref/mod.rs reads). For each one the pair is
drawn twice on Mesa 26.2.1's drm-shim (drm_shim_ref.sh run, a debug build),
once with NIR_DEBUG=print_vs and once with NIR_DEBUG=print_fs, so Mesa
prints one stage's NIR passes on stdout, and with BIFROST_MESA_DEBUG=shaders
(drm_shim_ref.sh's own), so it prints the NIR at selection on stderr. The two
points (docs/notes/mali-shader-nir.md, 7.2):

  ssa     the last NIR Mesa prints for the stage before the stage's first
          `nir_opt_find_array_copies` line. NIR_PASS prints a pass's name
          before the pass, and the NIR after it only when the pass made
          progress (src/compiler/nir/nir.h:5264-5296), so the point is fixed
          by a name. nir_opt_find_array_copies is printed only by gl_nir_opts
          (src/compiler/glsl/gl_nir_linker.c:67), right after its
          nir_lower_vars_to_ssa and nir_remove_dead_variables (:55-66); for
          a vertex and fragment pair its first round is the one of
          gl_nir_link_varyings (gl_nir_link_varyings.c:4175). Locals are SSA
          values there and no optimisation of gl_nir_opts has run. The
          stream must fit this model, else the pair is an ERROR: the two
          names before the first nir_opt_find_array_copies are
          nir_lower_vars_to_ssa then nir_remove_dead_variables, and one
          glsl_to_nir name comes before them. With both stages printed the
          name lines of the two interleave and a name without a dump has no
          stage; that is why each stage gets its own draw.
  select  the NIR bi_compile_variant_nir prints before selection
          (src/panfrost/compiler/bifrost/bifrost_compile.c:4409-4415), one
          print per variant: vs.position and vs.varying for an IDVS vertex
          shader (bifrost_nir.c:1299-1305, in that order; a vertex shader
          without varyings is IDVS too, its varying print empty), vs.position
          alone for one compiled without IDVS (BI_IDVS_NONE), fs for the
          fragment shader. Both draws print it; the two must give the same
          texts.

The draw must be the reference's: its words (every region of the committed
expect.json, and secondary_offset) must equal the extract of the new trace,
in both draws. The option sets are tried in order until one does:
tools/mali/g310_check.py's four (`--uniform-values`, none, `--prim points`
with and without it; a draw without the values is the one for a uniform
gl_ref.py cannot set, such as a uvec2 or a sampler in a struct), then
`--uniform-values --mat-columns`, with `--attrib-bindings` from <name>.bind
when there is one. A pair no set reproduces is listed as NO-DRAW and gets no
file.

Each dump is normalised (normalise()): the header (every line before the
first decl_function) and every decl_var line dropped, blank lines dropped,
the name comments dropped (an IO intrinsic's `// <variable>`, a deref
chain's `// &<chain>`), the SSA names renumbered per function in order of
definition, at the ssa point the functions other than the entry point
dropped (entry_only(): Mesa has removed them there without a print; a
`call` is an error), the names of function_temp and shader_temp variables
(Mesa's own: vec_ctor, const_temp, `#4`, ...) renamed t0, t1, ... in order
of first use in the dump, and the white space of each line reduced to single spaces
(the printer pads columns to the widest SSA name, which renumbering
changes; nesting stays in the braces). Everything else is kept: sizes,
swizzles, intrinsic indices, constants, the names of uniform, input and
output variables, block names, preds and succs, fp_math_ctrl.

Output: tools/mali/testdata/<name>.nir.sha, one line per stage variant and
point, "<variant> <point> : <sha256 of the normalised text>", and for the
ssa point " : <pass>,<pass>,...", the passes that printed a dump between
glsl_to_nir's and the point (the passes that changed the shader), in order,
repeats kept ("-" for none):

    vs ssa : ... : nir_inline_functions,nir_split_var_copies,...
    vs.position select : ...
    vs.varying select : ...   (an IDVS vertex shader)
    fs ssa : ... : ...
    fs select : ...

With --texts DIR (or NIR_REFS_TEXTS), DIR/<name>/<variant>.<point>.nir holds
each normalised text and .raw the dump as Mesa printed it; nothing of it is
committed. The shim is --build-dir, else MESA_SHIM_WORK, else
$CONNECTED_DEPS_DIR/mesa-shim (tools/mali/deqp_ref.sh's rule); it is not
built here (drm_shim_ref.sh build). --jobs: pairs in parallel (default 8).
One line per pair (WROTE, SAME, NO-DRAW, ERROR), then
"N written, M unchanged, K no draw, E errors"; exit 1 on NO-DRAW or ERROR.
`normalise` reads one dump (from its `shader:` line) on stdin and prints its
normalised text.
"""
import concurrent.futures
import fnmatch
import glob
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "testdata")
# tools/mali/g310_check.py's sets in its order, then --mat-columns
OPTION_SETS = [["--uniform-values"], [], ["--prim", "points", "--uniform-values"], ["--prim", "points"],
               ["--uniform-values", "--mat-columns"]]
STAGES = {"MESA_SHADER_VERTEX": "vs", "MESA_SHADER_FRAGMENT": "fs"}


class DumpError(Exception):
    pass


PASS_NAME = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)(?: \(finished\))?$")


def events(lines):
    """A stream as [("pass", name) or ("dump", (stage, dump lines))].

    A dump starts at `shader: MESA_SHADER_<stage>`; its header and variables
    run to the first decl_function; then each function is its decl_function
    line, and for one with a body, a blank line and `impl <name> {` to `}`
    (nir_print.c:2625-2722). The dump ends at the first line after a
    function that is not the next decl_function. Outside a dump, a line that
    is one identifier is a pass name NIR_PASS printed (`<pass> (finished)`
    for a pass that ran another one, nir.h:5281-5282, gives <pass>); other
    lines (gl_ref.py's own) are skipped.
    """
    out = []
    i = 0
    n = len(lines)
    while i < n:
        if not lines[i].startswith("shader: MESA_SHADER_"):
            m = PASS_NAME.match(lines[i])
            if m:
                out.append(("pass", m.group(1)))
            i += 1
            continue
        start = i
        stage = lines[i][len("shader: "):].strip()
        i += 1
        while i < n and not lines[i].startswith("decl_function "):
            if lines[i].startswith("shader: "):
                raise DumpError("line %d: a dump with no function" % (start + 1))
            i += 1
        while i < n and lines[i].startswith("decl_function "):
            i += 1
            if i + 1 < n and lines[i] == "" and lines[i + 1].startswith("impl "):
                i += 2
                while i < n and lines[i] != "}":
                    if lines[i].startswith("shader: "):
                        raise DumpError("line %d: a function with no end" % (start + 1))
                    i += 1
                if i == n:
                    raise DumpError("line %d: a function with no end" % (start + 1))
                i += 1
                if i < n and lines[i] == "":
                    i += 1
        out.append(("dump", (stage, lines[start:i])))
    return out


def cut_dumps(lines):
    """Every NIR print in a stream: [(stage, dump lines)]."""
    return [v for k, v in events(lines) if k == "dump"]


DEF = re.compile(r"^(?:(?:div|con) )?\d+(?:x\d+)? %(\d+) = ")
# a deref of a variable local to the shader: `deref_var &<name> (function_temp ...`
TEMP = re.compile(r"(= deref_var &)(\S+)( \((?:function_temp|shader_temp)[ )])")
SSA = re.compile(r"%(\d+)\b")
NAME_COMMENT = re.compile(r"\s+// [^/]*$")
CHAIN_COMMENT = re.compile(r"\s+// &.*$")


def entry_only(dump):
    """The dump without the functions that are not the entry point.

    At the ssa point Mesa has already run nir_remove_non_entrypoints
    (src/compiler/glsl/gl_nir_linker.c:158, in gl_nir_inline_functions).
    It is not a NIR_PASS and prints nothing, so a dump printed before it and
    still the last one at the point shows functions Mesa no longer has (the
    empty gl_mesa_tmp_<hash> of glsl_to_nir). Dropping them is that call's
    model only while nothing calls them: a `call` in the dump is an error.
    """
    out = []
    keep = True
    for line in dump:
        if line.strip().startswith("call "):
            raise DumpError("a call at the ssa point: %s" % line.strip())
        if line.startswith("decl_function "):
            keep = line.endswith(" (entrypoint)")
        if keep:
            out.append(line)
    return out


def normalise(dump):
    """The normalised text of one dump (lines as cut_dumps gives them)."""
    out = []
    impl = None
    seen_function = False
    temps = {}

    def rename_temp(m):
        # Mesa's names of its own temporaries (vec_ctor, const_temp, #4) say
        # nothing ours can match: t0, t1, ... in order of first use
        name = temps.setdefault(m.group(2), "t%d" % len(temps))
        return m.group(1) + name + m.group(3)
    for line in dump:
        if line.startswith("decl_function "):
            seen_function = True
        if not seen_function:
            continue
        # a function body runs from `impl <name> {` to the `}` in column 0
        if line.startswith("impl "):
            impl = []
        s = line.strip()
        if s and not s.startswith("decl_var "):
            s = CHAIN_COMMENT.sub("", s)
            if "@" in s:
                s = NAME_COMMENT.sub("", s)
            s = TEMP.sub(rename_temp, s)
            (out if impl is None else impl).append(" ".join(s.split()))
        if line == "}" and impl is not None:
            out.extend(renumber(impl))
            impl = None
    if not seen_function or impl is not None:
        raise DumpError("no decl_function in the dump" if not seen_function else "a function with no end")
    return "\n".join(out) + "\n"


def renumber(impl):
    """One function's lines with its SSA names numbered in order of definition."""
    names = {}
    for s in impl:
        m = DEF.match(s)
        if m:
            if m.group(1) in names:
                raise DumpError("%%%s defined twice in %s" % (m.group(1), impl[0]))
            names[m.group(1)] = "%%%d" % len(names)

    def rename(m):
        if m.group(1) not in names:
            raise DumpError("%%%s used but not defined in %s" % (m.group(1), impl[0]))
        return names[m.group(1)]
    return [SSA.sub(rename, s) for s in impl]


def ssa_dump(stdout_lines, key):
    """(dump, [pass, ...]) of one stage's stream (NIR_DEBUG=print_<key>): the
    last dump before the first nir_opt_find_array_copies name, and the passes
    that printed a dump between glsl_to_nir's and it. The model is checked:
    the two names before that name are nir_lower_vars_to_ssa then
    nir_remove_dead_variables (gl_nir_linker.c:55-67), and one glsl_to_nir
    comes before them."""
    names = []
    dumps = []
    for kind, v in events(stdout_lines):
        if kind == "pass":
            if v == "nir_opt_find_array_copies":
                break
            names.append(v)
            continue
        stage, dump = v
        if STAGES.get(stage) != key:
            raise DumpError("a %s dump in the %s stream" % (stage, key))
        if not names:
            raise DumpError("a %s dump before any pass name" % key)
        dumps.append((names[-1], dump))
    else:
        raise DumpError("no nir_opt_find_array_copies in the %s stream" % key)
    if names[-2:] != ["nir_lower_vars_to_ssa", "nir_remove_dead_variables"]:
        raise DumpError("%s: %s before nir_opt_find_array_copies, not nir_lower_vars_to_ssa, "
                        "nir_remove_dead_variables" % (key, ", ".join(names[-2:]) or "nothing"))
    if names[:-2].count("glsl_to_nir") != 1:
        raise DumpError("%s: %d glsl_to_nir names before the ssa point, 1 expected"
                        % (key, names[:-2].count("glsl_to_nir")))
    if not dumps or dumps[0][0] != "glsl_to_nir":
        raise DumpError("%s: the first dump is not glsl_to_nir's" % key)
    return dumps[-1][1], [p for p, _ in dumps[1:]]


def select_dumps(stderr_lines):
    """{"vs.position": dump, ["vs.varying": dump,] "fs": dump}: the prints of
    BIFROST_MESA_DEBUG=shaders. An IDVS vertex shader prints twice, position
    then varying (bifrost_nir.c:1299-1305; the varying variant is printed
    even when it is empty and the reference has no vs.varying region), any
    other once (BI_IDVS_NONE). Another count means more than one compile of
    a stage: an error, not a choice."""
    vs, fs = [], []
    for stage, dump in cut_dumps(stderr_lines):
        if stage == "MESA_SHADER_VERTEX":
            vs.append(dump)
        elif stage == "MESA_SHADER_FRAGMENT":
            fs.append(dump)
        else:
            raise DumpError("a %s at selection" % stage)
    if len(vs) not in (1, 2) or len(fs) != 1:
        raise DumpError("%d vertex and %d fragment prints at selection, 1 or 2 and 1 expected"
                        % (len(vs), len(fs)))
    out = {"vs.position": vs[0], "fs": fs[0]}
    if len(vs) == 2:
        out["vs.varying"] = vs[1]
    return out


ORDER = [("vs", "ssa"), ("vs.position", "select"), ("vs.varying", "select"),
         ("fs", "ssa"), ("fs", "select")]


def points_of(vs_stdout, fs_stdout, stderr_text, fs_stderr=None):
    """[(variant, point, raw dump, normalised text, passes)] in the file's
    order; passes is None for a select point. The ssa points come from each
    stage's own stream; the select points from stderr_text, and when
    fs_stderr (the other draw's) is given, its texts must be the same."""
    dumps = {}
    for key, text in (("vs", vs_stdout), ("fs", fs_stdout)):
        dump, passes = ssa_dump(text.split("\n"), key)
        dumps[(key, "ssa")] = (dump, passes)
    sel = select_dumps(stderr_text.split("\n"))
    if fs_stderr is not None:
        other = select_dumps(fs_stderr.split("\n"))
        if sorted(other) != sorted(sel) or any(normalise(other[k]) != normalise(sel[k]) for k in sel):
            raise DumpError("the two draws print different NIR at selection")
    for k, d in sel.items():
        dumps[(k, "select")] = (d, None)
    return [(v, p, dumps[(v, p)][0],
             normalise(entry_only(dumps[(v, p)][0]) if p == "ssa" else dumps[(v, p)][0]), dumps[(v, p)][1])
            for v, p in ORDER if (v, p) in dumps]


def sha_text(points):
    out = []
    for v, p, _, t, passes in points:
        line = "%s %s : %s" % (v, p, hashlib.sha256(t.encode()).hexdigest())
        if passes is not None:
            line += " : " + (",".join(passes) or "-")
        out.append(line + "\n")
    return "".join(out)


def words_equal(got, committed):
    """The extract's programs are the committed reference's: the same
    regions, the same words in each, the same secondary offset."""
    a, b = got.get("regions", {}), committed.get("regions", {})
    if set(a) != set(b) or got.get("secondary_offset") != committed.get("secondary_offset"):
        return False
    return all(a[r].get("words") == b[r].get("words") for r in b)


def bindings(base):
    """--attrib-bindings from <base>.bind (`NAME LOC` lines, # comments), or []."""
    path = base + ".bind"
    if not os.path.isfile(path):
        return []
    pairs = []
    with open(path) as f:
        for line in f:
            line = line.split("#", 1)[0].split()
            if line:
                if len(line) != 2:
                    raise DumpError("%s: not a NAME LOC line: %s" % (path, " ".join(line)))
                pairs.append("%s=%s" % tuple(line))
    return ["--attrib-bindings", ",".join(pairs)] if pairs else []


def draw(work, base, out, opts, key):
    """Draw with the NIR of stage key ("vs" or "fs") printed; (stdout text,
    stderr text, extract) or None."""
    env = dict(os.environ, NIR_DEBUG="print_" + key)
    r = subprocess.run(["bash", os.path.join(HERE, "drm_shim_ref.sh"), "run", work,
                        base + ".vert", base + ".frag", out] + opts,
                       capture_output=True, text=True, env=env)
    if r.returncode != 0:
        return None
    x = subprocess.run([sys.executable, "-I", os.path.join(HERE, "pan_ref_diff.py"), "extract",
                        out + ".trace", "--draw"], capture_output=True, text=True)
    if x.returncode != 0:
        return None
    with open(out + ".stdout", errors="replace") as f:
        so = f.read()
    with open(out + ".stderr", errors="replace") as f:
        se = f.read()
    return so, se, json.loads(x.stdout)


def one(work, name, texts, scratch):
    """(status, detail) for one pair; writes <name>.nir.sha."""
    base = os.path.join(DATA, name)
    with open(base + ".expect.json") as f:
        committed = json.load(f)
    extra = bindings(base)
    with tempfile.TemporaryDirectory(prefix="nir_refs.", dir=scratch) as tmp:
        for k, opts in enumerate(OPTION_SETS):
            vs = draw(work, base, os.path.join(tmp, "v%d" % k), opts + extra, "vs")
            if vs is not None and words_equal(vs[2], committed):
                break
        else:
            return "NO-DRAW", "no option set gives the reference's words"
        fs = draw(work, base, os.path.join(tmp, "f%d" % k), opts + extra, "fs")
        if fs is None or not words_equal(fs[2], committed):
            return "ERROR", "the print_fs draw does not give the words of the print_vs one"
        points = points_of(vs[0], fs[0], vs[1], fs[1])
    if texts:
        d = os.path.join(texts, name)
        os.makedirs(d, exist_ok=True)
        for v, p, raw, text, _ in points:
            with open(os.path.join(d, "%s.%s.raw" % (v, p)), "w") as f:
                f.write("\n".join(raw) + "\n")
            with open(os.path.join(d, "%s.%s.nir" % (v, p)), "w") as f:
                f.write(text)
    new = sha_text(points)
    path = base + ".nir.sha"
    old = open(path).read() if os.path.isfile(path) else None
    if old == new:
        return "SAME", " ".join(opts)
    with open(path, "w") as f:
        f.write(new)
    return "WROTE", " ".join(opts)


def select(args):
    """The reference names: every committed pair, or those the arguments name."""
    names = sorted(os.path.basename(p)[:-len(".expect.json")]
                   for p in glob.glob(os.path.join(DATA, "*.expect.json")))
    names = [n for n in names if os.path.isfile(os.path.join(DATA, n + ".vert"))
             and os.path.isfile(os.path.join(DATA, n + ".frag"))]
    if not args:
        return names
    return [n for n in names if any(fnmatch.fnmatchcase(n, a) for a in args)]


def shim_work(build_dir):
    work = build_dir or os.environ.get("MESA_SHIM_WORK")
    if not work and os.environ.get("CONNECTED_DEPS_DIR"):
        work = os.path.join(os.environ["CONNECTED_DEPS_DIR"], "mesa-shim")
    if not work:
        sys.exit("nir_refs: set MESA_SHIM_WORK or CONNECTED_DEPS_DIR, or pass --build-dir")
    inst = os.path.join(work, "inst")
    libs = glob.glob(os.path.join(inst, "lib64", "libpanfrost_noop_drm_shim.so")) + \
        glob.glob(os.path.join(inst, "lib", "*", "libpanfrost_noop_drm_shim.so"))
    if not libs:
        sys.exit("nir_refs: %s has no shim build (tools/mali/drm_shim_ref.sh build <tarball> %s)"
                 % (work, work))
    return os.path.abspath(work)


def parse_args(argv):
    """({option: value}, names) or None for a usage error."""
    vals = {"--build-dir": None, "--texts": os.environ.get("NIR_REFS_TEXTS"),
            "--jobs": "8", "--scratch": None}
    args = []
    i = 0
    while i < len(argv):
        if argv[i] in vals:
            if i + 1 >= len(argv):
                return None
            vals[argv[i]] = argv[i + 1]
            i += 2
        elif argv[i].startswith("-"):
            return None
        else:
            args.append(argv[i])
            i += 1
    if not vals["--jobs"].isdigit() or int(vals["--jobs"]) < 1:
        return None
    return vals, args


def main(argv):
    if "-h" in argv or "--help" in argv:
        print(__doc__)
        return 0
    if argv[:1] == ["normalise"] and len(argv) == 1:
        lines = sys.stdin.read().split("\n")
        dumps = cut_dumps(lines)
        if len(dumps) != 1:
            print("nir_refs: %d dumps on stdin, one expected" % len(dumps), file=sys.stderr)
            return 1
        sys.stdout.write(normalise(dumps[0][1]))
        return 0
    parsed = parse_args(argv)
    if parsed is None:
        print(__doc__, file=sys.stderr)
        return 2
    vals, args = parsed
    names = select(args)
    if not names:
        print("nir_refs: no reference selected", file=sys.stderr)
        return 2
    if vals["--scratch"] and not os.path.isdir(vals["--scratch"]):
        print("nir_refs: --scratch %s is not a directory" % vals["--scratch"], file=sys.stderr)
        return 2
    work = shim_work(vals["--build-dir"])
    texts = os.path.abspath(vals["--texts"]) if vals["--texts"] else None

    def job(name):
        try:
            return one(work, name, texts, vals["--scratch"])
        except (OSError, ValueError, DumpError, subprocess.SubprocessError) as e:
            return "ERROR", str(e)
    counts = {"WROTE": 0, "SAME": 0, "NO-DRAW": 0, "ERROR": 0}
    with concurrent.futures.ThreadPoolExecutor(int(vals["--jobs"])) as pool:
        for name, (status, detail) in zip(names, pool.map(job, names)):
            counts[status] += 1
            print("%s %s%s" % (status, name, " " + detail if detail else ""), flush=True)
    print("%d written, %d unchanged, %d no draw, %d errors"
          % (counts["WROTE"], counts["SAME"], counts["NO-DRAW"], counts["ERROR"]))
    return 1 if counts["NO-DRAW"] or counts["ERROR"] else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
