#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-preflight.sh - read-only host check before an i.MX8MP dEQP-GLES3 board session: the env, the
# stage 1 package, the table, the table image and its commit. One 'ok'/'FAIL' line per check; exit 0 only if all pass.
#   tools/cts/imx8mp-gles3-preflight.sh [--board imx8mp|x2] [--commit <sha, 8+ hex>] [--out <dir>] [--table <file>]
# Defaults: board = $IMX8MP_BOARD, else imx8mp; commit = git rev-parse origin/main, out = $IMX8MP_OUT,
# table = tools/cts/imx8mp-gles3-tree.txt.
# Writes nothing; BOARD_UART is only reported set/unset. The one board access is the board check below
# (ssh to $BOARD_HOST, read commands only; skipped when BOARD_HOST is unset or the board does not answer). imx8mp-env.sh is not sourced
# (it exits and creates $IMX8MP_OUT); the image layout is imx8mp-deqp-build.sh's: <out>/es3-<hash8>/zephyr/zephyr.bin.
# imx8mp (the Verdin): BOARD_UART is required and the board check is the /boot medium guard.
# x2 (the Armadillo-X2, tools/board.sh x2, the crash-route kexec; docs/es3/board-recipe-imx8mp.md, "Running a GLES3
# slice on the Armadillo-X2"): the image is <out>/x2/es3-<hash8>/zephyr/zephyr.bin and must carry the watchdog
# marker 'gc_wdog_arm: armed'; the loader and snvs_lpgpr must be built in driver/tools/kexec-zephyr; BOARD_UART is
# optional (a note, not a FAIL); the board check reads the device-tree model (the Armadillo-X2, an i.MX 8M Plus)
# and that /tmp is a tmpfs (the run stages only in /tmp/yakogl-kexec). The Verdin's /boot guard does not apply
# there (the run writes nothing to /boot, the eMMC or the U-Boot env) and says so; /var/tmp, where board.sh keeps
# its board lease, is reported as an info line. X2_TOOLS (default driver/tools/kexec-zephyr) is where the two
# tools are looked for (the selftest's stubs).
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
commit='' out=${IMX8MP_OUT:-} table=$HERE/imx8mp-gles3-tree.txt board=${IMX8MP_BOARD:-imx8mp}
while [ $# -gt 0 ]; do
	case $1 in
		--board) board=${2:?--board needs imx8mp or x2}; shift 2 ;;
		--commit) commit=${2:?--commit needs a sha}; shift 2 ;;
		--out) out=${2:?--out needs a directory}; shift 2 ;;
		--table) table=${2:?--table needs a file}; shift 2 ;;
		*) echo "usage: $0 [--board imx8mp|x2] [--commit <sha>] [--out <dir>] [--table <file>]" >&2; exit 2 ;;
	esac
done
case $board in
	imx8mp) idir=$out ;;
	x2) idir=${out:+$out/x2} ;;
	*) echo "imx8mp-gles3-preflight: --board is imx8mp or x2, not '$board'" >&2; exit 2 ;;
esac
fails=0
ok() { echo "  ok   $1"; }
bad() { echo "  FAIL $1"; fails=$((fails + 1)); }

IMX8MP_OUT=$out
for v in IMX8MP_OUT DEQP_STAGE1 ZEPHYR_WS HAL_NXP_DIR; do
	eval "val=\${$v:-}"
	if [ -z "$val" ]; then bad "$v is not set"
	elif [ -d "$val" ]; then ok "$v is set, directory exists"
	else bad "$v is set, directory does not exist"; fi
done
if [ -n "${BOARD_UART:-}" ]; then ok "BOARD_UART is set"
elif [ "$board" = x2 ]; then echo "  note BOARD_UART is unset: the X2 console is optional, the record is the result (no console, no recoveries observed)"
else bad "BOARD_UART is unset"; fi
if [ "$board" = x2 ]; then
	echo "  info board x2: measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL; tools/board.sh x2, crash-route kexec)"
	for t in kexec_zephyr snvs_lpgpr; do
		if [ -x "${X2_TOOLS:-$ROOT/driver/tools/kexec-zephyr}/$t" ]; then ok "x2 tool driver/tools/kexec-zephyr/$t is built"
		else bad "x2 tool driver/tools/kexec-zephyr/$t is not built (driver/tools/kexec-zephyr/README.md)"; fi
	done
fi

