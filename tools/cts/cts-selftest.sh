#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/cts-selftest.sh
#
# Host-only checks of the tools/cts parsers. No board, no GPU, no network.
# Each check says what it feeds in and what it expects, so a failure can be
# read rather than guessed at.
#
# Checks 1-3 tested the wedge detectors of the Linux dEQP runner
# (cts-board-run.sh's fence-timeout pattern and exit-status escalation,
# cts-run.sh's one-case-failure recovery). Those scripts drove dEQP over
# /dev/pvrgpu and /dev/gcgpu, whose kernel modules are retired
# (driver/tools/pvrlab/README.md, driver/tools/gclab/README.md), so the
# checks went with them; they are in the history at 49058ba4. Check 16 keeps
# its number so old logs still read.
#
# Checks 23-25 gate tools that no other gate ran: 23 tree-diff-selftest.sh
# (tools/cts/tree-diff.py on its fixtures), 24 the licence audit's own fixture
# test (tools/license_audit.py --self-test, tools/testdata/license_audit/),
# 25 the audit over the repository (fails on missing-spdx, unknown-id and the
# mismatch classes: a new file needs an SPDX line or an allowlist row,
# docs/release/license-audit.md).
set -u

HERE=$(dirname "$0")
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

echo
# ---- 16: cts-refusals.py parses the message format with a token ------------
#
# The format changed under this tool once (fc88b41 added `'<tok>' :`) and the
# tool went silently EMPTY rather than failing, because dEQP escapes the
# token's quotes as `&apos;` and the pattern stopped at `&`. A sweep run on
# that would have cost 40 minutes of board time and produced one nameless row.
# This uses the fixture in testdata/ rather than the `check` helper above,
# which compares a decide() verdict and takes different arguments.
echo
echo "== 16. cts-refusals.py on the tokened format (testdata fixture)"
rout=$(python3 "$HERE/cts-refusals.py" --cases 1 "$HERE/testdata" 2>&1)
for want in "assignment to this is not lowered yet" \
            "the code generator refused this fragment shader: Select(OutOfScratch)" \
            "on cases dEQP FAILED (real gaps)        : 4" \
            "on cases dEQP NOT SUPPORTED (excused)   : 1" \
            "on cases dEQP PASSED (correct refusals) : 1" \
            "Usc(Record(MatrixAttrib { index: 0 }))" \
            "no implicit conversion in GLSL ES 1.00"; do
    case "$rout" in
        *"$want"*) ok   "refusals: reports \"$want\"" ;;
        *)         bad  "refusals: MISSING \"$want\"" ;;
    esac
done
# the per-construct split: one message, two tokens
if printf '%s\n' "$rout" | grep -qE '^ +1 +\.$' &&
   printf '%s\n' "$rout" | grep -qE '^ +1 +\[\]$'; then
    ok  "refusals: splits one message by construct (. and [])"
else
    bad "refusals: did NOT split by construct -- the token was lost again"
fi

echo
# ---- 17: check-deqp-images.py --config on fixture .config files ------------
#
# The dEQP image builders (am62p-deqp-images.sh, pi4-deqp-images.sh,
# imx8mp-deqp-build.sh) run check-deqp-images.py --config on the
# zephyr/.config each build writes and mark an image that fails it
# (rc=not-conformance). That post-build check had never run: the workers
# have no Zephyr build. These fixtures are resolved configurations in the
# form a build writes them -- CONFIG_X=value lines, a disabled bool as
# "# CONFIG_X is not set", ints and strings always present -- one good per
# board plus the ms4 image, and one bad per rule --config enforces. A bad
# fixture must fail with a message naming the offending symbol, so a real
# board failure reads as the config line that caused it.
echo
echo "== 17. check-deqp-images.py --config on fixture .config files (testdata/deqp-config)"
CONF_DIR=$HERE/testdata/deqp-config
conf_ok() { # <fixture> [extra args]: the resolved config must pass the check
    local f=$1; shift
    if cout=$(python3 "$HERE/check-deqp-images.py" --config "$CONF_DIR/$f" "$@" 2>&1); then
        ok "config $f: a conformance image passes"
    else
        bad "config $f: should pass; the check said: $cout"
    fi
}
conf_bad() { # <fixture> <symbol> [extra args]: must fail, naming the symbol
    local f=$1 sym=$2; shift 2
    cout=$(python3 "$HERE/check-deqp-images.py" --config "$CONF_DIR/$f" "$@" 2>&1)
    if [ $? -eq 0 ]; then
        bad "config $f: should FAIL, but the check passed it"
    elif printf '%s\n' "$cout" | grep -q "$sym"; then
        ok "config $f: refused, the message names $sym"
    else
        bad "config $f: failed but did not name $sym; said: $cout"
    fi
}
conf_ok  am62p-sweep.config
conf_ok  am62p-ms4.config --ms4
conf_ok  imx8mp-sweep.config
conf_ok  pi4-sweep.config
conf_bad bad-compiler-missing.config YAKOGL_SHADERC_ONLINE
conf_bad bad-shaderc-offline.config YAKOGL_SHADERC_OFFLINE
conf_bad bad-precompiled-only.config YAKOGL_SHADERC_PRECOMPILED_ONLY
conf_bad bad-precompiled-tier.config YAKOGL_PRECOMPILED_SHADERS
conf_bad bad-report-es3.config YAKOGL_REPORT_ES2
conf_bad bad-gles3-report.config DEQP_REPORT_ES2
conf_ok  am62p-gles3.config --gles3
conf_bad bad-gles3-report.config YAKOGL_REPORT_ES2 --gles3
conf_bad am62p-sweep.config YAKOGL_REPORT_ES2 --gles3
conf_bad bad-sweep-ms4-name.config DEQP_GL_CONFIG_NAME
conf_bad bad-ms4-name-missing.config DEQP_GL_CONFIG_NAME --ms4
# the dEQP-GLES3 image (pi4-deqp-images.sh DEQP_MODULE=gles3 checks with
# --gles3): both REPORT_ES2 symbols off passes there and fails the GLES2 rule;
# a GLES2 sweep image judged as GLES3 fails naming REPORT_ES2
conf_ok  pi4-gles3-sweep.config --gles3
conf_bad pi4-gles3-sweep.config YAKOGL_REPORT_ES2
conf_bad pi4-sweep.config REPORT_ES2 --gles3

