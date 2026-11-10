#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Read-only probe of the NanoPC-T6 (Armbian 26.8.10 trixie, kernel
# 6.18.55-current-rockchip64) before a glmark2-es2 run built by
# tools/cts/mali-glmark2-build.sh.
#
#   BOARD_HOST=root@<t6> tools/cts/mali-glmark2-preflight.sh [--build DIR] [--stage DIR]
#   tools/cts/mali-glmark2-preflight.sh --host root@<t6> ...
#   tools/cts/mali-glmark2-preflight.sh --dry-run        (no ssh, no host needed)
#
#   --build DIR   a mali-glmark2-build.sh output: the floors its BUILD-INFO
#                 gives (else readelf -V of bin/glmark2-es2-mali) are judged
#                 against the board's libraries, and its RUNPATH is checked
#   --stage DIR   the board's staging directory (default /root/yakogl-mali)
#   --need-kb N   free space wanted there (default 1048576)
#   --dry-run     print the commands, run no ssh
#
# The host comes from BOARD_HOST (the target:nanopc-t6 lease) or --host, never
# from a file; BOARD_SSH_OPTS (optional) goes to ssh. Every command sent is a
# reading one (cat, uname, grep, readlink, test, df, ldconfig -p, dmesg); the
# board is not written: no module, /boot, U-Boot environment, Tailscale, sshd
# or partition access, not even a file under the stage.
#
# One line per check, "preflight: <check>: ok|FAIL|warn <detail>", then a
# summary. Exit 0 when no hard check failed, 1 when one did, 2 on usage.
# Hard: ssh, os/kernel, the three floors, build floors, render node, panthor,
# stage space, dmesg readable, RUNPATH and lib/libEGL.so + libGLESv2.so of --build. Warn: /boot space, a system
# libEGL/libGLESv2 (the 'sonama trap': a wrong RUNPATH would load glvnd/Mesa
# instead of the panthor-gles pair) when no --build proves the RUNPATH.
set -uo pipefail
me=${0##*/}
HERE=$(cd "$(dirname "$0")" && pwd)
GLIBC_FLOOR=2.41 GLIBCXX_FLOOR=3.4.33 CXXABI_FLOOR=1.3.15
PROGRAM=glmark2-es2-mali
host=${BOARD_HOST-} build= stage=/root/yakogl-mali need_kb=1048576 dry=0
while [ $# -gt 0 ]; do
  case $1 in
    --host) host=${2:?}; shift 2 ;;
    --build) build=${2:?}; shift 2 ;;
    --stage) stage=${2:?}; shift 2 ;;
    --need-kb) need_kb=${2:?}; shift 2 ;;
    --dry-run) dry=1; shift ;;
    -h|--help) sed -n '5,30p' "$0"; exit 0 ;;
    *) echo "$me: unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ "$dry" = 1 ] || [ -n "$host" ] || { echo "$me: set BOARD_HOST=<user@host> (the target:nanopc-t6 lease) or --host" >&2; exit 2; }
read -r -a opts <<< "${BOARD_SSH_OPTS-}"
KMSG=$HERE/mali-kmsg.py

max_of() { sed -n "s/^$1_\([0-9][0-9.]*\)\$/\1/p" | sort -V | tail -1; }
ver_le() { [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -1)" = "$2" ]; }

nfail=0 nwarn=0 nok=0
say() { # <check> <ok|FAIL|warn> <detail>
  echo "preflight: $1: $2 $3"
  case $2 in ok) nok=$((nok + 1)) ;; FAIL) nfail=$((nfail + 1)) ;; warn) nwarn=$((nwarn + 1)) ;; esac
}
finish() {
  echo "preflight: summary: $nok ok, $nfail FAIL, $nwarn warn"
  [ "$nfail" = 0 ]
  exit $?
}

# rssh <command>: one reading command over ssh
rssh() {
  ssh -o BatchMode=yes -o ConnectTimeout=10 "${opts[@]}" "$host" "$1"
}

C_REACH='echo yakogl-preflight-ok'
C_OSREL='cat /etc/os-release'
C_UNAME='uname -r'
C_GLIBC="grep -aoE 'GLIBC_[0-9]+(\\.[0-9]+)+' /lib/aarch64-linux-gnu/libc.so.6 | sort -u"
C_CXX="grep -aoE '(GLIBCXX|CXXABI)_[0-9]+(\\.[0-9]+)+' /usr/lib/aarch64-linux-gnu/libstdc++.so.6 | sort -u"
C_NODE='test -c /dev/dri/renderD128 && echo present'
C_DRIVER='basename "$(readlink -f /sys/class/drm/renderD128/device/driver)"'
C_MODULE='test -d /sys/module/panthor && echo loaded; grep -c "^panthor " /proc/modules'
C_DFSTAGE="d=$stage; [ -d \"\$d\" ] || d=/root; df -Pk \"\$d\""
C_DFBOOT='df -Pk /boot'
C_DMESG='dmesg 2>&1 | tail -n 300'
C_LIBS="ldconfig -p | grep -E 'lib(EGL|GLESv2)\\.so'"

