#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Plan #400's acceptance checks c1-c4 (and c5, informational) for the AM62P
# dEQP-GLES3 whole-tree record, in one command. Read-only: it runs the tree
# generator's checks and reads the result documents, it writes nothing.
#
#   tools/cts/am62p-gles3-acceptance.sh [--sha <sha>] [--root <repo dir>] [--base <rev>]
#
# --sha    the commit the whole tree must be recorded at (7 to 40 hex digits;
#          a header build matches when one is a prefix of the other). Without
#          it c1 accepts any single build and c4 looks for that build.
# --root   the repository whose record is checked (default the repository
#          holding this script). c3 runs --root's own generator, which reads
#          the table and mustpass list next to it.
# --base   the revision c5 diffs against (default origin/main, else HEAD).
#
# The criteria:
#
#   c1  the '# rows <a>-<b>: build <sha8>, run <date>' header lines of
#       docs/es3/am62p-gles3-notpass.tsv (gles3-notpass.py --splice) name
#       exactly one build, and their ranges cover rows 0..N-1 with N the
#       image count of tools/cts/am62p-gles3-tree.txt (no row twice). A row
#       end may carry a re-split suffix (22a-22d): it counts for its row.
#       With --sha, that one build is --sha.
#   c2  no line of the TSV starts with 'NoResult'.
#   c3  python3 tools/cts/gen-am62p-gles3-tree.py --check and --self-test
#       both exit 0.
#   c4  docs/notes/pvr-3d-mipmap-hang.md has an outcome section (a heading
#       naming 'Outcome') that names the build's sha8 and, for each of rows
#       22-38 (or its re-split rows 22a, 22b, ...), a line saying resets 0:
#       a table row '| <row> | ... |' whose 'resets' column is 0, or a line
#       'row <row> ... resets 0' / '... 0 resets'. No line of a row may give
#       a non-zero reset count.
#   c5  (informational, not in the exit status) git diff --quiet <base> --
#       tools/cts/RESULTS.md tools/cts/STATUS.md ROADMAP.md in --root.
#
# One line per criterion on stdout, 'c<N> PASS|FAIL <reason>'; detail goes to
# stderr. Exit 0 only when c1-c4 pass, 1 when one fails, 2 on a usage error.
# The self-test: tools/cts/am62p-gles3-acceptance-selftest.sh.
set -u

usage() {
	echo "usage: $(basename "$0") [--sha <sha>] [--root <repo dir>] [--base <rev>]" >&2
	exit 2
}

HERE=$(cd "$(dirname "$0")" && pwd)
SHA=
BASE=
ROOT=$(cd "$HERE/../.." && pwd)
while [ $# -gt 0 ]; do
	case $1 in
	--sha | --root | --base)
		[ $# -ge 2 ] || usage
		case $1 in
		--sha) SHA=$2 ;;
		--root) ROOT=$2 ;;
		--base) BASE=$2 ;;
		esac
		shift 2
		;;
	-h | --help)
		sed -n '4,/^set -u/p' "$0" | sed 's/^# \{0,1\}//; /^set -u/d'
		exit 0
		;;
	*) usage ;;
	esac
done
if [ -n "$SHA" ]; then
	SHA=$(printf '%s' "$SHA" | tr 'A-F' 'a-f')
	case $SHA in *[!0-9a-f]*) echo "am62p-gles3-acceptance: --sha needs hex digits, got '$SHA'" >&2; exit 2 ;; esac
	[ ${#SHA} -ge 7 ] && [ ${#SHA} -le 40 ] ||
		{ echo "am62p-gles3-acceptance: --sha needs 7 to 40 hex digits, got ${#SHA}" >&2; exit 2; }
fi
[ -d "$ROOT" ] || { echo "am62p-gles3-acceptance: no such directory: $ROOT" >&2; exit 2; }
ROOT=$(cd "$ROOT" && pwd)

TSV="$ROOT/docs/es3/am62p-gles3-notpass.tsv"
TREE="$ROOT/tools/cts/am62p-gles3-tree.txt"
GEN="$ROOT/tools/cts/gen-am62p-gles3-tree.py"
NOTE="$ROOT/docs/notes/pvr-3d-mipmap-hang.md"
FAILS=0

line() { # line <n> <status> <reason...>
	printf 'c%s %s %s\n' "$1" "$2" "$3"
	[ "$2" = PASS ] || FAILS=$((FAILS + 1))
}

verdict() { # verdict <n> <PASS ...|FAIL ...|other>
	case $2 in
	"PASS "*) line "$1" PASS "${2#PASS }" ;;
	"FAIL "*) line "$1" FAIL "${2#FAIL }" ;;
	*) line "$1" FAIL "the c$1 check could not run" ;;
	esac
}

