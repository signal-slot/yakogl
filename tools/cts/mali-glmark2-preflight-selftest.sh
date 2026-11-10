#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of tools/cts/mali-glmark2-preflight.sh without a board: a fake
# ssh on PATH (testdata/mali-glmark2-preflight/bin/ssh) serves canned outputs
# from testdata/mali-glmark2-preflight/<case>/ over the "good" case (a case
# directory holds only the files it changes). It covers good, glibc-too-old,
# no-panthor-node, system-libegl-present, ssh-unreachable, dmesg-denied, a
# --build whose BUILD-INFO needs more than the board has, and --dry-run making
# no ssh call. Every command the script sends is checked to be a reading one
# (a first-word allowlist, no redirection to a file, no write verb); a mutant
# of the script that sends a writing command must fail that check.
#
#   tools/cts/mali-glmark2-preflight-selftest.sh   prints "mali-glmark2-preflight-selftest: N checks passed"
#
# Host only, a few seconds.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
FIX=$HERE/testdata/mali-glmark2-preflight
SCRIPT=$HERE/mali-glmark2-preflight.sh
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mali-glmark2-preflight-selftest-XXXXXX")
trap 'rm -rf "$tmp"' EXIT
passed=0 failed=0
check() { # <what> <command...>
  local what=$1
  shift
  if "$@" > /dev/null 2>&1; then
    passed=$((passed + 1))
    echo "  ok    $what"
  else
    failed=$((failed + 1))
    echo "  FAIL  $what"
  fi
}

mkdir -p "$tmp/bin"
cp "$FIX/bin/ssh" "$tmp/bin/ssh"
chmod +x "$tmp/bin/ssh"

