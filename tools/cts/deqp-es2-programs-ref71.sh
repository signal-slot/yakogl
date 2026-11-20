#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# deqp-es2-programs-ref71.sh - Mesa's V3D 7.1 instruction counts for every
# harvested dEQP-GLES2 program, as a table.
#
# Usage: tools/cts/deqp-es2-programs-ref71.sh HARVEST_DIR [--out FILE] [--jobs N]
#   HARVEST_DIR  the output directory of tools/cts/deqp-es2-programs.sh: index.tsv,
#                programs/<id>.vert, programs/<id>.frag and the `commit` file.
#   --out        the table (default tools/shaders/deqp_es2_gen_counts_71.tsv).
#   --jobs       parallel v3d_shc runs (default: the CPU count; the table does
#                not depend on it).
#   V3D_SHC      the v3d_shc binary (tools/v3d/README.md: Mesa 26.2.1's build-v3d).
#                Needed. Without it the script exits 3 and says `gap: V3D_SHC`.
#
# The method is tools/v3d/es100_ref71.sh's: each source is translated and
# assembled as tools/v3d/glsl2v3d.py does (glsl2vivante's translate_es100_gl,
# then glslangValidator -G), and `v3d_shc VS.spv FS.spv --device 71` (rev 7,
# compat_rev 0) runs on the program's own vertex and fragment shader. There is
# no stub: the pair is the program dEQP linked, and the VS is keyed by the FS's
# inputs. A program that does not translate, assemble or compile is refused;
# both of its rows say so (the stage that failed names itself in the reason).
#
# Rows are keyed by the harvest's program id (sha256 of the text, never the text
# itself), one per program stage, sorted by id then stage. Columns: id, stage
# (vs|fs), compiled (yes|no), Mesa's reason ("ok" when compiled), instructions,
# threads, uniforms of the stage ("-" when refused; uniforms is the length of
# v3d_shc's `uniforms`). The header names the VK-GL-CTS commit, the Mesa version,
# the command and the totals. The table has no shader text and is deterministic.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
OUT="$REPO/tools/shaders/deqp_es2_gen_counts_71.tsv"
HARVEST="" JOBS=""
while [ $# -gt 0 ]; do
  case "$1" in
  --out) OUT="${2:?--out needs a file}"; shift 2 ;;
  --jobs) JOBS="${2:?--jobs needs a number}"; shift 2 ;;
  -h | --help) sed -n '5,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  -*) echo "deqp-es2-programs-ref71: unknown option $1" >&2; exit 2 ;;
  *) [ -z "$HARVEST" ] || { echo "deqp-es2-programs-ref71: one harvest directory" >&2; exit 2; }
     HARVEST="$1"; shift ;;
  esac
done

if [ -z "${V3D_SHC-}" ] || [ ! -x "${V3D_SHC-}" ]; then
  echo "gap: V3D_SHC (set it to Mesa 26.2.1's v3d_shc, tools/v3d/README.md)" >&2
  exit 3
fi
if [ -z "$HARVEST" ] || [ ! -f "$HARVEST/index.tsv" ] || [ ! -d "$HARVEST/programs" ]; then
  echo "usage: $0 HARVEST_DIR [--out FILE] [--jobs N]  (tools/cts/deqp-es2-programs.sh output)" >&2
  exit 2
fi
HARVEST="$(cd "$HARVEST" && pwd)"
CMD="tools/cts/deqp-es2-programs-ref71.sh HARVEST_DIR"  # no paths: the table must not depend on where it ran

exec python3 -I - "$REPO" "$HARVEST" "$OUT" "${JOBS:-0}" "$CMD" <<'PY'
import atexit, concurrent.futures, json, os, re, shutil, subprocess, sys, tempfile

repo, harvest, out_path, jobs, cmd = sys.argv[1:6]
jobs = int(jobs) or os.cpu_count() or 1
sys.path.insert(0, os.path.join(repo, "tools", "v3d"))
sys.path.insert(0, os.path.join(repo, "tools", "vivante"))
import glsl2v3d as g  # noqa: E402

shc = os.environ["V3D_SHC"]
work = tempfile.mkdtemp(prefix="deqp_es2_ref71-", dir=os.environ.get("TMPDIR"))
atexit.register(shutil.rmtree, work, True)


def die(msg):
    sys.exit("deqp-es2-programs-ref71: " + msg)


# ---- the harvest ----------------------------------------------------------
ids = []
with open(os.path.join(harvest, "index.tsv")) as f:
    head = f.readline().rstrip("\n").split("\t")
    if head[0] != "id":
        die("index.tsv has no header row")
    for line in f:
        if line.strip():
            ids.append(line.split("\t", 1)[0])
if len(set(ids)) != len(ids):
    die("index.tsv repeats a program id")
ids.sort()

cts = "unknown"
try:
    for line in open(os.path.join(harvest, "commit")):
        if line.startswith("cts="):
            cts = line[4:].split()[0]
except OSError:
    pass