# --- c1: one build, every tree row covered once ------------------------------
# Prints 'PASS|FAIL <reason>' on the first line and the one build (or '-') on
# the second.
c1_out=$(python3 -I - "$TSV" "$TREE" "$SHA" <<'PYEOF'
import re
import sys

tsv, tree, want = sys.argv[1], sys.argv[2], sys.argv[3]
RANGE = re.compile(r"^# rows (\d+)([a-z]*)-(\d+)([a-z]*): build ([0-9a-fA-F]+), run (\S+)\s*$")


def out(ok, why, build="-"):
    print(("PASS " if ok else "FAIL ") + why)
    print(build)
    sys.exit()


try:
    lines = open(tsv, encoding="utf-8", errors="replace").read().splitlines()
except OSError:
    out(False, "docs/es3/am62p-gles3-notpass.tsv does not exist")
try:
    n = sum(1 for l in open(tree, encoding="utf-8", errors="replace")
            if l.strip() and not l.lstrip().startswith("#"))
except OSError:
    out(False, "tools/cts/am62p-gles3-tree.txt does not exist")
ranges = []
for l in lines:
    m = RANGE.match(l)
    if m:
        ranges.append((int(m.group(1)), int(m.group(3)), m.group(5).lower()))
    elif l.startswith("# rows "):
        print("not a range header: " + l, file=sys.stderr)
if not ranges:
    out(False, "the TSV has no '# rows <a>-<b>: build <sha8>, run <date>' header line")
builds = sorted({b for _, _, b in ranges})
probs = []
if len(builds) != 1:
    probs.append("the header names %d builds (%s), not one" % (len(builds), ", ".join(builds)))
count = {}
for lo, hi, b in ranges:
    if hi < lo:
        probs.append("range %d-%d runs backwards" % (lo, hi))
    for r in range(lo, hi + 1):
        count[r] = count.get(r, 0) + 1
missing = [r for r in range(n) if r not in count]
extra = sorted(r for r in count if r >= n)
twice = sorted(r for r in count if count[r] > 1 and r < n)
if missing:
    probs.append("rows %s of 0-%d have no header range" % (",".join(map(str, missing)), n - 1))
if extra:
    probs.append("rows %s are beyond the tree's %d rows" % (",".join(map(str, extra)), n))
if twice:
    # a re-split row is named once per part (22a-22a, 22b-22b): allowed only
    # when every part is of the same build, which the one-build rule checks.
    suffixed = {int(m.group(1)) for m in map(RANGE.match, lines) if m and m.group(2)}
    bad = [r for r in twice if r not in suffixed]
    if bad:
        probs.append("rows %s are named by more than one range" % ",".join(map(str, bad)))
if want and len(builds) == 1 and not (want.startswith(builds[0]) or builds[0].startswith(want)):
    probs.append("the build is %s, not %s" % (builds[0], want[:8]))
if probs:
    out(False, "; ".join(probs), builds[0] if len(builds) == 1 else "-")
out(True, "rows 0-%d of %d tree images come from build %s" % (n - 1, n, builds[0]), builds[0])
PYEOF
)
c1_build=$(printf '%s\n' "$c1_out" | sed -n 2p)
verdict 1 "$(printf '%s\n' "$c1_out" | sed -n 1p)"

# --- c2: no NoResult line ----------------------------------------------------
if [ ! -f "$TSV" ]; then
	line 2 FAIL "docs/es3/am62p-gles3-notpass.tsv does not exist"
else
	c2_n=$(grep -c '^NoResult' "$TSV")
	if [ "$c2_n" -eq 0 ]; then
		line 2 PASS "no NoResult line in docs/es3/am62p-gles3-notpass.tsv"
	else
		grep '^NoResult' "$TSV" | head -5 >&2
		line 2 FAIL "$c2_n NoResult line(s) in docs/es3/am62p-gles3-notpass.tsv"
	fi
fi

# --- c3: the generator's --check and --self-test -----------------------------
if [ ! -f "$GEN" ]; then
	line 3 FAIL "tools/cts/gen-am62p-gles3-tree.py does not exist"
else
	c3_chk=$(python3 -I "$GEN" --check 2>&1)
	c3_chk_rc=$?
	c3_st=$(python3 -I "$GEN" --self-test 2>&1)
	c3_st_rc=$?
	if [ $c3_chk_rc -eq 0 ] && [ $c3_st_rc -eq 0 ]; then
		line 3 PASS "--check: $(printf '%s\n' "$c3_chk" | tail -1); --self-test: $(printf '%s\n' "$c3_st" | tail -1)"
	else
		printf '%s\n' "$c3_chk" "$c3_st" >&2
		c3_why=
		[ $c3_chk_rc -eq 0 ] || c3_why="--check exit $c3_chk_rc: $(printf '%s\n' "$c3_chk" | tail -1)"
		[ $c3_st_rc -eq 0 ] || c3_why="${c3_why:+$c3_why; }--self-test exit $c3_st_rc: $(printf '%s\n' "$c3_st" | tail -1)"
		line 3 FAIL "$c3_why"
	fi
