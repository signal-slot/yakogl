#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/x2-common.sh --commit <rev> [--dry-run]
#   tools/cts/x2-common.sh --self-test
#
# The i.MX8MP common set (GM = glmark2-es2 800x600, 33 scenes; GT = gl_test,
# tools/cts/common-tree.txt) on the Armadillo-X2, in the record format of
# `tools/cts/board-stage.sh imx8mp common`. The Verdin i.MX8MP is off the farm
# (farm rule of 2026-11-27): the X2 has the same SoC and GPU, and every record
# this script writes says it was measured there.
#
# What it does, in order, and stops at the first failure:
#   1. <rev> must be an ancestor of origin/main (X2C_MAIN_REF), else exit 3
#      before anything is built or touched. A short rev is resolved first.
#   2. a git worktree is checked out at the commit (X2C_WORK, default
#      build/x2-common under the checkout); images are built from it only,
#      never from the working tree: one west build for GM (glmark2, the
#      sample's default scenes) and one for GT (gl_test, default case list),
#      for armadillo_x2/mimx8ml8/a53 with CONFIG_GC_BOARD_WDOG_SELF_ARM=y.
#      An image without 'gc_wdog_arm: armed' is refused (exit 3).
#   3. the worktree's own tools/board.sh runs the rows (`board.sh x2 tree
#      common-tree.txt <outdir>`, TREE_IMAGE_DIR). That is the kexec-dev route:
#      board.sh stages the image and the record tools only under /tmp on the
#      board (a tmpfs, gone with the reset that ends the run), reads the record
#      through /dev/mem and the console (UART2). This script itself never
#      reaches the board: it writes nothing to /boot, U-Boot variables,
#      fsbla/metadata/fip partitions or the eMMC, never repartitions, never
#      loads or unloads a kernel module and never touches Tailscale or sshd.
#      Whatever stops containers (X2_STOP_CONTAINERS=1, with its UNDO line
#      printed first) is board.sh's, and only when the caller sets it.
#   4. the records are redacted (console-redact.py over every file, with the
#      login name of BOARD_HOST as --user), the marker is written, the per-scene
#      ms/frame table is made from the GM console, and common-tally.py's tally
#      is kept beside them.
#
# Records (gitignored, tools/cts/README.md):
#   records/imx8mp/common-<sha8>/                      what common-row.py and
#                                                      one-commit-docs.py read:
#     tree.txt, <YYYYMMDD-HHMM>-GM-<sha8>/, <...>-GT-<sha8>/ (meta.txt,
#     INDEX.txt, console.txt), measured-on, ms-per-scene.tsv, tally.txt,
#     x2-common.log
#   records/imx8mp/<YYYYMMDD-HHMM>-x2-common-<sha8>/   a dated copy of it (the
#                                                      directory a RESULTS row cites)
# measured-on holds the line 'measured-on: armadillo-x2' (then a note that it
# stands in for the Verdin i.MX8MP); it is in both directories. A record dir
# that exists is refused (move it away): a run never overwrites a record.
#
# Environment (nothing defaults to a machine, address or device):
#   ZEPHYR_WS     the west workspace (required)
#   GLMARK2_SRC   the glmark2 checkout of the sample (required)
#   BOARD_HOST    user@host of the X2 (required; read by board.sh)
#   BOARD_UART    the X2's console, as board.sh reads it (required: the record
#                 carries the console log and the per-scene figures from it)
#   X2_TOOLS      the directory of kexec_zephyr and snvs_lpgpr (board.sh's;
#                 default the worktree's driver/tools/kexec-zephyr; set, both are
#                 copied there, for a board.sh that reads only its checkout)
#   WEST_VENV     as the other cts scripts
#   X2C_MAIN_REF  the ref the commit must be on (default origin/main)
#   X2C_REPO      the checkout (default this script's)
#   X2C_WORK      where the worktree and the builds go
#   X2C_RECORDS   where records/ is (default <checkout>/records)
#   X2C_WEST, X2C_MODULES_SH, X2C_BOARD_SH  stubs for the self-test
#                 (X2C_BOARD_SH default: tools/board.sh of the worktree)
#
# --dry-run builds nothing and touches no board: it prints the commit, the
# directories and the commands. --self-test needs no board and no environment.
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=${X2C_REPO:-$(cd "$HERE/../.." && pwd)}
MAIN_REF=${X2C_MAIN_REF:-origin/main}
ZBOARD=armadillo_x2/mimx8ml8/a53
MARKER='gc_wdog_arm: armed'

