#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# End to end, without a board, GLMARK2_SRC, a cross compiler or the network:
# the NanoPC-T6 glmark2-es2 chain of plan #332, each script as it is on main.
#   1. tools/cts/mali-glmark2-build.sh --dry-run in a throwaway repository: the
#      plan names the panthor-gles pair, mali-glmark2-pbuffer.patch and the
#      BUILD-INFO (main's commit) the runner takes.
#   2. tools/cts/mali-glmark2-run.sh over all 33 scenes on the fake board of
#      tools/cts/mali-glmark2-selftest.sh (tools/cts/testdata/mali-glmark2-run/:
#      bin/, board-bin/ and the scenarios clean.tsv and faults.tsv used by
#      path; fixture-build/ copied once, its BUILD-INFO stamped with step 1's
#      commit). Each run directory has scenes.tsv with 33 rows, meta.txt and
#      summary.txt, and the run ends on the board's ssh answer (boot_id after).
#   3. tools/cts/glmark2-mali-tally.py on each run directory, read directly
#      (its runner-directory mode): exit 0, no verdict mismatch with the
#      runner, its verdicts, summary, picture lines and the --results-row,
#      --status-row and --causes drafts.
#
# The runner's scenes.tsv names each scene by its -b argument, options in the
# runner's own order (pulsar:quads=5:texture=false:light=false), while the
# tally matches the --list form, options sorted by name
# (light=false:quads=5:texture=false), and refuses the run directory at the
# first row that differs (14 of the 33 scenes do; pulsar, row 15, is the
# first). Step 3 therefore tallies a copy of each run directory whose
# scenes.tsv has every scene's options sorted (nothing else changes), and
# prints the tally's answer on the run directory as it is as a "note:" line,
# not a check.
#
#   tools/cts/glmark2-mali-e2e-selftest.sh    prints "glmark2-mali-e2e-selftest: N checks: N ok, 0 failed"
#
# Host only (bash, git, python3), about 30 s (faults.tsv's job timeout waits
# for a 2 s --timeout). Its temp directory is under $TMPDIR (else build/ of
# the checkout), never /tmp. Its gate row in tools/check.sh is added by plan
# #332's gate items:
#   bash tools/cts/glmark2-mali-e2e-selftest.sh
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
RUNFIX=$HERE/testdata/mali-glmark2-run
FIX=$HERE/testdata/glmark2-mali-e2e
BUILD_SH=$HERE/mali-glmark2-build.sh
RUN=$HERE/mali-glmark2-run.sh
TALLY=$HERE/glmark2-mali-tally.py
EXPECT=31
base=${TMPDIR:-$REPO/build}
case $base in /tmp | /tmp/* | /var/tmp | /var/tmp/*) base=$REPO/build ;; esac
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

# The throwaway repository the build and the runner take "main" from.
g() { git -C "$tmp/repo" -c user.name=selftest -c user.email=selftest -c commit.gpgsign=false "$@"; }
mkdir -p "$tmp/repo"
git -C "$tmp/repo" init -q
g commit -q --allow-empty -m c1
g branch -M main
c1=$(g rev-parse HEAD)

echo "glmark2-mali-e2e-selftest: 1. mali-glmark2-build.sh --dry-run"
# Fakes that fail if called, every input path pointing at nothing.
mkdir -p "$tmp/nobin" "$tmp/cwd-build"
for t in cargo meson ninja curl wget git-lfs rustc clang ssh scp; do
  printf '#!/bin/bash\necho "%s $*" >> "%s/called.log"\nexit 99\n' "$t" "$tmp" > "$tmp/nobin/$t"
done
chmod +x "$tmp"/nobin/*
B=$tmp/build-out
(cd "$tmp/cwd-build" && env PATH="$tmp/nobin:$PATH" MALI_GLMARK2_REPO="$tmp/repo" MALI_GLMARK2_MAIN_REF=main \
  TOOLCHAIN="$tmp/none/toolchain" GLMARK2_SRC="$tmp/none/glmark2" IMG_SYSROOT="$tmp/none/img" \
  BUILD="$tmp/none/build" bash "$BUILD_SH" --dry-run "$B" > "$tmp/build.txt" 2>&1; echo $? > "$tmp/build.rc")
check "the dry run exits 0, runs nothing and writes nothing" \
  bash -c "[ \"\$(cat '$tmp/build.rc')\" = 0 ] && [ ! -e '$tmp/called.log' ] && [ ! -e '$tmp/none' ] &&
    [ ! -e '$B' ] && [ ! -e '$B.part' ] && [ -z \"\$(ls -A '$tmp/cwd-build')\" ]"
check "its plan builds and checks the panthor-gles pair and puts it in lib/" \
  bash -c "grep -q '^+ .*/tools/mali/panthor-gles/build.sh ' '$tmp/build.txt' &&
    grep -q '^+ .*/tools/mali/panthor-gles/check.sh ' '$tmp/build.txt' &&
    grep -q '^+ cp .*/libGLESv2.so.2 .*/libEGL.so.1 $B.part/lib/\$' '$tmp/build.txt'"
