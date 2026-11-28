#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The six acceptance checks of the glmark2 one-commit plan (ROADMAP stage 2,
# #1196), in one command, on any tree. Read-only.
#
#   tools/cts/glmark2-one-commit-acceptance.sh [--root <repo dir>] [--ref <rev>] [--skip-website]
#
# --root          the tree to check (default: the repository holding this
#                 script). Files read, relative to it: README.md, ROADMAP.md,
#                 tools/cts/RESULTS.md, tools/cts/STATUS.md,
#                 docs/release/checklist.md, website/data.js.
# --ref           the revision the README commit must be an ancestor of
#                 (default origin/main; 'none' skips the ancestor test).
# --skip-website  c2 does not run tools/website-check.sh (a fake tree has no
#                 real website); the data.js-equals-README comparison still runs.
#
# README commit: the first backticked hash of the README's glmark2 table
# ('| Board | Score |'), as tools/cts/readme-glmark2-check.py reads it (its
# parser is imported). The criteria:
#
#   c1  readme-glmark2-check.py --ref <ref> --scenes terrain,shadow,refract
#   c2  tools/website-check.sh exits 0, and website/data.js glmark2.<board>
#       value, commit and date equal the README table's, for the three boards
#   c3  the checklist line starting '| 9 ' lacks 'left to #1196'; the ROADMAP
#       stage-2 section names the README commit and #17 and does not say the
#       Pi 4 pictures are checked by value only (checker --checklist --roadmap)
#   c4  a STATUS.md line that mentions glmark2 names the README commit
#   c5  readme-glmark2-check.py --ref <ref> --prose
#   c6  README.md, RESULTS.md and STATUS.md each have an i.MX8MP glmark2 text
#       (a table row, or a blank-line separated paragraph mentioning glmark2)
#       naming the README commit and 'Armadillo-X2'
#
# One line per check on stdout, 'c<N> PASS|FAIL <reason>'; detail on stderr.
# Exit 0 when all pass, 1 when one fails, 2 on a usage error. Self-test:
# tools/cts/glmark2-one-commit-acceptance-selftest.sh.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
REF=origin/main
SKIPWEB=0
usage() { echo "usage: $(basename "$0") [--root <dir>] [--ref <rev>] [--skip-website]" >&2; exit 2; }
while [ $# -gt 0 ]; do
	case $1 in
	--root) [ $# -ge 2 ] || usage; ROOT=$2; shift 2 ;;
	--ref) [ $# -ge 2 ] || usage; REF=$2; shift 2 ;;
	--skip-website) SKIPWEB=1; shift ;;
	-h | --help) sed -n '4,/^set -u/p' "$0" | sed 's/^# \{0,1\}//; /^set -u/d'; exit 0 ;;
	*) usage ;;
	esac
done
[ -d "$ROOT" ] || { echo "glmark2-one-commit-acceptance: no such directory: $ROOT" >&2; exit 2; }
ROOT=$(cd "$ROOT" && pwd)

CHK=$ROOT/tools/cts/readme-glmark2-check.py
[ -f "$CHK" ] || CHK=$HERE/readme-glmark2-check.py
README=$ROOT/README.md
RESULTS=$ROOT/tools/cts/RESULTS.md
STATUS=$ROOT/tools/cts/STATUS.md
CHECKLIST=$ROOT/docs/release/checklist.md
ROADMAP=$ROOT/ROADMAP.md
DATAJS=$ROOT/website/data.js
fail=0

# report <id> <rc> <ok text> <detail file>: one stdout line, detail to stderr
report() {
	local id=$1 rc=$2 ok=$3 f=$4 first
	if [ "$rc" -eq 0 ]; then
		echo "$id PASS $ok"
	else
		first=$(grep -m1 . "$f" 2>/dev/null | cut -c1-200)
		echo "$id FAIL ${first:-failed}"
		sed 's/^/    /' "$f" >&2
		fail=1
	fi
}

TMP=$(mktemp -d "${TMPDIR:-/tmp}/glmark2-one-commit.XXXXXX") || exit 2
trap 'rm -rf "$TMP"' EXIT
O=$TMP/out

checker() { python3 -I "$CHK" --readme "$README" --results "$RESULTS" --ref "$REF" \
	--status-file "$STATUS" --checklist-file "$CHECKLIST" --roadmap-file "$ROADMAP" "$@"; }

