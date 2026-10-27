#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Measure the in-process ("online") front end against dEQP-GLES2's own cases.

`tools/es100_oracle.py` scores the pipeline that ships today. This scores
`src/shaderc/online/`, the compiler being built for the target, against the
same corpus, and it deliberately does not touch that script: the 2787/2793 it
reports must not be able to move for a reason that has nothing to do with the
pipeline it measures.

It reuses that script's dEQP `.test` parser by importing it, so the two agree
about what a case says, and speaks to `yakogl-tools online-check`, which
answers with the same JSON protocol `shader-check` uses.

    tools/online_oracle.py                  # the whole gles2 shader set
    tools/online_oracle.py preprocessor     # only these files
    tools/online_oracle.py --show 20        # list the first 20 false rejects

What it counts, and why the three columns are not one score
-----------------------------------------------------------

The front end is incomplete by construction -- today the built-in function
signatures, the precision-qualifier rules and the cross-stage link rules are
still missing. A single "N of M correct" over a corpus where many failures are
*supposed* to come from rules that do not exist yet would say nothing about
whether today's code is right. So the cases are split by what they can
possibly measure:

* **false rejects** -- cases dEQP says must BUILD that the front end refuses.
  Every one is a defect in code that exists, and the number must be 0. This is
  the column that can fail.
* **must-fail rejected** -- cases dEQP says must fail to compile that the front
  end already refuses, out of how many there are. This only ever goes up as
  passes land; it is coverage, not a pass mark, and a case not yet rejected is
  not a defect. It can also go *down* for a good reason: when a case was being
  refused for the wrong reason and that reason is fixed, a correct verdict
  reached by a wrong argument stops counting.
* **excluded** -- cases whose verdict needs a step that does not exist (a link
  error), or that are not GLSL ES 1.00. They are reported as excluded, never
  scored, because a pipeline that cannot answer must not be recorded as having
  answered.

`online-check` answers a link request with a service error rather than a
verdict, which is what keeps the third column honest: if it answered `ok` the
link_fail cases would silently become passes.
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import es100_oracle as oracle  # noqa: E402  (the path is set up just above)

REPO = Path(__file__).resolve().parent.parent

# What dEQP's `expect` means for a front end that has no link step.
MUST_BUILD = ("pass", "build_successful", "validation_fail")
MUST_COMPILE_FAIL = ("compile_fail",)
NEEDS_LINK = ("link_fail", "compile_link_fail")


# ---------------------------------------------------------------------------
# dEQP-GLES3: the same corpus shape, the programs glsShaderLibraryCase.cpp
# writes for GLSL_VERSION_300_ES.
#
# The differences from ES 1.00, and nothing else, are carried here -- the
# `in`/`out` qualifiers, `highp` in the fragment shader, and the fragment
# colour as `layout(location = 0) out mediump vec4 dEQP_FragColor` -- so the
# ES 1.00 generator in es100_oracle.py, whose score must not move, is not
# touched. Line references are to modules/glshared/glsShaderLibraryCase.cpp.
# ---------------------------------------------------------------------------

ES3_COLOR_OUT = "layout(location = 0) out mediump vec4 dEQP_FragColor;\n"


def _is_float(t):
    return oracle.TYPES[t][0] == "float"


def gen_vertex_shader3(c):
    """genVertexShader (:141) for 300 es."""
    s = ["#version 300 es\n", "precision highp float;\n", "precision highp int;\n", "\n",
         "in highp vec4 dEQP_Position;\n"]
    for v in c.inputs:
        ft = oracle.TYPES[v.type][1]
        s.append(f"in {ft} a_{v.name};\n")
        s.append(f"out {ft} {v.name if _is_float(v.type) else 'v_' + v.name};\n")
    s.append("\nvoid main()\n{\n    gl_Position = dEQP_Position;\n")
    for v in c.inputs:
        name = v.name if _is_float(v.type) else f"v_{v.name}"
        s.append(f"    {name} = a_{v.name};\n")
    s.append("}\n")
    return "".join(s)


def gen_fragment_shader3(c):
    """genFragmentShader (:233) for 300 es: highp, and dEQP_FragColor."""
    s = ["#version 300 es\n", "precision highp float;\n", "precision highp int;\n", "\n",
         ES3_COLOR_OUT, "\n", oracle.gen_compare_functions(c.outputs, True), "\n"]
    for v in c.outputs:
        ft = oracle.TYPES[v.type][1]
        s.append(f"in {ft} {v.name if _is_float(v.type) else 'v_' + v.name};\n")
        s.append(f"uniform {v.type} ref_{v.name};\n")
    s.append("\nvoid main()\n{\n    ")
    s.append(oracle.gen_compare_op("dEQP_FragColor", c.outputs, "v_"))
    s.append("}\n")
    return "".join(s)


