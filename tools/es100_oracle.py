#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Measure this shader pipeline against dEQP-GLES2's own shader cases.

dEQP ships its `shaders.*` tests as data: `data/gles2/shaders/*.test` in a
VK-GL-CTS checkout holds ~1,400 cases, each with the exact GLSL ES 1.00 text
and a stated `expect compile_fail | link_fail | pass`. That makes it an oracle
a host can consult without a GPU: whether a shader *must* fail to compile, must
fail to link, or must build is a property of the language, and the pipeline
either agrees or does not.

This script parses those files the way dEQP's own parser does
(`framework/opengl/gluShaderLibrary.cpp` and
`modules/glshared/glsShaderLibraryCase.cpp`: the same lexer, the same `${...}`
specialization, the same expansion of a `both` case into a `_vertex` and a
`_fragment` sub-case), asks `yakogl-tools shader-check` for a verdict on each,
and prints how many cases' expectations were met, per file, with the
mismatches. No GPU, no pco_shc and no dEQP build are involved.

What it does not measure: whether a case that builds also *renders* the right
values. A `pass` here means the program built, which is the part a host can
check.

    tools/es100_oracle.py                       # the whole gles2 shader set
    tools/es100_oracle.py --legacy              # the pipeline before the ES front end
    tools/es100_oracle.py --show-mismatches 20  # list the first 20 per file
    tools/es100_oracle.py --codegen             # run pco_shc too (needs PCO_SHC)
    tools/es100_oracle.py linkage preprocessor  # only these files