if [ "$dry" = 1 ]; then
  echo "$me: dry run, no ssh; the commands each check sends:"
  for c in "$C_REACH" "$C_OSREL" "$C_UNAME" "$C_GLIBC" "$C_CXX" "$C_NODE" "$C_DRIVER" "$C_MODULE" "$C_DFSTAGE" "$C_DFBOOT" "$C_DMESG" "$C_LIBS"; do
    echo "+ ssh \$BOARD_HOST '$c'"
  done
  echo "$me: the dmesg output is classified on this host: python3 $KMSG -"
  [ -z "$build" ] || echo "$me: build floors from $build/BUILD-INFO (else readelf -V $build/bin/$PROGRAM)"
  exit 0
fi

# --- ssh
out=$(rssh "$C_REACH" 2>&1) && [ "$out" = yakogl-preflight-ok ] || {
  say ssh FAIL "board not reachable over ssh (not probing further)"
  finish
}
say ssh ok reachable

# --- os, kernel
osr=$(rssh "$C_OSREL" 2>&1) || osr=
pretty=$(sed -n 's/^PRETTY_NAME="\{0,1\}\([^"]*\)"\{0,1\}$/\1/p' <<< "$osr" | head -1)
codename=$(sed -n 's/^VERSION_CODENAME=//p' <<< "$osr" | head -1)
ver=$(sed -n 's/^VERSION_ID="\{0,1\}\([^"]*\)"\{0,1\}$/\1/p' <<< "$osr" | head -1)
if [ -z "$pretty" ]; then say os-release FAIL "cannot read /etc/os-release"
elif [ "$codename" = trixie ] || [ "${ver%%.*}" = 13 ]; then say os-release ok "$pretty"
else say os-release FAIL "$pretty (trixie wanted)"; fi
kern=$(rssh "$C_UNAME" 2>&1 | head -1)
case $kern in
  6.18.*rockchip64*) say uname ok "$kern" ;;
  '') say uname FAIL "uname -r gave nothing" ;;
  *) say uname warn "$kern (6.18.55-current-rockchip64 is the one the panthor record is for)" ;;
esac

# --- library floors
floor() { # <check> <family> <floor> <have-max>
  if [ -z "$4" ]; then say "$1" FAIL "no $2 version found"
  elif ver_le "$3" "$4"; then say "$1" ok "$2_$4 (floor $3)"
  else say "$1" FAIL "$2_$4 < floor $3"; fi
}
gl=$(rssh "$C_GLIBC" 2>&1) || gl=
cx=$(rssh "$C_CXX" 2>&1) || cx=
b_glibc=$(max_of GLIBC <<< "$gl")
b_glibcxx=$(max_of GLIBCXX <<< "$cx")
b_cxxabi=$(max_of CXXABI <<< "$cx")
floor glibc GLIBC "$GLIBC_FLOOR" "$b_glibc"
floor glibcxx GLIBCXX "$GLIBCXX_FLOOR" "$b_glibcxx"
floor cxxabi CXXABI "$CXXABI_FLOOR" "$b_cxxabi"

# --- the build's own needs and RUNPATH
runpath_ok=0 names_ok=0
if [ -n "$build" ]; then
  info=$build/BUILD-INFO
  bin=$build/bin/$PROGRAM
  n_glibc= n_glibcxx= n_cxxabi= src=
  if [ -f "$info" ]; then
    n_glibc=$(sed -n 's/^glibc_max=//p' "$info" | tail -1)
    n_glibcxx=$(sed -n 's/^glibcxx_max=//p' "$info" | tail -1)
    n_cxxabi=$(sed -n 's/^cxxabi_max=//p' "$info" | tail -1)
    src=BUILD-INFO
  elif [ -f "$bin" ] && command -v readelf > /dev/null 2>&1; then
    re=$(readelf -VW "$bin" 2>&1)
    v=$(grep -oE 'Name: (GLIBCXX|GLIBC|CXXABI)_[0-9][0-9.]*' <<< "$re" | sed 's/^Name: //' | sort -u)
    n_glibc=$(max_of GLIBC <<< "$v") n_glibcxx=$(max_of GLIBCXX <<< "$v") n_cxxabi=$(max_of CXXABI <<< "$v")
    src=readelf
  fi
  if [ -z "$src" ]; then
    say build FAIL "$build has neither BUILD-INFO nor bin/$PROGRAM (and readelf)"
  else
    for f in "GLIBC:$n_glibc:$b_glibc" "GLIBCXX:$n_glibcxx:$b_glibcxx" "CXXABI:$n_cxxabi:$b_cxxabi"; do
      IFS=: read -r fam need have <<< "$f"
      if [ -z "${need-}" ] || [ "$need" = none ]; then
        say "build-$fam" ok "needs no $fam version ($src)"
      elif [ -n "$have" ] && ver_le "$need" "$have"; then
        say "build-$fam" ok "needs ${fam}_$need, board has ${fam}_$have ($src)"
      else
        say "build-$fam" FAIL "needs ${fam}_$need, board has ${fam}_${have:-none} ($src)"
      fi
    done
  fi
  # glmark2 dlopen()s the unversioned libEGL.so/libGLESv2.so first: RUNPATH
  # protects only if the build's lib/ holds those names (links to the pair).
  miss=
  for l in libEGL.so libGLESv2.so; do [ -e "$build/lib/$l" ] || miss="$miss $l"; done
  if [ -z "$miss" ]; then names_ok=1; say build-egl-names ok "lib/ holds libEGL.so and libGLESv2.so"
  else say build-egl-names FAIL "lib/ lacks${miss}: glmark2 dlopen()s these names and would load the board's"; fi
  if [ -f "$bin" ] && command -v readelf > /dev/null 2>&1; then
    if readelf -dW "$bin" 2> /dev/null | grep -qE '\(RUNPATH\).*\[\$ORIGIN/\.\./lib\]'; then
      runpath_ok=1
      say build-runpath ok 'RUNPATH $ORIGIN/../lib'
    else
      say build-runpath FAIL "RUNPATH of bin/$PROGRAM is not \$ORIGIN/../lib"
    fi
  fi