echo
# ---- 19: check-gles3-tree.py: every GLES3 mustpass case once per table -------
#
# The dEQP-GLES3 tables (am62p-gles3-tree.txt, pi4-gles3-tree.txt,
# imx8mp-gles3-tree.txt) are split by hand to fit each board's watchdog
# window, and a split by hand drops or doubles a case easily (RESULTS.md 1r
# split its line 19 into 19a-19g and named 3D lists). The checker's self-test
# covers its rules; the three tables must then each select the 42,810 cases of
# gles3-mustpass.txt exactly once (the same three check-deqp-images.py's repo
# rule 6 reads).
echo
echo "== 19. check-gles3-tree.py: self-test, then the GLES3 tables against gles3-mustpass.txt"
if python3 -I "$HERE/check-gles3-tree.py" --self-test > /dev/null 2>&1; then
    ok "check-gles3-tree.py --self-test"
else
    bad "check-gles3-tree.py --self-test failed: $(python3 -I "$HERE/check-gles3-tree.py" --self-test 2>&1 | grep FAIL)"
fi
for t in am62p-gles3-tree.txt pi4-gles3-tree.txt imx8mp-gles3-tree.txt; do
    gout=$(python3 -I "$HERE/check-gles3-tree.py" "$HERE/$t" 2>&1)
    grc=$?
    gline=$(printf '%s\n' "$gout" | grep '^check-gles3-tree: ')
    if [ "$grc" -eq 0 ] && printf '%s\n' "$gline" | grep -q ' 42810 matched once, 0 not matched, 0 matched more than once, 0 empty images$'; then
        ok "$t: ${gline#check-gles3-tree: }"
    else
        bad "$t: rc $grc; $(printf '%s\n' "$gout" | grep -E 'FAIL|^check-gles3-tree: ' | head -3 | paste -sd';' -)"
    fi
done

# 20. tools/cts/images-selftest.sh: am62p-deqp-images.sh and
# imx8mp-gltest-build.sh against a fake west in a scratch layout (TREE_ROWS,
# a failed or image-less build, the WDOG self-arm flag, refused arguments),
# pi4-deqp-images.sh against a fake cmake/ninja (the gles2 dry run and
# build exactly as before, the gles3 path's three arguments and --gles3
# check, the stage 1 check), and needed_deps against what the builders read.
# Its count of passed checks is pinned, so a selftest that stops checking
# reads as a failure here, not as a pass -- and a new check in
# images-selftest.sh without a bump here is a red row too (pull request
# #1838 failed exactly there).
IMAGES_SELFTEST_CHECKS=109
echo
echo "== 20. images-selftest.sh: the image builders against a fake west"
iout=$("$HERE/images-selftest.sh" 2>&1)
irc=$?
iok=$(printf '%s\n' "$iout" | grep -c '^  ok ')
if [ "$irc" -eq 0 ] && [ "$iok" -eq "$IMAGES_SELFTEST_CHECKS" ]; then
    ok "images-selftest.sh: $iok checks passed"
else
    bad "images-selftest.sh: rc $irc, $iok of $IMAGES_SELFTEST_CHECKS ok; $(printf '%s\n' "$iout" | grep 'FAIL' | head -3 | paste -sd';' -)"
fi

# 20b. tools/cts/deps-selftest.sh: deps.toml and deps-env.sh (--check, the
# board sets, --packages, --build / --build-missing against a fake deps.toml in
# a scratch layout, and the MALI_CSFFW entry's build on a stand-in download).
# Its count is pinned like images-selftest's.
DEPS_SELFTEST_CHECKS=51
echo
echo "== 20b. deps-selftest.sh: deps.toml, deps-env.sh --check / --build"
dout=$("$HERE/deps-selftest.sh" 2>&1)
drc=$?
dok=$(printf '%s\n' "$dout" | grep -c '^  ok ')
if [ "$drc" -eq 0 ] && [ "$dok" -eq "$DEPS_SELFTEST_CHECKS" ]; then
    ok "deps-selftest.sh: $dok checks passed"
else
    bad "deps-selftest.sh: rc $drc, $dok of $DEPS_SELFTEST_CHECKS ok; $(printf '%s\n' "$dout" | grep 'FAIL' | head -3 | paste -sd';' -)"
fi

