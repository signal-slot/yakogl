#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
[ -n "${BASH_VERSION:-}" ] || exec bash "$0" "$@"
#
# gltest_rpi5: driver/samples/zephyr/gl_test/src/main_rpi5.c prints the lines
# tools/board.sh pi5 reads. The end line of the record is matched against the
# pi5 END_RE_DEFAULT taken from board.sh itself; the build line, the watchdog
# call, the verdict lines and the case names are checked in the source; and
# the V3D71 branch of the sample's CMakeLists.txt is checked to build
# main_rpi5.c and no rpi4 file the board layer already builds.
#
# Run from anywhere. Builds nothing. The last line is
# "gltest_rpi5: N checks, 0 failed"; exit 0 when all pass, 1 otherwise, 77 with
# "gap: <names>" when an input file is missing.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=gltest_rpi5
src=driver/samples/zephyr/gl_test/src/main_rpi5.c
cm=driver/samples/zephyr/gl_test/CMakeLists.txt
bs=tools/board.sh

gap=
for f in $src $cm $bs; do
  [ -f "$f" ] || gap="$gap $f"
done
if [ -n "$gap" ]; then
  echo "gap:$gap"
  exit 77
fi

total=0
fail=0
ok() { total=$((total + 1)); }
bad() { total=$((total + 1)); fail=$((fail + 1)); echo "$name: FAIL: $1"; }
need() { # need <description> <fixed string in source>
  if grep -qF -- "$2" "$src"; then ok; else bad "$1 ($2 not in $src)"; fi
}

# END_RE_DEFAULT of the pi5 arm of board.sh: the first assignment after the
# "pi5)" case label that mentions v3d_gl_test.
re=$(awk '/^ *pi5\)/ {p=1} p && /END_RE_DEFAULT=.*v3d_gl_test RECORD end/ {print; exit}' "$bs" |
  sed -e "s/^[^']*'//" -e "s/'.*$//")
if [ -z "$re" ]; then
  bad "no END_RE_DEFAULT with 'v3d_gl_test RECORD end' in the pi5 arm of $bs"
else
  ok
  # the line main_rpi5.c prints, with a build string filled in
  if printf 'v3d_gl_test RECORD end, build abc1234\n' | grep -qE -- "$re"; then ok
  else bad "the end line does not match board.sh pi5's END_RE_DEFAULT"; fi
  need "the end line's format" 'v3d_gl_test RECORD end, build %s\n'
fi

need "the build line" 'v3d_gl_test: build %s'
need "the watchdog line (rpi_wdog_announce prints 'board: PM watchdog resets')" 'rpi_wdog_announce();'
grep -qF 'board: PM watchdog resets' driver/zephyr/lib/board/rpi4/rpi_wdog.c && ok ||
  bad "rpi_wdog.c no longer prints 'board: PM watchdog resets'"
need "the verdict line" 'v3d_gl_test: case %d %s\n'
need "the count line" 'v3d_gl_test: %u of %u cases passed (%lld ms)\n'
need "the PASS name" '"PASS"'
need "the FAIL name" '"FAIL"'
need "the SKIP name" '"SKIP"'
need "the record's deadline" 'INCOMPLETE'
need "the walker's hook" 'void gl_test_case_result(int c, int rc)'
need "the OSAL argument hook" 'void *gl_test_v3d_osal_arg(void)'
need "the default platform" 'osal_zephyr_v3d_default_platform()'

# a UART-only image: no framebuffer, no fbcon
if grep -qE 'fbcon|rpi_fb_alloc|__printk_hook_install' "$src"; then bad "main_rpi5.c touches the screen"; else ok; fi

# the CMake branch
branch=$(awk '/^if\(CONFIG_YAKOGL_GPU_V3D71\)/ {p=1} /^elseif\(CONFIG_YAKOGL_GPU_V3D\)/ {p=0} p' "$cm")
if [ -z "$branch" ]; then
  bad "no if(CONFIG_YAKOGL_GPU_V3D71) branch before elseif(CONFIG_YAKOGL_GPU_V3D) in $cm"
else
  ok
  grep -q 'src/main_rpi5.c' <<<"$branch" && ok || bad "the V3D71 branch does not build src/main_rpi5.c"
  grep -q 'GL_TEST_ES100=1' <<<"$branch" && ok || bad "the V3D71 branch lacks GL_TEST_ES100=1"
  grep -q 'GL_TEST_VTL_PASSES' <<<"$branch" && ok || bad "the V3D71 branch lacks GL_TEST_VTL_PASSES"
  grep -q 'V3D_BUILD' <<<"$branch" && ok || bad "the V3D71 branch lacks V3D_BUILD"
  # the board layer's rpi_wdog.c, rpi_mbox.c and fbcon.c are not sources here
  if grep -E 'target_sources' <<<"$branch" | grep -qE 'rpi_wdog\.c|rpi_mbox\.c|fbcon\.c'; then
    bad "the V3D71 branch adds an rpi4 source the board layer builds"
  else ok; fi
fi

echo "$name: $total checks, $fail failed"
[ "$fail" -eq 0 ]
