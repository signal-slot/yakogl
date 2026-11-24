#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Plan #398's acceptance checks c1-c3 and c6 for the Pi 4 dEQP-GLES3 aborts2
# record (the session that re-splits the b1b5b845 aborts and runs the rest of
# the never-started cases), in one command. Read-only: it runs the record's
# checkers and reads the session directory, it writes nothing.
#
#   tools/cts/pi4-gles3-aborts2-acceptance.sh [--root <repo dir>]
#                                             [--session <dir>] [--selftests]
#
# --root        the repository whose record is checked (default the repository
#               holding this script). The tools are always this script's
#               tools/; --root names the data (its pi4-gles3-tree.txt,
#               gles3-mustpass.txt and tools/cts/log/).
# --session     check this session directory instead of looking for exactly
#               one tools/cts/log/2026-11-pi4-gles3-*-aborts2 (c2 then checks
#               the directory's contents, not the count).
# --selftests   run c6: pi4-gles3-preflight-selftest.sh,
#               pi4-session-tally-selftest.sh and cts-selftest.sh, each
#               expected to exit 0 (cts-selftest.sh can be slow, so without
#               this option c6 prints SKIP).
#
# The criteria, as plan #398 states them:
#
#   c1  python3 tools/cts/check-gles3-tree.py tools/cts/pi4-gles3-tree.txt
#       ends '<N> images, 42810 matched once, 0 not matched, 0 matched more
#       than once, 0 empty images' (42810 = the mustpass list: the number is
#       read from the repository's gles3-mustpass.txt) and N is the image
#       count the table header names ('in 114 images').
#   c2  exactly one tools/cts/log/2026-11-pi4-gles3-*-aborts2/ directory
#       exists (or --session names one). It holds README.md, table.txt and
#       cycles.txt, plus <LABEL>.console.txt and <LABEL>.tally.txt for each
#       label table.txt names. A label is a token of table.txt that is an
#       '# image <LABEL>:' name of pi4-gles3-tree.txt; the record's title
#       line must therefore name every image of the session (a range like
#       'S03a1..S03a4' names only its ends). A table.txt naming no label is
#       a defect, not a pass. The sha of the README.md 'commit:' line is an
#       ancestor of origin/main (git merge-base --is-ancestor); when the
#       repository has no origin/main (or no git) c2 says SKIP, not PASS.
#   c3  table.txt's TOTAL row's last column (the 'none' column) is 0: every
#       mustpass case in the record's scope has a result.
#   c6  tools/cts/pi4-gles3-preflight-selftest.sh,
#       tools/cts/pi4-session-tally-selftest.sh and tools/cts/cts-selftest.sh
#       exit 0 (behind --selftests).
#
# One line per criterion on stdout, 'c<N> PASS|FAIL|SKIP <reason>'; tool
# output and detail go to stderr. SKIP is not a failure. Exit 0 when no
# criterion fails, 1 when one fails, 2 on a usage error. The self-test:
# tools/cts/pi4-gles3-aborts2-acceptance-selftest.sh.
set -u

usage() {
	echo "usage: $(basename "$0") [--root <repo dir>] [--session <dir>] [--selftests]" >&2
	exit 2
}

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
SESSION=""
RUN_SELFT=0
while [ $# -gt 0 ]; do
	case $1 in
	--root)
		[ $# -ge 2 ] || usage
		ROOT=$2
		shift 2
		;;
	--session)
		[ $# -ge 2 ] || usage
		SESSION=$2
		shift 2
		;;
	--selftests)
		RUN_SELFT=1
		shift
		;;
	-h | --help)
		sed -n '4,/^set -u/p' "$0" | sed 's/^# \{0,1\}//; /^set -u/d'
		exit 0
		;;
	*) usage ;;
	esac
done
[ -d "$ROOT" ] || { echo "pi4-gles3-aborts2-acceptance: no such directory: $ROOT" >&2; exit 2; }
TREE="$ROOT/tools/cts/pi4-gles3-tree.txt"
[ -f "$TREE" ] ||
	{ echo "pi4-gles3-aborts2-acceptance: no tools/cts/pi4-gles3-tree.txt under $ROOT" >&2; exit 2; }

FAILS=0

line() { # line <n> <status> <reason...>
	printf 'c%s %s %s\n' "$1" "$2" "$3"
	[ "$2" != FAIL ] || FAILS=$((FAILS + 1))
}

# --- the session directory both c2 and c3 read -------------------------------
if [ -n "$SESSION" ]; then
	sds=("$SESSION")
else
	shopt -s nullglob
	sds=("$ROOT"/tools/cts/log/2026-11-pi4-gles3-*-aborts2)
	shopt -u nullglob