def specialize_vertex_only3(c):
    """specializeVertexShader (:285)."""
    decl = ["in highp vec4 dEQP_Position;\n"]
    setup, output = [], ["gl_Position = dEQP_Position;\n"]
    for v in c.inputs:
        ft = oracle.TYPES[v.type][1]
        if _is_float(v.type):
            decl.append(f"in {ft} {v.name};\n")
        else:
            decl.append(f"in {ft} a_{v.name};\n")
            setup.append(f"{v.type} {v.name} = {v.type}(a_{v.name});\n")
    for v in c.outputs:
        ft = oracle.TYPES[v.type][1]
        if _is_float(v.type):
            decl.append(f"out {ft} {v.name};\n")
        else:
            decl.append(f"out {ft} v_{v.name};\n")
            decl.append(f"{v.type} {v.name};\n")
            output.append(f"v_{v.name} = {ft}({v.name});\n")
    return oracle.specialize(c.both, {
        "DECLARATIONS": "".join(decl),
        "SETUP": "".join(setup),
        "OUTPUT": "".join(output),
        "POSITION_FRAG_COLOR": "gl_Position",
    })


def specialize_fragment_only3(c):
    """specializeFragmentShader (:359)."""
    decl = [oracle.gen_compare_functions(c.outputs, False), ES3_COLOR_OUT]
    setup = []
    for v in c.inputs:
        ft = oracle.TYPES[v.type][1]
        if _is_float(v.type):
            decl.append(f"in {ft} {v.name};\n")
        else:
            decl.append(f"in {ft} v_{v.name};\n")
            off = " * 1.0025" if v.type in ("int", "ivec2", "ivec3", "ivec4") else ""
            setup.append(f"{v.type} {v.name} = {v.type}(v_{v.name}{off});\n")
    for v in c.outputs:
        decl.append(f"uniform {v.type} ref_{v.name};\n")
        decl.append(f"{v.type} {v.name};\n")
    return oracle.specialize(c.both, {
        "DECLARATIONS": "".join(decl),
        "SETUP": "".join(setup),
        "OUTPUT": oracle.gen_compare_op("dEQP_FragColor", c.outputs, ""),
        "POSITION_FRAG_COLOR": "dEQP_FragColor",
    })


def specialize_complete3(c):
    """generateVertexSpecialization (:441) / generateFragmentSpecialization (:480)."""
    vdecl, vsetup = ["in highp vec4 dEQP_Position;\n"], []
    for v in c.inputs:
        if _is_float(v.type):
            vdecl.append(f"in {v.type} {v.name};\n")
        else:
            ft = oracle.TYPES[v.type][1]
            vdecl.append(f"in {ft} a_{v.name};\n")
            vsetup.append(f"{v.type} {v.name} = {v.type}(a_{v.name});\n")
    vdecl.append(oracle.gen_uniform_declarations(c.uniforms))
    fdecl = [oracle.gen_compare_functions(c.outputs, False), ES3_COLOR_OUT]
    for v in c.outputs:
        fdecl.append(f"uniform {v.type} ref_{v.name};\n")
        fdecl.append(f"{v.type} {v.name};\n")
    fdecl.append(oracle.gen_uniform_declarations(c.uniforms))
    vert = oracle.specialize(c.vertex, {
        "VERTEX_DECLARATIONS": "".join(vdecl),
        "VERTEX_SETUP": "".join(vsetup),
        "VERTEX_OUTPUT": "gl_Position = dEQP_Position;\n",
    })
    frag = oracle.specialize(c.fragment, {
        "FRAGMENT_DECLARATIONS": "".join(fdecl),
        "FRAGMENT_OUTPUT": oracle.gen_compare_op("dEQP_FragColor", c.outputs, ""),
        "FRAG_COLOR": "dEQP_FragColor",
    })
    return vert, frag


def programs_of3(c):
    """programs_of, for a `version 300 es` case."""
    if c.unsupported:
        return []
    if c.both is not None:
        return [
            (c.name + "_vertex", specialize_vertex_only3(c), gen_fragment_shader3(c)),
            (c.name + "_fragment", gen_vertex_shader3(c), specialize_fragment_only3(c)),
        ]
    if c.vertex is None or c.fragment is None:
        return []
    vert, frag = specialize_complete3(c)
    return [(c.name, vert, frag)]


