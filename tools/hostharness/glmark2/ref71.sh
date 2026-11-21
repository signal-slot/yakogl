#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# ref71.sh - Mesa's V3D 7.1 output for every program of the glmark2-es2 corpus.
#
# Usage: tools/hostharness/glmark2/ref71.sh <corpus dir> <out.tsv>
#   <corpus dir>  the generated corpus (index.txt and the .glsl files):
#                 `tools/hostharness/glmark2/run.sh "$GLMARK2_SRC"` writes it to
#                 build/glmark2-host/shaders. GPL text, never committed.
#   <out.tsv>     the table (the committed one is tools/shaders/glmark2_counts_71.tsv)
#
# environment:
#   V3D_SHC       Mesa 26.2.1's v3d_shc (tools/v3d/README.md H2); without it the
#                 script exits 3 and says `gap: V3D_SHC`. The script names the
#                 Mesa tree in the table's header from its path (the tree above
#                 build/src/broadcom/v3d_shc).
#   GLMARK2_SRC   the glmark2 checkout or archive directory the corpus came from;
#                 without it the script exits 3 and says `gap: GLMARK2_SRC`. Its
#                 commit goes into the header (a git checkout's HEAD, or the
#                 directory name `glmark2-<commit>` of GitHub's archive).
#
# A program is one vertex and one fragment shader of one index entry, the pairs
# tests/glmark2_corpus.rs tries (the index does not say which vertex shader went
# with which fragment shader of an entry, so every pair is a program; a pair that
# two entries share is one row). Its key is `<vs file>+<fs file>`, as that test
# names a pair. Each file is translated as tools/v3d/es100_ref71.sh does
# (glsl2vivante's translate_es100_gl, then glslangValidator -G), with the one
# change that a varying's location is taken from the vertex shader's order for
# both stages (the translation numbers varyings by declaration order, per
# stage), and run at `v3d_shc --device 71` (rev 7, compat_rev 0).
#
# glmark2's precision macros HIGHP_OR_DEFAULT and MEDIUMP_OR_DEFAULT are
# substituted in the compiled copy as glmark2 defines them on a GLES2 context
# (highp and mediump); the corpus files are not changed.
#
# Columns: key, vs instructions, fs instructions, vs threads, fs threads, reason
# ("ok" when Mesa compiled the program, else the failing step and its message;
# the numbers are "-" then). Names and numbers only, never shader text.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"

if [ $# -ne 2 ]; then
  sed -n '5,10p' "$0" >&2
  exit 2
fi
CORPUS="$1"
OUT="$2"

if [ -z "${V3D_SHC-}" ] || [ ! -x "${V3D_SHC-}" ]; then
  echo "gap: V3D_SHC (set it to Mesa 26.2.1's v3d_shc, tools/v3d/README.md H2)" >&2
  exit 3
fi
if [ -z "${GLMARK2_SRC-}" ] || [ ! -d "${GLMARK2_SRC-}" ]; then
  echo "gap: GLMARK2_SRC (the glmark2 directory the corpus was generated from, tools/hostharness/glmark2/README.md)" >&2
  exit 3
fi
if [ ! -f "$CORPUS/index.txt" ]; then
  echo "ref71.sh: $CORPUS has no index.txt: run tools/hostharness/glmark2/run.sh \"\$GLMARK2_SRC\" first" >&2
  exit 3
fi

exec python3 -I - "$REPO" "$CORPUS" "$OUT" <<'PY'
import atexit, json, os, re, shutil, subprocess, sys, tempfile

repo, corpus, out_path = sys.argv[1:4]
sys.path.insert(0, os.path.join(repo, "tools", "v3d"))
sys.path.insert(0, os.path.join(repo, "tools", "vivante"))
import glsl2v3d as g  # noqa: E402

shc = os.environ["V3D_SHC"]
src_dir = os.path.realpath(os.environ["GLMARK2_SRC"])


def die(msg):
    sys.exit("ref71.sh: " + msg)


def glmark2_version():
    r = subprocess.run(["git", "-C", src_dir, "rev-parse", "HEAD"], capture_output=True, text=True)
    if r.returncode == 0 and re.fullmatch(r"[0-9a-f]{40}", r.stdout.strip()):
        return r.stdout.strip()
    m = re.fullmatch(r"glmark2-([0-9a-f]{40})", os.path.basename(src_dir))
    if m:
        return m.group(1)
    die("cannot tell the commit of GLMARK2_SRC (a git checkout, or the archive directory glmark2-<commit>)")


def mesa_tree():
    p = os.path.realpath(shc)
    tree = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(p))))
    ver = os.path.join(tree, "VERSION")
    v = open(ver).read().strip() if os.path.isfile(ver) else "?"
    return "mesa-%s (v3d_shc, %s)" % (v, os.path.relpath(p, tree))


# the index, as tests/glmark2_corpus.rs reads it: `<file>: <entry>; <entry>`
files, entries = [], {}
for l in open(os.path.join(corpus, "index.txt")):
    if l.startswith("#") or not l.strip():
        continue
    f, es = l.rstrip("\n").split(": ", 1)
    files.append(f)
    for e in es.split("; "):
        slot = entries.setdefault(e, ([], []))
        slot[0 if ".vert." in f else 1].append(f)
pairs = sorted({(v, f) for vs, fs in entries.values() for v in vs for f in fs})
if not pairs:
    die("the index lists no vertex/fragment pair")