# the helper for c2 (comparison), c4 and c6: python, reusing the checker's parser
helper() { python3 -I - "$CHK" "$README" "$RESULTS" "$STATUS" "$DATAJS" "$@" <<'PY'
import importlib.util, json, re, sys
chk, readme, results, status, datajs, mode = sys.argv[1:7]
spec = importlib.util.spec_from_file_location("rgc", chk)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

def commit():
    try:
        rows = m.table_rows(readme, r"\|\s*Board\s*\|.*Score")
    except SystemExit:
        return None, []
    hs = m.board_hashes(rows)
    cs = []
    for b in m.WANT:
        h = hs.get(b)
        if h and not any(m.same(h[0], c) for c in cs):
            cs.append(h[0])
    return (cs[0] if len(cs) == 1 else None), rows

c, rows = commit()
if c is None:
    print("no single commit in the README glmark2 table")
    sys.exit(1)
c8 = c[:8]
def read(p):
    try:
        return open(p, encoding="utf-8").read()
    except OSError as e:
        print("%s: %s" % (p, e.strerror)); sys.exit(1)
fails = []
if mode == "data":
    try:
        t = read(datajs)
        figs = json.loads(t[t.index("{", t.index("=")):].rstrip().rstrip(";"))["figures"]
    except (ValueError, KeyError) as e:
        print("website/data.js does not parse: %s" % e); sys.exit(1)
    ids = {"AM62P": "am62p", "Raspberry Pi 4": "pi4", "i.MX8MP": "imx8mp"}
    for row in rows:
        b = m.BOARDS.get(m.norm(row[0]))
        if not b:
            continue
        cell = " ".join(row[1:]).replace("*", "")
        n = re.search(r"\d+(?:\.\d+)?", cell)
        h = m.HASH.search(cell)
        d = re.search(r"\d{4}-\d{2}-\d{2}", cell)
        f = figs.get("glmark2." + ids[b])
        if f is None:
            fails.append("data.js has no glmark2.%s" % ids[b]); continue
        if not n or float(n.group(0)) != float(f.get("value", -1)):
            fails.append("data.js glmark2.%s value %s, README %s" % (ids[b], f.get("value"), n and n.group(0)))
        if not h or not m.same(h.group(1), str(f.get("commit", ""))):
            fails.append("data.js glmark2.%s commit %s, README %s" % (ids[b], f.get("commit"), h and h.group(1)))
        if not d or d.group(0) != f.get("date"):
            fails.append("data.js glmark2.%s date %s, README %s" % (ids[b], f.get("date"), d and d.group(0)))
elif mode == "commit":
    pass
elif mode == "status":
    ok = [l for l in read(status).splitlines() if re.search("glmark2", l, re.I) and c8 in l]
    if not ok:
        fails.append("no STATUS.md line mentions glmark2 and names `%s`" % c8)
elif mode == "x2":
    x2 = re.compile(r"armadillo[- ]?x2", re.I)
    imx = re.compile(r"i\.?mx8mp", re.I)
    for name, p in (("README.md", readme), ("RESULTS.md", results), ("STATUS.md", status)):
        text = read(p)
        units = []
        for para in re.split(r"\n\s*\n", text):
            lines = para.splitlines()
            if lines and lines[0].startswith("|"):
                units += [("row", l) for l in lines if l.startswith("|")]
            else:
                units.append(("para", para))
        good = False
        for kind, u in units:
            if c8 not in u.replace("`", "") or not x2.search(u) or not imx.search(u):
                continue
            if kind == "row":
                first = u.strip("|").split("|")[0]
                good = bool(imx.search(first))
            else:
                good = bool(re.search("glmark2", u, re.I))
            if good:
                break
        if not good:
            fails.append("%s has no i.MX8MP glmark2 text naming `%s` and Armadillo-X2" % (name, c8))
for f in fails:
    print(f)
sys.exit(1 if fails else 0)
PY
}

checker --scenes terrain,shadow,refract >"$O" 2>&1; report c1 $? "one commit, scenes and record dirs" "$O"

(
	rc=0
	if [ "$SKIPWEB" -eq 1 ]; then
		:
	elif [ -f "$ROOT/tools/website-check.sh" ]; then
		w=$(bash "$ROOT/tools/website-check.sh" 2>&1) || { printf 'website-check failed: %s\n' "$w"; rc=1; }
	else
		echo "no tools/website-check.sh in $ROOT"; rc=1
	fi
	helper data || rc=1
	exit $rc
) >"$O" 2>&1
rc2=$?
note="data.js equals README"; [ "$SKIPWEB" -eq 1 ] || note="website-check ok, $note"
report c2 $rc2 "$note" "$O"

checker --checklist --roadmap >"$O" 2>&1
# --checklist --roadmap also re-run the table checks; only the STATUS/README-
# independent failures are c3's
grep -E '^FAIL: (CHECKLIST|ROADMAP)|^error' "$O" >"$O.3"
helper commit >>"$O.3" 2>&1
rc3=0; [ -s "$O.3" ] && rc3=1
report c3 $rc3 "checklist line 9 and ROADMAP stage 2 name the README commit" "$O.3"

helper status >"$O" 2>&1; report c4 $? "STATUS.md names the README commit" "$O"
checker --prose >"$O" 2>&1; report c5 $? "README prose" "$O"
helper x2 >"$O" 2>&1; report c6 $? "i.MX8MP glmark2 text names the commit and Armadillo-X2 in README, RESULTS, STATUS" "$O"

exit $fail
