#!/usr/bin/env bash
# Selftest of tools/mp2s_undo_order.py: crafted transcripts, no board.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/mp2s_undo_order.py
d=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_undo.XXXXXX") || exit 2
trap 'rm -rf "$d"' EXIT
fail=0

# expect NAME RC [LINE]: the transcript comes on stdin
expect() {
	local name=$1 rc=$2 line=${3:-} f=$d/$1.txt got
	cat >"$f"
	python3 -I "$tool" "$f" 2>"$d/err"
	got=$?
	if [ "$got" != "$rc" ]; then
		echo "FAIL $name: rc $got, want $rc"; cat "$d/err"; fail=1; return
	fi
	if [ -n "$line" ] && ! grep -q "line $line:" "$d/err"; then
		echo "FAIL $name: want line $line"; cat "$d/err"; fail=1; return
	fi
	echo "ok   $name"
}

expect good 0 <<'T'
UNDO banner is ignored
undo: rm -f /boot/yakogl/standalone/default.bin
change: cp image.bin /boot/yakogl/standalone/default.bin
undo: fw_setenv mp2s_left
change: fw_setenv mp2s_left 2
undo: fw_setenv boot_prefixes /extlinux/
change: fw_setenv boot_prefixes '/yakogl/standalone/ /extlinux/'
undo: cp /var/saved/x /boot/x
change: rm /boot/x
T
expect missing_undo 1 2 <<'T'
note
change: cp a /boot/yakogl/standalone/default.bin
T
expect undo_after_change 1 1 <<'T'
change: fw_setenv mp2s_left 2
undo: fw_setenv mp2s_left
T
expect mismatch_env 1 2 <<'T'
undo: fw_setenv mp2s_left 2
change: fw_setenv mp2s_left 2
T
expect mismatch_env_delete 1 2 <<'T'
undo: fw_setenv mp2s_left
change: fw_setenv mp2s_left
T
expect mismatch_rm 1 2 <<'T'
undo: rm /boot/f
change: rm /boot/f
T
expect mismatch_path 1 2 <<'T'
undo: rm /boot/g
change: cp a /boot/f
T
expect restore_saved 0 <<'T'
undo: mv /saved/f /boot/f
change: cp a /boot/f
T
expect bootcmd_write 1 1 <<'T'
undo: fw_setenv bootcmd
change: fw_setenv bootcmd 'run foo'
T
expect bootcount_write 1 1 <<'T'
change: fw_setenv bootcount 0
T
expect altbootcmd_undo 1 1 <<'T'
undo: fw_setenv altbootcmd
T
expect unknown_cmd 1 1 <<'T'
change: dd if=a of=/dev/mmcblk0
T
expect empty 0 </dev/null

python3 -I "$tool" a b c 2>/dev/null
[ $? = 2 ] && echo "ok   usage" || { echo "FAIL usage"; fail=1; }
python3 -I "$tool" "$d/nonexistent" 2>/dev/null
[ $? = 2 ] && echo "ok   unreadable" || { echo "FAIL unreadable"; fail=1; }
printf 'undo: rm /b/x\nchange: cp a /b/x\n' | python3 -I "$tool"
[ $? = 0 ] && echo "ok   stdin" || { echo "FAIL stdin"; fail=1; }
exit $fail