def programs_for(c, gles3):
    """The programs to judge: ES 1.00 cases in the gles2 corpus, ES 3.00 in gles3."""
    if gles3:
        return programs_of3(c) if c.version == 300 else []
    return oracle.programs_of(c) if c.version == 100 else []


class Online:
    """One `yakogl-tools online-check` process, spoken to in JSON lines."""

    def __init__(self, tools, scratch_kib):
        self.p = subprocess.Popen(
            [str(tools), "online-check", "--scratch", str(scratch_kib)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)

    def compiles(self, stage, src):
        """(True, '') if the front end accepts the shader, else (False, why)."""
        self.p.stdin.write(json.dumps({"op": "compile", "stage": stage, "source": src}) + "\n")
        self.p.stdin.flush()
        line = self.p.stdout.readline()
        if not line:
            raise SystemExit("online_oracle: online-check exited")
        r = json.loads(line)
        if "ok" not in r:
            # A broken service, not a broken shader. Never scored as a verdict.
            raise SystemExit(f"online_oracle: the tool is broken, not the shader: {r.get('error')}")
        return r["ok"], oracle.first_line(r.get("error"))

    def close(self):
        self.p.stdin.close()
        self.p.wait()


def run_file(path, tools, scratch_kib, show, gles3=False):
    cases = oracle.Parser(oracle.tokenize(path.read_text()), path.name).parse()
    pipe = Online(tools, scratch_kib)
    r = {"must_build": 0, "false_rejects": 0, "must_fail": 0, "must_fail_rejected": 0, "excluded": 0}
    detail = []
    accepts = []
    try:
        for c in cases:
            progs = programs_for(c, gles3)
            if not progs:
                r["excluded"] += 1
                continue
            for name, vert, frag in progs:
                if c.expect in NEEDS_LINK:
                    r["excluded"] += 1
                    continue
                rejected, why = False, ""
                for stage, src in (("vert", vert), ("frag", frag)):
                    ok, w = pipe.compiles(stage, src)
                    if not ok:
                        rejected, why = True, f"{stage}: {w}"
                        break
                if c.expect in MUST_BUILD:
                    r["must_build"] += 1
                    if rejected:
                        r["false_rejects"] += 1
                        detail.append({"case": name, "expect": c.expect, "why": why})
                elif c.expect in MUST_COMPILE_FAIL:
                    r["must_fail"] += 1
                    if rejected:
                        r["must_fail_rejected"] += 1
                    else:
                        # A FALSE ACCEPT: dEQP says this must not compile and
                        # we compiled it. Named, not just counted -- see the
                        # note under the totals.
                        accepts.append({"case": name, "file": path.name})
                else:
                    r["excluded"] += 1
    finally:
        pipe.close()
    r["shown"] = detail[:show] if show else []
    r["detail"] = detail
    r["accepts"] = accepts
    return r


def reasons_by_construct(detail):
    """The false rejects grouped by reason, commonest first.

    The reason is the message without its line and token, because
    `0:15: \'in0\' : reading this is not lowered yet` and
    `0:10: \'var\' : reading this is not lowered yet` are one missing pass,
    not two. Each group keeps one whole example so the group can be
    reproduced without re-running the sweep.
    """
    groups = {}
    for d in detail:
        why = d["why"]
        # "frag: 0:15: \'in0\' : reading this is not lowered yet" -> the tail.
        msg = why.rsplit(" : ", 1)[-1] if " : " in why else why
        g = groups.setdefault(msg, [0, d])
        g[0] += 1
    return sorted(((m, n, eg) for m, (n, eg) in groups.items()), key=lambda t: -t[1])


def build_tools():
    subprocess.run(["cargo", "build", "-p", "yakogl-tools"], cwd=REPO, check=True)
    return REPO / ".target/debug/yakogl-tools"


def head_commit():
    try:
        out = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=REPO,
                             capture_output=True, text=True, check=True)
        return out.stdout.strip()
    except (subprocess.CalledProcessError, OSError):
        return "unknown"


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="*", help="test file base names (default: all)")
    ap.add_argument("--cts", type=Path, default=oracle.DEFAULT_CTS)
    ap.add_argument("--tools", type=Path, default=None)
    ap.add_argument("--scratch", type=int, default=4096, metavar="KIB")
    ap.add_argument("--show", type=int, default=5, metavar="N",
                    help="list the first N false rejects per file")
    ap.add_argument("--json", type=Path, default=None)
    ap.add_argument("--gles3", action="store_true",
                    help="dEQP-GLES3's corpus (data/gles3/shaders), its `version 300 es` cases")
    a = ap.parse_args()

    data = a.cts / ("data/gles3/shaders" if a.gles3 else "data/gles2/shaders")
    if not data.is_dir():
        raise SystemExit(f"online_oracle: no dEQP shader data at {data} (--cts, or $VK_GL_CTS)")
    paths = sorted(data.glob("*.test"))
    if a.files:
        want = {f if f.endswith(".test") else f + ".test" for f in a.files}
        paths = [p for p in paths if p.name in want]
        if not paths:
            raise SystemExit(f"online_oracle: no such test file: {' '.join(sorted(want))}")

    tools = a.tools or build_tools()
    commit = head_commit()
    print(f"online front end at {commit}, dEQP data {data}\n")
    print(f"{'file':<34}{'must-build':>12}{'false rej':>11}{'must-fail':>11}{'rejected':>10}{'excl':>7}")

    totals = {"must_build": 0, "false_rejects": 0, "must_fail": 0, "must_fail_rejected": 0, "excluded": 0}
    report = {}
    reasons = []
    accepted = []
    for p in paths:
        r = run_file(p, tools, a.scratch, a.show, a.gles3)
        report[p.name] = {k: v for k, v in r.items() if k not in ("detail", "shown", "accepts")}
        reasons.extend(r["detail"])
        accepted.extend(r["accepts"])
        for k in totals:
            totals[k] += r[k]
        flag = "  <-" if r["false_rejects"] else ""
        print(f"{p.name:<34}{r['must_build']:>12}{r['false_rejects']:>11}"
              f"{r['must_fail']:>11}{r['must_fail_rejected']:>10}{r['excluded']:>7}{flag}")
        for d in r["shown"]:
            print(f"    FALSE REJECT {d['case']} (expect {d['expect']}): {d['why']}")

    print(f"\n{'TOTAL':<34}{totals['must_build']:>12}{totals['false_rejects']:>11}"
          f"{totals['must_fail']:>11}{totals['must_fail_rejected']:>10}{totals['excluded']:>7}")
    if reasons:
        print("\nthe false rejects, by the construct each one stops on:\n")
        for msg, n, eg in reasons_by_construct(reasons):
            print(f"  {n:>5}  {msg}")
            print(f"         e.g. {eg['case']}: {eg['why']}")

    if accepted:
        print("\nFALSE ACCEPTS -- dEQP says these must NOT compile and we compiled them:\n")
        by_file = {}
        for d in accepted:
            by_file.setdefault(d["file"], []).append(d["case"])
        for fn, names in sorted(by_file.items(), key=lambda kv: -len(kv[1])):
            print(f"  {len(names):>5}  {fn}")
            for n in sorted(names)[:2]:
                print(f"         e.g. {n}")

    print(f"\nfalse rejects: {totals['false_rejects']} (must be 0 -- each one is a defect in code that exists)")
    print(f"false accepts: {len(accepted)} of {totals['must_fail']} must-fail cases")
    if accepted:
        print("  These are a WORK LIST, not coverage. Each is a rule the compiler does not have yet,")
        print("  and until it does we compile a shader a conformant implementation refuses -- which an")
        print("  application then ships, and which fails elsewhere. The column was long phrased as")
        print("  'rejected N of M (coverage, not a pass mark)', and phrasing it that way is why nobody")
        print("  read it as defects.")
    else:
        print("  Every case in this corpus that must fail to compile now does.")
    print("  0 here is NOT conformance, and this is the sentence to remember when it reads 0:")
    print("  dEQP is not a complete oracle for this direction. The missing default float precision")
    print("  (section 4.5.3) was found ON SILICON, by running the reference and online pairs side")
    print("  by side, and dEQP's shader corpus has no case for it at all -- its own templates")
    print("  always declare it. 0 means this corpus has nothing more to say, not that the next")
    print("  invalid shader will be refused.")

    if a.json:
        a.json.write_text(json.dumps({"front_end": "online", "commit": commit,
                                      "totals": totals, "files": report}, indent=2) + "\n")

    return 1 if totals["false_rejects"] else 0


if __name__ == "__main__":
    sys.exit(main())