pkg=${DEQP_STAGE1:-}/build/modules/gles3/libdeqp-gles3-package.a
if [ -n "${DEQP_STAGE1:-}" ] && [ -f "$pkg" ]; then ok "stage 1 package exists (build/modules/gles3/libdeqp-gles3-package.a)"
else bad "stage 1 package missing (<DEQP_STAGE1>/build/modules/gles3/libdeqp-gles3-package.a)"; fi

if [ ! -f "$table" ]; then bad "table file not found"
elif python3 "$HERE/check-imx8mp-gles3-tree.py" -q --table "$table" > /dev/null 2>&1; then ok "table passes check-imx8mp-gles3-tree.py"
else bad "table fails check-imx8mp-gles3-tree.py"; fi

if [ -z "$commit" ]; then commit=$(git -C "$ROOT" rev-parse origin/main 2> /dev/null || true); fi
case $commit in
	'') bad "no commit: origin/main does not resolve (give --commit)" ;;
	*[!0-9a-fA-F]*) bad "commit is not hex"; commit='' ;;
	*) if [ ${#commit} -lt 8 ]; then bad "commit is shorter than 8 hex digits"; commit=''; fi ;;
esac
commit=$(printf %s "$commit" | tr 'A-F' 'a-f')

images=()
if [ -n "$idir" ]; then
	for d in "$idir"/es3-*; do
		if [ -f "$d/zephyr/zephyr.bin" ]; then images+=("$(basename "$d")"); fi
	done
fi
lay='<out>/es3-<hash8>/zephyr/zephyr.bin'
[ "$board" = x2 ] && lay='<out>/x2/es3-<hash8>/zephyr/zephyr.bin'
if [ ${#images[@]} -gt 0 ]; then ok "table image exists (${images[*]})"
else bad "no table image $lay"; fi
if [ -n "$commit" ]; then
	match=''
	for n in "${images[@]}"; do
		h=${n#es3-}
		case $commit in "$h"*) match=$n ;; esac
		case $h in "$commit"*) match=$n ;; esac
	done
	if [ -n "$match" ]; then ok "image $match is commit ${commit:0:8}"
	else bad "no image is named after commit ${commit:0:8}"; fi
	# board.sh x2 refuses an image without the GC watchdog self-arm marker (no remote power control there)
	if [ "$board" = x2 ] && [ -n "$match" ]; then
		if grep -aqF 'gc_wdog_arm: armed' "$idir/$match/zephyr/zephyr.bin"; then ok "image $match carries 'gc_wdog_arm: armed' (WDOG1 self-armed)"
		else bad "image $match lacks 'gc_wdog_arm: armed': build with IMX8MP_BOARD=x2 (WDOG_SELF_ARM not 0)"; fi
	fi
fi

# The Armadillo-X2 (read-only, over ssh to $BOARD_HOST; ssh options from $BOARD_SSH_OPTS, word-split): the
# device-tree model and compatible (the Armadillo-X2, an i.MX 8M Plus), /tmp a tmpfs (tools/board.sh x2 stages the
# loader, snvs_lpgpr and the image in /tmp/yakogl-kexec only), and where /var/tmp is (board.sh's lease file).
# A FAIL here needs a person, not a retry. The board's address and host name are never printed.
x2_probe='echo "model=$(tr -d "\000" 2>/dev/null < /proc/device-tree/model)"
echo "compat=$(tr "\000" " " 2>/dev/null < /proc/device-tree/compatible)"
echo "tmpfs=$(awk '"'"'$2 == "/tmp" { t = $3 } END { print t }'"'"' /proc/mounts 2>/dev/null)"
m=$(df -P /var/tmp 2>/dev/null | awk '"'"'NR == 2 { print $6 }'"'"')
echo "vartmp=$(awk -v m="$m" '"'"'$2 == m { t = $3 " " $1 " at " $2 } END { print t }'"'"' /proc/mounts 2>/dev/null)"
echo probe=done'
x2_check() {
	local probe model compat tfs vt
	if [ -z "${BOARD_HOST:-}" ]; then echo "  note the board not checked: BOARD_HOST is unset"; return; fi
	# shellcheck disable=SC2086
	probe=$(timeout 30 ssh -o BatchMode=yes -o ConnectTimeout=8 ${BOARD_SSH_OPTS:-} "$BOARD_HOST" "$x2_probe" 2> /dev/null)
	case $probe in *probe=done*) ;; *) echo "  note the board not checked: the board did not answer"; return ;; esac
	pv() { printf '%s\n' "$probe" | sed -n "s/^$1=//p" | head -1; }
	model=$(pv model); compat=$(pv compat); tfs=$(pv tmpfs); vt=$(pv vartmp)
	case $model in
		*[Aa]rmadillo*[Xx]2*)
			case " $compat " in
				*" fsl,imx8mp "*) ok "the board is the Armadillo-X2 (model '$model', fsl,imx8mp)" ;;
				*) bad "model '$model' but no fsl,imx8mp in the device-tree compatible: the slices need a person's decision" ;;
			esac ;;
		'') bad "the board's device-tree model is unreadable: not shown to be the Armadillo-X2; the slices need a person's decision" ;;
		*) bad "the board is '$model', not the Armadillo-X2 (--board x2): the slices need a person's decision" ;;
	esac
	if [ "$tfs" = tmpfs ]; then ok "/tmp is a tmpfs: a run stages only in /tmp/yakogl-kexec, gone at the reboot that ends it"
	else bad "/tmp is '${tfs:-unknown}', not a tmpfs: a run would stage outside a tmpfs; the slices need a person's decision"; fi
	echo "  note /boot medium guard does not apply on the X2: the crash-route kexec writes nothing to /boot, the eMMC or the U-Boot env"
	echo "  info /var/tmp (tools/board.sh's board lease file) is on: ${vt:-unknown}"
}

