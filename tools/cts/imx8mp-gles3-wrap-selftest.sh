#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx8mp-gles3-board.sh (usage errors, image choice, argument hand-off to tools/board.sh, for the Verdin
# and for the Armadillo-X2: --board x2 / IMX8MP_BOARD=x2, $IMX8MP_OUT/x2/es3-* images, board.sh x2) and of
# check-imx8mp-gles3-tree.py (passes on the committed table, fails on broken copies of it). The wrapper runs
# from a copy under a temporary ROOT whose tools/board.sh is a stub, so the real board.sh is never called.
# Host only; writes under ${TMPDIR:-$PWD}.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-$PWD}/gles3-wrap-selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
nfail=0 nok=0
ok() { echo "ok: $*"; nok=$((nok + 1)); }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }

# ---- (A) imx8mp-gles3-board.sh under a fake ROOT
root=$tmp/root
mkdir -p "$root/tools/cts"
cp "$here/imx8mp-gles3-board.sh" "$here/imx8mp-gles3-tree.txt" "$root/tools/cts/"
cat > "$root/tools/board.sh" <<'STUB'
#!/bin/bash
echo "ARGS: $*"
echo "IMAGE: ${TREE_IMAGE-<unset>}"
STUB
chmod +x "$root/tools/board.sh" "$root/tools/cts/imx8mp-gles3-board.sh"
w=$root/tools/cts/imx8mp-gles3-board.sh
outdir=$tmp/outdir

usage_err() { # description, 1 if exit 1 is a known mismatch, args...
	local what=$1 known=$2 rc; shift 2
	env -u TREE_IMAGE -u IMX8MP_OUT "$w" "$@" > "$tmp/o" 2> "$tmp/e"; rc=$?
	if [ $rc -eq 2 ] && grep -q 'usage' "$tmp/e"; then ok "$what: exit 2 and usage line"
	elif [ "$known" = 1 ] && [ $rc -eq 1 ] && grep -q 'usage' "$tmp/e"; then
		ok "$what: EXPECTED-MISMATCH: exit 1 (the :? parameter expansion), not 2; usage line printed"
	else bad "$what: exit $rc, stderr: $(cat "$tmp/e")"; fi
}
usage_err "no verb" 1
usage_err "unknown verb" 0 bogus "$outdir"
usage_err "tree without outdir" 1 tree
usage_err "tally without outdir" 1 tally
usage_err "--dry-run alone" 1 --dry-run

# no image under IMX8MP_OUT
mkdir -p "$tmp/empty/es3-none/zephyr"
env -u TREE_IMAGE IMX8MP_OUT="$tmp/empty" "$w" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 2 ] && grep -q 'no es3-\* image' "$tmp/e"; then ok "no image: exit 2, 'no es3-* image'"
else bad "no image: exit $rc, stderr: $(cat "$tmp/e")"; fi

# neither TREE_IMAGE nor IMX8MP_OUT
env -u TREE_IMAGE -u IMX8MP_OUT "$w" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -ne 0 ] && grep -q IMX8MP_OUT "$tmp/e"; then ok "no TREE_IMAGE and no IMX8MP_OUT: exit $rc naming IMX8MP_OUT"
else bad "no TREE_IMAGE and no IMX8MP_OUT: exit $rc, stderr: $(cat "$tmp/e")"; fi

# newest image wins
out=$tmp/imgs
mkdir -p "$out/es3-aaaaaaaa/zephyr" "$out/es3-bbbbbbbb/zephyr"
: > "$out/es3-aaaaaaaa/zephyr/zephyr.bin"; : > "$out/es3-bbbbbbbb/zephyr/zephyr.bin"
touch -d '2026-01-01 00:00:00' "$out/es3-aaaaaaaa/zephyr/zephyr.bin"
touch -d '2026-02-01 00:00:00' "$out/es3-bbbbbbbb/zephyr/zephyr.bin"
want_args="ARGS: --dry-run imx8mp tree $root/tools/cts/imx8mp-gles3-tree.txt $outdir"
env -u TREE_IMAGE IMX8MP_OUT="$out" "$w" --dry-run tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "$want_args" "$tmp/o" && grep -qxF "IMAGE: $out/es3-bbbbbbbb/zephyr/zephyr.bin" "$tmp/o"; then
	ok "newest es3-* image (bbbbbbbb) chosen, board.sh got '${want_args#ARGS: }'"
