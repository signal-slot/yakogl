#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# gate_st_table.sh - the ST shader reference table check (issues #1281, #7):
# tests/reference/vivante_blobs_st.rs regenerated from Mesa's etna_shc by
# `gen_shader_table.py --st --check` equals the committed file byte for byte.
# Host only, no board.
#
#   tools/vivante/gate_st_table.sh
#
# Environment:
#   MESA_ETNA_TREE  Mesa 26.2.1 source tree whose build-etnaviv holds etna_shc
#                   (tools/vivante/README.md)
#   ETNA_SHC        an etna_shc binary, replacing the one under the tree
#   GATE_ST_TABLE_GEN  a program replacing gen_shader_table.py (for testing)
#
# Exit codes: 0 the table matches; 1 mismatch (the generator's output, which
# names the changed rows, is on stdout); 2 gap (exactly one `gap:` line on
# stderr naming MESA_ETNA_TREE; the check was NOT run, and a gap is not a
# pass); 64 usage.

set -eu
export LC_ALL=C

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

gap() {
	echo "gap: MESA_ETNA_TREE $1; the gate_st_table check was NOT run" >&2
	exit 2
}

if [ $# -ne 0 ]; then
	echo "usage: $0" >&2
	exit 64
fi

# With an explicit ETNA_SHC the tree is not needed for the tool.
if [ -n "${ETNA_SHC:-}" ]; then
	[ -x "$ETNA_SHC" ] || gap "is not set or has no usable tool (ETNA_SHC $ETNA_SHC is not executable)"
else
	[ -n "${MESA_ETNA_TREE:-}" ] || gap "is not set (and ETNA_SHC is not set)"
	[ -d "$MESA_ETNA_TREE" ] || gap "is not a directory ($MESA_ETNA_TREE)"
	[ -x "$MESA_ETNA_TREE/build-etnaviv/src/gallium/drivers/etnaviv/etna_shc" ] ||
		gap "lacks build-etnaviv/src/gallium/drivers/etnaviv/etna_shc"
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gate_st_table.XXXXXX") || {
	echo "gate_st_table: cannot create a temporary directory" >&2
	exit 1
}
trap 'rm -rf "$tmp"' EXIT

rc=0
if [ -n "${GATE_ST_TABLE_GEN:-}" ]; then
	"$GATE_ST_TABLE_GEN" --st --check >"$tmp/out" 2>"$tmp/err" || rc=$?
else
	python3 -I "$HERE/gen_shader_table.py" --st --check >"$tmp/out" 2>"$tmp/err" || rc=$?
fi

case $rc in
0)
	cat "$tmp/out"
	echo "gate_st_table: ST shader table matches"
	;;
2)
	# the generator's own gap: no etna_shc, or no glslangValidator
	gap "has no usable etna_shc or glslangValidator"
	;;
1)
	cat "$tmp/out"
	cat "$tmp/err" >&2
	echo "gate_st_table: mismatch: the ST shader table differs from tests/reference/vivante_blobs_st.rs (rows named above)"
	exit 1
	;;
*)
	cat "$tmp/out"
	cat "$tmp/err" >&2
	echo "gate_st_table: the generator failed with exit $rc"
	exit 1
	;;
esac
