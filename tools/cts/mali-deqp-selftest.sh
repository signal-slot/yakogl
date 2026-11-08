#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of tools/cts/mali-deqp-run.sh, the NanoPC-T6 dEQP-GLES2
# runner, without a board: a fake ssh and scp on PATH run the board's side in
# a directory of this host, and a fake deqp-gles2 writes its .qpa from the
# fixtures in tools/cts/testdata/mali-deqp-run/ (README.md there). It covers a
# clean run, a crash mid-row with resume, a hang hitting the timeout, a panthor
# job-timeout line counted in resets, a missing "yakogl-mali: loaded" line
# (wrong-lib), lost ssh, a commit not on the main ref refused, and --dry-run
# making no ssh call (also bare, without --build and --out, with fakes that fail); that the slice keeps no host name or address;
# and the default 256x256 pbuffer.
#
#   tools/cts/mali-deqp-selftest.sh      prints "mali-deqp-selftest: N checks passed"
#
# Host only, about 20 s (the hang waits for a 2 s timeout).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
FIX=$HERE/testdata/mali-deqp-run
RUN=$HERE/mali-deqp-run.sh
EXPECT=32
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mali-deqp-selftest-XXXXXX")
# The fake board's dmesg -w of the lost-ssh case is never stopped by the
# runner (the board is gone): end every process still reading under $tmp.
# MALI_DEQP_SELFTEST_KEEP=1 keeps the directory and says where it is.
cleanup() {
  pkill -f -- "$tmp/" 2>/dev/null
  if [ "${MALI_DEQP_SELFTEST_KEEP-}" = 1 ]; then echo "mali-deqp-selftest: kept $tmp"; else rm -rf "$tmp"; fi
}
trap cleanup EXIT
passed=0 failed=0
check() { # <what> <command...>
  local what=$1
  shift
  if "$@" > /dev/null 2>&1; then
    passed=$((passed + 1))
    echo "  ok    $what"
  else
    failed=$((failed + 1))
    echo "  FAIL  $what"
  fi
}

# The fakes, executable whatever the checkout's modes.
mkdir -p "$tmp/bin" "$tmp/board-bin"
cp "$FIX"/bin/* "$tmp/bin/"
cp "$FIX"/board-bin/* "$tmp/board-bin/"
chmod +x "$tmp"/bin/* "$tmp"/board-bin/*

# A repository whose main has commit c1 and whose side branch has c2.
g() { git -C "$tmp/repo" -c user.name=selftest -c user.email=selftest -c commit.gpgsign=false "$@"; }
mkdir -p "$tmp/repo"
git -C "$tmp/repo" init -q
g commit -q --allow-empty -m c1
g branch -f main HEAD
c1=$(g rev-parse HEAD)
g checkout -q -b side
g commit -q --allow-empty -m c2
c2=$(g rev-parse HEAD)

# run <name> <scenario> <rows> [VAR=value...]: one runner run on a fresh fake
# board; the exit status goes to $tmp/<name>/rc, the output to .../out.txt.
run() {
  local name=$1 scen=$2 rows=$3 d=$tmp/$1
  shift 3
  mkdir -p "$d/board/root" "$d/build"
  cp -r "$FIX/fixture-build/." "$d/build/"
  chmod +x "$d/build/deqp-gles2"
  sed -i "s/^commit=.*/commit=${COMMIT:-$c1}/" "$d/build/build.txt"
  printf '[    0.000000] Booting Linux on physical CPU 0x0\n' > "$d/board/kmsg"
  echo 5a1e0000-0000-4000-8000-00000000b007 > "$d/board/boot_id"
  : > "$d/ssh.log"
  env PATH="$tmp/bin:$PATH" BOARD_HOST=root@board.invalid BOARD_LOCK="$tmp/board.lock" \
    FAKE_BOARD="$d/board" FAKE_BOARD_BIN="$tmp/board-bin" FAKE_LOG="$d/ssh.log" \
    FAKE_FIXTURES="$FIX" FAKE_SCENARIO="$FIX/scenarios/$scen.tsv" \
    MALI_DEQP_MAIN_REPO="$tmp/repo" MALI_DEQP_MAIN_REF=main RAW_DIR="$d/raw" \
    KMSG_SETTLE_S=0.3 STAGE_MARGIN_KB=0 HOLD_MAX=300 "$@" \
    bash "$RUN" --build "$d/build" --out "$d/slice" --rows "$rows" --tree "$FIX/tree.txt" \
    --session smoke > "$d/out.txt" 2>&1
  echo $? > "$d/rc"
}
rc_is() { [ "$(cat "$tmp/$1/rc")" = "$2" ]; }
# col <name> <file> <key column value> <column number>: one field of a slice table
col() { awk -F'\t' -v k="$3" -v c="$4" '$1 == k || $2 == k { print $c; exit }' "$tmp/$1/slice/$2"; }
status_of() { [ "$(col "$1" results.tsv "$2" 2)	$(col "$1" results.tsv "$2" 3)" = "$3" ]; }
count() { [ "$(awk -F'\t' -v s="$2" 'NR > 1 && $2 "\t" $3 ~ s' "$tmp/$1/slice/results.tsv" | wc -l)" = "$3" ]; }
clean_slice() { ! grep -rqiE 'board.invalid|198\.51\.100|02:00:5e' "$tmp/$1/slice"; }

