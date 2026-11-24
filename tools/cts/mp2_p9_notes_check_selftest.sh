#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_p9_notes_check_selftest.sh
#
# Host-only check of tools/cts/mp2-p9-notes-check.py on synthetic notes in a
# temporary directory: a complete set (c1, c2, c3 ok, exit 0) and one set per
# failure (no gl_test diff line in Board verdicts, the diff line outside it,
# missing not 0, a build sha8 that differs between the two notes, a prefix
# sha8 that does not, a Q11 line without a record name, one without a width,
# a Q11 line missing from the pre-study row, no Board verdicts section),
# --only, and the exit codes. The real notes are only read to check that a
# default run prints the summary line; its verdict depends on the tree and
# is not asserted. Nothing here is a real capture.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mp2-p9-notes-check.py
D=$(mktemp -d "${TMPDIR:-/tmp}/mp2-p9-notes.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }
nogrep() { ! grep -q -- "$2" <<<"$1"; }

# gen_full <dir>: a complete synthetic set (all three criteria ok)
gen_full() {
  local w=$1
  mkdir -p "$w"
  cat >"$w/bringup.md" <<'EOF'
# STM32MP257F-DK bring-up (synthetic note)

## A step section

Some step text.

## Board verdicts

| step | verdict |
| --- | --- |
| 9 `gl_test` set | run 2026-11-25, images built from `d4c3b2a1`, `20261125-1012-gl_test_set-9f8e7d6c`, every shard:

    mp2-gltest-diff: new 0, flaky 0, expected 2, missing 0

### How the first steps were run

Images built at e07d066a for steps 1-4.

## The next section

text
EOF
  cat >"$w/p9.md" <<'EOF'
# STM32MP257F-DK P9 (synthetic note)

## Procedure

The last lines are `mp2-p9-diff: shared S, new N, fixed F, missing M` and
`mp2-gltest-diff: new N, flaky F, expected E, missing M`.

## Result (2026-11-25)

dEQP-GLES2 sweep of all 83 groups, images built from `d4c3b2a1`:

    mp2-p9-diff: shared 19, new 0, fixed 0, missing 0

- Q11: the DK draws 3 px wide at glLineWidth(3.0) (record `20261125-1500-q11_wideline-9f8e7d6c`): option A stands.
EOF
  cat >"$w/prestudy.md" <<'EOF'
# Pre-study (synthetic note)

| Q11 | wide lines on the emulating core | **Answered (2026-11-25, q11-board):** the registers draw width 3 (`20261125-1500-q11_wideline-9f8e7d6c`); report 1 to 4 |
EOF
}

run() {
  local w=$1
  shift
  python3 -I "$TOOL" --bringup "$w/bringup.md" --p9 "$w/p9.md" \
           --prestudy "$w/prestudy.md" "$@" 2>&1
}

# the complete set: every criterion ok
w=$D/full; gen_full "$w"
out=$(run "$w"); rc=$?
chk "full: exit 0" test $rc -eq 0
chk "full: c1 ok" grep -qx 'c1: ok' <<<"$out"
chk "full: c2 ok" grep -qx 'c2: ok' <<<"$out"
chk "full: c3 ok" grep -qx 'c3: ok' <<<"$out"
chk "full: summary" grep -qx 'mp2-p9-notes-check: 3 ok, 0 missing' <<<"$out"

# no gl_test diff line in the section (the build commit stays)
w=$D/no-diffline; gen_full "$w"
grep -v 'mp2-gltest-diff: new' "$w/bringup.md" >"$w/t" && mv "$w/t" "$w/bringup.md"
out=$(run "$w"); rc=$?
chk "no diff line: exit 1" test $rc -eq 1
chk "no diff line: c1 missing" grep -q '^c1: missing:.*mp2-gltest-diff' <<<"$out"
chk "no diff line: c2 ok" grep -qx 'c2: ok' <<<"$out"
chk "no diff line: summary" grep -qx 'mp2-p9-notes-check: 2 ok, 1 missing' <<<"$out"

# the diff line outside the section (before Board verdicts)
w=$D/line-outside; gen_full "$w"
line=$(grep 'mp2-gltest-diff: new' "$w/bringup.md")
grep -v 'mp2-gltest-diff: new' "$w/bringup.md" >"$w/t" && mv "$w/t" "$w/bringup.md"
{ head -1 "$w/bringup.md"; printf '%s\n' "$line"; tail -n +2 "$w/bringup.md"; } >"$w/t2"
mv "$w/t2" "$w/bringup.md"
out=$(run "$w"); rc=$?
chk "line outside: exit 1" test $rc -eq 1
chk "line outside: c1 missing" grep -q '^c1: missing:.*mp2-gltest-diff' <<<"$out"
chk "line outside: c2 ok" grep -qx 'c2: ok' <<<"$out"

# no Board verdicts section at all
w=$D/no-section; gen_full "$w"
sed -i '/^## Board verdicts/,$d' "$w/bringup.md"
out=$(run "$w"); rc=$?
chk "no section: exit 1" test $rc -eq 1
chk "no section: c1 missing" grep -q "^c1: missing:.*'Board verdicts' section" <<<"$out"
chk "no section: c2 missing too" grep -q "^c2: missing:.*'Board verdicts'" <<<"$out"
chk "no section: c3 ok" grep -qx 'c3: ok' <<<"$out"
chk "no section: summary" grep -qx 'mp2-p9-notes-check: 1 ok, 2 missing' <<<"$out"

# the diff line there but missing not 0
w=$D/missing3; gen_full "$w"
sed -i 's/mp2-p9-diff: shared 19, new 0, fixed 0, missing 0/mp2-p9-diff: shared 19, new 0, fixed 0, missing 3/' "$w/p9.md"
out=$(run "$w"); rc=$?
chk "missing 3: exit 1" test $rc -eq 1
chk "missing 3: c2 message" grep -q '^c2: missing:.*missing 3, not 0' <<<"$out"
chk "missing 3: c1 ok" grep -qx 'c1: ok' <<<"$out"
chk "missing 3: summary" grep -qx 'mp2-p9-notes-check: 2 ok, 1 missing' <<<"$out"

# a different build sha8 in the two notes
w=$D/sha-differs; gen_full "$w"
sed -i 's/built from `d4c3b2a1`/built from `11aa22bb3`/' "$w/p9.md"
out=$(run "$w"); rc=$?
chk "sha differs: exit 1" test $rc -eq 1
chk "sha differs: c1 ok" grep -qx 'c1: ok' <<<"$out"
chk "sha differs: c2 names both" grep -q '^c2: missing:.*11aa22bb3.*d4c3b2a1' <<<"$out"
chk "sha differs: c3 ok" grep -qx 'c3: ok' <<<"$out"

# a 7-digit sha in one note and its 8-digit prefix in the other: the same commit
w=$D/sha-prefix; gen_full "$w"
sed -i 's/`d4c3b2a1`/`d4c3b2a`/' "$w/bringup.md"
out=$(run "$w"); rc=$?
chk "sha prefix: exit 0" test $rc -eq 0
chk "sha prefix: summary" grep -qx 'mp2-p9-notes-check: 3 ok, 0 missing' <<<"$out"

# no build sha8 in the P9 note
w=$D/sha-p9-absent; gen_full "$w"
sed -i 's/images built from `d4c3b2a1`:/the sweep:/' "$w/p9.md"
out=$(run "$w"); rc=$?
chk "sha absent in P9: exit 1" test $rc -eq 1
# (the synthetic paths print as p9.md, prestudy.md, bringup.md)
chk "sha absent in P9: c2 message" grep -q "^c2: missing:.*'built from <sha>' commit in p9.md" <<<"$out"

# a Q11 line with a width but no record name
w=$D/q11-no-record; gen_full "$w"
sed -i 's/^- Q11.*/- Q11: the DK draws 3 px wide at 3.0: keep option A./' "$w/p9.md"
out=$(run "$w"); rc=$?
chk "Q11 no record: exit 1" test $rc -eq 1
chk "Q11 no record: c3 names the P9 note" grep -q '^c3: missing:.*p9.md' <<<"$out"
chk "Q11 no record: c1 ok" grep -qx 'c1: ok' <<<"$out"
chk "Q11 no record: c2 ok" grep -qx 'c2: ok' <<<"$out"

# a Q11 line with a record name but no measured width
w=$D/q11-no-width; gen_full "$w"
sed -i 's/^- Q11.*/- Q11: see the board record `20261125-1500-q11_wideline-9f8e7d6c` for the measurement./' "$w/p9.md"
out=$(run "$w"); rc=$?
chk "Q11 no width: exit 1" test $rc -eq 1
chk "Q11 no width: c3 names the P9 note" grep -q '^c3: missing:.*p9.md' <<<"$out"

# the pre-study Q11 row without a record name
w=$D/q11-prestudy; gen_full "$w"
sed -i 's/^| Q11 .*/| Q11 | wide lines: the registers draw width 3; the range decision is open |/' "$w/prestudy.md"
out=$(run "$w"); rc=$?
chk "Q11 pre-study: exit 1" test $rc -eq 1
chk "Q11 pre-study: c3 names the pre-study" grep -q '^c3: missing:.*prestudy.md' <<<"$out"

# --only
out=$(run "$D/full" --only c2); rc=$?
chk "--only c2: exit 0" test $rc -eq 0
chk "--only c2: its line" grep -qx 'c2: ok' <<<"$out"
chk "--only c2: summary counts one" grep -qx 'mp2-p9-notes-check: 1 ok, 0 missing' <<<"$out"
chk "--only c2: no c1 line" nogrep "$out" '^c1:'
chk "--only c2: no c3 line" nogrep "$out" '^c3:'
out=$(run "$D/no-diffline" --only c1); rc=$?
chk "--only c1 failing: exit 1" test $rc -eq 1
chk "--only c1 failing: summary" grep -qx 'mp2-p9-notes-check: 0 ok, 1 missing' <<<"$out"
run "$D/full" --only c4 >/dev/null; chk "bad --only exits 2" test $? -eq 2

# a note that cannot be read
python3 -I "$TOOL" --bringup "$D/nope.md" >"$D/err.out" 2>&1
chk "missing note exits 2" test $? -eq 2
chk "error line" grep -q '^mp2-p9-notes-check: ' "$D/err.out"

# the real notes: the summary line prints whatever the tree holds
python3 -I "$TOOL" >"$D/real.out" 2>&1
tail -1 "$D/real.out" >"$D/real.last"
chk "real notes: summary line" grep -Eqx 'mp2-p9-notes-check: [0-9]+ ok, [0-9]+ missing' "$D/real.last"

echo "mp2_p9_notes_check selftest: $pass ok, $fail failed"
[ "$fail" -eq 0 ]
