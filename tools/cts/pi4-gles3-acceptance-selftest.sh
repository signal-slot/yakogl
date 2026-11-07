#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Hermetic self-test of tools/cts/pi4-gles3-acceptance.sh (plan #320's
# acceptance checks c1-c5): --root points at a scratch git tree holding a
# synthetic two-image record - a table with its own mustpass, one session
# directory with a README commit line and two complete tallies, a one-commit
# docs/es3/pi4-gles3-fails.tsv and the three result documents.
#
#   tools/cts/pi4-gles3-acceptance-selftest.sh
#
# One passing set, plus single-defect variants each failing exactly one
# criterion: a missing tally and a README naming another commit (c1), a table
# that leaves a case unmatched (c2), a pi4-gles3 file the redaction scan
# flags and a --root that is no git repository (c3), the old two-commit
# fails.tsv header (c4), and a Pi 4 entry of each document without the sha
# plus the old 9-of-91 ROADMAP claim (c5). Nothing outside $TMPDIR is read
# or written; the acceptance script itself stays read-only.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ACC="$HERE/pi4-gles3-acceptance.sh"
W=$(mktemp -d "${TMPDIR:-/tmp}/pi4-gles3-acceptance-selftest.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
fails=0

SHA=1234abcd5678ef901234abcd5678ef901234abcd
SHA8=${SHA:0:8}
OTHER=deadbeefdeadbeefdeadbeefdeadbeefdeadbeef

gitc() { # gitc <dir> <git args...>: fixed identity, no user or system config
	env -i PATH="$PATH" HOME="$W/home" GIT_CONFIG_NOSYSTEM=1 \
		GIT_AUTHOR_NAME=s GIT_AUTHOR_EMAIL=s@localhost \
		GIT_COMMITTER_NAME=s GIT_COMMITTER_EMAIL=s@localhost \
		git -c commit.gpgsign=false -C "$1" "${@:2}" >/dev/null
}

base="$W/base"
mkdir -p "$base/tools/cts/log/2026-11-pi4-gles3-$SHA8-rows1-2" "$base/docs/es3"

cat > "$base/tools/cts/pi4-gles3-tree.txt" <<EOF
# Fixture table of two images over the four mustpass cases.
# image A1: the functional cases
dEQP-GLES3.functional.*
# image B2: the info case
dEQP-GLES3.info.*
EOF
printf '%s\n' dEQP-GLES3.functional.a.x dEQP-GLES3.functional.a.y \
	dEQP-GLES3.functional.b.z dEQP-GLES3.info.vendor \
	> "$base/tools/cts/gles3-mustpass.txt"

printf 'commit: %s\n' "$SHA" \
	> "$base/tools/cts/log/2026-11-pi4-gles3-$SHA8-rows1-2/README.md"
for lab in A1 B2; do
	printf 'deqp: complete=1\ndeqp: summary: 2 cases begun, 2 with a result, 0 without\n' \
		> "$base/tools/cts/log/2026-11-pi4-gles3-$SHA8-rows1-2/$lab.tally.txt"
done

{
	printf '# Pi 4 dEQP-GLES3 at %s: every case that is not Pass, per image of tools/cts/pi4-gles3-tree.txt\n' "$SHA"
	printf '# rows: <record>:<status> <case>, sorted by case name (the records of tools/cts/log/2026-11-pi4-gles3-%s-rows1-2/)\n' "$SHA8"
	printf '# images run (2 of 2): A1 B2\n'
	printf 'build/f-A1.txt:Fail dEQP-GLES3.functional.a.x\n'
} > "$base/docs/es3/pi4-gles3-fails.tsv"

cat > "$base/tools/cts/RESULTS.md" <<EOF
# Board results

## ES 2.0: dEQP-GLES2 (stage 1, met)

old stuff

## ES 3.0: dEQP-GLES3 (stage 4)

- AM62P: whole tree done.

**Pi 4, whole tree** on \`$SHA8\` (2026-11-07): 2 of 2 images, 4 cases,
3 pass / 1 fail, 0 GPU recoveries.

## 5. More boards

nothing
EOF

{
	printf '# CTS status\n\n'
	printf -- '- **ES 2.0 (stage 1).** met.\n\n'
	printf -- '- **ES 3.0 (stage 4).** AM62P whole tree done。Pi 4 ran the whole 2-image tree at %s with 0 resets。i.MX8MP not run。\n' "$SHA8"
} > "$base/tools/cts/STATUS.md"

cat > "$base/ROADMAP.md" <<EOF
# ROADMAP

## 4. Stage 4

- AM62P: whole tree done.
- Raspberry Pi 4: the whole 2-image tree at \`$SHA8\` (2026-11-07), 0 resets,
  3 pass / 1 fail (#1261).
- i.MX8MP: not run.

The per-cause inventory is docs/es3-cts-failures.md.
EOF

gitc "$base" init -q
gitc "$base" add -A
gitc "$base" commit -q -m base

variant() { # variant <name>: copy the base tree, mutate, commit
	local v=$1
	cp -a "$base" "$W/$v"
	shift
	"$@" "$W/$v"
	[ -d "$W/$v/.git" ] || return 0
	gitc "$W/$v" add -A
	gitc "$W/$v" commit -q -m variant
}

run_acc() { # run_acc <name>: the acceptance script with a clean environment
	# env -i: the host's YAKOGL_REDACTION_PRIVATE* must not change what the
	# redaction scan of c3 prints.
	env -i PATH="$PATH" HOME="$W" bash "$ACC" --sha "$SHA" --root "$W/$1" \
		> "$W/$1.out" 2> "$W/$1.err"
	echo "$?" > "$W/$1.rc"
}

# m1: the second tally is gone (c1)
m1() { rm "$1/tools/cts/log/2026-11-pi4-gles3-$SHA8-rows1-2/B2.tally.txt"; }
# m2: the session README names another commit (c1)
m2() { printf 'commit: %s\n' "$OTHER" > "$1/tools/cts/log/2026-11-pi4-gles3-$SHA8-rows1-2/README.md"; }
# m3: image B2 selects nothing, one case is matched by no image (c2)
m3() {
	sed -i 's/dEQP-GLES3\.info\.\*/dEQP-GLES3.info.none*/' "$1/tools/cts/pi4-gles3-tree.txt"
}
# m4: a pi4-gles3 file the redaction scan flags (c3). The home path is
# assembled at run time so that this file itself holds none of it (the c3
# scan reads HEAD, which holds this file).
m4() { printf 'run from %s once\n' "/ho""me/fixtureuser/work" > "$1/pi4-gles3-secret-note.txt"; }
# m5: no git repository, so the redaction scan cannot run (c3)
m5() { rm -rf "$1/.git"; }
# m6: the fails.tsv header of the two replaced commits (c4)
m6() {
	{
		printf '# Pi 4 dEQP-GLES3 at 925dbca95380484c42c5ba33be6892e8adc02fa7 and 0ff583297427e87d37ae34e1858a868b849213e9: every case that is not Pass\n'
		printf '# rows: <record>:<status> <case>, sorted by case name\n'
		printf '# images run (1 of 2): A1\n'
		printf 'build/f-A1.txt:Fail dEQP-GLES3.functional.a.x\n'
	} > "$1/docs/es3/pi4-gles3-fails.tsv"
}
# m7: the Pi 4 entry of RESULTS.md names the old commit (c5)
m7() { sed -i "s/\*\*Pi 4, whole tree\*\* on \`$SHA8\`/**Pi 4, first image** on \`925dbca9\`/" "$1/tools/cts/RESULTS.md"; }
# m8: the Pi 4 sentence of STATUS.md names no commit (c5)
m8() { sed -i "s/ at $SHA8 with 0 resets/ of 2 images done/" "$1/tools/cts/STATUS.md"; }
# m9: ROADMAP.md still carries the 9-of-91 claim (c5)
m9() {
	sed -i "/Raspberry Pi 4:/,/3 pass \/ 1 fail (#1261)/c\\
- Raspberry Pi 4: 9 of its 91 images at \`925dbca9\` (2026-10-22): 4,917 cases." \
		"$1/ROADMAP.md"
}

check() { # check <name> <want rc> <want PASS/FAIL of c1..c5> <substring of stdout or stderr>
	local v=$1 grc=$2 gstat=$3 sub=$4
	[ "$(awk 'END { print NR }' "$W/$v.out")" -eq 5 ] || { echo "  FAIL $v: not five stdout lines"; fails=$((fails + 1)); return; }
	awk '$1 != "c" NR || NF < 3 { exit 1 }' "$W/$v.out" || { echo "  FAIL $v: a line is not 'c<N> <status> <reason>'"; fails=$((fails + 1)); return; }
	local stat rc
	stat=$(awk '{ printf "%s ", $2 }' "$W/$v.out")
	rc=$(cat "$W/$v.rc")
	if [ "$stat" != "$gstat " ] || [ "$rc" != "$grc" ] || ! grep -Fq -- "$sub" "$W/$v.out" "$W/$v.err" 2>/dev/null; then
		echo "  FAIL $v: rc $rc want $grc, status '$stat' want '$gstat ', '$sub' not printed"
		sed 's/^/       out /' "$W/$v.out"
		sed 's/^/       err /' "$W/$v.err" | head -5
		fails=$((fails + 1))
		return
	fi
	echo "  ok   $v: rc $rc, $gstat"
}

cp -a "$base" "$W/pass"
run_acc pass
check pass 0 "PASS PASS PASS PASS PASS" "4 matched once"

variant c1-missing-tally m1
run_acc c1-missing-tally
check c1-missing-tally 1 "FAIL PASS PASS PASS PASS" "missing (1): B2"

variant c1-other-commit m2
run_acc c1-other-commit
check c1-other-commit 1 "FAIL PASS PASS PASS PASS" "names commit $OTHER"

variant c2-case-unmatched m3
run_acc c2-case-unmatched
check c2-case-unmatched 1 "PASS FAIL PASS PASS PASS" "1 not matched"

variant c3-scan-hit m4
run_acc c3-scan-hit
check c3-scan-hit 1 "PASS PASS FAIL PASS PASS" "pi4-gles3-secret-note.txt"

variant c3-not-a-repo m5
run_acc c3-not-a-repo
check c3-not-a-repo 1 "PASS PASS FAIL PASS PASS" "did not run"

variant c4-old-header m6
run_acc c4-old-header
check c4-old-header 1 "PASS PASS PASS FAIL PASS" "925dbca9"

variant c5-results-old m7
run_acc c5-results-old
check c5-results-old 1 "PASS PASS PASS PASS FAIL" "RESULTS.md Pi 4 ES 3.0 entry does not name"

variant c5-status-nosha m8
run_acc c5-status-nosha
check c5-status-nosha 1 "PASS PASS PASS PASS FAIL" "STATUS.md Pi 4 ES 3.0 entry does not name"

variant c5-roadmap-old m9
run_acc c5-roadmap-old
check c5-roadmap-old 1 "PASS PASS PASS PASS FAIL" "9 of its 91 images at 925dbca9"

if [ $fails -eq 0 ]; then
	echo "pi4-gles3-acceptance-selftest: all checks passed"
	exit 0
fi
echo "pi4-gles3-acceptance-selftest: $fails check(s) FAILED"
exit 1