echo "mali-deqp-selftest: clean run over rows 1-3"
run clean clean 1-3
check "a clean run exits 0" rc_is clean 0
check "chunks.tsv has the shared header" \
  bash -c "head -1 '$tmp/clean/slice/chunks.tsv' | grep -qxP 'row\tlabel\tcases\tstatus\tseconds\tkmsg\tresets\tssh_after\tboot_same\texit_line'"
check "every chunk is done, 0 resets, ssh_after yes, boot_same yes" \
  bash -c "[ \"\$(awk -F'\t' 'NR > 1 { print \$2, \$3, \$4, \$7, \$8, \$9 }' '$tmp/clean/slice/chunks.tsv' | paste -sd,)\" = 'T1 6 done 0 yes yes,T2 6 done 0 yes yes,T3 5 done 0 yes yes' ]"
check "results.tsv has its header and all 17 cases: 16 Pass, 1 Fail" \
  bash -c "head -1 '$tmp/clean/slice/results.tsv' | grep -qxP 'case\tstatus\tdetail' && [ \$(wc -l < '$tmp/clean/slice/results.tsv') = 18 ] && [ \$(grep -cP '\tPass\t' '$tmp/clean/slice/results.tsv') = 16 ]"
check "meta.txt has the ten keys in order, the full commit and both boot_ids" \
  bash -c "[ \"\$(cut -d= -f1 '$tmp/clean/slice/meta.txt' | paste -sd' ')\" = 'commit session rows tree boot_id_before boot_id_after kernel lib started ended' ] &&
    grep -qx 'commit=$c1' '$tmp/clean/slice/meta.txt' && grep -qx 'session=smoke' '$tmp/clean/slice/meta.txt' &&
    grep -qx 'rows=1-3' '$tmp/clean/slice/meta.txt' && grep -qx 'kernel=6.18.55-current-rockchip64' '$tmp/clean/slice/meta.txt' &&
    grep -qx 'boot_id_after=5a1e0000-0000-4000-8000-00000000b007' '$tmp/clean/slice/meta.txt'"
check "the yakogl-mali-exit line is kept, redacted" \
  bash -c "awk -F'\t' 'NR == 2 { print \$10 }' '$tmp/clean/slice/chunks.tsv' | grep -qx 'yakogl-mali-exit: status=0 on <host> <addr>'"
check "the Fail's detail is kept, its host, address and MAC redacted" \
  status_of clean dEQP-GLES2.functional.flush_finish.wait "Fail	Image comparison failed on <host> at <addr> & <mac>"
check "kmsg/<label>.txt for each row, empty for a quiet chunk" \
  bash -c "for l in T1 T2 T3; do [ -f '$tmp/clean/slice/kmsg/'\$l.txt ] && [ ! -s '$tmp/clean/slice/kmsg/'\$l.txt ] || exit 1; done"
check "the stage is deleted from the board at the end" \
  bash -c "[ -d '$tmp/clean/board/root/yakogl-mali' ] && [ -z \"\$(ls -A '$tmp/clean/board/root/yakogl-mali')\" ]"
check "the run staged only under /root/yakogl-mali/<sha8>/" \
  bash -c "grep '^scp' '$tmp/clean/ssh.log' | grep -o 'board.invalid:[^ ]*' | grep -v '^board.invalid:/root/yakogl-mali/${c1:0:8}/' | grep -q . && exit 1; grep -q 'board.invalid:/root/yakogl-mali/${c1:0:8}/' '$tmp/clean/ssh.log'"
check "the slice keeps no host name, address or MAC" clean_slice clean
check "deqp-gles2 draws on a 256x256 pbuffer by default (clipping needs 200x200)" \
  bash -c "grep -q -- '--deqp-surface-type=pbuffer --deqp-surface-width=256 --deqp-surface-height=256 ' '$tmp/clean/ssh.log'"
