#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# tri_shaders71: tools/v3d/tri-shaders71/mesa_passthrough_interp_color.json
# and .h are, byte for byte, what tools/v3d/clif_shaders71.py extracts from
# tools/v3d/pi5/triangle.clif (Mesa 26.2.2 on the Raspberry Pi 5); each of the
# three programs (FS, VS, CS) ends in a thread switch with its two delay slots
# (the extractor refuses otherwise); the words, uniform streams, attribute
# records and record fields equal the drm-shim dump tools/v3d/shim71/triangle.clif
# (--check-shim); the header compiles as C11 with -Wall -Wextra -Werror.
# No Mesa build is needed (MESA_V3D_TREE is not read). Mesa's QPU disassembler
# is not used: tools/v3d/qpu_ref.c is built for 4.2 only (devinfo.ver = 42).
# Convention: tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=tri_shaders71
dir=build/v3d71/$name
mkdir -p "$dir" || exit 1
rm -f "$dir/mesa_passthrough_interp_color.json" "$dir/mesa_passthrough_interp_color.h" || exit 1
committed=tools/v3d/tri-shaders71
fail=0
programs=0

command -v python3 >/dev/null 2>&1 || {
  echo "$name: python3 is not installed; the extractor is a Python script"
  exit 1
}

if python3 -I tools/v3d/clif_shaders71.py --out-dir "$dir" 2>"$dir/extract.log"; then
  for ext in json h; do
    f=mesa_passthrough_interp_color.$ext
    if cmp -s "$dir/$f" "$committed/$f"; then
      echo "$name: $committed/$f equals a fresh extraction"
    else
      diff -u "$committed/$f" "$dir/$f" | head -20
      echo "$name: $committed/$f differs from a fresh extraction (run: python3 -I tools/v3d/clif_shaders71.py)"
      fail=$((fail + 1))
    fi
  done
  programs=3
else
  cat "$dir/extract.log"
  echo "$name: the extractor failed (a program without its thread end, or an unreadable dump)"
  fail=$((fail + 1))
fi

if python3 -I tools/v3d/clif_shaders71.py --check-shim >"$dir/shim.log" 2>&1; then
  echo "$name: $(cat "$dir/shim.log")"
else
  cat "$dir/shim.log"
  fail=$((fail + 1))
fi

cc=${CC:-cc}
if command -v "$cc" >/dev/null 2>&1; then
  printf '#include "mesa_passthrough_interp_color.h"\nint main(void) { return (int)(mesa71_tri_fs_code[0] != 0); }\n' >"$dir/hdr.c"
  if "$cc" -std=c11 -Wall -Wextra -Werror -I"$committed" -o "$dir/hdr" "$dir/hdr.c" 2>"$dir/cc.log"; then
    echo "$name: the header compiles as C11, -Wall -Wextra -Werror"
  else
    cat "$dir/cc.log"
    fail=$((fail + 1))
  fi
else
  echo "$name: no C compiler; the header's compile check is left out"
fi

echo "$name: $programs programs, $fail failed"
[ "$fail" -eq 0 ]
