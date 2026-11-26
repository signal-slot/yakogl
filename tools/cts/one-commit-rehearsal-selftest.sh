#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/one-commit-rehearsal-selftest.sh
#
# Rehearses the one-commit glmark2 docs edit (#1196) on synthetic records:
# copies the six real doc files to a temporary tree, builds three synthetic
# common records (am62p, pi4, imx8mp) at HEAD's sha8 with common-row.py's
# synth(), runs one-commit-docs.py --root on the copy, then
# readme-glmark2-check.py (--ref HEAD --scenes terrain,shadow,refract
# --verdicts --status --checklist --roadmap) and the WEBSITE_GLMARK2_DIR mode
# of tools/website-check.sh on the copy (both must pass), and last a copy whose
# data.js pi4 score is one point off (the website check must fail). The real
# files are never written. Needs python3 and git only.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
FILES="README.md tools/cts/RESULTS.md tools/cts/STATUS.md website/data.js docs/release/checklist.md ROADMAP.md"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/one-commit-rehearsal.XXXXXX")
trap 'rm -rf "$TMP"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

SHA8=$(git -C "$REPO" rev-parse --short=8 HEAD)

# (1) the six real files, copied
TREE=$TMP/tree
for f in $FILES; do
    mkdir -p "$TREE/$(dirname "$f")"
    cp "$REPO/$f" "$TREE/$f"
done

# (2) three synthetic records at HEAD's sha8, each with its dated copy to cite
BOARDS="am62p pi4 imx8mp"
for b in $BOARDS; do
    rec=$(python3 -I -c '
import importlib.util, sys
spec = importlib.util.spec_from_file_location("common_row", sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
print(m.synth(sys.argv[2], sys.argv[3], sys.argv[4]))
' "$HERE/common-row.py" "$TMP" "$b" "$SHA8")
    declare "REC_$b=$rec"
    dated=$TMP/records/$b/20261125-1200-common-$SHA8
    cp -r "$rec" "$dated"
    declare "CITE_$b=$dated"
done

# (3) the docs edit on the copy
python3 -I "$HERE/one-commit-docs.py" --root "$TREE" \
    --am62p "$REC_am62p" --pi4 "$REC_pi4" --imx8mp "$REC_imx8mp" \
    --cite-am62p "$CITE_am62p" --cite-pi4 "$CITE_pi4" --cite-imx8mp "$CITE_imx8mp" \
    >"$TMP/docs.out" 2>&1 || { cat "$TMP/docs.out" >&2; fail "one-commit-docs.py exited non-zero"; }
cmp -s "$TREE/README.md" "$REPO/README.md" && fail "README.md was not edited"

# (4) the record checker on the edited copy
python3 -I "$HERE/readme-glmark2-check.py" \
    --readme "$TREE/README.md" --results "$TREE/tools/cts/RESULTS.md" \
    --status-file "$TREE/tools/cts/STATUS.md" \
    --checklist-file "$TREE/docs/release/checklist.md" \
    --roadmap-file "$TREE/ROADMAP.md" \
    --ref HEAD --scenes terrain,shadow,refract \
    --verdicts --status --checklist --roadmap >"$TMP/check.out" 2>&1 \
    || { cat "$TMP/check.out" >&2; fail "readme-glmark2-check.py rejects the edited copy"; }

# (5) the website comparison on the edited README.md and data.js
WEB=$TMP/web
mkdir "$WEB"
cp "$TREE/README.md" "$WEB/README.md"
cp "$TREE/website/data.js" "$WEB/data.js"
WEBSITE_GLMARK2_DIR=$WEB bash "$REPO/tools/website-check.sh" >"$TMP/web.out" 2>&1 \
    || { cat "$TMP/web.out" >&2; fail "website-check.sh rejects the edited copy"; }
grep -q "glmark2 fixture ok" "$TMP/web.out" || { cat "$TMP/web.out" >&2; fail "website-check.sh did not run the glmark2 comparison"; }

# (6) one score one point off in data.js must fail
BAD=$TMP/webbad
mkdir "$BAD"
cp "$WEB/README.md" "$BAD/README.md"
python3 -I - "$WEB/data.js" "$BAD/data.js" <<'PY'
import re, sys
t = open(sys.argv[1], encoding="utf-8").read()
m = re.search(r'("glmark2\.pi4": \{[^}]*?"value": )(\d+)', t)
if not m:
    sys.exit("no glmark2.pi4 value in data.js")
t = t[:m.start(2)] + str(int(m.group(2)) + 1) + t[m.end(2):]
open(sys.argv[2], "w", encoding="utf-8").write(t)
PY
if WEBSITE_GLMARK2_DIR=$BAD bash "$REPO/tools/website-check.sh" >"$TMP/bad.out" 2>&1; then
    cat "$TMP/bad.out" >&2
    fail "website-check.sh passed a data.js whose pi4 score is one point off"
fi
grep -q "glmark2.pi4" "$TMP/bad.out" || { cat "$TMP/bad.out" >&2; fail "the failure does not name glmark2.pi4"; }

echo "one-commit-rehearsal-selftest: ok (docs edit, record check and website check pass at $SHA8; one point off fails)"