# ---- 21: cts-summary.py --by-message tallies failures by their detail -------
#
# host-deqp-run.sh writes this tally into every run's summary.txt, and
# ab43a20c5:tools/cts/log/es3-host-run.md maps it onto RESULTS.md 1r's per-message table. The
# fixture (testdata/by-message.qpa, a batch log as deqp writes it) holds one
# Pass and three Fail: two whose details differ only in the shader line
# `0:<n>:`, which must fold into one row, and one other. cts-summary.py reads
# .qpa with the CTS's own log parser, so this needs a VK-GL-CTS checkout
# ($CTS) and is skipped without one.
echo
echo "== 21. cts-summary.py --by-message (testdata/by-message.qpa)"
if [ -f "${CTS:-$HOME/com/github/KhronosGroup/VK-GL-CTS}/scripts/log/log_parser.py" ]; then
    sout=$(python3 "$HERE/cts-summary.py" --by-message 5 --list-failures 0 \
        "$HERE/testdata/by-message.qpa" 2>&1)
    for want in "failures by message (2 distinct, 3 cases):" \
                "       2  Compile failed: 0:N: not lowered yet" \
                "       1  Got invalid error"; do
        case "$sout" in
            *"$want"*) ok   "by-message: reports \"$want\"" ;;
            *)         bad  "by-message: MISSING \"$want\"" ;;
        esac
    done
else
    echo "  skip  no VK-GL-CTS log parser under \$CTS"
fi

echo
# ---- 22: common-tally.py judges the GM row per scene (testdata/glmark2-*) ----
#
# A GLMARK2 record says how many scenes ran, not whether they drew right:
# terrain's "GL error 0x502 in glGenerateMipmap", refract's refused
# depth-pass draw and the Pi 4's "out of GPU memory" lines all sat under a
# green record (tools/cts/RESULTS.md). The tally reads the GM run's
# console.txt, gives each such line to its scene, and makes the cell amber.
# Two Pi 4 GM run directories are built in a scratch directory from the
# fixtures, with the round-3 record (183, 33/0/0, pass): the Pi 4
# out-of-memory console as the earlier run, the terrain console as the new
# one. The row and the --prev diff must name the scenes; the exit stays 0
# (amber is a cell, the verdict is done).
echo
echo "== 22. common-tally.py: the GM row per scene, and its --prev diff (testdata/glmark2-*)"
gmtmp=$(mktemp -d)
gm_dir() { # <fixture> <subdir>: a Pi 4 GM run directory whose console is the fixture and the record
    local d=$gmtmp/$2/20261004-1520-GM-d0932b24
    mkdir -p "$d"
    printf 'board=pi4\nboot=tryboot\nname=glmark2\nprobe=0\nimage=zephyr.bin\nimage_sha256=d0932b243a844247\nimage_build=YAKOGL_SAMPLE_TREE=42bd90ff\nt_reboot=100\nt_back=417\nback=1\n' > "$d/meta.txt"
    { cat "$HERE/testdata/glmark2-$1.txt"; echo "15:47:01 yk-record: 594b1201 fc3f00b7 2184003f 03200258"; } > "$d/console.txt"
    echo "$d"
}
gm_prev=$(gm_dir pi4-oom prev)
gm_now=$(gm_dir terrain-0x502 now)
gout=$(python3 "$HERE/common-tally.py" pi4 --row "GM=$gm_now" --prev-row "GM=$gm_prev" 2>&1)
grc=$?
rm -rf "$gmtmp"
for want in "GM glmark2 rc=0 verdict=done result=record:GLMARK2" \
            "glmark2 cell amber; scenes in the console 2, clean 1, with known-bad lines 1" \
            "scene [terrain] <default>: amber, 17 lines (gl-error 16, incomplete-texture 1)" \
            "scene [terrain] <default>: gl-error 0 -> 16, incomplete-texture 0 -> 1, tiled-copy 2 -> 0" \
            "scene [refract] <default>: out-of-memory 1 -> 0, tiled-copy 3 -> 0"; do
    case "$gout" in
        *"$want"*) ok   "GM row: reports \"$want\"" ;;
        *)         bad  "GM row: MISSING \"$want\"" ;;
    esac
done
if [ "$grc" -eq 0 ]; then
    ok "GM row: exit 0 (amber is the cell; the verdict is done)"
else
    bad "GM row: exit $grc, expected 0"
fi

# The remark "no glmark2-mem lines" belongs to a console without any. The sample prints
# "glmark2-mem: entry N before setup|end: ..." (no scene name): that console gets no remark,
# the refract console (no glmark2-mem line at all) keeps it.
echo
echo "== 22a. common-tally.py: the no-glmark2-mem remark only on a console without the lines"
gmtmp=$(mktemp -d)
for fx in mem-nameless refract-refused; do
    d=$gmtmp/$fx/20261004-1520-GM-d0932b24
    mkdir -p "$d"
    printf 'board=pi4\nboot=tryboot\nname=glmark2\nprobe=0\nimage=zephyr.bin\nimage_sha256=d0932b243a844247\nimage_build=YAKOGL_SAMPLE_TREE=42bd90ff\nt_reboot=100\nt_back=417\nback=1\n' > "$d/meta.txt"
    { cat "$HERE/testdata/glmark2-$fx.txt"; echo "15:47:01 yk-record: 594b1201 fc3f00b7 2184003f 03200258"; } > "$d/console.txt"
done
mout=$(python3 "$HERE/common-tally.py" pi4 --row "GM=$gmtmp/mem-nameless/20261004-1520-GM-d0932b24" 2>&1)
rout=$(python3 "$HERE/common-tally.py" pi4 --row "GM=$gmtmp/refract-refused/20261004-1520-GM-d0932b24" 2>&1)
rm -rf "$gmtmp"
case "$mout" in
    *"scenes in the console 2, clean 1, with known-bad lines 1"*) ok "GM row, glmark2-mem lines without a scene name: both scenes read" ;;
    *) bad "GM row, glmark2-mem lines without a scene name: scenes not read" ;;
