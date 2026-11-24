#!/usr/bin/env bash
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

# synth NOTE-OUT COMMIT: fill every table row and add 33 bullets.
synth() {
    python3 - "$SKEL" "$1" "$2" <<'PY'
import sys
src, dst, sha = sys.argv[1:4]
out, scenes = [], []
for line in open(src).read().split("\n"):
    if line.startswith("| ["):
        c = [x.strip() for x in line.strip().strip("|").split("|")]
        scenes.append(c[0])
        line = ("| %s | %s | records/imx8mp/20261124-1200-scene-%s/ | dump | right |"
                % (c[0], sha, sha))
    for k in ("Not yet established", "To fill in", "Skeleton: no scene"):
        line = line.replace(k, "Established")
    out.append(line)
out +=[""] + ["- **%s**: looked at it." % s for s in scenes]
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

out=$(A --note "$SKEL"); st=$?
expect "skeleton fails c1 and c4" "$out" '^c1 FAIL' '^c4 FAIL'
if [ $st -eq 0 ]; then echo "FAIL  skeleton exits 0"; fail=1; fi

synth "$W/good.md" "$H"
out=$(A --note "$W/good.md"); st=$?
expect "filled note passes c1 c3 c4 c5" "$out" '^c1 ok$' '^c3 ok$' '^c4 ok$' '^c5 ok$'
if [ $st -ne 0 ]; then echo "FAIL  filled note exits $st"; fail=1; fi

cp "$W/good.md" "$W/ip.md"
echo "board at 192.168.1.20" >> "$W/ip.md"
out=$(A --note "$W/ip.md")
expect "IPv4-like string fails c5" "$out" '^c5 FAIL'

synth "$W/bad.md" deadbeef
out=$(A --note "$W/bad.md")
expect "foreign commit fails c3" "$out" '^c3 FAIL'

if [ $fail -eq 0 ]; then echo "accept self-test: all ok"; fi
exit $fail
