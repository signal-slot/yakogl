#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Harvest every shader program dEQP-GLES2 compiles: the whole
# tools/cts/gles2-mustpass.txt run on THIS HOST against the V3D 4.2 library
# (tools/cts/host-deqp-run.sh with GPU=v3d), then each program's vertex and
# fragment source, link status and info log read back from the .qpa logs.
#
#   tools/cts/deqp-es2-programs.sh [--cts DIR] [--out DIR] [--jobs N] [--chunk N]
#                                  [--stage all|build|run|extract]
#
# Why
# ---
# The C++-generated dEQP-GLES2 shaders (shaders.random, operator, struct,
# loops, functions, uniform_api, texture, fragment_ops, ...) are not in the
# data/gles2/shaders/*.test library: only a deqp-gles2 run writes them. The
# Pi 4 (tools/cts/RESULTS.md) and the Mali host measurement
# (tools/mali/deqp-host-measure.sh) meet them that way. This keeps them as a
# corpus a back end can be measured on without dEQP (the V3D 7.1 back end).
# The text is dEQP's output (Apache-2.0) and is NOT committed (size).
#
# Stages (--stage, default all = build, run, extract):
#   build    deqp-gles2 and the v3d library (host-deqp-run.sh --build-only),
#            under <repo>/build/host-deqp/ as host-deqp-run.sh keeps them
#   run      the whole mustpass in fresh-process chunks, logged with
#            --deqp-log-shader-sources=enable: OUT/run is host-deqp-run.sh's
#            run directory (c_NNN.qpa, aborted.txt, summary.txt, ...)
#   extract  tools/cts/deqp-es2-programs.py over OUT/run: the outputs below.
#            Deterministic: two extracts of the same logs are byte-identical
#
# Output (OUT, default <repo>/build/deqp-es2-programs/out-<tip>):
#   index.tsv        one row per program, deduplicated by the exact (vertex,
#                    fragment) text, sorted by id: id, the first case using it
#                    (mustpass order), the number of cases using it, the 4.2
#                    outcome (compiled | refused) and the first refusal line
#                    (empty when compiled). id = the first 16 hex digits of
#                    sha256(vs + '\0' + fs), stable across runs
#   programs/<id>.vert, programs/<id>.frag   the sources, verbatim
#   cases-without-program.txt   the mustpass cases that logged no complete
#                    program: case <TAB> not-run | no-program
#   commit           the CTS commit, the tip and the command (run stage)
#   run/             the deqp-gles2 logs (host-deqp-run.sh's run directory)
#
# --cts DIR is a VK-GL-CTS checkout with external sources fetched
# (external/fetch_sources.py); default <deps>/VK-GL-CTS of
# tools/cts/deps-env.sh --layout (docs/release/building.md section 9, the
# DEQP_WORK note of tools/cts/deps.toml: commit 93bca01). When it has no
# yakogl target, a private copy with tools/cts/cts-yakogl-target.patch is made
# in <repo>/build/deqp-es2-programs/cts (a --shared clone whose external/*/src
# link to DIR's), as tools/cts/cts-build-zephyr.sh does: DIR is never written.
#
# Every outcome here is HOST-ONLY: the Linux OSAL keeps the command stream
# without running it. What is real is the compile and link outcome and the
# info log, which is all this keeps.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
me=deqp-es2-programs

usage() {
    sed -n '10,11p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

cts= out= stage=all
jobs=${JOBS:-$(( $(nproc) > 3 ? $(nproc) - 2 : 1 ))}
chunk=${CHUNK:-100}
while [ $# -gt 0 ]; do
    case $1 in
        --cts) cts=${2:?--cts needs a directory}; shift 2 ;;
        --out) out=${2:?--out needs a directory}; shift 2 ;;
        --jobs) jobs=${2:?--jobs needs a number}; shift 2 ;;
        --chunk) chunk=${2:?--chunk needs a number}; shift 2 ;;
        --stage) stage=${2:?--stage needs all|build|run|extract}; shift 2 ;;
        -h | --help) sed -n '5,55p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "$me: unknown argument '$1'" >&2; usage ;;
    esac
done
case $stage in all | build | run | extract) ;;
    *) echo "$me: --stage must be all|build|run|extract" >&2; exit 2 ;; esac
COMMIT=$(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown)
WORK=$REPO/build/deqp-es2-programs
out=${out:-$WORK/out-$COMMIT}

# The CTS tree host-deqp-run.sh builds from: DIR itself when it has the yakogl
# target, else the private copy beside the build.
cts_tree() {
    if [ -z "$cts" ]; then
        local deps
        deps=$("$HERE/deps-env.sh" --layout 2>/dev/null || true)
        cts=${deps:-$HOME/.local/share/connected/deps/yakogl}/VK-GL-CTS
    fi
    [ -d "$cts/external/zlib/src" ] || {
        echo "$me: $cts has no external sources (python3 external/fetch_sources.py in it; docs/release/building.md section 9)" >&2
        exit 1; }
    if [ -f "$cts/targets/yakogl/yakogl.cmake" ]; then
        tree=$cts
        return
    fi
    tree=$WORK/cts
    if [ ! -f "$tree/targets/yakogl/yakogl.cmake" ]; then
        echo "$me: a private copy of $cts with the yakogl target in $tree"
        rm -rf "$tree"
        mkdir -p "$WORK"
        git clone -q --shared --no-checkout "$cts" "$tree"
        git -C "$tree" checkout -q --detach "$(git -C "$cts" rev-parse HEAD)"
        local d n
        for d in "$cts"/external/*/src; do
            n=$(basename "$(dirname "$d")")
            mkdir -p "$tree/external/$n"
            [ -e "$tree/external/$n/src" ] || ln -s "$d" "$tree/external/$n/src"
        done
        git -C "$tree" apply "$HERE/cts-yakogl-target.patch"
    fi
}

host_run() {
    "$HERE/host-deqp-run.sh" --gpu=v3d --module=gles2 --cts="$tree" --jobs="$jobs" --chunk="$chunk" "$@"
}

if [ "$stage" = all ] || [ "$stage" = build ]; then
    cts_tree
    host_run --build-only
fi

if [ "$stage" = all ] || [ "$stage" = run ]; then
    cts_tree
    mkdir -p "$out"
    start=$(date +%s)
    # host-deqp-run.sh builds (incrementally), runs the whole mustpass and
    # prints its report; it replaces OUT/run as a whole.
    host_run --out="$out/run"
    end=$(date +%s)
    {
        echo "cts=$(git -C "$tree" rev-parse HEAD 2>/dev/null || echo unknown) $tree"
        echo "tip=$COMMIT"
        echo "command=$0 --cts $cts --out $out --jobs $jobs --chunk $chunk --stage $stage"
        echo "run=host-deqp-run.sh --gpu=v3d --module=gles2 (YAKOGL_V3D_TILED_COPIES_BYTES=${YAKOGL_V3D_TILED_COPIES_BYTES:-16777216})"
        echo "run-seconds=$((end - start))"
    } > "$out/commit"
fi

if [ "$stage" = all ] || [ "$stage" = extract ]; then
    [ -f "$out/run/all.txt" ] || { echo "$me: $out/run is not a run directory (no all.txt; --stage run first)" >&2; exit 1; }
    python3 -I "$HERE/deqp-es2-programs.py" "$out"
fi
