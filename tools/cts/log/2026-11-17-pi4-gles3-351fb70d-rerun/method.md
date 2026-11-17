<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# How the 351fb70d rerun map is made

The map in `README.md` and its two tables are made by the two scripts below.
They are kept here as text rather than as files of the tree because they belong
to this record, not to `tools/cts/`. Run the steps from the root of a checkout of
the commit, with a VK-GL-CTS `93bca01861b0` checkout that has the yakogl target
(`tools/cts/cts-build.sh` applies `tools/cts/cts-yakogl-target.patch`).
Everything written goes under `build/` (gitignored). HOST-ONLY throughout.

## Steps

```sh
M=tools/cts/log/2026-11-17-pi4-gles3-351fb70d-rerun/method.md
mkdir -p build/rm
awk '/^```python rerun-map.py$/{f=1;next} /^```$/{f=0} f' $M > build/rm/rerun-map.py
awk '/^```sh percase.sh$/{f=1;next} /^```$/{f=0} f' $M > build/rm/percase.sh

# 1. the whole mustpass, chunked (builds deqp-gles3 and the V3D library)
tools/cts/host-deqp-run.sh --gpu=v3d --module=gles3 --cts=<VK-GL-CTS> --jobs=4

# 2. a first map without --percase, for the case list: the board non-Pass
#    cases outside plan #376 (2,763 at b1b5b845)
python3 -I build/rm/rerun-map.py --tree tools/cts/pi4-gles3-tree.txt \
    --mustpass tools/cts/gles3-mustpass.txt \
    --board 'tools/cts/log/2026-11-pi4-gles3-b1b5b845-rows*' \
    --host build/host-deqp/run-<commit>-v3d-gles3 \
    --failmap build/rm/failmap.tsv --hostmap build/rm/hostmap.tsv
awk -F'\t' 'NR>5 && $3!="NotSupported" && $1!~/^(S0[3-8]|T14|TF1|TF2)$/ {print $2}' \
    build/rm/failmap.tsv > build/rm/list

# 3. those cases one process each, then the map again with their results
bash build/rm/percase.sh build/rm/list build/rm/pc 4
python3 -I build/rm/rerun-map.py --tree tools/cts/pi4-gles3-tree.txt \
    --mustpass tools/cts/gles3-mustpass.txt \
    --board 'tools/cts/log/2026-11-pi4-gles3-b1b5b845-rows*' \
    --host build/host-deqp/run-<commit>-v3d-gles3 --percase build/rm/pc \
    --failmap build/rm/failmap.tsv --hostmap build/rm/hostmap.tsv
```

At 351fb70d step 3 prints
`rerun-map: 100 images, 42810 mustpass; no change expected 57, plan #376 9, rerun 34`.
`hostmap.tsv` has one row per image, and the README's per-image table is its
columns: "fixed" is `fixed + ns_changed`, "still" is `h_refused + h_api`.
`failmap.tsv` has one row per mustpass case the Pi 4 did not pass at b1b5b845
(7,110 rows), with its board status and class, its host status and class, and
whether it is fixed on the host.

## The replacement block for docs/es3/pi4-gles3-fails.tsv

This block moves plan #320's file onto b1b5b845 for all 100 images, in place of
the 0ff58329, 8990a5cc and 009f62a5 rows it still has (M1's 62 NoResult rows of
the 0ff58329 abort among them). The b1b5b845 consoles are the only input; the host
run does not change a row. The rows of the 34 rerun images change again once the
board rerun lands.

```sh
{ printf '%s\n' \
  '# Pi 4 dEQP-GLES3 at b1b5b8456ec69247c64d1bd07e42461266dc5136 only, every mustpass case that is not Pass in each of the' \
  '# 100 images of tools/cts/pi4-gles3-tree.txt, from the consoles of tools/cts/log/2026-11-pi4-gles3-b1b5b845-rows*/' \
  '# (S32c1-4 from S32c.console.txt, run before the re-split 361fdb4e). rows: <record>:<status> <case>, sorted by case;' \
  '# NoResult = the case never began. 7110 rows. The rows of the 34 rerun images change when the rerun lands.'
  awk -F'\t' 'NR>5 {l=$1; if (l ~ /^S32c[0-9]$/) l="S32c"; print l ".console.txt:" $3 " " $2}' build/rm/failmap.tsv |
    LC_ALL=C sort -t' ' -k2,2 -s; } > build/rm/fails-b1b5b845.tsv
```