# /boot medium (read-only, over ssh to $BOARD_HOST; ssh options from $BOARD_SSH_OPTS, word-split). The slice runner
# stages /boot/yakogl/once-*.bin; that must never land on the eMMC, on a protected microSD partition or on the
# rootfs. A FAIL here needs a person's decision, not a retry: "Which medium is the Verdin's /boot on, and may the
# runner stage files there?" The board's address and host name are never printed.
boot_probe='b=$(findmnt -no SOURCE,TARGET /boot 2>/dev/null); echo "mnt=$b"
src=${b%% *}; src=${src%%\[*}
if [ -n "$b" ] && [ -b "$src" ]; then
 echo "plabel=$(lsblk -ndo PARTLABEL "$src" 2>/dev/null)"
 d=$(lsblk -ndo PKNAME "$src" 2>/dev/null); echo "disk=$d"
 echo "type=$(cat /sys/block/$d/device/type 2>/dev/null)"
 if [ -e /sys/block/${d}boot0 ]; then echo boot0=yes; else echo boot0=no; fi
fi
echo "fwenv=$(awk '"'"'$1 !~ /^#/ && NF {print $1; exit}'"'"' /etc/fw_env.config 2>/dev/null)"
echo probe=done'
if [ "$board" = x2 ]; then
	x2_check
elif [ -z "${BOARD_HOST:-}" ]; then
	echo "  note /boot medium not checked: BOARD_HOST is unset"
else
	# shellcheck disable=SC2086
	probe=$(timeout 30 ssh -o BatchMode=yes -o ConnectTimeout=8 ${BOARD_SSH_OPTS:-} "$BOARD_HOST" "$boot_probe" 2> /dev/null)
	case $probe in
		*probe=done*)
			pv() { printf '%s\n' "$probe" | sed -n "s/^$1=//p" | head -1; }
			b=$(pv mnt); plabel=$(pv plabel); disk=$(pv disk); dtype=$(pv type); boot0=$(pv boot0); fwenv=$(pv fwenv)
			src=${b%% *}; src=${src%%\[*}
			why=''
			if [ -z "$b" ]; then why="/boot is not a separate mount, so staging would land on the rootfs"
			elif [ -z "$disk" ]; then why="/boot ($src) is not on a partition of a disk"
			elif [ "$dtype" = MMC ] || [ "$boot0" = yes ]; then why="/boot ($src) is on the eMMC disk $disk"
			else
				case $plabel in
					fsbla1|fsbla2|metadata1|metadata2|fip-a|fip-b) why="/boot ($src) is on the protected microSD partition '$plabel'" ;;
				esac
			fi
			if [ -n "$why" ]; then
				bad "$why: the slices need a person's decision (which medium is /boot on, may the runner stage files there?)"
			elif [ "$dtype" = SD ]; then ok "/boot is $src, partlabel '${plabel:-none}', SD card $disk (not eMMC, not protected)"
			else bad "/boot ($src) is on disk $disk of unknown type '$dtype': not an SD card; the slices need a person's decision"; fi
			if [ -n "$fwenv" ]; then echo "  info U-Boot env device (/etc/fw_env.config): $fwenv"
			else echo "  info U-Boot env device: /etc/fw_env.config not readable"; fi ;;
		*) echo "  note /boot medium not checked: the board did not answer" ;;
	esac
fi

if [ $fails -eq 0 ]; then echo "preflight: all ok"; exit 0; fi
echo "preflight: $fails check(s) FAILED"
exit 1