work = tempfile.mkdtemp(prefix="glmark2_ref71-", dir=os.environ.get("TMPDIR"))
atexit.register(shutil.rmtree, work, True)

PRECISION_MACROS = (("HIGHP_OR_DEFAULT", "highp"), ("MEDIUMP_OR_DEFAULT", "mediump"))
LOC = re.compile(r"layout\(location = (\d+)\) (in|out) (\w+) (\w+)((?:\[[^\]]*\])?);")


def translate(name, stage):
    text = open(os.path.join(corpus, name)).read()
    # glmark2 defines these two at load time (libmatrix/shader-source.cc:490-505);
    # on a GLES2 context GL_ES is set, and so is GL_FRAGMENT_PRECISION_HIGH on
    # the v3d, so both expand to the qualifier in either stage.
    for macro, qual in PRECISION_MACROS:
        text = re.sub(r"\b%s\b" % macro, qual, text)
    return g.gv.translate_es100_gl(text, stage)


def varying_locs(tr, direction):
    return {m.group(4): int(m.group(1)) for m in LOC.finditer(tr) if m.group(2) == direction}


def relocate(tr, direction, locs):
    def sub(m):
        if m.group(2) != direction or m.group(4) not in locs:
            return m.group(0)
        return "layout(location = %d) %s %s %s%s;" % (locs[m.group(4)], m.group(2), m.group(3),
                                                       m.group(4), m.group(5))
    return LOC.sub(sub, tr)


def assemble(tag, tr, stage):
    ext = ".vert" if stage == "vs" else ".frag"
    src = os.path.join(work, tag + ext)
    spv = os.path.join(work, tag + ".spv")
    open(src, "w").write(tr)
    r = subprocess.run([g.GLSLANG, "-G", src, "-o", spv], capture_output=True, text=True)
    if r.returncode:
        t = "<%s>" % stage
        msg = (r.stdout + r.stderr).replace(src, t)
        return None, "glslang %s: %s" % (stage, " / ".join(
            l.strip() for l in msg.splitlines() if l.strip() and l.strip() != t))
    return spv, None


def one(i, v, f):
    key = "%s+%s" % (v, f)
    bad = lambda why: (key, "-", "-", "-", "-", why)
    try:
        vtr, ftr = translate(v, "vs"), translate(f, "fs")
    except SystemExit as e:
        return bad("translate: %s" % e)
    # varyings keep the vertex shader's numbers; the fragment shader's others follow
    locs = varying_locs(vtr, "out")
    nxt = max(locs.values(), default=-1) + 1
    for m in LOC.finditer(ftr):
        if m.group(2) == "in" and m.group(4) not in locs:
            locs[m.group(4)] = nxt
            nxt += 1
    ftr = relocate(ftr, "in", locs)
    vspv, why = assemble("v%d" % i, vtr, "vs")
    if vspv is None:
        return bad(why)
    fspv, why = assemble("f%d" % i, ftr, "fs")
    if fspv is None:
        return bad(why)
    r = subprocess.run([shc, vspv, fspv, "--device", "71"], capture_output=True, text=True)
    if r.returncode:
        err = [l for l in r.stderr.splitlines() if l.strip() and not g.DISASM.match(l)]
        err = " / ".join(l.strip().replace(vspv, "<vs.spv>").replace(fspv, "<fs.spv>") for l in err)
        if r.returncode < 0:
            err = "v3d_shc killed by signal %d%s" % (-r.returncode, ": " + err if err else "")
        return bad("v3d_shc: " + (err or "exit %d" % r.returncode))
    js = json.loads(r.stdout)
    if js["gpu"]["ver"] != 71:
        die("%s was compiled for ver %s" % (key, js["gpu"]["ver"]))
    vs, fs = js["vs"], js["fs"]
    return (key, str(len(vs["code"])), str(len(fs["code"])), str(vs["threads"]), str(fs["threads"]), "ok")


rows = [one(i, v, f) for i, (v, f) in enumerate(pairs)]
n_ok = sum(r[5] == "ok" for r in rows)
commit, tree = glmark2_version(), mesa_tree()
with open(out_path, "w") as o:
    o.write("# SPDX-License-Identifier: Apache-2.0\n")
    o.write("# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n")
    o.write("# Mesa's V3D 7.1 output (v3d_shc --device 71, rev 7, compat_rev 0) for every\n")
    o.write("# vertex/fragment pair of the glmark2-es2 corpus: names and numbers only.\n")
    o.write("# command: V3D_SHC=<v3d_shc> GLMARK2_SRC=<glmark2> tools/hostharness/glmark2/ref71.sh \\\n")
    o.write("#   build/glmark2-host/shaders tools/shaders/glmark2_counts_71.tsv\n")
    o.write("# GLMARK2_SRC: glmark2 commit %s\n" % commit)
    o.write("# Mesa tree: %s\n" % tree)
    o.write("# programs: %d, compiled: %d, refused: %d (%d shaders in the index)\n"
            % (len(rows), n_ok, len(rows) - n_ok, len(files)))
    o.write("# columns: key (vs file+fs file), vs instructions, fs instructions, vs threads, fs threads, reason\n")
    for r in rows:
        o.write("\t".join(r).replace("\n", " / ") + "\n")
print("ref71.sh: %d programs, %d compiled at 7.1, %d refused -> %s"
      % (len(rows), n_ok, len(rows) - n_ok, out_path))
PY
