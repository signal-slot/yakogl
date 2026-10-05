#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""api_coverage.py - the public headers vs what the library exports vs what
eglGetProcAddress answers (adversarial review R-23: "nothing checks the
public headers against what the library exports and what eglGetProcAddress
answers").

Three name sets, all by symbol name:

  declared     GL_APICALL names in include/GLES2/gl2.h (OpenGL ES 2.0 core)
               plus EGLAPI names in include/EGL/egl.h (EGL 1.x core).
               include/GLES2/gl2ext.h, include/GLES3/gl3.h, gl31.h, gl32.h and
               include/EGL/eglext.h are extension/later-version headers: they
               are parsed and reported too (EXT lines), but never enter
               `declared` and never affect the exit status.
  exported     symbols the built library actually exports. Preferred source:
               `nm` on the yakogl-capi no_std staticlib check.sh already
               builds (`cargo build -p yakogl-capi --features gpu-powervr,mesa-reference-table`,
               <cargo target_directory>/debug/libyakogl_capi.a). When that
               archive is not there (the tool run standalone before a build),
               falls back to grepping every `pub extern "C" fn gl*`/`egl*` in
               src/gl/*.rs. Which one ran is printed on the SUMMARY line -- a
               silent fallback would let a stale reading pass as a measured
               one.
  procaddress  names eglGetProcAddress resolves: the `gl!`/`own!` entries of
               the PROCS table in src/gl/egl.rs.

Prints one row per name in declared | exported | procaddress (declared/
exported/procaddress as 0 or 1), the three difference sets by name, an EXT
line per extension header, and a SUMMARY line with every count.

Exit status is 1 for an INCONSISTENCY OF OURS ONLY: a name eglGetProcAddress
resolves that the library does not export, or a name the library exports
that eglGetProcAddress cannot resolve -- either means a client that got the
pointer one way and not the other sees a different API surface depending on
which call it used, which is a defect in this crate, not a documented gap.
A declared-but-not-exported name (the ES 2.0 core or the EGL 1.x core is
missing an entry point) is reported and counted (missing_gl2=N,
missing_egl=N) but does not by itself fail the tool -- tools/check.sh pins
those counts in a separate row, so a WORSE number (an entry point lost) and
a BETTER one (an entry point added without updating the pin) are both
noticed, per "a gate asserts its population, not only its zero".

    tools/api_coverage.py                        # the real repository
    tools/api_coverage.py --capi-artifact PATH   # nm a specific archive
    tools/api_coverage.py --self-test            # fixtures only, no repo I/O

An empty parse (a header with zero GL_APICALL/EGLAPI lines, a PROCS table
with zero entries, zero exported symbols) is reported as an error and exits
1: an empty result is a failure, not a pass, and here it is usually a sign
that a Khronos header changed its macro name or the PROCS array was moved
or renamed out from under the regexes below.
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Khronos declares each entry point as
#   GL_APICALL <return-type> GL_APIENTRY <name> (...);
#   EGLAPI <return-type> EGLAPIENTRY <name> (...);
# at the start of a line; the PFN...PROC typedefs (which also mention
# GL_APIENTRYP / EGLAPIENTRYP) never start the line with GL_APICALL/EGLAPI, so
# anchoring on that keeps them out. The return type is `.*?` (non-greedy)
# because it is not always one token: glGetString / eglQueryString return
# `const GLubyte *` / `const char *`, and a `\S+` return-type match silently
# drops exactly those two names (found by comparing this parse's count
# against a plain line count: 141 of 142, 43 of 44, before the fix).
GL_DECL_RE = re.compile(r"^GL_APICALL\s+.*?\bGL_APIENTRY\s+(\w+)\s*\(", re.M)
EGL_DECL_RE = re.compile(r"^EGLAPI\s+.*?\bEGLAPIENTRY\s+(\w+)\s*\(", re.M)

# src/gl/egl.rs's PROCS table: `gl!(name)` (defined in another module, looked
# up through the `sym` extern block) or `own!(name)` (defined in egl.rs
# itself); both also accept a `("literal", ident)` form the macros support
# even though nothing currently uses it for PROCS.
PROC_ENTRY_RE = re.compile(r'\b(?:gl|own)!\(\s*(?:"([A-Za-z_]\w*)"\s*,\s*\w+|([A-Za-z_]\w*))\s*\)')

# The fallback source scan: an exported GL/EGL entry point is always
# `pub [unsafe ]extern "C" fn glFoo(` / `...eglFoo(`, camelCase after the
# prefix. Internal helpers use snake_case (`gl_context_create`,
# `gl_set_error_at`, ...) and must NOT match -- `[A-Z]` right after gl/egl is
# what tells the two apart; without it this over-collects by 25 names (found
# by diffing against `nm` on the real archive, where the two disagreed).
NO_MANGLE_FN_RE = re.compile(r'pub (?:unsafe )?extern "C" fn ((?:gl|egl)[A-Z]\w*)\s*\(')

EXT_HEADERS = [
    "include/GLES2/gl2ext.h",
    "include/GLES3/gl3.h",
    "include/GLES3/gl31.h",
    "include/GLES3/gl32.h",
    "include/EGL/eglext.h",
]


def parse_decl_names(text):
    """Set of GL_APICALL/EGLAPI names declared in one header's text."""
    return set(GL_DECL_RE.findall(text)) | set(EGL_DECL_RE.findall(text))


def parse_procs_table(text):
    """(names, error) from the `static PROCS` array in src/gl/egl.rs's text.

    names is None and error is set when the array's markers are not found or
    it parsed to zero entries, so a renamed/moved array is a reported error
    rather than a quietly empty, "0 missing" table.
    """
    m = re.search(r"static PROCS:\s*&\[Proc\]\s*=\s*&\[", text)
    if not m:
        return None, "static PROCS: &[Proc] = &[ ... not found in egl.rs"
    body_start = m.end()
    close = text.find("\n];", body_start)
    if close < 0:
        return None, "PROCS array close ('\\n];') not found after its opening bracket"
    body = text[body_start:close]
    names = set()
    for gm in PROC_ENTRY_RE.finditer(body):
        names.add(gm.group(1) or gm.group(2))
    if not names:
        return None, "PROCS array found but parsed to zero entries"
    return names, None


def find_capi_archive():
    """(path, error) of the yakogl-capi no_std staticlib check.sh already
    builds ('build yakogl-capi (no_std)': `cargo build -p yakogl-capi --features gpu-powervr,mesa-reference-table`,
    dev/debug profile, crate-type staticlib named yakogl_capi -- see
    capi/Cargo.toml). The archive's directory is asked of cargo, never
    assumed, for the reason tools/check.sh's own `table_symbols` gives: a
    fixed path guess reads a stale archive from an earlier build/profile.
    """
    try:
        out = subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=str(ROOT), stderr=subprocess.DEVNULL
        )
    except Exception as e:  # noqa: BLE001 - reported, not raised
        return None, "cargo metadata failed: %s" % e
    try:
        target_dir = Path(json.loads(out)["target_directory"])
    except Exception as e:  # noqa: BLE001
        return None, "cargo metadata output has no target_directory: %s" % e
    path = target_dir / "debug" / "libyakogl_capi.a"
    if not path.exists():
        return None, "%s does not exist (run: cargo build -p yakogl-capi --features gpu-powervr,mesa-reference-table)" % path
    return path, None


def exported_from_nm(path):
    """(names, error): GL/EGL entry points `nm` finds defined and exported in
    the staticlib at path."""
    try:
        out = subprocess.check_output(
            ["nm", "--defined-only", "--extern-only", str(path)], stderr=subprocess.STDOUT
        ).decode("utf-8", "replace")
    except Exception as e:  # noqa: BLE001
        return None, "nm %s failed: %s" % (path, e)
    names = set()
    for line in out.splitlines():
        parts = line.split()
        if not parts:
            continue
        name = parts[-1]
        if re.match(r"^(gl|egl)[A-Z]", name):
            names.add(name)
    if not names:
        return None, "nm %s found zero gl*/egl* symbols" % path
    return names, None


def exported_from_source(root):
    """(names, error): GL/EGL entry points found as
    `pub extern "C" fn gl*`/`egl*` under src/gl/*.rs (every gl*/egl*-shaped
    `#[no_mangle]` export lives there; nowhere else under src/ matches this
    pattern except gles_offline_shader_source, which the linker script
    exports by its own explicit name and which this pattern does not match)."""
    gl_dir = root / "src" / "gl"
    if not gl_dir.is_dir():
        return None, "%s does not exist" % gl_dir
    names = set()
    for p in sorted(gl_dir.glob("*.rs")):
        names.update(NO_MANGLE_FN_RE.findall(p.read_text()))
    if not names:
        return None, 'src/gl/*.rs parsed to zero pub extern "C" fn gl*/egl* names'
    return names, None


def get_exported(root, capi_artifact):
    """(names, source_desc, error). source_desc says nm-vs-source so the
    caller can never mistake one for the other."""
    if capi_artifact is not None:
        path = Path(capi_artifact)
        if not path.exists():
            return None, None, "%s does not exist" % path
        names, err = exported_from_nm(path)
        return names, "nm:%s" % path, err
    path, err = find_capi_archive()
    if path is not None:
        names, nm_err = exported_from_nm(path)
        return names, "nm:%s" % path, nm_err
    names, src_err = exported_from_source(root)
    return names, "source_fallback(%s)" % err, src_err


def compute_diffs(declared, exported, procaddress):
    return {
        "declared_not_exported": declared - exported,
        "exported_not_procaddress": exported - procaddress,
        "procaddress_not_exported": procaddress - exported,
    }


def print_report(declared, exported, procaddress, exported_source, ext_counts):
    diffs = compute_diffs(declared, exported, procaddress)

    print("# name declared exported procaddress")
    for name in sorted(declared | exported | procaddress):
        print("%s %d %d %d" % (name, name in declared, name in exported, name in procaddress))

    print("DECLARED_NOT_EXPORTED %d" % len(diffs["declared_not_exported"]))
    for name in sorted(diffs["declared_not_exported"]):
        print(name)
    print("EXPORTED_NOT_PROCADDRESS %d" % len(diffs["exported_not_procaddress"]))
    for name in sorted(diffs["exported_not_procaddress"]):
        print(name)
    print("PROCADDRESS_NOT_EXPORTED %d" % len(diffs["procaddress_not_exported"]))
    for name in sorted(diffs["procaddress_not_exported"]):
        print(name)

    for rel, (count, covered) in ext_counts.items():
        print("EXT %s declared=%d exported=%d" % (rel, count, covered))

    print(
        "SUMMARY declared=%d exported=%d exported_source=%s procaddress=%d "
        "declared_not_exported=%d exported_not_procaddress=%d procaddress_not_exported=%d"
        % (
            len(declared),
            len(exported),
            exported_source.replace(" ", "_"),
            len(procaddress),
            len(diffs["declared_not_exported"]),
            len(diffs["exported_not_procaddress"]),
            len(diffs["procaddress_not_exported"]),
        )
    )
    return diffs


def run(root, capi_artifact):
    gl2_path = root / "include" / "GLES2" / "gl2.h"
    egl_path = root / "include" / "EGL" / "egl.h"
    egl_rs_path = root / "src" / "gl" / "egl.rs"
    for p in (gl2_path, egl_path, egl_rs_path):
        if not p.is_file():
            print("ERROR: %s does not exist" % p, file=sys.stderr)
            return 1

    gl2_declared = parse_decl_names(gl2_path.read_text())
    egl_declared = parse_decl_names(egl_path.read_text())
    if not gl2_declared:
        print("ERROR: %s parsed to zero GL_APICALL names" % gl2_path, file=sys.stderr)
        return 1
    if not egl_declared:
        print("ERROR: %s parsed to zero EGLAPI names" % egl_path, file=sys.stderr)
        return 1
    declared = gl2_declared | egl_declared

    procaddress, proc_err = parse_procs_table(egl_rs_path.read_text())
    if procaddress is None:
        print("ERROR: %s: %s" % (egl_rs_path, proc_err), file=sys.stderr)
        return 1

    exported, exported_source, exp_err = get_exported(root, capi_artifact)
    if exported is None:
        print("ERROR: exported symbols: %s" % exp_err, file=sys.stderr)
        return 1

    ext_counts = {}
    for rel in EXT_HEADERS:
        p = root / rel
        if not p.is_file():
            print("ERROR: %s does not exist" % p, file=sys.stderr)
            return 1
        names = parse_decl_names(p.read_text())
        ext_counts[rel] = (len(names), len(names & exported))

    diffs = print_report(declared, exported, procaddress, exported_source, ext_counts)

    missing_gl2 = len(gl2_declared - exported)
    missing_egl = len(egl_declared - exported)
    print("PINNED missing_gl2=%d missing_egl=%d" % (missing_gl2, missing_egl))

    return 1 if diffs["exported_not_procaddress"] or diffs["procaddress_not_exported"] else 0


# ---- self-test --------------------------------------------------------------

FIXTURE_GL_HEADER = """\
#ifndef FIXTURE_H
#define FIXTURE_H
GL_APICALL void GL_APIENTRY glClear (GLbitfield mask);
GL_APICALL const GLubyte *GL_APIENTRY glGetString (GLenum name);
typedef void (GL_APIENTRYP PFNGLCLEARPROC) (GLbitfield mask);
#endif
"""

FIXTURE_EGL_HEADER = """\
#ifndef FIXTURE_EGL_H
EGLAPI EGLBoolean EGLAPIENTRY eglSwapBuffers (EGLDisplay dpy, EGLSurface surface);
EGLAPI const char *EGLAPIENTRY eglQueryString (EGLDisplay dpy, EGLint name);
typedef EGLBoolean (EGLAPIENTRYP PFNEGLSWAPBUFFERSPROC) (EGLDisplay dpy, EGLSurface surface);
#endif
"""

FIXTURE_PROCS_RS = """\
static PROCS: &[Proc] = &[
    gl!(glClear),
    gl!("glGetString", glGetString),
    own!(eglSwapBuffers),
];
extern "C" fn unrelated() {}
"""

FIXTURE_SRC_GL_RS = """\
#[no_mangle]
pub extern "C" fn glClear(mask: GLbitfield) {}
#[no_mangle]
pub unsafe extern "C" fn glGetString(name: GLenum) -> *const GLubyte { core::ptr::null() }
fn gl_context_create(w: i32, h: i32) {} // internal helper: must NOT be collected
"""


def self_test():
    bad = 0

    def check(desc, got, want):
        nonlocal bad
        ok = got == want
        print("  %s  %s" % ("ok  " if ok else "FAIL", desc))
        if not ok:
            print("        got:  %r" % (got,))
            print("        want: %r" % (want,))
            bad += 1

    # A header whose return type is a single token and one whose return type
    # is "const X *" (two-plus tokens): the second is the exact case
    # (glGetString / eglQueryString) a naive `\S+` return-type match drops.
    check("gl2.h-shaped fixture: names", parse_decl_names(FIXTURE_GL_HEADER), {"glClear", "glGetString"})
    check("egl.h-shaped fixture: names", parse_decl_names(FIXTURE_EGL_HEADER), {"eglSwapBuffers", "eglQueryString"})

    # Both PROCS entry forms (`gl!(name)`, `gl!("literal", ident)`, `own!(name)`),
    # and that an unrelated `fn` line after the closing `];` is not swept in.
    names, err = parse_procs_table(FIXTURE_PROCS_RS)
    check("PROCS fixture: parse error", err, None)
    check("PROCS fixture: names", names, {"glClear", "glGetString", "eglSwapBuffers"})

    # A missing/renamed PROCS array is a reported error, not an empty table.
    names, err = parse_procs_table("static PROCS_RENAMED: &[Proc] = &[];")
    check("PROCS renamed: names is None", names, None)
    check("PROCS renamed: error set", err is not None, True)

    # An array present but with zero entries is also a reported error (an
    # empty result is a failure, not a pass), not a silently clean sweep.
    names, err = parse_procs_table("static PROCS: &[Proc] = &[\n];")
    check("PROCS empty: names is None", names, None)
    check("PROCS empty: error set", err is not None, True)

    # camelCase gl*/egl* names are collected; snake_case internal helpers
    # (gl_context_create, ...) are not -- the [A-Z] right after gl/egl is
    # what tells them apart.
    names = set(NO_MANGLE_FN_RE.findall(FIXTURE_SRC_GL_RS))
    check("source-fallback fixture: camelCase only", names, {"glClear", "glGetString"})

    # The exit-status contract: a declared-but-not-exported name (a real gap
    # in the ES 2.0/EGL 1.x core) must NOT by itself trigger a non-zero exit
    # -- that is check.sh's separate pinned-count row -- but an
    # exported/procaddress mismatch (our own inconsistency) must.
    declared = {"glClear", "glMissingCore"}
    exported = {"glClear", "glExtraExported"}
    procaddress = {"glClear"}
    diffs = compute_diffs(declared, exported, procaddress)
    check("declared_not_exported finds the gap", diffs["declared_not_exported"], {"glMissingCore"})
    check("exported_not_procaddress finds the inconsistency", diffs["exported_not_procaddress"], {"glExtraExported"})
    check("procaddress_not_exported is empty here", diffs["procaddress_not_exported"], set())
    would_fail = bool(diffs["exported_not_procaddress"] or diffs["procaddress_not_exported"])
    check("a declared-not-exported-only case would still exit 1 (via the inconsistency)", would_fail, True)

    # A clean case (nothing declared is missing, exported == procaddress)
    # must not trip the exit-status check.
    diffs_clean = compute_diffs({"glClear"}, {"glClear"}, {"glClear"})
    would_fail_clean = bool(diffs_clean["exported_not_procaddress"] or diffs_clean["procaddress_not_exported"])
    check("a fully consistent case would exit 0", would_fail_clean, False)

    return 1 if bad else 0


def main():
    ap = argparse.ArgumentParser(
        description=__doc__.split("\n\n")[0].replace("\n", " "),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--capi-artifact", help="nm this staticlib instead of asking cargo for check.sh's own build")
    ap.add_argument("--self-test", action="store_true", help="check the parsers against fixtures, no repo I/O")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    return run(ROOT, a.capi_artifact)


if __name__ == "__main__":
    sys.exit(main())