esac
case "$mout" in
    *"no glmark2-mem lines"*) bad "GM row: the remark \"no glmark2-mem lines\" on a console that has them" ;;
    *) ok "GM row: no \"no glmark2-mem lines\" remark on a console that has them" ;;
esac
case "$rout" in
    *"(no glmark2-mem lines: "*) ok "GM row: the remark stays on a console without glmark2-mem lines" ;;
    *) bad "GM row: the remark is MISSING on a console without glmark2-mem lines" ;;
esac

echo
# ---- 22b: common-tally.py --gm-table and --gm-diff (ms per frame per scene) ----
#
# Speed is read per scene in ms per frame (1000 / FPS) from same-board glmark2
# runs. The table pins the ms column, the gl-call time and the "end waited"
# count on the existing clean console; the diff pins the delta in ms and %,
# the 5% mark, a scene with no FPS or in one run only, and both scores on the
# two synthetic runs testdata/glmark2-speed-{a,b}.txt.
echo "== 22b. common-tally.py --gm-table / --gm-diff (testdata/glmark2-speed-*)"
tout=$(python3 "$HERE/common-tally.py" --gm-table "$HERE/testdata/glmark2-clean.txt" 2>&1)
for want in "[terrain] <default>                                 6    166.67     9.12  61 of 61" \
            "[conditionals] fragment-steps=0:vertex-steps=0     58     17.24     0.61  581 of 581" \
            "score 183; 5 scenes, 5 with an FPS, 492.35 ms/frame summed (5 from FrameTime, 0 from 1000/FPS)"; do
    case "$tout" in
        *"$want"*) ok   "gm-table: reports \"$want\"" ;;
        *)         bad  "gm-table: MISSING \"$want\"" ;;
    esac
done
dout=$(python3 "$HERE/common-tally.py" --gm-diff "$HERE/testdata/glmark2-speed-a.txt" \
       "$HERE/testdata/glmark2-speed-b.txt" 2>&1)
for want in "[build] <default>          10.00      9.80     -0.20    -2.0%" \
            "[shadow] <default>         50.00     40.00    -10.00   -20.0%  *" \
            "[terrain] <default>       125.00    125.00     +0.00    +0.0%" \
            "[refract] <default>            -    100.00         -        -  no FPS" \
            "[jellyfish] <default>          -     25.00         -        -  only in B" \
            "scenes moved by more than 5%: 1 of 5 (marked *)" \
            "score A 150, B 160 (+6.7%)"; do
    case "$dout" in
        *"$want"*) ok   "gm-diff: reports \"$want\"" ;;
        *)         bad  "gm-diff: MISSING \"$want\"" ;;
    esac
done
if printf '%s\n' "$dout" | grep -F '[build] <default>' | grep -qF '*'; then
    bad "gm-diff: a 2% move was marked"
else
    ok "gm-diff: a 2% move is not marked"
fi
# The same on a console shaped like a real one (testdata/glmark2-speed-late-{a,b}.txt): upstream
# prints "<info>:" when a scene starts and " FPS: n FrameTime: x ms" when it ends, so the driver's
# own lines and GL errors land between and the result line carries no scene. The five scenes the
# parser lost at e9f562d8 (use-vbo=false, gouraud, blinn-phong-inf, terrain, shadow) are in it;
# refract has FPS 3 and FrameTime 403.8, jellyfish an FPS and no FrameTime.
ldout=$(python3 "$HERE/common-tally.py" --gm-diff "$HERE/testdata/glmark2-speed-late-a.txt" \
        "$HERE/testdata/glmark2-speed-late-b.txt" 2>&1)
for want in "[build] use-vbo=false                   6.38      6.25     -0.13    -2.0%" \
            "[shading] shading=gouraud               4.41      4.00     -0.41    -9.3%  *" \
            "[shading] shading=blinn-phong-inf       4.58      4.58     +0.00    +0.0%" \
            "[terrain] <default>                   181.65    125.00    -56.65   -31.2%  *" \
            "[shadow] <default>                    175.74     50.00   -125.74   -71.5%  *" \
            "[refract] <default>                   403.80    200.00   -203.80   -50.5%  *" \
            "[jellyfish] <default>                  50.00     40.00    -10.00   -20.0%  * (A: 1000/FPS) (B: 1000/FPS)" \
            "[ideas] speed=duration                     -         -         -        -  no FPS" \
            "scenes moved by more than 5%: 5 of 9 (marked *)" \
            "ms/frame: A 7 from FrameTime, 1 from 1000/FPS; B 7 from FrameTime, 1 from 1000/FPS"; do
    case "$ldout" in
        *"$want"*) ok   "gm-diff late FPS: reports \"$want\"" ;;
        *)         bad  "gm-diff late FPS: MISSING \"$want\"" ;;
    esac
done
ltout=$(python3 "$HERE/common-tally.py" --gm-table "$HERE/testdata/glmark2-speed-late-a.txt" 2>&1)
for want in "[refract] <default>                    3    403.80   390.00  30 of 30    FrameTime" \
            "[jellyfish] <default>                 20     50.00        -  -           1000/FPS" \
            "score 150; 9 scenes, 8 with an FPS, 830.19 ms/frame summed (7 from FrameTime, 1 from 1000/FPS)"; do
    case "$ltout" in
        *"$want"*) ok   "gm-table late FPS: reports \"$want\"" ;;
        *)         bad  "gm-table late FPS: MISSING \"$want\"" ;;
    esac
