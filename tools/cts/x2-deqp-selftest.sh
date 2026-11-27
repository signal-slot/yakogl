#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/x2-deqp-selftest.sh
#
# Host-only check that tools/board.sh x2 runs and tallies a whole dEQP-GLES2
# tree on the Armadillo-X2 (the stand-in for the Verdin iMX8MP, same SoC and
# GC7000UL), without a board:
#   1. bash -n tools/board.sh;
#   2. the dry run of the whole tree (TREE_IMAGE, tools/cts/imx8mp-full-tree.txt,
#      BOARD_UART set) into records/x2/<YYYYMMDD-HHMM>-deqp-gles2-<sha8>: all
#      29 entries boot with their selector, each captures the console, the last
#      line names the x2 tally, no sudo, nothing runs (stubs) or is written;
#   3. the tally of synthetic X2 tree records (three-word LPGPR records, one
#      console per row) prints Fail, NoResult, crashes and NotSupported with
#      the names against BOARD_ACCEPTED, and exits with the gate's status: a
#      clean tree (exit 0), one with a Fail, a Crash, a case without a result
#      and an unaccepted NotSupported (exit 1), and one row built from the
#      real tools/cts/gles2-mustpass.txt and es2-accepted.txt (the defaults);
#   4. no stage-1 block for an x2 GLES3 tree or another board's GLES2 tree.
# Needs cargo (yakogl-tools, built once as tools/board_selftest.sh does).
set -u
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
B=$REPO/tools/board.sh
D=$(mktemp -d "${TMPDIR:-.}/x2-deqp-selftest.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
pass=0
fail=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
# want NAME COND OUTPUT: COND evaluated; the output printed when it fails
want() { if eval "$2"; then ok "$1"; else bad "$1"; printf '%s\n' "$3" | tail -30 | sed 's/^/        /'; fi; }

unset BOARD_HOST BOARD_UART BOARD_BOOT SEL TREE_IMAGE TREE_IMAGE_DIR TREE_TAG TREE_ROWS TREE_PREV \
    BOARD_MUSTPASS BOARD_ACCEPTED RUN_BOUND RETURN_BOUND X2_STOP_APPS X2_STOP_CONTAINERS
(cd "$REPO" && cargo build -q -p yakogl-tools) || { echo "x2-deqp-selftest: cannot build yakogl-tools"; exit 2; }
td=$(cd "$REPO" && cargo metadata --no-deps --format-version 1 | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
export YAKOGL_TOOLS=$td/debug/yakogl-tools
[ -x "$YAKOGL_TOOLS" ] || { echo "x2-deqp-selftest: no $YAKOGL_TOOLS"; exit 2; }

echo "== 1. bash -n tools/board.sh"
if o=$(bash -n "$B" 2>&1); then ok "bash -n tools/board.sh"; else bad "bash -n tools/board.sh"; echo "$o"; fi

echo "== 2. dry run of the whole tree on the X2"
# a fake image: the AArch64 Image header, image_size 4 MiB, the WDOG1 marker
{
    head -c 16 /dev/zero
    printf '\x00\x00\x40\x00\x00\x00\x00\x00'
    head -c 32 /dev/zero
    printf 'ARMd'
    head -c 1000 /dev/zero
    printf 'gc_wdog_arm: armed\0*** Booting Zephyr OS build selftest ***\0'
} > "$D/x2.bin"
SHA8=$(sha256sum "$D/x2.bin" | cut -c1-8)
mkdir -p "$D/stub"
for c in ssh scp curl ping fuser flock stty sudo kexec nc socat; do
    printf '#!/bin/sh\necho "%s $*" >> "%s/touched"\nexit 99\n' "$c" "$D" > "$D/stub/$c"
    chmod +x "$D/stub/$c"
done
OUT=$D/records/x2/20261127-1200-deqp-gles2-abcd1234
out=$(TREE_IMAGE=$D/x2.bin BOARD_UART=bridge:40001 PATH="$D/stub:$PATH" \
    "$B" --dry-run x2 tree "$REPO/tools/cts/imx8mp-full-tree.txt" "$OUT" 2>&1)
rc=$?
rows=$(grep -vcE '^[[:space:]]*(#|$)' "$REPO/tools/cts/imx8mp-full-tree.txt")
want "dry run: rc 0, the table has 29 entries" '[ "$rc" = 0 ] && [ "$rows" = 29 ]' "$out"
want "dry run: every entry e00..e28 boots once with its selector" \
    '[ "$(grep -c "^=== e[0-9][0-9] (x2.bin, selector [0-9]*) " <<<"$out")" = 29 ] &&
     for i in $(seq 0 28); do grep -q "^=== $(printf e%02d "$i") (x2.bin, selector $i) " <<<"$out" || exit 1; done' "$out"
want "dry run: entry 28 writes its selector (w1 0x5345001c) into LPGPR2" 'grep -qF "0x5345001c" <<<"$out"' "$out"
want "dry run: each entry captures the console (BOARD_UART) into its run directory" \
    '[ "$(grep -c "capture bridge:40001 <rundir>/console.raw <rundir>/console.ts" <<<"$out")" = 29 ] &&
     [ "$(grep -c "^DRY host\\$ mkdir -p $OUT/[0-9]\{8\}-[0-9]\{4\}-e[0-9][0-9]-$SHA8 " <<<"$out")" = 29 ]' "$out"
want "dry run: the jump is kexec-dev's crash route, no sudo" \
    '[ "$(grep -c "kexec_zephyr .*--crash" <<<"$out")" = 29 ] && ! grep -q sudo <<<"$out"' "$out"
want "dry run: the last line names the x2 tally of the record directory" \
    'grep -q "^DRY host\$ tools/board.sh x2 tally $OUT$" <<<"$out"' "$out"
want "dry run: nothing written, no stub ran" '[ ! -e "$D/records" ] && [ ! -s "$D/touched" ]' "$out"

echo "== 3. the tally of X2 tree records"
P=dEQP-GLES2.functional
printf '%s\n' "$P.clip_control.errors  # not claimed" "$P.multisample.depth  # no multisample surface" > "$D/accepted.txt"
printf '%s\n' "$P.a.one" "$P.a.two" "$P.b.one" "$P.b.two" "$P.clip_control.errors" "$P.multisample.depth" > "$D/mustpass.txt"

# xrow REC NAME SEL STATUS_AND_CASE...: one finished X2 run of the table image:
# meta.txt as board.sh x2 run leaves it, the three-word LPGPR record (DEQP,
# no begun word), INDEX.txt, and the console's "deqp: <Status> <case>" lines
xrow() {
    local rec=$1 name=$2 sel=$3 d n=0 np=0 nf=0 nn=0 no=0 s c line
    shift 3
    d=$rec/20261127-1200-$name-$SHA8
    mkdir -p "$d"
    printf '%s\n' board=x2 boot=kexec-dev "name=$name" probe=0 back=1 lost=3 up=40 t_reboot=100 t_back=200 \
        "rec0=00000000 00000000 00000000" > "$d/meta.txt"
    {
        echo "12:00:00 deqp: selector $sel"
        echo "12:00:00 deqp: case filter '$P.*'"
        for line in "$@"; do
            s=${line%% *}
            c=${line#* }
            echo "12:00:01 deqp: $s $c"
            n=$((n + 1))
            case "$s" in Pass) np=$((np + 1)) ;; Fail) nf=$((nf + 1)) ;; NotSupported) nn=$((nn + 1)) ;; *) no=$((no + 1)) ;; esac
        done
        echo "12:00:09 deqp: done: $np passed, $nf failed, $nn not supported, 0 warnings, complete=1"
        echo "12:00:09 deqp: summary: $n cases begun, $n with a result, 0 without"
    } > "$d/console.txt"
    printf '594b0801 %04x%04x %04x%04x\n' "$nf" "$np" "$no" "$nn" > "$d/snvs.txt"
    : > "$d/INDEX.txt"
}
# xrec REC FILTER0 FILTER1: tree.txt of a two-entry table image
xrec() {
    mkdir -p "$1"
    printf 'e00\t%s\t0\t%s\ne01\t%s\t1\t%s\n' "$D/x2.bin" "$2" "$D/x2.bin" "$3" > "$1/tree.txt"
}
NS1="NotSupported $P.clip_control.errors"
NS2="NotSupported $P.multisample.depth"
# tally BOARD REC: board.sh's tally with the fixture lists; sets out and rc
tally() { out=$(BOARD_ACCEPTED=$D/accepted.txt BOARD_MUSTPASS=$D/mustpass.txt "$B" "$1" tally "$2" 2>&1); rc=$?; }

