#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# stage.sh - put a Zephyr image where the NanoPC-T6's U-Boot loads it from
# (/boot/yakogl/<name> on the eMMC's ext4 root, mmc 0:1), check it, and take
# it away again (README.md, "Staging under /boot and the undo"). Over ssh,
# from Armbian, as root.
#
#   BOARD_HOST=root@<t6> driver/tools/uboot-t6/stage.sh stage <zephyr.bin> <fingerprint>
#   BOARD_HOST=root@<t6> driver/tools/uboot-t6/stage.sh check <zephyr.bin> <fingerprint>
#   BOARD_HOST=root@<t6> driver/tools/uboot-t6/stage.sh undo <fingerprint>
#
# <fingerprint> is a file on THIS host: the sha256 of every regular file and
# the target of every symbolic link under /boot except /boot/yakogl, sorted
# by name. `stage` writes it when it does not exist yet, before anything is
# written on the board; when it exists, `stage` and `check` go on only if
# /boot still matches it. Every command compares /boot with it again after
# its own change, so a stock file that changed is named. `undo` removes
# /boot/yakogl even when a stock file changed since `stage` (a kernel update
# under Armbian does that): it names the difference and exits 1 after the
# removal.
#
# A /boot/yakogl that is a symbolic link, or that holds one, is refused by
# `stage` and `check`: a write through it would land outside /boot/yakogl.
# `undo`'s rm -rf removes the link itself, never what it points to.
#
# What it writes on the board, and nothing else: the directory /boot/yakogl
# (mode 0755), one file in it per image (mode 0644, through a .part file in
# the same directory renamed over it), `sync`; `undo` removes /boot/yakogl
# with everything in it. It never writes another file of /boot, a block
# device, the eMMC's loader area or U-Boot's environment, and never loads a
# kernel module. The image's bytes go over ssh's stdin (no scp).
#
# Exit: 0 done and /boot's other files match the fingerprint; 1 something on
# the board differs (the copy, or a stock file: the message names it); 2
# usage or a missing tool; 3 refused before anything was written.
set -euo pipefail

STAGE_DIR=/boot/yakogl

die() { echo "stage.sh: $*" >&2; exit 2; }
refuse() { echo "stage.sh: REFUSED: $*" >&2; echo "stage.sh: nothing was written on the board." >&2; exit 3; }
fail() { echo "stage.sh: FAILED: $*" >&2; exit 1; }

usage="usage: stage.sh stage|check <zephyr.bin> <fingerprint> | stage.sh undo <fingerprint> (see the header)"
CMD=${1:-}
case "$CMD" in
stage | check) [ $# -eq 3 ] || die "$usage"; IMG=$2 FP=$3 ;;
undo) [ $# -eq 2 ] || die "$usage"; IMG= FP=$2 ;;
*) die "$usage" ;;
esac
[ -n "${BOARD_HOST:-}" ] || die "set BOARD_HOST=root@<the T6>"
# shellcheck disable=SC2206  # deliberately word-split
SSH_OPTS=(${BOARD_SSH_OPTS:-})
for t in ssh timeout sha256sum diff; do
	command -v "$t" >/dev/null || die "$t is not on this host"
done
NAME=
if [ -n "$IMG" ]; then
	[ -f "$IMG" ] || die "$IMG: no such file"
	NAME=$(basename "$IMG")
	[[ "$NAME" =~ ^[A-Za-z0-9_][A-Za-z0-9._-]*\.bin$ ]] ||
		refuse "$NAME: the staged name must be letters, digits, '.', '_' or '-', ending in .bin (prompt-go.py types it at the prompt)"
	sha=$(sha256sum "$IMG" | cut -d' ' -f1)
fi

board() { timeout 120 ssh "${SSH_OPTS[@]}" -o BatchMode=yes -o ConnectTimeout=10 "$BOARD_HOST" "$@"; }
# Every regular file under /boot but the staged directory, by name, then
# every symbolic link and its target.
FINGERPRINT="find /boot -path $STAGE_DIR -prune -o -type f -print0 | sort -z | xargs -0 -r sha256sum;
find /boot -path $STAGE_DIR -prune -o -type l -printf 'link %p -> %l\\n' | sort"
fingerprint() { board "$FINGERPRINT" </dev/null; }

