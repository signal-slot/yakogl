#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# selftest.sh - stage.sh and prompt-go.sh's dry run without a board.
#
#   bash driver/tools/uboot-t6/selftest.sh
#
# An `ssh` first in PATH plays the board: it runs the command it is given
# with bash on this host, with /boot, /proc/device-tree and
# /sys/kernel/kexec_loaded moved under a temporary directory, `id -u`
# answering 0, and it logs every command. It refuses a command that would
# reboot. The checks:
#
# - stage.sh stages, checks and undoes an image on a made-up /boot; the stock
#   files keep their bytes throughout; the fingerprint names a stock file
#   changed behind its back; it refuses a name U-Boot's prompt would not
#   take; and every command it sent is in the short list of what it may
#   write (install, cat into the .part file, chmod, mv, sync, rm -rf of
#   /boot/yakogl), with no block device, dd, module or reboot in any.
# - prompt-go.sh's dry run on a made-up image: refuses an image whose entry
#   lacks T6:G, refuses one that is not staged, and on a staged one prints
#   the commands and exits 0 without a reboot; --go refuses without T6_HAND
#   and with a T6_UART that is no tty.
# - prompt-reads.sh (prompt-lib.sh is shared): its dry run and the same two
#   refusals, with its own self-test answered by a fake python3.
#
# The last line: "uboot-t6 selftest: N checks run, F failed", with ", check
# lines not run: ..." when a check line of this file did not run.
set -u
here=$(cd "$(dirname "$0")" && pwd)
run=0
failed=0
# The lines of this file whose check has run. Every line that calls check
# must be in it at the end: a check that a condition skipped fails the
# test, and no count is kept by hand.
declare -A ran=()
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
root=$tmp/board
mkdir -p "$root/boot/dtb/rockchip" "$root/dt" "$tmp/fakebin"
printf 'friendlyarm,nanopc-t6\0rockchip,rk3588\0' >"$root/dt/compatible"
echo 0 >"$root/kexec_loaded"
printf 'kernel' >"$root/boot/Image"
printf 'dtb' >"$root/boot/dtb/rockchip/rk3588-nanopc-t6.dtb"
printf 'script' >"$root/boot/boot.scr"
ln -s boot.scr "$root/boot/boot.link"     # Armbian's /boot has links (Image, uInitrd, dtb)
stock() { (cd "$root/boot" && find . -path ./yakogl -prune -o -type f -print0 | sort -z | xargs -0 sha256sum); }
stock_before=$(stock)