R=$D/records/x2/20261127-1200-deqp-gles2-abcd1234
xrec "$R" "$P.a.*" "$P.b.*,$P.clip_control.*,$P.multisample.*"
xrow "$R" e00 0 "Pass $P.a.one" "Pass $P.a.two"
xrow "$R" e01 1 "Pass $P.b.one" "Pass $P.b.two" "$NS1" "$NS2"
tally x2 "$R"
want "clean tree: exit 0, Fail 0, NoResult 0, crashes 0, NotSupported 2, all accepted, stage1: PASS" \
    '[ "$rc" = 0 ] && grep -qx "Fail 0" <<<"$out" && grep -qx "NoResult 0" <<<"$out" && grep -qx "crashes 0" <<<"$out" &&
     grep -qx "NotSupported 2" <<<"$out" && grep -qx "NotSupported names: all in es2-accepted.txt" <<<"$out" &&
     grep -qx "stage1: PASS" <<<"$out"' "$out"
want "clean tree: the per-run verdicts and the selector form's TALLY line come first" \
    'grep -q "^board.sh x2 e00 rc=0 verdict=done boot=kexec-dev DEV" <<<"$out" &&
     grep -q "^TALLY (2 entries): begun 6 passed 4 failed 0 notsup 2 other 0 noresult 0" <<<"$out" &&
     grep -q "^== stage 1 (tools/cts/es2-tree-gate.py; NotSupported against " <<<"$out"' "$out"

