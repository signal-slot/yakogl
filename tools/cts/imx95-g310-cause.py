#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""imx95-g310-cause.py - tag each NanoPC-T6 (Mali-G610) vs Verdin iMX95
(Mali-G310) dEQP-GLES2 per-case result pair with a candidate G310 cause.

Usage:
  tools/cts/imx95-g310-cause.py [--summary] [--commit-cases FILE] [FILE]

Input (FILE, or stdin when FILE is absent or '-'): one case a line,
  case<TAB>t6_result<TAB>imx95_result
with each result one of Pass, Fail, NotSupported, QualityWarning,
CompatibilityWarning, InternalError, Crash, Timeout, Missing, Lost. Lines that
start with '#' and empty lines are copied as they are. A first line
'case<TAB>...' is taken as the header and gets a 'cause' column too.

Output (stdout): every input line, with a fourth column, the cause. With
--summary, a per-cause count table follows, as '#' lines (so the output
can be read again as input): '# cause<TAB>count', one row per cause in
rule order (zeros included), then '# total<TAB>N' and
'# g310-only<TAB>N': the differences no compiler rule, refusal or renderer
name explains (total less no-difference, compiler-refusal, compiler-commit
and renderer-name).

--commit-cases FILE lists the dEQP cases the compiler commits between the
T6 baseline (8ff01055) and the iMX95 images (e63ab28e: a824ec97, d3876279,
9fa5bbe0; src/shaderc/online/mali) change: one case a line, '#' comments and
empty lines ignored; a name ending in '*' is a prefix. A differing pair for
such a case is 'compiler-commit' (unless it is a hang).

A malformed line (not three fields, an empty case name, an unknown result)
stops the tool with exit 2 and a message naming the line; nothing is
written to stdout then. An empty input gives an empty output (and, with
--summary, the table with zeros) and exit 0.

A CAUSE IS A CANDIDATE, NOT A PROOF. Each rule matches a case name and the
two results against what is known to differ between the two GPUs
(docs/notes/mali-g310-vs-g610.md sections 3 and 5). It says where to look
first; only a rerun, a trace or a probe on the board shows the cause.

