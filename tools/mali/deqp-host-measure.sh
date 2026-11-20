#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# HOST-ONLY measure of the dEQP-GLES2 cases the Mali online compiler refused
# on the NanoPC-T6: every case whose refusals.tsv text in the board slices is a
# compile refusal is run through deqp-gles2 on THIS HOST against the Mali back
# end's library, which compiles and links every shader dEQP writes with the
# compiler at the current tip. The .qpa keeps the shader sources and the info
# logs; per case the first refusal is read from them, its family is named from
# the refusal text and its shape from the refusing shader's source.
#
#   tools/mali/deqp-host-measure.sh [--out DIR] [--cts DIR] [--jobs N] [--chunk N]
#                                   [--slice DIR]... [--filter STR] [--stage all|build|deqp|lib|run|extract]
#
# What it measures, and what it cannot
# ------------------------------------
# The library is the C pair (libGLESv2.so.2, libEGL.so.1) of the yakogl-capi
# crate with the Linux OSAL, gpu-mali, shaderc-online, no-shader-table and
# report-es2: the features of MALI_CONFIG in tools/check.sh with the C OSAL in
# place of host-test (host-test links std and is a compile error in the C
# archive). The top-level CMakeLists.txt takes powervr|vivante|v3d only, so
# the library is built here from a generated CMake project the way
# tools/hostharness/glmark2/mali/CMakeLists.txt builds its archive, plus the
# shared pair the top-level build links. src/osal/linux.c keeps the command
# stream instead of executing it and answers the PowerVR's contract, so a
# small C shim (host_osal.c, the GM2_MALI table of
# tools/hostharness/glmark2/shim.c) lends the compiler its scratch and keep
# regions and answers the Mali back end's OSAL queries. Nothing runs a
# stream: every glReadPixels reads black, every render verdict is
# meaningless, and every number this script prints is HOST-ONLY. What IS real
# is each shader's compile and each program's link outcome and our info log.
#
# Stages (--stage, default all):
#   build    deqp-gles2 for the host against the yakogl target (step 1 of
#            tools/cts/host-deqp-run.sh) and the Mali pair, both under
#            <repo>/build/host-deqp-mali/ (reused when present); `deqp` and
#            `lib` are its two halves alone
#   run      the case list from the slices' refusals.tsv (the cases whose
#            text holds "online shaderc: the code generator refused"), in
#            fresh-process chunks as host-deqp-run.sh runs them (a chunk whose
#            process stops early goes on in a fresh process), logs in OUT
#   extract  measure.tsv, summary.md, shapes/ and the other outputs below
#
# Output (OUT, default $TMPDIR/deqp-host-measure-<commit>; TMPDIR else /tmp):
#   cases.txt        the case list, in mustpass order
#   board.tsv        case, the board's status and its refusal text (the slices)
#   c_NNN, c_NNN.qpa, c_NNN.out, c_NNN.r<k>.qpa  the chunks and their logs
#   measure.tsv      per case: board status, board family, host status, the
#                    refusing stage, host family, shape, the host refusal text
#   summary.md       HOST-ONLY: per family a table of shapes with case counts
#                    (Fail/NotSupported by the BOARD's status), an example case
#                    and the count by stage; the families' totals; the cases
#                    not reproduced on the host, by reason
#   not-reproduced.tsv  cases whose host first refusal is not the board's
#   shapes/<family>_<shape>.vert|.frag  one representative pair per shape,
#                    verbatim from the .qpa (plus .case naming the case)
#   commit           the tip, the CTS commit and the command line
#
# Environment:
#   DEQP_WORK  VK-GL-CTS checkout with external sources and the yakogl target
#              (tools/cts/deps.toml); else <deps>/VK-GL-CTS of
#              tools/cts/deps-env.sh --layout. --cts overrides.
#   JOBS       chunks in parallel (default nproc - 1); CHUNK cases per
#              process (default 100)
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
me=deqp-host-measure
CTS_COMMIT=93bca01861b0e3ef3c387027a9791e6d065f900c
REFUSED='online shaderc: the code generator refused'

usage() {
	sed -n '13,15p' "$0" | sed 's/^# \{0,1\}//' >&2
	exit 2
}

out= cts=${DEQP_WORK:-} stage=all filter=
jobs=${JOBS:-$(( $(nproc) > 1 ? $(nproc) - 1 : 1 ))}
chunk=${CHUNK:-100}
slices=()
while [ $# -gt 0 ]; do
	case $1 in
	--out) out=${2:?--out needs a directory}; shift 2 ;;
	--cts) cts=${2:?--cts needs a directory}; shift 2 ;;
	--jobs) jobs=${2:?--jobs needs a number}; shift 2 ;;
	--chunk) chunk=${2:?--chunk needs a number}; shift 2 ;;
	--slice) slices+=("${2:?--slice needs a directory}"); shift 2 ;;
	--filter) filter=${2:?--filter needs a string}; shift 2 ;;
	--stage) stage=${2:?--stage needs all|build|run|extract}; shift 2 ;;
	-h | --help) sed -n '5,60p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
	*) echo "$me: unknown argument '$1'" >&2; usage ;;
	esac
done
case $stage in all | build | deqp | lib | run | extract) ;;
*) echo "$me: --stage must be all|build|deqp|lib|run|extract" >&2; exit 2 ;; esac

if [ -z "$cts" ]; then
	deps=$("$REPO/tools/cts/deps-env.sh" --layout 2>/dev/null || true)
	cts=${deps:-$HOME/.local/share/connected/deps/yakogl}/VK-GL-CTS
fi
[ "${#slices[@]}" -gt 0 ] || slices=("$REPO"/tools/cts/log/2026-11-15-t6-gles2-S{1,2,3,4}-37305608)
COMMIT=$(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown)
BASE=$REPO/build/host-deqp-mali
DEQP_DIR=$BASE/deqp
LIB=$BASE/lib
out=${out:-${TMPDIR:-/tmp}/$me-$COMMIT}
MUSTPASS=$REPO/tools/cts/gles2-mustpass.txt
# 256x256 as tools/cts/mali-deqp-run.sh runs the board (DEQP_SURFACE): at
# 64x64 shaders.invariance ends NotSupported ("Render target size must be at
# least 256x256") before it compiles anything.
SURFACE=${DEQP_SURFACE:-256}

echo "$me: tip $COMMIT, CTS $cts, out $out (HOST-ONLY: compile and link outcomes only)"