R2=$D/records/x2/20261127-1300-deqp-gles2-abcd1234
xrec "$R2" "$P.a.*" "$P.b.*,$P.clip_control.*,$P.multisample.*"
xrow "$R2" e00 0 "Pass $P.a.one" "Fail $P.a.two"
xrow "$R2" e01 1 "Crash $P.b.one" "NotSupported $P.b.two" "$NS2"
tally x2 "$R2"
want "bad tree: exit 1, Fail 1, NoResult 1, crashes 1, NotSupported 2, the unaccepted name, stage1: FAIL" \
    '[ "$rc" = 1 ] && grep -qx "Fail 1" <<<"$out" && grep -qx "  Fail: $P.a.two" <<<"$out" && grep -qx "NoResult 1" <<<"$out" &&
     grep -qx "crashes 1" <<<"$out" && grep -qx "  Crash: $P.b.one" <<<"$out" && grep -qx "NotSupported 2" <<<"$out" &&
     grep -qx "NotSupported names: not in es2-accepted.txt: $P.b.two" <<<"$out" && grep -q "^stage1: FAIL (" <<<"$out"' "$out"

# one row from the real lists, read with the defaults (no BOARD_ACCEPTED, no BOARD_MUSTPASS)
R3=$D/records/x2/20261127-1400-deqp-gles2-abcd1234
mkdir -p "$R3"
printf 'e00\t%s\t0\tdEQP-GLES2.*\n' "$D/x2.bin" > "$R3/tree.txt"
mapfile -t lines < <(python3 -I - "$REPO/tools/cts/gles2-mustpass.txt" "$REPO/tools/cts/es2-accepted.txt" <<'EOF'
import fnmatch, sys
def names(p):
    return [l.split("#", 1)[0].strip() for l in open(p, encoding="utf-8") if l.split("#", 1)[0].strip()]
acc = names(sys.argv[2])
for c in names(sys.argv[1]):
    print(("NotSupported " if any(fnmatch.fnmatchcase(c, p) for p in acc) else "Pass ") + c)
EOF
)
nacc=$(grep -c '^NotSupported ' < <(printf '%s\n' "${lines[@]}"))
xrow "$R3" e00 0 "${lines[@]}"
out=$("$B" x2 tally "$R3" 2>&1)
rc=$?
want "real lists ($((${#lines[@]})) mustpass cases, $nacc accepted): exit 0, NotSupported $nacc, all accepted, stage1: PASS" \
    '[ "$rc" = 0 ] && [ "$nacc" = 34 ] && grep -qx "Fail 0" <<<"$out" && grep -qx "NoResult 0" <<<"$out" &&
     grep -qx "crashes 0" <<<"$out" && grep -qx "NotSupported 34" <<<"$out" &&
     grep -qx "NotSupported names: all in es2-accepted.txt" <<<"$out" && grep -qx "stage1: PASS" <<<"$out"' "$out"
# a mustpass case missing from the console is NoResult
xrow "$R3" e00 0 "${lines[@]:1}"
out=$("$B" x2 tally "$R3" 2>&1)
rc=$?
want "real lists, the first mustpass case without a result line: exit 1, NoResult 1" \
    '[ "$rc" = 1 ] && grep -qx "NoResult 1" <<<"$out" && grep -q "^stage1: FAIL (.*NoResult 1" <<<"$out"' "$out"

echo "== 4. other trees and boards: no stage-1 block"
R4=$D/records/x2/20261127-1500-deqp-gles3-abcd1234
xrec "$R4" "dEQP-GLES3.functional.a.*" "dEQP-GLES3.functional.b.*"
xrow "$R4" e00 0 "Pass $P.a.one"
xrow "$R4" e01 1 "Pass $P.b.one"
tally x2 "$R4"
want "x2 GLES3 tree: no stage-1 block, exit 0" '[ "$rc" = 0 ] && ! grep -q "^== stage 1\|^stage1:" <<<"$out" && grep -q "^TALLY" <<<"$out"' "$out"
sed -i 's/^board=x2$/board=imx8mp/; s/^boot=kexec-dev$/boot=uboot-tftp/' "$R"/*/meta.txt
printf '594b0801 00000002 00000000 ffff0002\n' > "$R/20261127-1200-e00-$SHA8/snvs.txt"
printf '594b0801 00000002 00000002 ffff0004\n' > "$R/20261127-1200-e01-$SHA8/snvs.txt"
tally imx8mp "$R"
want "imx8mp GLES2 tree: unchanged, no stage-1 block, exit 0" \
    '[ "$rc" = 0 ] && ! grep -q "^== stage 1\|^stage1:" <<<"$out" && grep -q "^TALLY (2 entries)" <<<"$out"' "$out"

echo "x2-deqp-selftest: $pass ok, $fail failed"
[ "$fail" = 0 ]
