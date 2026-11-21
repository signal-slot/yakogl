#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mali-deqp-tally-selftest.sh
#
# Host-only check of tools/cts/mali-deqp-tally.py on the synthetic slices of
# tools/cts/testdata/mali-deqp/: the good slices must cover, tally and match
# the pinned markdown, and each failing fixture (duplicate case, missing case,
# missing chunk field, non-ancestor commit, stale doc block, ...) must make
# its mode exit non-zero naming the rule. A rerun slice (meta.txt reruns=)
# replaces the Lost rows of the slice it names (rerun/ pair); a rerun of a
# non-Lost case or of a slice not given fails. A slice with refusals.tsv
# (refusals/) gives each named case its own refusal as the cause; a row naming
# a case the slice does not hold fails. A replaces= slice (replaces/) swaps
# whole chunks of the slices it names; a missing case or an absent slice
# fails. --check-doc with --log takes the
# slices from the block's own "Slices:" line; a block without it fails.
# Nothing here is a real run.
set -u

HERE=$(dirname "$0")
WANT=65
OUT=$(python3 -I "$HERE/mali-deqp-tally.py" --self-test 2>&1)
RC=$?
printf '%s\n' "$OUT"
[ "$RC" -eq 0 ] || { echo "mali-deqp-tally-selftest: self-test exited $RC" >&2; exit 1; }
if printf '%s\n' "$OUT" | grep -qx "$WANT checks passed"; then
    exit 0
fi
echo "mali-deqp-tally-selftest: expected '$WANT checks passed'" >&2
exit 1
