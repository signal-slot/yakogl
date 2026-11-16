#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# t6-display-dump.sh without a board. A read of a display register whose
# power domain or bus clock is off may hang the NanoPC-T6's bus, and some
# words must never be read at all (nanopc-t6-display.md, "Must NOT be
# read"), so what the tool refuses is checked here before it meets a board:
#
#  - the read list: the built-in list passes its own check, and a line that
#    reads a forbidden word, another block's window or a regmap file other
#    than a GRF's is refused before the board is contacted;
#  - the dry run reads no register and copies nothing;
#  - --go refuses without T6_HAND or a record directory, when the board is
#    not a T6, when the login is not root, when the display is off (power
#    domain, bus clock, connector) at the first look or at the one under the
#    lock, and while another runner holds the lock;
#  - a full --go makes the reads of the list, in its order, and dump.txt
#    holds every word at its address;
#  - a display that goes off between two reads, a read that fails and a
#    read that returns fewer words stop the run there (exit 4): nothing
#    more is read, the record keeps the words before, and what was copied
#    to the board is removed.
#
# The board is a directory: an `ssh` first in PATH runs the command it is
# given with /sys, /proc and the staging directory rewritten into that
# directory, and MMIO_RD is a script that logs each call and answers
# address ^ 0x00ffff00, which is also what the made-up regmap files hold.
#
# The gate runs it from board_lock_selftest.sh.
set -u
here=$(cd "$(dirname "$0")" && pwd)
dump=$here/t6-display-dump.sh
run=0
failed=0
ran=()
check() { # <what> <0 = ok>
	run=$((run + 1))
	ran[${BASH_LINENO[0]}]=1
	if [ "$2" = 0 ]; then
		echo "$1 ok"
	else
		echo "$1 FAIL"
		failed=$((failed + 1))
	fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
tree=$tmp/board
dbg=$tree/sys/kernel/debug
value() { echo $(($1 ^ 0x00ffff00)); }

# ---- the fake board -----------------------------------------------------------
mkdir "$tmp/fakebin" "$tmp/boardbin"
cat >"$tmp/fakebin/ssh" <<EOF
#!/bin/sh
# The last argument is the command. $tree/off_at_ssh: the power domain vo1
# is off from the command of that number on.
for a in "\$@"; do cmd=\$a; done
echo ssh >>"$tree/ssh.log"
if [ -e "$tree/off_at_ssh" ] && [ "\$(wc -l <"$tree/ssh.log")" = "\$(cat "$tree/off_at_ssh")" ]; then
	sed -i 's/^vo1 .*/vo1 off-0 0/' "$dbg/pm_genpd/pm_genpd_summary"
fi
cmd=\$(printf '%s\n' "\$cmd" | sed -e 's#/tmp/yakogl-display-dump#$tree/stage#g' -e 's#/sys/#$tree/sys/#g' -e 's#/proc/#$tree/proc/#g')
PATH=$tmp/boardbin:\$PATH exec sh -c "\$cmd"
EOF
cat >"$tmp/fakebin/scp" <<EOF
#!/bin/sh
# The last two arguments are the file and <host>:<path>.
src= dst=
for a in "\$@"; do src=\$dst dst=\$a; done
cp "\$src" "\$(printf '%s\n' "\${dst#*:}" | sed 's#/tmp/yakogl-display-dump#$tree/stage#')"
EOF
printf '#!/bin/sh\ncat "%s/uid"\n' "$tree" >"$tmp/boardbin/id"
printf '#!/bin/sh\necho "[    2.000000] rockchip-drm display-subsystem: bound fde80000.hdmi"\n' >"$tmp/boardbin/dmesg"
# mmio_rd <address> [words]. $tree/fail_at: the read of that word fails.
# $tree/short_at: the read ends before that word. $tree/off_after: after the
# call that starts at that address, the power domain vo1 is off.
cat >"$tmp/mmio_rd" <<EOF
#!/bin/sh
echo "\$1 \${2:-1}" >>"$tree/mmio.log"
i=0
while [ "\$i" -lt "\${2:-1}" ]; do
	w=\$((\$1 + 4 * i))
	if [ -e "$tree/fail_at" ] && [ "\$w" = "\$((\$(cat "$tree/fail_at")))" ]; then
		echo "mmio_rd: the read failed" >&2
		exit 1
	fi
	if [ -e "$tree/short_at" ] && [ "\$w" = "\$((\$(cat "$tree/short_at")))" ]; then exit 0; fi
	printf '0x%08x: 0x%08x\n' "\$w" \$((w ^ 0x00ffff00))
	i=\$((i + 1))
done
if [ -e "$tree/off_after" ] && [ "\$((\$1))" = "\$((\$(cat "$tree/off_after")))" ]; then
	sed -i 's/^vo1 .*/vo1 off-0 0/' "$dbg/pm_genpd/pm_genpd_summary"
fi
EOF
chmod +x "$tmp/fakebin/ssh" "$tmp/fakebin/scp" "$tmp/boardbin/id" "$tmp/boardbin/dmesg" "$tmp/mmio_rd"

regmap_file() { # <directory name> <base> <digits of an offset> <last offset>
	local o
	mkdir -p "$dbg/regmap/$1"
	for ((o = 0; o <= $4; o += 4)); do
		printf "%0$3x: %08x\n" "$o" "$(value $(($2 + o)))"
	done >"$dbg/regmap/$1/registers"
}
# board_new: the T6 under Armbian with the picture on the first HDMI output.
board_new() {
	local k
	rm -rf "${tree:?}"
	mkdir -p "$tree/proc/device-tree" "$tree/sys/class/drm/card0-HDMI-A-1" "$tree/sys/class/drm/card0-HDMI-A-2" \
		"$dbg/pm_genpd" "$dbg/dri/0/vop2" "$dbg/clk"
	printf 'friendlyarm,nanopc-t6\0rockchip,rk3588\0' >"$tree/proc/device-tree/compatible"
	echo 0 >"$tree/uid"
	ln -s ../../devices/platform/fde80000.hdmi/i2c-9 "$tree/sys/class/drm/card0-HDMI-A-1/ddc"
	ln -s ../../devices/platform/fdea0000.hdmi/i2c-10 "$tree/sys/class/drm/card0-HDMI-A-2/ddc"
	echo connected >"$tree/sys/class/drm/card0-HDMI-A-1/status"
	echo enabled >"$tree/sys/class/drm/card0-HDMI-A-1/enabled"
	echo 1024x768 >"$tree/sys/class/drm/card0-HDMI-A-1/modes"
	: >"$tree/sys/class/drm/card0-HDMI-A-1/edid"
	echo disconnected >"$tree/sys/class/drm/card0-HDMI-A-2/status"
	echo disabled >"$tree/sys/class/drm/card0-HDMI-A-2/enabled"
	: >"$tree/sys/class/drm/card0-HDMI-A-2/modes"
	: >"$tree/sys/class/drm/card0-HDMI-A-2/edid"
	printf '%s\n' 'domain status children performance' '--------' 'vo1 on 0' 'vop on 0' 'vo0 off-0 0' 'gpu off-0 0' \
		>"$dbg/pm_genpd/pm_genpd_summary"
	for k in hclk_vop aclk_vop pclk_hdmitx0 pclk_hdptx0; do
		mkdir "$dbg/clk/$k"
		echo 1 >"$dbg/clk/$k/clk_enable_count"
	done
	printf '%s\n' ' hclk_vop 3 4 0 198000000' ' dclk_vop0 2 2 0 65000000' >"$dbg/clk/clk_summary"
	printf '%s\n' 'crtc[68]: video_port0' '	enable=1' '	active=1' \
		'	mode: "1024x768": 60 65000 1024 1048 1184 1344 768 771 777 806 0x40 0xa' \
		'crtc[88]: video_port1' '	enable=0' '	active=0' '	mode: "": 0 0 0 0 0 0 0 0 0 0 0x0 0x0' >"$dbg/dri/0/state"
	echo 'Video Port0: ACTIVE' >"$dbg/dri/0/vop2/summary"
	echo 'SYS:' >"$dbg/dri/0/vop2/regs"
	regmap_file dummy-syscon@0x00000000fd58c000 0xfd58c000 4 0x3fc
	regmap_file dummy-syscon@0x00000000fd5a4000 0xfd5a4000 2 0x1c
	regmap_file dummy-syscon@0x00000000fd5a8000 0xfd5a8000 2 0x5c
	regmap_file dummy-syscon@0x00000000fd5e0000 0xfd5e0000 2 0x84
	regmap_file fed60000.phy 0xfed60000 4 0x1c
	regmap_file fde80000.hdmi 0xfde80000 4 0x1c
}
# t6 <record directory or ''> [arguments]: the tool against the fake board.
# The caller sets T6_HAND, T6_DUMP_LIST and the lock where it wants them.
lock=$tmp/lock
t6() {
	local rec=$1
	shift
	PATH=$tmp/fakebin:$PATH BOARD_HOST=root@board.invalid BOARD_LOCK=$lock MMIO_RD=$tmp/mmio_rd \
		T6_RECORD_DIR=$rec bash "$dump" "$@" 2>&1
}
recs=0
rec_new() { recs=$((recs + 1)); rec=$tmp/rec.$recs; mkdir "$rec"; }
# What the fake has been asked: nothing at all, or no register.
untouched() { [ ! -e "$tree/ssh.log" ] && [ ! -e "$tree/mmio.log" ] && [ ! -e "$tree/stage" ]; }
no_read() { [ ! -e "$tree/mmio.log" ] && [ ! -e "$tree/stage" ]; }
# expected <list lines>: the dump's data lines for them.
expected() {
	local kind block a b c i
	while read -r kind block a b c; do
		case "$kind" in
		mem) for ((i = 0; i < b; i++)); do printf '0x%08x 32 0x%08x\n' $((a + 4 * i)) "$(value $((a + 4 * i)))"; done ;;
		regmap) for ((i = 0; i < c; i++)); do printf '0x%08x 32 0x%08x\n' $((a + b + 4 * i)) "$(value $((a + b + 4 * i)))"; done ;;
		esac
	done
}

