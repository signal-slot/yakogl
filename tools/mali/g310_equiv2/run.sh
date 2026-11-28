#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# g310_equiv2/run.sh - G310v2 = G610 over every Mali reference added or
# changed since the 7e27b1eb recheck (docs/notes/mali-g310-vs-g610.md,
# "Second extension"): the standing check tools/mali/g310_check.sh with that
# base, over every *.expect.json name (not only deqp_*). Needs
# CONNECTED_DEPS_DIR (the shim is built there on the first run).
#
#   tools/mali/g310_equiv2/run.sh [--since <rev>] [g310_check.sh options]
# Exit 1 if any reference differs; the last line is
# "N identical, M differ, K skipped".
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
since=7e27b1eb
if [ "${1:-}" = "--since" ]; then since="$2"; shift 2; fi
scratch="${TMPDIR:-/var/tmp}/g310_equiv2.$$"
mkdir -p "$scratch"
trap 'rm -rf "$scratch"' EXIT
exec "$here/../g310_check.sh" --since "$since" --scratch "$scratch" "$@" '*.expect.json'