check "the board lock is released (its record emptied)" bash -c "[ ! -s '$tmp/board.lock' ]"

echo "mali-deqp-selftest: a crash inside info.renderer, row 1"
run crash crash 1-1
check "a crash mid-row: chunk status crash, exit 0" \
  bash -c "rc_is() { [ \"\$(cat '$tmp/crash/rc')\" = 0 ]; }; rc_is && [ \"\$(awk -F'\t' 'NR == 2 { print \$4 }' '$tmp/crash/slice/chunks.tsv')\" = crash ]"
check "the crashed case is Lost chunk-crash" status_of crash dEQP-GLES2.info.renderer "Lost	chunk-crash"
check "the row resumes after it: the 5 other cases Pass, in a second process" \
  bash -c "[ \$(grep -cP '\tPass\t' '$tmp/crash/slice/results.tsv') = 5 ] && [ -s '$tmp/crash/raw/qpa/T1.r1.qpa' ] && [ ! -e '$tmp/crash/raw/qpa/T1.r2.qpa' ]"

echo "mali-deqp-selftest: a hang inside info.vendor, row 1 (CHUNK_LIMIT=2)"
run hang hang 1-1 CHUNK_LIMIT=2
check "a hang: chunk status hang, the case Lost chunk-hang" \
  bash -c "[ \"\$(awk -F'\t' 'NR == 2 { print \$4 }' '$tmp/hang/slice/chunks.tsv')\" = hang ]"
check "the hung case is Lost chunk-hang, the case after it Pass" \
  bash -c "grep -qxP 'dEQP-GLES2.info.vendor\tLost\tchunk-hang' '$tmp/hang/slice/results.tsv' && grep -qxP 'dEQP-GLES2.info.version\tPass\tPass' '$tmp/hang/slice/results.tsv'"

echo "mali-deqp-selftest: a panthor job timeout during row 2"
run jt jobtimeout 2-2
check "a job-timeout line counts in resets: status fault, kmsg 2, resets 1" \
  bash -c "[ \"\$(awk -F'\t' 'NR == 2 { print \$4, \$6, \$7 }' '$tmp/jt/slice/chunks.tsv')\" = 'fault 2 1' ]"
check "kmsg/T2.txt holds the panthor line, its address redacted" \
  bash -c "grep -q 'panthor fb000000.gpu: \[drm\] job timeout' '$tmp/jt/slice/kmsg/T2.txt' && grep -q 'peer <addr>' '$tmp/jt/slice/kmsg/T2.txt' && ! grep -q 198.51 '$tmp/jt/slice/kmsg/T2.txt'"

echo "mali-deqp-selftest: no 'yakogl-mali: loaded' line, rows 1-2"
run wl wronglib 1-2
check "wrong-lib: exit 9, row 1 lost and all its cases Lost wrong-lib, row 2 Lost not-run" \
  bash -c "[ \"\$(cat '$tmp/wl/rc')\" = 9 ] && [ \"\$(awk -F'\t' 'NR > 1 { print \$4 }' '$tmp/wl/slice/chunks.tsv' | paste -sd,)\" = lost,lost ] &&
    [ \$(grep -cP '\tLost\twrong-lib$' '$tmp/wl/slice/results.tsv') = 6 ] && [ \$(grep -cP '\tLost\tnot-run$' '$tmp/wl/slice/results.tsv') = 6 ]"

echo "mali-deqp-selftest: the board goes away inside row 2 of 1-3"
run lost lostssh 1-3
check "lost ssh: exit 8, the board reported as needing a person" \
  bash -c "[ \"\$(cat '$tmp/lost/rc')\" = 8 ] && grep -q 'needs a person' '$tmp/lost/out.txt'"
check "row 1 done; rows 2 and 3 lost with ssh_after no; their 11 cases Lost board-lost" \
  bash -c "[ \"\$(awk -F'\t' 'NR > 1 { print \$4, \$8 }' '$tmp/lost/slice/chunks.tsv' | paste -sd,)\" = 'done yes,lost no,lost no' ] &&
    [ \$(grep -cP '\tLost\tboard-lost$' '$tmp/lost/slice/results.tsv') = 11 ] && [ \$(grep -cP '\tPass\t' '$tmp/lost/slice/results.tsv') = 6 ]"
check "after the loss, one probe and no retry: two calls from the lost process on" \
  bash -c "[ \$(awk '/^ssh .*T2\.r0\.qpa/ { f = 1 } f' '$tmp/lost/ssh.log' | wc -l) = 2 ]"