die() { echo "x2-common: $1" >&2; exit "${2:-2}"; }
usage() { die "usage: tools/cts/x2-common.sh --commit <rev> [--dry-run] | --self-test"; }

# ---- the record steps (also what the self-test runs against stubs) -------------
# write_marker DIR
write_marker() {
    printf 'measured-on: armadillo-x2\nnote: i.MX 8M Plus, GC7000UL; stands in for the Verdin i.MX8MP (farm rule of 2026-11-27)\n' > "$1/measured-on"
}
# ms_table GMDIR > table: scene, ms/frame, fps (common-tally.py's glmark2_speed on console.txt)
ms_table() {
    python3 -I - "$HERE/common-tally.py" "$1/console.txt" << 'EOF'
import importlib.util, sys
spec = importlib.util.spec_from_file_location("common_tally", sys.argv[1])
m = importlib.util.module_from_spec(spec); sys.modules["common_tally"] = m; spec.loader.exec_module(m)
text = open(sys.argv[2], encoding="utf-8", errors="replace").read()
sp = m.glmark2_speed(text)
print("scene\tms/frame\tfps")
for s in sp["scenes"]:
    ms = "-" if s["ms"] is None else "%.2f" % s["ms"]
    fps = "-" if s["fps"] is None else str(s["fps"])
    print("%s\t%s\t%s" % (s["name"], ms, fps))
print("# score %s, %d scenes" % (sp["score"], len(sp["scenes"])))
EOF
}
# finish_record REC DATED: marker, redaction, ms table, tally, dated copy
finish_record() {
    local rec=$1 dated=$2 gm user=${BOARD_HOST%%@*}
    [ "$user" = "${BOARD_HOST:-}" ] && user=
    gm=$(find "$rec" -maxdepth 1 -type d -name '*-GM-*' | sort | tail -1)
    [ -n "$gm" ] || die "no GM run directory in $rec" 4
    [ -f "$gm/console.txt" ] || die "the GM run has no console.txt (BOARD_UART): the per-scene figures need it" 4
    find "$rec" -type f ! -name '*.jpg' -print0 | xargs -0 -r python3 -I "$HERE/console-redact.py" ${user:+--user "$user"} > /dev/null ||
        die "redaction failed in $rec" 4
    write_marker "$rec"
    ms_table "$gm" > "$rec/ms-per-scene.tsv" || die "cannot make the per-scene table" 4
    python3 -I "$HERE/common-tally.py" imx8mp "$rec" > "$rec/tally.txt" 2>&1
    echo "x2-common: tally rc=$? (tally.txt)"
    [ ! -e "$dated" ] || die "$dated exists" 4
    mkdir -p "$(dirname "$dated")" && cp -a "$rec" "$dated" || die "cannot write $dated" 4
}
# ping_shim DIR: board.sh's return check after the jump (kexec_wait_back) pings
# the host part of BOARD_HOST. A host that ssh reaches but ping does not (an
# ssh config alias, a forwarded port) never reads as back, so every row ended
# not_back after board.sh's 1500 s and the tree stopped before GT (run-x2-common,
# 2026-11-27). When that host does not answer one ping, DIR/ping goes first on
# PATH: it ignores its arguments and answers as one ssh login to BOARD_HOST does.
ping_shim() {
    local host=${BOARD_HOST#*@}
    ping -c1 -W1 "$host" > /dev/null 2>&1 && return 0
    mkdir -p "$1" || die "cannot create $1"
    printf '#!/bin/bash\n# x2-common ping shim: ssh stands in for ping\nexec timeout 10 ssh -o BatchMode=yes -o ConnectTimeout=5 "$X2C_PING_SSH" true < /dev/null > /dev/null 2>&1\n' > "$1/ping"
    chmod +x "$1/ping"
    export X2C_PING_SSH=$BOARD_HOST PATH=$1:$PATH
    echo "x2-common: the board host answers no ping: the return check asks ssh instead ($1/ping)"
}

# ---- the self-test ---------------------------------------------------------------
self_test() {
    local fail=0 T S R out rc sha
    ok() { printf '  ok    %s\n' "$1"; }
    bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
    T=$(mktemp -d "${TMPDIR:-/tmp}/x2common-self.XXXXXX") || exit 2
    trap 'rm -rf "$T"' EXIT
    S=$T/stub R=$T/repo
    mkdir -p "$S" "$R/tools/cts"
    # the repo: a main with one commit and a side branch commit not on main
    git -C "$R" init -q -b main && git -C "$R" config user.email t@t && git -C "$R" config user.name t
    cp "$HERE/common-tally.py" "$HERE/console-redact.py" "$R/tools/cts/"
    echo a > "$R/a"; git -C "$R" add -A; git -C "$R" commit -q -m one
    git -C "$R" update-ref refs/remotes/origin/main HEAD
    sha=$(git -C "$R" rev-parse HEAD)
    git -C "$R" checkout -q -b side; echo b > "$R/b"; git -C "$R" add b; git -C "$R" commit -q -m side
    local off; off=$(git -C "$R" rev-parse HEAD)
    git -C "$R" checkout -q main
    cat > "$S/board.sh" << 'EOF'
#!/bin/bash
# stub: x2 tree <table> <out>; writes what the real one does, with an address in it
echo "board.sh $*" >> "$STUB_LOG"
echo "ping-seen $(head -2 "$(command -v ping)" | tail -1) ssh-to=$X2C_PING_SSH" >> "$STUB_LOG"
out=$4; mkdir -p "$out"; echo rows > "$out/tree.txt"
for l in GM GT; do
    d=$out/20261128-1200-$l-abcd1234; mkdir -p "$d"
    echo "board=x2 board_measured=Armadillo-X2" > "$d/meta.txt"
    echo "verdict=done" > "$d/INDEX.txt"
    if [ $l = GM ]; then
        { echo "glmark2: score 100: 33 scenes ran, 0 failed, 0 unsupported"
          echo "[build] use-vbo=false: FPS: 50 FrameTime: 20.000 ms"
          echo "ssh root@$(printf '%s.%s.%s.%s' 192 0 2 9) /dev/tty$(printf USB3)"; } > "$d/console.txt"
    else echo "gl_test: all: 0 of 3 cases failed (3 passed; 0 timed out)" > "$d/console.txt"; fi
done
EOF
    cat > "$S/west" << 'EOF'
#!/bin/bash
echo "west $*" >> "$STUB_LOG"
d=; while [ $# -gt 0 ]; do [ "$1" = -d ] && d=$2; shift; done
mkdir -p "$d/zephyr"; echo "image ${STUB_MARK-gc_wdog_arm: armed}" > "$d/zephyr/zephyr.bin"
EOF
    printf '#!/bin/bash\necho stub-modules\n' > "$S/modules.sh"
    chmod +x "$S/board.sh" "$S/west" "$S/modules.sh"
    mkdir -p "$T/ws/zephyr" "$T/gm/src"
    echo loader > "$T/kexec_zephyr"; echo record > "$T/snvs_lpgpr"
    local -a E=(env STUB_LOG="$T/log" X2C_REPO="$R" X2C_WORK="$T/work" X2C_RECORDS="$T/records"
        X2C_WEST="$S/west" X2C_MODULES_SH="$S/modules.sh" X2C_BOARD_SH="$S/board.sh" X2_TOOLS="$T"
        ZEPHYR_WS="$T/ws" GLMARK2_SRC="$T/gm" BOARD_HOST=user@host.invalid BOARD_UART=uart.invalid:1)
    : > "$T/log"

    out=$("${E[@]}" bash "$HERE/x2-common.sh" 2>&1); rc=$?
    [ $rc = 2 ] && ok "no arguments: exit 2" || bad "no arguments: rc $rc"
    out=$("${E[@]}" bash "$HERE/x2-common.sh" --commit 2>&1); rc=$?
    [ $rc = 2 ] && ok "--commit without a value: exit 2" || bad "--commit without a value: rc $rc"
    out=$("${E[@]}" bash "$HERE/x2-common.sh" --bogus 2>&1); rc=$?
    [ $rc = 2 ] && ok "unknown option: exit 2" || bad "unknown option: rc $rc"
    out=$("${E[@]}" ZEPHYR_WS= bash "$HERE/x2-common.sh" --commit "$sha" 2>&1); rc=$?
    { [ $rc = 2 ] && [[ $out == *ZEPHYR_WS* ]]; } && ok "missing ZEPHYR_WS named: exit 2" || bad "missing ZEPHYR_WS: rc $rc: $out"
    out=$("${E[@]}" BOARD_UART= bash "$HERE/x2-common.sh" --commit "$sha" 2>&1); rc=$?
    { [ $rc = 2 ] && [[ $out == *BOARD_UART* ]]; } && ok "missing BOARD_UART named: exit 2" || bad "missing BOARD_UART: rc $rc: $out"
    out=$("${E[@]}" bash "$HERE/x2-common.sh" --commit "$off" 2>&1); rc=$?
    { [ $rc = 3 ] && [[ $out == *"not an ancestor"* ]]; } && ok "a commit off main is refused: exit 3" || bad "off-main: rc $rc: $out"
    [ ! -s "$T/log" ] && [ ! -e "$T/work" ] && ok "nothing built or run for the refused commit" || bad "refused commit left traces"
    out=$("${E[@]}" bash "$HERE/x2-common.sh" --commit nosuchrev 2>&1); rc=$?
    [ $rc = 3 ] && ok "an unknown rev is refused: exit 3" || bad "unknown rev: rc $rc"

    out=$("${E[@]}" bash "$HERE/x2-common.sh" --commit "$sha" --dry-run 2>&1); rc=$?
    { [ $rc = 0 ] && [[ $out == *"dry run"* ]] && [ ! -s "$T/log" ]; } && ok "--dry-run runs nothing" || bad "dry run: rc $rc: $out"

    out=$("${E[@]}" STUB_MARK=none bash "$HERE/x2-common.sh" --commit "$sha" 2>&1); rc=$?
    { [ $rc = 3 ] && [[ $out == *"watchdog marker"* ]] && ! grep -q board.sh "$T/log"; } && ok "an image without the watchdog marker is refused before the board" || bad "no marker: rc $rc: $out"
    rm -rf "$T/work" "$T/records"

    out=$("${E[@]}" bash "$HERE/x2-common.sh" --commit "$sha" 2>&1); rc=$?
    [ $rc = 0 ] || bad "full run: rc $rc: $out"
    local s8=${sha:0:8} rec=$T/records/imx8mp/common-${sha:0:8}
    [ -d "$rec" ] && ok "record directory common-<sha8>" || bad "no $rec"
    local dated; dated=$(ls -d "$T"/records/imx8mp/*-x2-common-"$s8" 2> /dev/null | head -1)
    [[ $dated =~ /[0-9]{8}-[0-9]{4}-x2-common-$s8$ ]] && ok "dated copy <YYYYMMDD-HHMM>-x2-common-<sha8>" || bad "no dated copy: $dated"
    ls -d "$rec"/*-GM-* "$rec"/*-GT-* > /dev/null 2>&1 && [ -f "$rec/tree.txt" ] && ok "layout: tree.txt, GM and GT run directories" || bad "layout"
    local d
    for d in "$rec" "$dated"; do
        grep -qx 'measured-on: armadillo-x2' "$d/measured-on" 2> /dev/null && ok "marker in ${d##*/}" || bad "marker in ${d##*/}"
    done
    grep -q '^\[build\]' "$rec/ms-per-scene.tsv" && grep -q 20.00 "$rec/ms-per-scene.tsv" && ok "ms-per-scene.tsv has the scene's ms/frame" || bad "ms table: $(cat "$rec/ms-per-scene.tsv" 2> /dev/null)"
    ! grep -rqE '192\.0\.2|ttyUSB' "$rec" && ok "console redacted" || bad "console not redacted"
    grep -q "x2 tree" "$T/log" && grep -q "armadillo_x2/mimx8ml8/a53" "$T/log" && ok "board.sh x2 tree, west for the X2 board" || bad "commands: $(cat "$T/log")"
    cmp -s "$T/kexec_zephyr" "$T/work/tree-$s8/driver/tools/kexec-zephyr/kexec_zephyr" && ok "X2_TOOLS' loader is copied into the worktree board.sh reads" || bad "X2_TOOLS not copied"
    grep -q "ping-seen # x2-common ping shim: ssh stands in for ping ssh-to=user@host.invalid" "$T/log" && ok "an unpingable board host: board.sh's return check pings through the ssh shim" || bad "ping shim: $(grep ping-seen "$T/log")"
    ! grep -qE '/boot|fsbla|fip-|metadata|mmcblk|modprobe|insmod|tailscale|sshd' "$T/log" && ok "no boot, eMMC, module, tailscale or sshd command" || bad "forbidden command in log"
    out=$("${E[@]}" bash "$HERE/x2-common.sh" --commit "$sha" 2>&1); rc=$?
    { [ $rc = 2 ] && [[ $out == *"exists"* ]]; } && ok "an existing record is never overwritten" || bad "overwrite: rc $rc: $out"
    if grep -rqE '[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+' "$HERE/x2-common.sh"; then bad "an address-like token in the script"; else ok "no address in the script"; fi
    [ $fail = 0 ] && echo "x2-common --self-test: all ok" || echo "x2-common --self-test: $fail failed"
    [ $fail = 0 ]
    exit $?
}

# ---- arguments --------------------------------------------------------------------
COMMIT= DRY=0
while [ $# -gt 0 ]; do
    case $1 in
    --self-test) [ $# -eq 1 ] || usage; self_test ;;
    --commit) [ $# -ge 2 ] || die "--commit needs a revision"; COMMIT=$2; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    -h | --help) sed -n '5,66p' "$0"; exit 0 ;;
    *) usage ;;
    esac
done
[ -n "$COMMIT" ] || usage
for v in ZEPHYR_WS GLMARK2_SRC BOARD_HOST BOARD_UART; do
    [ -n "${!v:-}" ] || die "set $v (the environment names the machine; see the header)"
done

# ---- the commit: an ancestor of origin/main, or refused -----------------------------
SHA=$(git -C "$REPO" rev-parse --verify -q "$COMMIT^{commit}") || die "refused: no commit $COMMIT in $REPO" 3
git -C "$REPO" rev-parse --verify -q "$MAIN_REF^{commit}" > /dev/null ||
    die "refused: no $MAIN_REF in $REPO, cannot show that $COMMIT is on it" 3
git -C "$REPO" merge-base --is-ancestor "$SHA" "$MAIN_REF" ||
    die "refused: $SHA is not an ancestor of $MAIN_REF (only images from a commit on main run on the board)" 3
S8=$(git -C "$REPO" rev-parse --short=8 "$SHA")
WORK=${X2C_WORK:-$REPO/build/x2-common}
RECS=${X2C_RECORDS:-$REPO/records}
REC=$RECS/imx8mp/common-$S8
DATED=$RECS/imx8mp/$(date +%Y%m%d-%H%M)-x2-common-$S8
WT=$WORK/tree-$S8
IMGS=$WORK/img-$S8
BOARD_SH=${X2C_BOARD_SH:-$WT/tools/board.sh}
WEST=${X2C_WEST:-west}

if [ "$DRY" = 1 ]; then
    echo "x2-common: dry run: $S8 ($SHA) is on $MAIN_REF"
    echo "  worktree $WT; images $IMGS/{GM,GT}; record $REC; dated copy $DATED"
    echo "  west build -b $ZBOARD glmark2 / gl_test (CONFIG_GC_BOARD_WDOG_SELF_ARM=y)"
    echo "  TREE_IMAGE_DIR=$IMGS board.sh x2 tree tools/cts/common-tree.txt $REC"
    exit 0
fi
[ ! -e "$REC" ] || die "$REC exists: a run never overwrites a record, move it away"

# ---- the worktree and the two images -----------------------------------------------
mkdir -p "$WORK" || die "cannot create $WORK"
git -C "$REPO" worktree prune
if [ ! -e "$WT" ]; then
    git -C "$REPO" worktree add --detach -q "$WT" "$SHA" || die "cannot check out $S8 into $WT"
fi
[ "$(git -C "$WT" rev-parse HEAD)" = "$SHA" ] || die "$WT is not at $S8: remove it and run again"
[ -z "$(git -C "$WT" status --porcelain --untracked-files=no)" ] || die "tracked changes in $WT: the image would not be $S8"
[ -n "${WEST_VENV:-}" ] && PATH=$WEST_VENV/bin:$PATH
export ZEPHYR_BASE=$ZEPHYR_WS/zephyr
MODS=$("${X2C_MODULES_SH:-$HERE/zephyr-modules.sh}" "$ZEPHYR_WS") || die "zephyr-modules.sh failed"
mkdir -p "$IMGS" || die "cannot create $IMGS"
build() { # LABEL SAMPLE
    local b=$IMGS/$1
    rm -rf "$b"
    echo "x2-common: west build $2 for $ZBOARD (log $b.log)"
    (cd "$ZEPHYR_WS" && "$WEST" build -p always -b "$ZBOARD" -d "$b" "$WT/driver/samples/zephyr/$2" -- \
        "-DZEPHYR_MODULES=$MODS" "-DYAKOGL_DIR=$WT" -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y \
        ${3:+"$3"}) > "$b.log" 2>&1 < /dev/null || die "build of $1 failed ($b.log)" 4
    [ -s "$b/zephyr/zephyr.bin" ] || die "build of $1 left no image ($b.log)" 4
    grep -aqF "$MARKER" "$b/zephyr/zephyr.bin" || die "refused: the $1 image has no watchdog marker '$MARKER'" 3
    echo "$1 rc=0 sha256=$(sha256sum "$b/zephyr/zephyr.bin" | cut -d' ' -f1) build=none" > "$b.built"
}
build GM glmark2 "-DGLMARK2_SRC=$GLMARK2_SRC"
build GT gl_test

# ---- the run -----------------------------------------------------------------------
# board.sh of the commit's own worktree; no stale selector, no tree diff, no
# watchdog override leaks in from the caller
TOOLS=${X2_TOOLS:-$WT/driver/tools/kexec-zephyr}
# a board.sh older than X2_TOOLS (dfabc888's) takes the two tools from its own
# checkout only: copy them there (untracked; the images are already built)
if [ "$TOOLS" != "$WT/driver/tools/kexec-zephyr" ]; then
    mkdir -p "$WT/driver/tools/kexec-zephyr" && cp "$TOOLS/kexec_zephyr" "$TOOLS/snvs_lpgpr" "$WT/driver/tools/kexec-zephyr/" ||
        die "cannot copy kexec_zephyr and snvs_lpgpr from X2_TOOLS into $WT"
fi
mkdir -p "$RECS/imx8mp" || die "cannot create $RECS/imx8mp"
ping_shim "$WORK/ping-shim"
echo "x2-common: board.sh x2 tree common-tree.txt $REC"
env -u TREE_PREV -u TREE_IMAGE -u SEL -u NETBOOT_NO_WATCHDOG TREE_IMAGE_DIR="$IMGS" X2_TOOLS="$TOOLS" \
    "$BOARD_SH" x2 tree "$WT/tools/cts/common-tree.txt" "$REC" < /dev/null 2>&1 | tee "$RECS/imx8mp/.x2-common-$S8.log"
rc=${PIPESTATUS[0]}
[ -d "$REC" ] && mv "$RECS/imx8mp/.x2-common-$S8.log" "$REC/x2-common.log"
[ "$rc" = 0 ] || { [ -d "$REC" ] && finish_record "$REC" "$DATED" > /dev/null 2>&1; die "board.sh x2 tree rc=$rc: a partial record, if any, is in $REC" "$rc"; }
finish_record "$REC" "$DATED"
echo "x2-common: record $REC"
echo "x2-common: dated copy $DATED"
echo "x2-common: measured on the Armadillo-X2 (stands in for the Verdin i.MX8MP)"
