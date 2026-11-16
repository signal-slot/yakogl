#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# rpi5_scanout_all: all Pi 5 display headers in one place.
#
# (1) driver/zephyr/lib/board/test/rpi5_scanout_all.c includes every header
#     of driver/zephyr/lib/board/rpi5_scanout/ in one translation unit
#     (twice: a header without its include guard clashes in the second pass)
#     and its main touches one symbol of each. It compiles with the host cc
#     under -std=c11 -Wall -Wextra -Werror (linked; the program is never
#     run), and for aarch64 under -std=c11 -ffreestanding -Wall -Wextra
#     -Werror -c: a duplicate macro, a clashing type or a missing include
#     guard between the headers shows as a FAIL naming the compiler output.
#     The script first checks that the .c file lists every header found in
#     the directory: a header added without a line in the .c file is a FAIL.
# (2) The aarch64 compiler is the first of: aarch64-zephyr-elf-gcc under
#     ZEPHYR_SDK_INSTALL_DIR (the two SDK layouts, as xbuild71.sh looks them
#     up), else aarch64-linux-gnu-gcc. A candidate is usable when a trivial
#     probe TU (the C headers these headers include) compiles with the same
#     flags: a cross compiler without a C library is not a verdict on this
#     code. No usable compiler: the run gaps (last line "gap: ...", exit 77),
#     it is not a pass.
# (3) Every tools/v3d/check71.d/<x>.sh named after a header <x>.h runs with
#     its output printed (one not executable in git runs with bash and is
#     noted: check71.sh would skip it), and the N of its result line sums
#     into this script's total. A result line is accepted as "<x>: N of N
#     checks, 0 failed", as the mock's own "<x>_mock: N of N checks, 0
#     failed", or bare "N of N checks, 0 failed" (edid.sh's form). A mock
#     script that exits 77 gaps (named, not a pass); any other non-zero
#     exit, or exit 0 without a passing result line, is a failure naming it.
#
# Run from anywhere; it also runs under plain sh. Builds into
# build/v3d71/rpi5_scanout_all/. When everything passed the last line is
# "rpi5_scanout_all: N of N checks, 0 failed" (N the sum of the mock counts)
# and the exit is 0; a failure names itself on a FAIL line and the last line
# counts and names the failed steps, exit 1; a gap does not hide a failure
# and wins alone with exit 77. Convention: tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=rpi5_scanout_all
out=build/v3d71/$name
src=driver/zephyr/lib/board/test/rpi5_scanout_all.c
hdir=driver/zephyr/lib/board/rpi5_scanout
cdir=tools/v3d/check71.d
cc=${CC:-cc}

if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
[ -f "$src" ] || { echo "$name: $src missing (run from the repository root), 1 failed"; exit 1; }
mkdir -p "$out" || exit 1