else bad "newest image: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
touch -d '2026-03-01 00:00:00' "$out/es3-aaaaaaaa/zephyr/zephyr.bin"
env -u TREE_IMAGE IMX8MP_OUT="$out" "$w" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "ARGS: imx8mp tree $root/tools/cts/imx8mp-gles3-tree.txt $outdir" "$tmp/o" \
	&& grep -qxF "IMAGE: $out/es3-aaaaaaaa/zephyr/zephyr.bin" "$tmp/o"; then
	ok "after touching aaaaaaaa it is the newest and chosen; no --dry-run without the flag"
else bad "newest image after touch: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi

# TREE_IMAGE wins over IMX8MP_OUT, and works without IMX8MP_OUT
TREE_IMAGE=$tmp/mine.bin IMX8MP_OUT="$out" "$w" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "IMAGE: $tmp/mine.bin" "$tmp/o"; then ok "TREE_IMAGE is used over the es3-* images"
else bad "TREE_IMAGE: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
env -u IMX8MP_OUT TREE_IMAGE="$tmp/mine.bin" "$w" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "IMAGE: $tmp/mine.bin" "$tmp/o"; then ok "TREE_IMAGE alone (no IMX8MP_OUT) is enough"
else bad "TREE_IMAGE alone: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi

# tally
env -u TREE_IMAGE -u IMX8MP_OUT "$w" tally "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "ARGS: imx8mp tally $outdir" "$tmp/o"; then ok "tally <outdir> -> 'imx8mp tally <outdir>'"
else bad "tally: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi

# ---- (A2) the Armadillo-X2: --board x2 or IMX8MP_BOARD=x2 runs tools/board.sh x2 with $IMX8MP_OUT/x2/es3-* images;
# the Verdin default and the X2 images never stand in for each other
mkdir -p "$out/x2/es3-cccccccc/zephyr" "$out/x2/es3-dddddddd/zephyr"
: > "$out/x2/es3-cccccccc/zephyr/zephyr.bin"; : > "$out/x2/es3-dddddddd/zephyr/zephyr.bin"
touch -d '2026-01-15 00:00:00' "$out/x2/es3-cccccccc/zephyr/zephyr.bin"
touch -d '2026-01-20 00:00:00' "$out/x2/es3-dddddddd/zephyr/zephyr.bin"   # older than the Verdin's aaaaaaaa
xwant="ARGS: --dry-run x2 tree $root/tools/cts/imx8mp-gles3-tree.txt $outdir"
for how in "--board x2" "--board=x2" "IMX8MP_BOARD=x2"; do
	case $how in IMX8MP_BOARD=x2) e=(IMX8MP_BOARD=x2); f=() ;; *) e=(); read -ra f <<< "$how" ;; esac
	env -u TREE_IMAGE -u IMX8MP_BOARD "${e[@]}" IMX8MP_OUT="$out" "$w" --dry-run "${f[@]}" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
	if [ $rc -eq 0 ] && grep -qxF "$xwant" "$tmp/o" && grep -qxF "IMAGE: $out/x2/es3-dddddddd/zephyr/zephyr.bin" "$tmp/o" \
		&& grep -qxF "imx8mp-gles3-board: tools/board.sh --dry-run x2 tree tools/cts/imx8mp-gles3-tree.txt $outdir" "$tmp/e"; then
		ok "x2 ($how): newest x2/es3-* image (dddddddd, not the newer Verdin aaaaaaaa), board.sh got '${xwant#ARGS: }'"
	else bad "x2 ($how): exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
done
# the board flag wins over IMX8MP_BOARD
env -u TREE_IMAGE IMX8MP_BOARD=x2 IMX8MP_OUT="$out" "$w" --board imx8mp tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "ARGS: imx8mp tree $root/tools/cts/imx8mp-gles3-tree.txt $outdir" "$tmp/o" \
	&& grep -qxF "IMAGE: $out/es3-aaaaaaaa/zephyr/zephyr.bin" "$tmp/o"; then
	ok "--board imx8mp over IMX8MP_BOARD=x2: the Verdin's board.sh and image"
