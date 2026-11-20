#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Mesa 7.1 (v3d_shc --device 71) counts for the dEQP-GLES2 .test library.

Run through tools/cts/es2lib-ref71.sh (its header has the usage and columns).
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

CTS_COMMIT = "93bca01861b0e3ef3c387027a9791e6d065f900c"
SHC_HEADER = re.compile(r"^(--- \w+: \d+ instructions$|\s*\d+ 0x[0-9a-f]{16} )")


def find_shaders(cands):
    for c in cands:
        for sub in ("", "data/gles2/shaders"):
            if os.path.isfile(os.path.join(c, sub, "linkage.test")):
                return os.path.join(c, sub)
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True)
    ap.add_argument("--cts", default=None)
    ap.add_argument("--out", default=None)
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 4)
    a = ap.parse_args()
    repo = a.repo
    out_path = a.out or os.path.join(repo, "tools", "shaders", "deqp_es2_lib_counts_71.tsv")

    sys.path.insert(0, os.path.join(repo, "tools", "v3d"))
    sys.path.insert(0, os.path.join(repo, "tools", "vivante"))
    sys.path.insert(0, os.path.join(repo, "tools"))
    import glsl2v3d as g
    import es100_oracle as o

    env = os.environ
    cands = [a.cts] if a.cts else []
    cands += [env[k] for k in ("DEQP_GLES2_TEST_DIR",) if env.get(k)]
    if env.get("CONNECTED_DEPS_DIR"):
        cands.append(os.path.join(env["CONNECTED_DEPS_DIR"], "VK-GL-CTS-gles2-data"))
    if env.get("VK_GL_CTS"):
        cands.append(env["VK_GL_CTS"])
    shaders = find_shaders(cands)
    if not shaders:
        sys.exit("gap: the dEQP-GLES2 .test files (--cts DIR with data/gles2/shaders, VK-GL-CTS %s)"
                 % CTS_COMMIT[:7])

    # (case name, vertex text, fragment text, stages to record)
    progs = []
    for p in sorted(Path(shaders).glob("*.test")):
        for c in o.Parser(o.tokenize(p.read_text()), p.name).parse():
            if c.version != 100 or c.expect not in ("pass", "build_successful"):
                continue
            for name, vs, fs in o.programs_of(c):
                if c.both is not None:
                    stages = ("vs",) if name.endswith("_vertex") else ("fs",)
                else:
                    stages = ("vs", "fs")
                progs.append((p.stem + "." + name, vs, fs, stages))
    progs.sort(key=lambda t: t[0])
    names = [t[0] for t in progs]
    if len(set(names)) != len(names):
        sys.exit("es2lib-ref71: duplicate program names")

    shc = env["V3D_SHC"]
    work = tempfile.mkdtemp(prefix="es2lib_ref71-", dir=env.get("TMPDIR"))

    def assemble(tag, text, stage):
        src = os.path.join(work, tag + (".vert" if stage == "vs" else ".frag"))
        spv = os.path.join(work, tag + ".spv")
        try:
            tr = g.gv.translate_es100_gl(text, stage)
        except SystemExit as e:
            return None, "%s translate: %s" % (stage, e)
        with open(src, "w") as f:
            f.write(tr)
        r = subprocess.run([g.GLSLANG, "-G", src, "-o", spv], capture_output=True, text=True)
        if r.returncode:
            tagname = "<%s>" % stage
            msg = (r.stdout + r.stderr).replace(src, tagname)
            lines = [l.strip() for l in msg.splitlines() if l.strip() and l.strip() != tagname]
            return None, "%s glslang: %s" % (stage, " / ".join(lines))
        return spv, None

    def one(i):
        name, vs, fs, stages = progs[i]

        def refused(why):
            return [(name, s, "no", why, "-", "-", "-") for s in stages]

        vspv, why = assemble("v%d" % i, vs, "vs")
        if vspv is None:
            return refused(why)
        fspv, why = assemble("f%d" % i, fs, "fs")
        if fspv is None:
            return refused(why)
        r = subprocess.run([shc, vspv, fspv, "--device", "71"], capture_output=True, text=True)
        if r.returncode:
            err = [l for l in r.stderr.splitlines() if l.strip() and not SHC_HEADER.match(l)]
            err = " / ".join(l.strip().replace(vspv, "<vs.spv>").replace(fspv, "<fs.spv>")
                             for l in err)
            if r.returncode < 0:
                err = "v3d_shc killed by signal %d%s" % (-r.returncode, ": " + err if err else "")
            return refused(err or "v3d_shc exit %d" % r.returncode)
        js = json.loads(r.stdout)
        if js["gpu"]["ver"] != 71:
            sys.exit("es2lib-ref71: %s was compiled for ver %s" % (name, js["gpu"]["ver"]))
        return [(name, s, "yes", "ok", str(len(js[s]["code"])), str(js[s]["threads"]),
                 str(len(js[s]["uniforms"]))) for s in stages]

    try:
        with ThreadPoolExecutor(max_workers=max(1, a.jobs)) as ex:
            rows = [r for rs in ex.map(one, range(len(progs))) for r in rs]
    finally:
        shutil.rmtree(work, True)
    rows.sort(key=lambda r: (r[0], r[1]))
    n_yes = sum(r[2] == "yes" for r in rows)
    with open(out_path, "w") as f:
        f.write("# SPDX-License-Identifier: Apache-2.0\n")
        f.write("# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n")
        f.write("# Mesa 26.2.1 v3d_shc --device 71 (rev 7, compat_rev 0) over the expect-pass\n")
        f.write("# programs (version 100) of VK-GL-CTS %s data/gles2/shaders/*.test,\n"
                % CTS_COMMIT[:7])
        f.write("# as tools/es100_oracle.py's programs_of() builds them; made by\n")
        f.write("# V3D_SHC=<v3d_shc> tools/cts/es2lib-ref71.sh. Names and counts only.\n")
        f.write("# programs: %d, rows: %d, compiled: %d, refused: %d\n"
                % (len(progs), len(rows), n_yes, len(rows) - n_yes))
        f.write("# columns: case, stage, compiled, reason, instructions, threads, uniforms\n")
        for r in rows:
            f.write("\t".join(r).replace("\n", " / ") + "\n")
    print("es2lib-ref71: %d programs, %d rows, %d compiled at 7.1, %d refused -> %s"
          % (len(progs), len(rows), n_yes, len(rows) - n_yes, out_path))


if __name__ == "__main__":
    main()
