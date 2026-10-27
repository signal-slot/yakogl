#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# zephyr-modules.sh - print, ';'-separated, the Zephyr module list for building this repository's samples
# in a west workspace: every cloned project of the workspace except Zephyr itself and except a full
# platform checkout, followed by this repository's driver/ and root.
#
#   tools/cts/zephyr-modules.sh <west workspace top>
#   west build ... -- "-DZEPHYR_MODULES=$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")"
#
# Why: the Verdin AM62P and Verdin iMX8M Plus A53 board support now lives in driver/ (boards/, soc/,
# dts/). A workspace whose manifest repository is a checkout of the full platform (which
# carried the same boards before) makes that checkout a module too, and Zephyr then stops with
# "Board(s): {'verdin_am62p'}, defined multiple times". Passing ZEPHYR_MODULES explicitly replaces
# west's module discovery, so such a workspace keeps working. In a workspace made from this
# repository's west.yml the list is simply what west would have found.
#
# A project is taken for a checkout of the full platform when it has boards/toradex/verdin_am62p/board.yml and is not
# this repository. Needs west on PATH (the workspace's venv).
#
# Every path is compared in its real spelling (pwd -P): west prints paths resolved through symbolic
# links, so a logical pwd does not match them when the workspace is reached through a link. The
# mismatch puts the Zephyr checkout into ZEPHYR_MODULES a second time under its real path, and
# Zephyr then records its arch linker snippets a second time as ../-chains (zephyr/linker_snippets.cmake).
set -euo pipefail
WS=${1:?usage: $0 <west workspace top>}
REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
command -v west >/dev/null || { echo "zephyr-modules.sh: west not on PATH" >&2; exit 2; }
zephyr=$(cd "$WS/zephyr" && pwd -P)
mods=()
while IFS= read -r p; do
	[ -d "$p" ] || continue
	p=$(cd "$p" && pwd -P)
	[ "$p" = "$zephyr" ] && continue
	[ "$p" = "$REPO" ] && continue
	[ -f "$p/boards/toradex/verdin_am62p/board.yml" ] && continue
	mods+=("$p")
done < <(cd "$WS" && west list -f '{abspath}')
mods+=("$REPO/driver" "$REPO")
(IFS=';'; echo "${mods[*]}")
