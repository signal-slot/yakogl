#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# t6-uboot-log-selftest.sh - tools/cts/t6-uboot-log.sh without a board
# (plan #408, item t6-log-tool).
#
#   bash tools/cts/t6-uboot-log-selftest.sh
#
# A fake NanoPC-T6 U-Boot record under $TMPDIR (made-up values only: a
# locally administered MAC, a 100.64/10 address, a made-up UUID) is copied
# into a fake log root (T6_UBOOT_LOG_ROOT; tools/cts/log is never written):
# the UART path, the MAC and the whole tailnet address are redacted by
# console-redact.py, the relative paths are kept, binary files and symbolic
# links are left out, the record itself is unchanged and the copy passes
# the acceptance grep. Then one record per secret kind that survives the
# redactor (the bridge URL, an upper-case tty path, a seven-pair MAC, a
# partly redacted tailnet address, a UUID, a serial number, the T6_UART
# value from the environment) is refused with exit 1, its kind named, the
# secret never echoed and nothing left behind in the log root. An existing
# destination is never overwritten (exit 3), bad arguments exit 2. No
# sample token appears whole in this file: samples are built from pieces
# at run time, and the end checks that this file and the tool pass the
# acceptance grep.
#
# Host only; scratch under ${TMPDIR:-$PWD}, never /tmp.
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tool="$here/t6-uboot-log.sh"
self="$here/t6-uboot-log-selftest.sh"
case "${TMPDIR:-$PWD}" in /tmp | /tmp/* | /var/tmp | /var/tmp/*)
  echo "refusing a scratch dir under /tmp; set TMPDIR" >&2
  exit 2
  ;;
esac
T=$(mktemp -d "${TMPDIR:-$PWD}/t6-uboot-log-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT HUP INT TERM
nok=0 nfail=0
ok() { echo "ok: $*"; nok=$((nok + 1)); }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }

LOG=$T/log
mkdir -p "$LOG"
# the caller's own board environment must not leak into the cases
run() {
  env -u BOARD_HOST -u BOARD_USER -u BOARD_UART -u T6_UART -u T6_HOST \
    -u REDACT_EXTRA T6_UBOOT_LOG_ROOT="$LOG" bash "$tool" "$@"
}

# samples built from pieces (no token whole in this file)
BR='rfc2217'"://"
UA='/dev/''tty''USB0'
UA_UP='/DEV/''TTY''USB1'
ACM='/dev/''tty''ACM2'
MAC='02:00:5e:''10:00:01'
MAC7='02:00:5e:''10:00:01:7f'
TN='100'""'.64.0.9'
TN_PART='100'""'.101.x.x'
uuid_re='[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}'
UUID='3f2a1c9e''-7b4d-4e2a-9c1f-''0a1b2c3d4e5f'
SER='iSer''ial 3 A1B2C3D4E5'
HOSTV='bridge-''fake-host'

acc='rfc2217'"://"'|/dev/''tty(USB|ACM)|([0-9a-f]{2}:){5}[0-9a-f]{2}|100\.(6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\.'

# --- a record the redactor cleans: copied
R=$T/rec/20261126-1200-t6-uboot-s2-0123abcd
mkdir -p "$R/20261126-1201-t6-uboot-go" "$R/20261126-1210-t6-uboot-reads"
printf 'U-Boot 2017.09\nconsole on %s at 1500000\n=> go 0x10000000\nT6:G\ngl_test: 50/50 PASS\n' "$UA" \
  > "$R/20261126-1201-t6-uboot-go/uart.log"
printf 'PASS\nexit 0\n' > "$R/20261126-1201-t6-uboot-go/verdict.txt"
printf 'ethaddr=%s\nserverip %s\nacm %s\n' "$MAC" "$TN" "$ACM" > "$R/20261126-1210-t6-uboot-reads/answers.txt"
{
  printf '=> mmc part\n\nPartition Map for MMC device 0  --   Partition Type: EFI\n\n'
  printf 'Part\tStart LBA\tEnd LBA\t\tName\n\tAttributes\n\tType GUID:\t%s\n\tPartition GUID:\t%s\n' "$UUID" "$UUID"
  printf '  1\t0x00008000\t0x0ffffff\t"rootfs"\n\tguid:\t%s\n=> printenv serial#\nserial%s=0123456789abcdef\n' "$UUID" '#'
} >> "$R/20261126-1210-t6-uboot-reads/answers.txt"
printf 'prompt-reads: 27 commands, exit 0\n' > "$R/prompt-reads.log"
printf '\0\1\2 %s\n' "$UA" > "$R/image.bin"
ln -s uart.log "$R/20261126-1201-t6-uboot-go/link.log"
cp -a "$R" "$T/rec.orig"
D=$LOG/2026-11-26-t6-uboot-s2-0123abcd
out=$(run "$R" 2026-11-26 s2 0123abcd 2>&1); rc=$?
if [ $rc -eq 0 ] && [ -d "$D" ]; then ok "a redactable record is copied (exit 0)"; else bad "copy: rc=$rc out=$out"; fi
if [ -f "$D/20261126-1201-t6-uboot-go/uart.log" ] && [ -f "$D/20261126-1210-t6-uboot-reads/answers.txt" ] &&
  [ -f "$D/20261126-1201-t6-uboot-go/verdict.txt" ] && [ -f "$D/prompt-reads.log" ]; then
  ok "the text files keep their relative paths"
else bad "copied tree: $(cd "$D" 2>/dev/null && find . -type f | sort | tr '\n' ' ')"; fi
if [ ! -e "$D/image.bin" ] && [ ! -e "$D/20261126-1201-t6-uboot-go/link.log" ]; then
  ok "binary file and symbolic link left out"
else bad "binary or link copied"; fi
if printf '%s\n' "$out" | grep -qF 'skipped (binary): image.bin' && printf '%s\n' "$out" | grep -qF 'skipped (symbolic link)'; then
  ok "the left-out files are named"
else bad "skip report: $out"; fi
if grep -qF 'console on <uart> at 1500000' "$D/20261126-1201-t6-uboot-go/uart.log" &&
  grep -qxF 'ethaddr=<mac>' "$D/20261126-1210-t6-uboot-reads/answers.txt" &&
  grep -qxF 'serverip <addr>' "$D/20261126-1210-t6-uboot-reads/answers.txt" &&
  grep -qxF 'acm <uart>' "$D/20261126-1210-t6-uboot-reads/answers.txt"; then
  ok "tty path, MAC and tailnet address redacted by console-redact.py"
else bad "redacted copy: $(cat "$D"/*/*)"; fi
if grep -qxF 'serial#=<serial>' "$D/20261126-1210-t6-uboot-reads/answers.txt" &&
  [ "$(grep -c '<uuid>' "$D/20261126-1210-t6-uboot-reads/answers.txt")" = 3 ] &&
  ! grep -rEiq -- "$uuid_re|0123456789abcdef" "$D"; then
  ok "mmc part UUIDs and serial# replaced, record copied"