fi
SDIR=""
no_session=""
if [ -n "$SESSION" ]; then
	if [ -d "$SESSION" ]; then
		SDIR=$SESSION
	else
		no_session="no such session directory: $SESSION"
	fi
elif [ ${#sds[@]} -eq 1 ]; then
	SDIR=${sds[0]}
elif [ ${#sds[@]} -eq 0 ]; then
	no_session="no aborts2 session: no directory matches tools/cts/log/2026-11-pi4-gles3-*-aborts2/ (the re-split-and-rerun session of plan #398 is not recorded here)"
else
	no_session="${#sds[@]} session directories match tools/cts/log/2026-11-pi4-gles3-*-aborts2/ (the plan records exactly one): $(printf '%s ' "${sds[@]}")"
fi

# --- c1: the table selects every mustpass case once, header and checker agree -
c1_mustpass="$ROOT/tools/cts/gles3-mustpass.txt"
[ -f "$c1_mustpass" ] || c1_mustpass="$HERE/gles3-mustpass.txt"
[ -f "$c1_mustpass" ] ||
	{ echo "pi4-gles3-aborts2-acceptance: no gles3-mustpass.txt under $ROOT or beside this script" >&2; exit 2; }
c1_want=$(awk '{ gsub(/^[ \t]+|[ \t]+$/, "") } !/^[ \t]*(#|$)/ { n++ } END { print n + 0 }' \
	"$c1_mustpass")
c1_out=$(python3 -I "$HERE/check-gles3-tree.py" --mustpass "$c1_mustpass" "$TREE" 2>&1)
c1_rc=$?
c1_last=$(printf '%s\n' "$c1_out" | tail -1)
if printf '%s\n' "$c1_last" | grep -Eq \
	": [0-9]+ images, $c1_want matched once, 0 not matched, 0 matched more than once, 0 empty images$" &&
	[ $c1_rc -eq 0 ]; then
	c1_n=$(printf '%s\n' "$c1_last" | sed -n 's/.*: \([0-9][0-9]*\) images,.*/\1/p')
	# the image count of the table header: the first '<N> images' of its '#'
	# lines ('in 114 images')
	c1_hdr=$(grep -E '^#' "$TREE" | grep -m1 -oE '[0-9]+ images' | awk '{ print $1; exit }')
	if [ -z "$c1_hdr" ]; then
		line 1 FAIL "the checker says $c1_n images but the table header names no '<N> images' count: $c1_last"
	elif [ "$c1_hdr" != "$c1_n" ]; then
		line 1 FAIL "the checker reports $c1_n images, the table header names $c1_hdr"
	else
		line 1 PASS "$c1_n images (as the table header names), $c1_want matched once, 0 not matched, 0 matched more than once, 0 empty images"
	fi
else
	printf '%s\n' "$c1_out" >&2
	if [ $c1_rc -ne 0 ]; then
		line 1 FAIL "check-gles3-tree.py exit $c1_rc: $c1_last"
	else
		line 1 FAIL "last line is not '<N> images, $c1_want matched once, 0 not matched, 0 matched more than once, 0 empty images': $c1_last"
	fi
fi

# --- c2: the aborts2 session directory, complete, on origin/main --------------
if [ -n "$no_session" ]; then
	line 2 FAIL "$no_session"
else
	c2_verdict=$(python3 -I - "$SDIR" "$ROOT" "$TREE" <<'PYEOF'
import os
import re
import subprocess
import sys

session, root, tree = sys.argv[1], sys.argv[2], sys.argv[3]
problems = []
for f in ("README.md", "table.txt", "cycles.txt"):
    if not os.path.isfile(os.path.join(session, f)):
        problems.append("no %s in %s" % (f, session))

labels = set()
with open(tree, encoding="utf-8", errors="replace") as f:
    for m in re.finditer(r"^#\s*image\s+([^\s:]+)\s*:", f.read(), re.M):
        labels.add(m.group(1))

names = []
tpath = os.path.join(session, "table.txt")
if os.path.isfile(tpath):
    with open(tpath, encoding="utf-8", errors="replace") as f:
        text = f.read()
    tokens = set(re.findall(r"(?<![A-Za-z0-9])([A-Za-z0-9]+)(?![A-Za-z0-9])", text))
    names = sorted(t for t in tokens if t in labels)
    if not names:
        problems.append("table.txt names none of pi4-gles3-tree.txt's image labels "
                        "(its title line must name every image of the session)")
    for lab in names:
        for ext in (".console.txt", ".tally.txt"):
            if not os.path.isfile(os.path.join(session, lab + ext)):
                problems.append("%s%s missing (table.txt names %s)" % (lab, ext, lab))

# the README's commit line, and its ancestry in the repository of --root
def git(*args):
    """True/False for the git exit status, None when git or the object cannot
    be asked at all (no git binary, not a repository): ancestry is then not
    checked (SKIP), never passed over."""
    try:
        return subprocess.run(("git", "-C", root) + args,
                              stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL).returncode == 0
    except OSError:
        return None


skip_ancestor = True
shas = []
rpath = os.path.join(session, "README.md")
if os.path.isfile(rpath):
    with open(rpath, encoding="utf-8", errors="replace") as f:
        shas = re.findall(r"^commit:?[ \t]+([0-9a-f]{7,40})\b", f.read(), re.M | re.I)
    if not shas:
        problems.append("README.md has no 'commit: <sha>' line")
else:
    shas = None
if shas:
    origin = git("rev-parse", "--verify", "-q", "origin/main")
    for sh in sorted(set(shas)):
        if git("rev-parse", "--verify", "-q", sh + "^{commit}") is False:
            problems.append("README.md commit %s is not a commit of %s" % (sh, root))
            skip_ancestor = False
        elif origin is True:
            if git("merge-base", "--is-ancestor", sh, "origin/main") is False:
                problems.append("README.md commit %s is not an ancestor of origin/main" % sh[:8])
                skip_ancestor = False
            else:
                skip_ancestor = False
        # origin is False or None: origin/main (or git) is missing, this sha
        # stays unchecked (SKIP, never PASS)

if problems:
    print("FAIL " + "; ".join(problems))
elif skip_ancestor:
    print("SKIP %d images named in table.txt have their console and tally files; "
          "the README.md commit's ancestry of origin/main was not checked "
          "(origin/main is missing)" % len(names))
else:
    print("PASS %d images named in table.txt have their console and tally files; "
          "README.md commit %s is an ancestor of origin/main"
          % (len(names), sorted(set(shas))[0][:8]))
PYEOF
)
	case $c2_verdict in
	"PASS "*) line 2 PASS "${c2_verdict#PASS }" ;;
	"SKIP "*) line 2 SKIP "${c2_verdict#SKIP }" ;;
	"FAIL "*) line 2 FAIL "${c2_verdict#FAIL }" ;;
	*) line 2 FAIL "the c2 check could not run" ;;
	esac