# ---- the read list --------------------------------------------------------------
board_new
plan=$(t6 '' --plan)
rc=$?
lines=$(grep -cE '^(mem|regmap) ' <<<"$plan")
words=$(awk '$1 == "mem" { n += $4 } $1 == "regmap" { n += $5 } END { print n + 0 }' <<<"$plan")
check "--plan prints the built-in list, checked ($lines lines, $words words, rc $rc), and contacts no board" \
	"$([ "$rc" = 0 ] && [ "$lines" -gt 0 ] && [ "$(tail -n 1 <<<"$plan")" = "# $lines lines, $words words" ] && untouched; echo $?)"

# Each of these must be refused: the request words and the I2C master of the
# HDMI controller, its offsets that did not answer on the board (0x10,
# 0x804, 0x80c: a bus error) or that Linux's header does not define, and a
# read that runs across one, the
# interrupt clear word, the VOP's LUT RAM and a read that runs into it, the word
# after the IOMMU's COMMAND, the PHY between two of its ranges, a block's
# name on another block's address, an address that is not a word's, the
# controller's regmap file, a regmap line past its file, no words, words
# that are no number, and a line that is no read.
while read -r bad; do
	printf '%s\n' 'mem pmu 0xfd8d810c 7' "$bad" >"$tmp/bad.list"
	out=$(T6_HAND=nobody T6_DUMP_LIST=$tmp/bad.list t6 "$tmp" --go)
	rc=$?
	check "a list with '$bad' is refused before the board is contacted (rc $rc)" \
		"$([ "$rc" = 3 ] && grep -qF "REFUSED: not in the windows of the read list: '$bad'" <<<"$out" && untouched; echo $?)"
