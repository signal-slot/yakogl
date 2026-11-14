#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_card_undo.sh - last-resort undo of the STM32MP257F-DK standalone
# default with the card in a PC (docs/notes/stm32mp257f-dk-standalone.md,
# section 5): rename yakogl/standalone/boot.scr.uimg to boot.scr.uimg.off
# (debugfs has no rename command: `ln` + `unlink` in one debugfs -w session) on
# the card's ext4 bootfs, so U-Boot's distro loop finds nothing under our
# prefix and ST's extlinux boots Linux.
#
# usage: tools/mp2s_card_undo.sh [--dry-run | --yes] <bootfs-partition-device-or-image>
#
# - --dry-run is the default: it only reads. --yes is needed to write.
# - Nothing is mounted: the write is `debugfs -w` (e2fsprogs) on the
#   unmounted target. A mounted target is refused.
# - The only path ever written is that one rename. No other path is touched.
# Needs: blkid, debugfs, dumpe2fs (e2fsprogs), sha256sum. Linux PC only.
set -u

SELF=${0##*/}
MODE=dry
TARGET=
for a in "$@"; do
  case "$a" in
    --yes) MODE=yes ;;
    --dry-run) MODE=dry ;;
    -h|--help) sed -n '5,19p' "$0"; exit 0 ;;
    -*) echo "$SELF: unknown option $a" >&2; exit 2 ;;
    *) [ -z "$TARGET" ] || { echo "$SELF: one target only" >&2; exit 2; }; TARGET=$a ;;
  esac
done
[ -n "$TARGET" ] || { echo "usage: $SELF [--dry-run | --yes] <bootfs device or image>" >&2; exit 2; }

DIR=/yakogl/standalone
SRC=$DIR/boot.scr.uimg
DST=$DIR/boot.scr.uimg.off

refuse() { echo "REFUSED: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || { echo "$SELF: $1 is missing (e2fsprogs/util-linux needed)" >&2; exit 2; }; }
for c in blkid debugfs dumpe2fs sha256sum awk; do need "$c"; done

[ -e "$TARGET" ] || refuse "$TARGET does not exist"
if [ -b "$TARGET" ]; then
  KIND=block
elif [ -f "$TARGET" ] && [ ! -L "$TARGET" ]; then
  KIND=file
else
  refuse "$TARGET is neither a block device nor a regular file"
fi
TARGET=$(readlink -f -- "$TARGET")

# ---- a whole disk is refused: we want the bootfs partition only
if [ "$KIND" = block ]; then
  need lsblk
  [ "$(lsblk -dno TYPE -- "$TARGET" 2>/dev/null)" = disk ] \
    && refuse "$TARGET is a whole disk; give the bootfs partition (for example /dev/sdX4)"
fi
PT=$(blkid -p -o value -s PTTYPE -- "$TARGET" 2>/dev/null || true)
[ -z "$PT" ] || refuse "$TARGET has a partition table ($PT): a whole disk, not the bootfs partition"

# ---- mounted is refused
if [ "$KIND" = block ]; then
  if awk -v d="$TARGET" '$1 != "" { print $1 }' /proc/self/mounts | while read -r s; do
       [ -e "$s" ] && [ "$(readlink -f -- "$s")" = "$TARGET" ] && echo hit
     done | grep -q hit; then
    refuse "$TARGET is mounted; unmount it first (nothing here needs it mounted)"
  fi
  if command -v lsblk >/dev/null 2>&1 && [ -n "$(lsblk -no MOUNTPOINT -- "$TARGET" 2>/dev/null | tr -d '[:space:]')" ]; then
    refuse "$TARGET is mounted; unmount it first"
  fi
else
  if command -v losetup >/dev/null 2>&1; then
    LOOPS=$(losetup -j "$TARGET" 2>/dev/null | cut -d: -f1)
    for l in $LOOPS; do
      if awk '{print $1}' /proc/self/mounts | grep -qx "$l"; then
        refuse "$TARGET is mounted through $l"
      fi
    done
  fi
fi

# ---- ext4 only, and clean
FST=$(blkid -p -o value -s TYPE -- "$TARGET" 2>/dev/null || true)
[ "$FST" = ext4 ] || refuse "$TARGET is '${FST:-unrecognised}', not ext4"
STATE=$(dumpe2fs -h "$TARGET" 2>/dev/null | sed -n 's/^Filesystem state: *//p')
[ "$STATE" = clean ] || refuse "ext4 state is '${STATE:-unknown}', not clean; fsck it from a person's shell first"

DBG() { debugfs -R "$1" "$TARGET" 2>/dev/null; }

# entries of a directory as "<mode>\t<name>" (ls -l: inode mode (n) uid gid size date time name)
lsdir() { DBG "ls -l $1" | awk 'NF>=9 { printf "%s\t", $2; n=""; for(i=9;i<=NF;i++) n = n (i>9?" ":"") $i; print n }'; }
mode_of() { # <dir> <name> -> mode octal string or empty
  lsdir "$1" | awk -F'\t' -v n="$2" '$2==n { print $1; exit }'
}
isdir() { case "$1" in 40*) return 0;; *) return 1;; esac; }
isreg() { case "$1" in 100*) return 0;; *) return 1;; esac; }

# ---- the layout: ST's stock files and ours, all real (no symlinks) on the path
m=$(mode_of / boot.scr.uimg); isreg "$m" || refuse "root has no regular boot.scr.uimg (ST's stock layout missing): not a stock bootfs"
EXT=
for e in mmc0_extlinux mmc1_extlinux; do
  m=$(mode_of / "$e"); if isdir "$m"; then EXT="$EXT $e"; fi