The rules, in order, the first that matches wins:

  no-difference     both boards Pass: nothing to explain (an addition to
                    the plan's list, so that a whole-tree diff can be fed in
                    unfiltered).
  compiler-refusal  the same non-Pass on both boards: the online compiler,
                    which is word-identical for the two GPUs (note section
                    5, `src/shaderc/online/mali/`), refuses or miscompiles
                    on both (issue #11251).
  hang              Timeout, Crash, Missing or Lost on the iMX95 only; checked
                    before every name rule.
  compiler-commit   a differing pair for a case of --commit-cases.
  renderer-name     dEQP-GLES2.info.*: GL_RENDERER is per model
                    ("Arm Mali-G310v2 (YakoGL)" against "Arm Mali-G610
                    (YakoGL)", src/mali/model.rs), so the info cases'
                    reported strings differ by design.
  tib-budget        cases that draw into a target whose tile buffer need
                    differs (see TIB_RULE below).
  tiler-levels      large framebuffers (see TILER_RULE below).
  shader-present    timing and performance cases: SHADER_PRESENT 0x1 is
                    one shader core against the G610's four (0x50005).
  unexplained       anything else.

Exit status: 0 done, 2 bad usage or malformed input.
"""

import argparse
import re
import sys

RESULTS = (
    "Pass",
    "Fail",
    "NotSupported",
    "QualityWarning",
    "CompatibilityWarning",
    "InternalError",
    "Crash",
    "Timeout",
    "Missing",
    "Lost",
)
HANG_RESULTS = ("Crash", "Timeout", "Missing", "Lost")

CAUSES = (
    "no-difference",
    "compiler-refusal",
    "hang",
    "compiler-commit",
    "renderer-name",
    "tib-budget",
    "tiler-levels",
    "shader-present",
    "unexplained",
)
NOT_G310 = ("no-difference", "compiler-refusal", "compiler-commit", "renderer-name")

RENDERER_RULE = re.compile(r"^dEQP-GLES2\.info\.")

# TIB_RULE. docs/notes/mali-g310-vs-g610.md section 5 ("src/mali/ and the
# kernel side"): the tile buffer comes from Mesa's MODEL_TB_SIZES row the
# board's GPU_ID and CORE_FEATURES select (src/mali/model.rs:95-120). The
# measured G310v2 gets TileBudget colour 8192, Z 4096 (halved from 16384 /
# 8192); the G610 colour 16384, Z 8192. tile_size_of (src/mali/frame.rs)
# takes colour >> log2_ceil(bytes a pixel, summed over the targets and
# their samples) and z >> log2_ceil(4 * samples), capped at
# MAX_EFFECTIVE_TILE (32 * 32 = 1024 pixels). So:
#   - one single-sample target of 4 bytes a pixel or less (every core ES 2.0
#     colour format: rgba8, rgb565, rgba4, rgb5_a1, bgra) gets a 1024-pixel
#     tile on both GPUs: the same frame;
#   - 9 to 16 bytes a pixel (a 4x multisample rgba8 target is 16; rgb32f is
#     12, rgba32f 16) is a 1024-pixel tile on the G610 and a smaller one on
#     the G310v2, and every 4x frame's Z tile differs (512 against 256);
#   - more than 16 bytes a pixel shrinks both, differently.
# The cases whose names say they draw into such a target, or into an FBO
# whose tile the frame code sizes (fbo.render.*), are the candidates:
# multisample.* and multisampled_render_to_texture (4x), render to texture,
# and float colour formats of 12 bytes a pixel or more. fbo.render.resize
# is left to the next rule: its targets are 4-byte formats, the difference
# there is the size. fbo.completeness.* and fbo.api.* draw nothing.
TIB_RULE = re.compile(
    r"\.multisample"  # multisample.*, multisampled_render_to_texture.*
    r"|render_?to_?texture"
    r"|\.fbo\.render\.(?!resize\.)"
    r"|(?:^|[._])rgba?32f(?:$|[._])"  # rgb32f 12, rgba32f 16 bytes a pixel
)

# TILER_RULE. docs/notes/mali-g310-vs-g610.md section 5: the tiler's level
# count is TILER_FEATURES bits 11:8 (src/mali/frame.rs tiler_max_levels),
# 8 on both boards (0x809), and the hierarchy mask follows Mesa 26.2.1's
# rule from it and the frame size. The mask changes with the framebuffer's
# size, so frames that grow (fbo.render.resize) or are 2048 pixels on a side
# or more are where a hierarchy difference would show first. With the same
# 8 levels on both boards this is the weakest candidate of the list.
TILER_RESIZE = re.compile(r"\.fbo\.render\.resize\.")
TILER_SIZE = re.compile(r"(?<![0-9])([0-9]+)x([0-9]+)(?![0-9])")
TILER_SIZE_WORD = re.compile(r"\.size\..*?(?<![0-9])([0-9]{4,})(?![0-9])")
TILER_MIN = 2048

# SHADER_PRESENT 0x1 (one core): a case whose result is a time or a rate
# can move on the G310 with no defect at all.
SHADER_RULE = re.compile(
    r"^dEQP-GLES2\.performance\."
    r"|(?:^|[._])(?:timing|timer|speed|throughput|benchmark)(?:$|[._])"
)


class Malformed(Exception):
    pass


def tiler_match(case):
    if TILER_RESIZE.search(case):
        return True
    for w, h in TILER_SIZE.findall(case):
        if max(int(w), int(h)) >= TILER_MIN:
            return True
    m = TILER_SIZE_WORD.search(case)
    return bool(m and int(m.group(1)) >= TILER_MIN)


def commit_match(case, commit):
    exact, prefixes = commit
    return case in exact or any(case.startswith(p) for p in prefixes)


def load_commit_cases(path):
    exact, prefixes = set(), []
    with open(path, encoding="utf-8") as fh:
        for raw in fh:
            x = raw.strip()
            if not x or x.startswith("#"):
                continue
            x = x.split("\t")[0].strip()
            if x.endswith("*"):
                prefixes.append(x[:-1])
            else:
                exact.add(x)
    return exact, tuple(prefixes)


def cause_of(case, t6, imx, commit=(frozenset(), ())):
    if t6 == imx:
        return "no-difference" if t6 == "Pass" else "compiler-refusal"
    if imx in HANG_RESULTS and t6 not in HANG_RESULTS:
        return "hang"
    if commit_match(case, commit):
        return "compiler-commit"
    if RENDERER_RULE.search(case):
        return "renderer-name"
    if TIB_RULE.search(case):
        return "tib-budget"
    if tiler_match(case):
        return "tiler-levels"
    if SHADER_RULE.search(case):
        return "shader-present"
    return "unexplained"


def tag(lines, commit=(frozenset(), ())):
    out = []
    counts = dict.fromkeys(CAUSES, 0)
    for n, raw in enumerate(lines, 1):
        line = raw.rstrip("\r\n")
        if line == "" or line.startswith("#"):
            out.append(line)
            continue
        f = line.split("\t")
        if len(f) != 3:
            raise Malformed("line %d: %d tab-separated fields, want 3 (case, t6_result, imx95_result)" % (n, len(f)))
        case, t6, imx = (x.strip() for x in f)
        if n == 1 and case == "case":
            out.append(line + "\tcause")
            continue
        if not case:
            raise Malformed("line %d: empty case name" % n)
        for who, r in (("t6_result", t6), ("imx95_result", imx)):
            if r not in RESULTS:
                raise Malformed("line %d: %s %r is not one of %s" % (n, who, r, ", ".join(RESULTS)))
        c = cause_of(case, t6, imx, commit)
        counts[c] += 1
        out.append(line + "\t" + c)
    return out, counts


def main(argv):
    ap = argparse.ArgumentParser(
        prog="imx95-g310-cause.py",
        description="Tag T6 (Mali-G610) vs Verdin iMX95 (Mali-G310) dEQP-GLES2 per-case "
        "result pairs with a candidate G310 cause (a candidate, not a proof).",
        epilog="Rules, first match wins: " + ", ".join(CAUSES) + ". Exit 0 done, 2 bad usage or input.",
    )
    ap.add_argument("--summary", action="store_true", help="append a per-cause count table as '#' lines")
    ap.add_argument("--commit-cases", metavar="FILE", help="dEQP cases the compiler commits 8ff01055..e63ab28e change (one a line)")
    ap.add_argument("file", nargs="?", default="-", help="the TSV (case, t6_result, imx95_result); '-' or absent: stdin")
    a = ap.parse_args(argv)
    try:
        if a.file == "-":
            lines = sys.stdin.read().splitlines()
        else:
            with open(a.file, encoding="utf-8") as fh:
                lines = fh.read().splitlines()
    except (OSError, UnicodeDecodeError) as e:
        print("imx95-g310-cause: %s: %s" % (a.file, e), file=sys.stderr)
        return 2
    commit = (frozenset(), ())
    if a.commit_cases:
        try:
            commit = load_commit_cases(a.commit_cases)
        except (OSError, UnicodeDecodeError) as e:
            print("imx95-g310-cause: %s: %s" % (a.commit_cases, e), file=sys.stderr)
            return 2
    try:
        out, counts = tag(lines, commit)
    except Malformed as e:
        print("imx95-g310-cause: %s: %s" % (a.file if a.file != "-" else "stdin", e), file=sys.stderr)
        return 2
    w = sys.stdout.write
    for line in out:
        w(line + "\n")
    if a.summary:
        w("# cause\tcount\n")
        for c in CAUSES:
            w("# %s\t%d\n" % (c, counts[c]))
        w("# total\t%d\n" % sum(counts.values()))
        w("# g310-only\t%d\n" % sum(n for c, n in counts.items() if c not in NOT_G310))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