# ---- the board, read only --------------------------------------------------
facts=$(board 'printf "compatible=%s\n" "$(tr "\0" " " </proc/device-tree/compatible)"
printf "uid=%s\n" "$(id -u)"
printf "boot=%s\n" "$([ -d /boot ] && [ ! -L /boot ] && echo dir)"
printf "stage_links=%s\n" "$(if [ -L '"$STAGE_DIR"' ]; then echo link; elif [ -d '"$STAGE_DIR"' ]; then find '"$STAGE_DIR"' -type l | head -n 1; fi)"' </dev/null) ||
	refuse "cannot read the board's facts over ssh ($BOARD_HOST)"
fact() { printf '%s\n' "$facts" | sed -n "s/^$1=//p" | head -1; }
case "$(fact compatible)" in *friendlyarm,nanopc-t6*) ;; *) refuse "$BOARD_HOST is not a NanoPC-T6: compatible '$(fact compatible)'" ;; esac
[ "$(fact uid)" = 0 ] || refuse "$BOARD_HOST is not root (/boot is root's)"
[ "$(fact boot)" = dir ] || refuse "/boot on $BOARD_HOST is not a directory"
if [ -n "$(fact stage_links)" ]; then
	[ "$CMD" = undo ] || refuse "$STAGE_DIR is a symbolic link or holds one ($(fact stage_links)): a write through it would leave /boot/yakogl"
	echo "stage.sh: $STAGE_DIR is a symbolic link or holds one ($(fact stage_links)); rm -rf removes the links, not their targets"
fi

now=$(fingerprint) || refuse "cannot read the fingerprint of /boot"
[ -n "$now" ] || refuse "the fingerprint of /boot is empty: no stock file found"
same_as_recorded() { diff <(printf '%s\n' "$1") "$FP" >/dev/null; }
differed=0
if [ -f "$FP" ]; then
	if ! same_as_recorded "$now"; then
		diff <(printf '%s\n' "$now") "$FP" | sed 's/^/  /' >&2 || true
		[ "$CMD" = stage ] && refuse "/boot's files differ from $FP (lines above: < now, > recorded)"
		[ "$CMD" = check ] && fail "/boot's files differ from $FP (lines above: < now, > recorded)"
		echo "stage.sh: /boot's files differ from $FP already, not by this tool (lines above); $STAGE_DIR is removed all the same" >&2
		differed=1
	else
		echo "stage.sh: /boot matches $FP ($(wc -l <"$FP") entries)"
	fi
elif [ "$CMD" = stage ]; then
	printf '%s\n' "$now" >"$FP" || die "cannot write $FP"
	echo "stage.sh: /boot's fingerprint written to $FP ($(wc -l <"$FP") entries), before anything on the board"
else
	die "$FP: no such file (stage writes it)"
fi

staged_sha() { board "sha256sum $STAGE_DIR/$NAME 2>/dev/null | cut -d' ' -f1" </dev/null; }

case "$CMD" in
stage)
	board "set -e; install -d -m 0755 $STAGE_DIR; cat > $STAGE_DIR/.$NAME.part; chmod 0644 $STAGE_DIR/.$NAME.part; mv -f $STAGE_DIR/.$NAME.part $STAGE_DIR/$NAME; sync" <"$IMG" ||
		fail "the copy to $STAGE_DIR/$NAME did not complete"
	;;
undo)
	board "rm -rf -- $STAGE_DIR && sync" </dev/null || fail "rm -rf -- $STAGE_DIR failed"
	left=$(board "[ -e $STAGE_DIR ] && echo left || true" </dev/null)
	[ -z "$left" ] || fail "$STAGE_DIR is still there after rm"
	echo "stage.sh: $STAGE_DIR removed"
	;;
esac

if [ -n "$NAME" ]; then
	have=$(staged_sha)
	[ "$have" = "$sha" ] || fail "$STAGE_DIR/$NAME on the board is '$have', not $IMG's $sha"
	echo "stage.sh: $STAGE_DIR/$NAME is $IMG (sha256 $sha)"
fi
after=$(fingerprint) || fail "cannot read the fingerprint of /boot again"
if ! same_as_recorded "$after"; then
	diff <(printf '%s\n' "$after") "$FP" | sed 's/^/  /' >&2 || true
	[ "$differed" = 0 ] || fail "$STAGE_DIR is gone; /boot's other files differ from $FP, as they did before undo (lines above)"
	fail "/boot's other files differ from $FP after $CMD (lines above)"
fi
case "$CMD" in
undo) echo "stage.sh: stock /boot restored: every file matches $FP and $STAGE_DIR is gone" ;;
*) echo "stage.sh: /boot's other files still match $FP" ;;
esac
