#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_guard_selftest.sh - tools/mp2s_guard.py without a board (plan #362,
# item oneshot-guard; docs/notes/stm32mp257f-dk-standalone.md section 7
# :544-547: the one-shot runners refuse while mp2s_left is set, and the
# standalone default refuses while yako_*/mp2z_* are set).
#
#   tools/mp2s_guard_selftest.sh
#
# Fixtures in $TMPDIR, both input forms:
#   - fw_printenv texts (pe FILE NAME=VALUE...): a clean ST environment,
#     mp2s_left=2, mp2s_left=forever, a stray mp2s_prefixes (no mp2s_left),
#     yako_once, mp2z_token, a doubled /yakogl/standalone/ prefix with no
#     mp2s_* left, yako_once plus the doubled prefix, and a garbage file;
#   - environment images (a tiny stdlib builder: CRC-32 LE, a flags byte per
#     copy, NUL-terminated strings, zero padding; one built with
#     tools/uboot_env.py's own set writer; one with the DK's real layout,
#     copies at 0x7e000/0x7c000 in a 512 KiB partition).
# Verdicts (30 checks):
#   - allow (exit 0, exactly one "mp2s-guard: allow:" line on stdout, empty
#     stderr): clean, both modes and both forms; a stray mp2s_* is not a
#     one-shot; yako_*/mp2z_* and a doubled prefix are not a standalone
#     default; mp2s_* are not one-shot state; the mp2s_left kept only in the
#     stale redundant copy (flags 0, the newer copy is clean) is not live;
#   - REFUSE (exit 3, exactly one line on stderr equal to the pinned message,
#     empty stdout): the canonical oneshot refusals for mp2s_left=2 (file and
#     stdin form) and mp2s_left=forever, the stray-variables refusal, the
#     standalone refusals for yako_once, mp2z_token, the doubled prefix, both
#     together, and the mp2z_load/mp2z_window left in the active copy of a
#     DK-layout image;
#   - error (exit 2, empty stdout, no REFUSE line, stderr naming it): a flipped
#     data byte (bad CRC, single copy), a bad CRC on the copy that is not
#     active, a missing --printenv file, a missing --env-image file, garbage
#     and an empty stdin ("unreadable input"), a missing --for, both input
#     files at once, a layout flag used with --printenv.
#
# A tools/check.sh row for this selftest waits for plan #341, so it is run
# by hand: tools/mp2s_guard_selftest.sh
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
T=$(mktemp -d "${TMPDIR:-$here}/mp2s-guard-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
pass=0
fail=0
WANT_CHECKS=30

# pe FILE NAME=VALUE...: a fw_printenv text. A NAME written twice keeps the
# last value (U-Boot's himport_r), as the guard reads it.
pe() {
	local f=$1
	shift
	: >"$f"
	local v
	for v in "$@"; do printf '%s\n' "$v" >>"$f"; done
	echo "$f"
}

# envbin FILE SIZE r|s LINE...: an environment image, one LINE per copy,
# FLAGS|NAME=VALUE|...  (values hold no '|'). CRC-32 LE, a flags byte with
# r, strings then zero padding to the copy size. Prints FILE. (The lines are
# argv, not stdin: stdin is the Python program itself.)
envbin() {
	python3 - "$1" "$2" "$3" "${@:4}" <<'PY'
import sys, zlib
path, size, mode = sys.argv[1], int(sys.argv[2], 0), sys.argv[3]
red = mode == "r"
hdr = 5 if red else 4
out = b""
for line in sys.argv[4:]:
    flag, *vars_ = line.split("|")
    data = b"".join(v.encode() + b"\0" for v in vars_) + b"\0"
    if len(data) + hdr > size:
        sys.exit("envbin: the strings need %d bytes, the copy size is %d" % (len(data) + hdr, size))
    area = data + b"\0" * (size - hdr - len(data))
    out += zlib.crc32(area).to_bytes(4, "little") + (bytes([int(flag, 0)]) if red else b"") + area
open(path, "wb").write(out)
PY
	echo "$1"
}

# corrupt FILE BYTEPOS: flip one byte.
corrupt() {
	python3 - "$@" <<'PY'
import sys
path, off = sys.argv[1], int(sys.argv[2], 0)
b = bytearray(open(path, "rb").read())
b[off] ^= 0xFF
open(path, "wb").write(b)
PY
}

# dklayout FILE: the u-boot-env partition as board-stm32mp2.sh reads it:
# 512 KiB, copy 1 at 0x7e000 (flags 0x08, the active one, armed with
# mp2z_load/mp2z_window) and copy 2 at 0x7c000 (flags 0x07, clean).
dklayout() {
	python3 - "$1" <<'PY'
import sys, zlib
path = sys.argv[1]
SIZE = 0x2000
CLEAN = ["bootcmd=run bootcmd_stm32mp", "boot_targets=mmc0 mmc1 distro",
         "boot_prefixes=/mmc0_ /mmc1_", "bootdelay=1"]
ARMED = CLEAN + ["mp2z_load=0xa0000000", "mp2z_window=0x10000000"]
def copy(entries, flags):
    data = b"".join(v.encode() + b"\0" for v in entries) + b"\0"
    area = data + b"\0" * (SIZE - 5 - len(data))
    return zlib.crc32(area).to_bytes(4, "little") + bytes([flags]) + area
buf = bytearray(0x80000)
buf[0x7c000:0x7c000 + SIZE] = copy(CLEAN, 0x07)
buf[0x7e000:0x7e000 + SIZE] = copy(ARMED, 0x08)
open(path, "wb").write(buf)
PY
	echo "$1"
}

report() { echo "FAIL $1"; sed 's/^/  out| /' "$T/out"; sed 's/^/  err| /' "$T/err"; fail=$((fail + 1)); }

# run_guard ARGS...: the guard with $T/out, $T/err captured and RC pinned.
# STDIN_FIXTURE=FILE feeds stdin for the stdin-form cases; /dev/null
# otherwise (the file-form cases never read stdin). STDIN_FIXTURE is reset
# by every run.
RC=0
STDIN_FIXTURE=
run_guard() {
	RC=0
	if [ -n "$STDIN_FIXTURE" ]; then
		python3 -I "$here/mp2s_guard.py" "$@" <"$STDIN_FIXTURE" >"$T/out" 2>"$T/err" || RC=$?
	else
		python3 -I "$here/mp2s_guard.py" "$@" </dev/null >"$T/out" 2>"$T/err" || RC=$?
	fi
	STDIN_FIXTURE=
}

# expect_allow DESC [NEEDLE] -- ARGS...
expect_allow() {
	local what=$1 needle=
	shift
	if [ $# -gt 0 ] && [ "$1" != "--" ]; then needle=$1; shift; fi
	shift # the --
	run_guard "$@"
	if [ "$RC" = 0 ] && [ "$(grep -c '^mp2s-guard: allow: ' "$T/out" || true)" = 1 ] \
		&& [ ! -s "$T/err" ] && { [ -z "$needle" ] || grep -qF -- "$needle" "$T/out"; }; then
		pass=$((pass + 1))
	else
		report "$what (want exit 0, one allow line matching '$needle', empty stderr; got rc $RC)"
	fi
}

# expect_exact DESC FULL_STDERR_LINE -- ARGS...  (exit 3; stderr is exactly
# this one line; stdout empty)
expect_exact() {
	local what=$1 full=$2
	shift 2
	shift # the --
	run_guard "$@"
	if [ "$RC" = 3 ] && [ ! -s "$T/out" ] && [ "$(wc -l <"$T/err")" = 1 ] && [ "$(cat "$T/err")" = "$full" ]; then
		pass=$((pass + 1))
	else
		report "$what (want exit 3 and exactly the pinned line on stderr; got rc $RC)"
	fi
}

# expect_error DESC NEEDLE... -- ARGS...  (exit 2, empty stdout, no REFUSE
# line, every NEEDLE on stderr)
expect_error() {
	local what=$1
	shift
	local needles=()
	while [ $# -gt 0 ] && [ "$1" != "--" ]; do needles+=("$1"); shift; done
	shift # the --
	run_guard "$@"
	local ok=1 n
	if [ "$RC" != 2 ] || [ -s "$T/out" ] || [ ! -s "$T/err" ] || grep -q 'REFUSE' "$T/err"; then
		ok=0
	fi
	for n in ${needles[@]+"${needles[@]}"}; do grep -qF -- "$n" "$T/err" || ok=0; done
	if [ "$ok" = 1 ]; then
		pass=$((pass + 1))
	else
		report "$what (want exit 2, empty stdout, no REFUSE line, a line naming each needle; got rc $RC)"
	fi
}

CLEAN=(bootcmd="run bootcmd_stm32mp" boot_targets="mmc0 mmc1 distro" \
	boot_prefixes="/mmc0_ /mmc1_" bootdelay=1)
MP2S=(mp2s_ver=1 mp2s_left=2 mp2s_prefixes="/mmc0_ /mmc1_" \
	boot_prefixes="/yakogl/standalone/ /mmc0_ /mmc1_")
FOREVER=(mp2s_ver=1 mp2s_left=forever mp2s_prefixes="/mmc0_ /mmc1_" \
	boot_prefixes="/yakogl/standalone/ /mmc0_ /mmc1_")
DUP=(boot_prefixes="/yakogl/standalone/ /yakogl/standalone/ /mmc0_")

PE_CLEAN=$(pe "$T/pe-clean.txt" "${CLEAN[@]}")
PE_MP2S2=$(pe "$T/pe-mp2s2.txt" "${CLEAN[@]}" "${MP2S[@]}")
PE_FOREVER=$(pe "$T/pe-forever.txt" "${CLEAN[@]}" "${FOREVER[@]}")
PE_STRAY=$(pe "$T/pe-stray.txt" "${CLEAN[@]}" "mp2s_prefixes=/mmc0_ /mmc1_")
PE_YAKO=$(pe "$T/pe-yako.txt" "${CLEAN[@]}" yako_once=tok123)
PE_MP2Z=$(pe "$T/pe-mp2z.txt" "${CLEAN[@]}" mp2z_token=deadbeef)
PE_DUP=$(pe "$T/pe-dup.txt" "${CLEAN[@]}" "${DUP[@]}")
PE_COMBO=$(pe "$T/pe-combo.txt" "${CLEAN[@]}" yako_once=tok123 "${DUP[@]}")
PE_GARBAGE=$(pe "$T/pe-garbage.txt" '## Error: "bootcmd" not defined' 'hello world')

IMG_CLEAN=$(envbin "$T/img-clean.bin" 0x200 s \
	"0|${CLEAN[0]}|${CLEAN[1]}|${CLEAN[2]}|${CLEAN[3]}")
IMG_RED_STALE=$(envbin "$T/img-red-stale.bin" 0x200 r \
	"0|${CLEAN[0]}|${CLEAN[1]}|${CLEAN[2]}|${CLEAN[3]}|mp2s_ver=1|mp2s_left=2|mp2s_prefixes=/mmc0_ /mmc1_" \
	"1|${CLEAN[0]}|${CLEAN[1]}|${CLEAN[2]}|${CLEAN[3]}")
IMG_RED_BAD2=$(envbin "$T/img-red-bad2.bin" 0x200 r \
	"0|${CLEAN[0]}|${CLEAN[1]}|${CLEAN[2]}|${CLEAN[3]}" \
	"1|${CLEAN[0]}|${CLEAN[1]}|${CLEAN[2]}|${CLEAN[3]}")
corrupt "$IMG_RED_BAD2" 0x210
IMG_BADCRC=$T/img-badcrc.bin
cp "$IMG_CLEAN" "$IMG_BADCRC"
corrupt "$IMG_BADCRC" 0x10
IMG_WRITER=$T/img-writer.bin
python3 -I "$here/uboot_env.py" set "$IMG_CLEAN" "$IMG_WRITER" mp2s_left=2 >"$T/out" 2>"$T/err" || {
	echo "FAIL the uboot_env.py set fixture failed"; sed 's/^/  err| /' "$T/err"; exit 1
}
IMG_DK=$(dklayout "$T/img-dk.bin")

# ---- allow: clean, and each route's own state is not the other's --------
expect_allow "clean printenv text, --for oneshot" "no standalone default (no mp2s_* in" -- \
	--for oneshot --printenv "$PE_CLEAN"
STDIN_FIXTURE=$PE_CLEAN
expect_allow "clean printenv text on stdin, --for standalone" \
	"no one-shot state (no yako_*, no mp2z_*, boot_prefixes free of /yakogl/standalone/) in stdin" -- \
	--for standalone
expect_allow "stray mp2s_prefixes is not one-shot state (--for standalone)" -- \
	--for standalone --printenv "$PE_STRAY"
expect_allow "a pending yako_once is not a standalone default (--for oneshot)" "no mp2s_* in" -- \
	--for oneshot --printenv "$PE_YAKO"
expect_allow "a leftover mp2z_token is not a standalone default (--for oneshot)" -- \
	--for oneshot --printenv "$PE_MP2Z"
expect_allow "a doubled /yakogl/standalone/ prefix with no mp2s_* is not a default (--for oneshot)" -- \
	--for oneshot --printenv "$PE_DUP"
expect_allow "clean env image, --for oneshot" -- --for oneshot --env-image "$IMG_CLEAN"
expect_allow "clean env image, --for standalone" -- --for standalone --env-image "$IMG_CLEAN"
expect_allow "mp2s_left in the image alone is not one-shot state (--for standalone)" -- \
	--for standalone --env-image "$IMG_WRITER"
expect_allow "mp2s_left only in the stale copy (flags 0; the active copy, flags 1, is clean)" -- \
	--for oneshot --env-image "$IMG_RED_STALE" --redundant --size 0x200
expect_allow "mp2z_load in a DK-layout image is not a standalone default (--for oneshot)" -- \
	--for oneshot --env-image "$IMG_DK" --redundant --size 0x2000 --offset 0x7e000 --offset2 0x7c000

# ---- REFUSE: the pinned messages ----------------------------------------
expect_exact "oneshot refuses an armed default (mp2s_left=2)" \
	"mp2s-guard: REFUSE: a standalone default is set (mp2s_left=2): run tools/board-stm32mp2-standalone.sh nodefault first" -- \
	--for oneshot --printenv "$PE_MP2S2"
STDIN_FIXTURE=$PE_MP2S2
expect_exact "oneshot refuses the same default read on stdin" \
	"mp2s-guard: REFUSE: a standalone default is set (mp2s_left=2): run tools/board-stm32mp2-standalone.sh nodefault first" -- \
	--for oneshot
expect_exact "oneshot refuses mp2s_left=forever" \
	"mp2s-guard: REFUSE: a standalone default is set (mp2s_left=forever): run tools/board-stm32mp2-standalone.sh nodefault first" -- \
	--for oneshot --printenv "$PE_FOREVER"
expect_exact "oneshot refuses a stray mp2s_* without mp2s_left" \
	"mp2s-guard: REFUSE: standalone variables are set (mp2s_prefixes=/mmc0_ /mmc1_): run tools/board-stm32mp2-standalone.sh nodefault first" -- \
	--for oneshot --printenv "$PE_STRAY"
expect_exact "standalone refuses yako_once" \
	"mp2s-guard: REFUSE: a one-shot is set (yako_once=tok123): run tools/board-stm32mp2.sh disarm (yako_once) or uninstall (the hook's variables)" -- \
	--for standalone --printenv "$PE_YAKO"
expect_exact "standalone refuses mp2z_token" \
	"mp2s-guard: REFUSE: a one-shot is set (mp2z_token=deadbeef): undo the typed one-shot first (stm32mp257f-dk-uboot-boot.md section 7)" -- \
	--for standalone --printenv "$PE_MP2Z"
expect_exact "standalone refuses a doubled /yakogl/standalone/ prefix with no mp2s_* left" \
	"mp2s-guard: REFUSE: boot_prefixes already contains /yakogl/standalone/ (boot_prefixes=/yakogl/standalone/ /yakogl/standalone/ /mmc0_): run tools/board-stm32mp2-standalone.sh nodefault first" -- \
	--for standalone --printenv "$PE_DUP"
expect_exact "standalone refuses yako_once and the doubled prefix together" \
	"mp2s-guard: REFUSE: a one-shot is set (yako_once=tok123): run tools/board-stm32mp2.sh disarm (yako_once) or uninstall (the hook's variables); boot_prefixes already contains /yakogl/standalone/ (boot_prefixes=/yakogl/standalone/ /yakogl/standalone/ /mmc0_): run tools/board-stm32mp2-standalone.sh nodefault first" -- \
	--for standalone --printenv "$PE_COMBO"
expect_exact "oneshot refuses an env image the uboot_env.py writer armed (mp2s_left=2)" \
	"mp2s-guard: REFUSE: a standalone default is set (mp2s_left=2): run tools/board-stm32mp2-standalone.sh nodefault first" -- \
	--for oneshot --env-image "$IMG_WRITER"
expect_exact "standalone refuses mp2z_load in the active copy of a DK-layout image" \
	"mp2s-guard: REFUSE: a one-shot is set (mp2z_load=0xa0000000 mp2z_window=0x10000000): undo the typed one-shot first (stm32mp257f-dk-uboot-boot.md section 7)" -- \
	--for standalone --env-image "$IMG_DK" --redundant --size 0x2000 --offset 0x7e000 --offset2 0x7c000

# ---- error (exit 2): a refusal the caller must also treat as a refusal --
expect_error "bad CRC in the single copy" "bad CRC" -- --for oneshot --env-image "$IMG_BADCRC"
expect_error "bad CRC in the copy that is not active names copy 2" "copy 2" "bad CRC" -- \
	--for oneshot --env-image "$IMG_RED_BAD2" --redundant --size 0x200
expect_error "missing --printenv file" "not a readable file" -- \
	--for oneshot --printenv "$T/does-not-exist.txt"
expect_error "missing --env-image file" "not a readable file" -- \
	--for standalone --env-image "$T/does-not-exist.bin"
expect_error "printenv text with no NAME=VALUE line" "unreadable input" -- \
	--for oneshot --printenv "$PE_GARBAGE"
expect_error "empty stdin" "unreadable input" -- --for standalone
expect_error "--for missing" "required: --for" -- --printenv "$PE_CLEAN"
expect_error "--printenv and --env-image at once" "not allowed with argument --printenv" -- \
	--for oneshot --printenv "$PE_CLEAN" --env-image "$IMG_CLEAN"
expect_error "a layout flag used with --printenv" "go with --env-image" -- \
	--for oneshot --printenv "$PE_CLEAN" --redundant

echo "mp2s_guard_selftest: $pass checks, $fail failed (pinned: $WANT_CHECKS)"
[ "$fail" = 0 ] && [ "$pass" = "$WANT_CHECKS" ]