mesa_ver = "26.2.1"  # tools/v3d/README.md; the build tree's VERSION file wins when found
for up in range(1, 6):
    p = shc
    for _ in range(up):
        p = os.path.dirname(p)
    v = os.path.join(p, "VERSION")
    if os.path.isfile(v):
        mesa_ver = open(v).read().strip()
        break
mesa = "Mesa " + mesa_ver

# ---- one program ----------------------------------------------------------
def assemble(name, text, stage):
    """Translate (as glsl2v3d does) and run glslang -G. (spv, None) or (None, why)."""
    ext = ".vert" if stage == "vs" else ".frag"
    src = os.path.join(work, name + ext)
    spv = os.path.join(work, name + ".spv")
    try:
        tr = g.gv.translate_es100_gl(text, stage)
    except SystemExit as e:
        return None, "translate: %s" % e
    except Exception as e:  # noqa: BLE001 - a translator bug is a refusal row
        return None, "translate: %s: %s" % (type(e).__name__, e)
    with open(src, "w") as f:
        f.write(tr)
    r = subprocess.run([g.GLSLANG, "-G", src, "-o", spv], capture_output=True, text=True)
    if r.returncode:
        tag = "<%s>" % stage
        msg = (r.stdout + r.stderr).replace(src, tag)
        lines = [l.strip() for l in msg.splitlines() if l.strip() and l.strip() != tag]
        return None, "glslang: " + " / ".join(lines)
    return spv, None


def clean(s):
    return re.sub(r"\s+", " ", s).strip().replace(work, "<work>")


def one(pid):
    """The two rows of program `pid`."""
    srcs = {}
    for stage, ext in (("vs", ".vert"), ("fs", ".frag")):
        with open(os.path.join(harvest, "programs", pid + ext), errors="replace") as f:
            srcs[stage] = f.read()
    spv, why = {}, {}
    for stage in ("vs", "fs"):
        spv[stage], why[stage] = assemble(pid + "_" + stage, srcs[stage], stage)
    bad = [s for s in ("vs", "fs") if spv[s] is None]
    reason = None
    if bad:
        reason = "; ".join("%s: %s" % (s, why[s]) for s in bad)
    else:
        r = subprocess.run([shc, spv["vs"], spv["fs"], "--device", "71"],
                           capture_output=True, text=True)
        if r.returncode:
            err = [l for l in r.stderr.splitlines() if l.strip() and not g.DISASM.match(l)]
            err = " / ".join(l.strip().replace(spv["vs"], "<vs.spv>").replace(spv["fs"], "<fs.spv>")
                             for l in err)
            if r.returncode < 0:
                err = "v3d_shc killed by signal %d%s" % (-r.returncode, ": " + err if err else "")
            reason = "v3d_shc: " + (err or "exit %d" % r.returncode)
        else:
            js = json.loads(r.stdout)
            if js["gpu"]["ver"] != 71:
                die("%s was compiled for ver %s" % (pid, js["gpu"]["ver"]))
    rows = []
    for stage in ("fs", "vs"):
        if reason is not None:
            rows.append((pid, stage, "no", clean(reason), "-", "-", "-"))
        else:
            s = js[stage]
            rows.append((pid, stage, "yes", "ok", str(len(s["code"])), str(s["threads"]),
                         str(len(s["uniforms"]))))
    return rows


with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as ex:
    per = list(ex.map(one, ids))
rows = sorted((r for p in per for r in p), key=lambda r: (r[0], r[1]))
n_yes = sum(r[2] == "yes" for r in rows)
ps = len(rows) // 2 if rows else 0
p_yes = sum(1 for p in per if p[0][2] == "yes")
reasons = {}
for p in per:
    if p[0][2] == "no":
        k = re.sub(r"\b[0-9a-f]{16}\b", "<id>", p[0][3])
        reasons[k] = reasons.get(k, 0) + 1

with open(out_path, "w") as f:
    f.write("# SPDX-License-Identifier: Apache-2.0\n")
    f.write("# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n")
    f.write("# %s v3d_shc --device 71 (rev 7, compat_rev 0) over every program dEQP-GLES2\n" % mesa)
    f.write("# compiles, the program's own VS and FS as one pair (no stub); rows keyed by the\n")
    f.write("# harvest's program id, no shader text (tools/cts/deqp-es2-programs-ref71.sh).\n")
    f.write("# VK-GL-CTS commit: %s\n" % cts)
    f.write("# command: %s\n" % cmd)
    f.write("# programs: %d, compiled: %d, refused: %d\n" % (len(per), p_yes, len(per) - p_yes))
    f.write("# program stages: %d, compiled: %d, refused: %d\n" % (len(rows), n_yes, len(rows) - n_yes))
    f.write("# columns: id, stage, compiled, reason, instructions, threads, uniforms\n")
    for r in rows:
        f.write("\t".join(r) + "\n")
print("deqp-es2-programs-ref71: %d programs, %d compiled at 7.1, %d refused -> %s"
      % (len(per), p_yes, len(per) - p_yes, out_path))
for k, n in sorted(reasons.items(), key=lambda kv: (-kv[1], kv[0]))[:15]:
    print("  %5d  %s" % (n, k[:160]))
PY