These four header lines replace the file's four comment lines at the top; the
rows replace all of its rows. The 7,110 rows are byte for byte the ones the b1e4c0f5 record made.

## percase.sh

```sh percase.sh
#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# One dEQP process per case, for the cases of a list: the deqp-gles3 and the
# V3D library that tools/cts/host-deqp-run.sh built (GPU=v3d MODULE=gles3),
# run with its own flags (run_chunk). Why: the V3D library logs a draw
# refusal once per cause and process (src/v3d/backend.rs count_refusal), so
# in a 100-case chunk only the first case of a cause shows it; alone, every
# case does. HOST-ONLY, as the run it repeats.
#
#   percase.sh <case list> <out dir> [jobs]
#
# Writes <out dir>/<n>.out (dEQP's stdout and the library's stderr) and
# <n>.qpa per case, in the c_*.out / c_*.qpa form rerun-map.py reads.
set -euo pipefail
REPO=$(git rev-parse --show-toplevel)
BASE=$REPO/build/host-deqp
LIST=$(realpath "$1"); OUT=$2; JOBS=${3:-4}
mkdir -p "$OUT"; OUT=$(realpath "$OUT")
export YAKOGL_V3D_TILED_COPIES_BYTES=${YAKOGL_V3D_TILED_COPIES_BYTES:-16777216}
export BASE OUT
one() {
    local n=$1 c=$2
    echo "$c" > "$OUT/c_$n"
    (cd "$BASE/deqp-gles3/modules/gles3" &&
     LD_LIBRARY_PATH="$BASE/lib-v3d-es3" timeout 300 ./deqp-gles3 --deqp-case="$c" \
        --deqp-log-filename="$OUT/c_$n.qpa" --deqp-log-shader-sources=enable --deqp-log-images=disable \
        --deqp-surface-type=pbuffer --deqp-surface-width=256 --deqp-surface-height=256 \
        > "$OUT/c_$n.out" 2>&1) || true
}
export -f one
nl -nrz -w5 "$LIST" | xargs -P "$JOBS" -L1 bash -c 'one "$0" "$1"'
echo "percase: $(ls "$OUT"/c_*.qpa | wc -l) of $(wc -l < "$LIST") cases logged in $OUT"
```

## rerun-map.py