fi

# --- render node, panthor
node=$(rssh "$C_NODE" 2>&1) || node=
if [ "$node" != present ]; then say render-node FAIL "/dev/dri/renderD128 does not exist"
else
  drv=$(rssh "$C_DRIVER" 2>&1 | head -1)
  if [ "$drv" = panthor ]; then say render-node ok "/dev/dri/renderD128 belongs to panthor"
  else say render-node FAIL "/dev/dri/renderD128 belongs to '${drv:-unknown}', not panthor"; fi
fi
mod=$(rssh "$C_MODULE" 2>&1) || mod=
if grep -q '^loaded$' <<< "$mod" || grep -qx '[1-9][0-9]*' <<< "$mod"; then say panthor-module ok loaded
else say panthor-module FAIL "panthor is not loaded (not loading it here)"; fi

# --- space
space() { # <check> <hard|soft> <df output> <need-kb> <what>
  local avail
  avail=$(awk 'END { print $4 }' <<< "$3")
  if ! [[ $avail =~ ^[0-9]+$ ]]; then
    if [ "$2" = hard ]; then say "$1" FAIL "cannot read free space of $5"; else say "$1" warn "cannot read free space of $5"; fi
  elif [ "$avail" -ge "$4" ]; then say "$1" ok "$((avail / 1024)) MiB free at $5"
  elif [ "$2" = hard ]; then say "$1" FAIL "$((avail / 1024)) MiB free at $5, $(($4 / 1024)) MiB wanted"
  else say "$1" warn "$((avail / 1024)) MiB free at $5, $(($4 / 1024)) MiB wanted"; fi
}
space stage-space hard "$(rssh "$C_DFSTAGE" 2>&1)" "$need_kb" "$stage (or /root)"
space boot-space soft "$(rssh "$C_DFBOOT" 2>&1)" 102400 /boot

# --- dmesg for mali-kmsg.py (read here; the runner reads it with dmesg -w)
dm=$(rssh "$C_DMESG" 2>&1) || dm=
if [ -z "$dm" ] || grep -qiE 'not permitted|permission denied|read kernel buffer failed|command not found' <<< "$dm"; then
  say dmesg FAIL "dmesg cannot be read ($(head -1 <<< "$dm" | cut -c1-80))"
elif k=$(python3 "$KMSG" - <<< "$dm" 2>&1); then
  say dmesg ok "readable; $(head -1 <<< "$k")"
else say dmesg FAIL "mali-kmsg.py cannot classify it: $(head -1 <<< "$k")"; fi

# --- system EGL/GLES the loader could pick up
libs=$(rssh "$C_LIBS" 2>&1) || libs=
if [ -z "$libs" ]; then say system-egl ok "no system libEGL/libGLESv2 in the loader cache"
else
  names=$(grep -oE 'lib(EGL|GLESv2)\.so[.0-9]*' <<< "$libs" | sort -u | paste -sd' ' -)
  if [ -n "$build" ] && [ "$names_ok" != 1 ]; then say system-egl FAIL "present ($names) and the build's lib/ lacks libEGL.so/libGLESv2.so: the board's would be loaded"
  elif [ "$runpath_ok" = 1 ]; then say system-egl ok "present ($names) but the build's lib/ holds libEGL.so/libGLESv2.so and RUNPATH \$ORIGIN/../lib wins"
  else say system-egl warn "present ($names): a RUNPATH other than \$ORIGIN/../lib would load glvnd/Mesa, not the panthor-gles pair (pass --build to prove it)"; fi
fi
finish