fi

# --- c4: the #1285 outcome section of the note -------------------------------
c4_sha=${SHA:0:8}
[ -n "$c4_sha" ] || { [ "$c1_build" != - ] && c4_sha=${c1_build:0:8}; }
c4_verdict=$(python3 -I - "$NOTE" "$c4_sha" <<'PYEOF'
import re
import sys

path, sha8 = sys.argv[1], sys.argv[2]
ROWS = range(22, 39)
try:
    lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
except OSError:
    print("FAIL docs/notes/pvr-3d-mipmap-hang.md does not exist")
    sys.exit()
if not sha8:
    print("FAIL no build to look for: c1 found no single build and --sha was not given")
    sys.exit()
start = next((i for i, l in enumerate(lines) if re.match(r"^#{1,6} .*\boutcome\b", l, re.I)), None)
if start is None:
    print("FAIL the note has no outcome section (a heading naming 'Outcome')")
    sys.exit()
level = len(lines[start]) - len(lines[start].lstrip("#"))
end = next((i for i in range(start + 1, len(lines))
            if re.match(r"^#{1,%d} " % level, lines[i])), len(lines))
sec = lines[start:end]
probs = []
if not any(re.search(r"\b%s" % re.escape(sha8[:7]), l.lower()) for l in sec):
    probs.append("the outcome section does not name build " + sha8)

ROW = r"(\d+)([a-z]*)"
resets = {}  # row -> [(label, count)]
col = None
for l in sec:
    if l.lstrip().startswith("|"):
        cells = [c.strip() for c in l.strip().strip("|").split("|")]
        low = [c.lower() for c in cells]
        if any("reset" in c for c in low) and not re.fullmatch(r"(row\s*)?" + ROW, low[0]):
            col = next(i for i, c in enumerate(low) if "reset" in c)
            continue
        m = re.fullmatch(r"(?:row\s*)?" + ROW, low[0].strip("`*"))
        if m and col is not None and col < len(cells):
            v = re.match(r"\d+", cells[col].strip("`* "))
            resets.setdefault(int(m.group(1)), []).append(
                (m.group(1) + m.group(2), int(v.group(0)) if v else None))
        continue
    col = None
    for m in re.finditer(r"\brows? " + ROW + r"\b", l, re.I):
        r = re.search(r"\bresets?\s*[:=]?\s*(\d+)|\b(\d+)\s+resets?\b", l[m.end():], re.I)
        if r:
            resets.setdefault(int(m.group(1)), []).append(
                (m.group(1) + m.group(2), int(r.group(1) or r.group(2))))
missing = [r for r in ROWS if r not in resets]
nonzero = ["%s (%s)" % (lab, "?" if v is None else v)
           for r in ROWS for lab, v in resets.get(r, []) if v != 0]
if missing:
    probs.append("rows %s have no resets entry" % ",".join(map(str, missing)))
if nonzero:
    probs.append("rows with resets not 0: " + ", ".join(nonzero))
print(("FAIL " if probs else "PASS ") + ("; ".join(probs) if probs else
      "the outcome section names %s and resets 0 for rows 22-38" % sha8))
PYEOF
)
verdict 4 "$c4_verdict"

# --- c5 (informational): the plan leaves RESULTS/STATUS/ROADMAP alone --------
if ! git -C "$ROOT" rev-parse --git-dir >/dev/null 2>&1; then
	printf 'c5 FAIL (informational) %s is not a git repository\n' "$ROOT"
else
	if [ -z "$BASE" ]; then
		BASE=origin/main
		git -C "$ROOT" rev-parse --verify -q "$BASE^{commit}" >/dev/null || BASE=HEAD
	fi
	if ! git -C "$ROOT" rev-parse --verify -q "$BASE^{commit}" >/dev/null; then
		printf 'c5 FAIL (informational) no such revision: %s\n' "$BASE"
	elif git -C "$ROOT" diff --quiet "$BASE" -- tools/cts/RESULTS.md tools/cts/STATUS.md ROADMAP.md; then
		printf 'c5 PASS (informational) RESULTS.md, STATUS.md and ROADMAP.md as at %s\n' "$BASE"
	else
		git -C "$ROOT" diff --stat "$BASE" -- tools/cts/RESULTS.md tools/cts/STATUS.md ROADMAP.md >&2
		printf 'c5 FAIL (informational) RESULTS.md, STATUS.md or ROADMAP.md differ from %s\n' "$BASE"
	fi
fi

[ $FAILS -eq 0 ] || exit 1
exit 0
