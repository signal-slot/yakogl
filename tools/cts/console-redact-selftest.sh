#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of console-redact.py: the version rule brackets the dEQP version token, the addr/uart/home
# rules replace what they match, the rewrites are idempotent, clean and binary files are left untouched,
# the tool's verdicts agree with the acceptance grep on every sample, the expectations for the real
# b1b5b845 abort consoles hold on copies (the committed logs are only read), --check-readme fails on a
# README without a first 'commit: <40 hex>' line, and neither this file nor the tool trips the
# acceptance grep. Samples are built by concatenation so no sample string appears whole in this file.
# Host only; scratch under ${TMPDIR:-$PWD}, never /tmp.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/console-redact.py
case "${TMPDIR:-$PWD}" in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "refusing a scratch dir under /tmp; set TMPDIR" >&2; exit 2 ;; esac
tmp=$(mktemp -d "${TMPDIR:-$PWD}/console-redact-selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
nfail=0 nok=0
ok() { echo "ok: $*"; nok=$((nok + 1)); }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }

# the acceptance grep, assembled by concatenation
acc='\b([0-9]{1,3}\.){3}[0-9]{1,3}\b|/dev/''tty|/home''/'

# the real version line of the b1b5b845 consoles and its redacted form; the concatenation splits
# the quotes ('..' ".." '..') so the quad never appears whole in this file
version_line='dEQP Core vulkan-cts-1.4.6'""'.2-555-g93bca01861b0e3ef3c387027a9791e6d065f900c (0x93bca018) starting..'
want_version='dEQP Core vulkan-cts-<1.4.6-2>-555-g93bca01861b0e3ef3c387027a9791e6d065f900c (0x93bca018) starting..'
testnet='vulkan-cts-192.0.2'""'.2-555-g93bca01861b0e3ef3c387027a9791e6d065f900c'
want_testnet='vulkan-cts-<192.0.2-2>-555-g93bca01861b0e3ef3c387027a9791e6d065f900c'
bare_addr='10'""'.20'""'.30'""'.40'
bvnc='BVNC 36.53.104'""'.796'
uart='/dev/''ttyAMA0'
uartdot='/dev/''tty…'
uartbare='/dev/''tty'
homepath='/home''/runner/logs/x.log'
clean1='Pass 2463 Fail 28 NotSupported 387'
clean2='v3d_deqp: build b1b5b845 at 210 s after boot (V3D 4.2, 8 QPUs)'
half='version 1.4.6 and 42.24'

# --- stdin to stdout
out=$(printf '%s\n' "$version_line" | python3 -I "$tool"); rc=$?
if [ $rc -eq 0 ] && [ "$out" = "$want_version" ]; then ok "stdin->stdout brackets the version token"; else bad "stdin->stdout: rc=$rc out=$out"; fi
if printf '%s\n' "$out" | grep -Eq -- "$acc"; then bad "stdout output still trips the acceptance grep"; else ok "stdout output clean"; fi

out=$(printf '%s and %s and %s and %s\n' "$testnet" "$uart" "$homepath" "$bare_addr" | python3 -I "$tool")
want="$want_testnet and <uart> and <home> and <addr>"
if [ "$out" = "$want" ]; then ok "stdin->stdout applies all rules"; else bad "stream mix: got $out"; fi
if printf '%s\n' "$out" | grep -Eq -- "$acc"; then bad "stream mix output trips the grep"; else ok "stream mix output clean"; fi

# --- in place, idempotence, clean and binary left alone
mkdir -p "$tmp/in/sub" "$tmp/eq"
printf '%s\n%s\n' "$version_line" "$clean2" > "$tmp/in/TF2a.console.txt"
printf '%s\n%s\n' "$uartdot" "$homepath" > "$tmp/in/sub/misc.txt"
printf '%s\n%s\n' "$bvnc" "$bare_addr" > "$tmp/in/dirty.tsv"
printf '%s\n' "$clean1" > "$tmp/in/clean.txt"
printf '\0\1 %s\n' "$bare_addr" > "$tmp/in/bin.dat"
out1=$(python3 -I "$tool" "$tmp/in" 2>&1); rc=$?
if [ $rc -eq 0 ]; then ok "in place exits 0"; else bad "in place rc=$rc"; fi
if grep -qF -- "$want_version" "$tmp/in/TF2a.console.txt" && ! grep -qF -- "$version_line" "$tmp/in/TF2a.console.txt"; then
    ok "console rewritten in place"; else bad "console not rewritten"; fi
