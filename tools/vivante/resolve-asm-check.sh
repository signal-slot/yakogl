#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Guard for src/vivante/ds_cache.rs resolve_row (item resolve-wide, see
# docs/notes/gc-slow-shadow-refract.md cause 1): the CPU depth resolve reads
# an uncached carve-out, so every narrow load is a bus read. This builds the
# image's aarch64 code to assembly, cuts resolve_row's body out of the .s
# file and reads the debug `.loc` lines to tell its parts apart:
#   - the aligned path (the source after the byte-wise fallback
#     `if ... { return; }`) holds no ldrb/ldrsb, and
#   - the function holds wide loads (`ldp`, or `ldr` into an x or q register).
# The byte-wise fallback for a row off the 4-byte boundary keeps its ldrb on
# purpose and is not counted. A build in which resolve_row has no symbol
# (inlined away) is "not done", not a pass.
#
# usage: tools/vivante/resolve-asm-check.sh [TARGET_DIR]
#        tools/vivante/resolve-asm-check.sh --check-asm FILE.s [SRC.rs]
# TARGET_DIR defaults to a directory under $TMPDIR (never /tmp). The second
# form only runs the extractor on an existing .s file (used by the selftest).
# Needs `rustup target add aarch64-unknown-none` for the build.
# Prints one verdict line: "resolve-asm-check: ok|FAIL|not done: ...".
# Exit 0 ok, 1 fail, 2 not done (no build, no code found).

set -u

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../.." && pwd)
src_rel=src/vivante/ds_cache.rs

# check_asm ASM SRC: prints the verdict line, returns 0/1/2.
check_asm() {
    local asm=$1 src=$2
    # Source lines of resolve_row: whole fn, and the aligned part after the
    # first `    }` that closes the fallback if-block.
    local first last split
    first=$(grep -n '^fn resolve_row' "$src" | head -1 | cut -d: -f1)
    [ -n "$first" ] || { echo "resolve-asm-check: not done: fn resolve_row not found in $src"; return 2; }
    last=$(awk -v s="$first" 'NR > s && /^}/ { print NR; exit }' "$src")
    split=$(awk -v s="$first" -v e="$last" 'NR > s && NR < e && /^    }$/ { print NR; exit }' "$src")
    [ -n "$last" ] && [ -n "$split" ] || { echo "resolve-asm-check: not done: cannot delimit resolve_row in $src"; return 2; }
    # The .file directive may come after the function, so find the number first.
    local fno
    fno=$(awk -v base="$(basename "$src")" '$1 == ".file" && index($0, base "\"") { print $2; exit }' "$asm")
    [ -n "$fno" ] || { echo "resolve-asm-check: not done: no debug .file entry for $(basename "$src") in $asm (no debuginfo?)"; return 2; }
    # The body: from the symbol's label to its .size line. Each instruction
    # takes the last .loc line of ds_cache.rs seen before it (sticky, so that
    # code inlined from core is charged to the line it was inlined at).
    awk -v lo="$((split + 1))" -v fno="$fno" '
        /^_R[A-Za-z0-9_]*8ds_cache11resolve_row:/ { inb = 1; next }
        inb && $1 == ".size" { exit }
        !inb { next }
        $1 == ".loc" { if ($2 == fno) l = $3; next }
        /^\t[a-z]/ {
            n++
            op = $1
            if (op ~ /^ld[u]?r[s]?b$/) { if (l >= lo) { nb++; if (!ex) ex = $0 } else fb++ }
            else if (op == "ldp" || ((op == "ldr" || op == "ldur") && $0 ~ /\t[qx][0-9]+,/)) w++
        }
        END {
            if (!n) { print "NONE"; exit }
            printf "%s %d %d %d %d %s\n", (nb ? "BYTE" : (w ? "OK" : "NOWIDE")), n, nb + 0, w + 0, fb + 0, ex
        }' "$asm" | {
        read -r kind n nb w fb ex
        case $kind in
        OK)     echo "resolve-asm-check: ok (resolve_row: $n instructions, $w wide loads, 0 ldrb in the aligned path, $fb in the byte-wise fallback)"; return 0 ;;
        BYTE)   echo "resolve-asm-check: FAIL (resolve_row aligned path has $nb ldrb, $w wide loads: $ex)"; return 1 ;;
        NOWIDE) echo "resolve-asm-check: FAIL (resolve_row has no ldp/ldr wide load, $n instructions)"; return 1 ;;
        *)      echo "resolve-asm-check: not done: no resolve_row function body in $asm (renamed, or inlined away?)"; return 2 ;;
        esac
    }
}

if [ "${1:-}" = "--check-asm" ]; then
    [ -r "${2:-}" ] || { echo "resolve-asm-check: not done: no .s file given"; exit 2; }
    check_asm "$2" "${3:-$root/$src_rel}"
    exit $?
fi

if [ -n "${1:-}" ]; then
    tdir=$1
else
    tdir=${TMPDIR:-}
    case $tdir in
    "" | /tmp | /tmp/* | /var/tmp | /var/tmp/*)
        echo "resolve-asm-check: not done: no TARGET_DIR argument and TMPDIR is unset or under /tmp"; exit 2 ;;
    esac
    tdir=$tdir/resolve-asm-check-target
fi

if ! command -v rustup >/dev/null 2>&1 || ! rustup target list --installed 2>/dev/null | grep -qx aarch64-unknown-none; then
    echo "resolve-asm-check: not done: target aarch64-unknown-none missing (rustup target add aarch64-unknown-none)"
    exit 2
fi

if ! (cd "$root" && CARGO_TARGET_DIR=$tdir cargo rustc -p yakogl --release --lib --no-default-features \
    --features osal-zephyr,gpu-vivante,shaderc-online,no-shader-table \
    --target aarch64-unknown-none -- --emit=asm) >&2; then
    echo "resolve-asm-check: not done: the aarch64 build failed"
    exit 2
fi
asm=
# cargo reruns rustc when the sources changed, so the newest .s is current.
for f in "$tdir"/aarch64-unknown-none/release/deps/yakogl-*.s; do
    [ -f "$f" ] && { [ -z "$asm" ] || [ "$f" -nt "$asm" ]; } && asm=$f
done
[ -n "$asm" ] || { echo "resolve-asm-check: not done: no yakogl .s file under $tdir"; exit 2; }
check_asm "$asm" "$root/$src_rel"