else bad "mmc part / serial# copy: $(cat "$D"/*/answers.txt)"; fi
if grep -qxF 'gl_test: 50/50 PASS' "$D/20261126-1201-t6-uboot-go/uart.log" && cmp -s "$R/prompt-reads.log" "$D/prompt-reads.log"; then
  ok "clean lines kept byte for byte"
else bad "clean lines damaged"; fi
if grep -rEiq -- "$acc" "$D"; then bad "the copy trips the acceptance grep"; else ok "the copy passes the acceptance grep"; fi
if diff -r "$R" "$T/rec.orig" >/dev/null 2>&1; then ok "the record is unchanged"; else bad "the record was changed"; fi
if [ -z "$(find "$LOG" -mindepth 1 -maxdepth 1 -name '.*')" ]; then ok "no scratch directory left"; else bad "scratch left in the log root"; fi

# --- the destination exists: never overwritten
printf 'keep\n' > "$D/mine.txt"
cp -a "$D" "$T/dest.orig"
run "$R" 2026-11-26 s2 0123abcd >/dev/null 2>&1; rc=$?
if [ $rc -eq 3 ] && diff -r "$D" "$T/dest.orig" >/dev/null; then ok "an existing destination is refused (exit 3) and untouched"
else bad "existing destination: rc=$rc"; fi
mkdir "$LOG/2026-11-26-t6-uboot-empty-0123abcd"
run "$R" 2026-11-26 empty 0123abcd >/dev/null 2>&1; rc=$?
if [ $rc -eq 3 ] && [ -z "$(ls -A "$LOG/2026-11-26-t6-uboot-empty-0123abcd")" ]; then ok "an existing empty destination is refused too"
else bad "empty destination: rc=$rc"; fi

