#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Mesa 7.1 (v3d_shc --device 71) counts for the in-tree dEQP repro shaders.

Run through tools/cts/deqp-repro-ref71.sh (its header has the usage and columns).
"""
import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

CONST = re.compile(r"^(?:pub\s+)?const\s+([A-Z0-9_]+)\s*:\s*&(?:'static\s+)?str\s*=\s*", re.M)
VARYING = re.compile(r"\bvarying\s+(?:(?:highp|mediump|lowp)\s+)?(float|vec[234]|mat[234])\s+([^;]+);")
SHC_HEADER = re.compile(r"^(--- \w+: \d+ instructions$|\s*\d+ 0x[0-9a-f]{16} )")


def rust_string(text, pos):
    """The string literal starting at text[pos] and the index after it, or None."""
    m = re.compile(r'r(#*)"').match(text, pos)
    if m:
        end = text.find('"' + m.group(1), m.end())
        return (text[m.end():end], end + 1 + len(m.group(1))) if end >= 0 else None
    if text[pos:pos + 1] != '"':
        return None
    out, i = [], pos + 1
    esc = {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", '"': '"', "'": "'"}
    while i < len(text):
        c = text[i]
        if c == '"':
            return "".join(out), i + 1
        if c == "\\":
            n = text[i + 1]
            if n == "\n":  # a line continuation drops the newline and the next indent
                i += 2
                while i < len(text) and text[i] in " \t\r\n":
                    i += 1
                continue
            out.append(esc[n])
            i += 2
            continue
        out.append(c)
        i += 1
    return None


def extract(path):
    """[(constant name, source text)] of the shader constants of a Rust file."""
    text = open(path).read()
    found = []
    for m in CONST.finditer(text):
        r = rust_string(text, m.end())
        if r and "void main" in r[0]:
            found.append((m.group(1), r[0]))
    return found


def stage_of(name, src):
    if name.endswith("_VS"):
        return "vs"
    if name.endswith("_FS"):
        return "fs"
    return "fs" if "gl_FragColor" in src or "gl_FragData" in src else "vs"


def varyings(g, text):
    out = []
    for m in VARYING.finditer(g.u.strip_comments(text)):
        for d in m.group(2).split(","):
            n = re.match(r"\s*(\w+)\s*(?:\[\s*(\d+)\s*\])?\s*$", d)
            if not n:
                sys.exit("deqp-repro-ref71: cannot read varying `%s`" % d.strip())
            out.append((m.group(1), n.group(1), int(n.group(2) or 0)))
    return out


def value(ty, src):
    if ty == "float":
        return src + ".x"
    if ty.startswith("vec"):
        return "%s(%s)" % (ty, src) if ty == "vec4" else "%s.%s" % (src, "xyzw"[:int(ty[3])])
    n = int(ty[3])
    return "%s(%s)" % (ty, ", ".join([src + ".x"] * (n * n)))


def stub_vs(vl):
    s = ["#version 100", "precision highp float;", "attribute vec4 a_pos;", "uniform vec4 u_stub;"]
    body = ["    gl_Position = a_pos;"]
    for ty, name, arr in vl:
        s.append("varying %s %s%s;" % (ty, name, "[%d]" % arr if arr else ""))
        for i in range(arr or 1):
            body.append("    %s%s = %s;" % (name, "[%d]" % i if arr else "", value(ty, "u_stub")))
    return "\n".join(s + ["void main()", "{"] + body + ["}", ""])


def stub_fs(vl):
    s = ["#version 100", "precision highp float;"]
    body = ["    vec4 c = vec4(0.0);"]
    for ty, name, arr in vl:
        s.append("varying %s %s%s;" % (ty, name, "[%d]" % arr if arr else ""))
        for i in range(arr or 1):
            e = name + ("[%d]" % i if arr else "")
            if ty.startswith("mat"):
                n = int(ty[3])
                body.append("    c.x += %s;" % " + ".join("%s[%d].x" % (e, k) for k in range(n)))
            else:
                body.append("    c += vec4(%s);" % e if ty == "float" else
                            "    c += vec4(%s%s);" % (e, ", 0.0" * (4 - int(ty[3])) if ty != "vec4" else ""))
    return "\n".join(s + ["void main()", "{"] + body + ["    gl_FragColor = c;", "}", ""])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True)
    ap.add_argument("--out", default=None)
    a = ap.parse_args()
    repo = a.repo
    out_path = a.out or os.path.join(repo, "tools", "shaders", "deqp_repro_counts_71.tsv")

    sys.path.insert(0, os.path.join(repo, "tools", "v3d"))
    sys.path.insert(0, os.path.join(repo, "tools", "vivante"))
    import glsl2v3d as g

    shc = os.environ["V3D_SHC"]
    files = [os.path.join(repo, "tests", "deqp_repro_shaders.rs")]
    files += sorted(glob.glob(os.path.join(repo, "tests", "mali_deqp_*.rs")))

    # case -> {stage: source}
    progs = {}
    for path in files:
        stem = os.path.basename(path)[:-3]
        for name, src in extract(path):
            base = re.sub(r"_(VS|FS)$", "", name)
            case = stem + "." + base.lower()
            st = stage_of(name, src)
            if st in progs.setdefault(case, {}):
                sys.exit("deqp-repro-ref71: %s has two %s sources" % (case, st))
            progs[case][st] = src

    work = tempfile.mkdtemp(prefix="deqp_repro_ref71-", dir=os.environ.get("TMPDIR"))

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
            tag_ = "<%s>" % stage
            msg = (r.stdout + r.stderr).replace(src, tag_)
            lines = [l.strip() for l in msg.splitlines() if l.strip() and l.strip() != tag_]
            return None, "%s glslang: %s" % (stage, " / ".join(lines))
        return spv, None

    rows = []
    try:
        for i, case in enumerate(sorted(progs)):
            have = progs[case]
            stages = sorted(have)

            def refused(why):
                return [(case, s, "no", why, "-", "-", "-") for s in stages]

            spv, why = {}, None
            for s in stages:
                spv[s], why = assemble("%s%d" % (s, i), have[s], s)
                if spv[s] is None:
                    break
            if why:
                rows += refused(why)
                continue
            for s, other in (("vs", "fs"), ("fs", "vs")):
                if s in have and other not in have:  # a lone stage: the other is a stub
                    p, pwhy = assemble("p%d" % i, stub_fs(varyings(g, have[s])) if s == "vs"
                                       else stub_vs(varyings(g, have[s])), other)
                    if p is None:
                        sys.exit("deqp-repro-ref71: the stub partner of %s does not assemble: %s"
                                 % (case, pwhy))
                    spv[other] = p
            r = subprocess.run([shc, spv["vs"], spv["fs"], "--device", "71"],
                               capture_output=True, text=True)
            if r.returncode:
                err = [l for l in r.stderr.splitlines() if l.strip() and not SHC_HEADER.match(l)]
                err = " / ".join(l.strip().replace(spv["vs"], "<vs.spv>").replace(spv["fs"], "<fs.spv>")
                                 for l in err)
                if r.returncode < 0:
                    err = "v3d_shc killed by signal %d%s" % (-r.returncode, ": " + err if err else "")
                rows += refused(err or "v3d_shc exit %d" % r.returncode)
                continue
            js = json.loads(r.stdout)
            if js["gpu"]["ver"] != 71:
                sys.exit("deqp-repro-ref71: %s was compiled for ver %s" % (case, js["gpu"]["ver"]))
            rows += [(case, s, "yes", "ok", str(len(js[s]["code"])), str(js[s]["threads"]),
                      str(len(js[s]["uniforms"]))) for s in stages]
    finally:
        shutil.rmtree(work, True)

    rows.sort(key=lambda r: (r[0], r[1]))
    n_yes = sum(r[2] == "yes" for r in rows)
    with open(out_path, "w") as f:
        f.write("# SPDX-License-Identifier: Apache-2.0\n")
        f.write("# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n")
        f.write("# Mesa 26.2.1 v3d_shc --device 71 (rev 7, compat_rev 0) over the dEQP-GLES2\n")
        f.write("# shader constants of tests/deqp_repro_shaders.rs and tests/mali_deqp_*.rs\n")
        f.write("# (dEQP, Apache-2.0; names and counts only); made by\n")
        f.write("# V3D_SHC=<v3d_shc> tools/cts/deqp-repro-ref71.sh. A lone stage was run with a\n")
        f.write("# stub partner whose numbers are not here.\n")
        f.write("# programs: %d, rows: %d, compiled: %d, refused: %d\n"
                % (len(progs), len(rows), n_yes, len(rows) - n_yes))
        f.write("# columns: case, stage, compiled, reason, instructions, threads, uniforms\n")
        for r in rows:
            f.write("\t".join(r).replace("\n", " / ") + "\n")
    print("deqp-repro-ref71: %d programs, %d rows, %d compiled at 7.1, %d refused -> %s"
          % (len(progs), len(rows), n_yes, len(rows) - n_yes, out_path))


if __name__ == "__main__":
    main()
