#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Score what the in-process front end COMPUTES against dEQP-GLES2's own values.

`tools/online_oracle.py` asks whether the online front end accepts each
dEQP shader. This asks whether the accepted shader gives dEQP's answer: every
`expect pass` case in `data/gles2/shaders/*.test` carries input values and
the outputs they must produce, and dEQP's own harness program compares them
in the shader and writes white on success. Each sub-case is compiled with the
online front end and RUN with the reference IR evaluator (`yakogl-tools
online-eval`, `src/shaderc/online/eval.rs`), and scored on that colour.

    tools/online_eval_oracle.py                  # the whole gles2 shader set
    tools/online_eval_oracle.py operators        # only these files
    tools/online_eval_oracle.py --show 20        # list the first 20 failures

Why it exists: on 2026-09-23 five wrong-code defects were found in a row --
int literals lowered as floats, constructors that never converted, scopes
that never ended, bool uniforms read as 0/1, any()/all() over unwritten
lanes -- and every one of them COMPILED. The compile oracle scored them all
as passes. This is the instrument for that class.

What it measures, and what it cannot
------------------------------------

It measures the FRONT END: the module the lowering produced, run by an
evaluator that implements the IR contract (`docs/online-shaderc/ir.md`). A
back end that miscompiles a correct module is invisible here by construction;
that is the board's and the back ends' own gates' question.

The outcomes are four and are never folded into each other:

* **pass**       -- the program ran and wrote dEQP's success colour
* **FAIL**       -- it ran and wrote anything else, or discarded: a defect
* **refused**    -- the front end refused it (the compile oracle's column;
                    counted here so the populations add up, not scored)
* **unmeasured** -- the evaluator cannot run it (a texture, a derivative, a
                    uniform array it does not place), printed by reason

Only `expect pass` cases carry values to check. A case with no outputs
passes trivially in dEQP too (it writes white); those are counted apart as
"no outputs", because a pass with nothing checked is not a measurement.

The varyings between the stages are passed per-vertex and uninterpolated,
which is exact for dEQP's value cases: every vertex of the quad carries the
same inputs.
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import es100_oracle as oracle  # noqa: E402  (the path is set up just above)
import online_oracle  # noqa: E402  (the ES 3.00 program generator, for --gles3)

REPO = Path(__file__).resolve().parent.parent

# Component count of each value type dEQP's gles2 and gles3 sets use.
SIZES = {"float": 1, "vec2": 2, "vec3": 3, "vec4": 4, "int": 1, "ivec2": 2, "ivec3": 3, "ivec4": 4,
         "uint": 1, "uvec2": 2, "uvec3": 3, "uvec4": 4,
         "bool": 1, "bvec2": 2, "bvec3": 3, "bvec4": 4, "mat2": 4, "mat3": 9, "mat4": 16,
         "mat2x3": 6, "mat2x4": 8, "mat3x2": 6, "mat3x4": 12, "mat4x2": 8, "mat4x3": 12}


class ValueParser(oracle.Parser):
    """es100_oracle's parser, keeping the values it skips.

    dEQP's syntax (gluShaderLibrary.cpp parseValue): `= [ a | b | c ];` for
    one value per sub-case, or a single value for all of them; a scalar is a
    number (optionally negated) or true/false, anything wider is written as
    its constructor with every component: `vec2(1.0, -2.0)`.
    """

    def values(self, c):
        self.take("values")
        self.take("{")
        c.data = {}
        while self.kind() != "}":
            storage = self.take()
            type_ = self.take()
            if type_ not in oracle.TYPES:
                raise oracle.ParseError(f"{self.path}: value type {type_!r} in {c.name}")
            name = self.take("ident") if self.kind() == "ident" else oracle.parse_string_literal(self.take("string"))
            self.take("=")
            if self.kind() == "[":
                self.take("[")
                vals = [self.element(type_)]
                while self.kind() == "|":
                    self.take("|")
                    vals.append(self.element(type_))
                self.take("]")
            else:
                vals = [self.element(type_)]
            self.take(";")
            {"uniform": c.uniforms, "input": c.inputs, "output": c.outputs}[storage].append(oracle.Value(name, type_))
            c.data[name] = vals
        self.take("}")

    def scalar(self):
        neg = False
        if self.kind() == "-":
            self.take()
            neg = True
        t = self.take()
        if t in ("true", "false"):
            v = 1.0 if t == "true" else 0.0
        else:
            v = float(t)
        return -v if neg else v

    def element(self, type_):
        n = SIZES.get(type_)
        if n is None:
            raise oracle.ParseError(f"{self.path}: no size for value type {type_}")
        if n == 1:
            return [self.scalar()]
        self.take()  # the type name
        self.take("(")
        comps = [self.scalar()]
        while self.kind() == ",":
            self.take(",")
            comps.append(self.scalar())
        self.take(")")
        if len(comps) == 1:
            comps = comps * n
        if len(comps) != n:
            raise oracle.ParseError(f"{self.path}: {type_} value with {len(comps)} components")
        return comps