done <<'EOF'
mem hdmi0 0xfde80040 1
mem hdmi0 0xfde8003c 2
mem hdmi0 0xfde800dc 2
mem hdmi0 0xfde80118 1
mem hdmi0 0xfde80974 1
mem hdmi0 0xfde80ab0 1
mem hdmi0 0xfde80010 1
mem hdmi0 0xfde80000 8
mem hdmi0 0xfde8004c 1
mem hdmi0 0xfde80044 5
mem hdmi0 0xfde80810 1
mem hdmi0 0xfde80804 1
mem hdmi0 0xfde8080c 1
mem hdmi0 0xfde80800 4
mem hdmi0 0xfde83018 1
mem vop 0xfdd92000 1
mem vop 0xfdd91ffc 2
mem vop_mmu 0xfdd97e0c 1
mem phy0 0xfed602a0 1
mem hdmi0 0xfdd90000 1
mem cru 0xfd7c0001 1
regmap hdmi0 0xfde80000 0x0 1
regmap sys_grf 0xfd58c000 0xffc 2
mem pmu 0xfd8d8000 0
mem pmu 0xfd8d8000 1x
mem pmu 0xfd8d8000 1 2
poke pmu 0xfd8d8000 1
EOF
: >"$tmp/empty.list"
out=$(T6_DUMP_LIST=$tmp/empty.list t6 '' --plan)
rc=$?
check "an empty list is refused (rc $rc)" "$([ "$rc" = 3 ] && grep -qF 'REFUSED: the read list is empty' <<<"$out"; echo $?)"
out=$(t6 '' --force)
rc=$?
check "an unknown argument is a usage error (rc $rc)" "$([ "$rc" = 2 ] && untouched; echo $?)"

