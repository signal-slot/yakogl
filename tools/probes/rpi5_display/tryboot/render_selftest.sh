#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi5_display/tryboot/render_selftest.sh
#
# Hermetic self-test of render.sh and tryboot.txt.in: no board, no network.
# Ends with 'render_selftest: N of N checks, 0 failed'; exit 0 only when
# nothing failed.
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd) || exit 1
render=$here/render.sh
tpl=$here/tryboot.txt.in
scratch=$(mktemp -d "${TMPDIR:-/tmp}/render_selftest.XXXXXX") || exit 1
trap 'rm -rf "$scratch"' EXIT
mkdir "$scratch/tmp" "$scratch/cwd"

n=0
failed=0
check() { # check <name> <command...>
	local name=$1
	shift
	n=$((n + 1))
	if "$@" >/dev/null 2>&1; then
		echo "ok   $name"
	else
		echo "FAIL $name"
		failed=$((failed + 1))
	fi
}
run() { # run <args...>: render in the empty cwd with a private TMPDIR
	(cd "$scratch/cwd" && TMPDIR=$scratch/tmp bash "$render" "$@")
}
rc() { # rc <expected> <args...>
	local want=$1 got
	shift
	run "$@" >/dev/null 2>&1
	got=$?
	[ "$got" -eq "$want" ]
}

url_re='https://www\.raspberrypi\.com/documentation/computers/(legacy_)?config_txt\.html#'
key_re='^[a-z_0-9]+=[^[:space:]#]*$'