class Evaluator:
    """One `yakogl-tools online-eval` process, spoken to in JSON lines."""

    def __init__(self, tools):
        self.p = subprocess.Popen([str(tools), "online-eval"], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, text=True, bufsize=1)

    def ask(self, req):
        self.p.stdin.write(json.dumps(req) + "\n")
        self.p.stdin.flush()
        line = self.p.stdout.readline()
        if not line:
            raise SystemExit("online_eval_oracle: online-eval exited")
        r = json.loads(line)
        if "error" in r:
            raise SystemExit(f"online_eval_oracle: the tool is broken, not the shader: {r['error']}")
        return r

    def close(self):
        self.p.stdin.close()
        self.p.wait()


def sub_case_count(c):
    n = 1
    for vals in c.data.values():
        if len(vals) > 1:
            if n > 1 and len(vals) != n:
                raise oracle.ParseError(f"{c.path}: {c.name} has value arrays of different lengths")
            n = len(vals)
    return n


def pick(vals, i):
    return vals[i] if len(vals) > 1 else vals[0]


def request(c, vert, frag, i):
    """The request for sub-case `i`: dEQP's attribute, uniform and reference
    values, encoded as its harness sets them (gluShaderLibrary.cpp)."""
    attribs = [["dEQP_Position", [0.0, 0.0, 0.0, 1.0]]]
    for v in c.inputs:
        # Both spellings: the vertex-only and complete wrappers name a float
        # input's attribute after the input and any other `a_<name>`, but
        # the fragment-only wrapper's vertex shader uses `a_<name>` for
        # every type. The tool ignores a name the program does not declare.
        val = pick(c.data[v.name], i)
        attribs.append([v.name, val, v.type])
        attribs.append([f"a_{v.name}", val, v.type])
    uniforms = []
    for v in c.uniforms:
        # A struct member by its GL name (`s.a`): the reflection flattens
        # struct uniforms to exactly these names.
        uniforms.append([v.name, v.type, pick(c.data[v.name], i)])
    for v in c.outputs:
        uniforms.append([f"ref_{v.name}", v.type, pick(c.data[v.name], i)])
    return {"vert": vert, "frag": frag, "attribs": attribs, "uniforms": uniforms}


def run_file(path, ev, show, gles3=False):
    cases = ValueParser(oracle.tokenize(path.read_text()), path.name).parse()
    r = {"sub_cases": 0, "pass": 0, "fail": 0, "refused": 0, "unmeasured": 0, "no_outputs": 0}
    fails, unmeasured = [], {}
    for c in cases:
        if c.expect != "pass" or not hasattr(c, "data"):
            continue
        progs = online_oracle.programs_for(c, gles3)
        for name, vert, frag in progs:
            for i in range(sub_case_count(c)):
                r["sub_cases"] += 1
                a = ev.ask(request(c, vert, frag, i))
                if "refused" in a:
                    r["refused"] += 1
                elif "unmeasured" in a:
                    r["unmeasured"] += 1
                    why = a["unmeasured"]
                    unmeasured[why] = unmeasured.get(why, 0) + 1
                elif a.get("discarded"):
                    r["fail"] += 1
                    fails.append({"case": f"{name}#{i}", "got": "discarded"})
                elif "undefined" in a:
                    # A lane no instruction defined reached an output (or a
                    # branch, or an index): a front-end defect a scalar GPU
                    # turns into garbage, so a FAIL, never unmeasured.
                    r["fail"] += 1
                    fails.append({"case": f"{name}#{i}", "got": "undefined lane: " + a["undefined"]})
                else:
                    # A NaN lane arrives as null (JSON has no NaN): a wrong
                    # colour like any other, never a crash of the instrument.
                    col = [float("nan") if x is None else x for x in a["color"]]
                    if all(x > 0.5 for x in col[:3]):
                        if c.outputs:
                            r["pass"] += 1
                        else:
                            r["no_outputs"] += 1
                    else:
                        r["fail"] += 1
                        fails.append({"case": f"{name}#{i}", "got": col,
                                      "inputs": {v.name: pick(c.data[v.name], i) for v in c.inputs},
                                      "expected": {v.name: pick(c.data[v.name], i) for v in c.outputs}})
    return r, fails, unmeasured


