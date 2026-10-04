#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Generate the host harness's entry-point interposer.

usage: gen_wrap.py <library.a> <include dir> <out dir>

For every gl*/egl* function the library exports (nm), this writes a C wrapper
`hhw_<name>` with the prototype from the library's own Khronos headers into
<out>/wrap_gen.c, and <out>/redefine.txt for `objcopy --redefine-syms`. The
harness programs' objects are rewritten with that file, so every call a
harness program makes goes through the wrapper while the library's internal
calls stay untouched (-Wl,--wrap would also catch cross-object references
inside the library itself, which differ between the C and the Rust build).

Each wrapper counts the call (per-program entry-point coverage) and logs what
the application gets back: return values (EGL/GL handles as stable ids),
values written through output pointers (the number of values written is
measured by probing the call against two sentinel buffers first, so an
over-long write is visible too), strings, and hashes of pixel/mapped data.
Names without a prototype in the headers get a counting-only assembly
trampoline, so nothing exported is silently left out.
"""
import re
import subprocess
import sys

lib, inc, out = sys.argv[1:4]

nm = subprocess.run(["nm", "-g", "--defined-only", lib], capture_output=True, text=True).stdout
names = sorted({l.split()[2] for l in nm.splitlines()
                if len(l.split()) == 3 and l.split()[1] == "T" and re.match(r"^(gl|egl)[A-Z]", l.split()[2])})

protos = {}
for h in ["GLES3/gl3.h", "GLES2/gl2.h", "GLES2/gl2ext.h", "EGL/egl.h", "EGL/eglext.h"]:
    try:
        src = open(f"{inc}/{h}").read()
    except FileNotFoundError:
        continue
    for m in re.finditer(r"(?:GL_APICALL|EGLAPI)\s+([^;{}()]*?)\s*\b(?:GL_APIENTRY|EGLAPIENTRY)\s+(\w+)\s*\(([^)]*)\)\s*;", src):
        ret, name, params = m.group(1).strip(), m.group(2), m.group(3).strip()
        protos.setdefault(name, (ret, params))

TYPEWORDS = {"const", "unsigned", "signed", "struct", "void", "int", "char", "float", "double", "long", "short"}


def split_params(p):
    if p in ("", "void"):
        return []
    res = []
    for i, a in enumerate(x.strip() for x in p.split(",")):
        m = re.match(r"^(.*?)(\w+)\s*(\[\s*\d*\s*\])?$", a)
        base, ident, arr = m.group(1).strip(), m.group(2), m.group(3)
        if not base or ident in TYPEWORDS or re.match(r"^(GL|EGL|khronos_)\w*$", ident) and not base.endswith("*") and base in ("const", ""):
            # no parameter name in the prototype
            base, ident = a, f"a{i}"
        if arr:
            base += " *"
        typ = re.sub(r"\s+", " ", base).strip()
        res.append((typ, ident))
    return res


def is_ptr(t):
    return "*" in t


HANDLE_TYPES = {"EGLDisplay", "EGLSurface", "EGLContext", "EGLConfig", "EGLImage", "EGLImageKHR", "EGLSync",
                "EGLSyncKHR", "GLsync", "EGLClientBuffer", "GLeglImageOES"}
SCALAR_FMT = {"GLenum": "0x%x", "GLbitfield": "0x%x", "EGLenum": "0x%x"}

# sentinel-probed getters: name -> (output parameter name, element size, print kind)
PROBE = {
    "glGetBooleanv": ("data", 1, "u8"), "glGetFloatv": ("data", 4, "x32"), "glGetIntegerv": ("data", 4, "i32"),
    "glGetInteger64v": ("data", 8, "i64"), "glGetIntegeri_v": ("data", 4, "i32"), "glGetInteger64i_v": ("data", 8, "i64"),
    "glGetUniformfv": ("params", 4, "x32"), "glGetUniformiv": ("params", 4, "i32"), "glGetUniformuiv": ("params", 4, "x32"),
    "glGetVertexAttribfv": ("params", 4, "x32"), "glGetVertexAttribiv": ("params", 4, "i32"),
    "glGetVertexAttribIiv": ("params", 4, "i32"), "glGetVertexAttribIuiv": ("params", 4, "x32"),
    "glGetTexParameterfv": ("params", 4, "x32"), "glGetTexParameteriv": ("params", 4, "i32"),
    "glGetBufferParameteriv": ("params", 4, "i32"), "glGetBufferParameteri64v": ("params", 8, "i64"),
    "glGetProgramiv": ("params", 4, "i32"), "glGetShaderiv": ("params", 4, "i32"),
    "glGetRenderbufferParameteriv": ("params", 4, "i32"),
    "glGetFramebufferAttachmentParameteriv": ("params", 4, "i32"),
    "glGetSamplerParameteriv": ("params", 4, "i32"), "glGetSamplerParameterfv": ("params", 4, "x32"),
    "glGetQueryiv": ("params", 4, "i32"), "glGetActiveUniformBlockiv": ("params", 4, "i32"),
    "glGetActiveUniformsiv": ("params", 4, "i32"), "glGetInternalformativ": ("params", 4, "i32"),
    "glGetSynciv": ("values", 4, "i32"),
    "glGetVertexAttribPointerv": ("pointer", 8, "ptr"), "glGetBufferPointerv": ("params", 8, "ptr"),
    "glGetBufferPointervOES": ("params", 8, "ptr"),
    "eglQuerySurface": ("value", 4, "i32"), "eglQueryContext": ("value", 4, "i32"),
    "eglGetConfigAttrib": ("value", 4, "i32"), "eglGetSyncAttrib": ("value", 8, "i64"),
    "eglGetSyncAttribKHR": ("value", 4, "i32"), "eglQueryDisplayAttribEXT": ("value", 8, "i64"),
    "eglQueryAPI": None,
}
# outputs printed after the call: name -> list of (parameter, kind[, count expression])
POST = {
    "glGetShaderPrecisionFormat": [("range", "i32", "2"), ("precision", "i32", "1")],
    "glGetShaderInfoLog": [("length", "i32", "1"), ("infoLog", "str", "bufSize")],
    "glGetProgramInfoLog": [("length", "i32", "1"), ("infoLog", "str", "bufSize")],
    "glGetShaderSource": [("length", "i32", "1"), ("source", "str", "bufSize")],
    "glGetActiveUniform": [("length", "i32", "1"), ("size", "i32", "1"), ("type", "x32", "1"), ("name", "str", "bufSize")],
    "glGetActiveAttrib": [("length", "i32", "1"), ("size", "i32", "1"), ("type", "x32", "1"), ("name", "str", "bufSize")],
    "glGetTransformFeedbackVarying": [("length", "i32", "1"), ("size", "i32", "1"), ("type", "x32", "1"), ("name", "str", "bufSize")],
    "glGetActiveUniformBlockName": [("length", "i32", "1"), ("uniformBlockName", "str", "bufSize")],
    "glGetAttachedShaders": [("count", "i32", "1"), ("shaders", "i32", "(count ? (*count > maxCount ? maxCount : *count) : 0)")],
    "glGetUniformIndices": [("uniformIndices", "i32", "(uniformCount > 64 ? 64 : uniformCount)")],
    "glGetProgramBinary": [("length", "i32", "1"), ("binaryFormat", "x32", "1"), ("binary", "hash", "(length ? *length : 0)")],
    "glGenBuffers": [("buffers", "i32", "HH_CAP(n)")], "glGenTextures": [("textures", "i32", "HH_CAP(n)")],
    "glGenFramebuffers": [("framebuffers", "i32", "HH_CAP(n)")],
    "glGenRenderbuffers": [("renderbuffers", "i32", "HH_CAP(n)")],
    "glGenVertexArrays": [("arrays", "i32", "HH_CAP(n)")], "glGenVertexArraysOES": [("arrays", "i32", "HH_CAP(n)")],
    "glGenSamplers": [("samplers", "i32", "HH_CAP(count)")], "glGenQueries": [("ids", "i32", "HH_CAP(n)")],
    "glGenTransformFeedbacks": [("ids", "i32", "HH_CAP(n)")],
    "eglInitialize": [("major", "i32", "1"), ("minor", "i32", "1")],
    "eglGetConfigs": [("num_config", "i32", "1"), ("configs", "hid", "(num_config ? HH_MIN(*num_config, config_size) : 0)")],
    "eglChooseConfig": [("num_config", "i32", "1"), ("configs", "hid", "(num_config ? HH_MIN(*num_config, config_size) : 0)")],
}
QUIET_ZERO = {"glGetError": "0", "eglGetError": "0x3000"}

lines = ['/* generated by tools/hostharness/gen_wrap.py - do not edit */',
         '#define GL_GLEXT_PROTOTYPES 1', '#define EGL_EGLEXT_PROTOTYPES 1',
         '#include <GLES3/gl3.h>', '#include <GLES2/gl2ext.h>', '#include <EGL/egl.h>', '#include <EGL/eglext.h>',
         '#include "hh_wrap.h"', '']
asm = []
redef = []
for idx, name in enumerate(names):
    redef.append(f"{name} hhw_{name}")
    if name not in protos:
        asm.append(name)
        continue
    ret, params = protos[name]
    ps = split_params(params)
    decl = ", ".join(f"{t} {n}" for t, n in ps) or "void"
    call = ", ".join(n for _, n in ps)
    isvoid = ret == "void"
    body = [f"\thh_call({idx});"]
    probe = PROBE.get(name)
    argfmt = []
    argval = []
    for t, n in ps:
        if not is_ptr(t) and t not in HANDLE_TYPES and t not in ("GLfloat", "GLclampf", "EGLNativeWindowType",
                                                                  "EGLNativePixmapType", "EGLNativeDisplayType"):
            argfmt.append(SCALAR_FMT.get(t, "%lld"))
            argval.append(f"(long long){n}" if SCALAR_FMT.get(t) is None else f"(unsigned){n}")
        elif t in HANDLE_TYPES:
            argfmt.append("h%u")
            argval.append(f"hh_hid((void *)(uintptr_t){n})")
        elif t in ("const GLchar *", "const char *"):
            argfmt.append("%s")
            argval.append(f"hh_cs({n})")
    argstr = ",".join(argfmt)
    argv = "".join(", " + v for v in argval)
    if probe:
        pn, esz, kind = probe
        probe_call = ", ".join(("(void *)hh_probe_buf" if n == pn else n) for _, n in ps)
        body.append(f"\tsize_t hh_n = 0;")
        body.append(f"\tif ({pn}) {{ HH_PROBE(hh_n, {esz}, {name}({probe_call})); }}")
    body.append(f"\thh_cur = {idx};")
    if isvoid:
        body.append(f"\t{name}({call});")
    else:
        body.append(f"\t{ret} hh_r = {name}({call});")
    body.append("\thh_cur = -1;")
    # the log line
    rline = None
    if not isvoid:
        if ret in HANDLE_TYPES:
            rline = ('" -> h%u"', "hh_hid((void *)(uintptr_t)hh_r)")
        elif ret in ("const GLubyte *", "const char *"):
            rline = None
        elif is_ptr(ret) or ret == "__eglMustCastToProperFunctionPointerType":
            rline = ('" -> %s"', 'hh_r ? "ptr" : "NULL"')
        elif ret in ("GLenum", "EGLenum"):
            rline = ('" -> 0x%x"', "(unsigned)hh_r")
        else:
            rline = ('" -> %lld"', "(long long)hh_r")
    pre = f'hh_printf("~{name}({argstr})"{argv});'
    if name in QUIET_ZERO:
        body.append(f"\tif (hh_r != {QUIET_ZERO[name]}) {{ hh_printf(\"~{name} -> 0x%x\\n\", (unsigned)hh_r); }}")
    else:
        body.append("\t" + pre)
        if not isvoid:
            if ret in ("const GLubyte *", "const char *"):
                body.append('\thh_printf(" -> "); hh_str((const char *)hh_r, 4096);')
            elif rline:
                body.append(f"\thh_printf({rline[0]}, {rline[1]});")
        if probe:
            pn, esz, kind = probe
            body.append(f'\thh_printf(" {pn}=");')
            body.append(f'\thh_vals("{kind}", {pn}, hh_n);')
        for (pn, kind, cnt) in POST.get(name, []):
            body.append(f'\thh_printf(" {pn}=");')
            if kind == "str":
                body.append(f"\tif ({pn} && ({cnt}) > 0) hh_str((const char *){pn}, (size_t)({cnt})); else hh_printf(\"-\");")
            elif kind == "hash":
                body.append(f"\tif ({pn}) hh_printf(\"#%016llx\", (unsigned long long)hh_hash({pn}, (size_t)({cnt}))); else hh_printf(\"-\");")
            elif kind == "hid":
                body.append(f"\tif ({pn}) {{ for (long hh_i = 0; hh_i < (long)({cnt}); hh_i++) hh_printf(\"h%u,\", hh_hid((void *){pn}[hh_i])); }} else hh_printf(\"-\");")
            else:
                body.append(f"\tif ({pn}) hh_vals(\"{kind}\", {pn}, (size_t)({cnt})); else hh_printf(\"-\");")
        # special data outputs
        if name == "glReadPixels":
            body.append("\thh_readpixels(width, height, format, type, pixels);")
        if name == "glMapBufferRange":
            body.append("\tif (hh_r && (access & GL_MAP_READ_BIT) && length > 0 && length < (1 << 26)) hh_printf(\" data=#%016llx\", (unsigned long long)hh_hash(hh_r, (size_t)length));")
        if name == "eglGetProcAddress":
            body.append("\thh_r = (__eglMustCastToProperFunctionPointerType)hh_map_proc((void *)hh_r);")
            body.append('\thh_printf(" = %s", hh_r ? hh_proc_name((void *)hh_r) : "-");')
        if name == "glPixelStorei":
            body.append("\thh_pixelstore(pname, param);")
        body.append('\thh_printf("\\n");')
    if not isvoid:
        body.append("\treturn hh_r;")
    body = [b for b in body if b]
    lines.append(f"{ret} hhw_{name}({decl})")
    lines.append("{")
    lines += body
    lines.append("}")
    lines.append("")

# eglGetProcAddress hands out the library's function; the application
# compares it with (and calls) what it links against, i.e. the wrapper
lines.append("static void *const hh_real[] = {")
lines += [f"\t(void *){n}," for n in names if n not in asm]
lines.append("};")
lines.append("static const char *const hh_pnames[] = {")
lines += [f'\t"{n}",' for n in names if n not in asm]
lines.append("};")
lines.append("static void *const hh_wrapped[] = {")
lines += [f"\t(void *)hhw_{n}," for n in names if n not in asm]
lines.append("};")
lines.append("void *hh_map_proc(void *p)")
lines.append("{")
lines.append(f"\tfor (unsigned i = 0; i < {len(names) - len(asm)}; i++)")
lines.append("\t\tif (p && p == hh_real[i])")
lines.append("\t\t\treturn hh_wrapped[i];")
lines.append("\treturn p;")
lines.append("}")
lines.append("const char *hh_proc_name(void *p)")
lines.append("{")
lines.append(f"\tfor (unsigned i = 0; i < {len(names) - len(asm)}; i++)")
lines.append("\t\tif (p == hh_wrapped[i])")
lines.append("\t\t\treturn hh_pnames[i];")
lines.append('\treturn "(not an entry point)";')
lines.append("}")
lines.append(f"const unsigned hh_nnames = {len(names)};")
lines.append("const char *const hh_names[] = {")
lines += [f'\t"{n}",' for n in names]
lines.append("};")
open(f"{out}/wrap_gen.c", "w").write("\n".join(lines) + "\n")

# counting-only trampolines for the names without a prototype (all arguments,
# including stack ones, pass through untouched)
s = ["/* generated by tools/hostharness/gen_wrap.py - do not edit */", ".text"]
for name in asm:
    idx = names.index(name)
    s += [f".globl hhw_{name}", f".type hhw_{name}, @function", f"hhw_{name}:",
          f"\tincq hh_calls+{8 * idx}(%rip)", f"\tjmp {name}"]
s.append('.section .note.GNU-stack,"",@progbits')
open(f"{out}/wrap_asm.S", "w").write("\n".join(s) + "\n")
open(f"{out}/redefine.txt", "w").write("\n".join(redef) + "\n")
open(f"{out}/exported.txt", "w").write("\n".join(names) + "\n")
sys.stderr.write(f"gen_wrap: {len(names)} entry points, {len(names) - len(asm)} with prototypes, "
                 f"{len(asm)} counted only: {' '.join(asm)}\n")