done
[ -n "$EXT" ] || refuse "root has neither mmc0_extlinux/ nor mmc1_extlinux/: not a stock bootfs"
m=$(mode_of / yakogl); isdir "$m" || refuse "no yakogl/ directory (a real one, not a link) at the root: no standalone default here"
m=$(mode_of /yakogl standalone); isdir "$m" || refuse "no yakogl/standalone/ directory (a real one, not a link): no standalone default here"
m=$(mode_of $DIR boot.scr.uimg); if [ -z "$m" ]; then
  if [ -n "$(mode_of $DIR boot.scr.uimg.off)" ]; then echo "Already undone: $DST exists and $SRC does not. Nothing to do."; exit 0; fi
  refuse "$SRC does not exist"
fi
isreg "$m" || refuse "$SRC is not a regular file (mode $m; a link or other object): not touching it"
[ -z "$(mode_of $DIR boot.scr.uimg.off)" ] || refuse "$DST already exists; not overwriting it"

# ---- sha256 of the files that must not change (stock files + the rest of ours)
SCRATCH=${TMPDIR:-/tmp}
W=$(mktemp -d "$SCRATCH/mp2s_card_undo.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT

hashset() { # <outfile>: sha256 of the stock files and of yakogl/standalone/ files except the renamed one
  local out=$1 dir name mode f i=0
  : > "$out"
  hashone() { # <path>
    i=$((i+1)); f=$W/dump.$i
    debugfs -R "dump $1 $f" "$TARGET" >/dev/null 2>&1 || { echo "dump failed: $1" >&2; return 1; }
    printf '%s  %s\n' "$(sha256sum < "$f" | cut -d' ' -f1)" "$1" >> "$out"; rm -f "$f"
  }
  hashone /boot.scr.uimg || return 1
  for dir in $EXT; do
    while IFS=$'\t' read -r mode name; do
      isreg "$mode" && { hashone "/$dir/$name" || return 1; }
    done < <(lsdir "/$dir")
  done
  while IFS=$'\t' read -r mode name; do
    case "$name" in boot.scr.uimg|boot.scr.uimg.off|.|..) continue;; esac
    isreg "$mode" && { hashone "$DIR/$name" || return 1; }
  done < <(lsdir $DIR)
  LC_ALL=C sort -k2 -o "$out" "$out"
}

hashset "$W/before" || refuse "could not read the stock files"
NH=$(wc -l < "$W/before")

IMG_SHA=
[ "$KIND" = file ] && IMG_SHA=$(sha256sum < "$TARGET" | cut -d' ' -f1)

echo "target:   $TARGET ($KIND, ext4, clean, not mounted)"
echo "stock:    /boot.scr.uimg and$(for e in $EXT; do printf ' /%s/' "$e"; done)"
echo "ours:     $SRC (the only path this tool writes)"
echo "stock files and the rest of yakogl/standalone checked: $NH file(s) hashed"
echo
echo "WRITE (debugfs has no rename: ln adds the new name without touching the link"
echo "count, unlink drops the old name without freeing the inode; together a rename):"
echo "  printf 'ln $SRC $DST\\nunlink $SRC\\n' | debugfs -w -f - $TARGET"
echo "UNDO of this undo (rename it back, same way, card unmounted):"
echo "  printf 'ln $DST $SRC\\nunlink $DST\\n' | debugfs -w -f - $TARGET"
echo

if [ "$MODE" != yes ]; then
  echo "DRY RUN: nothing was written. Add --yes to run the WRITE command."
  exit 0
fi

printf 'ln %s %s\nunlink %s\n' "$SRC" "$DST" "$SRC" | debugfs -w -f - "$TARGET" >"$W/rename.out" 2>&1
rc=$?
if [ $rc -ne 0 ] || grep -qi 'not found\|error\|cannot' "$W/rename.out"; then
  cat "$W/rename.out" >&2
  echo "FAILED: the rename did not succeed; read the directory back before anything else:" >&2
  lsdir $DIR >&2
  exit 1
fi
sync

echo "directory read back ($DIR):"
lsdir $DIR | sed 's/^/  /'
[ -z "$(mode_of $DIR boot.scr.uimg)" ] || { echo "FAILED: $SRC is still there" >&2; exit 1; }
isreg "$(mode_of $DIR boot.scr.uimg.off)" || { echo "FAILED: $DST is not there" >&2; exit 1; }

hashset "$W/after" || { echo "FAILED: could not read back the stock files" >&2; exit 1; }
if cmp -s "$W/before" "$W/after"; then
  echo "stock files: sha256 unchanged ($NH file(s)):"
  sed 's/^/  /' "$W/after"
else
  echo "FAILED: sha256 of a stock file changed:" >&2
  diff "$W/before" "$W/after" >&2
  exit 1
fi
dumpe2fs -h "$TARGET" 2>/dev/null | sed -n 's/^Filesystem state: */filesystem state: /p'

cat <<EOF

Done. Next:
  1. Put the card back in the DK (unmounted, after sync) and power it on.
     ST's extlinux boots OpenSTLinux.
  2. From Linux run:  tools/board-stm32mp2-standalone.sh nodefault
     It restores boot_prefixes and deletes the mp2s_* variables.
To put the default back instead: printf 'ln $DST $SRC\\nunlink $DST\\n' | debugfs -w -f - $TARGET
EOF
exit 0