# --- one refusal per secret kind that survives the redactor
refusal() { # refusal <label> <kind-word> <line> [env...]
  local label=$1 kind=$2 line=$3 r out rc
  shift 3
  r=$T/rec-$label
  mkdir -p "$r/sub"
  printf 'U-Boot 2017.09\n%s\n' "$line" > "$r/sub/uart.log"
  printf 'PASS\n' > "$r/verdict.txt"
  out=$(env "$@" T6_UBOOT_LOG_ROOT="$LOG" bash -c 'unset BOARD_HOST BOARD_USER BOARD_UART REDACT_EXTRA; exec bash "$0" "$@"' \
    "$tool" "$r" 2026-11-26 "r-$label" 0123abcd 2>&1); rc=$?
  if [ $rc -eq 1 ] && [ ! -e "$LOG/2026-11-26-t6-uboot-r-$label-0123abcd" ] &&
    [ -z "$(find "$LOG" -mindepth 1 -maxdepth 1 -name '.*')" ]; then
    ok "REFUSED $label (exit 1, nothing left behind)"
  else bad "$label: rc=$rc out=$out"; fi
  if printf '%s\n' "$out" | grep -Eq "REFUSED sub/uart.log(:[0-9]+)?: .*$kind"; then ok "$label refusal names the file and the kind ($kind)"
  else bad "$label: no REFUSED line naming $kind: $out"; fi
  if [ -n "$line" ] && printf '%s\n' "$out" | grep -qF -- "$line"; then bad "$label: the secret was echoed"; fi
}
refusal url "acceptance grep" "uart via ${BR}<addr>:4000" T6_UART=
refusal tty "acceptance grep" "console $UA_UP" T6_UART=
refusal mac "acceptance grep" "ethaddr $MAC7" T6_UART=
refusal tailnet "acceptance grep" "peer $TN_PART up" T6_UART=
refusal uuid "uuid" "disk id $UUID" T6_UART=
refusal bootid "uuid" "boot_id $UUID" T6_UART=
refusal serial "t6-redact-check usb-serial" "$SER" T6_UART=
refusal envhost "t6-redact-check env:T6_UART" "bridge $HOSTV:4000" T6_UART="$HOSTV"

# a secret in a path name is refused as well
r=$T/rec-pathname
mkdir -p "$r/$UUID"
printf 'PASS\n' > "$r/$UUID/verdict.txt"
out=$(run "$r" 2026-11-26 pathname 0123abcd 2>&1); rc=$?
if [ $rc -eq 1 ] && printf '%s\n' "$out" | grep -qF 'REFUSED (a path name): uuid' && [ ! -e "$LOG/2026-11-26-t6-uboot-pathname-0123abcd" ]; then
  ok "a UUID in a path name is refused"
else bad "path name: rc=$rc out=$out"; fi

# --exclude leaves a file out, so the rest lands
r=$T/rec-exclude
mkdir -p "$r"
printf 'disk id %s\n' "$UUID" > "$r/answers.txt"
printf 'PASS\n' > "$r/verdict.txt"
out=$(run --exclude answers.txt "$r" 2026-11-26 exclude 0123abcd 2>&1); rc=$?
E=$LOG/2026-11-26-t6-uboot-exclude-0123abcd
if [ $rc -eq 0 ] && [ -f "$E/verdict.txt" ] && [ ! -e "$E/answers.txt" ]; then ok "--exclude leaves the named file out"
else bad "--exclude: rc=$rc out=$out"; fi

# --- usage and bad arguments: exit 2, nothing written
before=$(find "$LOG" | sort)
for args in "" "$R" "$R 2026-11-26 s2" "$T/none 2026-11-26 x 0123abcd" "$R 26-11-26 x 0123abcd" \
  "$R 2026-11-26 Bad_Name 0123abcd" "$R 2026-11-26 x 0123ABCD" "$R 2026-11-26 x 0123abc" "--bogus $R 2026-11-26 x 0123abcd"; do
  # shellcheck disable=SC2086  # the case's words are split on purpose
  run $args >/dev/null 2>&1; rc=$?
  if [ $rc -eq 2 ]; then ok "bad arguments exit 2 (${args:-no arguments})"; else bad "args '$args': rc=$rc"; fi
done
mkdir -p "$T/rec-binonly"
printf '\0x\n' > "$T/rec-binonly/a.bin"
run "$T/rec-binonly" 2026-11-26 binonly 0123abcd >/dev/null 2>&1; rc=$?
if [ $rc -eq 2 ]; then ok "a record without text files exits 2"; else bad "binary-only: rc=$rc"; fi
ln -s "$R" "$T/rec-link"
run "$T/rec-link" 2026-11-26 link 0123abcd >/dev/null 2>&1; rc=$?
if [ $rc -eq 2 ]; then ok "a symbolic link as RECORD-DIR exits 2"; else bad "link record: rc=$rc"; fi
if [ "$(find "$LOG" | sort)" = "$before" ]; then ok "the failed runs wrote nothing"; else bad "a failed run wrote into the log root"; fi
if bash "$tool" --help | grep -qF 't6-uboot-log.sh [--exclude NAME]... RECORD-DIR DATE NAME SHA8'; then ok "--help prints the usage"
else bad "--help"; fi

# --- the tool and this file trip neither the acceptance grep nor t6-redact-check
for f in "$tool" "$self"; do
  if grep -Eiq -- "$acc" "$f"; then bad "$(basename "$f") trips the acceptance grep"
  else ok "$(basename "$f") passes the acceptance grep"; fi
  if env -u BOARD_HOST -u BOARD_USER -u BOARD_UART -u T6_UART -u T6_HOST -u REDACT_EXTRA \
    bash "$here/t6-redact-check.sh" "$f" >/dev/null 2>&1; then ok "$(basename "$f") passes t6-redact-check"
  else bad "t6-redact-check flags $(basename "$f")"; fi
done

echo "t6-uboot-log selftest: $nok ok, $nfail failed"
[ "$nfail" -eq 0 ]