# ---- 1. deqp-gles2 for the host --------------------------------------------
build_deqp() {
	[ -f "$cts/targets/yakogl/yakogl.cmake" ] || {
		echo "$me: $cts has no yakogl target (tools/cts/cts-build.sh applies cts-yakogl-target.patch)" >&2; exit 1; }
	[ -d "$cts/external/zlib/src" ] || {
		echo "$me: $cts has no external sources (python3 external/fetch_sources.py in it)" >&2; exit 1; }
	if [ ! -x "$DEQP_DIR/modules/gles2/deqp-gles2" ]; then
		mkdir -p "$BASE"
		cmake -S "$cts" -B "$DEQP_DIR" -G Ninja -DCMAKE_BUILD_TYPE=Release \
			-DDEQP_TARGET=yakogl -DSELECTED_BUILD_TARGETS=deqp-gles2 > "$DEQP_DIR-build.log" 2>&1 ||
			{ tail -20 "$DEQP_DIR-build.log"; exit 1; }
		cmake --build "$DEQP_DIR" -- -j"$(nproc)" >> "$DEQP_DIR-build.log" 2>&1 ||
			{ tail -20 "$DEQP_DIR-build.log"; exit 1; }
	fi
	echo "$me: deqp-gles2 at $DEQP_DIR/modules/gles2/deqp-gles2"
}

# ---- 2. the Mali pair on the Linux OSAL ------------------------------------
# A generated CMake project: the archive as tools/hostharness/glmark2/mali/
# CMakeLists.txt builds it, and the shared pair as the top-level
# CMakeLists.txt links it (--whole-archive, tools/yakogl.ver, -lm).
write_lib_project() {
	local src=$1
	mkdir -p "$src"
	cat > "$src/CMakeLists.txt" <<EOF
# Generated by tools/mali/deqp-host-measure.sh: the Mali pair on the Linux OSAL.
cmake_minimum_required(VERSION 3.20)
project(YakoGLDeqpHostMeasureMali C)
set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(YAKOGL_DIR "$REPO")
include("\${YAKOGL_DIR}/capi/yakogl_capi.cmake")

add_library(yakogl_c OBJECT
    "\${YAKOGL_DIR}/src/osal/linux.c"
    "\${YAKOGL_DIR}/src/abi_layout_check.c"
    "\${YAKOGL_DIR}/capi/eh_personality.c"
    "\${CMAKE_CURRENT_SOURCE_DIR}/host_osal.c")
target_include_directories(yakogl_c PRIVATE
    "\${YAKOGL_DIR}/include"
    "\${YAKOGL_DIR}/src"
    "\${YAKOGL_DIR}/src/osal"
    "\${YAKOGL_DIR}/src/vivante"
    "\${YAKOGL_DIR}/driver/src")
target_compile_definitions(yakogl_c PRIVATE YAKOGL_OSAL_LINUX YAKOGL_GPU_MALI
    "GL_MAX_CONTEXTS=\${YAKOGL_MAX_CONTEXTS}")
set_target_properties(yakogl_c PROPERTIES POSITION_INDEPENDENT_CODE ON)

set(YAKOGL_ARCHIVE "\${CMAKE_CURRENT_BINARY_DIR}/libyakogl.a")
yakogl_add_rust_archive(
  CRATE yakogl-capi
  MANIFEST "\${YAKOGL_DIR}/capi/Cargo.toml"
  LIB_NAME yakogl_capi
  FEATURES osal-linux gpu-mali shaderc-online no-shader-table report-es2
  OUTPUT "\${YAKOGL_ARCHIVE}"
  TARGET_NAME yakogl_archive
  C_OBJECT_LIBRARY yakogl_c)

set(YAKOGL_VERSION_SCRIPT "\${YAKOGL_DIR}/tools/yakogl.ver")
file(WRITE "\${CMAKE_CURRENT_BINARY_DIR}/yakogl_shared_tu.c"
"/* Generated: the anchor translation unit of libGLESv2.so.2. */\n"
"__attribute__((visibility(\"hidden\")))\n"
"const char yakogl_shared_build[] = \"yakogl\";\n")
add_library(GLESv2_shared SHARED "\${CMAKE_CURRENT_BINARY_DIR}/yakogl_shared_tu.c")
set_target_properties(GLESv2_shared PROPERTIES
    OUTPUT_NAME GLESv2 VERSION 2.0.0 SOVERSION 2
    LINK_DEPENDS "\${YAKOGL_VERSION_SCRIPT};\${YAKOGL_ARCHIVE}")
add_dependencies(GLESv2_shared yakogl_archive)
target_link_options(GLESv2_shared PRIVATE
    "-Wl,--whole-archive" "\${YAKOGL_ARCHIVE}" "-Wl,--no-whole-archive"
    "-Wl,--version-script=\${YAKOGL_VERSION_SCRIPT}")
target_link_libraries(GLESv2_shared PRIVATE m)

file(WRITE "\${CMAKE_CURRENT_BINARY_DIR}/egl_forward.c"
"/* Generated: libEGL.so.1 forwards to libGLESv2.so.2 (one state). */\n"
"extern unsigned int eglGetError(void);\n"
"__attribute__((visibility(\"hidden\")))\n"
"unsigned int (*const yakogl_egl_anchor)(void) = eglGetError;\n")
add_library(EGL_shared SHARED "\${CMAKE_CURRENT_BINARY_DIR}/egl_forward.c")
set_target_properties(EGL_shared PROPERTIES OUTPUT_NAME EGL VERSION 1.0.0 SOVERSION 1)
target_link_options(EGL_shared PRIVATE "-Wl,--no-as-needed")
target_link_libraries(EGL_shared PRIVATE GLESv2_shared)
EOF
	# The OSAL shim: the GM2_MALI table of tools/hostharness/glmark2/shim.c
	# without its crash trace and counters. linux.c answers the PowerVR's
	# contract on a build that is neither the GC7000's nor the V3D's and
	# refuses heap label 0, the one the Mali back end allocates from; this
	# table goes in front of it before the library's first call and gives the
	# back end what src/osal/mali.rs asks (the T6's counts as
	# src/mali/backend_tests.rs's mock answers them, a tiler heap in linux.c's
	# general heap, a submit that completes at once). The build has no
	# built-in compiler regions (no-shader-table without host-test): the shim
	# lends them at the library's sizes (src/shaderc/online/store.rs).
	cat > "$src/host_osal.c" <<'EOF'
/* Generated by tools/mali/deqp-host-measure.sh: the Mali back end's OSAL
 * contract on the Linux OSAL, after tools/hostharness/glmark2/shim.c
 * (GM2_MALI). Nothing runs the stream: HOST-ONLY. */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <gles_osal.h>

extern int shc_online_lend_scratch(unsigned char *buf, size_t len);
extern int shc_online_lend_keep(unsigned char *buf, size_t len);
extern void osal_set_ops(const struct osal_ops *ops);
#define HM_SCRATCH_BYTES (4u << 20)
#define HM_KEEP_BYTES (512u << 10)
#define OSAL_QUERY_MALI_GPU 15u
#define OSAL_QUERY_MALI_TILER_HEAP 16u
struct osal_mali_gpu {
	uint32_t gpu_id, tiler_features, cs_reg_count, unpreserved_cs_reg_count;
};
struct osal_mali_heap {
	uint64_t ctx_gpu, first_chunk_gpu;
	uint32_t chunk_size, reserved;
};
struct osal_mali_kick {
	uint64_t stream_gpu;
	uint32_t stream_bytes, reserved;
};
#define HM_HEAP_LABEL 0x8000000000ULL /* linux.c's HEAP_GENERAL */
#define HM_CHUNK_BYTES (2u << 20)

static const struct osal_ops *hm_real;
static struct osal_ops hm_ops;
static int64_t hm_fence;
static struct osal_gpu_mem hm_heap_ctx, hm_heap_chunk;

static int hm_alloc(uint64_t heap_base, size_t size, uint32_t flags, struct osal_gpu_mem *out)
{
	return hm_real->gpu_alloc(heap_base ? heap_base : HM_HEAP_LABEL, size, flags, out);
}

static int64_t hm_submit(const void *desc, size_t len)
{
	const struct osal_mali_kick *k = desc;

	if (!desc || len != sizeof(*k) || k->stream_bytes == 0 || k->stream_bytes % 8 ||
	    k->stream_gpu % 64)
		return -22;
	return ++hm_fence;
}

static int hm_wait(int64_t fence, int64_t timeout_us)
{
	(void)timeout_us;
	return (fence <= 0 || fence > hm_fence) ? -22 : 0;
}

static int64_t hm_completed(void)
{
	return hm_fence;
}

static int hm_query(uint32_t what, void *out, size_t out_sz)
{
	if (what == OSAL_QUERY_MALI_GPU) {
		static const struct osal_mali_gpu gpu = {
			.gpu_id = 0xa8670005u, .tiler_features = 0x809,
			.cs_reg_count = 96, .unpreserved_cs_reg_count = 4,
		};

		if (!out || out_sz != sizeof(gpu))
			return -22;
		memcpy(out, &gpu, sizeof(gpu));
		return 0;
	}
	if (what == OSAL_QUERY_MALI_TILER_HEAP) {
		struct osal_mali_heap *h = out;

		if (!out || out_sz != sizeof(*h))
			return -22;
		if (!hm_heap_ctx.cpu &&
		    (hm_alloc(0, 4096, OSAL_MEM_DATA, &hm_heap_ctx) != 0 ||
		     hm_alloc(0, HM_CHUNK_BYTES, OSAL_MEM_DATA, &hm_heap_chunk) != 0))
			return -12;
		memset(h, 0, sizeof(*h));
		h->ctx_gpu = hm_heap_ctx.gpu;
		h->first_chunk_gpu = hm_heap_chunk.gpu;
		h->chunk_size = HM_CHUNK_BYTES;
		return 0;
	}
	return hm_real->query(what, out, out_sz);
}

__attribute__((constructor(200))) static void hm_init(void)
{
	static uint64_t scratch[HM_SCRATCH_BYTES / 8], keep[HM_KEEP_BYTES / 8];
	int rs = shc_online_lend_scratch((unsigned char *)scratch, sizeof(scratch));
	int rk = shc_online_lend_keep((unsigned char *)keep, sizeof(keep));

	if (rs || rk)
		fprintf(stderr, "deqp-host-measure: the compiler refused its regions (scratch %d, keep %d)\n", rs, rk);
	if (!g_osal)
		return;
	hm_real = g_osal;
	hm_ops = *hm_real;
	hm_ops.gpu_alloc = hm_alloc;
	hm_ops.submit = hm_submit;
	hm_ops.wait = hm_wait;
	hm_ops.completed = hm_completed;
	hm_ops.query = hm_query;
	osal_set_ops(&hm_ops);
}
EOF
}

