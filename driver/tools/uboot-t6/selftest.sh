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
# - both with T6_UART=rfc2217://<host>:<port>: the dry runs say so without
#   connecting, --go goes through t6-uart-rfc2217.py, and no output names
#   the address.
# - prompt-lib.sh's exit_when_back, the runners' last step, with a fake
#   board(): a tool code that says "no Linux login on the UART" becomes the
#   code of the login seen when ssh finds a new boot_id, and stays when it
#   finds none (issue #11874); other codes are as before; the two runners
#   call it with their codes.
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
# The fake board's root has "yakogl" in its path on purpose: the gate's
# TMPDIR in CI has it too (~/.cache/yakogl/tmp), and a check that looked for
# the staged directory by that word alone failed there and nowhere else.
root=$tmp/yakogl/board
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
	"$([ "$(wc -l <"$fp")" = 4 ] && grep -q 'boot.link -> boot.scr$' "$fp" && ! grep -qF "$root/boot/yakogl" "$fp"; echo $?)"
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
# the probes with a picture are taken too; t6_display is not (it adopts
# what only Linux leaves)
for p in t6_cold triangle; do
	out=$(fake_go "$tmp/t6_hello.bin" "$p")
	rc=$?
	check "prompt-go dry run: probe $p is taken (rc $rc)" \
		"$([ "$rc" = 0 ] && grep -qF "its verdict ($p, T6_ROUTE=uboot)" <<<"$out"; echo $?)"
done
out=$(fake_go "$tmp/t6_hello.bin" t6_display)
rc=$?
check "prompt-go: t6_display is not a probe of the go route (rc $rc)" "$([ "$rc" = 2 ]; echo $?)"
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

# T6_UART=rfc2217://<host>:<port> (issue #11611). A listener on 127.0.0.1
# stands for the adapter: it logs each connection and closes it at once. The
# dry runs say T6_UART is an RFC 2217 address and do not connect; --go
# goes through ../kexec-zephyr/t6-uart-rfc2217.py (one connection, its
# refusal: exit 40, before the script's own checks). No output names the
# address.
cat >"$tmp/listen.py" <<'PY'
import os, socket, sys
s = socket.socket()
s.bind(("127.0.0.1", 0))
s.listen(4)
with open(sys.argv[1] + ".new", "w") as o:
    o.write("%d\n" % s.getsockname()[1])
os.rename(sys.argv[1] + ".new", sys.argv[1])
while True:
    c, _ = s.accept()
    with open(sys.argv[2], "a") as o:
        o.write("accepted\n")
    c.close()
PY
python3 -I "$tmp/listen.py" "$tmp/listen.port" "$tmp/listen.log" &
listen_pid=$!
trap 'kill "$listen_pid" 2>/dev/null; rm -rf "$tmp"' EXIT
for _ in $(seq 50); do [ -s "$tmp/listen.port" ] && break; sleep 0.1; done
lport=$(cat "$tmp/listen.port" 2>/dev/null)
lurl=rfc2217://127.0.0.1:${lport:-0}
: >"$tmp/listen.log"
no_address() { ! grep -qF "127.0.0.1" <<<"$1" && ! grep -qF ":$lport" <<<"$1"; }
out=$(T6_UART=$lurl fake_reads)
rc=$?
check "prompt-reads dry run with an RFC 2217 address: exit 0, says so, no address, no connection (rc $rc)" \
	"$([ "$rc" = 0 ] && [ -n "$lport" ] && grep -qF 'T6_UART is an RFC 2217 address (not opened in a dry run)' <<<"$out" &&
		no_address "$out" && [ ! -s "$tmp/listen.log" ]; echo $?)"
out=$(T6_UART=$lurl fake_go "$tmp/t6_hello.bin" t6_hello)
rc=$?
check "prompt-go dry run with an RFC 2217 address: exit 0, says so, no address, no connection (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'T6_UART is an RFC 2217 address (not opened in a dry run)' <<<"$out" &&
		no_address "$out" && [ ! -s "$tmp/listen.log" ]; echo $?)"
for r in reads go; do
	: >"$tmp/listen.log"
	if [ "$r" = reads ]; then
		out=$(T6_HAND=test T6_RECORD_DIR=$tmp T6_UART=$lurl fake_reads --go)
	else
		out=$(T6_HAND=test T6_RECORD_DIR=$tmp T6_UART=$lurl fake_go "$tmp/t6_hello.bin" t6_hello --go)
	fi
	rc=$?
	check "prompt-$r --go with an RFC 2217 address: through t6-uart-rfc2217.py, one connection, its refusal 40, no address (rc $rc)" \
		"$([ "$rc" = 40 ] && [ "$(grep -c accepted "$tmp/listen.log")" = 1 ] &&
			grep -qF 't6-uart-rfc2217: REFUSED: the server closed the connection' <<<"$out" && no_address "$out"; echo $?)"
