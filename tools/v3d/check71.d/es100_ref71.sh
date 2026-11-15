#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# es100_ref71: tools/v3d/es100_counts_71.tsv, Mesa's V3D 7.1 output for the es100
# corpus (the table the 7.1 back end is compared with), is complete and still
# what Mesa gives.
#   1. (no input) the table lists exactly the .vert/.frag sources of
#      tools/shaders/es100, each once, sorted, with well-formed rows and a header
#      that agrees with them (126 programs, 124 compiled, 2 refused by name);
#   2. (V3D_SHC, or the build places tools/v3d/glsl2v3d.py knows, plus
#      glslangValidator) tools/v3d/es100_ref71.sh is rerun and must write the
#      same bytes. Without Mesa's v3d_shc step 2 does not run and the last line
#      says so: the table was not compared with Mesa, and that is not a pass of
#      the comparison.
# Convention: tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1
name=es100_ref71
dir=build/v3d71/$name
mkdir -p "$dir" || exit 1

python3 -I - tools/v3d/es100_counts_71.tsv tools/shaders/es100 <<'PY' || exit 1
import os
import sys

table, root = sys.argv[1:3]
bad = []
want = sorted(os.path.relpath(os.path.join(d, f), os.path.dirname(root))
              for d, _, fs in os.walk(root) for f in fs if f.endswith((".vert", ".frag")))
have, yes, header = [], 0, ""
for line in open(table):
    line = line.rstrip("\n")
    if line.startswith("# programs:"):
        header = line
    if line.startswith("#") or not line.strip():
        continue
    c = line.split("\t")
    if len(c) != 7:
        bad.append("malformed row %r" % line)
        continue
    have.append(c[0])
    if c[1] != ("vs" if c[0].endswith(".vert") else "fs"):
        bad.append("%s: stage %s" % (c[0], c[1]))
    if c[2] == "yes":
        yes += 1
        if c[3] != "ok" or not all(x.isdigit() for x in c[4:]):
            bad.append("%s: compiled row is not ok / counts" % c[0])
    elif c[2] == "no":
        if not c[3] or c[3] == "ok" or c[4:] != ["-", "-", "-"]:
            bad.append("%s: a refusal names its reason and has no counts" % c[0])
    else:
        bad.append("%s: compiled is yes or no, not %r" % (c[0], c[2]))
if len(set(have)) != len(have):
    bad.append("a program is listed twice")
if have != sorted(have):
    bad.append("rows are not sorted by path")
if sorted(set(have)) != want:
    bad.append("programs differ from tools/shaders/es100: missing %s, extra %s"
               % (sorted(set(want) - set(have)), sorted(set(have) - set(want))))
if (len(have), yes) != (126, 124):
    bad.append("table has %d programs, %d compiled; the pins are 126 and 124" % (len(have), yes))
if header != "# programs: %d, compiled: %d, refused: %d" % (len(have), yes, len(have) - yes):
    bad.append("header %r does not agree with the rows" % header)
for b in bad:
    print("FAIL:", b)
if bad:
    sys.exit(1)
print("table: %d es100 programs, Mesa compiles %d at 7.1" % (len(have), yes))
PY

shc=${V3D_SHC:-}
if [ -z "$shc" ]; then
  rel=mesa-26.2.1/build-v3d/src/broadcom/v3d_shc
  deps=${CONNECTED_DEPS_DIR:-/nonexistent}
  for p in "$HOME/pvr-lab/mesa-build/$rel" "$deps/mesa-pan/$rel"; do
    if [ -x "$p" ]; then shc=$p; break; fi
  done
fi
if [ -z "$shc" ] || [ ! -x "$shc" ] || ! command -v glslangValidator >/dev/null 2>&1; then
  echo "$name: table complete (126 programs, 124 compiled); Mesa not rerun: gap V3D_SHC or glslangValidator missing (the table was not compared with Mesa), 0 failed"
  exit 0
fi
V3D_SHC=$shc TMPDIR=$dir tools/v3d/es100_ref71.sh --out "$dir/es100_counts_71.tsv" >"$dir/run.log" 2>&1 || {
  tail -20 "$dir/run.log"
  exit 1
}
if ! cmp -s "$dir/es100_counts_71.tsv" tools/v3d/es100_counts_71.tsv; then
  diff "$dir/es100_counts_71.tsv" tools/v3d/es100_counts_71.tsv | head -20
  echo "FAIL: Mesa's 7.1 output moved from tools/v3d/es100_counts_71.tsv"
  exit 1
fi
echo "$name: table complete and equal to a rerun of Mesa's v3d_shc, 0 failed"