if grep -qF -- "$clean2" "$tmp/in/TF2a.console.txt"; then ok "clean line kept byte for byte"; else bad "clean line damaged"; fi
if grep -qxF -- "<uart>" "$tmp/in/sub/misc.txt" && grep -qxF -- "<home>" "$tmp/in/sub/misc.txt"; then
    ok "dir walked, uart and home replaced whole-line"; else bad "misc.txt: $(cat "$tmp/in/sub/misc.txt")"; fi
if grep -qF -- "BVNC <addr>" "$tmp/in/dirty.tsv" && grep -qxF -- "<addr>" "$tmp/in/dirty.tsv"; then
    ok "BVNC-style quad replaced per rule b"; else bad "dirty.tsv: $(cat "$tmp/in/dirty.tsv")"; fi
out=$(python3 -I "$tool" "$tmp/in" 2>&1)
if printf '%s\n' "$out" | grep -qF -- "0 rewritten"; then ok "idempotent (second run rewrites nothing)"; else bad "not idempotent: $out"; fi
if ! cmp -s "$tmp/in/clean.txt" <(printf '%s\n' "$clean1"); then bad "clean file changed"; else ok "clean file untouched"; fi
if printf '%s\n' "$out1" | grep -qF -- "rewrote $tmp/in/TF2a.console.txt (version=1)"; then
    ok "report names the rule and count"; else bad "report line missing: $out1"; fi

# binary: a file with a NUL byte is skipped, like grep -I
printf '\0\1 %s\n' "$bare_addr" > "$tmp/in/bin.dat"
cp "$tmp/in/bin.dat" "$tmp/bin.orig"
python3 -I "$tool" "$tmp/in" >/dev/null 2>&1
if cmp -s "$tmp/bin.orig" "$tmp/in/bin.dat"; then ok "binary skipped"; else bad "binary rewritten"; fi

# --- the tool and the acceptance grep agree: grep hits  <=>  the tool changes the file
samples=("$version_line" "$want_version" "$testnet" "$want_testnet" "$bare_addr" "$bvnc" "$uart"
         "$uartdot" "$uartbare" "x/$uart" "/home" "$homepath" "$clean1" "$clean2" "$half" "1.4.6")
for s in "${samples[@]}"; do
    printf '%s\n' "$s" > "$tmp/eq/s.txt"
    cp "$tmp/eq/s.txt" "$tmp/s.orig"   # outside the dir, so the tool only sees s.txt
    grep -Eq -- "$acc" "$tmp/eq/s.txt"; g=$?
    python3 -I "$tool" "$tmp/eq" >/dev/null 2>&1
    if cmp -s "$tmp/s.orig" "$tmp/eq/s.txt"; then c=0; else c=1; fi
    if [ $g -eq 0 ] && [ $c -eq 1 ]; then ok "grep and tool agree (hit, rewritten)"
    elif [ $g -ne 0 ] && [ $c -eq 0 ]; then ok "grep and tool agree (clean, kept)"
    else bad "grep/tool differ (grep=$g changed=$c) on a sample"; fi
done
for want in "$want_version" "$want_testnet" "BVNC <addr> <addr>" "<uart> <home>" "<addr>"; do
    if printf '%s\n' "$want" | grep -Eq -- "$acc"; then bad "a redacted form still trips the grep"; else ok "redacted form clean: <...>"; fi
done