build_lib() {
	local src=$BASE/lib-src
	mkdir -p "$BASE"
	write_lib_project "$src"
	cmake -S "$src" -B "$LIB" > "$LIB-build.log" 2>&1 || { tail -20 "$LIB-build.log"; exit 1; }
	cmake --build "$LIB" -j"$(nproc)" >> "$LIB-build.log" 2>&1 || { tail -20 "$LIB-build.log"; exit 1; }
	[ -e "$LIB/libGLESv2.so.2" ] && [ -e "$LIB/libEGL.so.1" ] ||
		{ echo "$me: no libGLESv2.so.2 and libEGL.so.1 in $LIB" >&2; exit 1; }
	echo "$me: the Mali pair at $LIB (osal-linux gpu-mali shaderc-online no-shader-table report-es2)"
}

# ---- 3. the case list and the run ------------------------------------------
# run_chunk as tools/cts/host-deqp-run.sh: a chunk whose process stops early
# (a crash, the timeout) goes on in a fresh process over the cases no process
# of it has begun (c_NNN.r<k>.qpa); a process that begins none of them is
# stopped by the first one before it begins, which goes to c_NNN.skipped.
run_chunk() {
	local c=$1 list=$1 log=$1.qpa k=0 left rest
	left=$(wc -l < "$c")
	: > "$c.out"
	while :; do
		(cd "$DEQP_DIR/modules/gles2" &&
		 LD_LIBRARY_PATH="$LIB" timeout 600 ./deqp-gles2 --deqp-caselist-file="$list" \
			--deqp-log-filename="$log" --deqp-log-shader-sources=enable --deqp-log-images=disable \
			--deqp-surface-type=pbuffer --deqp-surface-width="$SURFACE" --deqp-surface-height="$SURFACE" >> "$c.out" 2>&1)
		echo "exit=$?" >> "$c.out"
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
export DEQP_DIR LIB SURFACE

run_cases() {
	local s n
	rm -rf "$out"
	mkdir -p "$out"
	for s in "${slices[@]}"; do
		[ -f "$s/refusals.tsv" ] && [ -f "$s/results.tsv" ] ||
			{ echo "$me: $s is not a slice (no refusals.tsv and results.tsv)" >&2; exit 1; }
	done
	# board.tsv: case, status, refusal text, for every case whose refusal is
	# a compile refusal (link refusals quote the compile refusal too).
	awk -F'\t' -v OFS='\t' -v want="$REFUSED" '
		FNR == 1 {next}
		FILENAME ~ /results\.tsv$/ {st[$1] = $2; next}
		index($2, want) {ref[$1] = $2}
		END {for (c in ref) print c, (c in st ? st[c] : "?"), ref[c]}' \
		$(for s in "${slices[@]}"; do echo "$s/results.tsv" "$s/refusals.tsv"; done) | sort > "$out/board.tsv"
	# in mustpass order
	awk -F'\t' 'FNR == NR {want[$1] = 1; next} ($0 in want)' "$out/board.tsv" "$MUSTPASS" > "$out/cases.txt"
	if [ -n "$filter" ]; then
		grep -F -- "$filter" "$out/cases.txt" > "$out/cases.filtered" || true
		mv "$out/cases.filtered" "$out/cases.txt"
	fi
	n=$(wc -l < "$out/cases.txt")
	[ "$n" -gt 0 ] || { echo "$me: no case to run" >&2; exit 1; }
	echo "$me: $n refused case(s) of ${#slices[@]} slice(s) ($(wc -l < "$out/board.tsv") in board.tsv)"
	(cd "$out" && split -l "$chunk" -d -a 3 cases.txt c_)
	{
		echo "tip=$COMMIT"
		echo "cts=$(git -C "$cts" rev-parse HEAD 2>/dev/null || echo "$CTS_COMMIT")"
		echo "command=$0 --out $out --cts $cts --jobs $jobs --chunk $chunk${filter:+ --filter $filter}"
		echo "slices=${slices[*]}"
		echo "library=$LIB (osal-linux gpu-mali shaderc-online no-shader-table report-es2)"
	} > "$out/commit"
	ls "$out"/c_??? | xargs -P "$jobs" -I{} bash -c 'run_chunk "$(realpath {})"' 2>/dev/null || true
	echo "$me: $(cat "$out"/c_*.qpa 2>/dev/null | grep -c '^#beginTestCaseResult' || true) of $n cases began; process exits:"
	grep -h '^exit=' "$out"/c_*.out | sort | uniq -c
}

# ---- 4. extract ---------------------------------------------------------------
extract() {
	[ -f "$out/board.tsv" ] || { echo "$me: $out is not a run directory (no board.tsv)" >&2; exit 1; }
	python3 -I - "$out" "$REPO" <<'PYEOF'
import html, os, re, sys
from collections import Counter, OrderedDict

out, repo = sys.argv[1], sys.argv[2]
REFUSED = 'online shaderc: the code generator refused'

# ---- the families: the refusal text's key, in the order of the plan ----------
# (family, taken by plan #377?, key substring of the Mali(Valhall: ...) text)
FAMILIES = [
    ('intval', True, 'an integer or boolean value'),
    ('varylink', True, 'two varying lanes of equal value'),
    ('varylink', True, 'a varying lane computed from no attribute'),
    ('varylink', True, 'a gl_Position lane left unwritten'),
    ('varylink', True, 'a varying lane left unwritten'),
    ('unitype', True, 'a uniform other than a float, vec2, vec3, vec4 or mat4'),
    ('unitype', True, 'a vertex shader uniform that is not highp'),
    ('unitype', True, 'a mediump fragment shader uniform other than'),
    ('uniarray', True, 'a uniform array is not lowered'),
    ('uniarray', True, 'a uniform read at a run-time index'),
    ('constop', True, 'a constant operation on or to a value that is not a finite normal float'),
    ('constop', True, 'a -0.0 that a comparison reads through a variable'),
    ('constop', True, 'a constant fp16 does not hold'),
    ('mixprec', True, 'a fragment shader that mixes highp and mediump or lowp'),
    ('mixprec', True, 'a fragment varying load or conversion of a form no reference draws'),
    ('colourpair', True, 'a colour pair of a constant then a varying lane'),
    ('colourpair', True, 'a colour pair of a varying lane and 0.0'),
    ('loops', True, 'a loop in a vertex shader'),
    ('loops', True, 'a loop Mesa keeps'),
    ('loops', True, 'a break or continue behind a run-time condition'),
    ('loops', True, "a loop past Mesa's unroll limits"),
    ('misc', True, 'an invariant output'),
    ('misc', True, "more than the split file's 32 work registers"),
    ('misc', True, 'a shader of more than 2048 IR values'),
    ('misc', True, 'a comparison other than'),
    ('misc', True, "a texture result's lanes"),
    ('misc', True, 'a built-in input'),
    ('misc', True, "a shape Mesa's algebraic rules rewrite without a reference"),
    # plan #367's glmark2 families (the names of tools/mali/corpus-verdicts.txt)
    ('367:ARITH', False, 'arithmetic other than +, -, *'),
    ('367:ARITH', False, 'three or more additions or multiplications in a chain'),
    ('367:ARITH', False, 'a fragment shader operand of a form or in a place no reference draws'),
    ('367:CONST_FOLD', False, "a constant product in a shape Mesa's algebraic rules move or fold"),
    ('367:FMA', False, 'a fused multiply-add other than'),
    ('367:LOCALS', False, 'local variables is not lowered yet'),
    ('367:FRAGCOLOR_LANE', False, "a gl_FragColor lane other than the varying's own lane"),
    ('367:TEX_COORD', False, 'a texture coordinate other than'),
    ('367:TEX_VERTEX', False, 'a texture look-up other than'),
    ('367:CONTROL_FLOW', False, 'control flow (if, a loop, a return before the end)'),
    # texts the tip emits that the slices' families did not name (first
    # ranked at 9ac31f30, docs/notes/mali-deqp-families.md section 18)
    ('new:DEFECT_IRINDEX', False, 'back end defect: an invariant of the pipeline did not hold (IrIndex'),
    ('new:DEFECT_ENCODE', False, 'back end defect: an invariant of the pipeline did not hold (Encode'),
    ('new:ATTR_LINK', False, 'the Valhall back end of a vertex shader at the link'),
    ('new:ATTR_FOLD', False, 'a vertex shader whose attribute numbering waits for a fold'),
    ('new:FRAGCOORD_FP16', False, 'fp16 arithmetic on gl_FragCoord'),
    ('new:DISCARD_FORM', False, 'a discard other than'),
    ('new:TEX_VS_FORM', False, 'a vertex shader texture look-up other than'),
    ('new:COLOUR_TOO_LARGE', False, 'a fragment colour of more than 255'),
]
TAKEN = OrderedDict()
for fam, taken, _ in FAMILIES:
    TAKEN.setdefault(fam, taken)

def family_of(text):
    """The family of a refusal text ('' when it is not a compile refusal)."""
    if not text or REFUSED not in text:
        return ''
    for fam, _, key in FAMILIES:
        if key in text:
            return fam
    return 'other'

def stage_of(text):
    m = re.search(r'refused this (vertex|fragment) shader', text or '')
    return m.group(1)[0] + 's' if m else ''

def short(text):
    """The Mali(Valhall: ...) part, cut at 'is not lowered' (a board text
    longer than the info log dEQP reads ends cut: same_refusal allows it)."""
    m = re.search(r'Mali\(Valhall: (.*)', text or '')
    t = m.group(1) if m else (text or '')
    t = t.split(' is not lowered')[0].rstrip(')')
    return t.strip()

def same_refusal(host, board):
    """The host's and the board's texts name the same refusal of the same
    stage: equal, or one the cut prefix of the other."""
    if stage_of(host) != stage_of(board):
        return False
    h, b = short(host), short(board)
    return h == b or h.startswith(b) or b.startswith(h)

# ---- the shapes: the construct of the refusing shader that triggers the text ---
# Each family's rules are tried in order on (src, text, stage, case name,
# partner's src); the first that holds names the shape, else 'other'.
def has(src, pat):
    return re.search(pat, src) is not None

def decls(src, kind):
    """(precision, type, name, is_array) of every declared `kind`."""
    r = []
    for m in re.finditer(r'(?m)^\s*(?:invariant\s+)?' + kind + r'\s+(?:(lowp|mediump|highp)\s+)?(\w+)\s+([^;]+);', src):
        for name in m.group(3).split(','):
            r.append((m.group(1) or '', m.group(2), name.strip().split('[')[0].strip(), '[' in name))
    return r

def utypes(src):
    return [ty for _, ty, _, _ in decls(src, 'uniform')]

def uarrays(src):
    return [ty for _, ty, _, arr in decls(src, 'uniform') if arr]

INTISH = ('int', 'ivec2', 'ivec3', 'ivec4')
BOOLISH = ('bool', 'bvec2', 'bvec3', 'bvec4')
RELATIONAL = r'\b(lessThan|lessThanEqual|greaterThan|greaterThanEqual|equal|notEqual|any|all|not)\s*\('

def strip_harness(src):
    """The dEQP shader-library fragment harness taken out: `bool isOk (...)`,
    `bool RES = isOk(...)` and `gl_FragColor = vec4(RES, RES, RES, 1.0)`,
    with the precision statements and the preprocessor lines (an `#if`
    expression is the preprocessor's, not a shader value)."""
    return '\n'.join(l for l in src.split('\n')
                     if not re.search(r'bool isOk\s*\(|bool RES = isOk|vec4\(RES, RES, RES|^\s*precision\s|^\s*#', l))

def intish(src):
    """An integer or boolean value outside the harness."""
    s = strip_harness(src)
    return (has(s, r'\b(int|ivec[234]|bool|bvec[234])\b') or has(s, RELATIONAL)
            or has(s, r'&&|\|\||\^\^|[!=<>]=|[<>]|\?'))

def random_group(case):
    return '.functional.shaders.random.' in case or '.fragment_ops.interaction.' in case

RANDOM = r'^random_mixed$'
TRANSC = r'\b(exp2|exp|log2|log|pow|sqrt|inversesqrt|sin|cos|tan|asin|acos|atan|radians|degrees)\s*\('
COMMON = r'\b(abs|sign|floor|ceil|fract|mod|min|max|clamp|mix|step|smoothstep)\s*\('
GEOM = r'\b(length|distance|dot|cross|normalize|faceforward|reflect|refract|matrixCompMult)\s*\('
CONST_CALL = r'const\s+\w+\s+\w+\s*=\s*[^;]*\b\w+\s*\('

RULES = {
    'intval': [
        ('random_mixed', lambda s, t, st, c, o: random_group(c)),
        ('int_uniform', lambda s, t, st, c, o: any(ty in INTISH for ty in utypes(s))),
        ('bool_uniform', lambda s, t, st, c, o: any(ty in BOOLISH for ty in utypes(s))),
        ('int_loop_counter', lambda s, t, st, c, o: has(s, r'for\s*\(\s*(?:(?:lowp|mediump|highp)\s+)?int\b')),
        ('ivec_operator', lambda s, t, st, c, o: has(s, r'\b(?:lowp|mediump|highp)\s+(?:int|ivec[234])\s+(?:in0|res)\b') or has(s, r'\bivec[234]\s*\(')),
        ('bvec_relational', lambda s, t, st, c, o: has(s, RELATIONAL) or has(s, r'\bbvec[234]\b')),
        ('scalar_conversion', lambda s, t, st, c, o: has(s, r'\b(int|bool)\s*\(') or has(s, r'\bfloat\s*\(\s*(?:in0|res|a|b)\b')),
        ('harness_isok', lambda s, t, st, c, o: 'bool RES = isOk' in s and not intish(s)),
        ('step_sign', lambda s, t, st, c, o: has(s, r'\b(step|sign)\s*\(')),
        ('bool_literal', lambda s, t, st, c, o: has(strip_harness(s), r'\b(true|false)\b')),
        ('bool_logic', lambda s, t, st, c, o: has(strip_harness(s), r'&&|\|\||\^\^|\bbool\b|!\w')),
        ('float_compare', lambda s, t, st, c, o: has(strip_harness(s), r'[!=<>]=|[<>]|\?')),
    ],
    'varylink': [
        ('equal_lanes_broadcast', lambda s, t, st, c, o: 'two varying lanes of equal value' in t and has(s, r'\bvec[234]\s*\(\s*\w+\s*\)')),
        ('equal_lanes', lambda s, t, st, c, o: 'two varying lanes of equal value' in t),
        ('lane_no_attribute', lambda s, t, st, c, o: 'computed from no attribute' in t),
        ('position_unwritten', lambda s, t, st, c, o: 'gl_Position lane left unwritten' in t),
        ('varying_unwritten', lambda s, t, st, c, o: 'a varying lane left unwritten' in t),
    ],
    'unitype': [
        ('vs_struct_uniform', lambda s, t, st, c, o: 'a uniform other than' in t and has(s, r'\bstruct\b')),
        ('vs_mat_uniform', lambda s, t, st, c, o: 'a uniform other than' in t and any(ty in ('mat2', 'mat3') for ty in utypes(s))),
        ('vs_int_uniform', lambda s, t, st, c, o: 'a uniform other than' in t and any(ty in INTISH for ty in utypes(s))),
        ('vs_bool_uniform', lambda s, t, st, c, o: 'a uniform other than' in t and any(ty in BOOLISH for ty in utypes(s))),
        ('vs_sampler_uniform', lambda s, t, st, c, o: 'a uniform other than' in t and any(ty.startswith('sampler') for ty in utypes(s))),
        ('vs_other_type', lambda s, t, st, c, o: 'a uniform other than' in t),
        ('vs_mediump_struct', lambda s, t, st, c, o: 'a vertex shader uniform that is not highp' in t and has(s, r'\bstruct\b')),
        ('vs_mediump_array', lambda s, t, st, c, o: 'a vertex shader uniform that is not highp' in t and uarrays(s)),
        ('vs_mediump_basic', lambda s, t, st, c, o: 'a vertex shader uniform that is not highp' in t),
        ('fs_struct_uniform', lambda s, t, st, c, o: has(s, r'\bstruct\b')),
        ('fs_mat_uniform', lambda s, t, st, c, o: any(ty.startswith('mat') for ty in utypes(s))),
        ('fs_bool_uniform', lambda s, t, st, c, o: any(ty in BOOLISH for ty in utypes(s))),
        ('fs_int_uniform', lambda s, t, st, c, o: any(ty in INTISH for ty in utypes(s))),
        ('fs_vec_array', lambda s, t, st, c, o: any(ty.startswith('vec') for ty in uarrays(s))),
        ('fs_highp_uniform', lambda s, t, st, c, o: any(p == 'highp' for p, _, _, _ in decls(s, 'uniform'))),
        ('fs_other', lambda s, t, st, c, o: True),
    ],
    'uniarray': [
        ('runtime_index', lambda s, t, st, c, o: 'a uniform read at a run-time index' in t),
        ('array_in_struct', lambda s, t, st, c, o: has(s, r'\bstruct\b') and not uarrays(s)),
        ('struct_array', lambda s, t, st, c, o: has(s, r'\bstruct\b') and any(ty not in INTISH + BOOLISH and not ty.startswith(('vec', 'mat', 'float', 'sampler')) for ty in uarrays(s))),
        ('bool_array', lambda s, t, st, c, o: any(ty in BOOLISH for ty in uarrays(s))),
        ('int_array', lambda s, t, st, c, o: any(ty in INTISH for ty in uarrays(s))),
        ('mat_array', lambda s, t, st, c, o: any(ty.startswith('mat') for ty in uarrays(s))),
        ('sampler_array', lambda s, t, st, c, o: any(ty.startswith('sampler') for ty in uarrays(s))),
        ('float_array', lambda s, t, st, c, o: 'float' in uarrays(s)),
        ('vec_array', lambda s, t, st, c, o: any(ty.startswith('vec') for ty in uarrays(s))),
        ('other_array', lambda s, t, st, c, o: True),
    ],
    'constop': [
        ('neg_zero_variable', lambda s, t, st, c, o: 'a -0.0 that a comparison reads' in t),
        ('fp16_constant', lambda s, t, st, c, o: 'a constant fp16 does not hold' in t),
        ('random_mixed', lambda s, t, st, c, o: random_group(c)),
        ('const_call_transcendental', lambda s, t, st, c, o: has(s, CONST_CALL) and has(s, TRANSC)),
        ('const_call_common', lambda s, t, st, c, o: has(s, CONST_CALL) and has(s, COMMON)),
        ('const_call_geometric', lambda s, t, st, c, o: has(s, CONST_CALL) and has(s, GEOM)),
        ('const_call_other', lambda s, t, st, c, o: has(s, CONST_CALL)),
        ('neg_zero_literal', lambda s, t, st, c, o: '-0.0' in s),
        ('zero_operand', lambda s, t, st, c, o: has(s, r'\b0\.0\b')),
        ('const_other', lambda s, t, st, c, o: True),
    ],
    'mixprec': [
        ('fs_uniform_compare_harness', lambda s, t, st, c, o: 'mixes highp and mediump' in t and 'compare_float' in s),
        ('fs_highp_declared', lambda s, t, st, c, o: 'mixes highp and mediump' in t and has(s, r'\bhighp\b')),
        ('fs_mixed_other', lambda s, t, st, c, o: 'mixes highp and mediump' in t),
        ('fs_varying_rgb_one', lambda s, t, st, c, o: has(s, r'gl_FragColor\s*=\s*vec4\(\s*\w+\.rgb\s*,\s*1\.0\s*\)')),
        ('fs_varying_plain', lambda s, t, st, c, o: has(s, r'gl_FragColor\s*=\s*v_\w+\s*;')),
        ('fs_varying_other', lambda s, t, st, c, o: True),
    ],
    'colourpair': [
        ('constant_then_varying', lambda s, t, st, c, o: 'a constant then a varying lane' in t),
        ('varying_and_zero', lambda s, t, st, c, o: 'a varying lane and 0.0' in t),
    ],
    'loops': [
        ('vs_for_index', lambda s, t, st, c, o: 'a loop in a vertex shader' in t and has(s, r'\bfor\b') and '.shaders.indexing.' in c),
        ('vs_for_constant', lambda s, t, st, c, o: 'a loop in a vertex shader' in t and has(s, r'for\s*\([^;]*;\s*\w+(?:\.\w)?\s*<=?\s*\d+\s*;')),
        ('vs_for_uniform', lambda s, t, st, c, o: 'a loop in a vertex shader' in t and has(s, r'\bfor\b') and any(ty in INTISH for ty in utypes(s))),
        ('vs_for_dynamic', lambda s, t, st, c, o: 'a loop in a vertex shader' in t and has(s, r'\bfor\b')),
        ('vs_while', lambda s, t, st, c, o: 'a loop in a vertex shader' in t and has(s, r'\bwhile\b') and not has(s, r'\bdo\b')),
        ('vs_do_while', lambda s, t, st, c, o: 'a loop in a vertex shader' in t and has(s, r'\bdo\b')),
        ('vs_other', lambda s, t, st, c, o: 'a loop in a vertex shader' in t),
        ('fs_kept_for_index', lambda s, t, st, c, o: 'a loop Mesa keeps' in t and has(s, r'\bfor\b') and ('.shaders.indexing.' in c or '.shaders.struct.' in c)),
        ('fs_kept_for', lambda s, t, st, c, o: 'a loop Mesa keeps' in t and has(s, r'\bfor\b')),
        ('fs_kept_while', lambda s, t, st, c, o: 'a loop Mesa keeps' in t and has(s, r'\bwhile\b') and not has(s, r'\bdo\b')),
        ('fs_kept_do_while', lambda s, t, st, c, o: 'a loop Mesa keeps' in t and has(s, r'\bdo\b')),
        ('fs_kept_other', lambda s, t, st, c, o: 'a loop Mesa keeps' in t),
        ('fs_unrolled_break', lambda s, t, st, c, o: 'a break or continue behind a run-time condition' in t),
        ('fs_past_limits', lambda s, t, st, c, o: "past Mesa's unroll limits" in t),
    ],
    'misc': [
        ('invariant_output', lambda s, t, st, c, o: 'an invariant output' in t),
        ('work_registers', lambda s, t, st, c, o: 'work registers' in t),
        ('ir_values', lambda s, t, st, c, o: 'IR values' in t),
        ('comparison_form', lambda s, t, st, c, o: 'a comparison other than' in t),
        ('texture_lanes', lambda s, t, st, c, o: "a texture result's lanes" in t),
        ('builtin_input', lambda s, t, st, c, o: 'a built-in input' in t),
        ('algebraic_rewrite', lambda s, t, st, c, o: "algebraic rules rewrite without a reference" in t),
    ],
}

# Which shapes plan #367 holds through its glmark2 references (the gm2_*
# files under tools/mali/testdata; docs/notes/mali-glmark2-forecast.md):
# the int counter of a for loop and the loop forms with a constant or an
# int-uniform bound (gm2_intloop_*, gm2_cf_loop_*). Everything else in the
# taken families is this plan's; a shape named here with a reason is taken
# by this plan but overlaps a #367 reference.
OWNER = {
    ('intval', 'int_loop_counter'): '#367 (gm2_intloop_* int counters; the compare or return in the body is left)',
    ('loops', 'vs_for_constant'): '#367 (gm2_intloop_const_vs)',
    ('loops', 'vs_for_uniform'): '#367 (gm2_cf_loop_uni_vs, gm2_intloop_uniform_vs)',
    ('loops', 'fs_kept_for'): '#367 (gm2_cf_loop_uni_fs, gm2_intloop_uniform_fs)',
    ('intval', 'bool_logic'): 'this plan (#367 holds only && of two float compares in a one-armed if, gm2_cf_ifand_vs)',
}

def owner_of(fam, shape):
    if not TAKEN.get(fam, False):
        return '#367' if fam.startswith('367') else '-'
    return OWNER.get((fam, shape), 'this plan')

def shape_of(fam, src, text, stage, case, other):
    for shape, pred in RULES.get(fam, []):
        try:
            if pred(src, text, stage, case, other):
                return shape
        except Exception:
            pass
    return 'other'

# ---- the .qpa: per case its status, shaders and info logs ----------------------
RE_CASE = re.compile(r'^#beginTestCaseResult (\S+)', re.M)
RE_RESULT = re.compile(r'<Result\s+StatusCode="([^"]+)"')
# dEQP logs a program in two forms. glu::ShaderProgram (shaders.*, most
# groups): <ShaderProgram LinkStatus=..> with the program's <InfoLog> (the
# link log) FIRST, then <VertexShader CompileStatus=..> <ShaderSource>..
# </ShaderSource> <InfoLog>..</InfoLog> per shader. The attribute_location,
# shader_api and a few other groups log each shader as <Section Name="Vertex
# Shader Info"> with a <KernelSource> and an inner <Section Name="Info Log">
# <Text>, and the link as <Section Name="Program Link Info"> after them.
RE_UNIT = re.compile(
    r'<ShaderProgram\b[^>]*>\s*(?:<InfoLog>(?P<plog>.*?)</InfoLog>)?(?P<p>)'
    r'|<(?P<a>Vertex|Fragment)Shader\b[^>]*>(?P<ab>.*?)</(?P=a)Shader>'
    r'|(?P<e></ShaderProgram>)'
    r'|<Section Name="(?P<b>Vertex|Fragment) Shader Info"[^>]*>\s*<KernelSource>(?P<bsrc>.*?)</KernelSource>'
    r'\s*(?:<Section Name="Info Log"[^>]*>\s*<Text>(?P<blog>.*?)</Text>)?'
    r'|<Section Name="Program Link Info"[^>]*>\s*(?:<Section Name="Info Log"[^>]*>\s*<Text>(?P<dlog>.*?)</Text>)?(?P<d>)',
    re.S)
RE_SOURCE = re.compile(r'<ShaderSource>(.*?)</ShaderSource>', re.S)
RE_INFO = re.compile(r'<InfoLog>(.*?)</InfoLog>', re.S)

def unesc(s):
    return html.unescape(s)

def parse_case(block):
    """(status, [(vs_src, fs_src, vs_log, fs_log, link_log), ...]) of one
    case: one tuple per program, in order of appearance."""
    m = RE_RESULT.search(block)
    status = m.group(1) if m else 'NoResult'
    programs = []
    cur = {'vs': ('', ''), 'fs': ('', ''), 'link': ''}
    def flush():
        if cur['vs'][0] or cur['fs'][0] or cur['link']:
            programs.append((cur['vs'][0], cur['fs'][0], cur['vs'][1], cur['fs'][1], cur['link']))
        cur['vs'] = ('', ''); cur['fs'] = ('', ''); cur['link'] = ''
    for u in RE_UNIT.finditer(block):
        if u.group('p') is not None:
            flush()
            cur['link'] = unesc(u.group('plog') or '')
        elif u.group('a'):
            st = 'vs' if u.group('a') == 'Vertex' else 'fs'
            ms = RE_SOURCE.search(u.group('ab')); ml = RE_INFO.search(u.group('ab'))
            if cur[st][0]:
                flush()
            cur[st] = (unesc(ms.group(1)) if ms else '', unesc(ml.group(1)) if ml else '')
        elif u.group('e'):
            flush()
        elif u.group('b'):
            st = 'vs' if u.group('b') == 'Vertex' else 'fs'
            if cur[st][0]:
                flush()
            cur[st] = (unesc(u.group('bsrc')), unesc(u.group('blog') or ''))
        else:
            cur['link'] = unesc(u.group('dlog') or '')
            flush()
    flush()
    return status, programs

cases = {}
for q in sorted(f for f in os.listdir(out) if f.startswith('c_') and f.endswith('.qpa')):
    with open(os.path.join(out, q), errors='replace') as f:
        data = f.read()
    parts = RE_CASE.split(data)
    for i in range(1, len(parts), 2):
        cases[parts[i]] = parse_case(parts[i + 1])

board = {}
with open(os.path.join(out, 'board.tsv')) as f:
    for line in f:
        c, st, text = line.rstrip('\n').split('\t', 2)
        board[c] = (st, text)
with open(os.path.join(out, 'cases.txt')) as f:
    order = [l.strip() for l in f if l.strip()]

def first_refusal(programs):
    """(stage, text, src, other_src, next_text) of the first refusal in the
    case's programs; next_text is the partner shader's own refusal when dEQP
    compiled it too (both shaders are compiled before the link), else ''."""
    for vs, fs, vl, fl, link in programs:
        for stage, log, src, other, olog in (('vs', vl, vs, fs, fl), ('fs', fl, fs, vs, vl)):
            if REFUSED in log:
                return stage, log, src, other, (olog if REFUSED in olog else '')
        if REFUSED in link:
            st = stage_of(link)
            return st or 'link', link, (vs if st == 'vs' else fs), (fs if st == 'vs' else vs), ''
    return '', '', '', '', ''

rows = []
shapes = OrderedDict()
notrep = []
srcdir = os.path.join(out, 'src')
os.makedirs(srcdir, exist_ok=True)
for c in order:
    bst, btext = board.get(c, ('?', ''))
    bfam = family_of(btext)
    if c not in cases:
        rows.append((c, bst, bfam, 'NotRun', '', '', '', '', ''))
        notrep.append((c, bst, bfam, 'not run on the host', short(btext)))
        continue
    status, programs = cases[c]
    stage, text, src, other, ntext = first_refusal(programs)
    hfam = family_of(text)
    if not text:
        rows.append((c, bst, bfam, status, '', '', '', '', ''))
        m = RE_RESULT.search('')
        notrep.append((c, bst, bfam, 'no compile refusal on the host (dEQP %s)' % status, short(btext)))
        continue
    shape = shape_of(hfam, src, text, stage, c, other) if TAKEN.get(hfam, False) else ('367' if hfam.startswith('367') else 'other')
    if not same_refusal(text, btext):
        notrep.append((c, bst, bfam, 'host refusal differs: %s %s' % (stage, short(text)), short(btext)))
    nfam = family_of(ntext)
    rows.append((c, bst, bfam, status, stage, hfam, shape, short(text), nfam))
    for ext, s in (('.vert', src if stage == 'vs' else other), ('.frag', src if stage == 'fs' else other)):
        with open(os.path.join(srcdir, c + ext), 'w') as f:
            f.write(s if not s or s.endswith('\n') else s + '\n')
    k = (hfam, shape)
    d = shapes.setdefault(k, {'cases': 0, 'Fail': 0, 'NotSupported': 0, 'other_status': 0,
                              'stages': Counter(), 'moved': 0, 'next': Counter(), 'groups': Counter(),
                              'first': {}})
    # the representative: the first case (mustpass order) of the shape's
    # largest dEQP group, chosen at the end
    grp = '.'.join(c.split('.')[2:4])
    d['first'].setdefault(grp, (c, other if stage == 'fs' else src, src if stage == 'fs' else other))
    d['cases'] += 1
    if bst in ('Fail', 'NotSupported'):
        d[bst] += 1
    else:
        d['other_status'] += 1
    d['stages'][stage] += 1
    d['next'][nfam or '-'] += 1
    d['groups']['.'.join(c.split('.')[2:4])] += 1
    if bfam != hfam:
        d['moved'] += 1

for d in shapes.values():
    d['example'], d['vs'], d['fs'] = d['first'][d['groups'].most_common(1)[0][0]]

with open(os.path.join(out, 'measure.tsv'), 'w') as f:
    f.write('case\tboard_status\tboard_family\thost_status\tstage\thost_family\tshape\thost_refusal\tnext_family\n')
    for r in rows:
        f.write('\t'.join(r) + '\n')
with open(os.path.join(out, 'not-reproduced.tsv'), 'w') as f:
    f.write('case\tboard_status\tboard_family\treason\tboard_refusal\n')
    for r in notrep:
        f.write('\t'.join(r) + '\n')

# the representative pairs: one per shape of a taken family (the first case
# of the shape in mustpass order), with its case name and an index
sd = os.path.join(out, 'shapes')
os.makedirs(sd, exist_ok=True)
with open(os.path.join(sd, 'index.tsv'), 'w') as idx:
    idx.write('family\tshape\towner\tcases\tfail\tnotsupported\texample\n')
    for (fam, shape), d in shapes.items():
        if not TAKEN.get(fam, False):
            continue
        base = os.path.join(sd, '%s_%s' % (fam, shape))
        for ext, s in (('.vert', d['vs']), ('.frag', d['fs'])):
            with open(base + ext, 'w') as f:
                f.write(s if not s or s.endswith('\n') else s + '\n')
        with open(base + '.case', 'w') as f:
            f.write(d['example'] + '\n')
        idx.write('%s\t%s\t%s\t%d\t%d\t%d\t%s\n' % (fam, shape, owner_of(fam, shape), d['cases'], d['Fail'], d['NotSupported'], d['example']))

# summary.md
fam_order = list(TAKEN.keys()) + ['other']
tot = Counter(); totF = Counter(); totN = Counter()
for (fam, shape), d in shapes.items():
    tot[fam] += d['cases']; totF[fam] += d['Fail']; totN[fam] += d['NotSupported']
def fmt_counter(cnt, n=3):
    return ', '.join('%s %d' % (k, v) for k, v in cnt.most_common(n))
with open(os.path.join(out, 'summary.md'), 'w') as f:
    f.write('HOST-ONLY: %d cases asked, %d began on the host, %d with a compile refusal on the host, %d not reproduced.\n\n'
            % (len(order), sum(1 for c in order if c in cases), sum(1 for r in rows if r[5]), len(notrep)))
    f.write('| Family | Owner | Cases (HOST-ONLY) | Fail (board) | NotSupported (board) |\n|---|---|---:|---:|---:|\n')
    for fam in fam_order:
        if tot[fam]:
            f.write('| %s | %s | %d | %d | %d |\n' % (fam, 'this plan' if TAKEN.get(fam) else ('#367' if fam.startswith('367') else '-'), tot[fam], totF[fam], totN[fam]))
    for fam in fam_order:
        items = [(k, d) for k, d in shapes.items() if k[0] == fam]
        if not items:
            continue
        f.write('\n### %s (HOST-ONLY)\n\n| Shape | Cases | Fail | NS | VS / FS | Owner | Example | dEQP groups | Next refusal (partner shader) |\n|---|---:|---:|---:|---|---|---|---|---|\n' % fam)
        for (_, shape), d in sorted(items, key=lambda kd: -kd[1]['cases']):
            f.write('| %s | %d | %d | %d | %d / %d | %s | `%s` | %s | %s |\n' % (
                shape, d['cases'], d['Fail'], d['NotSupported'], d['stages']['vs'], d['stages']['fs'],
                owner_of(fam, shape), d['example'], fmt_counter(d['groups']), fmt_counter(d['next'])))
    f.write('\n### Not reproduced (HOST-ONLY)\n\n')
    for reason, n in Counter(r[3].split(':')[0] for r in notrep).most_common():
        f.write('- %s: %d\n' % (reason, n))
    if notrep:
        f.write('\nBy board family, host outcome and board refusal:\n\n')
        for (bf, reason, bref), n in Counter((r[2], r[3], r[4][:60]) for r in notrep).most_common():
            f.write('- %s -> %s (board: %s): %d\n' % (bf, reason, bref, n))
print('%s: %d cases, %d with a host refusal, %d shapes, %d not reproduced; see %s/summary.md (HOST-ONLY)'
      % ('deqp-host-measure', len(order), sum(1 for r in rows if r[5]), len(shapes), len(notrep), out))
PYEOF
}

case $stage in
all) build_deqp; build_lib; run_cases; extract ;;
build) build_deqp; build_lib ;;
deqp) build_deqp ;;
lib) build_lib ;;
run) run_cases ;;
extract) extract ;;
esac
echo "$me: results in $out (HOST-ONLY: never quote a render verdict from them)"
