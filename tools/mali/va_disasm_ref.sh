#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Mesa 26.2.1's Valhall disassembly of the words of a fixture, for
# src/shaderc/online/mali/isa/tests.rs:
#
#   tools/mali/va_disasm_ref.sh tools/mali/isa-ref/board-2026-10-18.txt \
#     > tools/mali/isa-ref/board-2026-10-18.mesa-26.2.1.txt
#
# valhall_disasm.c is made with Mesa's own command (valhall/meson.build:28-35)
# from the vendored, unmodified disasm.py and ISA.xml; it needs python3 with
# mako and a C compiler. Nothing is written unless every step succeeds.
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 <fixture>" >&2; exit 2; }
root=$(cd "$(dirname "$0")/../.." && pwd)
va="$root/third_party/mesa-26.2.1/src/panfrost/compiler/bifrost/valhall"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

PYTHONDONTWRITEBYTECODE=1 python3 "$va/disasm.py" --xml "$va/ISA.xml" > "$tmp/valhall_disasm.c"
cc -std=gnu11 -O1 -Wall -I"$va" -o "$tmp/va_disasm_ref" "$tmp/valhall_disasm.c" "$root/tools/mali/va_disasm_ref.c"
build=$(git -C "$root" describe --always --dirty)
# Everything goes to a temp file first; nothing is printed unless every step
# succeeded.
{
	echo "# Mesa 26.2.1's va_disasm_instr (valhall/disasm.py on the vendored ISA.xml) on the words of $(basename "$1"),"
	echo "# by tools/mali/va_disasm_ref.sh: build $build; the \"# binary\" lines below are that file's."
	grep -E '^(# binary |[0-9a-f]{2} )' "$1" | "$tmp/va_disasm_ref"
} > "$tmp/out"
cat "$tmp/out"
