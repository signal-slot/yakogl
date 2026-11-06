#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# ident71: tools/v3d/ident71.py decodes the V3D identity words: the Pi 4's
# measured 4.2 words (docs/notes/pi4-v3d-bringup.md) give 4.2.14.0, 1 core, 2
# slices x 4 QPUs, VPM 65536 bytes, PA/VA 35 bits and Mesa's 4.2 variant; words
# built from the documented 7.1 layout (not a measurement: Mesa's drm-shim
# reports 7.1.7.0, and the slice, VPM and width values are chosen to exercise
# every field) give ver 71 and Mesa's 7.1 variant (draw index from rev 10,
# unpack max0 from rev 7); the three input forms (NAME=0xHEX lines, the
# v3d_hw.c one-line log, a debugfs revision) agree; --compare exits 1 on a
# difference and 0 on equal words. A D3 ident under tools/v3d/pi5/ (any file
# named *ident*) is decoded as a further vector and must be a 7.1.
# No Mesa reference build is needed. Convention: tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=ident71
dir=build/v3d71/$name
mkdir -p "$dir" || exit 1
py=tools/v3d/ident71.py
fail=0 n=0

bad() { echo "FAIL: $*"; fail=$((fail + 1)); }
# want FILE FIELD VALUE
want() {
  n=$((n + 1))
  local got
  got=$(python3 -I "$py" --field "$2" "$1" 2>&1)
  [ "$got" = "$3" ] || bad "$1: $2 is '$got', want '$3'"
}
# cmp_rc WANT A B
cmp_rc() {
  n=$((n + 1))
  python3 -I "$py" --compare "$2" "$3" >"$dir/cmp.out" 2>&1
  local rc=$?
  [ "$rc" = "$1" ] || { bad "--compare $2 $3 exits $rc, want $1"; cat "$dir/cmp.out"; }
}

printf '%s\n' HUB_IDENT0=0x42554856 HUB_IDENT1=0x000e1124 HUB_IDENT2=0x00000100 \
  HUB_IDENT3=0x00000e00 CORE_IDENT0=0x04443356 CORE_IDENT1=0x81001422 \
  CORE_IDENT2=0x40078121 MMU_DEBUG_INFO=0x550 >"$dir/v42.txt"
printf '%s\n' HUB_IDENT0=0x42554856 HUB_IDENT1=0x000e0117 HUB_IDENT2=0x00000100 \
  HUB_IDENT3=0x00000700 CORE_IDENT0=0x07443356 CORE_IDENT1=0x80002441 \
  CORE_IDENT2=0x40078121 MMU_DEBUG_INFO=0x650 >"$dir/v71.txt"
# the same 7.1 words as v3d_hw.c logs them (CTL_ names, no 0x)
echo "V3D raw: HUB_IDENT0..3 42554856 000e0117 00000100 00000700 CTL_IDENT0..2 07443356 80002441 40078121 MMU_DEBUG_INFO 00000650" >"$dir/v71_log.txt"
# a 7.1.10 part, to see the draw-index step
sed 's/^HUB_IDENT3=.*/HUB_IDENT3=0x00000a00/' "$dir/v71.txt" >"$dir/v71_10.txt"
# a debugfs v3d_ident style revision line and a v3d_regs style dump
printf 'Revision:   4.2.14.0\nMMU:        yes\n' >"$dir/v42_debugfs.txt"
printf 'V3D_HUB_IDENT1 (0x0c): 0x000e1124\nV3D_CTL_0_IDENT0 (0x00): 0x04443356\nV3D_CTL_0_IDENT1 (0x04): 0x81001422\n' >"$dir/v42_regs.txt"

want "$dir/v42.txt" ver 42
want "$dir/v42.txt" hub.magic VHUB
want "$dir/v42.txt" core.magic V3D
want "$dir/v42.txt" hub.ncores 1
want "$dir/v42.txt" hub.with_tfu 1
want "$dir/v42.txt" hub.iprev 14
want "$dir/v42.txt" hub.compat_rev 0
want "$dir/v42.txt" core.nslc 2
want "$dir/v42.txt" core.qups 4
want "$dir/v42.txt" core.qpu_count 8
want "$dir/v42.txt" core.ntmu 1
want "$dir/v42.txt" core.vpm_bytes 65536
want "$dir/v42.txt" mmu.pa_width 35
want "$dir/v42.txt" mmu.va_width 35
want "$dir/v42.txt" mesa.has_accumulators 1
want "$dir/v42.txt" mesa.needs_default_attribute_values 1
want "$dir/v42.txt" mesa.cle_readahead 256
want "$dir/v42.txt" mesa.max_render_targets 4
want "$dir/v42.txt" mesa.draw_index 0

want "$dir/v71.txt" ver 71
want "$dir/v71.txt" hub.tver 7
want "$dir/v71.txt" hub.rev 1
want "$dir/v71.txt" hub.iprev 7
want "$dir/v71.txt" core.qpu_count 16
want "$dir/v71.txt" core.vpm_bytes 65536
want "$dir/v71.txt" mmu.pa_width 36
want "$dir/v71.txt" mesa.has_accumulators 0
want "$dir/v71.txt" mesa.needs_default_attribute_values 0
want "$dir/v71.txt" mesa.clipper_xy_granularity 64
want "$dir/v71.txt" mesa.cle_readahead 1024
want "$dir/v71.txt" mesa.max_render_targets 8
want "$dir/v71.txt" mesa.max_framebuffer_size 7680
want "$dir/v71.txt" mesa.unpack_max0 1
want "$dir/v71.txt" mesa.draw_index 0
want "$dir/v71_10.txt" mesa.draw_index 1
want "$dir/v71_log.txt" raw.CORE_IDENT1 0x80002441
want "$dir/v71_log.txt" mmu.pa_width 36
want "$dir/v42_debugfs.txt" hub.iprev 14
want "$dir/v42_debugfs.txt" hub.ver 42
want "$dir/v42_regs.txt" core.qpu_count 8

cmp_rc 0 "$dir/v71.txt" "$dir/v71_log.txt"
cmp_rc 0 "$dir/v42.txt" "$dir/v42_debugfs.txt"
cmp_rc 0 "$dir/v42.txt" "$dir/v42_regs.txt"
cmp_rc 1 "$dir/v42.txt" "$dir/v71.txt"
cmp_rc 1 "$dir/v71.txt" "$dir/v71_10.txt"
python3 -I "$py" --compare "$dir/v71.txt" "$dir/v71_10.txt" | grep -q 'hub.iprev.*DIFF' ||
  bad "--compare does not mark the differing field"
n=$((n + 1))
: >"$dir/empty.txt"
python3 -I "$py" "$dir/empty.txt" >/dev/null 2>&1
[ $? = 2 ] || bad "an input with no identity words does not exit 2"

# D3's ident under tools/v3d/pi5/, when it is there
for f in tools/v3d/pi5/*ident*; do
  [ -f "$f" ] || continue
  want "$f" ver 71
  n=$((n + 1))
  python3 -I "$py" "$f" >"$dir/pi5.out" 2>&1 || bad "$f does not decode"
done

echo "$name: $n vectors, $fail failed"
[ "$fail" = 0 ]