# --- --check-readme
sha='b1b5b8456ec69247c64d1bd07e42461266dc5136'
mkreadme() { # $1 dir, $2 body...
    mkdir -p "$1"
    {
        printf '<!-- SPDX-License-Identifier: Apache-2.0 -->\n<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->\n'
        printf '%b' "$2"
    } > "$1/README.md"
}
mkreadme "$tmp/r/good" '# Title\n\ncommit: '"$sha"'\n'
mkreadme "$tmp/r/first" 'commit: '"$sha"'\n# Title\n'
mkreadme "$tmp/r/crlf" '# Title\r\ncommit: '"$sha"'\r\n'
mkreadme "$tmp/r/nocommit" '# Title\nno commit line here\n'
mkreadme "$tmp/r/short" '# Title\ncommit: b1b5b8456ec69247c64d1bd07e42461266dc513\n'
mkreadme "$tmp/r/notsha" '# Title\ncommit: origin/main\ncommit: '"$sha"'\n'
mkdir -p "$tmp/r/noreadme"
out=$(python3 -I "$tool" --check-readme "$tmp/r/good" 2>&1); rc=$?
if [ $rc -eq 0 ] && printf '%s\n' "$out" | grep -qF -- "1 README ok"; then ok "README with commit after the SPDX header passes"; else bad "good README: rc=$rc out=$out"; fi
out=$(python3 -I "$tool" --check-readme "$tmp/r/first" "$tmp/r/crlf" 2>&1); rc=$?
if [ $rc -eq 0 ] && printf '%s\n' "$out" | grep -qF -- "2 README ok"; then ok "commit first line and CRLF accepted"; else bad "first/crlf: rc=$rc out=$out"; fi
for bad_case in nocommit short notsha noreadme; do
    python3 -I "$tool" --check-readme "$tmp/r/$bad_case" >/dev/null 2>&1; rc=$?
    if [ $rc -eq 1 ]; then ok "--check-readme fails on $bad_case"; else bad "$bad_case: rc=$rc, expected 1"; fi
done
out=$(python3 -I "$tool" --check-readme "$tmp/r/good" "$tmp/r/short" 2>&1); rc=$?
if [ $rc -eq 1 ] && printf '%s\n' "$out" | grep -qF -- "1 failed, 1 ok"; then ok "mixed dirs: failed counted"; else bad "mixed: rc=$rc out=$out"; fi
out=$(python3 -I "$tool" --check-readme "$tmp/r/does-not-exist" 2>&1); rc=$?
if [ $rc -eq 2 ]; then ok "missing DIR exits 2"; else bad "missing DIR rc=$rc"; fi
python3 -I "$tool" --check-readme >/dev/null 2>&1; rc=$?
if [ $rc -eq 2 ]; then ok "--check-readme without a DIR exits 2"; else bad "no DIR rc=$rc"; fi

# --- the real b1b5b845 abort consoles, on copies only
abort_dir=$here/log/2026-11-pi4-gles3-b1b5b845-aborts
if ls "$abort_dir"/*.console.txt >/dev/null 2>&1; then
    before=$(grep -lF -- "$version_line" "$abort_dir"/*.console.txt | wc -l)
    if grep -qE -- "$acc" "$abort_dir"/*.console.txt; then
        ok "the committed abort consoles trip the acceptance grep as expected (they are not edited here)"
    else
        bad "the committed abort consoles unexpectedly already clean"
    fi
    mkdir -p "$tmp/aborts"
    for f in "$abort_dir"/*.console.txt; do cp "$f" "$tmp/aborts/$(basename "$f")"; done
    python3 -I "$tool" "$tmp/aborts" >/dev/null 2>&1
    if grep -Eq -- "$acc" "$tmp/aborts"/*.console.txt; then
        bad "redacted copies still trip the acceptance grep"
    else
        ok "redacted copies carry no line the acceptance grep matches"
    fi
    after=$(grep -lF -- "$want_version" "$tmp/aborts"/*.console.txt | wc -l)
    if [ "$before" = "$after" ] && [ "$before" -gt 0 ]; then
        ok "every version line became the bracketed form ($before files)"
    else
        bad "version lines: before=$before after=$after"
    fi
    nbadlines=0
    for f in "$abort_dir"/*.console.txt; do
        b=$(wc -l < "$f"); a=$(wc -l < "$tmp/aborts/$(basename "$f")")
        [ "$b" = "$a" ] || { nbadlines=$((nbadlines + 1)); echo "  line count differs: $(basename "$f") $b vs $a" >&2; }
    done
    if [ $nbadlines -eq 0 ]; then ok "line counts preserved in every copy"; else bad "$nbadlines copies changed line count"; fi
    if grep -qF -- "$version_line" "$abort_dir/TF2a.console.txt"; then
        ok "the committed logs are left untouched"
    else
        bad "the committed abort console was modified by the selftest"
    fi
else
    ok "no committed b1b5b845 abort consoles in this checkout"
fi

# --- this file and the tool carry nothing the acceptance grep matches
if grep -nE -- "$acc" "$tool" "$0" >/dev/null; then
    bad "the tool or this selftest trips the acceptance grep"
    grep -nE -- "$acc" "$tool" "$0" >&2
else
    ok "tool and selftest are themselves clean under the acceptance grep"
fi

echo "console-redact selftest: $nok ok, $nfail failed"
[ $nfail -eq 0 ]
