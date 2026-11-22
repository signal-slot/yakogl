#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mali-testdata-ref71.sh - Mesa's V3D 7.1 output for every vertex/fragment pair
# of tools/mali/testdata (the dEQP-GLES2 shapes the Mali back end is measured
# on), as a table of names and counts.
#
# Usage: tools/cts/mali-testdata-ref71.sh --shc <v3d_shc> [--out <tsv>]
#   --shc  Mesa 26.2.1's v3d_shc (tools/v3d/README.md H2: build-v3d, or
#          `tools/v3d/drm_shim_ref.sh build <mesa-26.2.1.tar.xz> <work dir>`,
#          which gives <work dir>/mesa-26.2.1/build/src/broadcom/v3d_shc).
#          V3D_SHC in the environment does the same. Without one the script
#          exits 3 and says `gap: V3D_SHC`.
#   --out  where to write (default tools/shaders/mali_testdata_counts_71.tsv).
#
# Regenerate the committed table:
#   tools/cts/mali-testdata-ref71.sh --shc <v3d_shc>
# A second run with the same v3d_shc writes the same bytes (rows are sorted,
# the parallel runs are collected in order).
#
# A program is one X.vert and the X.frag of the same name (a file whose other
# stage is missing is not a program; the header counts them). Each file is
# translated as tools/hostharness/glmark2/ref71.sh does (glsl2vivante's
# translate_es100_gl, glslangValidator -G, a varying's location taken from the
# vertex shader's order for both stages) and run at `v3d_shc --device 71`
# (rev 7, compat_rev 0). Columns: name (X), vs instructions, fs instructions,
# vs threads, fs threads, reason ("ok" when Mesa compiled the program, else the
# failing step and its message; the numbers are "-" then). Names and numbers
# only, never shader text. Needs glslangValidator and python3.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
OUT="$REPO/tools/shaders/mali_testdata_counts_71.tsv"
usage() { sed -n '8,9p' "$0" >&2; exit 2; }
while [ $# -gt 0 ]; do
  case "$1" in
  --shc) [ $# -ge 2 ] || usage; V3D_SHC="$2"; shift 2 ;;
  --out) [ $# -ge 2 ] || usage; OUT="$2"; shift 2 ;;
  *) usage ;;
  esac
done
if [ -z "${V3D_SHC-}" ] || [ ! -x "${V3D_SHC-}" ]; then
  echo "gap: V3D_SHC (set it to Mesa 26.2.1's v3d_shc, tools/v3d/README.md H2)" >&2
  exit 3
fi
export V3D_SHC

exec python3 -I - "$REPO" "$OUT" <<'PY'
import atexit, json, os, re, shutil, subprocess, sys, tempfile
from concurrent.futures import ThreadPoolExecutor

repo, out_path = sys.argv[1:3]
sys.path.insert(0, os.path.join(repo, "tools", "v3d"))
sys.path.insert(0, os.path.join(repo, "tools", "vivante"))
import glsl2v3d as g  # noqa: E402

shc = os.environ["V3D_SHC"]
data = os.path.join(repo, "tools", "mali", "testdata")


def die(msg):
    sys.exit("mali-testdata-ref71.sh: " + msg)


def mesa_tree():
    p = os.path.realpath(shc)
    tree = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(p))))
    ver = os.path.join(tree, "VERSION")
    v = open(ver).read().strip() if os.path.isfile(ver) else "?"
    return "mesa-%s (v3d_shc, %s)" % (v, os.path.relpath(p, tree))


vs_names = {f[:-5] for f in os.listdir(data) if f.endswith(".vert")}
fs_names = {f[:-5] for f in os.listdir(data) if f.endswith(".frag")}
names = sorted(vs_names & fs_names)
unpaired = sorted(vs_names ^ fs_names)
if not names:
    die("no X.vert + X.frag pair in %s" % data)

work = tempfile.mkdtemp(prefix="mali_ref71-", dir=os.environ.get("TMPDIR"))
atexit.register(shutil.rmtree, work, True)

LOC = re.compile(r"layout\(location = (\d+)\) (in|out) (\w+) (\w+)((?:\[[^\]]*\])?);")


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


def one(i, name):
    def bad(why):
        return (name, "-", "-", "-", "-", why)
    try:
        vtr = g.gv.translate_es100_gl(open(os.path.join(data, name + ".vert")).read(), "vs")
        ftr = g.gv.translate_es100_gl(open(os.path.join(data, name + ".frag")).read(), "fs")
    except SystemExit as e:
        return bad("translate: %s" % e)
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
        die("%s was compiled for ver %s" % (name, js["gpu"]["ver"]))
    vs, fs = js["vs"], js["fs"]
    return (name, str(len(vs["code"])), str(len(fs["code"])), str(vs["threads"]), str(fs["threads"]), "ok")


with ThreadPoolExecutor(max_workers=os.cpu_count() or 1) as ex:
    rows = list(ex.map(lambda a: one(*a), enumerate(names)))
n_ok = sum(r[5] == "ok" for r in rows)
with open(out_path, "w") as o:
    o.write("# SPDX-License-Identifier: Apache-2.0\n")
    o.write("# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n")
    o.write("# Mesa's V3D 7.1 output (v3d_shc --device 71, rev 7, compat_rev 0) for every\n")
    o.write("# vertex/fragment pair of tools/mali/testdata: names and numbers only.\n")
    o.write("# command: tools/cts/mali-testdata-ref71.sh --shc <v3d_shc>\n")
    o.write("# Mesa tree: %s\n" % mesa_tree())
    o.write("# programs: %d, compiled: %d, refused: %d (%d files without their other stage, not rows)\n"
            % (len(rows), n_ok, len(rows) - n_ok, len(unpaired)))
    o.write("# columns: name, vs instructions, fs instructions, vs threads, fs threads, reason\n")
    for r in rows:
        o.write("\t".join(r).replace("\n", " / ") + "\n")
print("mali-testdata-ref71.sh: %d programs, %d compiled at 7.1, %d refused -> %s"
      % (len(rows), n_ok, len(rows) - n_ok, out_path))
PY