# run_case <name> <case> [args...]: the script under test (UNDER, default the
# real one) on a case; output to $tmp/<name>.out, rc to $tmp/<name>.rc, the
# commands sent to $tmp/<name>.log
run_case() {
  local name=$1 case=$2 script=${UNDER:-$SCRIPT}
  shift 2
  rm -rf "$tmp/case-$name"
  mkdir -p "$tmp/case-$name"
  cp "$FIX"/good/* "$tmp/case-$name/"
  [ "$case" = good ] || cp "$FIX/$case"/* "$tmp/case-$name/"
  : > "$tmp/$name.log"
  env PATH="$tmp/bin:$PATH" BOARD_HOST=root@board.invalid FAKE_CASE="$tmp/case-$name" FAKE_LOG="$tmp/$name.log" \
    bash "$script" "$@" > "$tmp/$name.out" 2>&1
  echo $? > "$tmp/$name.rc"
}
rc_is() { [ "$(cat "$tmp/$1.rc")" = "$2" ]; }
has() { grep -qF -- "$2" "$tmp/$1.out"; }
lacks() { ! grep -qF -- "$2" "$tmp/$1.out"; }

# reading_only <log>: every command is on the allowlist and writes nothing
reading_only() {
  local cmd stripped first
  [ -s "$1" ] || return 1
  while IFS= read -r cmd; do
    stripped=${cmd//2>&1/}
    stripped=${stripped//2>\/dev\/null/}
    stripped=${stripped//>\/dev\/null/}
    case $stripped in *'>'*) echo "redirection: $cmd" >&2; return 1 ;; esac
    if grep -qwE 'rm|mv|cp|mkdir|rmdir|touch|tee|dd|truncate|ln|chmod|chown|mount|umount|modprobe|insmod|rmmod|depmod|systemctl|service|tailscale|sshd|fw_setenv|setenv|mkfs|fdisk|parted|sfdisk|sgdisk|reboot|poweroff|kexec|sysctl|sed -i|apt|apt-get|dpkg|wget|curl' <<< "$stripped"; then
      echo "write verb: $cmd" >&2
      return 1
    fi
    first=${stripped%% *}
    case $first in
      echo | cat | uname | grep | test | basename | d=/* | df | dmesg | ldconfig) ;;
      *) echo "not allowlisted: $cmd" >&2; return 1 ;;
    esac
  done < "$1"
}

# --- good
run_case good good
check "good: exit 0" rc_is good 0
for c in ssh os-release uname glibc glibcxx cxxabi render-node panthor-module stage-space boot-space dmesg system-egl; do
  check "good: $c line" has good "preflight: $c: ok"
done
check "good: summary" has good "preflight: summary: 12 ok, 0 FAIL, 0 warn"
check "good: glibc detail" has good "GLIBC_2.41 (floor 2.41)"
check "good: reading commands only" reading_only "$tmp/good.log"
check "good: no host name in output" lacks good "board.invalid"

# --- glibc too old
run_case old glibc-too-old
check "glibc-too-old: exit 1" rc_is old 1
check "glibc-too-old: glibc FAIL" has old "preflight: glibc: FAIL GLIBC_2.36 < floor 2.41"
check "glibc-too-old: other checks still run" has old "preflight: render-node: ok"

# --- no panthor node
run_case nonode no-panthor-node
check "no-panthor-node: exit 1" rc_is nonode 1
check "no-panthor-node: render-node FAIL" has nonode "preflight: render-node: FAIL /dev/dri/renderD128 does not exist"
check "no-panthor-node: panthor-module FAIL" has nonode "preflight: panthor-module: FAIL"

# --- the node belongs to another driver
run_case wrongdrv wrong-driver
check "wrong-driver: exit 1" rc_is wrongdrv 1
check "wrong-driver: render-node FAIL" has wrongdrv "belongs to 'etnaviv', not panthor"

# --- a system libEGL/libGLESv2 in the loader cache
run_case egl system-libegl-present
check "system-libegl-present: exit 0 (a warning)" rc_is egl 0
check "system-libegl-present: warn names the libs" has egl "preflight: system-egl: warn present (libEGL.so.1 libGLESv2.so.2"
check "system-libegl-present: summary counts it" has egl "0 FAIL, 1 warn"

# --- ssh unreachable
run_case lost ssh-unreachable
check "ssh-unreachable: exit 1" rc_is lost 1
check "ssh-unreachable: ssh FAIL" has lost "preflight: ssh: FAIL"
check "ssh-unreachable: nothing else probed" lacks lost "preflight: os-release"

# --- dmesg not readable
run_case dm dmesg-denied
check "dmesg-denied: exit 1" rc_is dm 1
check "dmesg-denied: dmesg FAIL" has dm "preflight: dmesg: FAIL"

# --- a build: its floors against the board's
mkdir -p "$tmp/build-ok/lib" "$tmp/build-new/lib" "$tmp/build-nonames"
for d in build-ok build-new; do : > "$tmp/$d/lib/libEGL.so"; : > "$tmp/$d/lib/libGLESv2.so"; done
printf 'commit=x\nglibc_max=2.38\nglibcxx_max=3.4.30\ncxxabi_max=1.3.14\n' > "$tmp/build-ok/BUILD-INFO"
printf 'commit=x\nglibc_max=2.41\nglibcxx_max=3.4.34\ncxxabi_max=none\n' > "$tmp/build-new/BUILD-INFO"
run_case bok good --build "$tmp/build-ok"
check "build-ok: exit 0" rc_is bok 0
check "build-ok: GLIBCXX line" has bok "preflight: build-GLIBCXX: ok needs GLIBCXX_3.4.30, board has GLIBCXX_3.4.33"
run_case bnew good --build "$tmp/build-new"
check "build-new: exit 1" rc_is bnew 1
check "build-new: GLIBCXX FAIL" has bnew "preflight: build-GLIBCXX: FAIL needs GLIBCXX_3.4.34, board has GLIBCXX_3.4.33"
check "build-new: cxxabi none is ok" has bnew "preflight: build-CXXABI: ok needs no CXXABI version"
check "build-new: reading commands only" reading_only "$tmp/bnew.log"

# --- a build without the unversioned EGL/GLES names fails system-egl
cp "$tmp/build-ok/BUILD-INFO" "$tmp/build-nonames/"
run_case bnames system-libegl-present --build "$tmp/build-nonames"
check "build-egl-names: exit 1" rc_is bnames 1
check "build-egl-names: FAIL" has bnames "preflight: build-egl-names: FAIL lib/ lacks libEGL.so libGLESv2.so"
check "build-egl-names: system-egl FAIL" has bnames "preflight: system-egl: FAIL present"
run_case bnames2 system-libegl-present --build "$tmp/build-ok"
check "build-egl-names: ok with the names" has bnames2 "preflight: build-egl-names: ok"
check "build-egl-names: system-egl not FAIL" lacks bnames2 "preflight: system-egl: FAIL"

# --- no host: usage error, no ssh
: > "$tmp/nohost.log"
env -u BOARD_HOST PATH="$tmp/bin:$PATH" FAKE_CASE="$tmp/case-good" FAKE_LOG="$tmp/nohost.log" bash "$SCRIPT" > "$tmp/nohost.out" 2>&1
echo $? > "$tmp/nohost.rc"
check "no host: exit 2" rc_is nohost 2
check "no host: no ssh call" test ! -s "$tmp/nohost.log"

# --- --dry-run: the commands, no ssh, no host
: > "$tmp/dry.log"
env -u BOARD_HOST PATH="$tmp/bin:$PATH" FAKE_CASE="$tmp/case-good" FAKE_LOG="$tmp/dry.log" bash "$SCRIPT" --dry-run > "$tmp/dry.out" 2>&1
echo $? > "$tmp/dry.rc"
check "dry-run: exit 0" rc_is dry 0
check "dry-run: no ssh call" test ! -s "$tmp/dry.log"
check "dry-run: prints the commands" has dry "+ ssh \$BOARD_HOST 'uname -r'"
# the printed commands go through the same reading-only check
sed -n "s/^+ ssh \\\$BOARD_HOST '\\(.*\\)'\$/\\1/p" "$tmp/dry.out" > "$tmp/dry-cmds.log"
check "dry-run: printed commands are reading ones" reading_only "$tmp/dry-cmds.log"

# --- the negative cases: a mutant of the script that writes must be caught
mutant() { # <name> <sed expression>
  cp "$SCRIPT" "$tmp/mut-$1.sh"
  sed -i "$2" "$tmp/mut-$1.sh"
  ln -sf "$HERE/mali-kmsg.py" "$tmp/mali-kmsg.py"
  UNDER=$tmp/mut-$1.sh run_case "mut-$1" good
}
rejected() { ! reading_only "$1"; }
mutant rm "s|^C_UNAME='uname -r'|C_UNAME='uname -r; rm -f /root/yakogl-mali/x'|"
check "mutant rm: the write check catches it" rejected "$tmp/mut-rm.log"
mutant redirect "s|^C_NODE='test -c /dev/dri/renderD128 \&\& echo present'|C_NODE='test -c /dev/dri/renderD128 \&\& echo present > /boot/x'|"
check "mutant redirect: the write check catches it" rejected "$tmp/mut-redirect.log"
mutant modprobe "s|^C_MODULE='|C_MODULE='modprobe panthor; |"
check "mutant modprobe: the write check catches it" rejected "$tmp/mut-modprobe.log"
check "the real script is not rejected (control)" reading_only "$tmp/good.log"

# --- no address or credential in the script or the testdata
check "no address or credential in the files" bash -c '! grep -rEn "[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+|password|ts\.net" "$1" "$2"' _ "$SCRIPT" "$FIX"

if [ "$failed" != 0 ]; then
  for f in "$tmp"/*.out; do
    echo "--- ${f##*/}"
    cat "$f"
  done
fi
echo "mali-glmark2-preflight-selftest: $passed checks passed, $failed failed"
[ "$failed" = 0 ]
