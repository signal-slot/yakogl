#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# dEQP-GLES2 on THIS HOST against the in-process compiler: every generated
# shader dEQP writes, compiled and linked by our library, without a board.
#
#   tools/cts/host-deqp-run.sh                  # the whole mustpass list
#   tools/cts/host-deqp-run.sh shaders          # cases whose name contains this
#
# What it measures, and what it cannot
# ------------------------------------
# The library is built with the LINUX OSAL, which keeps the command stream
# instead of executing it: every glReadPixels reads black, so every render
# verdict here is meaningless and must never be quoted. What IS real is each
# case's compile and link outcome and our info log, which dEQP records in the
# .qpa with the shader sources -- exactly the population the .test corpus
# (tools/online_oracle.py) cannot reach: the C++-generated shaders.* groups,
# uniform_api, texture, fragment_ops and the rest. tools/cts/cts-refusals.py
# buckets the refusals, separating cases dEQP failed (gaps) from negative
# cases it passed (correct refusals). Label every number from this run
# HOST-ONLY.
#
# Why 100 cases per process
# -------------------------
# Until 2026-09-23 the compiler's keep arena had no free path: after about 200
# compiles in one process every compile refused with `Abi(OutOfMemory
# { left: 0 })` (a single long run gave 199 honest cases, then 7,299
# identical refusals). glDeleteShader and glDeleteProgram now give it back,
# and `CHUNK=100000 tools/cts/host-deqp-run.sh <filter>` runs a whole group in
# ONE process -- which is the check that the free path holds under a real
# application. Chunks stay the default because a crash costs one chunk rather
# than every case after it -- a panic that aborted the first single-process
# run silently removed 2,743 mustpass cases from its case list. The run prints how many
# cases BEGAN against how many were asked for; a shortfall is the aborted
# chunks, reported by their exit status 134.
#
# Environment:
#   CTS       VK-GL-CTS checkout with the yakogl target applied (cts-build.sh
#             applies it)                (default ~/com/github/KhronosGroup/VK-GL-CTS)
#   GPU       vivante | powervr | v3d: whose online back end compiles (default vivante,
#             the GC7000's); each GPU gets its own library directory
#   MODULE    gles2 | gles3 (default gles2). gles3 builds deqp-gles3 in its own
#             directory, runs the union of VK-GL-CTS's android/cts/main/gles3-main-*.txt
#             (the yearly lists are increments; read from the checkout, not
#             copied), and links a library that
#             reports OpenGL ES 3.0 (YAKOGL_REPORT_ES2=OFF), in its own
#             directories so the two modules never share a build
#   OUT       run directory              (default <repo>/build/host-deqp/run-<commit>,
#             run-<commit>-powervr for the PowerVR)
#   JOBS      chunks in parallel         (default nproc - 2)
#   CHUNK     cases per process          (default 100)
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
CTS=${CTS:-$HOME/com/github/KhronosGroup/VK-GL-CTS}
BASE=$REPO/build/host-deqp
COMMIT=$(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown)
GPU=${GPU:-vivante}
case "$GPU" in
    vivante) LIB=$BASE/lib; SUFFIX= ;;
    powervr) LIB=$BASE/lib-powervr; SUFFIX=-powervr ;;
    v3d) LIB=$BASE/lib-v3d; SUFFIX=-v3d ;;
    *) echo "host-deqp-run: GPU must be vivante, powervr or v3d, not '$GPU'" >&2; exit 1 ;;
esac
MODULE=${MODULE:-gles2}
case "$MODULE" in
    gles2) DEQP_DIR=$BASE/deqp; MUSTPASS=$HERE/gles2-mustpass.txt; REPORT_ES2=ON ;;
    gles3) DEQP_DIR=$BASE/deqp-gles3; MUSTPASS=$CTS/android/cts/main/gles3-main-*.txt
           REPORT_ES2=OFF; LIB=$LIB-es3; SUFFIX=$SUFFIX-gles3 ;;
    *) echo "host-deqp-run: MODULE must be gles2 or gles3, not '$MODULE'" >&2; exit 1 ;;
esac
OUT=${OUT:-$BASE/run-$COMMIT$SUFFIX}
JOBS=${JOBS:-$(( $(nproc) > 3 ? $(nproc) - 2 : 1 ))}
CHUNK=${CHUNK:-100}
FILTER=${1:-}