# ---- the dry run ---------------------------------------------------------------
out=$(t6 '')
rc=$?
check "the dry run names the connector and the mode, reads no register and copies nothing (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'board: card0-HDMI-A-1 (fde80000.hdmi) connected and enabled; EDID 0 bytes; mode "1024x768": 60 65000' <<<"$out" &&
		grep -qF 'dry run: nothing was copied and no register was read' <<<"$out" && [ -e "$tree/ssh.log" ] && no_read; echo $?)"

# ---- what --go refuses -----------------------------------------------------------
rec_new
out=$(t6 "$rec" --go)
rc=$?
check "--go without T6_HAND is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: --go needs T6_HAND' <<<"$out" && no_read; echo $?)"
out=$(T6_HAND=nobody t6 "$tmp/no-such-directory" --go)
rc=$?
check "--go without a record directory is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'REFUSED: --go needs T6_RECORD_DIR' <<<"$out" && no_read; echo $?)"
# refused <what> <the refusal's words>: --go on the board as it is now.
refused() {
	rec_new
	out=$(T6_HAND=nobody t6 "$rec" --go)
	rc=$?
	check "--go: $1 is refused, no register read, no record (rc $rc)" \
		"$([ "$rc" = 3 ] && grep -qF "$2" <<<"$out" && no_read && [ -z "$(ls "$rec")" ]; echo $?)"
}
board_new
sed -i 's/^vo1 .*/vo1 off-0 0/' "$dbg/pm_genpd/pm_genpd_summary"
refused "the power domain vo1 off" 'the display is not on: power domain vo1 is not on'
board_new
sed -i 's/^vop .*/vop off-0 0/' "$dbg/pm_genpd/pm_genpd_summary"
refused "the power domain vop off" 'the display is not on: power domain vop is not on'
for k in hclk_vop aclk_vop pclk_hdmitx0 pclk_hdptx0; do
	board_new
	echo 0 >"$dbg/clk/$k/clk_enable_count"
	refused "the bus clock $k without a user" "the display is not on: clock $k has no user"