cat >"$tmp/fakebin/ssh" <<EOF
#!/bin/bash
cmd=\${!#}
printf '%s\n---\n' "\$cmd" >>"\${SSH_LOG:-$tmp/ssh.log}"
case "\$cmd" in *reboot*) exit 1 ;; esac
# FAKE_DURING_COPY: a stock file changes while stage.sh copies the image.
case "\$cmd" in *"cat > /boot/yakogl/"*) [ -z "\${FAKE_DURING_COPY:-}" ] || printf x >>"$root/boot/boot.scr" ;; esac
cmd=\${cmd//\/proc\/device-tree/$root/dt}
cmd=\${cmd//\/sys\/kernel\/kexec_loaded/$root/kexec_loaded}
cmd=\${cmd//\/boot/$root/boot}
exec bash -c "id() { echo 0; }; \$cmd"
EOF
chmod +x "$tmp/fakebin/ssh"
fake() { PATH=$tmp/fakebin:$PATH BOARD_HOST=root@t6.invalid "$@" 2>&1; }
# prompt-go.sh's commands go to a log of their own.
fake_go() { SSH_LOG=$tmp/go.log fake bash "$here/prompt-go.sh" "$@"; }

# A made-up image: the arm64 Image magic, the watchdog marker, the lines
# prompt-go.sh looks for, as the real ones are in the image.
mkimg() { # <file> [extra line]
	{
		head -c 56 /dev/zero
		printf 'ARM\x64'
		head -c 4 /dev/zero
		printf 'yakogl-wdt: nanopc_t6 WDT0 armed at boot; top 13; lifetime_s 60; alive_s 5; proof 0; load 0x10000000\0'
		printf 'board: watchdog armed\0'
		[ -z "${2:-}" ] || printf '%s\0' "$2"
		head -c 4096 /dev/zero
	} >"$1"
}
mkimg "$tmp/t6_hello.bin" 'T6:G go: SCTLR_EL2 '
fp=$tmp/boot.fingerprint

# ---- stage.sh -----------------------------------------------------------------
out=$(fake bash "$here/stage.sh" stage "$tmp/t6_hello.bin" "$fp")
rc=$?
check "stage: exit 0, the copy is the image (rc $rc)" \
	"$([ "$rc" = 0 ] && cmp -s "$tmp/t6_hello.bin" "$root/boot/yakogl/t6_hello.bin"; echo $?)"
check "stage: the fingerprint was written before the copy: the 3 stock files and the stock link" \
	"$([ "$(wc -l <"$fp")" = 4 ] && grep -q 'boot.link -> boot.scr$' "$fp" && ! grep -q yakogl "$fp"; echo $?)"
check "stage: no .part file is left, the mode is 0644" \
	"$([ ! -e "$root/boot/yakogl/.t6_hello.bin.part" ] && [ "$(stat -c %a "$root/boot/yakogl/t6_hello.bin")" = 644 ]; echo $?)"
check "stage: the stock files keep their bytes" "$([ "$(stock)" = "$stock_before" ]; echo $?)"
out=$(fake bash "$here/stage.sh" check "$tmp/t6_hello.bin" "$fp")
rc=$?
check "check: exit 0 (rc $rc)" "$([ "$rc" = 0 ] && grep -q 'still match' <<<"$out"; echo $?)"
out=$(fake bash "$here/stage.sh" stage "$tmp/t6_hello.bin" "$fp")
rc=$?
check "stage again over the same fingerprint: exit 0 (rc $rc)" "$([ "$rc" = 0 ]; echo $?)"

# prompt-go.sh's dry run, while the image is staged.
out=$(fake_go "$tmp/t6_hello.bin" t6_hello)
rc=$?
check "prompt-go dry run: exit 0 and the plan (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'ext4load mmc 0:1 0x10000000 /boot/yakogl/t6_hello.bin' <<<"$out" &&
		grep -qF 'go 0x10000000' <<<"$out" && grep -qF 'dry run: nothing was written' <<<"$out"; echo $?)"
mkdir "$tmp/nog"
mkimg "$tmp/nog/t6_hello.bin"
out=$(fake_go "$tmp/nog/t6_hello.bin" t6_hello)
rc=$?
check "prompt-go: an image without T6:G is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'has no T6:G line' <<<"$out"; echo $?)"
mkimg "$tmp/t6_other.bin" 'T6:G go: SCTLR_EL2 '
out=$(fake_go "$tmp/t6_other.bin" t6_hello)
rc=$?
check "prompt-go: an image that is not staged is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'stage it first' <<<"$out"; echo $?)"
out=$(fake_go "$tmp/t6_hello.bin" t6_nonesuch)
rc=$?
check "prompt-go: an unknown probe is usage (rc $rc)" "$([ "$rc" = 2 ]; echo $?)"
# --go's own refusals before the board lock (prompt-lib.sh's uart_ready among
# them), and never a reboot
out=$(fake_go "$tmp/t6_hello.bin" t6_hello --go)
rc=$?
check "prompt-go --go: refused without T6_HAND (rc $rc)" "$([ "$rc" = 3 ] && grep -qF 'needs T6_HAND' <<<"$out"; echo $?)"
: >"$tmp/not-a-tty"
out=$(T6_HAND=test T6_RECORD_DIR=$tmp T6_UART=$tmp/not-a-tty fake_go "$tmp/t6_hello.bin" t6_hello --go)
rc=$?
check "prompt-go --go: a T6_UART that is no character device is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'is not a character device' <<<"$out"; echo $?)"

# prompt-reads.sh, which shares prompt-lib.sh: its dry run and its refusals.
# A python3 first in PATH answers the reads' own self-test (a minute; it
# runs in board_lock_selftest.sh) and passes everything else on.
real_py=$(command -v python3)
cat >"$tmp/fakebin/python3" <<PY
#!/bin/bash
for a in "\$@"; do [ "\$a" = --self-test ] && { echo "prompt-reads self-test: 1 checks run, 0 failed"; exit 0; }; done
exec "$real_py" "\$@"
PY
chmod +x "$tmp/fakebin/python3"
fake_reads() { SSH_LOG=$tmp/go.log fake bash "$here/prompt-reads.sh" "$@"; }
out=$(fake_reads)
rc=$?
check "prompt-reads dry run: exit 0, the plan, the state read (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'md.l 0xfd7c083c 1' <<<"$out" && grep -qF 'board: NanoPC-T6, up' <<<"$out" &&
		grep -qF 'dry run: nothing was written' <<<"$out"; echo $?)"
out=$(fake_reads --go)
rc=$?
check "prompt-reads --go: refused without T6_HAND (rc $rc)" "$([ "$rc" = 3 ] && grep -qF 'needs T6_HAND' <<<"$out"; echo $?)"
out=$(T6_HAND=test T6_RECORD_DIR=$tmp T6_UART=$tmp/not-a-tty fake_reads --go)
rc=$?
check "prompt-reads --go: a T6_UART that is no character device is refused (rc $rc)" \
	"$([ "$rc" = 3 ] && grep -qF 'is not a character device' <<<"$out"; echo $?)"
rm "$tmp/fakebin/python3"

# A stock file changed behind the fingerprint's back: check fails, naming it.
cp "$root/boot/boot.scr" "$tmp/boot.scr.keep"
printf 'changed' >"$root/boot/boot.scr"
out=$(fake bash "$here/stage.sh" check "$tmp/t6_hello.bin" "$fp")
rc=$?
check "check: a stock file that changed is named (rc $rc)" \
	"$([ "$rc" = 1 ] && grep -q 'boot.scr' <<<"$out"; echo $?)"
out=$(fake bash "$here/stage.sh" stage "$tmp/t6_hello.bin" "$fp")
rc=$?
check "stage: refused while a stock file differs (rc $rc)" "$([ "$rc" = 3 ]; echo $?)"
cp "$tmp/boot.scr.keep" "$root/boot/boot.scr"

# A stock file that changes during the copy: the fingerprint read after the
# change names it (stage exits 1, not 0).
out=$(FAKE_DURING_COPY=1 fake bash "$here/stage.sh" stage "$tmp/t6_hello.bin" "$fp")
rc=$?
check "stage: a stock file changed during the copy is named afterwards (rc $rc)" \
	"$([ "$rc" = 1 ] && grep -q "boot.scr" <<<"$out" && grep -q 'differ from .* after stage' <<<"$out"; echo $?)"
cp "$tmp/boot.scr.keep" "$root/boot/boot.scr"

# A stock symbolic link pointed elsewhere is named too.
ln -sfn Image "$root/boot/boot.link"
out=$(fake bash "$here/stage.sh" check "$tmp/t6_hello.bin" "$fp")
rc=$?
check "check: a stock link with another target is named (rc $rc)" \
	"$([ "$rc" = 1 ] && grep -q 'boot.link -> Image' <<<"$out"; echo $?)"
ln -sfn boot.scr "$root/boot/boot.link"

# The undo.
out=$(fake bash "$here/stage.sh" undo "$fp")
rc=$?
check "undo: exit 0, /boot/yakogl gone, 'stock /boot restored' (rc $rc)" \
	"$([ "$rc" = 0 ] && [ ! -e "$root/boot/yakogl" ] && grep -q 'stock /boot restored' <<<"$out"; echo $?)"
check "undo: the stock files keep their bytes" "$([ "$(stock)" = "$stock_before" ]; echo $?)"

# undo after a stock file changed (a kernel update): /boot/yakogl is removed
# all the same, the change is named, exit 1.
out=$(fake bash "$here/stage.sh" stage "$tmp/t6_hello.bin" "$fp")
printf 'new kernel' >"$root/boot/Image"
out=$(fake bash "$here/stage.sh" undo "$fp")
rc=$?
check "undo after a stock change: /boot/yakogl gone, the change named, exit 1 (rc $rc)" \
	"$([ "$rc" = 1 ] && [ ! -e "$root/boot/yakogl" ] && grep -q 'Image' <<<"$out" && grep -q 'removed all the same' <<<"$out"; echo $?)"
printf 'kernel' >"$root/boot/Image"

# /boot/yakogl as a symbolic link, or holding one: stage and check refuse;
# undo removes the link, not its target.
mkdir "$tmp/outside"
ln -s "$tmp/outside" "$root/boot/yakogl"
out=$(fake bash "$here/stage.sh" stage "$tmp/t6_hello.bin" "$fp")
rc=$?
check "stage: /boot/yakogl as a symbolic link is refused, nothing written through it (rc $rc)" \
	"$([ "$rc" = 3 ] && [ -z "$(ls -A "$tmp/outside")" ]; echo $?)"
out=$(fake bash "$here/stage.sh" undo "$fp")
rc=$?
check "undo: the link is removed, its target kept (rc $rc)" \
	"$([ "$rc" = 0 ] && [ ! -e "$root/boot/yakogl" ] && [ -d "$tmp/outside" ]; echo $?)"
mkdir "$root/boot/yakogl"
ln -s "$tmp/outside/x" "$root/boot/yakogl/t6_hello.bin"
out=$(fake bash "$here/stage.sh" check "$tmp/t6_hello.bin" "$fp")
rc=$?
check "check: a link inside /boot/yakogl is refused (rc $rc)" "$([ "$rc" = 3 ]; echo $?)"
rm -r "$root/boot/yakogl"

out=$(fake bash "$here/stage.sh" undo "$tmp/none.fingerprint")
rc=$?
check "undo without a fingerprint file is usage (rc $rc)" "$([ "$rc" = 2 ] && [ ! -e "$tmp/none.fingerprint" ]; echo $?)"

# A name the prompt would not take: refused before ssh.
cp "$tmp/t6_hello.bin" "$tmp/bad name.bin"
: >"$tmp/ssh.log.before"; cp "$tmp/ssh.log" "$tmp/ssh.log.before"
out=$(fake bash "$here/stage.sh" stage "$tmp/bad name.bin" "$fp")
rc=$?
check "stage: a name with a space is refused before ssh (rc $rc)" \
	"$([ "$rc" = 3 ] && cmp -s "$tmp/ssh.log" "$tmp/ssh.log.before"; echo $?)"

# What stage.sh sent: the writes are only these, and nothing touches a device.
writes=$(sed '/^---$/d' "$tmp/ssh.log" | grep -E 'install |cat >|chmod |mv |rm |dd |of=|mkfs|modprobe|insmod|/dev/(mmc|sd|nvme|disk|mtd)|reboot|saveenv|fw_setenv|tee ' || true)
allowed='^(set -e; install -d -m 0755 /boot/yakogl; cat > /boot/yakogl/\.t6_hello\.bin\.part; chmod 0644 /boot/yakogl/\.t6_hello\.bin\.part; mv -f /boot/yakogl/\.t6_hello\.bin\.part /boot/yakogl/t6_hello\.bin; sync|rm -rf -- /boot/yakogl && sync)$'
bad=$(grep -vE "$allowed" <<<"$writes" || true)
check "every write sent is in the list, no device, dd, module or reboot (other: '${bad:0:200}')" "$([ -z "$bad" ]; echo $?)"
check "the writes were sent: the stage and the undo" \
	"$(grep -q '^rm -rf -- /boot/yakogl && sync$' <<<"$writes" && grep -q 'install -d -m 0755 /boot/yakogl' <<<"$writes"; echo $?)"
check "no command of stage.sh or of prompt-go.sh's dry runs asks for a reboot" \
	"$(! cat "$tmp/ssh.log" "$tmp/go.log" | grep -q 'reboot'; echo $?)"

not_run=
while IFS=: read -r line _; do
	[ -n "${ran[$line]:-}" ] || not_run+=" $line"
# a check call starts its line or follows ; & | ( with no quote before it
# ("stage.sh" check "..." is an argument, not a call)
done < <(grep -nE '^([^#"]*[;&|(][[:space:]]*|[[:space:]]*)check "' "$0")
echo "uboot-t6 selftest: $run checks run, $failed failed${not_run:+, check lines not run:$not_run}"
[ "$failed" = 0 ] && [ "$run" -gt 0 ] && [ -z "$not_run" ]