done
python3 "$HERE/common-tally.py" --gm-table "$HERE/testdata/README.md" >/dev/null 2>&1
if [ $? -eq 2 ]; then
    ok "gm-table: a file without scene lines is a usage error (exit 2)"
else
    bad "gm-table: a file without scene lines did not exit 2"
fi

# Rare shapes of the console: a header with driver text glued to it and no glmark2-mem /
# glmark2-frame line (the options are read in full, up to the first text that is no option); one
# scene run twice before its frame line arrives (each frame line goes to its own run); a run that
# set up failed before the run with an FPS (a failed run prints no frame line, so the line goes
# to the run with an FPS).
echo
echo "== 22b2. common-tally.py --gm-table: a glued header, a scene run twice, a failed run first"
b2_cases=(glued-header:"[build] use-vbo=false:interleave=true    157      6.38"
          glued-header:"[shading] shading=gouraud                227      4.41"
          twice-before-frame:"[build] use-vbo=false       157      6.38     2.73  1 of 1568"
          twice-before-frame:"[build] use-vbo=false #2    100     10.00     9.00  7 of 1000"
          failed-then-fps:"[build] use-vbo=false #2    157      6.38     2.73  1 of 1568")
for case in "${b2_cases[@]}"; do
    f=${case%%:*}; want=${case#*:}
    gout=$(python3 "$HERE/common-tally.py" --gm-table "$HERE/testdata/glmark2-$f.txt" 2>&1)
    case "$gout" in
        *"$want"*) ok  "gm-table $f: reports \"$want\"" ;;
        *)         bad "gm-table $f: MISSING \"$want\"" ;;
    esac
done

echo
# ---- 22c: common-tally.py judges the glmark2 pictures (testdata/glmark2-picture-*) ----
#
# The "glmark2-picture:" lines are compared per scene with a committed reference
# table (glmark2-pictures.txt, a column per board). The fixtures are judged
# against testdata/glmark2-pictures-test.txt (a pi4 column of three scenes):
# match (a probe 3 levels off, crc16 differs: still a match), mismatch
# (terrain's centre probe and mean), all-black (shadow) and absent (terrain's
# readback failed), through a Pi 4 GM run directory (the cell) and
# --gm-pictures; a board with no column (am62p) has no reference. The
# production table must have its host column of the 33 default scenes.
echo
echo "== 22c. common-tally.py: the glmark2 pictures against the reference table (testdata/glmark2-picture-*)"
pictab="$HERE/testdata/glmark2-pictures-test.txt"
pictmp=$(mktemp -d)
pic_dir() { # <fixture>: a Pi 4 GM run directory whose console is the fixture and the round-3 record
    local d=$pictmp/$1/20261004-1520-GM-d0932b24
    mkdir -p "$d"
    printf 'board=pi4\nboot=tryboot\nname=glmark2\nprobe=0\nimage=zephyr.bin\nimage_sha256=d0932b243a844247\nimage_build=YAKOGL_SAMPLE_TREE=42bd90ff\nt_reboot=100\nt_back=417\nback=1\n' > "$d/meta.txt"
    { cat "$HERE/testdata/glmark2-picture-$1.txt"; echo "15:47:01 yk-record: 594b1201 fc3f00b7 2184003f 03200258"; } > "$d/console.txt"
    echo "$d"
}
pic_row() { # <fixture>: the pi4 row output against the test table
    COMMON_TALLY_PICTURES="$pictab" python3 "$HERE/common-tally.py" pi4 --row "GM=$(pic_dir "$1")" 2>&1
}
pic_expect() { # <name> <output> <wanted line>...
    local name=$1 out=$2 want
    shift 2
    for want in "$@"; do
        case "$out" in
            *"$want"*) ok   "pictures $name: reports \"$want\"" ;;
            *)         bad  "pictures $name: MISSING \"$want\"" ;;
        esac
    done
}
pic_expect match "$(pic_row match)" \
    "glmark2 cell green; scenes in the console 3" \
    "glmark2 pictures against column pi4: 3 scenes, match 3" \
    "scene [terrain] <default>: picture match (crc16 4321, reference 1234)"
pic_expect mismatch "$(pic_row mismatch)" \
    "glmark2 cell amber; scenes in the console 3" \
    "glmark2 pictures against column pi4: 3 scenes, match 2, mismatch 1" \
    "scene [terrain] <default>: picture mismatch (probe c 7a2020 vs 7a9ec0 (off 160); mean 90 60 50 vs 90 120 100 (off 60))"
pic_expect all-black "$(pic_row black)" \
    "glmark2 cell failed; scenes in the console 3" \
    "glmark2 pictures against column pi4: 3 scenes, match 2, black 1" \
    "scene [shadow] <default>: picture black (all black, the reference is not)"
pic_expect absent "$(pic_row absent)" \
    "glmark2 cell amber; scenes in the console 3" \
    "scene [terrain] <default>: picture absent (the readback failed)"
# a value that does not parse (one extra character) is unreadable with its raw text; only a literal
# "absent ..." line is absent
pic_expect unreadable "$(pic_row unreadable)" \
    "glmark2 cell amber; scenes in the console 3" \
    "glmark2 pictures against column pi4: 3 scenes, match 1, absent 1, unreadable 1" \
    "scene [terrain] <default>: picture unreadable (does not parse: 800x600; probes c 7a9ec0ff, tl 5f8fd0ff," \
    "scene [shadow] <default>: picture absent (the readback failed)"