else bad "--board imx8mp over IMX8MP_BOARD=x2: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
# a newer X2 image leaves the Verdin default unchanged
touch -d '2026-04-01 00:00:00' "$out/x2/es3-cccccccc/zephyr/zephyr.bin"
env -u TREE_IMAGE -u IMX8MP_BOARD IMX8MP_OUT="$out" "$w" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "ARGS: imx8mp tree $root/tools/cts/imx8mp-gles3-tree.txt $outdir" "$tmp/o" \
	&& grep -qxF "IMAGE: $out/es3-aaaaaaaa/zephyr/zephyr.bin" "$tmp/o"; then
	ok "Verdin default: a newer X2 image is not picked up, board.sh imx8mp"
else bad "Verdin default beside a newer X2 image: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
env -u TREE_IMAGE -u IMX8MP_BOARD IMX8MP_OUT="$out" "$w" --board x2 tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "IMAGE: $out/x2/es3-cccccccc/zephyr/zephyr.bin" "$tmp/o"; then ok "x2: after touching cccccccc it is chosen"
else bad "x2 newest after touch: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
# x2 with Verdin images only
env -u TREE_IMAGE -u IMX8MP_BOARD IMX8MP_OUT="$tmp/empty" "$w" --board x2 tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 2 ] && grep -q 'no x2/es3-\* image' "$tmp/e" && [ ! -s "$tmp/o" ]; then ok "x2 without an X2 image: exit 2, 'no x2/es3-* image', board.sh not run"
else bad "x2 without an X2 image: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
mkdir -p "$tmp/vonly/es3-eeeeeeee/zephyr"; : > "$tmp/vonly/es3-eeeeeeee/zephyr/zephyr.bin"
env -u TREE_IMAGE -u IMX8MP_BOARD IMX8MP_OUT="$tmp/vonly" "$w" --board x2 tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 2 ] && [ ! -s "$tmp/o" ]; then ok "x2 beside a Verdin image only: exit 2, the Verdin image is never used"
else bad "x2 beside a Verdin image only: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
# TREE_IMAGE wins on x2 too
env -u IMX8MP_BOARD TREE_IMAGE=$tmp/mine.bin IMX8MP_OUT="$out" "$w" --board x2 tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "IMAGE: $tmp/mine.bin" "$tmp/o" && grep -q '^ARGS: x2 tree ' "$tmp/o"; then ok "x2: TREE_IMAGE is used over the x2/es3-* images"
else bad "x2 TREE_IMAGE: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
# tally
env -u TREE_IMAGE -u IMX8MP_OUT IMX8MP_BOARD=x2 "$w" tally "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -qxF "ARGS: x2 tally $outdir" "$tmp/o" && grep -qxF "imx8mp-gles3-board: tools/board.sh x2 tally $outdir" "$tmp/e"; then
	ok "x2 tally <outdir> -> 'x2 tally <outdir>'"
else bad "x2 tally: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
# unknown boards
for b in pi4 verdin ''; do
	env -u TREE_IMAGE -u IMX8MP_BOARD IMX8MP_OUT="$out" "$w" --board "$b" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
	if [ $rc -eq 2 ] && [ ! -s "$tmp/o" ]; then ok "--board '$b': exit 2, board.sh not run"
	else bad "--board '$b': exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
done
env -u TREE_IMAGE IMX8MP_BOARD=pi4 IMX8MP_OUT="$out" "$w" tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 2 ] && grep -q "is imx8mp or x2, not 'pi4'" "$tmp/e"; then ok "IMX8MP_BOARD=pi4: exit 2 naming the two boards"
else bad "IMX8MP_BOARD=pi4: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi

# ---- (A3) the X2 knobs for a host that reaches the board by ssh only: X2_PING (tools/cts/x2-ping/ping) and
# X2_KEEP_KEY (persist the live authorized_keys for the session, put the persisted copy back after board.sh). A fake
# ssh runs the board side under $tmp/board (paths /root, /live and /tmp mapped there); a fake persist_file copies
# the live file to the persisted one. The real ssh and the real board are never used.
mkdir -p "$root/tools/cts/x2-ping" "$tmp/bin"
cp "$here/x2-ping/ping" "$root/tools/cts/x2-ping/ping"; chmod +x "$root/tools/cts/x2-ping/ping"
fb=$tmp/board
cat > "$tmp/bin/ssh" <<STUB
#!/bin/bash
# fake ssh: FAKE_SSH=down (no route), denied (sshd up, key refused), else run the command under $fb
while [ \$# -gt 0 ]; do case \$1 in -o) shift 2 ;; -*) shift ;; *) break ;; esac; done
echo "\$1" >> "$tmp/ssh-dest"; shift
case \${FAKE_SSH:-up} in
	down) echo "ssh: connect to host: Connection timed out" >&2; exit 255 ;;
	denied) echo "root@BOARD: Permission denied (publickey)." >&2; exit 255 ;;
