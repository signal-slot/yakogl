#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of tools/cts/mali-glmark2-run.sh, the NanoPC-T6 glmark2-es2
# runner, without a board: a fake ssh and scp on PATH run the board's side in
# a directory of this host, and a fake glmark2-es2-mali does to each scene what
# a scenario of tools/cts/testdata/mali-glmark2-run/ says (README.md there).
# It covers a clean run (scenes.tsv, pictures.txt, kmsg/, the stage made and
# removed under the login's home, the 800x600 command), a GL error, out of GPU
# memory, a refused draw, a GPU fault with its panthor lines, a job timeout, a
# failed setup, a crash, a hang killed by the timeout, a result with only an
# FPS, a result line split from its header by the back end's stdout log (the
# board's shape), the board not coming back and a reboot (exit 5, no retry), a missing
# or foreign "yakogl-mali: loaded" line (refused, exit 3), a busy render
# node (exit 4), a commit not on the main ref or a dev build (refused before
# any ssh, run with --dev), --scenes from a file, and --dry-run touching
# nothing (also bare, with fakes that fail when called); that the run
# directory keeps no host name, address or MAC. And the panthor fault paths
# (v6.18 message formats, classified by mali-kmsg.py as it is): a page fault,
# CS_FATAL, CS_FAULT and a SIGTERM-proof job timeout, each followed by a
# probe of the device that passes; a fault that leaves the device dead (two
# failed probes: exit 5, "needs a person", the final ssh-reachability line,
# the rest not-run); a fault after which the board stops answering (no retry).
#
#   tools/cts/mali-glmark2-selftest.sh    prints "mali-glmark2-selftest: N checks passed"
#
# Host only, about 30 s (the hang waits for a 2 s timeout, the lost board for
# a 2 s bound). Its gate row in tools/check.sh is added by a separate item of
# plan #332 (gate-row), after #327's mali-gate-rows; this script does not add it.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
FIX=$HERE/testdata/mali-glmark2-run
RUN=$HERE/mali-glmark2-run.sh
EXPECT=89
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mali-glmark2-selftest-XXXXXX")
# MALI_GLMARK2_SELFTEST_KEEP=1 keeps the directory and says where it is.
cleanup() {
  pkill -f -- "$tmp/" 2>/dev/null
  if [ "${MALI_GLMARK2_SELFTEST_KEEP-}" = 1 ]; then echo "mali-glmark2-selftest: kept $tmp"; else rm -rf "$tmp"; fi
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

# run <name> <scenario> [runner args...]: one runner run on a fresh fake board
# (VAR=value words before the name go to env); the exit status goes to
# $tmp/<name>/rc, the output to .../out.txt.
run() {
  local name=$1 scen=$2 d=$tmp/$1
  shift 2
  mkdir -p "$d/board/home" "$d/build"
  cp -r "$FIX/fixture-build/." "$d/build/"
  chmod +x "$d/build/bin/glmark2-es2-mali"
  sed -i "s/^commit=.*/commit=${COMMIT:-$c1}/; s/^dev=.*/dev=${DEV:-no}/" "$d/build/BUILD-INFO"
  printf '[    0.000000] Booting Linux on physical CPU 0x0\n[    9.500000] panthor fb000000.gpu: [drm] clock rate = 1000000000\n' > "$d/board/kmsg"
  echo 5a1e0000-0000-4000-8000-00000000b007 > "$d/board/boot_id"
  [ -z "${BUSY-}" ] || touch "$d/board/busy"
  : > "$d/ssh.log"
  env PATH="$tmp/bin:$PATH" BOARD_HOST=selftest@board.invalid BOARD_LOCK="$tmp/board.lock" \
    FAKE_BOARD="$d/board" FAKE_BOARD_BIN="$tmp/board-bin" FAKE_LOG="$d/ssh.log" \
    FAKE_FIXTURES="$FIX" FAKE_SCENARIO="$FIX/scenarios/$scen.tsv" \
    MALI_GLMARK2_REPO="$tmp/repo" MALI_GLMARK2_MAIN_REF=main RAW_DIR="$d/raw" \
    BOARD_BACK_S=2 BOARD_POLL_S=0.5 KILL_GRACE_S=1 STAGE_MARGIN_KB=0 HOLD_MAX=300 \
    PROBE_PAUSE_S=0 PROBE_TIMEOUT_S=5 \
    bash "$RUN" "$@" "$d/build" "$d/out" > "$d/out.txt" 2>&1
  echo $? > "$d/rc"
}
rc_is() { [ "$(cat "$tmp/$1/rc")" = "$2" ]; }
# col <name> <row> <column>: one field of scenes.tsv (row 1 is the first scene)
col() { awk -F'\t' -v r="$(($2 + 1))" -v c="$3" 'NR == r { print $c }' "$tmp/$1/out/scenes.tsv"; }
verdicts() { [ "$(awk -F'\t' 'NR > 1 { print $3 }' "$tmp/$1/out/scenes.tsv" | paste -sd,)" = "$2" ]; }
clean_out() { ! grep -rqiE 'board\.invalid|198\.51\.100|02:00:5e|selftest@' "$tmp/$1/out"; }
stage_gone() { [ ! -e "$tmp/$1/board/home/yakogl-mali-glmark2" ]; }
calls() { [ "$(wc -l < "$tmp/$1/board/glmark2.calls")" = "$2" ]; }

echo "mali-glmark2-selftest: a clean run over scenes 1-3"
run clean clean --scenes 1-3
check "a clean run exits 0" rc_is clean 0
check "scenes.tsv has its header and three ok rows with the full commit" \
  bash -c "head -1 '$tmp/clean/out/scenes.tsv' | grep -qxP 'scene\tcommit\tverdict\tms_frame\tpicture\tkmsg\texit_line' &&
    [ \"\$(awk -F'\t' 'NR > 1 { print \$1, \$2, \$3 }' '$tmp/clean/out/scenes.tsv' | paste -sd,)\" = \
'build:use-vbo=false $c1 ok,build:use-vbo=true $c1 ok,texture:texture-filter=nearest $c1 ok' ]"
check "ms/frame is glmark2's FrameTime, kmsg 0, the exit line kept" \
  bash -c "[ '$(col clean 1 4) $(col clean 1 6)' = '0.800 0' ] &&
    [ '$(col clean 1 7)' = 'yakogl-mali-exit: draws=600 refused=0 first_refused=- gpu_faults=0 job_timeouts=0' ]"
check "the picture column holds the text after 'glmark2-picture: <info>: '" \
  bash -c "[ '$(col clean 2 5)' = '800x600; probes c 336699ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 1a2b, crc16q 3c4d; mean 51 102 153; black no' ]"
check "pictures.txt has one 'mali | <info> | <picture>' line a scene" \
  bash -c "[ \$(grep -c '^mali | ' '$tmp/clean/out/pictures.txt') = 3 ] &&
    grep -qx 'mali | \[texture\] texture-filter=nearest | 800x600; probes c 336699ff, .*; black no' '$tmp/clean/out/pictures.txt'"
check "the summary line counts 3/3 ok" \
  bash -c "grep -qx 'mali-glmark2: 3/3 scenes ok, failed 0, commit ${c1:0:8}' '$tmp/clean/out.txt' &&
    grep -qx 'mali-glmark2: 3/3 scenes ok, failed 0, commit ${c1:0:8}' '$tmp/clean/out/summary.txt'"
check "kmsg/<n>.log and .class for each scene, empty and all zero (the boot lines left out)" \
  bash -c "for n in 1 2 3; do [ -f '$tmp/clean/out/kmsg/'\$n.log ] && [ ! -s '$tmp/clean/out/kmsg/'\$n.log ] &&
    grep -q '^kmsg: resets=0 timeouts=0 mmu_faults=0 fw_faults=0 gpu_faults=0 cs_faults=0 other_panthor=0' '$tmp/clean/out/kmsg/'\$n.class || exit 1; done"
check "meta.txt has the commit, the kernel and both boot_ids" \
  bash -c "grep -qx 'commit=$c1' '$tmp/clean/out/meta.txt' && grep -qx 'kernel=6.18.55-current-rockchip64' '$tmp/clean/out/meta.txt' &&
    grep -qx 'boot_id_before=5a1e0000-0000-4000-8000-00000000b007' '$tmp/clean/out/meta.txt' &&
    grep -qx 'boot_id_after=5a1e0000-0000-4000-8000-00000000b007' '$tmp/clean/out/meta.txt' && grep -qx 'dev=no' '$tmp/clean/out/meta.txt'"
check "each scene ran as its own process: --size 800x600 -b '<scene>' on the staged lib" \
  bash -c "[ \$(grep -c -- \"--size 800x600 -b \\\"\\\$1\\\"' glmark2 '\" '$tmp/clean/ssh.log') = 3 ] &&
    grep -q 'env LD_LIBRARY_PATH=$tmp/clean/board/home/yakogl-mali-glmark2/${c1:0:8}-[0-9-]*/lib ' '$tmp/clean/ssh.log'"
check "the run staged only under <home>/yakogl-mali-glmark2/<sha8>-<stamp>/" \
  bash -c "grep '^scp' '$tmp/clean/ssh.log' | grep -o 'board.invalid:[^ ]*' | grep -v '^board.invalid:$tmp/clean/board/home/yakogl-mali-glmark2/${c1:0:8}-' | grep -q . && exit 1;
    grep -q 'board.invalid:$tmp/clean/board/home/yakogl-mali-glmark2/${c1:0:8}-' '$tmp/clean/ssh.log'"
check "the stage (and the directory the run made for it) is removed at the end" stage_gone clean
check "the run directory keeps no host name, address or MAC" clean_out clean
check "the board lock is released (its record emptied)" bash -c "[ ! -s '$tmp/board.lock' ]"

check "without FB_DUMP: no GLMARK2_FB on the board command, no png directory, no dump text" \
  bash -c "! grep -q GLMARK2_FB '$tmp/clean/ssh.log' && [ ! -e '$tmp/clean/raw/png' ] && ! grep -q 'glmark2-fb:' '$tmp/clean/out/log/1.err'"

echo "mali-glmark2-selftest: FB_DUMP=1 over scenes 1-2 (a 16x12 frame dump in each scene's stderr)"
FB_DUMP=1 run fbdump clean --scenes 1-2
check "exit 0, both scenes ok (the verdict is as without the dump)" \
  bash -c "[ \"\$(cat '$tmp/fbdump/rc')\" = 0 ] && grep -qx 'mali-glmark2: 2/2 scenes ok, failed 0, commit ${c1:0:8}' '$tmp/fbdump/out.txt'"
check "GLMARK2_FB=1 is on each scene's board command" \
  bash -c "[ \$(grep -c 'env LD_LIBRARY_PATH=[^ ]*/lib GLMARK2_FB=1 ' '$tmp/fbdump/ssh.log') = 2 ]"
check "a PNG of the dump's size per scene in RAW_DIR/png/, nowhere in the run directory" \
  bash -c "for n in 01 02; do python3 -I -c \"import struct,sys; d=open(sys.argv[1],'rb').read(); sys.exit(0 if d[:8]==b'\\x89PNG\\r\\n\\x1a\\n' and struct.unpack('>II', d[16:24])==(16,12) else 1)\" '$tmp/fbdump/raw/png/'\$n.png || exit 1; done;
    grep -q 'crc32 .* agrees' '$tmp/fbdump/raw/png/01.txt' && grep -q 'picture value agrees' '$tmp/fbdump/raw/png/02.txt' &&
    [ -z \"\$(find '$tmp/fbdump/out' -name '*.png')\" ]"
check "the raw stderr keeps the dump; the redacted log/<n>.err has none, one 'dump stripped' line and the picture line" \
  bash -c "for n in 1 2; do grep -q '^glmark2-fb: 11 ' '$tmp/fbdump/raw/run/'\$n.err &&
    ! grep -q 'glmark2-fb: [0-9e]' '$tmp/fbdump/out/log/'\$n.err && ! grep -q 'begin' '$tmp/fbdump/out/log/'\$n.err &&
    [ \$(grep -c '^glmark2-fb: dump stripped' '$tmp/fbdump/out/log/'\$n.err) = 1 ] &&
    grep -q '^glmark2-picture: ' '$tmp/fbdump/out/log/'\$n.err &&
    grep -q '^yakogl-mali-exit: ' '$tmp/fbdump/out/log/'\$n.err || exit 1; done"
check "the picture still reaches scenes.tsv and pictures.txt, and the run directory is clean" \
  bash -c "[ \$(grep -c '^mali | ' '$tmp/fbdump/out/pictures.txt') = 2 ] && grep -q 'probes c ' '$tmp/fbdump/out/pictures.txt' && ! grep -rq 'glmark2-fb: [0-9]' '$tmp/fbdump/out'" 
check "glmark2-fb.py --rundir on the redacted run directory finds no dump (and none damaged)" \
  bash -c "python3 -I '$HERE/glmark2-fb.py' --rundir '$tmp/fbdump/out' '$tmp/fbdump/pngs' | grep -c 'no dump' | grep -qx 2"
check "the run directory keeps no host name, address or MAC" clean_out fbdump

echo "mali-glmark2-selftest: scenes 1-10, each with another outcome (--timeout 2)"
run mixed mixed --scenes 1-10 --timeout 2
check "every scene ran: exit 0" rc_is mixed 0
check "the verdicts, in order" verdicts mixed \
  ok,gl-error,out-of-gpu-memory,refused-draw,gpu-fault,job-timeout,failed-scene,crash,hang,ok
check "a GL error keeps its frame time" bash -c "[ '$(col mixed 2 4)' = 0.800 ]"
check "a refused draw: the exit line's refused count and first reason" \
  bash -c "[ '$(col mixed 4 7)' = 'yakogl-mali-exit: draws=0 refused=600 first_refused=draw: a varying of 5 components gpu_faults=0 job_timeouts=0' ]"
check "a GPU fault: kmsg 2 (MMU and GPU fault), kmsg/5.log holds the panthor lines, address and MAC redacted" \
  bash -c "[ '$(col mixed 5 6)' = 2 ] && grep -q 'Unhandled Page fault in AS1' '$tmp/mixed/out/kmsg/5.log' &&
    grep -q 'GPU Fault 0x00000088' '$tmp/mixed/out/kmsg/5.log' && grep -q 'peer <addr> <mac>' '$tmp/mixed/out/kmsg/5.log' &&
    grep -q 'mmu_faults=1 fw_faults=0 gpu_faults=1' '$tmp/mixed/out/kmsg/5.class' && [ \$(wc -l < '$tmp/mixed/out/kmsg/5.log') = 3 ]"
check "a job timeout: kmsg 1, its frame time 500.000, only its own lines in kmsg/6.log" \
  bash -c "[ '$(col mixed 6 6) $(col mixed 6 4)' = '1 500.000' ] && grep -q 'job timeout: pid=' '$tmp/mixed/out/kmsg/6.log' &&
    ! grep -q 'GPU Fault' '$tmp/mixed/out/kmsg/6.log'"
check "a failed setup has no frame time and no picture" bash -c "[ '$(col mixed 7 4) $(col mixed 7 5)' = '- -' ]"
check "a crash has no exit line" bash -c "[ '$(col mixed 8 7)' = - ]"
check "the hang was killed and nothing of it is left on the board" \
  bash -c "! pgrep -f '[${tmp:0:1}]${tmp:1}/mixed/board/home/yakogl-mali-glmark2' && grep -q 'scene 9: hang' '$tmp/mixed/out.txt'"
check "an FPS without FrameTime gives 1000 / FPS" bash -c "[ '$(col mixed 10 4)' = 4.000 ]"
check "pictures.txt leaves out the scenes without a picture line (setup, crash, hang)" \
  bash -c "[ \$(grep -c '^mali | ' '$tmp/mixed/out/pictures.txt') = 7 ]"
check "the summary line names each failure cause in run order" \
  bash -c "grep -qx 'mali-glmark2: 2/10 scenes ok, failed 8 (gl-error 1, out-of-gpu-memory 1, refused-draw 1, gpu-fault 1, job-timeout 1, failed-scene 1, crash 1, hang 1), commit ${c1:0:8}' '$tmp/mixed/out.txt'"
check "the stage is removed and the run directory is clean" bash -c "[ ! -e '$tmp/mixed/board/home/yakogl-mali-glmark2' ] && ! grep -rqiE 'board\.invalid|198\.51\.100|02:00:5e' '$tmp/mixed/out'"

echo "mali-glmark2-selftest: a result line split from its header by the back end's stdout log"
run split split --scenes 1-2
check "the split result is its header's scene: ok, FrameTime 0.959 ms, the picture kept" \
  bash -c "[ \"\$(cat '$tmp/split/rc')\" = 0 ] && [ '$(col split 1 3) $(col split 1 4)' = 'ok 0.959' ] &&
    [ '$(col split 2 3) $(col split 2 4)' = 'ok 0.800' ] && [ \$(grep -c '^mali | ' '$tmp/split/out/pictures.txt') = 2 ]"
check "...and the glued header line is read as the header: 2/2 ok" \
  bash -c "grep -qx 'mali-glmark2: 2/2 scenes ok, failed 0, commit ${c1:0:8}' '$tmp/split/out.txt'"

echo "mali-glmark2-selftest: scenes 1-8, each with another memory failure (--timeout 2)"
run oom oom --scenes 1-8 --timeout 2
check "every scene ran: exit 0" rc_is oom 0
check "scene 1 ok, scenes 2-8 out-of-gpu-memory: the back end's lines, GL_OUT_OF_MEMORY (not gl-error), a depth/stencil refusal (not refused-draw), the kernel's lines" \
  verdicts oom ok,out-of-gpu-memory,out-of-gpu-memory,out-of-gpu-memory,out-of-gpu-memory,out-of-gpu-memory,out-of-gpu-memory,out-of-gpu-memory
check "the kernel's memory lines are kept in kmsg/8.log" \
  bash -c "grep -q 'Out of memory: Killed' '$tmp/oom/out/kmsg/8.log' && grep -q 'page allocation failure' '$tmp/oom/out/kmsg/8.log'"
check "the summary names out-of-gpu-memory 7 times" \
  bash -c "grep -qx 'mali-glmark2: 1/8 scenes ok, failed 7 (out-of-gpu-memory 7), commit ${c1:0:8}' '$tmp/oom/out.txt'"
check "the run directory is clean" clean_out oom

echo "mali-glmark2-selftest: the board goes away in scene 2 of 1-4"
run lost lostssh --scenes 1-4
check "lost ssh: exit 5, the board reported as needing a person" \
  bash -c "[ \"\$(cat '$tmp/lost/rc')\" = 5 ] && grep -q 'needs a person' '$tmp/lost/out.txt'"
check "scene 1 ok, scene 2 board-lost, scenes 3 and 4 not-run" verdicts lost ok,board-lost,not-run,not-run
check "no retry: glmark2 started twice" calls lost 2
check "no ssh call ran a scene after the loss" bash -c "[ \$(grep -c 'glmark2-es2-mali --size' '$tmp/lost/ssh.log') = 2 ]"
check "the summary is written for what ran" \
  bash -c "grep -qx 'mali-glmark2: 1/4 scenes ok, failed 3 (board-lost 1, not-run 2), commit ${c1:0:8}' '$tmp/lost/out/summary.txt'"

echo "mali-glmark2-selftest: the exact v6.18 fault messages, as mali-kmsg.py classifies them"
for pair in page-fault:mmu_faults cs-fatal:cs_faults cs-fault:cs_faults job-timeout:timeouts; do
  f=${pair%%:*} k=${pair##*:}
  check "kmsg/$f.txt counts one $k (the unprefixed detail lines are not counted)" \
    bash -c "grep -v '^#' '$FIX/kmsg/$f.txt' | python3 -I '$HERE/mali-kmsg.py' --json - | python3 -I -c \
\"import json,sys; r=json.load(sys.stdin); sys.exit(0 if r['$k']==1 and sum(v for x,v in r.items() if x!='first')==1 else 1)\""
done

echo "mali-glmark2-selftest: faults the board survives (scenes 2-6, --timeout 2)"
run faults faults --scenes 2-6 --timeout 2
check "a page fault, CS_FATAL, CS_FAULT and a SIGTERM-proof job timeout, then a clean scene: exit 0" rc_is faults 0
check "the verdicts: three gpu-fault, the TERM-proof hang, ok" verdicts faults gpu-fault,gpu-fault,gpu-fault,hang,ok
check "a CS_FATAL or CS_FAULT alone counts (kmsg 1) and the page fault is one count, not six" \
  bash -c "[ '$(col faults 1 6) $(col faults 2 6) $(col faults 3 6)' = '1 1 1' ] &&
    grep -q 'mmu_faults=1' '$tmp/faults/out/kmsg/1.class' && grep -q 'cs_faults=1' '$tmp/faults/out/kmsg/2.class' &&
    grep -q 'source id 0x2200' '$tmp/faults/out/kmsg/1.log'"
check "the process that ignores SIGTERM was killed and nothing of it is left" \
  bash -c "! pgrep -f '[${tmp:0:1}]${tmp:1}/faults/board/home/yakogl-mali-glmark2'"
check "each faulting scene was followed by a probe that passed (4 probes, build:use-vbo=false)" \
  bash -c "[ \$(grep -c -- \"-b \\\"\\\$1\\\"' glmark2 'build:use-vbo=false'\" '$tmp/faults/ssh.log') = 4 ] &&
    [ \"\$(awk -F'\t' 'NR > 1 { print \$1 \$2 \$3 }' '$tmp/faults/out/probes.tsv' | paste -sd,)\" = 11pass,21pass,31pass,41pass ]"
check "5 scenes and 4 probes ran; the clean scene was not probed" calls faults 9
check "no 'needs a person' for a board that came back" bash -c "! grep -q 'needs a person' '$tmp/faults/out.txt'"
check "the hang's own kmsg lines are kept" bash -c "grep -q 'job timeout: pid=' '$tmp/faults/out/kmsg/4.log'"

echo "mali-glmark2-selftest: a page fault that leaves the device dead (scenes 2-5)"
run wedge wedge --scenes 2-5
check "the probe fails twice: exit 5, 'needs a person' and the final ssh-reachability line" \
  bash -c "[ \"\$(cat '$tmp/wedge/rc')\" = 5 ] && grep -q 'needs a person' '$tmp/wedge/out.txt' &&
    grep -q 'final ssh reachability: answers, boot_id unchanged' '$tmp/wedge/out.txt'"
check "scene 2 gpu-fault, scenes 3-5 not-run" verdicts wedge gpu-fault,not-run,not-run,not-run
check "one scene and exactly two probes ran, no third" calls wedge 3
check "probes.tsv records two failed probes (crash)" \
  bash -c "[ \"\$(awk -F'\t' 'NR > 1 { print \$1 \$2 \$3 \$4 }' '$tmp/wedge/out/probes.tsv' | paste -sd,)\" = 11failcrash,12failcrash ]"
check "the stage is removed although the run needs a person" stage_gone wedge

echo "mali-glmark2-selftest: a page fault, and the board does not come back (scenes 2-5)"
run faultdown faultdown --scenes 2-5
check "ssh stops answering in the probe: exit 5, 'needs a person', final line says it does not answer" \
  bash -c "[ \"\$(cat '$tmp/faultdown/rc')\" = 5 ] && grep -q 'needs a person' '$tmp/faultdown/out.txt' &&
    grep -q 'final ssh reachability: does not answer' '$tmp/faultdown/out.txt'"
check "scene 2 gpu-fault, the rest not-run" verdicts faultdown gpu-fault,not-run,not-run,not-run
check "never retried: one scene and one probe, no second probe, no later scene" \
  bash -c "[ \$(grep -c 'glmark2-es2-mali --size' '$tmp/faultdown/ssh.log') = 2 ] && [ \"\$(wc -l < '$tmp/faultdown/out/probes.tsv')\" = 2 ] &&
    grep -q 'board-lost' '$tmp/faultdown/out/probes.tsv'"

echo "mali-glmark2-selftest: the board reboots in scene 1 of 1-2"
run reboot reboot --scenes 1-2
check "a new boot_id: exit 5, scene 1 board-lost, scene 2 not-run" \
  bash -c "[ \"\$(cat '$tmp/reboot/rc')\" = 5 ] && grep -q 'rebooted' '$tmp/reboot/out.txt'"
check "the reboot's verdicts" verdicts reboot board-lost,not-run

echo "mali-glmark2-selftest: no 'yakogl-mali: loaded' line in scene 1 of 1-2"
run nolib nolib --scenes 1-2
check "a missing loaded line: refused, exit 3, wrong-lib then not-run" \
  bash -c "[ \"\$(cat '$tmp/nolib/rc')\" = 3 ] && grep -q \"no 'yakogl-mali: loaded' line\" '$tmp/nolib/out.txt'"
check "the missing line's verdicts" verdicts nolib wrong-lib,not-run
check "after the refusal no other scene ran" calls nolib 1
check "after the refusal the stage is removed" stage_gone nolib
run foreign foreignlib --scenes 1
check "a loaded line naming the board's own library: refused, exit 3, wrong-lib" \
  bash -c "[ \"\$(cat '$tmp/foreign/rc')\" = 3 ] && grep -q 'naming a library outside' '$tmp/foreign/out.txt'"
check "the foreign library's verdict" verdicts foreign wrong-lib

echo "mali-glmark2-selftest: the render node is busy"
BUSY=1 run busy clean --scenes 1
check "a busy render node: exit 4, nothing staged or run" \
  bash -c "[ \"\$(cat '$tmp/busy/rc')\" = 4 ] && [ ! -e '$tmp/busy/board/glmark2.calls' ] && ! grep -q '^scp' '$tmp/busy/ssh.log'"

echo "mali-glmark2-selftest: builds that are not a record"
COMMIT=$c2 run side clean --scenes 1
check "a commit not on the main ref is refused (exit 3) before any ssh" \
  bash -c "[ \"\$(cat '$tmp/side/rc')\" = 3 ] && [ ! -s '$tmp/side/ssh.log' ] && grep -q 'is not on main' '$tmp/side/out.txt'"
DEV='yes (--dev: a local test build, not a record)' run devb clean --scenes 1
check "a dev=yes build is refused (exit 3) before any ssh" \
  bash -c "[ \"\$(cat '$tmp/devb/rc')\" = 3 ] && [ ! -s '$tmp/devb/ssh.log' ] && grep -q 'dev=yes' '$tmp/devb/out.txt'"
COMMIT=$c2 run sidedev clean --dev --scenes 1
check "--dev runs it anyway, and meta.txt says dev=yes" \
  bash -c "[ \"\$(cat '$tmp/sidedev/rc')\" = 0 ] && grep -q '^dev=yes' '$tmp/sidedev/out/meta.txt' && [ '$(col sidedev 1 3)' = ok ]"

echo "mali-glmark2-selftest: --scenes from a file"
run sfile clean --scenes "$FIX/scenes.txt"
check "a scenes file: an index, a scene with ';' and an option-less one, in order" \
  bash -c "[ \"\$(cat '$tmp/sfile/rc')\" = 0 ] && [ \"\$(awk -F'\t' 'NR > 1 { print \$1 }' '$tmp/sfile/out/scenes.tsv' | paste -sd' ')\" = 'build:use-vbo=false effect2d:kernel=0,1,0;1,-4,1;0,1,0; refract' ]"
check "the option-less scene's picture is under '[refract] <default>'" \
  bash -c "grep -q '^mali | \[refract\] <default> | 800x600' '$tmp/sfile/out/pictures.txt' &&
    grep -q '^mali | \[effect2d\] kernel=0,1,0;1,-4,1;0,1,0; | ' '$tmp/sfile/out/pictures.txt'"
mkdir -p "$tmp/badscene"
bash "$RUN" --dry-run --scenes "build:use-vbo=false'x" > "$tmp/badscene/out.txt" 2>&1
echo $? > "$tmp/badscene/rc"
check "a scene with a quote in it is refused (exit 1)" bash -c "[ \"\$(cat '$tmp/badscene/rc')\" = 1 ] && grep -q 'letters, digits' '$tmp/badscene/out.txt'"
check "an index outside 1-33 is refused (exit 1)" bash -c "! bash '$RUN' --dry-run --scenes 34 > /dev/null 2>&1"

echo "mali-glmark2-selftest: --dry-run on the fixture build, fakes that fail if called"
mkdir -p "$tmp/nobin" "$tmp/dry"
for t in ssh scp; do
  printf '#!/bin/bash\necho "%s $*" >> "%s/dry/called.log"\nexit 99\n' "$t" "$tmp" > "$tmp/nobin/$t"
done
chmod +x "$tmp/nobin"/*
dry() { # <name> <args...>: a dry run in a scratch cwd; output to dry/<name>.txt, status to .rc
  local n=$1
  shift
  (cd "$tmp/dry" && env PATH="$tmp/nobin:$PATH" BOARD_HOST=selftest@board.invalid MALI_GLMARK2_REPO="$tmp/repo" MALI_GLMARK2_MAIN_REF=main \
    bash "$RUN" "$@" > "$tmp/dry/$n.txt" 2>&1; echo $? > "$tmp/dry/$n.rc")
}
mkdir -p "$tmp/drybuild"
cp -r "$FIX/fixture-build/." "$tmp/drybuild/"
sed -i "s/^commit=.*/commit=$c1/" "$tmp/drybuild/BUILD-INFO"
dry a --dry-run "$tmp/drybuild" "$tmp/dry/out"
check "--dry-run lists the 33 default scenes in order and exits 0" \
  bash -c "[ \"\$(cat '$tmp/dry/a.rc')\" = 0 ] && grep -q ': 33 scene(s), each its own process, timeout 180 s' '$tmp/dry/a.txt' &&
    grep -q '  1 build:use-vbo=false$' '$tmp/dry/a.txt' && grep -q '  33 loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true$' '$tmp/dry/a.txt'"
check "--dry-run prints the commit check and the per-scene command" \
  bash -c "grep -q 'commit $c1, on main: yes' '$tmp/dry/a.txt' &&
    grep -q \"exec timeout -k 10 180 env LD_LIBRARY_PATH=<stage>/lib <stage>/bin/glmark2-es2-mali --size 800x600 -b\" '$tmp/dry/a.txt'"
COMMIT_DRY=1 dry b --dry-run --scenes '22-25' --timeout 60
check "a bare --dry-run (no build, no outdir) exits 0 and prints the plan" \
  bash -c "[ \"\$(cat '$tmp/dry/b.rc')\" = 0 ] && grep -q 'build: not given' '$tmp/dry/b.txt' &&
    grep -q ': 4 scene(s), each its own process, timeout 60 s' '$tmp/dry/b.txt' && grep -q '  4 refract$' '$tmp/dry/b.txt'"
sed -i "s/^commit=.*/commit=$c2/" "$tmp/drybuild/BUILD-INFO"
dry c --dry-run "$tmp/drybuild" "$tmp/dry/out"
check "--dry-run says a real run would refuse a commit not on main, and exits 0" \
  bash -c "[ \"\$(cat '$tmp/dry/c.rc')\" = 0 ] && grep -q 'a real run refuses it, exit 3' '$tmp/dry/c.txt'"
check "--dry-run never calls ssh or scp, takes no lock and creates nothing" \
  bash -c "[ ! -e '$tmp/dry/called.log' ] && [ -z \"\$(ls -A '$tmp/dry' | grep -v '^[abce]\.')\" ]"
FB_DUMP=1 dry e --dry-run --scenes 1-2 "$tmp/drybuild" "$tmp/dry/out"
check "--dry-run with FB_DUMP=1 says so and shows GLMARK2_FB=1 in the command; without it neither appears" \
  bash -c "[ \"\$(cat '$tmp/dry/e.rc')\" = 0 ] && grep -q 'FB_DUMP=1: GLMARK2_FB=1 on each scene' '$tmp/dry/e.txt' &&
    grep -q 'LD_LIBRARY_PATH=<stage>/lib GLMARK2_FB=1 <stage>/bin/glmark2-es2-mali' '$tmp/dry/e.txt' &&
    ! grep -q 'GLMARK2_FB\|FB_DUMP' '$tmp/dry/a.txt'"
dry d "$tmp/drybuild"
check "a real run without <outdir> exits 1 (usage)" bash -c "[ \"\$(cat '$tmp/dry/d.rc')\" = 1 ]"

total=$((passed + failed))
if [ "$failed" -ne 0 ] || [ "$total" -ne "$EXPECT" ]; then
  echo "mali-glmark2-selftest: $failed of $total checks failed (the self-test pins $EXPECT checks)" >&2
  for d in "$tmp"/*/out.txt; do echo "--- $d"; tail -20 "$d"; done >&2
  exit 1
fi
echo "mali-glmark2-selftest: $passed checks passed"