[ -f "$CTS/targets/yakogl/yakogl.cmake" ] || {
    echo "host-deqp-run: $CTS has no yakogl target (tools/cts/cts-build.sh applies it)" >&2; exit 1; }

echo "host-deqp-run: library and harness from $COMMIT, GPU $GPU, $MODULE (HOST-ONLY: compile/link outcomes only)"

# 1. deqp-<module> for the host, against the YakoGL target (dlopen()s our libs).
if [ ! -x "$DEQP_DIR/modules/$MODULE/deqp-$MODULE" ]; then
    mkdir -p "$BASE"
    cmake -S "$CTS" -B "$DEQP_DIR" -G Ninja -DCMAKE_BUILD_TYPE=Release \
        -DDEQP_TARGET=yakogl -DSELECTED_BUILD_TARGETS=deqp-$MODULE > "$DEQP_DIR-build.log" 2>&1 ||
        { tail -20 "$DEQP_DIR-build.log"; exit 1; }
    cmake --build "$DEQP_DIR" -- -j"$(nproc)" >> "$DEQP_DIR-build.log" 2>&1 ||
        { tail -20 "$DEQP_DIR-build.log"; exit 1; }
fi

# 2. The library: linux OSAL, the chosen GPU's back end, the online compiler.
mkdir -p "$BASE"
cmake -S "$REPO" -B "$LIB" -DYAKOGL_OSAL=linux -DYAKOGL_GPU="$GPU" \
    -DYAKOGL_SHADERC=online -DYAKOGL_REPORT_ES2=$REPORT_ES2 -DYAKOGL_SHARED=ON > "$LIB-build.log" 2>&1 ||
    { tail -20 "$LIB-build.log"; exit 1; }
cmake --build "$LIB" -j"$(nproc)" >> "$LIB-build.log" 2>&1 ||
    { tail -20 "$LIB-build.log"; exit 1; }
[ -e "$LIB/libGLESv2.so.2" ] || { echo "host-deqp-run: no libGLESv2.so.2 in $LIB" >&2; exit 1; }

# 3. The case list, in fresh-process chunks.
rm -rf "$OUT"
mkdir -p "$OUT"
# shellcheck disable=SC2086  # MUSTPASS may be a glob of increments
awk '!seen[$0]++' $MUSTPASS > "$OUT/mustpass.txt"
if [ -n "$FILTER" ]; then
    grep -F -- "$FILTER" "$OUT/mustpass.txt" > "$OUT/all.txt" || true
else
    cp "$OUT/mustpass.txt" "$OUT/all.txt"
fi
asked=$(wc -l < "$OUT/all.txt")
[ "$asked" -gt 0 ] || { echo "host-deqp-run: no case matches '$FILTER'" >&2; exit 1; }
(cd "$OUT" && split -l "$CHUNK" -d -a 3 all.txt c_)
echo "$COMMIT $GPU $MODULE" > "$OUT/commit"

run_chunk() {
    local c=$1
    (cd "$DEQP_DIR/modules/$MODULE" &&
     LD_LIBRARY_PATH="$LIB" timeout 600 ./deqp-$MODULE --deqp-caselist-file="$c" \
        --deqp-log-filename="$c.qpa" --deqp-log-shader-sources=enable --deqp-log-images=disable \
        --deqp-surface-type=pbuffer --deqp-surface-width=64 --deqp-surface-height=64 > "$c.out" 2>&1)
    echo "exit=$?" >> "$c.out"
}
export -f run_chunk
export BASE LIB DEQP_DIR MODULE
ls "$OUT"/c_??? | xargs -P "$JOBS" -I{} bash -c 'run_chunk "$(realpath {})"' 2>/dev/null || true

# 4. What happened, and the refusals by reason.
begun=$(cat "$OUT"/c_*.qpa 2>/dev/null | grep -c '^#beginTestCaseResult' || true)
echo "host-deqp-run: $begun of $asked cases began; chunk exits:"
grep -h '^exit=' "$OUT"/c_*.out | sort | uniq -c
grep -h -A1 'panic' "$OUT"/c_*.out 2>/dev/null | grep panicked | sort | uniq -c || true
python3 "$HERE/cts-refusals.py" --cases 2 "$OUT"
echo "host-deqp-run: results in $OUT (HOST-ONLY: never quote a render verdict from them)"
