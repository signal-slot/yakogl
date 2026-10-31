#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# pi4-gles3-run.sh: the dEQP-GLES3 tree on the Pi 4 from one checkout, the
# commands of docs/es3/board-recipe-pi4.md sections 3 to 5 with the
# environment filled from the dependency layout and the leased resources.
#
#   tools/cts/pi4-gles3-run.sh build [LABEL...]    images into build/deqp/pi4-gles3-<HEAD8>
#   tools/cts/pi4-gles3-run.sh run [rows]          board.sh pi4 tree (TREE_ROWS=<rows> when given)
#   tools/cts/pi4-gles3-run.sh tally               the tally of the records
#
# Environment (nothing about a machine or an address is in this file):
#   CONNECTED_DEPS_DIR      the dependency layout (deqp-stage1-rpi_4b-gles3 from
#                           pi4-gles3-stage1.sh, zephyr-ws, the SDK)
#   RESOURCE_PI4_ELEGANCE_SSH  user@host of the board's Linux (else BOARD_HOST)
#   RESOURCE_USB1_HOST, RESOURCE_USB1_PORT  the serial bridge (else BOARD_UART)
#   BOARD_HDMI_TIMINGS      else read from the board's tryboot.txt/config.txt
# The records go to build/records-pi4-gles3-<HEAD8> (not in the tree).
set -u
REPO=$(cd "$(dirname "$0")/../.." && pwd)
DEPS=${CONNECTED_DEPS_DIR:-${CCFARM_DEPS_DIR:-}}
[ -n "$DEPS" ] || { echo "pi4-gles3-run: set CONNECTED_DEPS_DIR" >&2; exit 2; }
cd "$REPO"
HEAD8=${PI4_GLES3_BUILD:-$(git rev-parse --short=8 HEAD)} # PI4_GLES3_BUILD: the commit the images were built from, when HEAD moved since
IMG=build/deqp/pi4-gles3-$HEAD8
REC=build/records-pi4-gles3-$HEAD8
TABLE=tools/cts/pi4-gles3-tree.txt
export BOARD_MUSTPASS=$REPO/tools/cts/gles3-mustpass.txt
verb=${1:-}; shift || true
case "$verb" in
build)
	export ZEPHYR_WS=$DEPS/zephyr-ws WEST_VENV=$DEPS/zephyr-ws/.venv
	export ZEPHYR_SDK_INSTALL_DIR=$DEPS/zephyr-sdk-1.0.1 ZEPHYR_TOOLCHAIN_VARIANT=zephyr
	export RUSTUP_TOOLCHAIN=nightly DEQP_WORK=$DEPS/deqp-stage1-rpi_4b-gles3 DEQP_MODULE=gles3
	export CMAKE_POLICY_VERSION_MINIMUM=3.5 PATH=$HOME/.cargo/bin:$PATH
	if [ $# -eq 0 ]; then # every label whose image is not built yet (the builder never reuses a directory)
		for l in $(awk '/^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); print l }' "$TABLE"); do
			[ -e "$IMG/$l" ] || set -- "$@" "$l"
		done
	fi
	exec tools/cts/pi4-deqp-images.sh "$TABLE" "$IMG" "$@"
	;;
run | tally)
	export BOARD_HOST=${BOARD_HOST:-${RESOURCE_PI4_ELEGANCE_SSH:-}}
	export BOARD_UART=${BOARD_UART:-${RESOURCE_USB1_HOST:-}:${RESOURCE_USB1_PORT:-}}
	[ -n "$BOARD_HOST" ] || { echo "pi4-gles3-run: no BOARD_HOST" >&2; exit 2; }
	if [ -z "${BOARD_HDMI_TIMINGS:-}" ]; then
		# the panel's timings, as the board's own boot files carry them
		BOARD_HDMI_TIMINGS=$(ssh "$BOARD_HOST" -- 'cat /boot/firmware/tryboot.txt /boot/firmware/config.txt 2>/dev/null' |
			sed -n 's/^hdmi_timings=//p' | head -1)
		export BOARD_HDMI_TIMINGS
	fi
	[ -n "$BOARD_HDMI_TIMINGS" ] || { echo "pi4-gles3-run: no BOARD_HDMI_TIMINGS" >&2; exit 2; }
	# board.sh refuses a board whose who(1) lists a session from an address that
	# is not one of this host's. The lease forwards the board to this machine,
	# so our own ssh login shows with the forwarder's address (the one the
	# board reports in $SSH_CONNECTION). A wrapper `ip` first on PATH adds that
	# one address to `ip -o addr`; the board's other sessions still refuse.
	own=$(ssh "$BOARD_HOST" -- 'echo ${SSH_CONNECTION%% *}')
	if [ -n "$own" ] && ! ip -o addr 2>/dev/null | grep -qF "$own/"; then
		mkdir -p build/pi4-bin
		realip=$(command -v ip)
		printf '%s\n' '#!/bin/bash' "$realip \"\$@\"" \
			"case \"\$*\" in *'-o addr'*) echo '9: lease inet $own/32 scope global lease' ;; esac" > build/pi4-bin/ip
		chmod +x build/pi4-bin/ip
		export PATH=$REPO/build/pi4-bin:$PATH
	fi
	# The lease forwards the board with a login that only takes the form
	# `ssh user@host -- command`; board.sh calls `ssh -o K=V ... host command`.
	# A wrapper `ssh` first on PATH turns the second form into the first
	# (dropping the -o pairs) and logs a failed call to build/pi4-bin/ssh.log.
	# scp's own calls (joined -oK=V options) pass through unchanged.
	mkdir -p build/pi4-bin
	realssh=$(PATH=${PATH#"$REPO/build/pi4-bin:"} command -v ssh)
	cat > build/pi4-bin/ssh <<EOF
#!/bin/bash
real=$realssh
case "\$1" in
-o) ;;
*) exec "\$real" "\$@" ;;
esac
while [ "\$1" = -o ]; do shift 2; done
host=\$1
shift
t0=\$(date +%s)
if [ -p /dev/stdin ] || [ -f /dev/stdin ]; then "\$real" "\$host" -- "\$@"; else "\$real" -n "\$host" -- "\$@"; fi
rc=\$?
echo "\$(date +%T) rc=\$rc \$((\$(date +%s) - t0))s \$host \$*" | cut -c1-160 >> $REPO/build/pi4-bin/ssh.log
exit \$rc
EOF
	chmod +x build/pi4-bin/ssh
	export PATH=$REPO/build/pi4-bin:$PATH
	if [ "$verb" = tally ]; then exec tools/board.sh pi4 tally "$REC"; fi
	[ $# -gt 0 ] && export TREE_ROWS=$1
	export TREE_IMAGE_DIR=$IMG
	exec tools/board.sh pi4 tree "$TABLE" "$REC"
	;;
*) echo "usage: $0 build [LABEL...] | run [rows] | tally" >&2; exit 2 ;;
esac