check "its plan applies mali-glmark2-pbuffer.patch and builds the pbuffer-glesv2 flavour" \
  bash -c "grep -q '^+ git -C .* apply .*/tools/cts/mali-glmark2-pbuffer.patch\$' '$tmp/build.txt' &&
    grep -q -- '-Dflavors=pbuffer-glesv2' '$tmp/build.txt' && grep -q '^+ cp .* $B.part/bin/glmark2-es2-mali\$' '$tmp/build.txt'"
built=$(sed -n "s|^+ write $B.part/BUILD-INFO (commit=\([0-9a-f]*\) dev=no .*flavour=pbuffer.*|\1|p" "$tmp/build.txt" | head -1)
check "its plan writes BUILD-INFO with main's commit, dev=no, flavour pbuffer" test "$built" = "$c1"

echo "glmark2-mali-e2e-selftest: 2. mali-glmark2-run.sh, 33 scenes on the fake board"
# The fixture build in step 1's layout, stamped with the commit step 1 would
# write (the one copy: the runner needs a BUILD-INFO naming main).
mkdir -p "$tmp/build"
cp -r "$RUNFIX/fixture-build/." "$tmp/build/"
chmod +x "$tmp/build/bin/glmark2-es2-mali"
sed -i "s/^commit=.*/commit=${built:-none}/; s/^dev=.*/dev=no/" "$tmp/build/BUILD-INFO"
boot=5a1e0000-0000-4000-8000-00000000b007
# runner <scenario> [runner args...]: one run of all 33 scenes on a fresh fake
# board; the run directory is $tmp/<scenario>/out, the output .../run.txt.
runner() {
  local scen=$1 d=$tmp/$1
  shift
  mkdir -p "$d/board/home"
  printf '[    0.000000] Booting Linux on physical CPU 0x0\n[    9.500000] panthor fb000000.gpu: [drm] clock rate = 1000000000\n' > "$d/board/kmsg"
  echo "$boot" > "$d/board/boot_id"
  : > "$d/ssh.log"
  env PATH="$RUNFIX/bin:$PATH" BOARD_HOST=selftest@board.invalid BOARD_LOCK="$tmp/board.lock" \
    FAKE_BOARD="$d/board" FAKE_BOARD_BIN="$RUNFIX/board-bin" FAKE_LOG="$d/ssh.log" \
    FAKE_FIXTURES="$RUNFIX" FAKE_SCENARIO="$RUNFIX/scenarios/$scen.tsv" \
    MALI_GLMARK2_REPO="$tmp/repo" MALI_GLMARK2_MAIN_REF=main RAW_DIR="$d/raw" \
    BOARD_BACK_S=2 BOARD_POLL_S=0.5 KILL_GRACE_S=1 STAGE_MARGIN_KB=0 HOLD_MAX=600 \
    PROBE_PAUSE_S=0 PROBE_TIMEOUT_S=5 \
    bash "$RUN" "$@" "$tmp/build" "$d/out" > "$d/run.txt" 2>&1
  echo $? > "$d/rc"
}
# verdicts <scenario>: the runner's 33 verdicts, space separated
verdicts() { awk -F'\t' 'NR > 1 { printf "%s ", $3 }' "$tmp/$1/out/scenes.tsv"; }
board_clean() { ! pgrep -f "[${tmp:0:1}]${tmp:1}/$1/board/home" && [ -z "$(ls -A "$tmp/$1/board/home")" ]; }
ok_n() { local i; for ((i = $1; i <= $2; i++)); do printf 'ok '; done; }
want_clean=$(ok_n 1 33)
# faults.tsv: scenes 2-4 a page fault, CS_FATAL and CS_FAULT, scene 5 a
# SIGTERM-proof job timeout (killed after --timeout 2 and the grace)
want_faults="ok gpu-fault gpu-fault gpu-fault hang $(ok_n 6 33)"
for scen in clean faults; do
  echo "glmark2-mali-e2e-selftest: 2. ... scenario $scen.tsv"
  O=$tmp/$scen/out
  runner "$scen" --timeout 2
  case $scen in clean) want=$want_clean sum="33/33 scenes ok, failed 0" ;;
    faults) want=$want_faults sum="29/33 scenes ok, failed 4 (gpu-fault 3, hang 1)" ;; esac
  check "$scen: the runner exits 0; scenes.tsv has its header and 33 rows, the verdicts expected" \
    bash -c "[ \"\$(cat '$tmp/$scen/rc')\" = 0 ] && [ \$(wc -l < '$O/scenes.tsv') = 34 ] &&
      [ \"\$(awk -F'\t' 'NR > 1 && \$2 == \"$c1\"' '$O/scenes.tsv' | wc -l)\" = 33 ] &&
      [ \"$(verdicts "$scen")\" = '$want' ]"
  check "$scen: meta.txt has the commit, the kernel and both boot_ids" \
    bash -c "grep -qx 'commit=$c1' '$O/meta.txt' && grep -qx 'kernel=6.18.55-current-rockchip64' '$O/meta.txt' &&
      grep -qx 'boot_id_before=$boot' '$O/meta.txt' && grep -qx 'boot_id_after=$boot' '$O/meta.txt'"
  check "$scen: summary.txt and the output's summary line: $sum" \
    bash -c "grep -qx 'mali-glmark2: $sum, commit ${c1:0:8}' '$O/summary.txt' &&
      grep -qx 'mali-glmark2: $sum, commit ${c1:0:8}' '$tmp/$scen/run.txt'"
  check "$scen: the board answers ssh at the end with the boot_id it had (the final reachability line)" \
    bash -c "grep -q '^mali-glmark2-run: boot_id after $boot; run directory ' '$tmp/$scen/run.txt' &&
      [ \"\$(grep -v '^mali-glmark2: ' '$tmp/$scen/run.txt' | tail -1 | cut -d';' -f1)\" = 'mali-glmark2-run: boot_id after $boot' ]"
  check "$scen: nothing of the run is left on the board" board_clean "$scen"
  check "$scen: the run directory keeps no host name, address or MAC" \
    bash -c "! grep -rqiE 'board\.invalid|198\.51\.100|02:00:5e|selftest@' '$O'"
done

echo "glmark2-mali-e2e-selftest: 3. glmark2-mali-tally.py on the run directories"
tally() { env TMPDIR="$tmp" python3 -I "$TALLY" "$@"; }
# sorted <rundir> <copy>: the run directory with each scenes.tsv scene's
# options in --list order (sorted by name); every other byte as it is.
sorted() {
  cp -r "$1" "$2"
  python3 -I -c '
import sys
p = sys.argv[1]
rows = open(p, encoding="utf-8").read().splitlines()
out = rows[:1]
for row in rows[1:]:
    f = row.split("\t")
    name, sep, opts = f[0].partition(":")
    if sep and opts:
        f[0] = name + ":" + ":".join(sorted(opts.split(":"), key=lambda o: o.partition("=")[0]))
    out.append("\t".join(f))
open(p, "w", encoding="utf-8").write("\n".join(out) + "\n")
' "$2/scenes.tsv"
}
bash "$HERE/glmark2-host-ref.sh" --list > "$tmp/list.txt"
for scen in clean faults; do
  O=$tmp/$scen/out
  S=$tmp/$scen/sorted
  # The run directory as the runner wrote it: reported, not checked.
  tally --table "$FIX/pictures.txt" "$O" > /dev/null 2> "$tmp/$scen/as-is.err"
  rc=$?
  if [ "$rc" = 0 ]; then
    echo "  note: $scen: the tally takes the runner's scenes.tsv as it is (exit 0)"
  else
    echo "  note: $scen: the tally refuses the runner's scenes.tsv as it is (exit $rc): $(head -1 "$tmp/$scen/as-is.err" | sed "s|$tmp|<tmp>|g")"
  fi
  sorted "$O" "$S"
  check "$scen: the sorted copy is the run but for scenes.tsv's scene names, now --list's line for line" \
    bash -c "diff -r -x scenes.tsv '$O' '$S' && cmp <(cut -f2- '$O/scenes.tsv') <(cut -f2- '$S/scenes.tsv') &&
      awk -F'\t' 'NR > 1 { n = index(\$1, \":\"); print n ? \"[\" substr(\$1, 1, n - 1) \"] \" substr(\$1, n + 1) : \"[\" \$1 \"] <default>\" }' '$S/scenes.tsv' |
        cmp - <(cut -f2 '$tmp/list.txt')"
  tally --table "$FIX/pictures.txt" "$S" > "$tmp/$scen/table.txt" 2> "$tmp/$scen/table.err"
  echo $? > "$tmp/$scen/table.rc"
  check "$scen: the tally reads the runner directory: exit 0, no verdict mismatch, nothing on stderr" \
    bash -c "[ \"\$(cat '$tmp/$scen/table.rc')\" = 0 ] && [ ! -s '$tmp/$scen/table.err' ]"
  case $scen in clean) want=$want_clean sum='scenes run 33 of 33; ok 33; GL error lines 0, out of GPU memory lines 0, panthor kernel lines 0 (resets 0); ' ;;
    faults) want=$want_faults sum='scenes run 33 of 33; hang 1, gpu-fault 3, ok 29; GL error lines 0, out of GPU memory lines 0, panthor kernel lines ' ;; esac
  check "$scen: the tally's verdicts are the runner's, scene by scene" \
    bash -c "[ \"\$(awk '\$1 ~ /^[0-9][0-9]\$/ { for (i = 2; i <= NF; i++) if (\$i ~ /^(ok|gpu-fault|hang|crash|oom|gl-error|fail|not-run)\$/) { printf \"%s \", \$i; break } }' '$tmp/$scen/table.txt')\" = '$want' ]"
  check "$scen: the tally's summary line" grep -qF "$sum" "$tmp/$scen/table.txt"
  case $scen in clean) pics='match 2, mismatch 1, no-reference 30' ;;
    faults) pics='absent 1, match 1, mismatch 1, no-reference 30' ;; esac
  check "$scen: the pictures against the fixture's mali column: $pics" \
    grep -qx "pictures against column mali: 33 scenes, $pics" "$tmp/$scen/table.txt"
  tally --picture-lines --table "$FIX/pictures.txt" "$S" > "$tmp/$scen/picture-lines.txt" 2> "$tmp/$scen/picture-lines.err"
  check "$scen: --picture-lines are the runner's pictures.txt, line for line" \
    bash -c "[ \$(grep -c '^mali | ' '$tmp/$scen/picture-lines.txt') -ge 32 ] &&
      grep -v '^#' '$O/pictures.txt' | cmp - '$tmp/$scen/picture-lines.txt'"
