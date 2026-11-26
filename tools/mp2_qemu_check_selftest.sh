#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2_qemu_check_selftest.sh -- self-test of tools/mp2_qemu_check.sh.
#
# Runs the helper under env -i with a fake PATH of stubs in a directory under
# $TMPDIR (removed on exit), present and missing, and checks every line and
# the exit code. It also checks that the helper's host-tool list is run.sh's
# own. Prints "mp2_qemu_check_selftest: N ok, M failed"; exit 0 when M is 0.
set -uo pipefail

case "${BASH_SOURCE[0]}" in */*) here=${BASH_SOURCE[0]%/*} ;; *) here=. ;; esac
here=$(cd "$here" && pwd) || exit 2
helper=$here/mp2_qemu_check.sh
run_sh=$here/../driver/tools/uboot-mp2/qemu/run.sh
bash_bin=$(command -v bash)

work=$(mktemp -d "${TMPDIR:-/tmp}/mp2_qemu_check_selftest.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT

ok=0 failed=0
check() { # NAME COND-EXIT-STATUS [DETAIL]
  if [ "$2" -eq 0 ]; then
    ok=$((ok + 1))
  else
    failed=$((failed + 1))
    echo "FAIL: $1${3:+: $3}"
  fi
}

stub() { # PATH...
  local f
  for f in "$@"; do
    mkdir -p "${f%/*}"
    printf '#!/bin/sh\nexit 0\n' > "$f"
    chmod +x "$f"
  done
}

# base: what the helper itself needs besides bash builtins (build-uboot.sh's
# top level runs dirname). None of the row's inputs is in it.
base=$work/base
mkdir -p "$base"
for t in dirname sed; do ln -s "$(command -v "$t")" "$base/$t"; done

all=$work/all
stub "$all"/{qemu-system-aarch64,aarch64-linux-gnu-gcc,aarch64-linux-gnu-objcopy,python3,mke2fs,sfdisk,truncate,sha256sum,dd,git,make}

names=(qemu-system-aarch64 aarch64-cross python3 mke2fs sfdisk truncate sha256sum dd git make)

out="" rc=0
run() { # PATH [VAR=VALUE...]
  local p=$1
  shift
  rc=0
  out=$(env -i HOME="$work" PATH="$p" MP2_QEMU_CHECK_SBIN="${SBIN-}" "$@" "$bash_bin" "$helper" 2>&1) || rc=$?
}
status_of() { # NAME -> "ok" / "missing" / ""
  printf '%s\n' "$out" | while read -r s n _; do [ "$n" = "$1" ] && echo "$s"; done
}
detail_of() { # NAME -> the third column
  printf '%s\n' "$out" | while read -r s n d; do [ "$n" = "$1" ] && echo "$d"; done
}
expect() { # CASE NAME STATUS [DETAIL-PREFIX]
  local s d
  s=$(status_of "$2")
  [ "$s" = "$3" ]
  check "$1: $2 is $3" $? "got '$s' in:"$'\n'"$out"
  if [ -n "${4-}" ]; then
    d=$(detail_of "$2")
    [ "${d#"$4"}" != "$d" ]
    check "$1: $2 detail starts with $4" $? "got '$d'"
  fi
}
expect_rc() { # CASE RC
  [ "$rc" -eq "$2" ]
  check "$1: exit $2" $? "got $rc"
}
expect_lines() { # CASE: one line per input, in order, ok/missing only
  local got
  got=$(printf '%s\n' "$out" | while read -r s n _; do echo "$n"; done | paste -sd' ')
  [ "$got" = "${names[*]}" ]
  check "$1: one line per input in order" $? "got '$got'"
  ! printf '%s\n' "$out" | grep -Ev '^(ok|missing) +[^ ]+ +[^ ]' > /dev/null
  check "$1: every line is 'ok|missing NAME DETAIL'" $? "$out"
}

# 1. Everything present on PATH.
SBIN=""
run "$all:$base"
expect_rc all 0
expect_lines all
for n in qemu-system-aarch64 python3 mke2fs sfdisk truncate sha256sum dd git make; do
  expect all "$n" ok "$all/$n"
done
expect all aarch64-cross ok "aarch64-linux-gnu-"
[ "$(printf '%s\n' "$out" | grep -c '^ok ')" -eq "${#names[@]}" ]
check "all: ${#names[@]} ok lines" $?

# 2. Nothing present.
run "$base"
expect_rc none 1
expect_lines none
for n in "${names[@]}"; do expect none "$n" missing; done
expect none qemu-system-aarch64 missing "not on PATH and MP2_QEMU unset"

# 3. One host tool missing: only its line turns missing, and exit 1.
for t in python3 mke2fs sfdisk truncate sha256sum dd git make; do
  one=$work/one-$t
  rm -rf "$one"
  mkdir -p "$one"
  for f in "$all"/*; do [ "${f##*/}" = "$t" ] || ln -s "$f" "$one/"; done
  run "$one:$base"
  expect_rc "no-$t" 1
  expect "no-$t" "$t" missing
  [ "$(printf '%s\n' "$out" | grep -c '^missing ')" -eq 1 ]
  check "no-$t: exactly one missing line" $? "$out"
done