done
rm "$tmp/fakebin/python3"

# prompt-lib.sh's exit_when_back (issue #11874). The UART's stream can stop
# in the kernel's output; ssh is the other witness of Linux. A subshell
# sources prompt-lib.sh and stands in for the runner around it: rec() to a
# file, board() answering a new boot_id ("new") or nothing ("none", looked
# for 0 s).
back() { # <tool's code> <new|none> [<code>:<code when back> ...]
	local code=$1 state=$2
	shift 2
	rm -rf "$tmp/back"
	mkdir "$tmp/back"
	(
		# shellcheck disable=SC2034  # read by prompt-lib.sh
		REC=$tmp/back TOOL=prompt-go T6_HAND=test old_boot=old-boot t0=$(date +%s) py_rc=$code
		BACK_WAIT_S=0
		[ "$state" = none ] || BACK_WAIT_S=30
		rec() { printf '%s\n' "$*" >>"$REC/record.txt"; }
		# shellcheck source=prompt-lib.sh
		. "$here/prompt-lib.sh"
		board() {
			[ "$state" = new ] || return 1
			case "$1" in *boot_id*) echo new-boot ;; *) echo 12.5 ;; esac
		}
		exit_when_back "$@"
	) >/dev/null 2>&1
}
saw='saw no Linux login on the UART (exit'
back 10 new 6:8 10:0
rc=$?
check "exit_when_back: no login on the UART after the image, a new boot_id over ssh, verdict PASS: exit 0, said in the record (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF "$saw 10), but Armbian answers over ssh with a new boot_id: Linux is back; exit 0" "$tmp/back/record.txt"; echo $?)"
back 10 new 6:8 10:11
rc=$?
check "exit_when_back: the same with the verdict FAIL: exit 11 (rc $rc)" "$([ "$rc" = 11 ]; echo $?)"
back 10 none 6:8 10:0
rc=$?
check "exit_when_back: no login on the UART and no Armbian over ssh: exit 10, no sign of Linux, a hand needed (rc $rc)" \
	"$([ "$rc" = 10 ] && grep -qF 'no Linux login on the UART (exit 10) and no Armbian over ssh' "$tmp/back/record.txt" &&
		grep -qF 'needs test at its power switch' "$tmp/back/record.txt" && ! grep -qF "$saw" "$tmp/back/record.txt"; echo $?)"
back 0 new 6:8 10:0
rc=$?
check "exit_when_back: the login seen on the UART, Armbian back: exit 0 as before, no line about the UART (rc $rc)" \
	"$([ "$rc" = 0 ] && grep -qF 'back in Armbian' "$tmp/back/record.txt" && ! grep -qF "$saw" "$tmp/back/record.txt"; echo $?)"
back 6 new 6:8 10:0
rc=$?
check "exit_when_back: prompt-go's 6 (refused, reset, no login on the UART) with Armbian back: exit 8 (rc $rc)" "$([ "$rc" = 8 ]; echo $?)"
back 6 new 6:0
rc=$?
check "exit_when_back: prompt-reads' 6 with Armbian back: exit 0 (rc $rc)" "$([ "$rc" = 0 ]; echo $?)"
back 9 none 6:8 10:0
rc=$?
check "exit_when_back: a code not in the list, no Armbian: exit 1 as before (rc $rc)" "$([ "$rc" = 1 ]; echo $?)"
back 5 none 6:8 10:0
rc=$?
check "exit_when_back: stopped at the prompt, no Armbian: exit 5 as before (rc $rc)" "$([ "$rc" = 5 ]; echo $?)"
# The runners hand it their codes: prompt-go.sh 6:8 and 10:<0 when the
# verdict's last line is PASS, else 11>, prompt-reads.sh 6:0.
check "prompt-go.sh and prompt-reads.sh call exit_when_back with their no-login codes" \
	"$(grep -qx 'exit_when_back 6:8 10:$seen' "$here/prompt-go.sh" &&
		grep -qxF '[ "$(tail -n 1 "$REC/verdict.txt" 2>/dev/null)" != "VERDICT $PROBE PASS" ] || seen=0' "$here/prompt-go.sh" &&
		grep -qx 'exit_when_back 6:0' "$here/prompt-reads.sh"; echo $?)"

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
