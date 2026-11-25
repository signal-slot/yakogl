#!/usr/bin/env python3
"""Tag each T6 (Mali-G610) vs Verdin iMX95 (Mali-G310) dEQP case with a
candidate G310 cause.

A cause is a CANDIDATE, not a proof: it is chosen from the case name and the
two results only, by the rules below, and it says where to look first. A
board re-run with the constant changed is the proof.

Input (stdin or FILE): one case a line, 'case<TAB>t6_result<TAB>imx95_result'.
Results: Pass Fail NotSupported QualityWarning CompatibilityWarning
InternalError Crash Timeout Missing. Blank lines and '#' lines are copied
through. Output: the same lines with a fourth column 'cause'. --summary adds
a per-cause count table as '# ' lines at the end. A malformed line exits 2
with a message on stderr.

The G610 constants the G310 still carries (docs/notes/mali-g310-vs-g610.md
sections 3 and 5): TIB_BUDGET (a G310 v1-v3 has half the G610's tile buffer,
so targets of 12 bytes a pixel or more can fail), MEASURED_TILER_LEVELS, the
renderer name, and SHADER_PRESENT 0x1 (one shader core).

Rules, in order, first match wins (a case whose two results are both Pass
is 'none', not a difference):
  compiler-refusal  the same non-Pass on both boards (Fail, NotSupported,
                    QualityWarning, CompatibilityWarning, InternalError):
                    the online compiler (issue #11251), not the board
  renderer-name     dEQP-GLES2.info.*: strings that carry the G610 name
  tib-budget        names that imply >= 12 bytes a pixel of tile buffer
  tiler-levels      large framebuffers
  shader-present    timing and performance cases
  hang              Timeout/Crash/Missing on the iMX95 only
  unexplained       anything else
"""
import re
import sys

RESULTS = {"Pass", "Fail", "NotSupported", "QualityWarning",
           "CompatibilityWarning", "InternalError", "Crash", "Timeout",
           "Missing"}
REFUSAL = {"Fail", "NotSupported", "QualityWarning", "CompatibilityWarning",
           "InternalError"}
HANGS = {"Timeout", "Crash", "Missing"}
CAUSES = ["none", "compiler-refusal", "renderer-name", "tib-budget",
          "tiler-levels", "shader-present", "hang", "unexplained"]

# tib-budget (note section 3: the tile buffer is shared by all colour and
# depth/stencil targets of a tile, a G310 v1-v3 has half the G610's). The
# GLES2 tree has no MRT, so the 12+ bytes a pixel come from a wide format or
# from multisampling: 32-bit float colour (RGB32F 12, RGBA32F 16 bytes), a
# multisample case (4 samples of RGBA8 plus depth/stencil is 24 bytes) and
# an fbo case that combines colour with depth and stencil at a float format.
TIB = re.compile(
    r"(multisample|rgba32f|rgb32f|float_?32|(?:^|[._])32f(?:[._]|$)"
    r"|rgba_?float|depth_stencil.*float|float.*depth_stencil)")
# tiler-levels (note section 5: MEASURED_TILER_LEVELS was measured on the
# G610): resize of a framebuffer, and any size token of 2048 or more.
TILER = re.compile(r"(fbo\.render\.resize|(?<!\d)(2048|4096|8192|16384)(?!\d))")
# shader-present (note section 3: one core, SHADER_PRESENT 0x1, against the
# G610's several): only time-dependent cases can notice.
SHADER = re.compile(r"(performance|timing|benchmark|(?:^|[._])perf(?:[._]|$))")


def classify(case, t6, imx):
    low = case.lower()
    if t6 == "Pass" and imx == "Pass":
        return "none"
    if t6 == imx and t6 in REFUSAL:
        return "compiler-refusal"
    if low.startswith("deqp-gles2.info."):
        return "renderer-name"
    if TIB.search(low):
        return "tib-budget"
    if TILER.search(low):
        return "tiler-levels"
    if SHADER.search(low):
        return "shader-present"
    if imx in HANGS and t6 != imx:
        return "hang"
    return "unexplained"


def main(argv):
    summary = False
    path = None
    for a in argv:
        if a in ("-h", "--help"):
            print(__doc__)
            return 0
        if a == "--summary":
            summary = True
        elif a.startswith("-") and a != "-":
            sys.stderr.write("imx95-g310-cause: unknown option %s\n" % a)
            return 2
        elif path is None:
            path = a
        else:
            sys.stderr.write("imx95-g310-cause: more than one input file\n")
            return 2
    try:
        if path in (None, "-"):
            text = sys.stdin.read()
        else:
            with open(path, encoding="utf-8") as fh:
                text = fh.read()
    except OSError as e:
        sys.stderr.write("imx95-g310-cause: %s\n" % e)
        return 2
    out = []
    counts = dict.fromkeys(CAUSES, 0)
    for n, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            out.append(line)
            continue
        f = line.split("\t")
        if len(f) != 3 or not f[0]:
            sys.stderr.write("imx95-g310-cause: line %d: want 3 tab-separated "
                             "columns (case, t6, imx95)\n" % n)
            return 2
        for r in f[1:]:
            if r not in RESULTS:
                sys.stderr.write("imx95-g310-cause: line %d: unknown result "
                                 "'%s'\n" % (n, r))
                return 2
        c = classify(*f)
        counts[c] += 1
        out.append("\t".join(f + [c]))
    for ln in out:
        print(ln)
    if summary:
        print("# cause\tcount")
        for c in CAUSES:
            print("# %s\t%d" % (c, counts[c]))
        print("# total\t%d" % sum(counts.values()))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