# 4. mke2fs and sfdisk only in an sbin directory (run.sh's /usr/sbin, /sbin).
part=$work/part
mkdir -p "$part"
for f in "$all"/*; do case "${f##*/}" in mke2fs | sfdisk) ;; *) ln -s "$f" "$part/" ;; esac; done
stub "$work/sbin/mke2fs" "$work/sbin2/sfdisk"
SBIN="$work/sbin $work/sbin2"
run "$part:$base"
expect_rc sbin 0
expect sbin mke2fs ok "$work/sbin/mke2fs"
expect sbin sfdisk ok "$work/sbin2/sfdisk"
SBIN=""
run "$part:$base"
expect_rc no-sbin 1
expect no-sbin mke2fs missing
expect no-sbin sfdisk missing

# 5. MP2_QEMU: an executable off PATH is ok, a non-executable one is missing
#    even with qemu-system-aarch64 on PATH (run.sh takes MP2_QEMU first).
noq=$work/noq
mkdir -p "$noq"
for f in "$all"/*; do [ "${f##*/}" = qemu-system-aarch64 ] || ln -s "$f" "$noq/"; done
stub "$work/qemu/bin/qemu-system-aarch64"
run "$noq:$base" MP2_QEMU="$work/qemu/bin/qemu-system-aarch64"
expect_rc mp2_qemu 0
expect mp2_qemu qemu-system-aarch64 ok "$work/qemu/bin/qemu-system-aarch64 (MP2_QEMU)"
printf 'not a program\n' > "$work/qemu/plain"
run "$all:$base" MP2_QEMU="$work/qemu/plain"
expect_rc mp2_qemu-bad 1
expect mp2_qemu-bad qemu-system-aarch64 missing "MP2_QEMU=$work/qemu/plain is not executable"

# 6. The cross prefix: the Zephyr SDK fallbacks, CROSS_COMPILE, objcopy.
nocc=$work/nocc
mkdir -p "$nocc"
for f in "$all"/*; do case "${f##*/}" in aarch64-*) ;; *) ln -s "$f" "$nocc/" ;; esac; done
sdk=$work/sdk/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-
stub "${sdk}gcc" "${sdk}objcopy"
run "$nocc:$base" ZEPHYR_SDK_INSTALL_DIR="$work/sdk"
expect_rc sdk 0
expect sdk aarch64-cross ok "$sdk"
deps=$work/deps/zephyr-sdk-0.17.0/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-
stub "${deps}gcc" "${deps}objcopy"
run "$nocc:$base" CONNECTED_DEPS_DIR="$work/deps"
expect_rc deps 0
expect deps aarch64-cross ok "$deps"
run "$nocc:$base"
expect_rc no-cross 1
expect no-cross aarch64-cross missing "no aarch64-linux-gnu-gcc"
stub "$work/cc/bin/my-aarch64-gcc" "$work/cc/bin/my-aarch64-objcopy"
run "$nocc:$base:$work/cc/bin" CROSS_COMPILE=my-aarch64-
expect_rc cross_compile 0
expect cross_compile aarch64-cross ok "my-aarch64-"
run "$all:$base" CROSS_COMPILE=nosuch-
expect_rc cross_compile-bad 1
expect cross_compile-bad aarch64-cross missing "CROSS_COMPILE=nosuch-"
rm "$work/cc/bin/my-aarch64-objcopy"
run "$nocc:$base:$work/cc/bin" CROSS_COMPILE=my-aarch64-
expect_rc no-objcopy 1
expect no-objcopy aarch64-cross missing "my-aarch64-gcc is there but my-aarch64-objcopy is not"

# 7. An argument is a usage error (exit 2); --help prints the header.
rc=0
env -i HOME="$work" PATH="$all:$base" "$bash_bin" "$helper" --bogus > /dev/null 2>&1 || rc=$?
expect_rc usage 2
rc=0
out=$(env -i HOME="$work" PATH="$all:$base" "$bash_bin" "$helper" --help 2>&1) || rc=$?
expect_rc help 0
printf '%s\n' "$out" | grep -q 'mp2_qemu_check.sh -- report the inputs'
check "help: prints the header" $?

# 8. The helper's host-tool list is run.sh's ("for t in ...; do" line).
want=$(sed -n 's/^for t in \(.*\); do$/\1/p' "$run_sh")
have=$(sed -n 's/^for t in \(.*\); do$/\1/p' "$helper")
[ -n "$want" ] && [ "$want" = "$have" ]
check "the host tools are run.sh's own" $? "run.sh '$want', helper '$have'"
grep -q 'command -v qemu-system-aarch64' "$run_sh" && grep -q 'MP2_QEMU:-' "$run_sh"
check "run.sh still takes MP2_QEMU, else qemu-system-aarch64 on PATH" $?
grep -q 'cross=$(mp2_qemu_cross)' "$run_sh" && grep -q 'objcopy' "$run_sh"
check "run.sh still takes mp2_qemu_cross and the prefix's objcopy" $?
grep -q '\[ -x "/usr/sbin/$t" \] || \[ -x "/sbin/$t" \]' "$run_sh"
check "run.sh still looks in /usr/sbin and /sbin" $?

# 9. Run from its own directory by a bare file name: it still finds run.sh's
#    build-uboot.sh.
rc=0
out=$(cd "$here" && env -i HOME="$work" PATH="$all:$base" MP2_QEMU_CHECK_SBIN= "$bash_bin" mp2_qemu_check.sh 2>&1) || rc=$?
expect_rc bare-name 0
expect bare-name aarch64-cross ok "aarch64-linux-gnu-"

echo "mp2_qemu_check_selftest: $ok ok, $failed failed"
[ "$failed" -eq 0 ]
