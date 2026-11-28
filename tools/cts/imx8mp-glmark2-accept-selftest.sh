#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of imx8mp-glmark2-accept.sh on synthetic notes built from the real
# skeleton note. Needs bash, python3, git and TMPDIR. Exit 2 if TMPDIR is unset.
set -u
[ -n "${TMPDIR:-}" ] || { echo "TMPDIR is not set" >&2; exit 2; }
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
ACC=$HERE/imx8mp-glmark2-accept.sh
SKEL=$ROOT/docs/notes/imx8mp-glmark2-pictures-by-eye.md
W=$(mktemp -d "$TMPDIR/accept-selftest.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
H=$(git -C "$ROOT" rev-parse --short=8 HEAD) || exit 2

# synth NOTE-OUT COMMIT|skeleton: built from the real note, which board
# sessions fill in over time (it may be a skeleton or finished). Its own
# '- **[' bullets are dropped; then either every row is filled, the skeleton
# phrases scrubbed and 33 bullets added, or every row is reset to the skeleton
# and the skeleton phrases added.
synth() {
    python3 - "$SKEL" "$1" "$2" <<'PY'
import sys
src, dst, sha = sys.argv[1:4]
# "x2:<sha>": the rows of an Armadillo-X2 KVM record (photograph+probes route)
route = "dump"
if sha.startswith("x2:"):
    sha, route = sha[3:], "photograph+probes"
out, scenes = [], []
for line in open(src).read().split("\n"):
    if line.startswith("- **["):
        continue
    if line.startswith("| ["):
        c = [x.strip() for x in line.strip().strip("|").split("|")]
        scenes.append(c[0])
        if sha == "skeleton":
            line = "| %s | - | - | - | not yet looked at |" % c[0]
        else:
            line = ("| %s | %s | records/imx8mp/20261124-1200-scene-%s/ | %s | right |"
                    % (c[0], sha, sha, route))
    if sha != "skeleton":
        for k in ("Not yet established", "To fill in", "Skeleton: no scene"):
            line = line.replace(k, "Established")
    out.append(line)
if sha == "skeleton":
    out.append("Skeleton: no scene looked at. Not yet established. To fill in.")
else:
    # the bullets go under '## What was looked at' (verdicts.py --complete
    # checks them there); a note without that section gets one at the end
    bullets = ["- **%s**: looked at it." % s for s in scenes]
    if "## What was looked at" in out:
        i = out.index("## What was looked at") + 1
        while i < len(out) and not out[i].startswith("## "):
            i += 1
        out[i:i] = bullets + [""]
    else:
        out += ["", "## What was looked at", ""] + bullets
open(dst, "w").write("\n".join(out) + "\n")
PY
}

fail=0
expect() { # label output pattern...
    local label=$1 out=$2 p
    shift 2
    for p in "$@"; do
        if ! printf '%s\n' "$out" | grep -q -- "$p"; then
            echo "FAIL  $label: no '$p' in:"
            printf '%s\n' "$out" | sed 's/^/        /'
            fail=1
            return
        fi
    done
    echo "ok    $label"
}
A() { bash "$ACC" --main HEAD --no-fetch --skip c2 "$@" 2>&1; }

synth "$W/skel.md" skeleton
out=$(A --note "$W/skel.md"); st=$?
expect "skeleton fails c1 and c4" "$out" '^c1 FAIL' '^c4 FAIL'
if [ $st -eq 0 ]; then echo "FAIL  skeleton exits 0"; fail=1; fi

synth "$W/good.md" "$H"
out=$(A --note "$W/good.md"); st=$?
expect "filled note passes c1 c3 c4 c5" "$out" '^c1 ok$' '^c3 ok$' '^c4 ok$' '^c5 ok$'
if [ $st -ne 0 ]; then echo "FAIL  filled note exits $st"; fail=1; fi

# an X2 record: KVM screenshots judged beside the probes of the same boot
synth "$W/x2.md" "x2:$H"
echo "Measured on the Armadillo-X2 (board=x2), 1920x1080 KVM screenshots, scene at 0,0,800,600." >> "$W/x2.md"
out=$(A --note "$W/x2.md"); st=$?
expect "X2 KVM note passes c1 c3 c4 c5" "$out" '^c1 ok$' '^c3 ok$' '^c4 ok$' '^c5 ok$'
if [ $st -ne 0 ]; then echo "FAIL  X2 KVM note exits $st"; fail=1; fi

cp "$W/good.md" "$W/ip.md"
echo "board at 10.$((1+1)).3.4" >> "$W/ip.md"
out=$(A --note "$W/ip.md")
expect "IPv4-like string fails c5" "$out" '^c5 FAIL'

synth "$W/bad.md" deadbeef
out=$(A --note "$W/bad.md")
expect "foreign commit fails c3" "$out" '^c3 FAIL'

if [ $fail -eq 0 ]; then echo "accept self-test: all ok"; fi
exit $fail