# ---- the headers and the .c file's list ----
headers=""
for h in "$hdir"/*.h; do
  [ -f "$h" ] || { echo "$name: no headers in $hdir, 1 failed"; exit 1; }
  headers="$headers $(basename "$h" .h)"
done
headers=${headers# }
[ -n "$headers" ] || { echo "$name: no headers in $hdir, 1 failed"; exit 1; }

failed_names=""
gap_names=""
checks=0
nfail() { # name a failed step (no spaces in the name: the verdict line
          # counts them as words): nfail "<what>"
  failed_names="$failed_names $1"
}

missing=""
for x in $headers; do
  grep -q "\"../rpi5_scanout/$x\.h\"" "$src" || missing="$missing $x"
done
if [ -n "$missing" ]; then
  echo "FAIL    $name: no #include of:${missing} in $src"
  nfail "include-list"
else
  n=0
  for x in $headers; do n=$((n + 1)); done
  echo "ok      include-list: $n headers in $hdir, all included twice in $src"
fi

# ---- (1) host: -std=c11 -Wall -Wextra -Werror, linked ----
if "$cc" -std=c11 -Wall -Wextra -Werror "$src" -o "$out/$name.host" 2>"$out/host.err"; then
  rm -f "$out/host.err"
  echo "ok      host C11 compile and link ($cc)"
else
  echo "FAIL    $name: host C11 compile, $cc output:"
  sed 's/^/        /' "$out/host.err"
  nfail host-c11-compile
fi

# ---- (2) aarch64: the Zephyr SDK's compiler or the Linux cross gcc ----
cands=""
if [ -n "${ZEPHYR_SDK_INSTALL_DIR:-}" ]; then
  for g in "$ZEPHYR_SDK_INSTALL_DIR/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc" \
           "$ZEPHYR_SDK_INSTALL_DIR/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc"; do
    [ -x "$g" ] && cands="$cands $g"
  done
fi
if command -v aarch64-linux-gnu-gcc >/dev/null 2>&1; then
  cands="$cands aarch64-linux-gnu-gcc"
fi

# The probe: the C headers rpi5_scanout_all.c reaches through the headers
# (stdarg, stdbool, stddef, stdint, stdio, string), compiled for the target.
cat >"$out/probe.c" <<'EOF'
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
int scanout_all_probe(void);
int scanout_all_probe(void) { return 0; }
EOF
aacc=""
for c in $cands; do
  if "$c" -std=c11 -ffreestanding -Wall -Wextra -Werror -c "$out/probe.c" \
    -o "$out/probe.o" 2>"$out/probe.err"; then
    rm -f "$out/probe.err"
    aacc=$c
    break
  fi
done
if [ -z "$aacc" ]; then
  echo "note    $name: no aarch64 compiler (ZEPHYR_SDK_INSTALL_DIR unset or"
  echo "note      without aarch64-zephyr-elf-gcc in it, aarch64-linux-gnu-gcc"
  echo "note      absent): the aarch64 compile is a gap, not a pass"
  gap_names="$gap_names ZEPHYR_SDK_INSTALL_DIR (no aarch64-zephyr-elf-gcc, no aarch64-linux-gnu-gcc)"
elif "$aacc" -std=c11 -ffreestanding -Wall -Wextra -Werror -c "$src" \
  -o "$out/$name.aarch64.o" 2>"$out/aarch64.err"; then
  rm -f "$out/aarch64.err"
  echo "ok      aarch64 compile ($aacc)"
else
  echo "FAIL    $name: aarch64 compile, $aacc output:"
  sed 's/^/        /' "$out/aarch64.err"
  nfail aarch64-compile
fi

# ---- (3) the mock of every header ----
for x in $headers; do
  s=$cdir/$x.sh
  if [ ! -f "$s" ]; then
    echo "FAIL    $name: $s missing (the header $x.h has no check script)"
    nfail "no-$x.sh"
    continue
  fi
  if [ -x "$s" ]; then
    o=$("$s" 2>&1)
  else
    echo "note    $s is not executable (check71.sh would skip it): running it with bash"
    o=$(bash "$s" 2>&1)
  fi
  rc=$?
  printf '%s\n' "$o"
  if [ "$rc" -eq 77 ]; then
    gap_names="$gap_names $x.sh"
    continue
  fi
  n=$(printf '%s\n' "$o" | tail -n 1 |
    sed -nE "s/^(($x|${x}_mock): )?([0-9]+) of \\3 checks, 0 failed\$/\\3/p")
  if [ "$rc" -ne 0 ] || [ -z "$n" ]; then
    echo "FAIL    $name: $s exited $rc without a passing result line"
    nfail "$x.sh"
  else
    checks=$((checks + n))
  fi
done

# ---- verdict ----
if [ -n "$failed_names" ]; then
  f=0
  for w in $failed_names; do f=$((f + 1)); done
  echo "$name: ${f} failed:${failed_names}, ${checks} mock checks passed"
  [ -n "$gap_names" ] && echo "note    gaps:${gap_names}"
  exit 1
fi
if [ -n "$gap_names" ]; then
  echo "gap: ${gap_names# }"
  exit 77
fi
echo "$name: $checks of $checks checks, 0 failed"
exit 0