esac
[ -f "$tmp/ssh-fail-restore" ] && [[ \$* == *orig* ]] && { echo "Connection closed" >&2; exit 255; }
cmd=\$(printf '%s' "\$*" | sed -E "s#(^|[ ;=<>])/(root|live|tmp)/#\\1$fb/\\2/#g")
PATH="$tmp/bin:\$PATH" exec sh -c "\$cmd"
STUB
cat > "$tmp/bin/persist_file" <<STUB
#!/bin/sh
cp "\$1" "$fb/live/rootfs\${1#$fb}"
STUB
chmod +x "$tmp/bin/ssh" "$tmp/bin/persist_file"
pg=$root/tools/cts/x2-ping/ping
PATH="$tmp/bin:$PATH" FAKE_SSH=up "$pg" -c1 -W1 board > "$tmp/o" 2>&1 && [ ! -s "$tmp/o" ] && ok "x2-ping: sshd answers -> 0, nothing printed" \
	|| bad "x2-ping up: $(cat "$tmp/o")"
PATH="$tmp/bin:$PATH" FAKE_SSH=denied "$pg" -c1 -W1 board && ok "x2-ping: refused at authentication is an answer (Linux is up)" \
	|| bad "x2-ping: Permission denied not taken as up"
PATH="$tmp/bin:$PATH" FAKE_SSH=down "$pg" -c1 -W1 board && bad "x2-ping: a timeout taken as an answer" || ok "x2-ping: no route -> 1"
"$pg" -c1 -W1 > /dev/null 2>&1; [ $? = 2 ] && ok "x2-ping: no host -> 2" || bad "x2-ping: no host"
: > "$tmp/ssh-dest"
PATH="$tmp/bin:$PATH" BOARD_HOST=root@BOARD "$pg" -c1 -W1 BOARD; PATH="$tmp/bin:$PATH" BOARD_HOST=root@BOARD "$pg" other
[ "$(tr '\n' ' ' < "$tmp/ssh-dest")" = "root@BOARD other " ] && ok "x2-ping: BOARD_HOST's user for its own host only" \
	|| bad "x2-ping: destinations $(cat "$tmp/ssh-dest")"
# the stub board.sh says which ping it would run and what the persisted keys hold while it runs
cat > "$root/tools/board.sh" <<STUB
#!/bin/bash
echo "ARGS: \$*"
echo "PING: \$(command -v ping)"
[ -f "$fb/live/rootfs/root/.ssh/authorized_keys" ] && echo "PERSISTED-DURING: \$(grep -c . "$fb/live/rootfs/root/.ssh/authorized_keys")"
exit \${STUB_RC:-0}
STUB
chmod +x "$root/tools/board.sh"
xrun() { env -u IMX8MP_BOARD TMPDIR="$tmp" TREE_IMAGE=$tmp/mine.bin PATH="$tmp/bin:$PATH" BOARD_HOST=root@BOARD "$@"; }
xrun X2_PING=ssh "$w" --board x2 tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
[ $rc = 0 ] && grep -qxF "PING: $root/tools/cts/x2-ping/ping" "$tmp/o" && grep -q 'X2_PING=ssh' "$tmp/e" \
	&& ok "X2_PING=ssh: board.sh x2 runs with tools/cts/x2-ping/ping" || bad "X2_PING=ssh: exit $rc: $(cat "$tmp/o" "$tmp/e")"
xrun X2_PING=icmp "$w" --board x2 tree "$outdir" > "$tmp/o" 2> "$tmp/e"
grep -q "PING: $root/tools/cts/x2-ping/ping" "$tmp/o" && bad "X2_PING=icmp: the stand-in was used" || ok "X2_PING=icmp: PATH left alone"
xrun X2_PING=ssh "$w" --board imx8mp tree "$outdir" > "$tmp/o" 2> "$tmp/e"
grep -q "PING: $root/tools/cts/x2-ping/ping" "$tmp/o" && bad "X2_PING on the Verdin: the stand-in was used" \
	|| ok "X2_PING is not read for the Verdin"
