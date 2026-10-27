# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-env.sh - sourced by the i.MX8MP build scripts (imx8mp-deqp-stage1.sh, imx8mp-deqp-build.sh,
# imx8mp-gltest-build.sh): checks the Zephyr build environment they need and fails with a clear message
# when something is unset. Not meant to be run on its own. Running the images on the board is
# tools/board.sh imx8mp (its board half, BOARD_HOST and the kexec loader, went there in Step 1).
#
# Needs (export them; nothing here has a machine-specific default):
#   ZEPHYR_WORKSPACE        the west workspace top (the directory holding zephyr/ and .west/)
#   ZEPHYR_VENV             a Python venv with west installed (its bin/activate is sourced)
#   ZEPHYR_SDK_INSTALL_DIR  the Zephyr SDK (1.0.1 was used; the aarch64-zephyr-elf toolchain)
#   IMX8MP_OUT              where images and logs are written (one sub-directory per image)
# Optional:
#   CLUSTER_DIR              ignored: the Verdin iMX8M Plus A53 board is in driver/boards (a warning is printed)
#   ZEPHYR_BASE             ignored when it is not $ZEPHYR_WORKSPACE/zephyr: it is set from the workspace (a line is printed)
#   BOARD                   Zephyr board (default verdin_imx8mp/mimx8ml8/a53)
# No credentials are read or written by these scripts.

imx8mp_need() {
	for v in "$@"; do
		eval "val=\${$v:-}"
		if [ -z "$val" ]; then
			echo "imx8mp: $v is not set -- see tools/cts/imx8mp-env.sh for what each variable is" >&2
			exit 2
		fi
	done
}

# The repository root, from this file's location (tools/cts/).
YAKOGL_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
BOARD=${BOARD:-verdin_imx8mp/mimx8ml8/a53}

imx8mp_zephyr_env() {
	imx8mp_need ZEPHYR_WORKSPACE ZEPHYR_VENV ZEPHYR_SDK_INSTALL_DIR IMX8MP_OUT
	[ -n "${CLUSTER_DIR:-}" ] && echo "imx8mp: CLUSTER_DIR is ignored (the board support is in driver/)" >&2
	[ -f "$ZEPHYR_VENV/bin/activate" ] || { echo "imx8mp: $ZEPHYR_VENV/bin/activate not found" >&2; exit 2; }
	# shellcheck disable=SC1091
	. "$ZEPHYR_VENV/bin/activate"
	# ZEPHYR_BASE always follows the workspace: a value preset for another workspace would build the
	# wrong Zephyr tree, so it is replaced (one line says so).
	[ -d "$ZEPHYR_WORKSPACE/zephyr" ] || { echo "imx8mp: $ZEPHYR_WORKSPACE/zephyr not found (ZEPHYR_WORKSPACE must be the west workspace top)" >&2; exit 2; }
	zb=$(cd "$ZEPHYR_WORKSPACE/zephyr" && pwd -P)
	if [ -n "${ZEPHYR_BASE:-}" ] && [ "$(cd "$ZEPHYR_BASE" 2>/dev/null && pwd -P)" != "$zb" ]; then
		echo "imx8mp: preset ZEPHYR_BASE=$ZEPHYR_BASE is not in ZEPHYR_WORKSPACE; using $zb" >&2
	fi
	export ZEPHYR_BASE=$zb
	export ZEPHYR_SDK_INSTALL_DIR
	mkdir -p "$IMX8MP_OUT"
	# The module list, explicitly: a workspace whose manifest is a SafeUI checkout would otherwise
	# define the Verdin boards twice (see zephyr-modules.sh).
	ZEPHYR_MODULES=$("$YAKOGL_DIR/tools/cts/zephyr-modules.sh" "$ZEPHYR_WORKSPACE") || exit 2
	export ZEPHYR_MODULES
}
