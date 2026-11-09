#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Plan #320's acceptance checks c1-c5 for the Pi 4 dEQP-GLES3 whole-tree
# record, in one command. Read-only: it runs the record's checkers and greps
# the result documents, it writes nothing.
#
#   tools/cts/pi4-gles3-acceptance.sh [--sha <full sha>] [--root <repo dir>]
#
# --sha    the one commit the whole tree is recorded at (default the plan's
#          b1b5b8456ec69247c64d1bd07e42461266dc5136).
# --root   the repository whose record is checked (default the repository
#          holding this script). The checkers themselves are always this
#          script's tools/; --root names the data.
#
# The criteria, as plan #320 states them:
#
#   c1  every '# image <LABEL>:' of tools/cts/pi4-gles3-tree.txt has
#       <LABEL>.tally.txt in a tools/cts/log/*-pi4-gles3-<sha8>-* directory,
#       and the '^commit' lines of those READMEs name only that sha:
#       check-pi4-gles3-log.py --commit <sha> over those directories.
#   c2  tools/cts/check-gles3-tree.py tools/cts/pi4-gles3-tree.txt ends
#       '<N> images, <M> matched once, 0 not matched, 0 matched more than
#       once, 0 empty images' with <M> the whole mustpass (42810 cases).
#   c3  python3 tools/release/redaction-scan.py HEAD 2>&1 | grep -c
#       pi4-gles3 prints 0 (the scan must have run: its report header), in a
#       --root that is a git repository of its own (not a directory inside one).
#   c4  head -3 docs/es3/pi4-gles3-fails.tsv names <sha8> and no
#       925dbca9/0ff58329, states 'images run (N of N)', and the whole file
#       names neither old sha.
#   c5  tools/cts/RESULTS.md, tools/cts/STATUS.md and ROADMAP.md each name
#       <sha8> in the Pi 4 ES 3.0 entry, and ROADMAP.md no longer holds
#       '9 of its 91 images at 925dbca9'.
#
# One line per criterion on stdout, 'c<N> PASS|FAIL <reason>'; tool output
# and detail go to stderr. Exit 0 only when all five pass, 2 on a usage
# error. The self-test: tools/cts/pi4-gles3-acceptance-selftest.sh.
set -u

usage() {
	echo "usage: $(basename "$0") [--sha <full sha>] [--root <repo dir>]" >&2
	exit 2
}

HERE=$(cd "$(dirname "$0")" && pwd)
SHA=b1b5b8456ec69247c64d1bd07e42461266dc5136
ROOT=$(cd "$HERE/../.." && pwd)
while [ $# -gt 0 ]; do
	case $1 in
	--sha)
		[ $# -ge 2 ] || usage
		SHA=$2
		shift 2
		;;
	--root)
		[ $# -ge 2 ] || usage
		ROOT=$2
		shift 2
		;;
	-h | --help)
		sed -n '4,/^set -u/p' "$0" | sed 's/^# \{0,1\}//; /^set -u/d'
		exit 0
		;;
	*) usage ;;
	esac