xrun X2_PING=bogus "$w" --board x2 tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
[ $rc = 2 ] && [ ! -s "$tmp/o" ] && grep -q "auto, icmp or ssh, not 'bogus'" "$tmp/e" && ok "X2_PING=bogus: exit 2, board.sh not run" \
	|| bad "X2_PING=bogus: exit $rc: $(cat "$tmp/o" "$tmp/e")"
mkboard() {
	rm -rf "$fb"; mkdir -p "$fb/root/.ssh" "$fb/live/rootfs/root/.ssh" "$fb/tmp"
	printf 'ssh-ed25519 AAAAfarm1 node1\nssh-ed25519 AAAAfarm2 node2\n' > "$fb/live/rootfs/root/.ssh/authorized_keys"
	cp "$fb/live/rootfs/root/.ssh/authorized_keys" "$fb/root/.ssh/authorized_keys"
	sha0=$(sha256sum < "$fb/live/rootfs/root/.ssh/authorized_keys")
}
mkboard; echo 'no-port-forwarding ssh-ed25519 AAAAlease lease' >> "$fb/root/.ssh/authorized_keys"; live0=$(sha256sum < "$fb/root/.ssh/authorized_keys")
xrun X2_PING=icmp X2_KEEP_KEY=1 "$w" --board x2 tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc = 0 ] && grep -qxF 'PERSISTED-DURING: 3' "$tmp/o" && [ "$(sha256sum < "$fb/live/rootfs/root/.ssh/authorized_keys")" = "$sha0" ] \
	&& [ "$(sha256sum < "$fb/root/.ssh/authorized_keys")" = "$live0" ] && grep -q 'is back as it was' "$tmp/e" \
	&& [ -z "$(ls "$fb/tmp")" ]; then
	ok "X2_KEEP_KEY=1: the lease key persisted while board.sh ran; the persisted copy back byte for byte, the live file kept"
else bad "X2_KEEP_KEY=1: exit $rc: $(cat "$tmp/o" "$tmp/e")"; fi
mkboard; echo 'ssh-ed25519 AAAAlease lease' >> "$fb/root/.ssh/authorized_keys"
xrun X2_PING=icmp X2_KEEP_KEY=1 STUB_RC=5 "$w" --board x2 tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
[ $rc = 5 ] && [ "$(sha256sum < "$fb/live/rootfs/root/.ssh/authorized_keys")" = "$sha0" ] \
	&& ok "X2_KEEP_KEY=1: board.sh's rc 5 is kept and the copy still goes back" || bad "X2_KEEP_KEY rc 5: exit $rc: $(cat "$tmp/e")"
mkboard
xrun X2_PING=icmp X2_KEEP_KEY=1 "$w" --board x2 tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
[ $rc = 0 ] && grep -q 'nothing to keep' "$tmp/e" && [ "$(sha256sum < "$fb/live/rootfs/root/.ssh/authorized_keys")" = "$sha0" ] \
	&& ok "X2_KEEP_KEY=1 with every key persisted already: nothing written" || bad "X2_KEEP_KEY nothing to keep: exit $rc: $(cat "$tmp/e")"
mkboard; echo 'ssh-ed25519 AAAAlease lease' >> "$fb/root/.ssh/authorized_keys"; : > "$tmp/ssh-fail-restore"
xrun X2_PING=icmp X2_KEEP_KEY=1 "$w" --board x2 tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
rm -f "$tmp/ssh-fail-restore"
[ $rc = 7 ] && grep -q 'NOT restored' "$tmp/e" && grep -q 'persist_file' "$tmp/e" \
	&& ok "X2_KEEP_KEY=1: a restore that cannot run exits 7 and prints the undo" || bad "X2_KEEP_KEY failed restore: exit $rc: $(cat "$tmp/e")"
kept=$(ls -d "$tmp"/x2-keep-key.* 2>/dev/null)
[ -n "$kept" ] && [ "$(sha256sum < "$kept/persisted")" = "$sha0" ] \
	&& ok "X2_KEEP_KEY: after a failed restore the copy the undo names is kept" || bad "X2_KEEP_KEY: no copy kept after a failed restore"
