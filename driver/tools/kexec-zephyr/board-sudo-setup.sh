#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board-sudo-setup.sh - give the torizon user on the Verdin i.MX8MP a
# password-less sudo for exactly the three things the lab needs and nothing
# else: reloading the gcgpu driver (a wedged GPU is cleared only by a module
# reload), loading a newer gcgpu.ko from one fixed directory, and reading the
# kernel log (measured on the lab board: plain dmesg works for torizon there,
# so the rule only matters on an image that sets kernel.dmesg_restrict; the
# wedge capture falls back to the unprivileged read and says which it used).
#
# Run it ONCE from the host. The board's sudo password is typed interactively
# on the ssh -t prompt; it never enters a script, an environment variable or
# a command line, which is the rule this repository keeps for every board.
#
#   BOARD_HOST=torizon@<i.MX8MP> driver/tools/kexec-zephyr/board-sudo-setup.sh [--ko-dir DIR] [--blacklist-galcore] [--dry-run]
#
# What it installs on the board:
#
#   /usr/local/sbin/gcgpu-reload   refuses: gcgpu.ko is retired; kept
#                                  because the sudoers rule names it
#   /usr/local/sbin/kexec-zephyr   unload the GPU drivers and boot the staged Zephyr
#   /usr/local/sbin/fault-record-dump  print the 64-byte A53 fault record (no arguments)
#                                  image: $KEXEC_DIR/kexec_zephyr $KEXEC_DIR/zephyr.bin
#                                  [0xLOADADDR] [--crash]; those two arguments are the
#                                  only ones it accepts
#   /etc/sudoers.d/gcgpu           torizon may run, without a password:
#                                    gcgpu-reload, kexec-zephyr, rmmod gcgpu,
#                                    rmmod galcore, modprobe galcore, dmesg
#
# modprobe galcore is the mirror of rmmod galcore: the vendor-stack control
# (tools/cts, the weston-vivante container) needs the vendor driver back for
# its arms, and gcgpu-reload takes the GPU away from it again afterwards.
#   /etc/modprobe.d/gcgpu-lab.conf (only with --blacklist-galcore) "blacklist galcore"
#
# Both wrappers run files torizon can overwrite (the module, the loader, the
# image), so in effect torizon can run code as root on this lab board; that is
# the trade the lab made when it staged gcgpu.ko, and the wrappers exist to keep
# the sudoers rule from becoming "run anything" by accident rather than to stop
# a determined torizon. Do not install this on a board that is not the lab's.
#
# galcore is NXP's own driver for the same GPU. It auto-loads on every boot and
# binds the node, after which gcgpu loads but never binds (no /dev/gcgpu, no
# gcgpu line in dmesg). A board that rebooted (2026-09-22: a watchdog reset
# under a wedge) therefore comes back unusable for the lab until galcore is
# removed, so the wrapper removes it itself and the sudoers rule allows the
# manual form too. --blacklist-galcore stops it loading at boot at all; that
# turns off the stock GPU stack (weston, the vendor GLES) on that board, so it
# is opt-in.
#
# /etc on Torizon OS is writable and survives OSTree updates (it is merged
# into each deployment), so this is done once per board, not once per image.
#
# gcgpu.ko is RETIRED: the C kernel side it was built from is
# deleted. The installed gcgpu-reload now refuses with that message. The
# kexec-zephyr and fault-record-dump wrappers are what the Zephyr crash route
# uses, and they are unchanged.
#
# After it, the lab tooling uses:
#
#   ssh $BOARD_HOST sudo -n /usr/local/sbin/kexec-zephyr ...  # boot the staged Zephyr image (kexec_run.sh)
#   ssh $BOARD_HOST sudo -n dmesg                            # the wedge capture
#
# and `sudo -n` fails loud (exit 1, "a password is required") if the rule is
# ever missing, instead of hanging on a prompt nobody can answer.

set -euo pipefail

die() { echo "board-sudo-setup: $*" >&2; exit 2; }

KO_DIR=/home/${BOARD_USER:-torizon}/gcgpu
KEXEC_DIR=/home/${BOARD_USER:-torizon}/kexec
DRY_RUN=0
BLACKLIST_GALCORE=0
while [ $# -gt 0 ]; do
	case "$1" in
	--ko-dir) [ $# -ge 2 ] || die "--ko-dir needs a directory"; KO_DIR=$2; shift 2 ;;
	--dry-run) DRY_RUN=1; shift ;;
	--blacklist-galcore) BLACKLIST_GALCORE=1; shift ;;
	-h|--help) sed -n '2,42p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
	*) die "unknown argument: $1" ;;
	esac
done

