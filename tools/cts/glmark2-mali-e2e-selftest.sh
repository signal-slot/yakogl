#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# End to end, without a board, GLMARK2_SRC or a cross compiler: the NanoPC-T6
# glmark2-es2 chain of plan #332, each script as it is on main.
#   1. tools/cts/mali-glmark2-build.sh --dry-run in a throwaway repository: the
#      commit it would write to BUILD-INFO and the layout it would make are
#      what the runner takes.
#   2. tools/cts/mali-glmark2-run.sh over all 33 scenes on the fake board of
#      tools/cts/testdata/mali-glmark2-run/ (used by path, not copied), with the
#      fixture build stamped with that commit and the scenario of
#      tools/cts/testdata/glmark2-mali-e2e/: a GPU fault in scene 5, a hang in
#      scene 23 (--timeout 2), the rest clean. The board must still answer with
#      the same boot_id at the end, the stage must be gone, and no command may
#      name /boot.
#   3. tools/cts/glmark2-mali-tally.py on the run. The runner's run directory
#      (meta.txt, scenes.tsv, log/, kmsg/) is not the layout the tally reads
#      (env.txt, <NN>-<slug>.log|kmsg|status): the tally refuses it, and this
#      test pins that refusal (README.md there, "The layout gap"). The fixture
#      runner-to-tally.py maps the one onto the other; the tally's verdicts,
#      its 'mali | ...' picture lines, its --results-row, --status-row and
#      --causes drafts are then checked against the runner's own verdicts and
#      pictures and against *.expected.in (byte for byte, placeholders filled).
#
#   tools/cts/glmark2-mali-e2e-selftest.sh    prints "glmark2-mali-e2e-selftest: N checks passed"
#
# Host only (bash, git, python3), about 20 s (the hang waits for a 2 s
# timeout). Its gate row in tools/check.sh is added by plan #332's gate items:
#   bash tools/cts/glmark2-mali-e2e-selftest.sh
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
RUNFIX=$HERE/testdata/mali-glmark2-run
FIX=$HERE/testdata/glmark2-mali-e2e
BUILD_SH=$HERE/mali-glmark2-build.sh
RUN=$HERE/mali-glmark2-run.sh
TALLY=$HERE/glmark2-mali-tally.py
EXPECT=26
base=${TMPDIR:-$REPO/build}
mkdir -p "$base"
tmp=$(mktemp -d "$base/glmark2-mali-e2e-selftest-XXXXXX")
# GLMARK2_MALI_E2E_KEEP=1 keeps the directory and says where it is.
cleanup() {
  pkill -f -- "$tmp/" 2>/dev/null
  if [ "${GLMARK2_MALI_E2E_KEEP-}" = 1 ]; then echo "glmark2-mali-e2e-selftest: kept $tmp"; else rm -rf "$tmp"; fi
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

# The throwaway repository the build and the runner take "on main" from.
g() { git -C "$tmp/repo" -c user.name=selftest -c user.email=selftest -c commit.gpgsign=false "$@"; }
mkdir -p "$tmp/repo"
git -C "$tmp/repo" init -q
echo one > "$tmp/repo/file.txt"
g add file.txt
g commit -q -m c1
g branch -M main
c1=$(g rev-parse HEAD)

echo "glmark2-mali-e2e-selftest: 1. mali-glmark2-build.sh --dry-run"
# Fakes that fail if called, every input path pointing at nothing.
mkdir -p "$tmp/nobin" "$tmp/cwd-build"
for t in cargo meson ninja curl rustc clang ssh scp; do
  printf '#!/bin/bash\necho "%s $*" >> "%s/called.log"\nexit 99\n' "$t" "$tmp" > "$tmp/nobin/$t"
done
chmod +x "$tmp"/nobin/*
B=$tmp/build-out
(cd "$tmp/cwd-build" && env PATH="$tmp/nobin:$PATH" MALI_GLMARK2_REPO="$tmp/repo" MALI_GLMARK2_MAIN_REF=main \
  TOOLCHAIN="$tmp/none/toolchain" GLMARK2_SRC="$tmp/none/glmark2" IMG_SYSROOT="$tmp/none/img" \
  BUILD="$tmp/none/build" bash "$BUILD_SH" --dry-run "$B" > "$tmp/build.txt" 2>&1; echo $? > "$tmp/build.rc")
check "the build's dry run exits 0, runs nothing and writes nothing" \
  bash -c "[ \"\$(cat '$tmp/build.rc')\" = 0 ] && [ ! -e '$tmp/called.log' ] && [ ! -e '$tmp/none' ] &&
    [ ! -e '$B' ] && [ ! -e '$B.part' ] && [ -z \"\$(ls -A '$tmp/cwd-build')\" ]"
built=$(sed -n "s|^+ write $B.part/BUILD-INFO (commit=\([0-9a-f]*\) dev=no .*flavour=pbuffer.*|\1|p" "$tmp/build.txt" | head -1)
check "its BUILD-INFO would say main's commit, dev=no, flavour pbuffer" test "$built" = "$c1"
check "it would make the layout the runner takes: bin/glmark2-es2-mali, lib/ the pair, share/glmark2, BUILD-INFO" \
  bash -c "grep -q '^+ cp .* $B.part/bin/glmark2-es2-mali\$' '$tmp/build.txt' &&
    grep -q '^+ cp .*/libGLESv2.so.2 .*/libEGL.so.1 $B.part/lib/\$' '$tmp/build.txt' &&
    grep -q '^+ cp -r .* $B.part/share/glmark2/\$' '$tmp/build.txt' && grep -q '^+ mv $B.part $B\$' '$tmp/build.txt'"

echo "glmark2-mali-e2e-selftest: 2. mali-glmark2-run.sh, 33 scenes on the fake board"
mkdir -p "$tmp/bin" "$tmp/board-bin" "$tmp/board/home" "$tmp/build"
cp "$RUNFIX"/bin/* "$tmp/bin/"
cp "$RUNFIX"/board-bin/* "$tmp/board-bin/"
chmod +x "$tmp"/bin/* "$tmp"/board-bin/*
# The fixture build in the layout of step 1, stamped with the commit step 1 would write.
cp -r "$RUNFIX/fixture-build/." "$tmp/build/"
chmod +x "$tmp/build/bin/glmark2-es2-mali"
sed -i "s/^commit=.*/commit=${built:-none}/; s/^dev=.*/dev=no/" "$tmp/build/BUILD-INFO"
printf '[    0.000000] Booting Linux on physical CPU 0x0\n[    9.500000] panthor fb000000.gpu: [drm] clock rate = 1000000000\n' > "$tmp/board/kmsg"
boot=5a1e0000-0000-4000-8000-00000000b007
echo "$boot" > "$tmp/board/boot_id"
: > "$tmp/ssh.log"
O=$tmp/t6-glmark2-e2e
env PATH="$tmp/bin:$PATH" BOARD_HOST=selftest@board.invalid BOARD_LOCK="$tmp/board.lock" \
  FAKE_BOARD="$tmp/board" FAKE_BOARD_BIN="$tmp/board-bin" FAKE_LOG="$tmp/ssh.log" \
  FAKE_FIXTURES="$RUNFIX" FAKE_SCENARIO="$FIX/scenario.tsv" \
  MALI_GLMARK2_REPO="$tmp/repo" MALI_GLMARK2_MAIN_REF=main RAW_DIR="$tmp/raw" \
  BOARD_BACK_S=2 BOARD_POLL_S=0.5 KILL_GRACE_S=1 STAGE_MARGIN_KB=0 HOLD_MAX=600 \
  bash "$RUN" --timeout 2 "$tmp/build" "$O" > "$tmp/run.txt" 2>&1
echo $? > "$tmp/run.rc"
want_verdicts=$(printf 'ok %.0s' {1..4}; printf 'gpu-fault '; printf 'ok %.0s' {6..22}; printf 'hang '; printf 'ok %.0s' {24..33})
check "the runner exits 0 and every scene ran: gpu-fault in 5, hang in 23, the other 31 ok" \
  bash -c "[ \"\$(cat '$tmp/run.rc')\" = 0 ] &&
    [ \"\$(awk -F'\t' 'NR > 1 { printf \"%s \", \$3 }' '$O/scenes.tsv')\" = '$want_verdicts' ]"
check "its summary line: 31/33 ok, gpu-fault 1, hang 1, the commit of step 1" \
  grep -qx "mali-glmark2: 31/33 scenes ok, failed 2 (gpu-fault 1, hang 1), commit ${c1:0:8}" "$tmp/run.txt"
check "the board still answers ssh at the end, with the boot_id it had before" \
  bash -c "grep -q '^mali-glmark2-run: boot_id before $boot, kernel 6.18.55-current-rockchip64\$' '$tmp/run.txt' &&
    grep -q '^mali-glmark2-run: boot_id after $boot; run directory ' '$tmp/run.txt' &&
    grep -qx 'boot_id_after=$boot' '$O/meta.txt'"
board_clean() { ! pgrep -f "[${tmp:0:1}]${tmp:1}/board/home" && [ -z "$(ls -A "$tmp/board/home")" ]; }
check "the hang was killed and the stage removed: nothing of the run is left on the board" board_clean
check "no command of the run names /boot (the runner never touches it: no restore line to look for)" \
  bash -c "grep -q . '$tmp/ssh.log' && ! grep -E \"(^|[ =:'\\\"])/boot(/|[ ;'\\\"]|\\\$)\" '$tmp/ssh.log'"
check "the run directory keeps no host name, address or MAC" \
  bash -c "! grep -rqiE 'board\.invalid|198\.51\.100|02:00:5e|selftest@' '$O'"

echo "glmark2-mali-e2e-selftest: 3. glmark2-mali-tally.py on the run"
tally() { python3 -I "$TALLY" "$@"; }
tally --table "$FIX/pictures.txt" "$O" > /dev/null 2> "$tmp/direct.err"
echo $? > "$tmp/direct.rc"
check "the layout gap, pinned: the tally refuses the runner's own directory (exit 1, no env.txt)" \
  bash -c "[ \"\$(cat '$tmp/direct.rc')\" = 1 ] && grep -q '^glmark2-mali-tally: malformed: .*/env.txt: No such file' '$tmp/direct.err'"
check "...and none of the files it reads is there (env.txt, <NN>-<slug>.log|kmsg|status)" \
  bash -c "[ ! -e '$O/env.txt' ] && [ -z \"\$(ls '$O' | grep -E '^[0-9][0-9]-.*\.(log|kmsg|status)\$')\" ]"
bash "$HERE/glmark2-host-ref.sh" --list > "$tmp/list.txt"
T=$tmp/tally/t6-glmark2-e2e
python3 -I "$FIX/runner-to-tally.py" "$O" "$tmp/run.txt" "$tmp/list.txt" "$tmp/build/lib/libGLESv2.so.2" NanoPC-T6 "$T" 2> "$tmp/bridge.err"
check "runner-to-tally.py maps each of the runner's 33 scenes to a --list slug" \
  bash -c "[ \$(ls '$T' | grep -cE '^[0-9][0-9]-[a-z0-9-]+\.log\$') = 33 ] && [ ! -s '$tmp/bridge.err' ]"
check "the hang's status is exit=124 timeout=1, the others timeout=0" \
  bash -c "[ \"\$(cat '$T/23-terrain-default.status' | paste -sd' ')\" = 'exit=124 timeout=1' ] &&
    [ \$(cat '$T'/*.status | grep -cx 'timeout=0') = 32 ]"
cp -r "$T" "$tmp/class-only"
cp "$O/kmsg/1.class" "$tmp/class-only/01-build-use-vbo-false.kmsg"
tally --table "$FIX/pictures.txt" "$tmp/class-only" > /dev/null 2> "$tmp/class-only.err"
echo $? > "$tmp/class-only.rc"
check "the kmsg gap, pinned: the tally refuses the runner's kmsg/<n>.class line (cs_faults=), takes mali-kmsg.py --json" \
  bash -c "grep -q ' cs_faults=0 ' '$O/kmsg/1.class' && [ \"\$(cat '$tmp/class-only.rc')\" = 1 ] &&
    grep -q \"01-build-use-vbo-false.kmsg: no mali-kmsg.py line ('kmsg: resets=N ...')\$\" '$tmp/class-only.err'"
tally --table "$FIX/pictures.txt" "$T" > "$tmp/table.txt" 2> "$tmp/table.err"
echo $? > "$tmp/table.rc"
check "the tally takes the mapped run: exit 0" bash -c "[ \"\$(cat '$tmp/table.rc')\" = 0 ]"
check "the tally's verdicts are the runner's, scene by scene" \
  bash -c "[ \"\$(awk '\$1 ~ /^[0-9][0-9]\$/ { for (i = 2; i <= NF; i++) if (\$i ~ /^(ok|gpu-fault|hang|crash|oom|gl-error|fail|not-run)\$/) { printf \"%s \", \$i; break } }' '$tmp/table.txt')\" = '$want_verdicts' ]"
check "the tally's summary: 33 run, ok 31, gpu-fault 1, hang 1, 2 panthor lines" \
  grep -q '^scenes run 33 of 33; hang 1, gpu-fault 1, ok 31; GL error lines 0, out of GPU memory lines 0, panthor kernel lines 2 (resets 0); ' "$tmp/table.txt"
check "the pictures against the fixture's mali column: 2 match, 1 mismatch, 29 no-reference, 1 absent" \
  grep -qx 'pictures against column mali: 33 scenes, absent 1, match 2, mismatch 1, no-reference 29' "$tmp/table.txt"
tally --picture-lines --table "$FIX/pictures.txt" "$T" > "$tmp/picture-lines.txt" 2> "$tmp/picture-lines.err"
check "--picture-lines: 32 'mali | <scene> | <picture>' lines, the runner's pictures.txt line for line" \
  bash -c "[ \$(grep -c '^mali | ' '$tmp/picture-lines.txt') = 32 ] && grep -v '^#' '$O/pictures.txt' | cmp - '$tmp/picture-lines.txt'"
check "--picture-lines: the hang's scene has none, named on stderr" \
  bash -c "grep -qx 'glmark2-mali-tally: \[terrain\] <default> has no picture value (absent), no line for it' '$tmp/picture-lines.err' &&
    [ \$(wc -l < '$tmp/picture-lines.err') = 1 ]"
check "--picture-lines: every line in glmark2-pictures.txt's form" \
  bash -c "grep -v '^mali | \[[a-z0-9]*\] [^|]* | 800x600; probes c [0-9a-f]\{8\}, tl [0-9a-f]\{8\}, tr [0-9a-f]\{8\}, bl [0-9a-f]\{8\}, br [0-9a-f]\{8\}; crc16 [0-9a-f]\{4\}, crc16q [0-9a-f]\{4\}; mean [0-9]* [0-9]* [0-9]*; black \(yes\|no\)\$' '$tmp/picture-lines.txt' | grep -q . && exit 1; true"
date=$(sed -n 's/^date=//p' "$T/env.txt")
pair16=$(sha256sum "$tmp/build/lib/libGLESv2.so.2" | cut -c1-16)
# expect <name> <tally mode>: the tally's draft equals <name>.expected.in with
# @COMMIT@, @SHA8@, @DATE@ and @PAIR16@ filled in.
expect() {
  local n=$1
  shift
  tally "$@" --table "$FIX/pictures.txt" "$T" > "$tmp/$n.txt" 2> "$tmp/$n.err" || return 1
  sed "s/@COMMIT@/$c1/g; s/@SHA8@/${c1:0:8}/g; s/@DATE@/$date/g; s/@PAIR16@/$pair16/g" "$FIX/$n.expected.in" > "$tmp/$n.want"
  cmp -s "$tmp/$n.want" "$tmp/$n.txt" || { diff -u "$tmp/$n.want" "$tmp/$n.txt" | head -20 >&2; return 1; }
}
check "--results-row: the RESULTS.md row draft is results-row.expected.in" expect results-row --results-row
check "...its scene 5 is the gpu-fault with the panthor page fault, scene 23 the hang" \
  bash -c "grep -q '^  | 5 | \`\[texture\] texture-filter=mipmap\` | 25.00 | 40 | gpu-fault: ' '$tmp/results-row.txt' &&
    grep -q '^  | 23 | \`\[terrain\] <default>\` | - | - | hang: no exit within the runner' '$tmp/results-row.txt'"
check "--status-row: the STATUS.md line draft is status-row.expected.in" expect status-row --status-row
check "--causes: two draft issues (the hang, the GPU fault), causes.expected.in" expect causes --causes
check "...neither is marked as plan #309's" bash -c "! grep -q '#309' '$tmp/causes.txt'"

total=$((passed + failed))
if [ "$failed" -ne 0 ] || [ "$total" -ne "$EXPECT" ]; then
  echo "glmark2-mali-e2e-selftest: $failed of $total checks failed (the self-test pins $EXPECT checks)" >&2
  for f in build.txt run.txt direct.err bridge.err table.err; do echo "--- $f"; tail -12 "$tmp/$f" 2> /dev/null; done >&2
  exit 1
fi
echo "glmark2-mali-e2e-selftest: $passed checks passed"
