#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Write tests/fixtures/v3d_texref.txt: v3d-tex's C reference functions'
# outputs (tools/v3d/texref_fixture.c), headed by the counts and by the
# SHA-256 of the two headers they come from. Run from the repository root.
# The scratch directory is $TEXREF_TMP (default: a new directory under
# ${TMPDIR:-/tmp}), and the program is left there.
set -eu
dir=tools/v3d
out=tests/fixtures/v3d_texref.txt
work=${TEXREF_TMP:-$(mktemp -d)}
cc -std=c11 -Wall -Wextra -Werror -O1 -o "$work/texref_fixture" "$dir/texref_fixture.c"
"$work/texref_fixture" > "$work/rows" 2> "$work/counts"
{
	echo "# v3d-tex's C reference functions on their own fixtures; written by $dir/texref_fixture.sh,"
	echo "# read by src/shaderc/online/texref.rs. Floats are bit patterns in hex."
	echo "# tex_filter_ref.h: $(sha256sum "$dir/tex_filter_ref.h" | cut -d' ' -f1)"
	echo "# tex_mip_case.h: $(sha256sum "$dir/tex_mip_case.h" | cut -d' ' -f1)"
	cat "$work/counts"
	cat "$work/rows"
} > "$out"
echo "texref_fixture: $(grep -c '^cube ' "$out") cube rows, $(grep -c '^level ' "$out") level rows -> $out"
