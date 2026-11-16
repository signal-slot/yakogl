#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/deqp-limits-tally-selftest.sh
#
# Host-only check of tools/cts/deqp-limits-tally.py --require-pass limits on
# the run directories in tools/cts/testdata/deqp-limits-tally/, written the
# way tools/cts/host-deqp-run.sh writes one (commit, all.txt, c_000.qpa):
#   allpass  every selected limit case Pass, plus two Fail rows the
#            selection must not take (implementation_limits.max_texture_size,
#            and state_query.integers.max_varying_vectors_getinteger, a
#            limit word but not a name of tools/cts/gles3-mustpass.txt)
#   onefail  the same with integers64.max_uniform_block_size_getfloat Fail
#   missing  the same with integers.max_vertex_output_components_getboolean
#            absent from all.txt and the log
# The selected names are checked against an independent grep of
# tools/cts/gles3-mustpass.txt, and the default output (no --require-pass)
# against onefail.default.txt, which the script before --require-pass
# wrote. Nothing here is a real run.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TALLY=$HERE/deqp-limits-tally.py
FIX=$HERE/testdata/deqp-limits-tally
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
# chk <label> <command...>: ok when the command succeeds
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }

# tally <args...> -> $OUT (stdout), $ERR (stderr), $RC; run in the fixture
# directory, so the run line names the run as given (allpass, ...).
tally() {
    local e
    e=$(mktemp "${TMPDIR:-.}/deqp-limits-tally.XXXXXX") || exit 2
    OUT=$(cd "$FIX" && python3 -I "$TALLY" "$@" 2>"$e"); RC=$?
    ERR=$(cat "$e"); rm -f "$e"
}
has() { printf '%s\n' "$OUT" | grep -qF -- "$1"; }
hasnt() { ! has "$1"; }
last_is() { [ "$(printf '%s\n' "$OUT" | tail -1)" = "$1" ]; }
rc_is() { [ "$RC" -eq "$1" ]; }
rc_nonzero() { [ "$RC" -ne 0 ]; }
# the case names of the verdict lines, in order
selected() { printf '%s\n' "$OUT" | sed -n 's/^  [A-Za-z]\+ \+\(dEQP-[^ ]*\).*/\1/p'; }

W='max_varying_vectors|max_varying_components|max_fragment_input_components|max_vertex_output_components|max_uniform_block_size|max_combined_vertex_uniform_components|max_combined_fragment_uniform_components'
want=$(grep -E "^dEQP-GLES3\.functional\.(implementation_limits|state_query\.(integers|integers64|floats|boolean))\.($W)(_get[a-z0-9]+)?\$" \
    "$HERE/gles3-mustpass.txt")
nwant=$(printf '%s\n' "$want" | grep -c .)
selected_is_want() { [ "$(selected)" = "$want" ]; }

# all Pass: exit 0, exactly the mustpass names, the unselected Fail rows not
# listed.
tally --require-pass limits allpass
chk "all-pass: exit 0" rc_is 0
chk "all-pass: the mustpass names its 28 limit cases" [ "$nwant" -eq 28 ]
chk "all-pass: one line per gles3-mustpass.txt limit case, in its order" selected_is_want
chk "all-pass: ends '28 selected, 28 pass, 0 not-pass'" last_is '28 selected, 28 pass, 0 not-pass'
chk "all-pass: implementation_limits.max_texture_size is not selected" hasnt 'max_texture_size'
chk "all-pass: integers.max_varying_vectors_getinteger (not in the mustpass) is not selected" \
    hasnt 'state_query.integers.max_varying_vectors_getinteger '
chk "all-pass: the run line names the GPU" has 'run allpass  gpu=v3d module=gles3 commit=a11a11a'

# one Fail: non-zero, the case named with its message.
tally --require-pass limits onefail
chk "one Fail: exit non-zero (3)" rc_is 3
chk "one Fail: the Fail line names the case and its message" \
    has '  Fail          dEQP-GLES3.functional.state_query.integers64.max_uniform_block_size_getfloat  Got invalid float value: 6144, expected at least 16384'
chk "one Fail: ends '28 selected, 27 pass, 1 not-pass'" last_is '28 selected, 27 pass, 1 not-pass'
chk "one Fail: still every selected case listed" selected_is_want

# a missing case: non-zero, MISSING named.
tally --require-pass limits missing
chk "missing case: exit non-zero (3)" rc_is 3
chk "missing case: the MISSING line names the case" \
    has '  MISSING       dEQP-GLES3.functional.state_query.integers.max_vertex_output_components_getboolean  not in the run'"'"'s case list or logs'
chk "missing case: ends '28 selected, 27 pass, 1 not-pass'" last_is '28 selected, 27 pass, 1 not-pass'
chk "missing case: still every selected case listed" selected_is_want

# several runs: each its own block; one failing run fails the verdict.
tally --require-pass limits allpass onefail
chk "two runs, one with a Fail: exit non-zero" rc_nonzero
chk "two runs: both blocks printed" has '28 selected, 28 pass, 0 not-pass'
chk "two runs: ends with the failing run's line" last_is '28 selected, 27 pass, 1 not-pass'

# the default output is unchanged, byte for byte.
tally onefail
chk "no --require-pass: exit 0" rc_is 0
chk "no --require-pass: the output is byte-identical to onefail.default.txt" \
    [ "$OUT" = "$(cat "$FIX/onefail.default.txt")" ]

# command-line refusals.
tally --require-pass limits --compare allpass onefail
chk "--require-pass with --compare: a usage error (exit 2)" rc_is 2
tally --require-pass nosuch allpass
chk "--require-pass with an unknown set: a usage error (exit 2)" rc_is 2
tally --require-pass limits
chk "--require-pass without a run: a usage error (exit 2)" rc_is 2
tally --require-pass limits allpass/commit
chk "--require-pass on a path that is not a run: exit 1" rc_is 1

# the script's own inline selftest still passes.
tally --selftest
chk "deqp-limits-tally.py --selftest: exit 0" rc_is 0

printf '%d ok, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
