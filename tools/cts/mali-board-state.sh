#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Read-only before/after snapshot of the NanoPC-T6's /boot, U-Boot environment
# and loaded kernel modules: the evidence that a glmark2 run left /boot byte for
# byte as it was and loaded or unloaded no module.
#
#   tools/cts/mali-board-state.sh snapshot <host> <outfile>
#   tools/cts/mali-board-state.sh compare <before> <after>
#
# <host> is user@host from the target:nanopc-t6 lease (BOARD_HOST in the other
# Mali scripts), given by the caller, never read from a file; BOARD_SSH_OPTS
# (optional) goes to ssh. Only reading commands are sent (find, sort,
# sha256sum, fw_printenv, cat, uname); nothing is written on the board and
# nothing needs root.
#
# snapshot writes key=value lines: boot_files, boot_sha256 (sha256 over the
# path-sorted `sha256sum` lines of every regular file of `find /boot -xdev`),
# uboot_env (sha256 of fw_printenv, or "unavailable" when it is missing or not
# readable without root), modules, modules_sha256 (count and sha256 of the
# sorted module-name column of /proc/modules), boot_id, kernel.
#
# compare prints "mali-board-state: <what> unchanged (...)" or "... CHANGED"
# for /boot, modules and the U-Boot env, then an info line for boot_id.
# Exit: snapshot 0 written, 4 board unreachable or a command failed;
# compare 0 all unchanged, 1 something CHANGED; 2 usage or an unreadable file.
set -uo pipefail
me=${0##*/}
usage() { echo "usage: $me snapshot <host> <outfile> | compare <before> <after>" >&2; exit 2; }
[ $# -ge 1 ] || usage
mode=$1
shift

C_REACH='echo yakogl-state-ok'
C_BOOT='find /boot -xdev -type f -print0 | LC_ALL=C sort -z | xargs -0 -r sha256sum'
C_ENV='if command -v fw_printenv > /dev/null 2>&1; then fw_printenv 2> /dev/null || echo yakogl-state-unavailable; else echo yakogl-state-unavailable; fi'
C_MODS='cat /proc/modules'
C_BOOTID='cat /proc/sys/kernel/random/boot_id'
C_UNAME='uname -r'

sha() { sha256sum | cut -d' ' -f1; }

snapshot() {
  [ $# = 2 ] || usage
  local host=$1 outfile=$2 out boot env mods bid kern opts
  [ -n "$host" ] || usage
  read -r -a opts <<< "${BOARD_SSH_OPTS-}"
  rssh() { ssh -o BatchMode=yes -o ConnectTimeout=10 "${opts[@]}" "$host" "$1"; }
  out=$(rssh "$C_REACH" 2> /dev/null) && [ "$out" = yakogl-state-ok ] || {
    echo "$me: board not reachable over ssh (BatchMode, 10 s); no snapshot written" >&2
    exit 4
  }
  boot=$(rssh "$C_BOOT") || { echo "$me: cannot read every file under /boot (sha256sum failed); no snapshot written" >&2; exit 4; }
  env=$(rssh "$C_ENV") || { echo "$me: ssh failed reading the U-Boot environment; no snapshot written" >&2; exit 4; }
  mods=$(rssh "$C_MODS") || { echo "$me: cannot read /proc/modules; no snapshot written" >&2; exit 4; }
  bid=$(rssh "$C_BOOTID") || { echo "$me: cannot read boot_id; no snapshot written" >&2; exit 4; }
  kern=$(rssh "$C_UNAME") || { echo "$me: uname -r failed; no snapshot written" >&2; exit 4; }
  local nboot=0 hboot envh nmod=0 hmod names
  [ -z "$boot" ] || nboot=$(printf '%s\n' "$boot" | wc -l)
  hboot=$(printf '%s' "$boot" | sha)
  if [ -z "$env" ] || [ "$env" = yakogl-state-unavailable ]; then
    envh=unavailable
  else
    envh=$(printf '%s\n' "$env" | sha)
  fi
  names=$(printf '%s\n' "$mods" | awk 'NF { print $1 }' | LC_ALL=C sort)
  [ -z "$names" ] || nmod=$(printf '%s\n' "$names" | wc -l)
  hmod=$(printf '%s' "$names" | sha)
  {
    echo "boot_files=$nboot"
    echo "boot_sha256=$hboot"
    echo "uboot_env=$envh"
    echo "modules=$nmod"
    echo "modules_sha256=$hmod"
    echo "boot_id=$bid"
    echo "kernel=$kern"
  } > "$outfile" || { echo "$me: cannot write $outfile" >&2; exit 2; }
  echo "$me: snapshot of $nboot /boot files and $nmod modules written to $outfile"
}

get() { sed -n "s/^$2=//p" "$1" | tail -1; }

compare() {
  [ $# = 2 ] || usage
  local b=$1 a=$2 f rc=0 bn bh an ah
  for f in "$b" "$a"; do
    [ -r "$f" ] || { echo "$me: cannot read $f" >&2; exit 2; }
  done
  bn=$(get "$b" boot_files) bh=$(get "$b" boot_sha256) an=$(get "$a" boot_files) ah=$(get "$a" boot_sha256)
  if [ -n "$bh" ] && [ "$bh" = "$ah" ] && [ "$bn" = "$an" ]; then
    echo "mali-board-state: /boot unchanged ($bn files, sha256 $bh)"
  else
    echo "mali-board-state: /boot CHANGED (before $bn files, sha256 ${bh:-missing}; after $an files, sha256 ${ah:-missing})"
    rc=1
  fi
  bn=$(get "$b" modules) bh=$(get "$b" modules_sha256) an=$(get "$a" modules) ah=$(get "$a" modules_sha256)
  if [ -n "$bh" ] && [ "$bh" = "$ah" ] && [ "$bn" = "$an" ]; then
    echo "mali-board-state: modules unchanged ($bn modules, sha256 $bh)"
  else
    echo "mali-board-state: modules CHANGED (before $bn modules, sha256 ${bh:-missing}; after $an modules, sha256 ${ah:-missing})"
    rc=1
  fi
  bh=$(get "$b" uboot_env) ah=$(get "$a" uboot_env)
  if [ -z "$bh" ] || [ -z "$ah" ]; then
    echo "mali-board-state: U-Boot env CHANGED (before ${bh:-missing}; after ${ah:-missing})"
    rc=1
  elif [ "$bh" = unavailable ] && [ "$ah" = unavailable ]; then
    echo "mali-board-state: U-Boot env unavailable in both snapshots (fw_printenv missing or needs root; not compared)"
  elif [ "$bh" = "$ah" ]; then
    echo "mali-board-state: U-Boot env unchanged (sha256 $bh)"
  else
    echo "mali-board-state: U-Boot env CHANGED (before $bh; after $ah)"
    rc=1
  fi
  bh=$(get "$b" boot_id) ah=$(get "$a" boot_id)
  if [ "$bh" = "$ah" ]; then
    echo "mali-board-state: boot_id unchanged ($bh)"
  else
    echo "mali-board-state: boot_id differs (before $bh; after $ah): the board rebooted"
  fi
  exit "$rc"
}

case $mode in
  snapshot) snapshot "$@" ;;
  compare) compare "$@" ;;
  -h | --help) sed -n '5,28p' "$0" ;;
  *) usage ;;
esac