```python rerun-map.py
#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The Pi 4 dEQP-GLES3 rerun map: each image's b1b5b845 board result against a host run.

  python3 -I rerun-map.py --tree tools/cts/pi4-gles3-tree.txt \
      --mustpass tools/cts/gles3-mustpass.txt \
      --board 'tools/cts/log/2026-11-pi4-gles3-b1b5b845-rows*' \
      --host build/host-deqp/run-<commit>-v3d-gles3 --percase build/rm/pc \
      --failmap failmap.tsv --hostmap hostmap.tsv

Board: every <LABEL>.console.txt of the --board directories (the UART console of
one image; S32c's console stands for S32c1..S32c4, the re-split of 361fdb4e).
Host: the run directory of tools/cts/host-deqp-run.sh (GPU=v3d, MODULE=gles3):
its c_*.qpa give each case's status and message, its c_*.out the library's
stderr lines between dEQP's "Test case '<name>'.." lines. The library logs a
draw refusal once per cause and process, so --percase (percase.sh's directory:
the board's non-Pass cases, one process each) gives those cases their own lines
and results, which win over the chunked run's.

Each mustpass case the board did not pass gets a class on each side:

  refusal: <text>   a yakogl line that refuses (a compile, a link, a draw)
  api: <message>    a dEQP verdict a host run reproduces (a compile or link
                    failure, a GL error, an incomplete framebuffer, a query)
  render: <message> a pixel verdict: on the board a real Fail, on the host
                    meaningless (the LINUX OSAL reads back black)
  never began       the board's dEQP stopped before the case (NoResult)

A case is "fixed on the host" when its board class was a refusal, an api
verdict or never began, and the host run gives it neither a refusal nor an
api verdict (it passes, or fails only a render verdict, HOST-ONLY). An image
is "rerun" when it has a fixed case or a NotSupported that the host no longer
reports, "no change expected" otherwise. The images of plan #376 are named so.
"""
import argparse
import glob
import html
import os
import re
import sys
from collections import Counter, OrderedDict
from fnmatch import fnmatchcase

PLAN376 = {"S03", "S04", "S05", "S06", "S07", "S08", "T14", "TF1", "TF2"}
CASE = re.compile(r"^Test case '([^']+)'\.\.")
STATUS = re.compile(r"^  (Pass|Fail|NotSupported|QualityWarning|CompatibilityWarning|"
                    r"ResourceError|InternalError|Crash|Timeout) \((.*)\)\s*$")
# A yakogl line that refuses something. Informational lines (arena sizes, the
# clear program, "has no uniform" for an optimised-out name) are not.
REFUSAL = re.compile(r"refused|NotYet|does not fit|is declared|has another|has different|"
                     r"not recorded|no GPU memory|truncated|is not declared|cannot")
RENDER = {"Image comparison failed", "Fail", "Image verification failed", "Shader failed",
          "Got unexpected texture size", "Failed", "Incorrect rasterization",
          "Found invalid pixel values", "Interpolation was calculated using projected coordinateds",
          "Writing to container with deleted attachment modified a new object.",
          "Image compare failed", "Shader output 0 has low-quality shader precision"}


def norm(text):
    """A message with its numbers and source lines folded, so classes count."""
    text = re.sub(r"\bprogram \d+\b", "program N", text)
    text = re.sub(r"\.cpp:\d+", ".cpp:N", text)
    text = re.sub(r"\(-?\d+\)", "(N)", text)
    text = re.sub(r"\b\d{3,}\b", "N", text)
    return text.strip()


def read_tree(tree):
    rows, label = [], None
    for line in open(tree, encoding="utf-8"):
        line = line.rstrip("\n")
        m = re.match(r"^# image ([^:]+):", line)
        if m:
            label = m.group(1)
            continue
        if not line.strip() or line.startswith("#"):
            continue
        rows.append((label, line.strip().split(",")))
        label = None
    return rows


def image_of(rows, mustpass):
    out = OrderedDict((lab, []) for lab, _ in rows)
    for name in mustpass:
        for lab, pats in rows:
            if any(fnmatchcase(name, p) for p in pats):
                out[lab].append(name)
                break
    return out


def parse_stream(path, cases):
    """{case: (status, message, [refusal lines])} of a console or a c_*.out file."""
    cur = None
    for line in open(path, encoding="utf-8", errors="replace"):
        line = line.rstrip("\n")
        m = CASE.match(line)
        if m:
            cur = m.group(1)
            cases.setdefault(cur, [None, "", []])
            continue
        if cur is None:
            continue
        m = STATUS.match(line)
        if m:
            cases[cur][0], cases[cur][1] = m.group(1), m.group(2)
            continue
        if line.startswith("yakogl") and REFUSAL.search(line):
            r = norm(line.split(": ", 1)[1] if ": " in line else line)
            if r not in cases[cur][2]:
                cases[cur][2].append(r)
    return cases


def parse_qpa(paths):
    """{case: (status, message)} of a host run's .qpa logs (the last result wins)."""
    st, cur = {}, None
    res = re.compile(r'^\s*<Result StatusCode="([^"]+)">(.*?)</Result>')
    for p in paths:
        for line in open(p, encoding="utf-8", errors="replace"):
            if line.startswith("#beginTestCaseResult "):
                cur = line.split()[1]
                st[cur] = ("Incomplete", "")
                continue
            m = res.match(line)
            if m and cur:
                st[cur] = (m.group(1), html.unescape(m.group(2)))
    return st


def klass(status, msg, refusals):
    if status is None:
        return "never began"
    if status in ("Pass", "NotSupported"):
        return status
    if refusals:
        return "refusal: " + refusals[0]
    if msg in RENDER:
        return "render: " + msg
    return "api: " + norm(msg)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tree", required=True)
    ap.add_argument("--mustpass", required=True)
    ap.add_argument("--board", required=True, help="glob of the board session directories")
    ap.add_argument("--host", required=True)
    ap.add_argument("--percase", help="percase.sh's directory: these cases' own runs win")
    ap.add_argument("--failmap", required=True)
    ap.add_argument("--hostmap", required=True)
    a = ap.parse_args()

    rows = read_tree(a.tree)
    mustpass = [l.strip() for l in open(a.mustpass, encoding="utf-8")
                if l.strip() and not l.startswith("#")]
    images = image_of(rows, mustpass)

    consoles = {}
    for d in sorted(glob.glob(a.board)):
        for p in sorted(glob.glob(os.path.join(d, "*.console.txt"))):
            consoles[os.path.basename(p)[: -len(".console.txt")]] = p

    host_stream = {}
    for p in sorted(glob.glob(os.path.join(a.host, "c_*.out"))):
        parse_stream(p, host_stream)
    host_qpa = parse_qpa(sorted(glob.glob(os.path.join(a.host, "c_*.qpa"))))
    if a.percase:
        # one process per case (percase.sh): its refusal lines are the case's own
        pc_stream = {}
        for p in sorted(glob.glob(os.path.join(a.percase, "c_*.out"))):
            parse_stream(p, pc_stream)
        host_stream.update(pc_stream)
        host_qpa.update(parse_qpa(sorted(glob.glob(os.path.join(a.percase, "c_*.qpa")))))
    skipped = set()
    for p in glob.glob(os.path.join(a.host, "c_*.skipped")):
        skipped.update(l.strip() for l in open(p, encoding="utf-8") if l.strip())

    def host(case):
        if case in skipped or case not in host_qpa:
            return "never began", "-"
        status, msg = host_qpa[case]
        refs = host_stream.get(case, [None, "", []])[2]
        return klass(status if status != "Incomplete" else "Crash", msg, refs), status

    fm = open(a.failmap, "w", encoding="utf-8")
    fm.write("# SPDX-License-Identifier: Apache-2.0\n")
    fm.write("# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n")
    fm.write("# every mustpass case the Pi 4 did not pass at b1b5b845, with its class on the board\n")
    fm.write("# and on the host run (rerun-map.py; a host render verdict is HOST-ONLY and means nothing)\n")
    fm.write("image\tcase\tboard_status\tboard_class\thost_status\thost_class\tfixed_on_host\n")
    hm = open(a.hostmap, "w", encoding="utf-8")
    hm.write("# SPDX-License-Identifier: Apache-2.0\n")
    hm.write("# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n")
    hm.write("# one row per image of the tree (rerun-map.py): b1b5b845 board counts, host counts, verdict\n")
    hm.write("image\tmustpass\tb_pass\tb_fail\tb_ns\tb_other\tb_never\th_began\th_never\t"
             "h_refused\th_api\tfixed\tns_changed\tverdict\tboard_classes\thost_classes\n")
    totals = Counter()
    for lab, names in images.items():
        src = consoles.get(lab) or consoles.get(re.sub(r"\d+$", "", lab))
        board = parse_stream(src, {}) if src else {}
        c = Counter()
        bcls, hcls = Counter(), Counter()
        for n in names:
            st, msg, refs = board.get(n, [None, "", []])
            hk, hst = host(n)
            c["h_began"] += hk != "never began"
            c["h_never"] += hk == "never began"
            if st == "Pass":
                c["b_pass"] += 1
                continue
            bk = klass(st, msg, refs)
            if st is None:
                c["b_never"] += 1
            elif st == "Fail":
                c["b_fail"] += 1
            elif st == "NotSupported":
                c["b_ns"] += 1
            else:
                c["b_other"] += 1
            fixed = ""
            if bk.split(":")[0] in ("refusal", "api", "never began") and \
                    not hk.startswith(("refusal", "api", "never began")):
                fixed = "yes"
                c["fixed"] += 1
            if st == "NotSupported" and hst not in ("NotSupported", "-"):
                fixed = "n/s changed"
                c["ns_changed"] += 1
            if st != "NotSupported":
                c["h_refused"] += hk.startswith("refusal")
                c["h_api"] += hk.startswith("api")
                bcls[bk] += 1
                if not hk.startswith(("render", "Pass")):
                    hcls[hk] += 1
            fm.write("\t".join([lab, n, st or "NoResult", bk, hst, hk, fixed]) + "\n")
        if lab in PLAN376:
            verdict = "plan #376"
        elif c["fixed"] or c["ns_changed"]:
            verdict = "rerun"
        else:
            verdict = "no change expected"
        totals[verdict] += 1
        fmt = lambda cc: "; ".join("%s x%d" % (k, v) for k, v in cc.most_common()) or "-"
        hm.write("\t".join([lab, str(len(names))] + [str(c[k]) for k in (
            "b_pass", "b_fail", "b_ns", "b_other", "b_never", "h_began", "h_never",
            "h_refused", "h_api", "fixed", "ns_changed")] + [verdict, fmt(bcls), fmt(hcls)]) + "\n")
    print("rerun-map: %d images, %d mustpass; %s" % (
        len(images), sum(len(v) for v in images.values()),
        ", ".join("%s %d" % kv for kv in sorted(totals.items()))))
    return 0


if __name__ == "__main__":
    sys.exit(main())
```