# a mismatch is the cell; the verdict stays done, so the row exits 0 (--gm-pictures exits 1)
COMMON_TALLY_PICTURES="$pictab" python3 "$HERE/common-tally.py" pi4 --row "GM=$(pic_dir mismatch)" >/dev/null 2>&1
if [ $? -eq 0 ]; then
    ok "pictures mismatch: the row exits 0 (amber is the cell; the verdict is done)"
else
    bad "pictures mismatch: the row did not exit 0"
fi
python3 "$HERE/common-tally.py" --gm-pictures "$HERE/testdata/glmark2-picture-match.txt" pi4 --table "$pictab" >/dev/null 2>&1
mrc=$?
python3 "$HERE/common-tally.py" --gm-pictures "$HERE/testdata/glmark2-picture-mismatch.txt" pi4 --table "$pictab" >/dev/null 2>&1
xrc=$?
if [ "$mrc" -eq 0 ] && [ "$xrc" -eq 1 ]; then
    ok "--gm-pictures: exit 0 on a match, 1 on a mismatch"
else
    bad "--gm-pictures: exit $mrc on a match (want 0), $xrc on a mismatch (want 1)"
fi
nref=$(python3 "$HERE/common-tally.py" --gm-pictures "$HERE/testdata/glmark2-picture-match.txt" am62p --table "$pictab" 2>&1)
pic_expect no-reference "$nref" \
    "glmark2 pictures against column am62p: 3 scenes, match 0, no-reference 3" \
    "scene [jellyfish] <default>: picture no reference"
rm -rf "$pictmp"
# --gm-picture-lines prints a console as table lines, with the console's own text
plines=$(python3 "$HERE/common-tally.py" --gm-picture-lines "$HERE/testdata/glmark2-picture-match.txt" pi4 2>/dev/null | grep -c '^pi4 | \[')
if [ "$plines" -eq 3 ]; then
    ok "--gm-picture-lines: 3 table lines for the pi4 column"
else
    bad "--gm-picture-lines: $plines line(s), want 3"
fi
hostn=$(grep -c '^host | ' "$HERE/glmark2-pictures.txt")
if [ "$hostn" -eq 33 ]; then
    ok "glmark2-pictures.txt: a host column of 33 scenes (the parse is checked by --self-test)"
else
    bad "glmark2-pictures.txt: host column has $hostn line(s), want 33"
fi

# ---- 22d: common-tally.py reads glmark2-pages.txt on the i.MX8MP (testdata/imx8mp-pages) ----
#
# The i.MX8MP has no console: `tools/board.sh imx8mp gm-lines <outdir>` decodes the page records
# into <outdir>/glmark2-pages.txt (driver/docs/glmark2.md, "Per scene without a console").
# testdata/imx8mp-pages/glmark2-pages.txt is that decoder's output on the 18 synthetic page runs of
# driver/rust/tests/gm-record/runs (tools/board_selftest.sh and the record tests pin it); the
# GM run directory beside it is a synthetic record. Checked: the GM row's speed and picture
# verdicts, what the record drops shown as "not in record" (no failure), and the table, diff
# and picture modes taking the file.
echo
echo "== 22d. common-tally.py: the i.MX8MP GM row from glmark2-pages.txt (testdata/imx8mp-pages)"
pg="$HERE/testdata/imx8mp-pages"
pgout=$(python3 "$HERE/common-tally.py" imx8mp "$pg" 2>&1)
pg_expect() { # <name> <output> <wanted text>...
    local name=$1 out=$2 want
    shift 2
    for want in "$@"; do
        case "$out" in
            *"$want"*) ok   "pages $name: reports \"$want\"" ;;
            *)         bad  "pages $name: MISSING \"$want\"" ;;
        esac
    done
}
pg_expect row "$pgout" \
    "glmark2 cell amber; scenes in glmark2-pages.txt 30, with an FPS 28, 515.67 ms/frame summed" \
    "not in record: FrameTime, the gl / swap / show / frame means, the glmark2-mem lines, the GL error lines" \
    "glmark2 pictures against column imx8mp: 4 scenes, match 0, mismatch 2, absent 1, not-in-record 1" \
    "scene [terrain] <default>: picture mismatch (probe c 6080a0 vs " \
    "scene [jellyfish] <default>: picture absent (the readback failed)" \
    "scene [conditionals] fragment-steps=0:vertex-steps=0: picture not in record (page P0 of scene 25 missing)"
# a missing page is no failure: the row stays verdict done and exits 0 (a mismatch is only the cell)
case "$pgout" in
    *"rows 1, with a run 1, without 0; verdict done 1, other 0"*) ok "pages row: verdict done" ;;
    *) bad "pages row: verdict done 1 missing" ;;
esac
pgtab=$(python3 "$HERE/common-tally.py" --gm-table "$pg/glmark2-pages.txt" 2>&1)
pg_expect table "$pgtab" \
    "[terrain] <default>" \
    "score not in record; 30 scenes, 28 with an FPS, 515.67 ms/frame summed (0 from FrameTime, 28 from 1000/FPS)" \
    "not in record  not in record 1000/FPS" \
    "FrameTime, the gl / swap / show / frame means and the GL error lines are not in record"
case "$pgtab" in
    *"[terrain] <default>"*" 6 "*"166.67 not in record"*) ok "pages table: terrain 6 FPS, 166.67 ms/frame" ;;
    *) bad "pages table: terrain row not 6 FPS, 166.67 ms/frame" ;;
