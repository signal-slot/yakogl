#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# decode71.sh: the gl_test-shape references tools/v3d/pi5/gl_*.clif (Mesa's
# V3D_DEBUG=clif on the drm-shim at 7.1.10, tools/v3d/drm_shim_ref.sh run with
# tools/v3d/pi5/src/gl_shapes.py) read with the V3D 7.1 packet spec of Mesa's
# CLE decoder, the check tools/v3d/check71.d/shim71.sh makes of shim71/:
# every packet each ctrllist names is a command of the 7.1 spec and no list is
# empty. The checker is the one shim71.sh builds (build/v3d71/shim71), against
# the vendored reference build (tools/v3d/mesa_ref.sh) or MESA_V3D_TREE. Also:
# each .stdout says the shim reported V3D 7.1.10.2 and glGetError 0, and each
# file has as many jobs (one @wait_render_all_cores per submitted job) as its
# shape's glFinish calls make (EXPECTED below).
# Exit 0 when every file passes; 1 otherwise; 77 when shim71.sh is a gap.
set -u
cd "$(dirname "$0")/../../../.." || exit 1

# shape -> jobs (gl_shapes.py's docstring; one job per glFinish of the case)
EXPECTED=(scissor:2 scisbox:3 writemask:4 viewport:3 fbods:9 colormask:6 mrt4:1 mrt8:1)

out=$(bash tools/v3d/check71.d/shim71.sh 2>&1)
rc=$?
if [ $rc -ne 0 ]; then
  printf '%s\n' "$out"
  exit $rc
fi
chk=build/v3d71/shim71/shim71_check
[ -x "$chk" ] || { echo "decode71: $chk was not built"; exit 1; }

failed=0
files=()
for e in "${EXPECTED[@]}"; do
  shape=${e%%:*} want=${e#*:}
  c=tools/v3d/pi5/gl_$shape.clif
  s=tools/v3d/pi5/gl_$shape.stdout
  files+=("$c")
  grep -q '^gl_shapes: renderer V3D 7\.1\.10\.2$' "$s" ||
    { echo "FAIL: $s does not say the shim reported V3D 7.1.10.2"; failed=1; }
  grep -q "^gl_shapes: $shape, framebuffer 64x64, glGetError 0x0$" "$s" ||
    { echo "FAIL: $s does not end in glGetError 0x0"; failed=1; }
  jobs=$(grep -c '^MESA: info: @wait_render_all_cores$' "$c")
  [ "$jobs" = "$want" ] ||
    { echo "FAIL: $c has $jobs jobs, $want expected"; failed=1; }
  echo "decode71: gl_$shape: $jobs jobs"
done
"$chk" "${files[@]}" || failed=1
exit $failed