done
board_new
rm "$dbg/clk/pclk_hdptx0/clk_enable_count"
refused "a bus clock whose enable count cannot be read" 'the display is not on: clock pclk_hdptx0 has no user'
board_new
echo disconnected >"$tree/sys/class/drm/card0-HDMI-A-1/status"
refused "a connector that is not connected" 'the display is not on: card0-HDMI-A-1 is not connected'
board_new
echo disabled >"$tree/sys/class/drm/card0-HDMI-A-1/enabled"
refused "a connector that is not enabled" 'the display is not on: card0-HDMI-A-1 is not enabled'
# The picture on the second output only: the list reads the first one's blocks.
board_new
rm -r "$tree/sys/class/drm/card0-HDMI-A-1"
echo connected >"$tree/sys/class/drm/card0-HDMI-A-2/status"
echo enabled >"$tree/sys/class/drm/card0-HDMI-A-2/enabled"
refused "a board with the picture on the second HDMI output" 'the display is not on: no connector of fde80000.hdmi'
board_new
printf 'radxa,rock-5b\0rockchip,rk3588\0' >"$tree/proc/device-tree/compatible"
refused "another RK3588 board" 'is not a NanoPC-T6'
board_new
echo 1000 >"$tree/uid"
refused "a login that is not root" 'the ssh login is not root (uid 1000)'
# The state is read again under the lock: a display that went off after the
# first look (the board's second ssh command) is refused there.
board_new
echo 2 >"$tree/off_at_ssh"
refused "a power domain that goes off before the lock is taken" 'under the lock: the display is not on: power domain vo1 is not on'
# Another runner holds the board: refused at once by board_lock.sh (exit 1).
# The holder takes the lock as a runner does and keeps it until it is told.
board_new
rec_new
mkfifo "$tmp/release"
cat >"$tmp/holder.sh" <<EOF
#!/bin/bash
HOLD_MAX=0 LOCK_WAIT=0
. "$here/board_lock.sh"
: >"$tmp/held"
read -r _ <"$tmp/release"
EOF
BOARD_LOCK=$lock bash "$tmp/holder.sh" &
holder=$!
for ((i = 0; i < 50; i++)); do
	[ -e "$tmp/held" ] && break
	sleep 0.1
done
t0=$(date +%s)
out=$(T6_HAND=nobody t6 "$rec" --go)
rc=$?
check "--go while another runner holds the board lock refuses at once and names it (rc $rc, $(($(date +%s) - t0)) s), no register read" \
	"$([ -e "$tmp/held" ] && [ "$rc" = 1 ] && grep -qF 'board_lock: busy' <<<"$out" && grep -qF 'holder.sh' <<<"$out" &&
		[ $(($(date +%s) - t0)) -le 5 ] && no_read && [ -z "$(ls "$rec")" ]; echo $?)"
echo >"$tmp/release"
wait "$holder"