done
# The drafts, on the faults run: the commit, the faults and the hang named.
S=$tmp/faults/sorted
tally --results-row --table "$FIX/pictures.txt" "$S" > "$tmp/results-row.txt" 2> "$tmp/results-row.err"
check "--results-row: a NanoPC-T6 row from main ${c1:0:8}, 4 failed, scene 5 the hang" \
  bash -c "[ ! -s '$tmp/results-row.err' ] && grep -q '^| NanoPC-T6 | .*33 of 33 scenes ran, 4 failed, .*main \`${c1:0:8}\`' '$tmp/results-row.txt' &&
    grep -q '^  | 5 | \`\[texture\] texture-filter=mipmap\` | .* | hang' '$tmp/results-row.txt' &&
    [ \$(grep -c '^  | [0-9]* | .* | gpu-fault' '$tmp/results-row.txt') = 3 ]"
tally --status-row --table "$FIX/pictures.txt" "$S" > "$tmp/status-row.txt" 2> "$tmp/status-row.err"
check "--status-row: one NanoPC-T6 line from main ${c1:0:8}" \
  bash -c "[ ! -s '$tmp/status-row.err' ] && [ \$(grep -c . '$tmp/status-row.txt') = 1 ] &&
    grep -q '^- \*\*NanoPC-T6（.* main ${c1:0:8} から .*実行 33/33、正常 29、失敗 4' '$tmp/status-row.txt'"
tally --causes "$S" > "$tmp/causes.txt" 2> "$tmp/causes.err"
check "--causes: draft issues for the GPU faults and the hang, none marked as plan #309's" \
  bash -c "[ ! -s '$tmp/causes.err' ] && grep -qi 'hang' '$tmp/causes.txt' && grep -qi 'gpu-fault\|GPU fault' '$tmp/causes.txt' &&
    ! grep -q '#309' '$tmp/causes.txt'"

total=$((passed + failed))
echo "glmark2-mali-e2e-selftest: $total checks: $passed ok, $failed failed"
if [ "$failed" -ne 0 ] || [ "$total" -ne "$EXPECT" ]; then
  echo "glmark2-mali-e2e-selftest: the self-test pins $EXPECT checks" >&2
  for f in build.txt clean/run.txt faults/run.txt clean/table.err faults/table.err; do echo "--- $f"; tail -12 "$tmp/$f" 2> /dev/null; done >&2
  exit 1
fi
