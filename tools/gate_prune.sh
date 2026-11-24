#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# usage: tools/gate_prune.sh <target-dir> [--incremental] [--artefacts]
#                            [--if-free-below <GiB>]
#
# Prunes a cargo target directory between the rows of tools/check.sh. The
# full gate of 2026-11-14 filled its disk: the shared target directory
# reached 180 GB, 80 GB of it debug/incremental (docs/release/checklist.md,
# 'Line 12: full gate (2026-11-14)').
#
#   --incremental        remove every <profile>/incremental directory
#                        (<target>/<profile>/ and <target>/<triple>/<profile>/)
#   --artefacts          remove the compiled test and example executables
#                        (<profile>/deps/<name>-<hash> files with the
#                        executable bit and no extension, <profile>/examples)
#                        of finished rows. Kept: rlib/rmeta/.so/.d files
#                        of deps, build-script output (<profile>/build), the
#                        .fingerprint dirs, every release* profile tree, and
#                        any .a archive: later rows relink or reuse them.
#   --if-free-below <G>  act only when the filesystem holding the target
#                        directory has less than G GiB free
#
# With neither --incremental nor --artefacts it does both. The mode flags
# select what is removed; --if-free-below only gates it. It prints what it
# removed and the space it freed.
#
# Refused (exit 2): a path that is not a directory holding CACHEDIR.TAG or
# .rustc_info.json; /, $HOME, the repository root, or any directory that
# contains one of them. Nothing is followed out of the target: removal uses
# find -P and a symlink is unlinked, never entered.
#
# YAKOGL_TEST_OK (check.sh's TEST_OK_DIR) lives outside the target
# directory by default (<target>/../test-ok) and is never touched here.

set -u

die() { echo "gate_prune.sh: $*" >&2; exit 2; }

usage() {
  sed -n '5,6p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

target='' do_inc=0 do_art=0 threshold=''
while [ $# -gt 0 ]; do
  case $1 in
  --incremental) do_inc=1 ;;
  --artefacts) do_art=1 ;;
  --if-free-below)
    [ $# -ge 2 ] || die "--if-free-below needs a number of GiB"
    threshold=$2
    shift
    [[ $threshold =~ ^[0-9]+$ ]] || die "--if-free-below '$threshold' is not a whole number of GiB"
    ;;
  -h | --help) usage ;;
  -*) die "unknown option $1" ;;
  *)
    [ -z "$target" ] || die "one target directory only"
    target=$1
    ;;
  esac
  shift
done
[ -n "$target" ] || usage
[ "$do_inc" = 1 ] || [ "$do_art" = 1 ] || { do_inc=1; do_art=1; }

[ -d "$target" ] || die "'$target' is not a directory"
if [ ! -f "$target/CACHEDIR.TAG" ] && [ ! -f "$target/.rustc_info.json" ]; then
  die "'$target' is not a cargo target directory (no CACHEDIR.TAG or .rustc_info.json)"
fi
root=$(realpath "$target") || die "cannot resolve '$target'"

here=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
repo=$(git -C "$here" rev-parse --show-toplevel 2>/dev/null || echo "$here")
repo=$(realpath "$repo")
protected=("/" "$repo")
[ -n "${HOME-}" ] && protected+=("$(realpath "$HOME")")
for p in "${protected[@]}"; do
  # $root is $p or an ancestor of it
  case "$p/" in
  "$root"/*) die "refusing '$root': it is or contains '$p'" ;;
  esac
done

if [ -n "$threshold" ]; then
  free_kb=$(df -Pk "$root" | awk 'NR==2 { print $4 }')
  [[ $free_kb =~ ^[0-9]+$ ]] || die "cannot read the free space of '$root'"
  if [ "$free_kb" -ge $((threshold * 1024 * 1024)) ]; then
    echo "gate_prune.sh: $((free_kb / 1024 / 1024)) GiB free on '$root', not below $threshold GiB: nothing removed"
    exit 0
  fi
fi

freed=0 count=0
# remove_one <path>: size it (-P, no symlink followed), remove it, report.
remove_one() {
  local p=$1 kb rel
  kb=$(du -skP "$p" 2>/dev/null | awk '{ print $1 }')
  [[ ${kb-} =~ ^[0-9]+$ ]] || kb=0
  rm -rf -- "$p" || return 0
  freed=$((freed + kb))
  count=$((count + 1))
  rel=${p#$root/}
  echo "removed $rel ($((kb / 1024)) MiB)"
}

if [ "$do_inc" = 1 ]; then
  while IFS= read -r -d '' d; do remove_one "$d"; done \
    < <(find -P "$root" -mindepth 2 -maxdepth 3 -type d -name incremental -print0)
fi

if [ "$do_art" = 1 ]; then
  # <target>/<profile> and <target>/<triple>/<profile>, never a release* one
  while IFS= read -r -d '' deps; do
    prof=$(dirname "$deps")
    rel=${prof#$root/}
    case /$rel/ in */release*) continue ;; esac
    while IFS= read -r -d '' f; do remove_one "$f"; done \
      < <(find -P "$deps" -maxdepth 1 -type f -perm /111 ! -name '*.*' -print0)
    if [ -d "$prof/examples" ] && [ ! -L "$prof/examples" ]; then
      while IFS= read -r -d '' f; do remove_one "$f"; done \
        < <(find -P "$prof/examples" -mindepth 1 -maxdepth 1 -print0)
    fi
  done < <(find -P "$root" -mindepth 2 -maxdepth 3 -type d -name deps -print0)
fi

echo "gate_prune.sh: removed $count item(s), freed $((freed / 1024)) MiB ($freed KiB) under $root"