rm -rf "$tmp"/x2-keep-key.*
xrun X2_PING=icmp X2_KEEP_KEY=1 "$w" --dry-run --board x2 tree "$outdir" > "$tmp/o" 2> "$tmp/e"; rc=$?
[ $rc = 0 ] && grep -q '^DRY board\$ cat /live/rootfs/root/.ssh/authorized_keys' "$tmp/e" && grep -q '^DRY board\$ the kept copy' "$tmp/e" \
	&& ok "X2_KEEP_KEY=1 --dry-run: the keep and restore steps printed, nothing run" || bad "X2_KEEP_KEY dry run: exit $rc: $(cat "$tmp/e")"
ls -d "$tmp"/x2-keep-key.* > /dev/null 2>&1 && bad "X2_KEEP_KEY: a kept-copy directory left behind after a restore" \
	|| ok "X2_KEEP_KEY: no kept-copy directory left behind"

# ---- (B) check-imx8mp-gles3-tree.py
chk=$here/check-imx8mp-gles3-tree.py
table=$here/imx8mp-gles3-tree.txt
must=$here/gles3-mustpass.txt
run_chk() { python3 "$chk" -q --table "$1" --mustpass "${2:-$must}" > "$tmp/c" 2>&1; }

if run_chk "$table" && tail -1 "$tmp/c" | grep -qx OK; then ok "checker passes on the committed table"
else bad "checker rejects the committed table: $(tail -3 "$tmp/c")"; fi

expect_fail() { # description, table, mustpass ('' = committed), text the output must hold
	if run_chk "$2" "${3:-$must}"; then bad "$1: checker passed"
	elif ! grep -q -- "$4" "$tmp/c"; then bad "$1: failed, but without '$4': $(tail -3 "$tmp/c")"
	else ok "$1: checker fails ($4)"; fi
}
grep -v '^#' "$table" | grep . > "$tmp/rows.txt"
nrows=$(wc -l < "$tmp/rows.txt")

sed '1d' "$tmp/rows.txt" > "$tmp/dropped.txt"
expect_fail "dropped row" "$tmp/dropped.txt" "" "matched by no entry"

cat "$tmp/rows.txt" <(sed -n '1p' "$tmp/rows.txt") > "$tmp/dupentry.txt"
expect_fail "duplicated row" "$tmp/dupentry.txt" "" "more than one entry"

first=$(sed -n '1p' "$tmp/rows.txt" | cut -d, -f1)
cat "$tmp/rows.txt" <(echo "$first") > "$tmp/duppat.txt"
expect_fail "pattern of row 0 repeated in a new row" "$tmp/duppat.txt" "" "more than one entry"

cat "$tmp/rows.txt" <(echo 'dEQP-GLES3.*') > "$tmp/overlap.txt"
expect_fail "row overlapping the others" "$tmp/overlap.txt" "" "more than one entry"

cat "$tmp/rows.txt" <(echo 'dEQP-GLES3.info.*,') > "$tmp/emptypat.txt"
expect_fail "empty pattern" "$tmp/emptypat.txt" "" "empty pattern"

cat "$tmp/rows.txt" <(head -c 2100 /dev/zero | tr '\0' x; echo) > "$tmp/long.txt"
expect_fail "entry over 2035 characters" "$tmp/long.txt" "" "characters (max 2035)"

cat "$tmp/rows.txt" <(seq 1 256 | sed 's/^/dEQP-GLES3.nosuch.case/') > "$tmp/many.txt"
expect_fail "more than 256 entries" "$tmp/many.txt" "" "(max 256)"

cat "$must" <(echo 'dEQP-GLES3.nosuch.group.case') > "$tmp/must-extra.txt"
expect_fail "mustpass case the table does not cover" "$table" "$tmp/must-extra.txt" "not matched: dEQP-GLES3.nosuch"

sed '1s/\*.*$/zzz/' "$tmp/rows.txt" > "$tmp/narrow.txt"
expect_fail "row narrowed so its cases go unmatched" "$tmp/narrow.txt" "" "matched by no entry"

# comments and blank lines are ignored
{ echo '# comment'; echo; cat "$tmp/rows.txt"; } > "$tmp/commented.txt"
if run_chk "$tmp/commented.txt"; then ok "comment and blank lines in the table are ignored ($nrows rows)"
else bad "comments rejected: $(tail -3 "$tmp/c")"; fi

echo "$nok ok, $nfail failed"
[ "$nfail" -eq 0 ]