done
SHA=$(printf '%s' "$SHA" | tr 'A-F' 'a-f')
case $SHA in *[!0-9a-f]* | "") echo "pi4-gles3-acceptance: --sha needs a 40-digit hex sha, got '$SHA'" >&2; exit 2 ;; esac
[ ${#SHA} -eq 40 ] || { echo "pi4-gles3-acceptance: --sha needs a full 40-digit sha, got ${#SHA} digits" >&2; exit 2; }
[ -d "$ROOT" ] || { echo "pi4-gles3-acceptance: no such directory: $ROOT" >&2; exit 2; }
[ -f "$ROOT/tools/cts/pi4-gles3-tree.txt" ] ||
	{ echo "pi4-gles3-acceptance: no tools/cts/pi4-gles3-tree.txt under $ROOT" >&2; exit 2; }

SHA8=${SHA:0:8}
# The two earlier Pi 4 records plan #320 replaces (c4 and c5 name them).
OLD_SHA1=925dbca9
OLD_SHA2=0ff58329
TREE="$ROOT/tools/cts/pi4-gles3-tree.txt"
FAILS=0

line() { # line <n> <status> <reason...>
	printf 'c%s %s %s\n' "$1" "$2" "$3"
	[ "$2" = PASS ] || FAILS=$((FAILS + 1))
}

# --- c1: one complete tally per image, session READMEs on one commit --------
c1_dirs=()
shopt -s nullglob
for d in "$ROOT"/tools/cts/log/*-pi4-gles3-"$SHA8"-*; do
	[ -d "$d" ] && c1_dirs+=("$d")
done
shopt -u nullglob
if [ ${#c1_dirs[@]} -eq 0 ]; then
	line 1 FAIL "no session directory matches tools/cts/log/*-pi4-gles3-${SHA8}-*"
else
	c1_out=$(python3 -I "$HERE/check-pi4-gles3-log.py" --commit "$SHA" \
		--tree "$TREE" "${c1_dirs[@]}" 2>&1)
	c1_rc=$?
	if [ $c1_rc -eq 0 ]; then
		line 1 PASS "$(printf '%s\n' "$c1_out" | tail -1)"
	else
		c1_why=$(printf '%s\n' "$c1_out" | sed -n 's/^FAIL: //p' | head -1)
		[ -n "$c1_why" ] || c1_why=$(printf '%s\n' "$c1_out" | tail -1)
		printf '%s\n' "$c1_out" >&2
		line 1 FAIL "$c1_why"
	fi
fi

# --- c2: the table selects every mustpass case exactly once ------------------
c2_mustpass="$ROOT/tools/cts/gles3-mustpass.txt"
[ -f "$c2_mustpass" ] || c2_mustpass="$HERE/gles3-mustpass.txt"
c2_want=$(awk '{ gsub(/^[ \t]+|[ \t]+$/, "") } !/^[ \t]*(#|$)/ { n++ } END { print n + 0 }' \
	"$c2_mustpass")
c2_out=$(python3 -I "$HERE/check-gles3-tree.py" --mustpass "$c2_mustpass" "$TREE" 2>&1)
c2_rc=$?
c2_last=$(printf '%s\n' "$c2_out" | tail -1)
if printf '%s\n' "$c2_last" | grep -Eq \
	": [0-9]+ images, $c2_want matched once, 0 not matched, 0 matched more than once, 0 empty images$" &&
	[ $c2_rc -eq 0 ]; then
	line 2 PASS "$c2_last"
else
	printf '%s\n' "$c2_out" >&2
	if [ $c2_rc -ne 0 ]; then
		line 2 FAIL "check-gles3-tree.py exit $c2_rc: $c2_last"
	else
		line 2 FAIL "last line is not '<N> images, $c2_want matched once, 0 not matched, 0 matched more than once, 0 empty images': $c2_last"
	fi
fi

# --- c3: the redaction scan of HEAD names no pi4-gles3 file ------------------
# The scan reads the repository that holds $ROOT (git rev-parse), so a $ROOT
# that is not a repository of its own would have the scan run on whatever
# repository encloses it (a fixture under build/ or target/: this checkout)
# and report that HEAD. The check requires $ROOT to be its own top.
c3_top=$(cd "$ROOT" && git rev-parse --show-toplevel 2>/dev/null) || c3_top=
if [ "$c3_top" != "$(cd "$ROOT" && pwd -P)" ]; then
	c3_out="redaction-scan: not run: $ROOT is not a git repository of its own${c3_top:+ (git would read $c3_top)}"
else
	c3_out=$(cd "$ROOT" && python3 -I "$HERE/../release/redaction-scan.py" HEAD 2>&1)
fi
if ! printf '%s\n' "$c3_out" | grep -Eq '^redaction-scan: .*, [0-9]+ files '; then
	printf '%s\n' "$c3_out" >&2
	line 3 FAIL "redaction-scan.py HEAD did not run in $ROOT: $(printf '%s\n' "$c3_out" | head -1)"
else
	c3_n=$(printf '%s\n' "$c3_out" | grep -c 'pi4-gles3' || true)
	if [ "$c3_n" -eq 0 ]; then
		line 3 PASS "redaction-scan HEAD: 0 lines mention pi4-gles3"
	else
		printf '%s\n' "$c3_out" | grep 'pi4-gles3' >&2
		line 3 FAIL "redaction-scan HEAD prints $c3_n line(s) mentioning pi4-gles3"
	fi
fi

# --- c4: docs/es3/pi4-gles3-fails.tsv is the one-commit file -----------------
c4_verdict=$(python3 -I - "$ROOT" "$SHA8" "$OLD_SHA1" "$OLD_SHA2" <<'PYEOF'
import os
import re
import sys

root, sha8, olds = sys.argv[1], sys.argv[2], sys.argv[3:]
path = os.path.join(root, "docs", "es3", "pi4-gles3-fails.tsv")
if not os.path.isfile(path):
    print("FAIL docs/es3/pi4-gles3-fails.tsv does not exist")
    sys.exit()
text = open(path, encoding="utf-8", errors="replace").read()
head3 = "\n".join(text.splitlines()[:3])
probs = []
if sha8 not in head3:
    probs.append("head -3 does not name " + sha8)
for old in olds:
    if old in head3:
        probs.append("head -3 names the replaced commit " + old)
    elif old in text:
        probs.append("the file names the replaced commit " + old)
m = re.search(r"images run \((\d+) of (\d+)\)", head3)
if not m:
    probs.append("head -3 states no 'images run (N of M)'")
elif m.group(1) != m.group(2) or int(m.group(1)) == 0:
    probs.append("images run (%s of %s) is not every image run" % (m.group(1), m.group(2)))
print(("FAIL " if probs else "PASS ") +
      (": ".join(probs) if probs else "head -3 names %s only and every image run" % sha8))
PYEOF
)
case $c4_verdict in
"PASS "*) line 4 PASS "${c4_verdict#PASS }" ;;
"FAIL "*) line 4 FAIL "${c4_verdict#FAIL }" ;;
*) line 4 FAIL "the c4 check could not run" ;;
esac

# --- c5: the three result documents carry the one-commit record --------------
c5_verdict=$(python3 -I - "$ROOT" "$SHA8" "$OLD_SHA1" <<'PYEOF'
import os
import re
import sys

root, sha8, old = sys.argv[1], sys.argv[2], sys.argv[3]
probs = []


def read(rel):
    path = os.path.join(root, *rel.split("/"))
    try:
        return open(path, encoding="utf-8", errors="replace").read().splitlines()
    except OSError:
        return None


def until(lines, start, is_end):
    """lines[start:i] of one block, ending before the first is_end line."""
    i = start
    while i < len(lines) and not is_end(lines[i]):
        i += 1
    return lines[start:i]


# RESULTS.md: the paragraph starting '**Pi 4' inside the ES 3.0 section.
lines = read("tools/cts/RESULTS.md")
if lines is None:
    probs.append("tools/cts/RESULTS.md does not exist")
else:
    start = next((i + 1 for i, l in enumerate(lines)
                  if re.match(r"^#{1,3} .*ES 3\.0", l)), None)
    if start is None:
        probs.append("RESULTS.md has no ES 3.0 section")
    else:
        sec = until(lines, start, lambda l: re.match(r"^#{1,2} ", l))
        blocks = re.split(r"\n[ \t]*\n", "\n".join(sec))
        entries = [b for b in blocks if b.lstrip().startswith("**Pi 4")]
        if not entries:
            probs.append("RESULTS.md ES 3.0 section has no '**Pi 4' entry")
        elif not any(sha8 in b for b in entries):
            probs.append("RESULTS.md Pi 4 ES 3.0 entry does not name " + sha8)

# STATUS.md: the 'Pi 4' sentences of the '- **ES 3.0' bullet.
lines = read("tools/cts/STATUS.md")
if lines is None:
    probs.append("tools/cts/STATUS.md does not exist")
else:
    start = next((i for i, l in enumerate(lines) if l.startswith("- **ES 3.0")), None)
    if start is None:
        probs.append("STATUS.md has no ES 3.0 entry (bullet '- **ES 3.0')")
    else:
        bullet = [lines[start]] + until(lines, start + 1,
                                        lambda l: not l.strip() or l[:2] == "- "
                                        or l.startswith("#"))
        sentences = [s for s in re.split(r"[。\n]", "\n".join(bullet)) if "Pi 4" in s]
        if not sentences:
            probs.append("STATUS.md ES 3.0 entry has no Pi 4 sentence")
        elif not any(sha8 in s for s in sentences):
            probs.append("STATUS.md Pi 4 ES 3.0 entry does not name " + sha8)

# ROADMAP.md: the '- Raspberry Pi 4:' bullet; the old 9-of-91 claim is gone.
lines = read("ROADMAP.md")
if lines is None:
    probs.append("ROADMAP.md does not exist")
else:
    whole = "\n".join(lines)
    start = next((i for i, l in enumerate(lines)
                  if re.match(r"^-\s*(Raspberry Pi 4|Pi 4)[:：]", l)), None)
    if start is None:
        probs.append("ROADMAP.md has no 'Raspberry Pi 4' entry")
    else:
        entry = [lines[start]] + until(
            lines, start + 1,
            lambda l: not l.strip() or l[0] in "#-" or not l[:1].isspace())
        if not any(sha8 in l for l in entry):
            probs.append("ROADMAP.md Pi 4 entry does not name " + sha8)
    if re.search(r"9 of its 91 images at `?%s" % re.escape(old), whole):
        probs.append("ROADMAP.md still says '9 of its 91 images at %s'" % old)

print(("FAIL " if probs else "PASS ") +
      (": ".join(probs) if probs else
       "RESULTS.md, STATUS.md and ROADMAP.md name %s in the Pi 4 ES 3.0 entry" % sha8))
PYEOF
)
case $c5_verdict in
"PASS "*) line 5 PASS "${c5_verdict#PASS }" ;;
"FAIL "*) line 5 FAIL "${c5_verdict#FAIL }" ;;
*) line 5 FAIL "the c5 check could not run" ;;
esac

[ $FAILS -eq 0 ] || exit 1
exit 0
