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
# 25 the audit in report mode over the repository (fails on unknown-id or a
# crash only; the missing-spdx count is printed as information, --strict stays
# off until the owner decides, docs/release/license-audit.md).
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
# ---- 18: common-tally.py tallies the cluster row (testdata/cluster-run) ------
#
# The CL row of common-tree.txt (the AGL cluster sample) is judged by its
# PROBES record, and its rate comes from the console's FINGERPRINT, PERF and
# APP records (`yakogl-tools record --log`). The fixture replays the Pi 4
# cluster run of 2026-10-04: its APP and PROBES words are verbatim from
# tools/cts/RESULTS.md, its PERF words carry that run's printed numbers, and
# its FINGERPRINT line is the Pi 4 cluster console form pinned in
# recordcmd.rs's throughput_from_a_log (testdata/README.md says which is
# which). The tally must show the PROBES verdict, the cluster probes line and
# the throughput line computed from those records.
echo
echo "== 18. common-tally.py on the cluster fixture (testdata/cluster-run)"
CL_RUN=$HERE/testdata/cluster-run/20261004-0438-cluster-11236a24
tout=$(python3 "$HERE/common-tally.py" pi4 --row "CL=$CL_RUN" 2>&1)
trc=$?
for want in "CL cluster rc=0 verdict=done result=record:PROBES" \
            "cluster probes pass" \
            "throughput: 1920x720: 8395 frames in 60019 ms, 139.9 fps, 193.4 Mpx/s; draw 4.9 ms mean, 3.54 ms/Mpx" \
            "resets (console count) 0" \
            "image=11236a247d72bdf0" \
            "build=PVR_SAMPLE_TREE=031de9a2,PVR_SAMPLE_TABLE=1449eb0f"; do
    case "$tout" in
        *"$want"*) ok   "cluster row: reports \"$want\"" ;;
        *)         bad  "cluster row: MISSING \"$want\"" ;;
    esac
done
if [ "$trc" -eq 0 ]; then
    ok "cluster row: exit 0 (the row has a run and its verdict is done)"
else
    bad "cluster row: exit $trc, expected 0"
fi

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
# and pi4-deqp-images.sh against a fake cmake/ninja (the gles2 dry run and
# build exactly as before, the gles3 path's three arguments and --gles3
# check, the stage 1 check). Its count of passed checks is pinned, so a
# selftest that stops checking reads as a failure here, not as a pass.
IMAGES_SELFTEST_CHECKS=74
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
    printf 'board=pi4\nboot=tryboot\nname=glmark2\nprobe=0\nimage=zephyr.bin\nimage_sha256=d0932b243a844247\nimage_build=PVR_SAMPLE_TREE=42bd90ff\nt_reboot=100\nt_back=417\nback=1\n' > "$d/meta.txt"
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
# ---- 25: licence audit, report mode, whole repository ----------------------
#
# Report mode (no --strict): the exit status is 0 whatever the findings, so a
# non-zero exit is a crash. unknown-id (an identifier with no text in
# LICENSES/) is the one finding that must stay at 0. missing-spdx is printed
# only: whether those files get headers or allowlist rules is the owner's call.
echo
echo "== 25. license_audit.py in report mode over the repository"
larep=$(python3 -I "$LA" 2>&1)
larc=$?
if [ "$larc" -ne 0 ]; then
    bad "license audit: crashed (exit $larc)"
    printf '%s\n' "$larep" | tail -5 | sed 's/^/        | /'
else
    launk=$(printf '%s\n' "$larep" | sed -n 's/^unknown-id: \([0-9][0-9]*\)$/\1/p')
    lamiss=$(printf '%s\n' "$larep" | sed -n 's/^missing-spdx: \([0-9][0-9]*\)$/\1/p')
    if [ -z "$launk" ] || [ -z "$lamiss" ]; then
        bad "license audit: no unknown-id / missing-spdx line in the report"
    elif [ "$launk" -ne 0 ]; then
        bad "license audit: $launk unknown-id finding(s) (python3 -I tools/license_audit.py --list unknown-id)"
    else
        ok "license audit: unknown-id 0; missing-spdx $lamiss (information, strict mode is off)"
    fi
fi

if [ "$fail" -eq 0 ]; then
    echo "== all checks passed"
else
    echo "== $fail check(s) FAILED" >&2
fi
[ "$fail" -eq 0 ]