fi

# --- c3: the session's TOTAL row has no case without a result ------------------
if [ -n "$no_session" ]; then
	line 3 FAIL "no table.txt to read: $no_session"
else
	c3_verdict=$(python3 -I - "$SDIR" <<'PYEOF'
import os
import sys

session = sys.argv[1]
tpath = os.path.join(session, "table.txt")
if not os.path.isfile(tpath):
    print("FAIL no table.txt in %s (see c2)" % session)
    sys.exit()
with open(tpath, encoding="utf-8", errors="replace") as f:
    rows = [l for l in f.read().splitlines() if l.split() and l.split()[0] == "TOTAL"]
if not rows:
    print("FAIL table.txt holds no row starting 'TOTAL'")
elif len(rows[-1].split()) < 2:
    print("FAIL the TOTAL row has no column after its name: %s" % rows[-1])
elif rows[-1].split()[-1] != "0":
    print("FAIL the TOTAL row's last column (none) is %s, not 0: %s"
          % (rows[-1].split()[-1], rows[-1]))
else:
    print("PASS the TOTAL row's last column (none) is 0: every case in the record's scope has a result")
PYEOF
)
	case $c3_verdict in
	"PASS "*) line 3 PASS "${c3_verdict#PASS }" ;;
	"FAIL "*) line 3 FAIL "${c3_verdict#FAIL }" ;;
	*) line 3 FAIL "the c3 check could not run" ;;
	esac
fi

# --- c6: the three self-tests, behind --selftests (cts-selftest.sh is slow) ----
if [ $RUN_SELFT -eq 0 ]; then
	line 6 SKIP "not run: pass --selftests for pi4-gles3-preflight-selftest.sh, pi4-session-tally-selftest.sh and cts-selftest.sh"
else
	c6_bad=""
	for s in pi4-gles3-preflight-selftest.sh pi4-session-tally-selftest.sh cts-selftest.sh; do
		if ! [ -f "$HERE/$s" ]; then
			c6_bad="$c6_bad $s (no such file)"
			continue
		fi
		c6_out=$(bash "$HERE/$s" 2>&1)
		c6_rc=$?
		if [ $c6_rc -ne 0 ]; then
			printf '%s\n' "$c6_out" >&2
			c6_bad="$c6_bad $s (exit $c6_rc)"
		fi
	done
	if [ -z "$c6_bad" ]; then
		line 6 PASS "pi4-gles3-preflight-selftest.sh, pi4-session-tally-selftest.sh and cts-selftest.sh exit 0"
	else
		line 6 FAIL "self-test(s) did not exit 0:${c6_bad# }"
	fi
fi

[ $FAILS -eq 0 ] || exit 1
exit 0