# ---- a full --go -----------------------------------------------------------------
board_new
rec_new
out=$(T6_HAND=nobody t6 "$rec" --go)
rc=$?
dir=$(echo "$rec"/*-t6-display-dump)
check "--go on the built-in list ends with exit 0 and says how many words it read (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF "done: $words words in $dir/dump.txt" "$dir/record.txt"; echo $?)"
grep -E '^mem ' <<<"$plan" | cut -d' ' -f3,4 >"$tmp/want.log"
check "... mmio_rd was called once for each mem line of the list, in its order" \
	"$(cmp -s "$tmp/want.log" "$tree/mmio.log"; echo $?)"
expected <<<"$plan" >"$tmp/want.txt"
grep -v '^#' "$dir/dump.txt" >"$tmp/got.txt"
check "... dump.txt holds every word of the list at its address, GRF words at base + offset ($(wc -l <"$tmp/got.txt") lines)" \
	"$([ "$(wc -l <"$tmp/want.txt")" = "$words" ] && cmp -s "$tmp/want.txt" "$tmp/got.txt"; echo $?)"
check "... its header names the tool, the kernel, the connector and the mode, and every other line is a comment; the build is in record.txt only" \
	"$(grep -q '^# driver/tools/kexec-zephyr/t6-display-dump.sh on$' "$dir/dump.txt" &&
		grep -q ' build ' "$dir/record.txt" &&
		grep -qF '# Connector card0-HDMI-A-1 (fde80000.hdmi) connected and enabled; EDID 0 bytes.' "$dir/dump.txt" &&
		grep -qF '# Mode (drm_mode line of the active CRTC): "1024x768": 60 65000' "$dir/dump.txt" &&
		! grep -vE '^(#|0x[0-9a-f]{8} 32 0x[0-9a-f]{8}$)' "$dir/dump.txt" | grep -q .; echo $?)"
check "... files.txt holds Linux's own view; the HDMI controller's regmap file is not in it" \
	"$(grep -qF 'Video Port0: ACTIVE' "$dir/files.txt" && grep -qF 'regmap/fed60000.phy/registers' "$dir/files.txt" &&
		! grep -q 'fde80000.hdmi/registers' "$dir/files.txt" && ! grep -q 'files.txt is not complete' "$dir/record.txt"; echo $?)"
check "... what it copied to the board is removed, and the lock's record is cleared" \
	"$([ ! -e "$tree/stage" ] && [ ! -e "$dir/dump.body" ] && [ ! -s "$lock" ]; echo $?)"

# ---- a run that must stop ----------------------------------------------------------
# A short list: two PMU lines, one GRF line, two VOP lines.
printf '%s\n' '# a part' 'mem pmu 0xfd8d810c 7' 'mem pmu 0xfd8d814c 2' 'regmap vo1_grf 0xfd5a8000 0x008 2' \
	'mem vop 0xfdd90000 4' 'mem vop 0xfdd90600 4' >"$tmp/part.list"
# stopped <what> <words kept> <mmio_rd calls> <the record's words>
stopped() {
	rec_new
	out=$(T6_HAND=nobody T6_DUMP_LIST=$tmp/part.list t6 "$rec" --go)
	rc=$?
	dir=$(echo "$rec"/*-t6-display-dump)
	check "--go: $1 stops the run there: exit 4, $2 words kept, $3 mmio_rd calls, no dump.txt, nothing left on the board (rc $rc)" \
		"$([ "$rc" = 4 ] && grep -qF "$4" "$dir/record.txt" && [ "$(wc -l <"$dir/dump.body")" = "$2" ] &&
			[ "$(wc -l <"$tree/mmio.log")" = "$3" ] && [ ! -e "$dir/dump.txt" ] && [ ! -e "$tree/stage" ]; echo $?)"
}
board_new
rec_new
out=$(T6_HAND=nobody T6_DUMP_LIST=$tmp/part.list t6 "$rec" --go)
rc=$?
dir=$(echo "$rec"/*-t6-display-dump)
grep -v '^#' "$tmp/part.list" | expected >"$tmp/want.txt"
grep -v '^#' "$dir/dump.txt" >"$tmp/got.txt"
check "--go with T6_DUMP_LIST reads that list only and says so in the header (rc $rc)" \
	"$([ "$rc" = 0 ] && cmp -s "$tmp/want.txt" "$tmp/got.txt" && grep -qF '# List: another list, 5 lines, 19 words.' "$dir/dump.txt"; echo $?)"
board_new
echo 0xfd8d814c >"$tree/off_after"
stopped "a power domain that goes off after the second read" 9 2 "STOPPED at 'regmap vo1_grf 0xfd5a8000 0x008 2': the command failed (display off: power domain vo1 is not on)"
board_new
echo 0xfdd90008 >"$tree/fail_at"
stopped "a read that fails" 11 3 "STOPPED at 'mem vop 0xfdd90000 4': the command failed"
board_new
echo 0xfd8d8150 >"$tree/short_at"
stopped "a read that returns one word of two" 8 2 "STOPPED at 'mem pmu 0xfd8d814c 2': 1 of 2 words came back"
board_new
sed -i '4,$d' "$dbg/regmap/dummy-syscon@0x00000000fd5a8000/registers"
stopped "a regmap file that ends before the word" 10 2 "STOPPED at 'regmap vo1_grf 0xfd5a8000 0x008 2': 1 of 2 words came back"
board_new
rm -r "$dbg/regmap/dummy-syscon@0x00000000fd5a8000"
stopped "a GRF without a regmap file" 9 2 "STOPPED at 'regmap vo1_grf 0xfd5a8000 0x008 2': the command failed (no regmap file of 0xfd5a8000)"

not_run=
while IFS=: read -r line _; do
	[ -n "${ran[$line]:-}" ] || not_run+=" $line"
done < <(grep -nE '^([^#]*[[:space:];&|(])?check "' "$0")
echo "t6-display-dump-selftest: $run checks run, $failed failed${not_run:+, check lines not run:$not_run}"
[ "$failed" = 0 ] && [ "$run" -gt 0 ] && [ -z "$not_run" ]