esac
pgdiff=$(python3 "$HERE/common-tally.py" --gm-diff "$pg/glmark2-pages.txt" "$pg/glmark2-pages.txt" 2>&1)
pg_expect diff "$pgdiff" \
    "scenes moved by more than 5%: 0 of 30" \
    "ms/frame: A 0 from FrameTime, 28 from 1000/FPS; B 0 from FrameTime, 28 from 1000/FPS" \
    "score A not in record, B not in record"
pgpic=$(python3 "$HERE/common-tally.py" --gm-pictures "$pg/glmark2-pages.txt" imx8mp 2>&1)
pg_expect pictures "$pgpic" \
    "glmark2 pictures against column imx8mp: 4 scenes, match 0, mismatch 2, absent 1, not-in-record 1"
# the same file in a run directory is found there; a console beats it (no console in the fixture)
pgrun=$(mktemp -d)
mkdir "$pgrun/GM-run"
cp "$pg"/20261018-0000-GM-aaaaaaaa/*.txt "$pgrun/GM-run/"
cp "$pg/glmark2-pages.txt" "$pgrun/GM-run/"
pgrow=$(python3 "$HERE/common-tally.py" imx8mp --row "GM=$pgrun/GM-run" 2>&1)
pg_expect run-dir "$pgrow" "glmark2 cell amber; scenes in glmark2-pages.txt 30"
rm "$pgrun/GM-run/glmark2-pages.txt"
pgnone=$(python3 "$HERE/common-tally.py" imx8mp --row "GM=$pgrun/GM-run" 2>&1)
pg_expect without "$pgnone" "glmark2 cell green (record only); per-scene lines absent (no console)"
rm -rf "$pgrun"

echo
# ---- 23: tree-diff-selftest.sh ---------------------------------------------
echo
echo "== 23. tree-diff-selftest.sh: tree-diff.py on its fixtures (testdata/tree-diff)"
tdout=$(bash "$HERE/tree-diff-selftest.sh" 2>&1)
tdrc=$?
tdpass=$(printf '%s\n' "$tdout" | grep -c '^  ok ')
if [ "$tdrc" -eq 0 ] && [ "$tdpass" -gt 0 ]; then
    ok "tree-diff-selftest.sh: $tdpass check(s) passed"
else
    bad "tree-diff-selftest.sh: exit $tdrc, $tdpass ok line(s)"
    printf '%s\n' "$tdout" | grep -v '^  ok ' | sed 's/^/        | /'
fi

echo
# ---- 24: licence audit self-test -------------------------------------------
echo
echo "== 24. license_audit.py --self-test (tools/testdata/license_audit)"
LA=$HERE/../license_audit.py
laout=$(python3 -I "$LA" --self-test 2>&1)
larc=$?
lapass=$(printf '%s\n' "$laout" | grep -c '^ok ')
if [ "$larc" -eq 0 ] && [ "$lapass" -gt 0 ] &&
   printf '%s\n' "$laout" | grep -q '^self-test: passed'; then
    ok "license_audit.py --self-test: $lapass fixture check(s) passed"
else
    bad "license_audit.py --self-test: exit $larc, $lapass ok line(s)"
    printf '%s\n' "$laout" | sed 's/^/        | /'
fi

echo
# ---- 25: licence audit, strict on the decided classes -----------------------
#
# A tracked file without an SPDX line (or an allowlist row in
# tools/license_audit.allow, or a .license sidecar) fails: missing-spdx must be
# 0, and so must unknown-id (an identifier with no text in LICENSES/) and the
# two mismatch classes, and unused-license (a text in LICENSES/ no file
# names). `undecided` rows (provenance open, listed in
# docs/release/license-audit.md) are printed as information. A new source file: add
# the two SPDX lines; a new data file: add a row to the allowlist.
echo
echo "== 25. license_audit.py --fail-on missing-spdx,... over the repository"
larep=$(python3 -I "$LA" --fail-on missing-spdx,unknown-id,third-party-mismatch,generated-mismatch,unused-license 2>&1)
larc=$?
launk=$(printf '%s\n' "$larep" | sed -n 's/^unknown-id: \([0-9][0-9]*\)$/\1/p')
lamiss=$(printf '%s\n' "$larep" | sed -n 's/^missing-spdx: \([0-9][0-9]*\)$/\1/p')
laund=$(printf '%s\n' "$larep" | sed -n 's/^undecided: \([0-9][0-9]*\)$/\1/p')
if [ -z "$launk" ] || [ -z "$lamiss" ]; then
    bad "license audit: no unknown-id / missing-spdx line in the report (exit $larc)"
    printf '%s\n' "$larep" | tail -5 | sed 's/^/        | /'
elif [ "$larc" -ne 0 ]; then
    bad "license audit: missing-spdx $lamiss, unknown-id $launk, or another --fail-on class (python3 -I tools/license_audit.py)"
else
    ok "license audit: missing-spdx 0, unknown-id 0; undecided ${laund:-0} (information)"
fi

# ---- 26: glmark2-fb.py decodes a synthetic frame dump ----------------------
#
# The frame dump (CONFIG_GLMARK2_FRAME_DUMP) is read on the host: a clean
# log gives a PNG that agrees with the picture value, a lost and a garbled
# row are named, a wrong picture line is refused.
echo
echo "== 26. glmark2-fb.py --self-test"
if fbout=$(python3 -I "$HERE/glmark2-fb.py" --self-test 2>&1); then
    ok "glmark2-fb.py: clean, damaged and disagreeing dumps judged"
else
    bad "glmark2-fb.py --self-test failed"
    printf '%s\n' "$fbout" | tail -8 | sed 's/^/        | /'
fi

# ---- 27: readme-glmark2-check.py on its fixtures ---------------------------
#
# The live README fails this check by design until its boards are measured at
# one commit, so only fixtures (testdata/readme-glmark2/) are gated here.
# Each case: README fixture, RESULTS fixture, ref, expected exit status and a
# word the output must contain.
echo
echo "== 27. readme-glmark2-check.py (testdata/readme-glmark2)"
RG=$HERE/readme-glmark2-check.py
RGD=$HERE/testdata/readme-glmark2
RG_EXTRA=
rgcase() {
    local name=$1 readme=$2 results=$3 ref=$4 want=$5 word=$6 out rc
    out=$(python3 -I "$RG" --readme "$RGD/$readme" --results "$RGD/$results" --ref "$ref" ${RG_EXTRA:-} 2>&1)
    rc=$?
    if [ "$rc" -eq "$want" ] && printf '%s\n' "$out" | grep -q -- "$word"; then
        ok "$name: exit $rc, output has '$word'"
    else
        bad "$name: exit $rc (want $want), output lacks '$word'"
        printf '%s\n' "$out" | sed 's/^/        | /'
    fi
}
rgcase "one commit, a row per board (short and long hash)" \
    pass-README.md pass-RESULTS.md none 0 '^ok:'
rgcase "unknown ref skips the ancestor check with a warning" \
    pass-README.md pass-RESULTS.md no-such-ref-27 0 'warning: ancestor check skipped'
rgcase "boards name different commits" \
    differ-README.md pass-RESULTS.md none 1 'name 2 commits'
rgcase "a board is missing from the README" \
    missing-board-README.md pass-RESULTS.md none 1 'no glmark2 row for Raspberry Pi 4'
rgcase "RESULTS has no row citing the commit" \
    pass-README.md results-missing-RESULTS.md none 1 'Raspberry Pi 4 glmark2 row does not cite'
rgcase "README without the table is a parse error" \
    noparse-README.md pass-RESULTS.md none 2 '^error:'
# --scenes terrain,shadow,refract: figures and a record directory per board
RG_EXTRA="--scenes terrain,shadow,refract"
rgcase "--scenes: figures and a record dir for each board" \
    pass-README.md scenes-pass-RESULTS.md none 0 '^ok:'
rgcase "--scenes: a row without the three figures or a record dir fails" \
    pass-README.md pass-RESULTS.md none 1 'i.MX8MP glmark2 row has no .terrain <n> ms. figure'
rgcase "--scenes: a scene figure is missing" \
    pass-README.md scenes-missing-RESULTS.md none 1 'Raspberry Pi 4 glmark2 row has no .shadow <n> ms'
rgcase "--scenes: a non-numeric figure" \
    pass-README.md scenes-nonnumeric-RESULTS.md none 1 'i.MX8MP glmark2 row has a non-numeric shadow figure .n/a.'
rgcase "--scenes: a record directory is missing" \
    pass-README.md scenes-norecord-RESULTS.md none 1 'i.MX8MP glmark2 row cites no record directory'
RG_EXTRA=

echo "== 28. glmark2-ms.py (testdata/glmark2-ms)"
GMS=$HERE/glmark2-ms.py
GMD=$HERE/testdata/glmark2-ms
for b in pi4 imx8mp am62p; do
    out=$(python3 -I "$GMS" "$GMD/pass-$b.txt" 2>&1)
    rc=$?
    if [ "$rc" -eq 0 ] && [ "$out" = "$(cat "$GMD/pass-$b.expected")" ]; then
        ok "$b: table as expected, exit 0"
    else
        bad "$b: exit $rc (want 0) or the table differs from pass-$b.expected"
        printf '%s\n' "$out" | sed 's/^/        | /'
    fi
    out=$(python3 -I "$GMS" --cells "$GMD/pass-$b.txt" 2>&1)
    rc=$?
    if [ "$rc" -eq 0 ] && [ "$out" = "$(cat "$GMD/pass-$b.cells")" ]; then
        ok "$b: --cells as expected, exit 0"
    else
        bad "$b: --cells exit $rc (want 0) or differs from pass-$b.cells"
        printf '%s\n' "$out" | sed 's/^/        | /'
    fi
done
gmscase() {
    local name=$1 file=$2 want=$3 word=$4 out rc
    out=$(python3 -I "$GMS" "$GMD/$file" 2>&1)
    rc=$?
    if [ "$rc" -eq "$want" ] && printf '%s\n' "$out" | grep -q -- "$word"; then
        ok "$name: exit $rc, output has '$word'"
    else
        bad "$name: exit $rc (want $want), output lacks '$word'"
        printf '%s\n' "$out" | sed 's/^/        | /'
    fi
}
gmscase "a cut-off FrameTime" fail-damaged.txt 1 'FAIL: damaged scene line'
gmscase "a started scene with no result" fail-missing.txt 1 'FAIL: scene started, no result line'
gmscase "no glmark2 output" fail-nooutput.txt 2 '^error: no glmark2 output'

if [ "$fail" -eq 0 ]; then
    echo "== all checks passed"
else
    echo "== $fail check(s) FAILED" >&2
fi
[ "$fail" -eq 0 ]
