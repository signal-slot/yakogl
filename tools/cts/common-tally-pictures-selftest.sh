#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/common-tally-pictures-selftest.sh
#
# Self-test of glmark2_pictures() in tools/cts/common-tally.py on the synthetic,
# redacted logs of testdata/common-tally-pictures/:
#   unparsed-after.log  a parsed picture line, then (after the "board: record, "
#                       marker) an unparseable line of the same scene: the parsed
#                       value is kept, the scene printed after it is not read
#   marker.log          lines after the marker are ignored, even parseable ones
#   ordinary.log        no marker, no bad line: a scene printed twice keeps its
#                       last line (unchanged behaviour)
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
command -v python3 > /dev/null || { echo "common-tally-pictures-selftest: python3 missing" >&2; exit 2; }
python3 -I - "$HERE" <<'PY'
import importlib.util, sys
here = sys.argv[1]
spec = importlib.util.spec_from_file_location("common_tally", here + "/common-tally.py")
ct = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ct)
D = here + "/testdata/common-tally-pictures/"
def load(n):
    return ct.glmark2_pictures(open(D + n).read())
fail = 0
def check(name, ok, got):
    global fail
    print(("ok   " if ok else "FAIL ") + name + ("" if ok else "  got: %r" % (got,)))
    fail += not ok
B, T = "[build] use-vbo=false", "[texture] x"
r = load("unparsed-after.log")
check("parsed line kept over a later unparseable one", r["scenes"][B] is not None and r["scenes"][B]["crc16"] == "7282"
      and r["raw"][B].startswith("800x600; probes c 000000ff"), r)
check("order and other scenes unchanged", r["order"] == [B, T] and r["scenes"][T]["crc16"] == "1111", r)
r = load("marker.log")
check("lines after the marker are ignored", r["order"] == [B] and r["scenes"][B]["crc16"] == "7282", r)
r = load("ordinary.log")
check("ordinary log: a scene printed twice keeps its last line", r["order"] == [B, T] and r["scenes"][T]["crc16"] == "3333"
      and r["scenes"][B]["crc16"] == "7282", r)
sys.exit(1 if fail else 0)
PY
