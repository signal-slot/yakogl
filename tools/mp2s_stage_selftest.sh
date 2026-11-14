#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_stage_selftest.sh - tools/mp2s_stage.py on a temp root, no board:
# the happy path, a foreign file, a symlink escape, a path outside the
# directory, a read-back mismatch, an idempotent remove-all and the UNDO line
# coming before the first write.
#
#   tools/mp2s_stage_selftest.sh
set -u
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_stage.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
st() { python3 -I "$here/mp2s_stage.py" --root "$R" "$@"; }
fails=0
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; fails=$((fails + 1)); }
expect0() { local m=$1; shift; if "$@" >"$tmp/o" 2>"$tmp/e"; then ok "$m"; else bad "$m"; cat "$tmp/o" "$tmp/e"; fi; }
expect1() { # message, stderr pattern, command
    local m=$1 pat=$2; shift 2
    if "$@" >"$tmp/o" 2>"$tmp/e"; then bad "$m: exit 0"; elif grep -q -- "$pat" "$tmp/e"; then ok "$m"; else bad "$m: no '$pat'"; cat "$tmp/e"; fi
}
mk() { R=$tmp/$1; rm -rf "$R"; mkdir -p "$R/boot"; D=$R/boot/yakogl/standalone; }
printf 'payload-%s' 1 >"$tmp/src"
sum=$(sha256sum "$tmp/src" | cut -d' ' -f1)
names="boot.scr.uimg boot.scr.uimg.sha256 default.bin backup/u-boot-env.bin boot-sha256.txt"

# 1. happy path
mk happy
expect0 "check-dir on a missing directory" st check-dir
for n in $names; do
    expect0 "put $n" st put "$tmp/src" "$n"
    [ "$(sha256sum "$D/$n" | cut -d' ' -f1)" = "$sum" ] && ok "$n content" || bad "$n content"
done
[ -z "$(find "$D" -name '*.part')" ] && ok "no .part left" || bad ".part left"
expect0 "check-dir with all files" st check-dir
expect0 "put again (replace)" st put "$tmp/src" default.bin
expect0 "remove default.bin" st remove default.bin
[ ! -e "$D/default.bin" ] && ok "default.bin gone" || bad "default.bin still there"
expect0 "remove of an absent file" st remove default.bin

# 2. remove-all: only contract files, then empty dirs, idempotent
expect0 "remove-all" st remove-all
[ ! -e "$D" ] && ok "directory removed" || bad "directory left"
[ -d "$R/boot" ] && ok "/boot kept" || bad "/boot gone"
expect0 "remove-all again (idempotent)" st remove-all

# 3. foreign file
mk foreign
st put "$tmp/src" default.bin >/dev/null
echo x >"$D/notes.txt"
expect1 "check-dir refuses a foreign file" "notes.txt" st check-dir
expect1 "put refuses with a foreign file" "notes.txt" st put "$tmp/src" boot-sha256.txt
expect1 "remove-all refuses a foreign file" "notes.txt" st remove-all
[ -e "$D/default.bin" ] && ok "remove-all removed nothing" || bad "remove-all removed a file"
mkdir "$D/extra"
rm "$D/notes.txt"
expect1 "foreign directory refused" "extra" st check-dir
rmdir "$D/extra"; echo x >"$D/default.bin.part"
expect1 "a stale .part is foreign" "default.bin.part" st check-dir

# 4. names outside the contract and paths outside the directory
mk names
for n in ../x /etc/passwd ../../evil default.bin/../../x backup/other other.bin; do
    expect1 "put refuses name $n" "not a file of the contract" st put "$tmp/src" "$n"
    expect1 "remove refuses name $n" "not a file of the contract" st remove "$n"
done
[ ! -e "$D" ] && ok "refusals created nothing" || bad "refusal created $D"

