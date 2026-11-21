#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Selftest of tools/vivante/resolve-asm-check.sh: feeds its extractor canned
# .s snippets (pass, fail by ldrb, fail without a wide load, not done) and a
# canned source file. No build, no target, no board.
# usage: bash tools/vivante/resolve-asm-check-selftest.sh

set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
chk=$here/resolve-asm-check.sh
tmp=$(mktemp -d "${TMPDIR:-.}/resolve-asm-selftest.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT

# Canned source: fn at line 1, the fallback block closes at line 7, fn ends
# at line 13.
cat >"$tmp/ds_cache.rs" <<'S'
fn resolve_row(src: &[u8], out: &mut [f32]) {
    let n = 1;
    if !pre.is_empty() {
        for o in out {
            *o = bytes(s);
        }
    }
    let wide = 1;
    for q in mid {
        read_volatile(q);
    }
    narrow(tail);
}
S

sym=_RNvNtNtCsX_6yakogl7vivante8ds_cache11resolve_row
T=$'\t'
# asm BODY: the fallback has ldrb at line 5, BODY is the aligned path.
asm() {
    printf '%s.file%s160 "/w" "src/vivante/ds_cache.rs"\n' "$T" "$T"
    printf '%s.type%s%s,@function\n%s:\n' "$T" "$T" "$sym" "$sym"
    printf '%s.loc%s160 1 0\n%sstr%sx30, [sp, #-16]!\n' "$T" "$T" "$T" "$T"
    printf '%s.loc%s160 5 9\n%sldrb%sw8, [x0, #2]\n%sldrb%sw9, [x0, #3]\n' "$T" "$T" "$T" "$T" "$T" "$T"
    printf '%s.loc%s160 8 5\n%sldr%sd0, [x8, :lo12:.LCPI0_0]\n' "$T" "$T" "$T" "$T"
    printf '%s\n' "$1"
    printf '%s.loc%s160 13 1\n%sret\n%s.size%s%s, .Lfunc_end0-%s\n' "$T" "$T" "$T" "$T" "$T" "$sym" "$sym"
    # after .size: another function, not resolve_row
    printf '%s.loc%s160 10 5\n%sldrb%sw9, [x1]\n' "$T" "$T" "$T" "$T"
}
PASS="${T}.loc${T}160 10 9
${T}ldp${T}x16, x17, [x11]
${T}.loc${T}160 12 5
${T}ldr${T}w11, [x9]"
FAILB="${T}.loc${T}160 10 9
${T}ldp${T}x16, x17, [x11]
${T}.loc${T}160 10 9
${T}ldrb${T}w10, [x11]"
NOWIDE="${T}.loc${T}160 10 9
${T}ldr${T}w16, [x11]"

fail=0
expect() { # name, want exit, want word, snippet
    local out rc
    asm "$4" >"$tmp/$1.s"
    out=$(bash "$chk" --check-asm "$tmp/$1.s" "$tmp/ds_cache.rs")
    rc=$?
    if [ "$rc" = "$2" ] && [[ $out == *"resolve-asm-check: $3"* ]]; then
        echo "ok   $1: $out"
    else
        echo "FAIL $1: exit $rc, got: $out"
        fail=1
    fi
}
expect pass 0 ok "$PASS"
expect fail-ldrb 1 FAIL "$FAILB"
expect fail-nowide 1 FAIL "$NOWIDE"

# not done: the symbol is missing (inlined away)
sed "s/$sym/other/g" "$tmp/pass.s" >"$tmp/inlined.s"
out=$(bash "$chk" --check-asm "$tmp/inlined.s" "$tmp/ds_cache.rs")
rc=$?
if [ "$rc" = 2 ] && [[ $out == *"not done"* ]]; then
    echo "ok   inlined: $out"
else
    echo "FAIL inlined: exit $rc, got: $out"
    fail=1
fi

if [ "$fail" = 0 ]; then echo "resolve-asm-check-selftest: ok"; else echo "resolve-asm-check-selftest: FAIL"; fi
exit "$fail"
