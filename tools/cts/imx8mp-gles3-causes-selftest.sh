#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx8mp-gles3-causes.py: WDOG1 and Hang marker lines map to cause ids. No feature sets, no board.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/imx8mp-gles3-causes.py
tmp=$(mktemp -d "${TMPDIR:-/tmp}/causes-st.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

# made-up cause map and table (entry 0 srgb blend, 1 depth, 2 shaders, 3 filtering.3d)
printf 'gc-hang-srgb-blend\tdEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.*\tsrgb\n' > "$tmp/c.tsv"
printf 'gc-hang-depth\tdEQP-GLES3.functional.fragment_ops.depth.*\tdepth\n' >> "$tmp/c.tsv"
printf 'gc-wdog1-f3d\tdEQP-GLES3.functional.texture.filtering.3d.*\tf3d\n' >> "$tmp/c.tsv"
printf 'gc-hang-f3d\tdEQP-GLES3.functional.texture.filtering.3d.*\tf3d hang\n' >> "$tmp/c.tsv"
printf 'other\tdEQP-GLES3.functional.shaders.*\tother\n' >> "$tmp/c.tsv"
{ :
  echo 'dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.rgb_func_alpha_func.*'
  echo 'dEQP-GLES3.functional.fragment_ops.depth.*'
  echo 'dEQP-GLES3.functional.shaders.a.*'
  echo 'dEQP-GLES3.functional.texture.filtering.3d.combinations.nearest_*'; } > "$tmp/t.txt"
cat > "$tmp/README.md" <<'R'
# slice
## Resets and hangs
WDOG1 e03: the entry ended on a WDOG1 reset (rc=0 verdict=done_with_resets resets=1 back_after=612s)
Hang: e00: the entry hung (rc=1 verdict=no_record resets=absent back_after=1500s)
Hang: e01: the entry hung (rc=1 verdict=no_record); last case dEQP-GLES3.functional.fragment_ops.depth.x
Hang: e02: the entry hung (rc=1 verdict=no_record)
R
out=$(python3 -I "$tool" "$tmp/README.md" --causes "$tmp/c.tsv" --table "$tmp/t.txt") || fail "tool"
echo "$out" | grep -q '^WDOG1	e03	gc-wdog1-f3d	' || fail "WDOG1 e03 not gc-wdog1-f3d: $out"
echo "$out" | grep -q '^Hang	e00	gc-hang-srgb-blend	' || fail "Hang e00 not gc-hang-srgb-blend: $out"
echo "$out" | grep -q '^Hang	e01	gc-hang-depth	' || fail "Hang e01 by case"
echo "$out" | grep -q '^Hang	e02	unclassified	' || fail "Hang e02 (only a non-hang id) must be unclassified"
[ "$(echo "$out" | wc -l)" = 4 ] || fail "four lines wanted"
# a WDOG1 line with no wdog1 id falls back to the hang ids
printf 'WDOG1 e00: x\n' > "$tmp/w.md"
python3 -I "$tool" "$tmp/w.md" --causes "$tmp/c.tsv" --table "$tmp/t.txt" | grep -q '	gc-hang-srgb-blend	' || fail "wdog fallback"
# the real map and table: e58 is a WDOG1 id, e13 a hang id
printf 'WDOG1 e58: the entry ended on a WDOG1 reset (x)\nHang: e13: the entry hung (x)\nHang: e57: x\n' > "$tmp/r.md"
r=$(python3 -I "$tool" "$tmp/r.md") || fail "real map"
echo "$r" | grep -q '^WDOG1	e58	gc-wdog1-' || fail "real e58: $r"
echo "$r" | grep -q '^Hang	e13	gc-gpu-hang-fbo-srgb-blend	' || fail "real e13: $r"
echo "$r" | grep -q '^Hang	e57	gc-gpu-hang-filtering-2d-array	' || fail "real e57: $r"
if python3 -I "$tool" 2>/dev/null; then fail "no args accepted"; fi
echo "imx8mp-gles3-causes-selftest: ok"