# Form: no '#' off comment lines, every key line bare, URL in the comment
# block directly above each key line. $2 = 1 allows the template's @ROUTE@.
form() {
	awk -v url="$url_re" -v key="$key_re" -v tplmode="$2" '
		/^#/ { block = block $0 "\n"; next }
		/^$/ { block = ""; next }
		{
			if ($0 ~ /#/) { print "hash on line " NR; bad = 1 }
			if (tplmode && $0 == "@ROUTE@") { block = ""; next }
			if (!($0 ~ key || $0 == "[all]")) { print "bad key line " NR ": " $0; bad = 1 }
			if (block !~ url) { print "no citation above line " NR; bad = 1 }
			block = ""
		}
		END { exit bad }' "$1"
}

run fw kernel8.img >"$scratch/fw.txt"
run bare kernel8.img >"$scratch/bare.txt"

check "template form" form "$tpl" 1
check "fw form" form "$scratch/fw.txt" 0
check "bare form" form "$scratch/bare.txt" 0
check "fw has no trailing space" bash -c '! grep -q "[[:space:]]$" "$1"' _ "$scratch/fw.txt"
check "bare has no trailing space" bash -c '! grep -q "[[:space:]]$" "$1"' _ "$scratch/bare.txt"
check "fw kernel line exact" grep -qx 'kernel=kernel8.img' "$scratch/fw.txt"
check "bare kernel line exact" grep -qx 'kernel=kernel8.img' "$scratch/bare.txt"
run bare a.b_c+d-1.img >"$scratch/odd.txt"
check "kernel renders the whole name" grep -qx 'kernel=a.b_c+d-1.img' "$scratch/odd.txt"
check "fw base keys" bash -c 'for k in "[all]" enable_uart=1 uart_2ndstage=1 arm_64bit=1; do grep -qxF "$k" "$1" || exit 1; done' _ "$scratch/fw.txt"
check "fw route keys" bash -c 'for k in hdmi_force_hotplug=1 disable_fw_kms_setup=0 framebuffer_width=1920 framebuffer_height=1080 framebuffer_depth=32 framebuffer_ignore_alpha=1; do grep -qxF "$k" "$1" || exit 1; done' _ "$scratch/fw.txt"
check "fw has no bare keys" bash -c '! grep -qxF -e disable_fw_kms_setup=1 -e hdmi_force_hotplug=0 "$1"' _ "$scratch/fw.txt"
check "bare route keys" bash -c 'grep -qxF disable_fw_kms_setup=1 "$1" && grep -qxF hdmi_force_hotplug=0 "$1"' _ "$scratch/bare.txt"
check "bare has no fw keys" bash -c '! grep -q -e ^framebuffer -e hdmi_force_hotplug=1 -e disable_fw_kms_setup=0 "$1"' _ "$scratch/bare.txt"
check "bare names the documented stop" grep -q 'RPI_FIRMWARE_NOTIFY_DISPLAY_DONE' "$scratch/bare.txt"
check "fw marks legacy keys unverified" grep -q 'unverified on the Pi 5 (U3, board check)' "$scratch/fw.txt"
check "arm_64bit comment" grep -q 'ignore it' "$scratch/fw.txt"
check "no markers left" bash -c '! grep -q -e @ -e ROUTE "$1" "$2"' _ "$scratch/fw.txt" "$scratch/bare.txt"
leak='([0-9]{1,3}\.){3}[0-9]{1,3}|([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}|/dev/|tty|ssh|password|passwd|secret|token|credential|user|host|port'
check "no address, host or credential" bash -c '! grep -Eiq "$1" "$2" "$3"' _ "$leak" "$scratch/fw.txt" "$scratch/bare.txt"
run fw kernel8.img >"$scratch/fw2.txt"
check "deterministic" cmp "$scratch/fw.txt" "$scratch/fw2.txt"

check "render creates no files" bash -c '[ -z "$(ls -A "$1")$(ls -A "$2")" ]' _ "$scratch/cwd" "$scratch/tmp"

check "refuse empty name" rc 2 fw ""
check "refuse slash" rc 2 fw a/b
check "refuse dotdot path" rc 2 fw ../k
check "refuse space" rc 2 bare "a b"
check "refuse tab" rc 2 bare $'a\tb'
check "refuse newline" rc 2 bare $'a\nb'
check "refuse leading dot" rc 2 fw .hidden
check "refuse leading dash" rc 2 fw -x
check "refuse hash" rc 2 fw 'k#x'
check "refuse dollar" rc 2 fw 'k$x'
check "refuse 61 chars" rc 2 fw "$(printf 'a%.0s' {1..61})"
check "accept 60 chars" rc 0 fw "$(printf 'a%.0s' {1..60})"
check "refuse unknown variant" rc 2 other kernel8.img
check "refuse missing args" rc 2
check "refuse extra args" rc 2 fw a b

# Broken templates: missing route block (4), missing placeholder (5).
sed '/^# @ROUTE bare$/,/^# @ENDROUTE$/d' "$tpl" >"$scratch/noroute.in"
sed 's/^@ROUTE@$//' "$tpl" >"$scratch/noph.in"
sed 's/^kernel=@IMAGE@$/kernel=x/' "$tpl" >"$scratch/noimg.in"
check "exit 4 for missing route block" bash -c 'TMPDIR=$2 TRYBOOT_TEMPLATE=$1 bash "$3" bare k.img >/dev/null 2>&1; [ $? -eq 4 ]' _ "$scratch/noroute.in" "$scratch/tmp" "$render"
check "exit 5 for missing @ROUTE@" bash -c 'TMPDIR=$2 TRYBOOT_TEMPLATE=$1 bash "$3" fw k.img >/dev/null 2>&1; [ $? -eq 5 ]' _ "$scratch/noph.in" "$scratch/tmp" "$render"
check "exit 5 for missing @IMAGE@" bash -c 'TMPDIR=$2 TRYBOOT_TEMPLATE=$1 bash "$3" fw k.img >/dev/null 2>&1; [ $? -eq 5 ]' _ "$scratch/noimg.in" "$scratch/tmp" "$render"
check "no output on failure" bash -c '[ -z "$(TMPDIR=$2 TRYBOOT_TEMPLATE=$1 bash "$3" bare k.img 2>/dev/null)" ]' _ "$scratch/noroute.in" "$scratch/tmp" "$render"
check "scratch removed after failures" bash -c '[ -z "$(ls -A "$1")" ]' _ "$scratch/tmp"

# The self-test's own form check must reject the old trailing-comment form.
printf '# c\n[all]\nkernel=k.img  # https://www.raspberrypi.com/documentation/computers/config_txt.html#kernel\n' >"$scratch/old.txt"
notform() { ! form "$@"; }
check "form check rejects trailing comment" notform "$scratch/old.txt" 0

echo "render_selftest: $((n - failed)) of $n checks, $failed failed"
[ "$failed" -eq 0 ]