# 5. symlink escapes
mk link1; mkdir -p "$D/backup" "$tmp/outside"
ln -s "$tmp/outside/stolen" "$D/default.bin"
expect1 "symlinked file refused" "symlink" st put "$tmp/src" default.bin
expect1 "symlink is foreign to check-dir" "default.bin" st check-dir
[ ! -e "$tmp/outside/stolen" ] && ok "nothing written through the link" || bad "wrote through the link"
rm "$D/default.bin"; rmdir "$D/backup"; ln -s "$tmp/outside" "$D/backup"
expect1 "symlinked backup/ refused" "backup" st put "$tmp/src" backup/u-boot-env.bin
[ -z "$(ls "$tmp/outside")" ] && ok "nothing written through backup/" || bad "wrote through backup/"
mk link2; mkdir -p "$R/boot/yakogl"; ln -s "$tmp/outside" "$R/boot/yakogl/standalone"
expect1 "symlinked standalone/ refused" "outside the standalone" st put "$tmp/src" default.bin
expect1 "remove-all through a link refused" "outside the standalone" st remove-all
[ -z "$(ls "$tmp/outside")" ] && ok "outside untouched" || bad "outside touched"
mk link3; ln -s "$tmp/outside" "$R/boot/yakogl"
expect1 "symlinked yakogl/ refused" "outside the standalone" st put "$tmp/src" default.bin
echo keep >"$tmp/outside/keep"
mk link4; mkdir -p "$D"; ln -s "$tmp/outside/keep" "$D/boot-sha256.txt"
expect1 "remove-all refuses a symlink" "boot-sha256.txt" st remove-all
[ -f "$tmp/outside/keep" ] && ok "link target kept" || bad "link target removed"

# 6. read-back mismatch removes the .part and the target
mk fault
st put "$tmp/src" default.bin >/dev/null
echo new >"$tmp/src2"
if MP2S_STAGE_FAULT=readback st put "$tmp/src2" default.bin >"$tmp/o" 2>"$tmp/e"; then bad "mismatch: exit 0"
elif grep -q "read-back" "$tmp/e"; then ok "read-back mismatch refused"; else bad "mismatch message"; cat "$tmp/e"; fi
[ ! -e "$D/default.bin" ] && [ ! -e "$D/default.bin.part" ] && ok "target and .part removed" || bad "target or .part left"
grep -q '^UNDO: ' "$tmp/o" && ok "UNDO printed before the failed write" || bad "no UNDO on the failed write"

# 7. UNDO first, naming the reverse action
mk undo
st put "$tmp/src" default.bin >"$tmp/o"
[ "$(head -1 "$tmp/o" | cut -c1-6)" = "UNDO: " ] && grep -q 'remove default.bin' "$tmp/o" && ok "put: UNDO first, names remove" || bad "put UNDO"
st remove default.bin >"$tmp/o"
[ "$(head -1 "$tmp/o" | cut -c1-6)" = "UNDO: " ] && grep -q 'put <source> default.bin' "$tmp/o" && ok "remove: UNDO first, names put" || bad "remove UNDO"
st put "$tmp/src" default.bin >/dev/null
st remove-all >"$tmp/o"
[ "$(head -1 "$tmp/o" | cut -c1-6)" = "UNDO: " ] && ok "remove-all: UNDO first" || bad "remove-all UNDO"
# ordering proof: the UNDO line is out before the source is even read
mk order
st put "$tmp/missing" default.bin >"$tmp/o" 2>/dev/null
[ "$(head -1 "$tmp/o" | cut -c1-6)" = "UNDO: " ] && ok "UNDO printed before a source error" || bad "UNDO after the error"
[ ! -e "$D" ] && ok "nothing created on a source error" || bad "directory created"
# a refusal prints no UNDO (nothing is about to be written)
mk none
st put "$tmp/src" nope.bin >"$tmp/o" 2>/dev/null
grep -q UNDO "$tmp/o" && bad "UNDO printed for a refused name" || ok "no UNDO for a refused name"

echo
if [ "$fails" -eq 0 ]; then echo "mp2s_stage_selftest: all ok"; exit 0; fi
echo "mp2s_stage_selftest: $fails failed"; exit 1