echo "mali-deqp-selftest: a build from a commit not on main"
COMMIT=$c2 run side clean 1-1
check "a commit not on the main ref is refused (exit 6) before any ssh" \
  bash -c "[ \"\$(cat '$tmp/side/rc')\" = 6 ] && [ ! -s '$tmp/side/ssh.log' ] && grep -q 'is not on main' '$tmp/side/out.txt'"

echo "mali-deqp-selftest: --dry-run --rows 1-1 on the fixture build"
mkdir -p "$tmp/dry"
: > "$tmp/dry/ssh.log"
env PATH="$tmp/bin:$PATH" FAKE_LOG="$tmp/dry/ssh.log" FAKE_BOARD="$tmp/dry/none" \
  bash "$RUN" --dry-run --rows 1-1 --build "$FIX/fixture-build" --out "$tmp/dry/slice" --tree "$FIX/tree.txt" > "$tmp/dry/out.txt" 2>&1
echo $? > "$tmp/dry/rc"
check "--dry-run prints its plan (row, case count, the deqp command) and exits 0" \
  bash -c "[ \"\$(cat '$tmp/dry/rc')\" = 0 ] && grep -q 'row 1 T1: 6 case(s)' '$tmp/dry/out.txt' &&
    grep -q 'setsid -w timeout -k 10 <limit> env LD_LIBRARY_PATH=/root/yakogl-mali/009f62a5/lib' '$tmp/dry/out.txt'"
check "--dry-run makes no ssh or scp call and writes no slice" \
  bash -c "[ ! -s '$tmp/dry/ssh.log' ] && [ ! -e '$tmp/dry/slice' ]"

echo "mali-deqp-selftest: bare --dry-run (no --build, no --out), fakes that fail if called"
mkdir -p "$tmp/nobin" "$tmp/bare"
for t in ssh scp; do
  printf '#!/bin/bash\necho "%s $*" >> "%s/bare/called.log"\nexit 99\n' "$t" "$tmp" > "$tmp/nobin/$t"
done
chmod +x "$tmp/nobin"/*
bare() { # <name> <args...>: a dry run in a scratch cwd; output to bare/<name>.txt, status to .rc
  local n=$1
  shift
  (cd "$tmp/bare" && env PATH="$tmp/nobin:$PATH" bash "$RUN" "$@" > "$tmp/bare/$n.txt" 2>&1; echo $? > "$tmp/bare/$n.rc")
}
bare a --dry-run --rows 1-1 --tree "$FIX/tree.txt"
check "--dry-run --rows 1-1 --tree T without --build/--out exits 0 and prints the plan" \
  bash -c "[ \"\$(cat '$tmp/bare/a.rc')\" = 0 ] && grep -q 'build: not given' '$tmp/bare/a.txt' &&
    grep -q 'skipped (no build)' '$tmp/bare/a.txt' && grep -q 'row 1 T1: 6 case(s)' '$tmp/bare/a.txt' &&
    grep -q 'slice .*build/mali-deqp-run/adhoc-rows1-1,' '$tmp/bare/a.txt'"
MALI_DEQP_TREE=$FIX/tree.txt bare b --dry-run --rows 1-1
check "the bare form takes its tree from MALI_DEQP_TREE" \
  bash -c "[ \"\$(cat '$tmp/bare/b.rc')\" = 0 ] && grep -q 'row 1 T1: 6 case(s)' '$tmp/bare/b.txt'"
MALI_DEQP_TREE=$tmp/bare/no-such-tree.txt bare c --dry-run --rows 1-1
check "a missing default tree is said so and exits 1" \
  bash -c "[ \"\$(cat '$tmp/bare/c.rc')\" = 1 ] && grep -q 'no tree table' '$tmp/bare/c.txt'"
check "a bare dry run never calls ssh or scp and creates nothing" \
  bash -c "[ ! -e '$tmp/bare/called.log' ] && [ -z \"\$(ls -A '$tmp/bare' | grep -v '^[abc]\.')\" ]"
bare d --rows 1-1 --tree "$FIX/tree.txt"
check "a real run without --build or --out still exits 1 (usage)" \
  bash -c "[ \"\$(cat '$tmp/bare/d.rc')\" = 1 ]"

total=$((passed + failed))
if [ "$failed" -ne 0 ] || [ "$total" -ne "$EXPECT" ]; then
  echo "mali-deqp-selftest: $failed of $total checks failed (the self-test pins $EXPECT checks)" >&2
  for d in "$tmp"/*/out.txt; do echo "--- $d"; tail -20 "$d"; done >&2
  exit 1
fi
echo "mali-deqp-selftest: $passed checks passed"
