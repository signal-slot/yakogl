#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-merge-selftest.sh - the synthetic-record selftest of imx8mp-gles3-merge.py (its --selftest):
# disjoint slices, board-form slices with '-' rows (M1), missing entries and --partial (M2), clean and
# non-clean README verdict lines (M3), Armadillo-X2 three-word rows with begun and noresult "-" (w3 not stored).
# Records are written under $TMPDIR (never /tmp) and removed.
#
#   TMPDIR=<dir> tools/cts/imx8mp-gles3-merge-selftest.sh
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
: "${TMPDIR:?set TMPDIR to a directory (synthetic records are never written to /tmp)}"
[ -d "$TMPDIR" ] || { echo "TMPDIR is not a directory" >&2; exit 2; }
case $(cd "$TMPDIR" && pwd -P) in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "TMPDIR must not be under /tmp" >&2; exit 2 ;; esac
exec python3 "$HERE/imx8mp-gles3-merge.py" --selftest