[ -n "${BOARD_HOST:-}" ] || die "BOARD_HOST is not set (torizon@<i.MX8MP>); there is no default on purpose"
case "$KO_DIR" in
/*) ;;
*) die "--ko-dir must be an absolute path on the board (got '$KO_DIR')" ;;
esac

# gcgpu.ko is retired, so the wrapper only refuses. It is still
# installed because /etc/sudoers.d/gcgpu names it: a board set up before the
# retirement then gets the reason instead of a stale module.
wrapper=$(cat <<EOF
#!/bin/sh
# gcgpu-reload - installed by driver/tools/kexec-zephyr/board-sudo-setup.sh.
echo "gcgpu-reload: gcgpu.ko is retired with the C kernel side; the Rust kernel side runs on Zephyr; see driver/docs/vivante-kernel-rust.md" >&2
exit 2
EOF
)

# CONFIG_GC_SAMPLE_FAULT_RECORD_ADDR in the Zephyr images (zephyr/lib/board/imx8mp/Kconfig
# default). If an image moves it, this must move with it or the dump reads
# the wrong page and decodes to EMPTY -- which reads like "it did not fault".
: "${FAULT_RECORD_ADDR:=$((0x81006000))}"

# The fault-record wrapper: ONE fixed physical address, no arguments at all.
#
# A Zephyr sample leaves its fault syndrome in a page of carveout DDR that
# Linux leaves no-map (gc_sample.c, CONFIG_GC_SAMPLE_FAULT_RECORD), and that
# page is the only exact channel this board has: the panel needs eyes and the
# return time carries one small number. Reading it means /dev/mem, which is
# root.
#
# So the wrapper takes NO arguments and the address is baked in. `dd
# if=/dev/mem` in a sudoers rule would be a read-anything primitive -- kernel
# memory, keys, anything -- and this is the narrowest thing that still answers
# the question: 64 bytes at one address, base64 on stdout.
fault_wrapper=$(cat <<EOF
#!/bin/sh
# fault-record-dump - print the A53 fault record (64 bytes, base64), as root.
# Installed by driver/tools/kexec-zephyr/board-sudo-setup.sh. NO ARGUMENTS: the
# address is fixed here on purpose, so this cannot become a general
# read-anything. tools/cts/fault-record.py decodes what it prints.
set -e
ADDR=$FAULT_RECORD_ADDR
if [ \$# -ne 0 ]; then
	echo "fault-record-dump: takes no arguments (the address is fixed)" >&2
	exit 2
fi
dd if=/dev/mem bs=1 skip=\$ADDR count=64 status=none | base64 -w0
echo
EOF
)

# The kexec wrapper: one fixed loader, one fixed image, and the only two
# arguments the loader takes from the lab (a hex load address, --crash). The
# GPU driver has to release the GPU before Zephyr takes it (kexec_run.sh's
# BOARD_GPU_MODULES), so the wrapper unloads whichever of the four is loaded.
kexec_wrapper=$(cat <<EOF
#!/bin/sh
# kexec-zephyr - boot the lab's staged Zephyr image from Linux, as root.
# Installed by driver/tools/kexec-zephyr/board-sudo-setup.sh; the sudoers rule in
# /etc/sudoers.d/gcgpu names this file, so keep it running only \$LOADER on
# \$IMG. driver/tools/kexec-zephyr/kexec_run.sh stages both and calls this.
set -e
LOADER=$KEXEC_DIR/kexec_zephyr
IMG=$KEXEC_DIR/zephyr.bin
ADDR=""
CRASH=""
for a in "\$@"; do
	case "\$a" in
	--crash) CRASH=--crash ;;
	*)
		if printf '%s' "\$a" | grep -Eq '^0x[0-9a-fA-F]{1,16}\$'; then
			ADDR=\$a
		else
			echo "kexec-zephyr: unexpected argument '\$a' (only --crash and a 0x load address)" >&2
			exit 2
		fi
		;;
	esac
done
[ -x "\$LOADER" ] || { echo "kexec-zephyr: no loader at \$LOADER" >&2; exit 2; }
[ -f "\$IMG" ] || { echo "kexec-zephyr: no image at \$IMG" >&2; exit 2; }
sync
for m in pvrgpu pvrsrvkm gcgpu galcore; do
	if lsmod | grep -q "^\$m "; then
		rmmod "\$m"
	fi
done
echo "kexec-zephyr: jumping to \$IMG \$ADDR \$CRASH"
exec "\$LOADER" "\$IMG" \$ADDR \$CRASH
EOF
)

# Paths are read off the board, not assumed: sudoers matches the full path.
remote_paths='printf "%s %s %s\n" "$(command -v rmmod)" "$(command -v dmesg)" "$(command -v modprobe)"'

if [ "$DRY_RUN" = 1 ]; then
	echo "== dry run: would connect to $BOARD_HOST and install"
	echo "-- /usr/local/sbin/gcgpu-reload:"
	printf '%s\n' "$wrapper" | sed 's/^/   /'
	echo "-- /usr/local/sbin/kexec-zephyr:"
	printf '%s\n' "$kexec_wrapper" | sed 's/^/   /'
	echo "-- /etc/sudoers.d/gcgpu (rmmod/dmesg paths read from the board at run time):"
	echo "   torizon ALL=(root) NOPASSWD: /usr/local/sbin/gcgpu-reload, /usr/local/sbin/kexec-zephyr, /usr/local/sbin/fault-record-dump, <rmmod> gcgpu, <rmmod> galcore, <modprobe> galcore, <dmesg>"
	[ "$BLACKLIST_GALCORE" = 1 ] && echo "-- /etc/modprobe.d/gcgpu-lab.conf: blacklist galcore"
	echo "-- then: visudo -c on the file, and 'sudo -n <dmesg> | tail -1' as the proof"
	exit 0
fi

echo "== reading tool paths from $BOARD_HOST"
read -r RMMOD DMESG MODPROBE < <(ssh -o BatchMode=yes "$BOARD_HOST" "$remote_paths")
[ -n "$RMMOD" ] && [ -n "$DMESG" ] && [ -n "$MODPROBE" ] || die "could not resolve rmmod/dmesg/modprobe on the board (got '$RMMOD' '$DMESG' '$MODPROBE')"
echo "   rmmod=$RMMOD dmesg=$DMESG modprobe=$MODPROBE"


sudoers="torizon ALL=(root) NOPASSWD: /usr/local/sbin/gcgpu-reload, /usr/local/sbin/kexec-zephyr, /usr/local/sbin/fault-record-dump, $RMMOD gcgpu, $RMMOD galcore, $MODPROBE galcore, $DMESG"

# One sudo, one password prompt, everything written in a root shell: the
# wrapper, the sudoers file (mode 440, then visudo -c; a file visudo rejects
# is removed again so a typo cannot lock sudo), and the staging directory.
remote_install=$(cat <<EOF
set -e
umask 022
cat > /usr/local/sbin/gcgpu-reload <<'WRAP'
$wrapper
WRAP
chmod 755 /usr/local/sbin/gcgpu-reload
cat > /usr/local/sbin/kexec-zephyr <<'WRAP2'
$kexec_wrapper
WRAP2
chmod 755 /usr/local/sbin/kexec-zephyr
cat > /usr/local/sbin/fault-record-dump <<'WRAP3'
$fault_wrapper
WRAP3
chmod 755 /usr/local/sbin/fault-record-dump
mkdir -p /etc/sudoers.d
printf '%s\n' '$sudoers' > /etc/sudoers.d/gcgpu
chmod 440 /etc/sudoers.d/gcgpu
if ! visudo -c -f /etc/sudoers.d/gcgpu; then
	rm -f /etc/sudoers.d/gcgpu
	echo "sudoers file rejected by visudo; removed again" >&2
	exit 1
fi
mkdir -p '$KO_DIR' '$KEXEC_DIR'
chown torizon:torizon '$KO_DIR' '$KEXEC_DIR'
if [ '$BLACKLIST_GALCORE' = 1 ]; then
	mkdir -p /etc/modprobe.d
	printf 'blacklist galcore\n' > /etc/modprobe.d/gcgpu-lab.conf
	echo "installed: /etc/modprobe.d/gcgpu-lab.conf (galcore no longer loads at boot)"
fi
echo "installed: /usr/local/sbin/gcgpu-reload, /usr/local/sbin/kexec-zephyr, /usr/local/sbin/fault-record-dump, /etc/sudoers.d/gcgpu, $KO_DIR/, $KEXEC_DIR/"
EOF
)

echo "== installing on $BOARD_HOST (type the board's sudo password at the prompt)"
ssh -t "$BOARD_HOST" "sudo sh -c '$(printf '%s' "$remote_install" | sed "s/'/'\\\\''/g")'"

echo "== proof: a password-less dmesg and the wrapper's presence"
ssh -o BatchMode=yes "$BOARD_HOST" "sudo -n $DMESG | tail -1 && test -x /usr/local/sbin/gcgpu-reload && echo 'gcgpu-reload: present' && test -x /usr/local/sbin/kexec-zephyr && echo 'kexec-zephyr: present' && { ls -l $KO_DIR/gcgpu.ko 2>/dev/null || echo \"$KO_DIR/gcgpu.ko: not staged yet (copy the module there before the first reload)\"; }"
echo "== done. Boot Zephyr with: driver/tools/kexec-zephyr/kexec_run.sh (it calls sudo -n /usr/local/sbin/kexec-zephyr)"
