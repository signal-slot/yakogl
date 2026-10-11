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
# The dEQP-GLES3 tables (am62p-gles3-tree.txt, pi4-gles3-tree.txt) are split
# by hand to fit each board's watchdog window, and a split by hand drops or
# doubles a case easily (RESULTS.md 1r split its line 19 into 19a-19g and
# named 3D lists). The checker's self-test covers its rules; the two tables
# must then each select the 42,810 cases of gles3-mustpass.txt exactly once.
echo
echo "== 19. check-gles3-tree.py: self-test, then the GLES3 tables against gles3-mustpass.txt"
if python3 -I "$HERE/check-gles3-tree.py" --self-test > /dev/null 2>&1; then
    ok "check-gles3-tree.py --self-test"
else
    bad "check-gles3-tree.py --self-test failed: $(python3 -I "$HERE/check-gles3-tree.py" --self-test 2>&1 | grep FAIL)"
fi
for t in am62p-gles3-tree.txt pi4-gles3-tree.txt; do
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
# a failed or image-less build, the WDOG self-arm flag, refused arguments).
# Its count of passed checks is pinned, so a selftest that stops checking
# reads as a failure here, not as a pass.
IMAGES_SELFTEST_CHECKS=34
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

if [ "$fail" -eq 0 ]; then
    echo "== all checks passed"
else
    echo "== $fail check(s) FAILED" >&2
fi
[ "$fail" -eq 0 ]
