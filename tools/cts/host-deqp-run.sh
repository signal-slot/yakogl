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
# cases BEGAN against how many were asked for, and every chunk that needed more
# than one process or lost cases (aborted.txt). A chunk goes on after its
# process stops (run_chunk): the exit status alone hid that, because a
# ResourceError ends dEQP's run with exit 1, and the first full gles3 run on
# the V3D (2026-10-09) lost 1,275 cases that way, each such chunk exiting 1 like any chunk with
# a failing case.
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
# Options equal to the environment variables below, for callers that cannot set
# them: --gpu=G --module=M --cts=DIR --out=DIR --chunk=N --jobs=N (before the filter).
# --report=DIR builds and runs nothing: it prints step 4's report of the
# finished run in DIR again (and rewrites its aborted.txt and summary.txt).
while [ $# -gt 0 ]; do
    case "$1" in
        --gpu=*) GPU=${1#*=} ;;
        --module=*) MODULE=${1#*=} ;;
        --cts=*) CTS=${1#*=} ;;
        --out=*) OUT=${1#*=} ;;
        --chunk=*) CHUNK=${1#*=} ;;
        --jobs=*) JOBS=${1#*=} ;;
        --report=*) OUT=${1#*=}; REPORT=1 ;;
        *) break ;;
    esac
    shift
done
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

if [ -n "${REPORT:-}" ]; then
    [ -f "$OUT/all.txt" ] || { echo "host-deqp-run: $OUT is not a run directory" >&2; exit 1; }
    echo "host-deqp-run: report of $OUT, run from $(cat "$OUT/commit" 2>/dev/null) (HOST-ONLY)"
else
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

# A chunk whose process stops early (a crash, the timeout, a ResourceError --
# which ends dEQP's whole run -- or a die() in a group's init) goes on in a
# fresh process with the cases no process of it has begun, logged to
# c_NNN.r<k>.qpa. A process that begins none of them is stopped by the first
# one before it begins (lifetime.*'s init dies with no case open): that case
# goes to c_NNN.skipped, never began, and the rest run. Bounded: every
# process either begins a case or skips one.
run_chunk() {
    local c=$1 list=$1 log=$1.qpa k=0 left rest
    left=$(wc -l < "$c")
    : > "$c.out"
    while :; do
        (cd "$DEQP_DIR/modules/$MODULE" &&
         LD_LIBRARY_PATH="$LIB" timeout 600 ./deqp-$MODULE --deqp-caselist-file="$list" \
            --deqp-log-filename="$log" --deqp-log-shader-sources=enable --deqp-log-images=disable \
            --deqp-surface-type=pbuffer --deqp-surface-width=64 --deqp-surface-height=64 >> "$c.out" 2>&1)
        echo "exit=$?" >> "$c.out"
        # (Not awk's FNR==NR: with nothing begun yet its first input is empty
        # and the whole list would read as begun.)
        grep -vxF -f <(cat "$c".qpa "$c".r*.qpa 2>/dev/null | sed -n 's/^#beginTestCaseResult //p'
                       cat "$c.skipped" 2>/dev/null) "$c" > "$c.rest" || true
        rest=$(wc -l < "$c.rest")
        if [ "$rest" -gt 0 ] && [ "$rest" -eq "$left" ]; then
            head -1 "$c.rest" >> "$c.skipped"
            sed -i 1d "$c.rest"
            rest=$((rest - 1))
        fi
        [ "$rest" -gt 0 ] || break
        left=$rest k=$((k + 1)) list=$c.rest log=$c.r$k.qpa
    done
    rm -f "$c.rest"
}
export -f run_chunk
export BASE LIB DEQP_DIR MODULE
ls "$OUT"/c_??? | xargs -P "$JOBS" -I{} bash -c 'run_chunk "$(realpath {})"' 2>/dev/null || true

fi

# 4. What happened, and the refusals by reason.
asked=$(wc -l < "$OUT/all.txt")
begun=$(cat "$OUT"/c_*.qpa 2>/dev/null | grep -c '^#beginTestCaseResult' || true)
echo "host-deqp-run: $begun of $asked cases began; process exits (a resumed chunk has several):"
grep -h '^exit=' "$OUT"/c_*.out | sort | uniq -c
grep -h -A1 'panic' "$OUT"/c_*.out 2>/dev/null | grep panicked | sort | uniq -c || true
# Every chunk that needed more than one process or lost cases: each process's
# exit and where it stopped -- "in X" for a case that began and has no end
# (a crash or the timeout), "after X" for one that ended (a ResourceError
# ends dEQP's run there; so does die() in the next group's init) -- and the
# cases skipped because they stopped every process before they began. Exit 1
# alone is dEQP's "not every case passed".
for c in "$OUT"/c_???; do
    # Runs of the same exit or stop are folded: "137 x56".
    exits=$(sed -n 's/^exit=//p' "$c.out" | uniq -c | awk '{print ($1 > 1 ? $2 " x" $1 : $2)}' | paste -sd,)
    listed=$(wc -l < "$c")
    began=$(cat "$c".qpa "$c".r*.qpa 2>/dev/null | grep -c '^#beginTestCaseResult' || true)
    case "$exits" in 0|1) [ "${began:-0}" -eq "$listed" ] && continue ;; esac
    stops=$(for q in $(ls "$c".qpa "$c".r*.qpa 2>/dev/null | sort -V); do
        awk '/^#beginTestCaseResult/{n=$2; open=1} /^#endTestCaseResult|^#terminateTestCaseResult/{open=0}
             END{print (n == "" ? "before any case" : (open ? "in " : "after ") n)}' "$q"
    done | uniq -c | awk '{n = $1; sub(/^ *[0-9]+ /, ""); print (n > 1 ? $0 " x" n : $0)}' |
        paste -sd';' | sed 's/;/; /g' || true)
    skipped=$(cat "$c.skipped" 2>/dev/null | wc -l || true)
    echo "host-deqp-run: chunk $(basename "$c"): exits $exits; ${began:-0} of $listed began; stopped: $stops; never began: $skipped$( [ "$skipped" -gt 0 ] && echo " ($(head -1 "$c.skipped") ..)")"
done | tee "$OUT/aborted.txt"
python3 "$HERE/cts-refusals.py" --cases 2 "$OUT"
# The per-group table, the failures by message and every failing case, kept
# beside the logs (HOST-ONLY: a render verdict in them means nothing).
CTS="$CTS" python3 "$HERE/cts-summary.py" --mustpass "$OUT/all.txt" --by-group 3 \
    --by-message 100000 --list-failures 0 --failures-file "$OUT/failures.tsv" \
    "$OUT" > "$OUT/summary.txt" 2>&1 || true
echo "host-deqp-run: groups, failures by message: $OUT/summary.txt"
echo "host-deqp-run: results in $OUT (HOST-ONLY: never quote a render verdict from them)"
