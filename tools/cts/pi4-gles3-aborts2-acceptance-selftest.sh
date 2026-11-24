#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Hermetic self-test of tools/cts/pi4-gles3-aborts2-acceptance.sh (plan #398's
# acceptance checks c1-c3 and c6): --root points at a copy of the fixture
# repository tools/cts/testdata/pi4-gles3-aborts2-acceptance/root/ made a git
# repository of its own, whose origin/main is a descendant of the commit its
# session README names (the placeholder 'commit:' sha is replaced with the
# first real commit, so the ancestry check runs for real).
#
#   tools/cts/pi4-gles3-aborts2-acceptance-selftest.sh
#
# One passing set, plus single-defect variants each failing exactly one
# criterion: a header count the checker disagrees with and a table that leaves
# a case unmatched (c1), no session directory, two session directories, a
# missing cycles.txt, a missing tally, a table.txt naming no label, a README
# commit that is no commit and one that is no ancestor of origin/main (c2),
# a TOTAL row whose last column is 1978 (c3), and the --selftests run against
# stub self-tests that pass and that fail (c6). --session names the directory
# and names a missing one. c6 of every non-stub run is SKIP. Nothing outside
# $TMPDIR is read or written; cts-selftest.sh (slow) is never run.
#
# Every git call runs in an environment of its own: no user, system or global
# configuration of the host (GIT_CONFIG_NOSYSTEM, a HOME of this test's), and
# safe.directory for every path (GIT_CONFIG_COUNT injection: the gate's
# runners are generic machines, and a scratch tree on an odd mount must read
# as trusted the same way here and there). The initial branch is named on
# git init, so no host default branch enters the record.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ACC="$HERE/pi4-gles3-aborts2-acceptance.sh"
TD="$HERE/testdata/pi4-gles3-aborts2-acceptance/root"
W=$(mktemp -d "${TMPDIR:-/tmp}/pi4-gles3-aborts2-acceptance-selftest.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
fails=0
SD=tools/cts/log/2026-11-pi4-gles3-1234abcd-aborts2

# the git environment of this test: fixed identity, no config of the host,
# every scratch path trusted (a file mode of the host's must not decide
# whether a fixture commit resolves)
GITENV=(env -i PATH="$PATH" GIT_CONFIG_NOSYSTEM=1
	GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory GIT_CONFIG_VALUE_0='*')

gitc() { # gitc <dir> <git args...>: a git step whose failure ends the test
	# (a half-built fixture would otherwise be checked as if it were whole)
	"${GITENV[@]}" HOME="$W/home" \
		GIT_AUTHOR_NAME=s GIT_AUTHOR_EMAIL=s@localhost \
		GIT_COMMITTER_NAME=s GIT_COMMITTER_EMAIL=s@localhost \
		git -c commit.gpgsign=false -C "$1" "${@:2}" >/dev/null ||
		{ echo "pi4-gles3-aborts2-acceptance-selftest: git ${*:2} failed in $1" >&2; exit 2; }
}
gitq() { # gitq <dir> <git args...>: same, stdout returned
	"${GITENV[@]}" HOME="$W/home" \
		GIT_AUTHOR_NAME=s GIT_AUTHOR_EMAIL=s@localhost \
		GIT_COMMITTER_NAME=s GIT_COMMITTER_EMAIL=s@localhost \
		git -c commit.gpgsign=false -C "$1" "${@:2}" 2>/dev/null
}

prepare() { # prepare <name>: fixture copy + git repo, README names commit A,
	# origin/main is commit B (a descendant), so c2's ancestor check passes.
	local dir=$W/$1 sha
	cp -a "$TD" "$dir" || exit 2
	gitc "$dir" init -q -b main
	gitc "$dir" add -A
	gitc "$dir" commit -q -m base
	sha=$(gitq "$dir" rev-parse HEAD)
	[ -n "$sha" ] || { echo "pi4-gles3-aborts2-acceptance-selftest: empty rev-parse in $dir" >&2; exit 2; }
	sed -i "s/^commit: .*/commit: $sha/" "$dir/$SD/README.md"
	gitc "$dir" add -A
	gitc "$dir" commit -q -m readme
	gitc "$dir" update-ref refs/remotes/origin/main HEAD
}

variant() { # variant <name> <mutation...>: prepare, mutate (may be empty)
	local v=$1
	shift
	prepare "$v"
	[ $# -eq 0 ] || "$@" "$W/$v"
}

run_acc() { # run_acc <name> [--script <script>] [extra acceptance args...]:
	# the acceptance script with a clean environment (no cts-selftest.sh run:
	# no variant passes --selftests at the real script); same git isolation
	# as above, so the c2 ancestry check reads the fixture and not the host
	local v=$1 script=$ACC
	shift
	if [ "${1:-}" = --script ]; then
		script=$2
		shift 2
	fi
	"${GITENV[@]}" HOME="$W" LANG=C LC_ALL=C bash "$script" --root "$W/$v" "$@" \
		> "$W/$v.out" 2> "$W/$v.err"
	echo "$?" > "$W/$v.rc"
}

check() { # check <name> <want rc> <want statuses of c1 c2 c3 c6> <substring of stdout or stderr>
	local v=$1 grc=$2 gstat=$3 sub=$4
	[ "$(awk 'END { print NR }' "$W/$v.out")" -eq 4 ] || { echo "  FAIL $v: not four stdout lines"; sed 's/^/       out /' "$W/$v.out"; fails=$((fails + 1)); return; }
	# the criteria are c1, c2, c3, c6 (plan numbering, not consecutive), so
	# the line's first column is checked as 'c<digit>', not as 'c<NR>'
	awk '$1 !~ /^c[0-9]+$/ || ($2 != "PASS" && $2 != "FAIL" && $2 != "SKIP") || NF < 3 { print "bad line " NR ": [" $0 "]"; exit 1 }' "$W/$v.out" || { echo "  FAIL $v: a line is not 'c<N> PASS|FAIL|SKIP <reason>'"; fails=$((fails + 1)); return; }
	local stat rc
	stat=$(awk '{ printf "%s ", $1 }' "$W/$v.out")$(awk '{ printf "%s ", $2 }' "$W/$v.out")
	rc=$(cat "$W/$v.rc")
	if [ "$stat" != "c1 c2 c3 c6 $gstat " ] || [ "$rc" != "$grc" ] || ! grep -Fq -- "$sub" "$W/$v.out" "$W/$v.err" 2>/dev/null; then
		echo "  FAIL $v: rc $rc want $grc, criteria+status '$stat' want 'c1 c2 c3 c6 $gstat ', '$sub' not printed"
		sed 's/^/       out /' "$W/$v.out"
		sed 's/^/       err /' "$W/$v.err" | head -5
		fails=$((fails + 1))
		return
	fi
	echo "  ok   $v: rc $rc, $gstat"
}

# m1: the table header counts three images, the checker reports two (c1)
m1() { sed -i 's/ in 2 images,/ in 3 images,/' "$1/tools/cts/pi4-gles3-tree.txt"; }
# m2: image B2 selects nothing, one case is matched by no image (c1)
m2() { sed -i 's/dEQP-GLES3\.info\.\*/dEQP-GLES3.info.none*/' "$1/tools/cts/pi4-gles3-tree.txt"; }
# m3: the session directory is gone (c2 and c3)
m3() { rm -rf "$1/$SD"; }
# m4: a second aborts2 session directory (c2)
m4() { cp -a "$1/$SD" "${1}/${SD%-*}-5678ef90-aborts2"; }
# m5: cycles.txt is gone (c2)
m5() { rm "$1/$SD/cycles.txt"; }
# m6: the tally of B2, a label of table.txt, is gone (c2)
m6() { rm "$1/$SD/B2.tally.txt"; }
# m7: table.txt's title names no image label (c2)
m7() { sed -i 's/, A1 B2 (2 images)/ (2 images)/' "$1/$SD/table.txt"; }
# m8: the README names a sha that is no commit of the repository (c2)
m8() { sed -i 's/^commit: .*/commit: ffffffffffffffffffffffffffffffffffffffff/' "$1/$SD/README.md"; }
# m9: the README names a real commit of a side branch, not an ancestor (c2)
m9() {
	local d=$1 sha
	gitc "$d" checkout -q -b side
	gitc "$d" commit -q --allow-empty -m side
	sha=$(gitq "$d" rev-parse HEAD)
	gitc "$d" checkout -q main
	sed -i "s/^commit: .*/commit: $sha/" "$d/$SD/README.md"
}
# m10: the TOTAL row ends in 1978, not 0 (c3)
m10() { sed -i 's/^TOTAL.*/TOTAL                               3      1      0      0      0   1978/' "$1/$SD/table.txt"; }
# m11: the origin/main ref is gone (c2 SKIP)
m11() { gitc "$1" update-ref -d refs/remotes/origin/main; }

mkdir -p "$W/home" || exit 2

# --- the passing record and its single-defect variants ------------------------
variant pass
run_acc pass
check pass 0 "PASS PASS PASS SKIP" "2 images (as the table header names), 4 matched once"

variant pass-session
run_acc pass-session --session "$W/pass-session/$SD"
check pass-session 0 "PASS PASS PASS SKIP" "is an ancestor of origin/main"

variant session-missing
run_acc session-missing --session "$W/session-missing/$SD-deleted"
check session-missing 1 "PASS FAIL FAIL SKIP" "no such session directory"

variant c1-header m1
run_acc c1-header
check c1-header 1 "FAIL PASS PASS SKIP" "the table header names 3"

variant c1-case-unmatched m2
run_acc c1-case-unmatched
check c1-case-unmatched 1 "FAIL PASS PASS SKIP" "1 not matched"

variant no-session m3
run_acc no-session
check no-session 1 "PASS FAIL FAIL SKIP" "no aborts2 session: no directory matches tools/cts/log/2026-11-pi4-gles3-*-aborts2/"

variant two-sessions m4
run_acc two-sessions
check two-sessions 1 "PASS FAIL FAIL SKIP" "2 session directories match"

variant cycles-missing m5
run_acc cycles-missing
check cycles-missing 1 "PASS FAIL PASS SKIP" "no cycles.txt"

variant tally-missing m6
run_acc tally-missing
check tally-missing 1 "PASS FAIL PASS SKIP" "B2.tally.txt missing"

variant table-no-labels m7
run_acc table-no-labels
check table-no-labels 1 "PASS FAIL PASS SKIP" "names none of pi4-gles3-tree.txt's image labels"

variant commit-unknown m8
run_acc commit-unknown
check commit-unknown 1 "PASS FAIL PASS SKIP" "is not a commit of"

variant commit-side-branch m9
run_acc commit-side-branch
check commit-side-branch 1 "PASS FAIL PASS SKIP" "is not an ancestor of origin/main"

variant total-nonzero m10
run_acc total-nonzero
check total-nonzero 1 "PASS PASS FAIL SKIP" "last column (none) is 1978, not 0"

variant no-origin m11
run_acc no-origin
check no-origin 0 "PASS SKIP PASS SKIP" "origin/main"

# --- c6 runs the self-tests of its own tools/ directory: stubs ---------------
# the acceptance script with a stub tools/ of its own (its check-gles3-tree.py
# next to it, the three self-tests as exit-<n> stubs), so --selftests is
# exercised without touching the real (slow) cts-selftest.sh
stub_bin() { # stub_bin <dir> <rc>: bin dir with the script, its checker and stubs
	mkdir -p "$1" || return 1
	cp "$ACC" "$HERE/check-gles3-tree.py" "$1/" || return 1
	local s rc=$2
	for s in pi4-gles3-preflight-selftest.sh pi4-session-tally-selftest.sh cts-selftest.sh; do
		printf '#!/bin/sh\necho "stub %s"\nexit %d\n' "$s" "$rc" > "$1/$s"
		chmod +x "$1/$s"
	done
}
variant c6-stub-pass
stub_bin "$W/bin-pass" 0
run_acc c6-stub-pass --script "$W/bin-pass/pi4-gles3-aborts2-acceptance.sh" --selftests
check c6-stub-pass 0 "PASS PASS PASS PASS" "exit 0"

variant c6-stub-fail
stub_bin "$W/bin-fail" 0
printf '#!/bin/sh\necho "stub fails"\nexit 3\n' > "$W/bin-fail/cts-selftest.sh"
chmod +x "$W/bin-fail/cts-selftest.sh"
run_acc c6-stub-fail --script "$W/bin-fail/pi4-gles3-aborts2-acceptance.sh" --selftests
check c6-stub-fail 1 "PASS PASS PASS FAIL" "cts-selftest.sh (exit 3)"

if [ $fails -eq 0 ]; then
	echo "pi4-gles3-aborts2-acceptance-selftest: all checks passed"
	exit 0
fi
echo "pi4-gles3-aborts2-acceptance-selftest: $fails check(s) FAILED"
exit 1