Options: `--cts DIR` (a VK-GL-CTS checkout; default
`~/com/github/KhronosGroup/VK-GL-CTS`, `$VK_GL_CTS` overrides), `--tools PATH`
(a prebuilt `yakogl-tools`; by default the script builds the repository's own),
`--jobs N`, `--json FILE`.
"""

import argparse
import json
import os
import queue
import re
import subprocess
import threading
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_CTS = Path(os.environ.get("VK_GL_CTS", Path.home() / "com/github/KhronosGroup/VK-GL-CTS"))

# ---------------------------------------------------------------------------
# The .test lexer — ShaderParser::advanceToken, gluShaderLibrary.cpp:560.
# ---------------------------------------------------------------------------

KEYWORDS = {
    "true", "false", "desc", "expect", "group", "case", "end", "output_color",
    "format", "values", "both", "vertex", "fragment", "uniform", "input",
    "output", "float", "vec2", "vec3", "vec4", "mat2", "mat2x3", "mat2x4",
    "mat3x2", "mat3", "mat3x4", "mat4x2", "mat4x3", "mat4", "int", "ivec2",
    "ivec3", "ivec4", "uint", "uvec2", "uvec3", "uvec4", "bool", "bvec2",
    "bvec3", "bvec4", "version", "tessellation_control",
    "tessellation_evaluation", "geometry", "require", "in", "import",
    "pipeline_program", "active_stages",
}

# dEQP's isCaseNameChar: a token that starts with a letter runs on
# through '.' and '-' too, so `u_struct.member` is one identifier.
_IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_.\-]*")
_NUMBER = re.compile(r"\d+(\.\d*([eE][-+]?\d+)?)?")


class ParseError(Exception):
    pass


def tokenize(text):
    """dEQP's lexer: '#' starts an end-of-line comment, '""' quotes a shader."""
    toks, i, n = [], 0, len(text)
    while True:
        while True:
            while i < n and text[i] in " \t\r\n":
                i += 1
            if i < n and text[i] == "#":
                while i < n and text[i] not in "\r\n":
                    i += 1
            else:
                break
        if i >= n:
            toks.append(("eof", ""))
            return toks
        c = text[i]
        if c.isalpha() or c == "_":
            m = _IDENT.match(text, i)
            w = m.group(0)
            toks.append((w if w in KEYWORDS else "ident", w))
            i = m.end()
        elif c.isdigit():
            m = _NUMBER.match(text, i)
            w = m.group(0)
            toks.append(("floatlit" if "." in w else "intlit", w))
            i = m.end()
        elif c == '"' and i + 1 < n and text[i + 1] == '"':
            j = i + 2
            while not (text[j] == '"' and text[j + 1] == '"'):
                j += 2 if text[j] == "\\" else 1
            toks.append(("source", text[i:j + 2]))
            i = j + 2
        elif c in "\"'":
            j = i + 1
            while text[j] != c:
                j += 2 if text[j] == "\\" else 1
            toks.append(("string", text[i:j + 1]))
            i = j + 1
        elif c in "=+-,|;()[]{}>":
            toks.append((c, c))
            i += 1
        else:
            raise ParseError(f"unexpected character {c!r}")


def parse_shader_source(tok):
    """ShaderParser::parseShaderSource + removeExtraIndentation."""
    p = tok[2:].lstrip(" ")
    if p.startswith("\r"):
        p = p[1:]
    if p.startswith("\n"):
        p = p[1:]
    body, i = [], 0
    while not (p[i] == '"' and p[i + 1] == '"'):
        if p[i] == "\\":
            body.append({"n": "\n", "t": "\t"}.get(p[i + 1], p[i + 1]))
            i += 2
        else:
            body.append(p[i])
            i += 1
    return remove_extra_indentation("".join(body))


def remove_extra_indentation(src):
    """gluShaderLibrary.cpp: the first line sets the indent; a tab is four."""
    indent, k = 0, 0
    while k < len(src) and src[k] in " \t\r\n":
        indent += 4 if src[k] == "\t" else 1
        k += 1
    out, at_line_start, omitted, pos = [], True, 0, 0
    while pos < len(src):
        c = src[pos]
        if at_line_start and omitted < indent and c in " \t":
            omitted += 4 if c == "\t" else 1
        elif c in "\r\n":
            if c == "\r" and pos + 1 < len(src) and src[pos + 1] == "\n":
                pos += 1
            out.append("\n")
            at_line_start, omitted = True, 0
        else:
            out.append(c)
            at_line_start = False
        pos += 1
    return "".join(out)


def parse_string_literal(tok):
    return tok[1:-1].replace("\\n", "\n").replace("\\t", "\t")


# ---------------------------------------------------------------------------
# The GLSL types a `values` block may name: (scalar kind, all-float shape).
# ---------------------------------------------------------------------------

TYPES = {
    "float": ("float", "float"), "vec2": ("float", "vec2"),
    "vec3": ("float", "vec3"), "vec4": ("float", "vec4"),
    "int": ("int", "float"), "ivec2": ("int", "vec2"),
    "ivec3": ("int", "vec3"), "ivec4": ("int", "vec4"),
    "uint": ("uint", "float"), "uvec2": ("uint", "vec2"),
    "uvec3": ("uint", "vec3"), "uvec4": ("uint", "vec4"),
    "bool": ("bool", "float"), "bvec2": ("bool", "vec2"),
    "bvec3": ("bool", "vec3"), "bvec4": ("bool", "vec4"),
    "mat2": ("float", "mat2"), "mat2x3": ("float", "mat2x3"),
    "mat2x4": ("float", "mat2x4"), "mat3x2": ("float", "mat3x2"),
    "mat3": ("float", "mat3"), "mat3x4": ("float", "mat3x4"),
    "mat4x2": ("float", "mat4x2"), "mat4x3": ("float", "mat4x3"),
    "mat4": ("float", "mat4"),
}


class Value:
    __slots__ = ("name", "type")

    def __init__(self, name, type_):
        self.name, self.type = name, type_


class Case:
    def __init__(self, name, path):
        self.name, self.path = name, path
        self.expect = "pass"
        self.version = 100
        self.requires = []
        self.inputs, self.outputs, self.uniforms = [], [], []
        self.both = self.vertex = self.fragment = None
        self.unsupported = None  # why this oracle cannot judge the case


# ---------------------------------------------------------------------------
# The parser.
# ---------------------------------------------------------------------------

class Parser:
    def __init__(self, toks, path):
        self.t, self.i, self.path = toks, 0, path

    def kind(self):
        return self.t[self.i][0]

    def text(self):
        return self.t[self.i][1]

    def take(self, kind=None):
        k, v = self.t[self.i]
        if kind is not None and k != kind:
            raise ParseError(f"{self.path}: expected {kind}, got {k} {v!r}")
        self.i += 1
        return v

    def parse(self):
        """Every case in the file, named as dEQP names it (groups joined by '.')."""
        cases = []
        while self.kind() != "eof":
            if self.kind() == "group":
                self.group(cases, [])
            elif self.kind() == "case":
                self.case(cases, [])
            elif self.kind() == "import":
                self.take()
                self.take()  # imports are not used by the gles2 shader set
            else:
                raise ParseError(f"{self.path}: unexpected {self.text()!r} at top level")
        return cases

    def group(self, cases, prefix):
        self.take("group")
        name = self.take()  # any token, as in dEQP
        if self.kind() == "string":
            self.take()
        while self.kind() != "end":
            if self.kind() == "group":
                self.group(cases, prefix + [name])
            elif self.kind() == "case":
                self.case(cases, prefix + [name])
            else:
                raise ParseError(f"{self.path}: unexpected {self.text()!r} in group {name}")
        self.take("end")

    def case(self, cases, prefix):
        self.take("case")
        # dEQP allows any token as a case name ("case version", "case uniform").
        name = ".".join(prefix + [self.take()])
        c = Case(name, self.path)
        while self.kind() != "end":
            k = self.kind()
            if k == "desc":
                self.take()
                self.take("string")
            elif k == "expect":
                self.take()
                c.expect = self.take()
            elif k == "values":
                self.values(c)
            elif k == "version":
                self.take()
                c.version = int(self.take("intlit"))
                if self.kind() == "ident" and self.text() == "es":
                    self.take()
            elif k == "require":
                self.take()
                c.requires.append(self.take())
            elif k in ("both", "vertex", "fragment"):
                self.take()
                setattr(c, k, parse_shader_source(self.take("source")))
            elif k in ("tessellation_control", "tessellation_evaluation", "geometry"):
                self.take()
                self.take("source")
                c.unsupported = "needs a stage GL ES 2.0 does not have"
            elif k == "pipeline_program":
                self.skip_pipeline_program()
                c.unsupported = "separable programs are not GL ES 2.0"
            elif k in ("output_color", "format"):
                self.take()
                while self.kind() not in ("end", "eof"):
                    self.take()
            else:
                raise ParseError(f"{self.path}: unexpected {self.text()!r} in case {name}")
        self.take("end")
        cases.append(c)

    def skip_pipeline_program(self):
        self.take("pipeline_program")
        depth = 0
        while True:
            k = self.kind()
            if k == "end" and depth == 0:
                self.take()
                return
            if k == "{":
                depth += 1
            elif k == "}":
                depth -= 1
            self.take()

    def values(self, c):
        self.take("values")
        self.take("{")
        while self.kind() != "}":
            storage = self.take()  # uniform | input | output
            type_ = self.take()
            if type_ not in TYPES:
                raise ParseError(f"{self.path}: value type {type_!r} in {c.name}")
            name = self.take("ident") if self.kind() == "ident" else parse_string_literal(self.take("string"))
            self.take("=")
            self.skip_value()
            self.take(";")
            {"uniform": c.uniforms, "input": c.inputs, "output": c.outputs}[storage].append(Value(name, type_))
        self.take("}")

    def skip_value(self):
        """The values themselves never change a compile or link verdict."""
        depth = 0
        while True:
            k = self.kind()
            if depth == 0 and k == ";":
                return
            if k in "([{":
                depth += 1
            elif k in ")]}":
                depth -= 1
            self.take()


# ---------------------------------------------------------------------------
# Specialization — glsShaderLibraryCase.cpp.
# ---------------------------------------------------------------------------

_PARAM = re.compile(r"\$\{([^}]*)\}")


def specialize(src, params):
    """tcu::StringTemplate::specialize: ${NAME}, :single-line, :opt, :default=X."""
    def sub(m):
        name, _, flags = m.group(1).partition(":")
        if name in params:
            v = params[name]
            return v.replace("\n", " ") if flags == "single-line" else v
        if flags.startswith("default="):
            return flags[len("default="):]
        if flags == "opt":
            return ""
        raise ParseError(f"no value for template parameter {name!r}")
    return _PARAM.sub(sub, src)


_CMP_FLOAT_ARG = {
    "bool": "bool isOk (float a, bool b) { return ((a > 0.5) == b); }",
    "bvec2": "bool isOk (vec2 a, bvec2 b) { return (greaterThan(a, vec2(0.5)) == b); }",
    "bvec3": "bool isOk (vec3 a, bvec3 b) { return (greaterThan(a, vec3(0.5)) == b); }",
    "bvec4": "bool isOk (vec4 a, bvec4 b) { return (greaterThan(a, vec4(0.5)) == b); }",
    "int": "bool isOk (float a, int b)  { float atemp = a+0.5; return (float(b) <= atemp && atemp <= float(b+1)); }",
    "ivec2": "bool isOk (vec2 a, ivec2 b) { return (ivec2(floor(a + 0.5)) == b); }",
    "ivec3": "bool isOk (vec3 a, ivec3 b) { return (ivec3(floor(a + 0.5)) == b); }",
    "ivec4": "bool isOk (vec4 a, ivec4 b) { return (ivec4(floor(a + 0.5)) == b); }",
    "uint": "bool isOk (float a, uint b) { float atemp = a+0.5; return (float(b) <= atemp && atemp <= float(b+1u)); }",
    "uvec2": "bool isOk (vec2 a, uvec2 b) { return (uvec2(floor(a + 0.5)) == b); }",
    "uvec3": "bool isOk (vec3 a, uvec3 b) { return (uvec3(floor(a + 0.5)) == b); }",
    "uvec4": "bool isOk (vec4 a, uvec4 b) { return (uvec4(floor(a + 0.5)) == b); }",
}

_CMP_FLOAT = {
    "float": "bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }",
    "vec2": "bool isOk (vec2 a, vec2 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }",
    "vec3": "bool isOk (vec3 a, vec3 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }",
    "vec4": "bool isOk (vec4 a, vec4 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }",
    "mat2": "bool isOk (mat2 a, mat2 b, float eps) { vec2 diff = max(abs(a[0]-b[0]), abs(a[1]-b[1])); return all(lessThanEqual(diff, vec2(eps))); }",
    "mat2x3": "bool isOk (mat2x3 a, mat2x3 b, float eps) { vec3 diff = max(abs(a[0]-b[0]), abs(a[1]-b[1])); return all(lessThanEqual(diff, vec3(eps))); }",
    "mat2x4": "bool isOk (mat2x4 a, mat2x4 b, float eps) { vec4 diff = max(abs(a[0]-b[0]), abs(a[1]-b[1])); return all(lessThanEqual(diff, vec4(eps))); }",
    "mat3x2": "bool isOk (mat3x2 a, mat3x2 b, float eps) { vec2 diff = max(max(abs(a[0]-b[0]), abs(a[1]-b[1])), abs(a[2]-b[2])); return all(lessThanEqual(diff, vec2(eps))); }",
    "mat3": "bool isOk (mat3 a, mat3 b, float eps) { vec3 diff = max(max(abs(a[0]-b[0]), abs(a[1]-b[1])), abs(a[2]-b[2])); return all(lessThanEqual(diff, vec3(eps))); }",
    "mat3x4": "bool isOk (mat3x4 a, mat3x4 b, float eps) { vec4 diff = max(max(abs(a[0]-b[0]), abs(a[1]-b[1])), abs(a[2]-b[2])); return all(lessThanEqual(diff, vec4(eps))); }",
    "mat4x2": "bool isOk (mat4x2 a, mat4x2 b, float eps) { vec2 diff = max(max(abs(a[0]-b[0]), abs(a[1]-b[1])), max(abs(a[2]-b[2]), abs(a[3]-b[3]))); return all(lessThanEqual(diff, vec2(eps))); }",
    "mat4x3": "bool isOk (mat4x3 a, mat4x3 b, float eps) { vec3 diff = max(max(abs(a[0]-b[0]), abs(a[1]-b[1])), max(abs(a[2]-b[2]), abs(a[3]-b[3]))); return all(lessThanEqual(diff, vec3(eps))); }",
    "mat4": "bool isOk (mat4 a, mat4 b, float eps) { vec4 diff = max(max(abs(a[0]-b[0]), abs(a[1]-b[1])), max(abs(a[2]-b[2]), abs(a[3]-b[3]))); return all(lessThanEqual(diff, vec4(eps))); }",
}

_NON_FLOAT = ("bool", "bvec2", "bvec3", "bvec4", "int", "ivec2", "ivec3",
              "ivec4", "uint", "uvec2", "uvec3", "uvec4")


def gen_compare_functions(outputs, use_float_types):
    """gluShaderLibrary.cpp:1912 — only the isOk() a case's outputs need."""
    kinds = {v.type for v in outputs}
    out = []
    for t in _NON_FLOAT:
        if t in kinds:
            out.append(_CMP_FLOAT_ARG[t] if use_float_types else f"bool isOk ({t} a, {t} b) {{ return (a == b); }}")
    for t, fn in _CMP_FLOAT.items():
        if t in kinds:
            out.append(fn)
    return "".join(s + "\n" for s in out)


def gen_compare_op(dst, outputs, non_float_prefix):
    """gluShaderLibrary's genCompareOp."""
    lines, first = [], True
    for v in outputs:
        prefix = "bool RES = " if first else "RES = RES && "
        if TYPES[v.type][0] == "float":
            lines.append(f"{prefix}isOk({v.name}, ref_{v.name}, 0.05);\n")
        else:
            lines.append(f"{prefix}isOk({non_float_prefix}{v.name}, ref_{v.name});\n")
        first = False
    if first:
        return f"{dst} = vec4(1.0);\n"
    return "".join(lines) + f"{dst} = vec4(RES, RES, RES, 1.0);\n"


def gen_uniform_declarations(uniforms):
    return "".join(f"uniform {v.type} {v.name};\n" for v in uniforms if "." not in v.name)


def gen_vertex_shader(c):
    """The vertex shader dEQP writes for a fragment-only ('both') sub-case."""
    s = ["#version 100\n", "precision highp float;\n", "precision highp int;\n", "\n",
         "attribute highp vec4 dEQP_Position;\n"]
    for v in c.inputs:
        ft = TYPES[v.type][1]
        s.append(f"attribute {ft} a_{v.name};\n")
        s.append(f"varying {ft} {v.name if TYPES[v.type][0] == 'float' else 'v_' + v.name};\n")
    s.append("\nvoid main()\n{\n    gl_Position = dEQP_Position;\n")
    for v in c.inputs:
        name = v.name if TYPES[v.type][0] == "float" else f"v_{v.name}"
        s.append(f"    {name} = a_{v.name};\n")
    s.append("}\n")
    return "".join(s)


def gen_fragment_shader(c):
    """The fragment shader dEQP writes for a vertex-only ('both') sub-case."""
    s = ["#version 100\n", "precision mediump float;\n", "precision mediump int;\n", "\n",
         gen_compare_functions(c.outputs, True), "\n"]
    for v in c.outputs:
        ft = TYPES[v.type][1]
        s.append(f"varying {ft} {v.name if TYPES[v.type][0] == 'float' else 'v_' + v.name};\n")
        s.append(f"uniform {v.type} ref_{v.name};\n")
    s.append("\nvoid main()\n{\n    ")
    s.append(gen_compare_op("gl_FragColor", c.outputs, "v_"))
    s.append("}\n")
    return "".join(s)


def specialize_vertex_only(c):
    decl = ["attribute highp vec4 dEQP_Position;\n"]
    setup, output = [], ["gl_Position = dEQP_Position;\n"]
    for v in c.inputs:
        kind, ft = TYPES[v.type]
        if kind == "float":
            decl.append(f"attribute {ft} {v.name};\n")
        else:
            decl.append(f"attribute {ft} a_{v.name};\n")
            setup.append(f"{v.type} {v.name} = {v.type}(a_{v.name});\n")
    for v in c.outputs:
        kind, ft = TYPES[v.type]
        if kind == "float":
            decl.append(f"varying {ft} {v.name};\n")
        else:
            decl.append(f"varying {ft} v_{v.name};\n")
            decl.append(f"{v.type} {v.name};\n")
            output.append(f"v_{v.name} = {ft}({v.name});\n")
    return specialize(c.both, {
        "DECLARATIONS": "".join(decl),
        "SETUP": "".join(setup),
        "OUTPUT": "".join(output),
        "POSITION_FRAG_COLOR": "gl_Position",
    })


def specialize_fragment_only(c):
    decl, setup = [gen_compare_functions(c.outputs, False)], []
    for v in c.inputs:
        kind, ft = TYPES[v.type]
        if kind == "float":
            decl.append(f"varying {ft} {v.name};\n")
        else:
            decl.append(f"varying {ft} v_{v.name};\n")
            off = " * 1.0025" if v.type in ("int", "ivec2", "ivec3", "ivec4") else ""
            setup.append(f"{v.type} {v.name} = {v.type}(v_{v.name}{off});\n")
    for v in c.outputs:
        decl.append(f"uniform {v.type} ref_{v.name};\n")
        decl.append(f"{v.type} {v.name};\n")
    return specialize(c.both, {
        "DECLARATIONS": "".join(decl),
        "SETUP": "".join(setup),
        "OUTPUT": gen_compare_op("gl_FragColor", c.outputs, ""),
        "POSITION_FRAG_COLOR": "gl_FragColor",
    })


def specialize_complete(c):
    """A case with its own vertex and fragment text."""
    vdecl, vsetup = ["attribute highp vec4 dEQP_Position;\n"], []
    for v in c.inputs:
        kind, ft = TYPES[v.type]
        if kind == "float":
            vdecl.append(f"attribute {v.type} {v.name};\n")
        else:
            vdecl.append(f"attribute {ft} a_{v.name};\n")
            vsetup.append(f"{v.type} {v.name} = {v.type}(a_{v.name});\n")
    vdecl.append(gen_uniform_declarations(c.uniforms))
    fdecl = [gen_compare_functions(c.outputs, False)]
    for v in c.outputs:
        fdecl.append(f"uniform {v.type} ref_{v.name};\n")
        fdecl.append(f"{v.type} {v.name};\n")
    fdecl.append(gen_uniform_declarations(c.uniforms))
    vert = specialize(c.vertex, {
        "VERTEX_DECLARATIONS": "".join(vdecl),
        "VERTEX_SETUP": "".join(vsetup),
        "VERTEX_OUTPUT": "gl_Position = dEQP_Position;\n",
    })
    frag = specialize(c.fragment, {
        "FRAGMENT_DECLARATIONS": "".join(fdecl),
        "FRAGMENT_OUTPUT": gen_compare_op("gl_FragColor", c.outputs, ""),
        "FRAG_COLOR": "gl_FragColor",
    })
    return vert, frag


def programs_of(c):
    """The (sub-case name, vertex text, fragment text) dEQP builds for a case."""
    if c.unsupported:
        return []
    if c.both is not None:
        return [
            (c.name + "_vertex", specialize_vertex_only(c), gen_fragment_shader(c)),
            (c.name + "_fragment", gen_vertex_shader(c), specialize_fragment_only(c)),
        ]
    if c.vertex is None or c.fragment is None:
        return []
    vert, frag = specialize_complete(c)
    return [(c.name, vert, frag)]


# ---------------------------------------------------------------------------
# The pipeline under test.
# ---------------------------------------------------------------------------

class Pipeline:
    """One `yakogl-tools shader-check` process, spoken to in JSON lines."""

    def __init__(self, tools, extra):
        self.p = subprocess.Popen([str(tools), "shader-check"] + extra,
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  text=True, bufsize=1)

    def ask(self, req):
        self.p.stdin.write(json.dumps(req) + "\n")
        self.p.stdin.flush()
        line = self.p.stdout.readline()
        if not line:
            raise SystemExit("es100_oracle: shader-check exited")
        r = json.loads(line)
        if "ok" not in r:
            # A broken service, not a broken shader: never scored as a verdict.
            raise SystemExit(f"es100_oracle: the pipeline is broken, not the shader: {r.get('error')}")
        return r

    def verdict(self, vert, frag):
        """'compile_fail' | 'link_fail' | 'pass', and the first line of why."""
        for stage, src in (("vert", vert), ("frag", frag)):
            r = self.ask({"op": "compile", "stage": stage, "source": src})
            if not r["ok"]:
                return "compile_fail", f"{stage}: {first_line(r.get('error'))}"
        r = self.ask({"op": "link", "vert": vert, "frag": frag})
        if not r["ok"]:
            return "link_fail", first_line(r.get("error"))
        return "pass", ""

    def close(self):
        self.p.stdin.close()
        self.p.wait()


def first_line(text):
    for line in (text or "").splitlines():
        if line.strip():
            return line.strip()
    return ""


# What our verdict must be for each `expect`.
WANT = {
    "pass": "pass",
    "build_successful": "pass",
    "validation_fail": "pass",   # the program builds; the draw is what fails
    "compile_fail": "compile_fail",
    "link_fail": "link_fail",
}


def judge(expect, got):
    if expect == "compile_link_fail":
        return got in ("compile_fail", "link_fail")
    return got == WANT[expect]


def run_file(path, tools, extra, show):
    cases = Parser(tokenize(path.read_text()), path.name).parse()
    pipe = Pipeline(tools, extra)
    total = matched = skipped = 0
    mismatches = []
    try:
        for c in cases:
            progs = [] if c.version != 100 else programs_of(c)
            if not progs:
                skipped += 1
                continue
            for name, vert, frag in progs:
                total += 1
                got, why = pipe.verdict(vert, frag)
                if judge(c.expect, got):
                    matched += 1
                else:
                    mismatches.append({"case": name, "expect": c.expect, "got": got, "why": why})
    finally:
        pipe.close()
    return {"total": total, "matched": matched, "skipped": skipped,
            "mismatches": mismatches[:show] if show else [],
            "mismatch_count": len(mismatches)}


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("files", nargs="*", help="test file base names (default: all)")
    ap.add_argument("--cts", type=Path, default=DEFAULT_CTS)
    ap.add_argument("--tools", type=Path, default=None)
    ap.add_argument("--legacy", action="store_true",
                    help="measure the pipeline as it was before the ES front end")
    ap.add_argument("--codegen", action="store_true", help="run pco_shc too")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 4)
    ap.add_argument("--show-mismatches", type=int, default=5, metavar="N")
    ap.add_argument("--json", type=Path, default=None)
    a = ap.parse_args()

    data = a.cts / "data/gles2/shaders"
    if not data.is_dir():
        raise SystemExit(f"es100_oracle: no dEQP shader data at {data} (--cts DIR or $VK_GL_CTS)")
    paths = sorted(data.glob("*.test"))
    if a.files:
        want = {f.removesuffix(".test") for f in a.files}
        paths = [p for p in paths if p.stem in want]
        if not paths:
            raise SystemExit(f"es100_oracle: no such test file(s): {' '.join(sorted(want))}")

    tools = a.tools
    if tools is None:
        subprocess.run(["cargo", "build", "-q", "-p", "yakogl-tools"], cwd=REPO, check=True)
        target = os.environ.get("CARGO_TARGET_DIR", "target")
        tools = REPO / target / "debug" / "yakogl-tools"
    extra = (["--legacy"] if a.legacy else []) + (["--codegen"] if a.codegen else [])

    results, work, lock = {}, queue.Queue(), threading.Lock()  # noqa: E501
    for p in paths:
        work.put(p)

    def worker():
        while True:
            try:
                p = work.get_nowait()
            except queue.Empty:
                return
            try:
                r = run_file(p, tools, extra, a.show_mismatches)
            except BaseException as e:   # a thread that dies quietly would
                r = {"total": 0, "matched": 0, "skipped": 0,   # silently drop
                     "mismatches": [], "mismatch_count": 0,    # a whole file
                     "broken": f"{type(e).__name__}: {e}"}
            with lock:
                results[p.stem] = r

    threads = [threading.Thread(target=worker) for _ in range(max(1, min(a.jobs, len(paths))))]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    total = matched = skipped = 0
    print(f"{'file':<36} {'cases':>6} {'met':>6} {'missed':>7} {'skipped':>8}")
    for stem in sorted(results):
        r = results[stem]
        total += r["total"]
        matched += r["matched"]
        skipped += r["skipped"]
        broken = r.get("broken")
        print(f"{stem:<36} {r['total']:>6} {r['matched']:>6} {r['mismatch_count']:>7} {r['skipped']:>8}"
              + (f"   !! {broken}" if broken else ""))
    pct = 100.0 * matched / total if total else 0.0
    mode = "legacy" if a.legacy else "current"
    print(f"{'TOTAL (' + mode + ')':<36} {total:>6} {matched:>6} {total - matched:>7} {skipped:>8}"
          f"   {pct:.1f}% of expectations met")

    if a.show_mismatches:
        for stem in sorted(results):
            r = results[stem]
            if not r["mismatches"]:
                continue
            print(f"\n--- {stem}: {r['mismatch_count']} mismatches, first {len(r['mismatches'])} ---")
            for m in r["mismatches"]:
                why = f" ({m['why']})" if m["why"] else ""
                print(f"  {m['case']}: expected {m['expect']}, got {m['got']}{why}")

    if a.json:
        a.json.write_text(json.dumps({"total": total, "matched": matched,
                                      "skipped": skipped, "legacy": a.legacy,
                                      "files": results}, indent=1) + "\n")


if __name__ == "__main__":
    main()
