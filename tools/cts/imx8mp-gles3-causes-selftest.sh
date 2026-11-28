#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx8mp-gles3-causes.py: the WDOG1 and Hang marker lines of a made-up slice README (the line format
# of imx8mp-gles3-slice.py, tools/cts/testdata/imx8mp-gles3-slice/) go to the cause ids of a made-up map by table
# entry or by last case, then to the committed map and table. Host only; writes under $TMPDIR.
[ -n "${BASH_VERSION:-}" ] || exec bash "$0" "$@"
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:?TMPDIR must be set}/gles3-causes-selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
tool="$here/imx8mp-gles3-causes.py"
T=$'\t'

# Made-up map: a non-event id first (never a candidate), hang and wdog1 ids.
cat > "$tmp/c.tsv" <<C
# made-up cause map
rc2-sized-formats${T}dEQP-GLES3.functional.*${T}not an event id
gc-hang-srgb-blend${T}dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.*${T}srgb blend hang
gc-hang-depth${T}dEQP-GLES3.functional.fragment_ops.depth.*${T}depth hang
gc-wdog1-filtering-3d${T}dEQP-GLES3.functional.texture.filtering.3d.*${T}3d filtering reset
gc-hang-filtering-3d${T}dEQP-GLES3.functional.texture.filtering.3d.*${T}3d filtering hang
C
# Made-up table: e00 srgb blend + depth, e01 shaders, e02 filtering.3d, e03 2d filtering.
cat > "$tmp/t.txt" <<'T'
# made-up table
dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.rgb_func_alpha_func.*,dEQP-GLES3.functional.fragment_ops.depth.*
dEQP-GLES3.functional.shaders.a.*
dEQP-GLES3.functional.texture.filtering.3d.combinations.nearest_*
dEQP-GLES3.functional.texture.filtering.2d.*
T
run() { python3 -I "$tool" "$@" --causes "$tmp/c.tsv" --table "$tmp/t.txt"; }

# One WDOG1 line and one Hang line, as imx8mp-gles3-slice.py writes them.
mkdir "$tmp/slice"
cat > "$tmp/slice/README.md" <<'R'
# made-up slice

## Resets and hangs

- WDOG1 e02: the entry ended on a WDOG1 reset (rc=0 verdict=done_with_resets resets=1 back_after=612s)
- Hang: e00: the entry hung (rc=1 verdict=no_record resets=absent back_after=1500s); last case dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.rgb_func_alpha_func.src.one
R
out=$(run "$tmp/slice") || fail "exit status on the made-up slice"
[ "$out" = "WDOG1${T}e02${T}gc-wdog1-filtering-3d${T}WDOG1 e02: the entry ended on a WDOG1 reset (rc=0 verdict=done_with_resets resets=1 back_after=612s)
Hang${T}e00${T}gc-hang-srgb-blend${T}Hang: e00: the entry hung (rc=1 verdict=no_record resets=absent back_after=1500s); last case dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.rgb_func_alpha_func.src.one" ] ||
  fail "made-up slice: $out"
[ "$(run "$tmp/slice/README.md")" = "$out" ] || fail "a README path and its slice directory differ"

# By entry without a case: every overlapping hang id of the entry, in map order; the last case picks one.
printf -- '- Hang: e00: the entry hung (x)\n- Hang: e00: the entry hung (x); last case dEQP-GLES3.functional.fragment_ops.depth.depth_test_func.less\n' > "$tmp/e00.md"
out=$(run "$tmp/e00.md") || fail "e00"
echo "$out" | sed -n 1p | grep -q "^Hang${T}e00${T}gc-hang-srgb-blend,gc-hang-depth${T}" || fail "e00 by entry: $out"
echo "$out" | sed -n 2p | grep -q "^Hang${T}e00${T}gc-hang-depth${T}" || fail "e00 by case: $out"
# A case no event id matches falls back to the entry.
printf -- '- Hang: e02: the entry hung (x); last case dEQP-GLES3.functional.other.x\n' > "$tmp/fb.md"
run "$tmp/fb.md" | grep -q "^Hang${T}e02${T}gc-hang-filtering-3d${T}" || fail "case fallback to entry"
# A Hang line never takes a wdog1 id; a WDOG1 line with no wdog1 id falls back to the hang ids.
printf -- '- Hang: e02: x\n- WDOG1 e00: x\n' > "$tmp/k.md"
out=$(run "$tmp/k.md") || fail "kinds"
echo "$out" | sed -n 1p | grep -q "^Hang${T}e02${T}gc-hang-filtering-3d${T}" || fail "Hang took a wdog1 id: $out"
echo "$out" | sed -n 2p | grep -q "^WDOG1${T}e00${T}gc-hang-srgb-blend,gc-hang-depth${T}" || fail "WDOG1 fallback: $out"
# Only non-event ids overlap: unclassified. Other lines are ignored.
printf 'text\n- Hang: e01: the entry hung (x)\n- e01 passed\n' > "$tmp/u.md"
[ "$(run "$tmp/u.md")" = "Hang${T}e01${T}unclassified${T}Hang: e01: the entry hung (x)" ] || fail "unclassified"
# Refusals: an entry outside the table, a missing README, a malformed map line, no argument.
printf -- '- Hang: e04: x\n' > "$tmp/o.md"
if run "$tmp/o.md" >/dev/null 2>&1; then fail "entry outside the table accepted"; fi
if run "$tmp/missing.md" >/dev/null 2>&1; then fail "missing README accepted"; fi
printf 'gc-hang-x\n' > "$tmp/bad.tsv"
if python3 -I "$tool" "$tmp/slice" --causes "$tmp/bad.tsv" --table "$tmp/t.txt" >/dev/null 2>&1; then
  fail "malformed map accepted"; fi
if python3 -I "$tool" >/dev/null 2>&1; then fail "no argument accepted"; fi

# The committed map and table: the WDOG1 e58 line gets a gc-wdog1-* id, the Hang e13 and e57 lines their hang ids.
printf -- '- WDOG1 e58: the entry ended on a WDOG1 reset (x)\n- Hang: e13: the entry hung (x)\n- Hang: e57: the entry hung (x)\n' > "$tmp/r.md"
out=$(python3 -I "$tool" "$tmp/r.md") || fail "committed map"
echo "$out" | sed -n 1p | grep -q "^WDOG1${T}e58${T}gc-wdog1-[^,${T}]*${T}" || fail "committed e58: $out"
echo "$out" | sed -n 2p | grep -q "^Hang${T}e13${T}gc-gpu-hang-fbo-srgb-blend${T}" || fail "committed e13: $out"
echo "$out" | sed -n 3p | grep -q "^Hang${T}e57${T}gc-gpu-hang-filtering-2d-array${T}" || fail "committed e57: $out"

echo "imx8mp-gles3-causes-selftest: ok"