def head_commit():
    try:
        out = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=REPO,
                             capture_output=True, text=True, check=True)
        return out.stdout.strip()
    except (subprocess.CalledProcessError, OSError):
        return "unknown"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="*", help="test file base names (default: all)")
    ap.add_argument("--cts", type=Path, default=oracle.DEFAULT_CTS)
    ap.add_argument("--tools", type=Path, default=None)
    ap.add_argument("--show", type=int, default=5, metavar="N", help="list the first N failures per file")
    ap.add_argument("--json", type=Path, default=None)
    ap.add_argument("--gles3", action="store_true",
                    help="dEQP-GLES3's corpus (data/gles3/shaders), its `version 300 es` cases")
    a = ap.parse_args()

    data = a.cts / ("data/gles3/shaders" if a.gles3 else "data/gles2/shaders")
    if not data.is_dir():
        raise SystemExit(f"online_eval_oracle: no dEQP shader data at {data} (--cts, or $VK_GL_CTS)")
    paths = sorted(data.glob("*.test"))
    if a.files:
        want = {f if f.endswith(".test") else f + ".test" for f in a.files}
        paths = [p for p in paths if p.name in want]
        if not paths:
            raise SystemExit(f"online_eval_oracle: no such test file: {' '.join(sorted(want))}")
    tools = a.tools
    if tools is None:
        subprocess.run(["cargo", "build", "-p", "yakogl-tools"], cwd=REPO, check=True)
        tools = REPO / ".target/debug/yakogl-tools"

    print(f"online front end at {head_commit()}, evaluated against dEQP values in {data}\n")
    print(f"{'file':<34}{'sub-cases':>10}{'pass':>7}{'FAIL':>7}{'refused':>9}{'unmeas':>8}{'no-out':>8}")
    ev = Evaluator(tools)
    totals = {"sub_cases": 0, "pass": 0, "fail": 0, "refused": 0, "unmeasured": 0, "no_outputs": 0}
    all_fails, all_unmeasured, report = [], {}, {}
    try:
        for p in paths:
            r, fails, unm = run_file(p, ev, a.show, a.gles3)
            report[p.name] = r
            for k in totals:
                totals[k] += r[k]
            for k, n in unm.items():
                all_unmeasured[k] = all_unmeasured.get(k, 0) + n
            all_fails.extend(fails)
            flag = "  <-" if r["fail"] else ""
            print(f"{p.name:<34}{r['sub_cases']:>10}{r['pass']:>7}{r['fail']:>7}{r['refused']:>9}"
                  f"{r['unmeasured']:>8}{r['no_outputs']:>8}{flag}")
            for f in fails[:a.show]:
                print(f"    FAIL {f['case']}: got {f['got']}"
                      + (f"  in {f['inputs']} want {f['expected']}" if "inputs" in f else ""))
    finally:
        ev.close()

    print(f"\n{'TOTAL':<34}{totals['sub_cases']:>10}{totals['pass']:>7}{totals['fail']:>7}"
          f"{totals['refused']:>9}{totals['unmeasured']:>8}{totals['no_outputs']:>8}")
    counted = sum(totals[k] for k in ("pass", "fail", "refused", "unmeasured", "no_outputs"))
    if counted != totals["sub_cases"]:
        raise SystemExit(f"online_eval_oracle: {totals['sub_cases']} sub-cases but {counted} outcomes")
    if totals["sub_cases"] == 0:
        raise SystemExit("online_eval_oracle: measured nothing")
    if all_unmeasured:
        print("\nunmeasured, by reason (never a pass, never a fail):\n")
        for why, n in sorted(all_unmeasured.items(), key=lambda kv: -kv[1]):
            print(f"  {n:>5}  {why}")
    print(f"\nwrong results: {totals['fail']} of {totals['sub_cases']} sub-cases "
          f"({totals['pass']} pass, {totals['refused']} refused, {totals['unmeasured']} unmeasured, "
          f"{totals['no_outputs']} with no outputs to check)")
    if a.json:
        a.json.write_text(json.dumps({"commit": head_commit(), "totals": totals, "files": report,
                                      "fails": all_fails, "unmeasured": all_unmeasured}, indent=2) + "\n")
    return 1 if totals["fail"] else 0


if __name__ == "__main__":
    sys.exit(main())
